// Fuzzy Text Index Implementation

use crate::document::DocumentId;
use crate::error::{IronBaseError, Result};
use crate::value_utils::get_nested_value;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use strsim::{damerau_levenshtein, jaro_winkler, normalized_levenshtein};

/// Magic bytes for .fzidx files
const FUZZY_INDEX_MAGIC: &[u8; 8] = b"IRONFZX\0";
/// Current file format version
const FUZZY_INDEX_VERSION: u32 = 1;
/// Header size in bytes
const HEADER_SIZE: u64 = 64;
const FUZZY_LAZY_LOAD_THRESHOLD_BYTES: u64 = 100 * 1024 * 1024;

/// Fuzzy matching algorithm
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FuzzyAlgorithm {
    /// Jaro-Winkler similarity (default) - fast, good for names
    #[default]
    JaroWinkler,
    /// Normalized Levenshtein distance - accurate edit distance
    Levenshtein,
    /// Damerau-Levenshtein - includes transpositions
    DamerauLevenshtein,
}

impl FuzzyAlgorithm {
    /// Calculate similarity between two strings (0.0 to 1.0)
    pub fn similarity(&self, a: &str, b: &str) -> f64 {
        let a_lower = a.to_lowercase();
        let b_lower = b.to_lowercase();
        let result = match self {
            FuzzyAlgorithm::JaroWinkler => jaro_winkler(&a_lower, &b_lower),
            FuzzyAlgorithm::Levenshtein => normalized_levenshtein(&a_lower, &b_lower),
            FuzzyAlgorithm::DamerauLevenshtein => {
                let max_len = a_lower.len().max(b_lower.len());
                if max_len == 0 {
                    return 1.0;
                }
                let distance = damerau_levenshtein(&a_lower, &b_lower);
                1.0 - (distance as f64 / max_len as f64)
            }
        };

        // Defensive: treat NaN as no match (consistent with btree.rs pattern)
        if result.is_nan() {
            0.0
        } else {
            result
        }
    }

    /// Parse algorithm name from string
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "jaro_winkler" | "jarowinkler" => Some(FuzzyAlgorithm::JaroWinkler),
            "levenshtein" => Some(FuzzyAlgorithm::Levenshtein),
            "damerau_levenshtein" | "dameraulevenshtein" => {
                Some(FuzzyAlgorithm::DamerauLevenshtein)
            }
            _ => None,
        }
    }
}

/// Fuzzy index metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuzzyIndexMetadata {
    pub name: String,
    pub field: String,
    pub algorithm: FuzzyAlgorithm,
    pub threshold: f64,
    pub num_entries: usize,
    /// True while index is being built - query planner should ignore this index
    #[serde(default)]
    pub building: bool,
}

/// Fuzzy text index - stores string values for similarity search
///
/// Unlike B+ tree indexes which support exact matching and range queries,
/// fuzzy indexes support similarity-based search using algorithms like
/// Jaro-Winkler or Levenshtein distance.
///
/// # Performance Characteristics
/// - Insert: O(1)
/// - Search: O(n) where n = number of indexed values
/// - Storage: ~40-60% overhead per indexed field
///
/// # Example
/// ```rust,ignore
/// let mut index = FuzzyIndex::new("name_fuzzy", "name", FuzzyAlgorithm::JaroWinkler, 0.8);
/// index.insert("John Smith", doc_id);
/// let matches = index.search("Jon Smyth", None); // Returns similar matches
/// ```
#[derive(Debug, Clone)]
pub struct FuzzyIndex {
    pub metadata: FuzzyIndexMetadata,
    /// Indexed entries: (lowercase_value, original_value, document_id)
    entries: Vec<(String, String, DocumentId)>,
    /// Optional storage path for persistence
    storage_path: Option<PathBuf>,
    lazy_mode: bool,
    entries_offset: u64,
    entries_size: u64,
}

impl FuzzyIndex {
    /// Create a new fuzzy index
    pub fn new(name: &str, field: &str, algorithm: FuzzyAlgorithm, threshold: f64) -> Self {
        FuzzyIndex {
            metadata: FuzzyIndexMetadata {
                name: name.to_string(),
                field: field.to_string(),
                algorithm,
                threshold: threshold.clamp(0.0, 1.0),
                num_entries: 0,
                building: false,
            },
            entries: Vec::new(),
            storage_path: None,
            lazy_mode: false,
            entries_offset: 0,
            entries_size: 0,
        }
    }

    /// Create a new fuzzy index with storage path for persistence
    pub fn new_with_storage(
        name: &str,
        field: &str,
        algorithm: FuzzyAlgorithm,
        threshold: f64,
        storage_path: PathBuf,
    ) -> Self {
        FuzzyIndex {
            metadata: FuzzyIndexMetadata {
                name: name.to_string(),
                field: field.to_string(),
                algorithm,
                threshold: threshold.clamp(0.0, 1.0),
                num_entries: 0,
                building: false,
            },
            entries: Vec::new(),
            storage_path: Some(storage_path),
            lazy_mode: false,
            entries_offset: 0,
            entries_size: 0,
        }
    }

    /// Check if this index is currently being built
    pub fn is_building(&self) -> bool {
        self.metadata.building
    }

    /// Set the building flag (for index creation lifecycle)
    pub fn set_building(&mut self, building: bool) {
        self.metadata.building = building;
    }

    /// Set storage path for persistence
    pub fn set_storage_path(&mut self, path: PathBuf) {
        self.storage_path = Some(path);
    }

    /// Get storage path
    pub fn storage_path(&self) -> Option<&PathBuf> {
        self.storage_path.as_ref()
    }

    /// Get entry count (for checking if loaded from file)
    pub fn entry_count(&self) -> usize {
        self.metadata.num_entries
    }

    /// Insert a value into the fuzzy index
    pub fn insert(&mut self, value: &str, doc_id: DocumentId) {
        let _ = self.ensure_entries_loaded();
        let lower = value.to_lowercase();
        self.entries.push((lower, value.to_string(), doc_id));
        self.metadata.num_entries = self.entries.len();
    }

    /// Remove a document from the fuzzy index
    pub fn remove(&mut self, doc_id: &DocumentId) {
        let _ = self.ensure_entries_loaded();
        self.entries.retain(|(_, _, id)| id != doc_id);
        self.metadata.num_entries = self.entries.len();
    }

    /// Remove a specific value-document pair
    pub fn remove_value(&mut self, value: &str, doc_id: &DocumentId) {
        let _ = self.ensure_entries_loaded();
        let lower = value.to_lowercase();
        self.entries
            .retain(|(l, _, id)| !(l == &lower && id == doc_id));
        self.metadata.num_entries = self.entries.len();
    }

    /// Search for similar values
    ///
    /// Returns document IDs where the indexed value has similarity >= threshold
    /// Optionally override the default threshold for this search
    pub fn search(&self, query: &str, threshold_override: Option<f64>) -> Vec<(DocumentId, f64)> {
        let threshold = threshold_override.unwrap_or(self.metadata.threshold);
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();

        if self.lazy_mode && self.entries.is_empty() {
            if let Some(path) = &self.storage_path {
                if let Ok(mut on_disk) = self.search_in_file(path, &query_lower, threshold) {
                    results.append(&mut on_disk);
                }
                results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                return results;
            }
        }

        for (lower_value, _original, doc_id) in &self.entries {
            let similarity = self
                .metadata
                .algorithm
                .similarity(&query_lower, lower_value);
            if similarity >= threshold {
                results.push((doc_id.clone(), similarity));
            }
        }

        // Sort by similarity descending
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        results
    }

    /// Search with algorithm override
    pub fn search_with_algorithm(
        &self,
        query: &str,
        algorithm: FuzzyAlgorithm,
        threshold: f64,
    ) -> Vec<(DocumentId, f64)> {
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();

        for (lower_value, _original, doc_id) in &self.entries {
            let similarity = algorithm.similarity(&query_lower, lower_value);
            if similarity >= threshold {
                results.push((doc_id.clone(), similarity));
            }
        }

        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        results
    }

    /// Search with ExecutionContext for cancellation support
    ///
    /// This is the cancellation-aware version of `search`. It checks the
    /// ExecutionContext during the expensive similarity calculations.
    ///
    /// # Arguments
    /// * `query` - The search query string
    /// * `threshold_override` - Optional threshold override (0.0-1.0)
    /// * `algorithm_override` - Optional algorithm override
    /// * `limit` - Optional limit for early exit (returns top N results)
    /// * `ctx` - Optional execution context for cancellation
    pub fn search_with_ctx(
        &self,
        query: &str,
        threshold_override: Option<f64>,
        algorithm_override: Option<FuzzyAlgorithm>,
        limit: Option<usize>,
        ctx: Option<&crate::execution::ExecutionContext>,
    ) -> Result<Vec<(DocumentId, f64)>> {
        let threshold = threshold_override.unwrap_or(self.metadata.threshold);
        let algorithm = algorithm_override.unwrap_or(self.metadata.algorithm);
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();

        // KNOWN ISSUE (BUG #12): Lazy mode file search does NOT check ctx for cancellation.
        // The search_in_file() uses serde Visitor pattern which doesn't support periodic
        // cancellation checks. Risk is LOW because:
        // 1. Fuzzy indexes are typically small (only indexed field values)
        // 2. Lazy mode only activates for very large indexes (>100MB now)
        // 3. Outer search_with_ctx() checks cancellation during document loading
        // TODO: Refactor to support ctx - options: pre-load entries, custom streaming, or thread-local deadline
        if self.lazy_mode && self.entries.is_empty() {
            if let Some(path) = &self.storage_path {
                if let Ok(mut on_disk) = self.search_in_file(path, &query_lower, threshold) {
                    results.append(&mut on_disk);
                }
                results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                // Apply limit after sorting
                if let Some(lim) = limit {
                    results.truncate(lim);
                }
                return Ok(results);
            }
        }

        // Collect all matches above threshold
        // Note: iteration counter starts from 0 here - lazy-mode branch above always returns early
        for (iteration, (lower_value, _original, doc_id)) in self.entries.iter().enumerate() {
            if let Some(exec_ctx) = ctx {
                exec_ctx.maybe_check(iteration)?;
            }

            let similarity = algorithm.similarity(&query_lower, lower_value);
            if similarity >= threshold {
                results.push((doc_id.clone(), similarity));
            }
        }

        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Apply limit after sorting to get top N results
        if let Some(lim) = limit {
            results.truncate(lim);
        }

        Ok(results)
    }

    /// Get index size
    pub fn size(&self) -> usize {
        self.metadata.num_entries
    }

    /// Clear the index
    pub fn clear(&mut self) {
        self.entries.clear();
        self.metadata.num_entries = 0;
    }

    /// Rebuild index from documents
    ///
    /// Clears existing entries and rebuilds from the provided documents
    pub fn rebuild<'a, I>(&mut self, documents: I)
    where
        I: Iterator<Item = (&'a serde_json::Value, &'a DocumentId)>,
    {
        self.entries.clear();

        for (doc, doc_id) in documents {
            if let Some(value) = get_nested_value(doc, &self.metadata.field) {
                if let Some(s) = value.as_str() {
                    self.insert(s, doc_id.clone());
                }
            }
        }
    }

    // ========== PERSISTENCE METHODS ==========

    /// Flush index to storage file (.fzidx)
    ///
    /// File format:
    /// - Header (64 bytes): magic, version, entry_count, entries_offset, metadata_offset
    /// - Entries (bincode): Vec<(String, String, DocumentId)>
    /// - Metadata (JSON): FuzzyIndexMetadata
    pub fn flush(&self) -> Result<()> {
        let path = self.storage_path.as_ref().ok_or_else(|| {
            IronBaseError::IndexError("No storage path set for fuzzy index".to_string())
        })?;
        if self.lazy_mode && self.entries.is_empty() {
            return Ok(());
        }

        let mut file = File::create(path).map_err(|e| {
            IronBaseError::IndexError(format!("Failed to create fuzzy index file: {}", e))
        })?;

        // Serialize entries with JSON (more compatible with enum types like DocumentId)
        let entries_data = serde_json::to_vec(&self.entries).map_err(|e| {
            IronBaseError::IndexError(format!("Failed to serialize fuzzy entries: {}", e))
        })?;

        // Serialize metadata as JSON
        let metadata_json = serde_json::to_vec(&self.metadata).map_err(|e| {
            IronBaseError::IndexError(format!("Failed to serialize fuzzy metadata: {}", e))
        })?;

        // Calculate offsets
        let entries_offset = HEADER_SIZE;
        let metadata_offset = entries_offset + entries_data.len() as u64;

        // Write header
        file.write_all(FUZZY_INDEX_MAGIC).map_err(|e| {
            IronBaseError::IndexError(format!("Failed to write magic bytes: {}", e))
        })?;

        file.write_all(&FUZZY_INDEX_VERSION.to_le_bytes())
            .map_err(|e| IronBaseError::IndexError(format!("Failed to write version: {}", e)))?;

        file.write_all(&(self.entries.len() as u64).to_le_bytes())
            .map_err(|e| {
                IronBaseError::IndexError(format!("Failed to write entry count: {}", e))
            })?;

        file.write_all(&entries_offset.to_le_bytes()).map_err(|e| {
            IronBaseError::IndexError(format!("Failed to write entries offset: {}", e))
        })?;

        file.write_all(&metadata_offset.to_le_bytes())
            .map_err(|e| {
                IronBaseError::IndexError(format!("Failed to write metadata offset: {}", e))
            })?;

        // Padding to HEADER_SIZE (64 - 8 - 4 - 8 - 8 - 8 = 28 bytes)
        let padding = vec![0u8; 28];
        file.write_all(&padding)
            .map_err(|e| IronBaseError::IndexError(format!("Failed to write padding: {}", e)))?;

        // Write entries
        file.write_all(&entries_data)
            .map_err(|e| IronBaseError::IndexError(format!("Failed to write entries: {}", e)))?;

        // Write metadata
        file.write_all(&metadata_json)
            .map_err(|e| IronBaseError::IndexError(format!("Failed to write metadata: {}", e)))?;

        // Fsync for durability
        file.sync_all()
            .map_err(|e| IronBaseError::IndexError(format!("Failed to sync fuzzy index: {}", e)))?;

        Ok(())
    }

    /// Save index to file (public API, calls flush)
    pub fn save_to_file(&self) -> Result<()> {
        self.flush()
    }

    /// Load fuzzy index from file (.fzidx)
    pub fn load_from_file(path: &std::path::Path) -> Result<Self> {
        let mut file = File::open(path).map_err(|e| {
            IronBaseError::IndexError(format!("Failed to open fuzzy index file: {}", e))
        })?;

        // Read and verify magic bytes
        let mut magic = [0u8; 8];
        file.read_exact(&mut magic)
            .map_err(|e| IronBaseError::IndexError(format!("Failed to read magic bytes: {}", e)))?;

        if &magic != FUZZY_INDEX_MAGIC {
            return Err(IronBaseError::IndexError(
                "Invalid fuzzy index file: bad magic bytes".to_string(),
            ));
        }

        // Read version
        let mut version_bytes = [0u8; 4];
        file.read_exact(&mut version_bytes)
            .map_err(|e| IronBaseError::IndexError(format!("Failed to read version: {}", e)))?;
        let version = u32::from_le_bytes(version_bytes);

        if version > FUZZY_INDEX_VERSION {
            return Err(IronBaseError::IndexError(format!(
                "Unsupported fuzzy index version: {} (max: {})",
                version, FUZZY_INDEX_VERSION
            )));
        }

        // Read entry count
        let mut entry_count_bytes = [0u8; 8];
        file.read_exact(&mut entry_count_bytes)
            .map_err(|e| IronBaseError::IndexError(format!("Failed to read entry count: {}", e)))?;
        let _entry_count = u64::from_le_bytes(entry_count_bytes);

        // Read entries offset
        let mut entries_offset_bytes = [0u8; 8];
        file.read_exact(&mut entries_offset_bytes).map_err(|e| {
            IronBaseError::IndexError(format!("Failed to read entries offset: {}", e))
        })?;
        let entries_offset = u64::from_le_bytes(entries_offset_bytes);

        // Read metadata offset
        let mut metadata_offset_bytes = [0u8; 8];
        file.read_exact(&mut metadata_offset_bytes).map_err(|e| {
            IronBaseError::IndexError(format!("Failed to read metadata offset: {}", e))
        })?;
        let metadata_offset = u64::from_le_bytes(metadata_offset_bytes);

        // Calculate sizes
        let entries_size = metadata_offset - entries_offset;

        let lazy_mode = entries_size > FUZZY_LAZY_LOAD_THRESHOLD_BYTES;
        let mut entries: Vec<(String, String, DocumentId)> = Vec::new();

        if !lazy_mode {
            // Read entries without buffering the entire section in memory
            file.seek(SeekFrom::Start(entries_offset)).map_err(|e| {
                IronBaseError::IndexError(format!("Failed to seek to entries: {}", e))
            })?;

            let mut reader = std::io::Read::by_ref(&mut file).take(entries_size);
            entries = serde_json::from_reader(&mut reader).map_err(|e| {
                IronBaseError::IndexError(format!("Failed to deserialize fuzzy entries: {}", e))
            })?;
        }

        // Read metadata (rest of file)
        file.seek(SeekFrom::Start(metadata_offset))
            .map_err(|e| IronBaseError::IndexError(format!("Failed to seek to metadata: {}", e)))?;

        let mut metadata_data = Vec::new();
        file.read_to_end(&mut metadata_data)
            .map_err(|e| IronBaseError::IndexError(format!("Failed to read metadata: {}", e)))?;

        let metadata: FuzzyIndexMetadata = serde_json::from_slice(&metadata_data).map_err(|e| {
            IronBaseError::IndexError(format!("Failed to deserialize fuzzy metadata: {}", e))
        })?;

        Ok(FuzzyIndex {
            metadata,
            entries,
            storage_path: Some(path.to_path_buf()),
            lazy_mode,
            entries_offset,
            entries_size,
        })
    }

    fn ensure_entries_loaded(&mut self) -> Result<()> {
        if !self.lazy_mode || !self.entries.is_empty() {
            return Ok(());
        }

        let path = self.storage_path.as_ref().ok_or_else(|| {
            IronBaseError::IndexError("No storage path set for fuzzy index".to_string())
        })?;
        let mut file = File::open(path).map_err(|e| {
            IronBaseError::IndexError(format!("Failed to open fuzzy index file: {}", e))
        })?;
        file.seek(SeekFrom::Start(self.entries_offset))
            .map_err(|e| IronBaseError::IndexError(format!("Failed to seek to entries: {}", e)))?;

        let mut reader = std::io::Read::by_ref(&mut file).take(self.entries_size);
        self.entries = serde_json::from_reader(&mut reader).map_err(|e| {
            IronBaseError::IndexError(format!("Failed to deserialize fuzzy entries: {}", e))
        })?;
        self.metadata.num_entries = self.entries.len();
        self.lazy_mode = false;

        Ok(())
    }

    fn search_in_file(
        &self,
        path: &std::path::Path,
        query_lower: &str,
        threshold: f64,
    ) -> Result<Vec<(DocumentId, f64)>> {
        use serde::de::{Deserializer, SeqAccess, Visitor};

        let mut file = File::open(path).map_err(|e| {
            IronBaseError::IndexError(format!("Failed to open fuzzy index file: {}", e))
        })?;
        file.seek(SeekFrom::Start(self.entries_offset))
            .map_err(|e| IronBaseError::IndexError(format!("Failed to seek to entries: {}", e)))?;

        let mut results = Vec::new();
        let mut reader = std::io::Read::by_ref(&mut file).take(self.entries_size);
        let mut de = serde_json::Deserializer::from_reader(&mut reader);

        struct EntryVisitor<'a> {
            query_lower: &'a str,
            threshold: f64,
            algorithm: FuzzyAlgorithm,
            results: &'a mut Vec<(DocumentId, f64)>,
        }

        impl<'de, 'a> Visitor<'de> for EntryVisitor<'a> {
            type Value = ();

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a sequence of fuzzy index entries")
            }

            fn visit_seq<A>(self, mut seq: A) -> std::result::Result<(), A::Error>
            where
                A: SeqAccess<'de>,
            {
                while let Some((lower, _original, doc_id)) =
                    seq.next_element::<(String, String, DocumentId)>()?
                {
                    let similarity = self.algorithm.similarity(self.query_lower, &lower);
                    if similarity >= self.threshold {
                        self.results.push((doc_id, similarity));
                    }
                }
                Ok(())
            }
        }

        let visitor = EntryVisitor {
            query_lower,
            threshold,
            algorithm: self.metadata.algorithm,
            results: &mut results,
        };

        de.deserialize_seq(visitor).map_err(|e| {
            IronBaseError::IndexError(format!("Failed to stream fuzzy entries: {}", e))
        })?;

        Ok(results)
    }
}

// ============================================================================
// Extended Search API (fulltext_search_ext-consistent)
// ============================================================================

use serde_json::Value;
use std::collections::HashMap as StdHashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

/// Options for extended fuzzy search (consistent with FulltextSearchOptions)
///
/// # Example
/// ```rust,ignore
/// use ironbase_core::FuzzySearchOptions;
/// use serde_json::json;
///
/// let options = FuzzySearchOptions::new()
///     .with_limit(10)
///     .with_threshold(0.75)
///     .with_filter(json!({"status": "active"}))
///     .with_projection(HashMap::from([("name".to_string(), 1)]));
/// ```
#[derive(Debug, Clone, Default)]
pub struct FuzzySearchOptions {
    /// Algorithm to use (default: from index metadata)
    pub algorithm: Option<FuzzyAlgorithm>,
    /// Minimum similarity threshold (0.0-1.0, default: from index metadata)
    pub threshold: Option<f64>,
    /// Maximum results to return (default: 10)
    pub limit: Option<usize>,
    /// Results to skip for pagination
    pub skip: Option<usize>,
    /// Field projection (include/exclude): {"field": 1} or {"field": 0}
    pub projection: Option<StdHashMap<String, i32>>,
    /// MongoDB-style post-filter applied to fuzzy results
    /// Example: {"status": "active", "category": {"$in": ["A", "B"]}}
    pub filter: Option<Value>,
    /// Enable highlight of matched value (default: false)
    pub highlight: bool,
    /// Cancellation flag for cooperative cancellation
    pub cancel_flag: Option<Arc<AtomicBool>>,
    /// Deadline for timeout support
    pub deadline: Option<Instant>,
}

impl FuzzySearchOptions {
    /// Create new options with defaults
    pub fn new() -> Self {
        Self::default()
    }

    /// Set algorithm override
    pub fn with_algorithm(mut self, algorithm: FuzzyAlgorithm) -> Self {
        self.algorithm = Some(algorithm);
        self
    }

    /// Set threshold override
    pub fn with_threshold(mut self, threshold: f64) -> Self {
        self.threshold = Some(threshold.clamp(0.0, 1.0));
        self
    }

    /// Set result limit
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Set skip for pagination
    pub fn with_skip(mut self, skip: usize) -> Self {
        self.skip = Some(skip);
        self
    }

    /// Set field projection
    pub fn with_projection(mut self, projection: StdHashMap<String, i32>) -> Self {
        self.projection = Some(projection);
        self
    }

    /// Set MongoDB-style post-filter
    pub fn with_filter(mut self, filter: Value) -> Self {
        self.filter = Some(filter);
        self
    }

    /// Enable highlight
    pub fn with_highlight(mut self) -> Self {
        self.highlight = true;
        self
    }

    /// Set cancellation flag
    pub fn with_cancel_flag(mut self, flag: Arc<AtomicBool>) -> Self {
        self.cancel_flag = Some(flag);
        self
    }

    /// Set deadline
    pub fn with_deadline(mut self, deadline: Instant) -> Self {
        self.deadline = Some(deadline);
        self
    }
}

/// Result from extended fuzzy search (consistent with FulltextSearchResultExt)
#[derive(Debug, Clone)]
pub struct FuzzySearchResult {
    /// The matched document (with projection applied if specified)
    pub document: Value,
    /// Similarity score (0.0-1.0)
    pub score: f64,
    /// The original value that matched the query
    pub matched_value: String,
    /// Optional highlight showing the matched value with <mark> tags
    pub highlight: Option<String>,
}
