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
        match self {
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
            },
            entries: Vec::new(),
            storage_path: None,
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
            },
            entries: Vec::new(),
            storage_path: Some(storage_path),
        }
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
        self.entries.len()
    }

    /// Insert a value into the fuzzy index
    pub fn insert(&mut self, value: &str, doc_id: DocumentId) {
        let lower = value.to_lowercase();
        self.entries.push((lower, value.to_string(), doc_id));
        self.metadata.num_entries = self.entries.len();
    }

    /// Remove a document from the fuzzy index
    pub fn remove(&mut self, doc_id: &DocumentId) {
        self.entries.retain(|(_, _, id)| id != doc_id);
        self.metadata.num_entries = self.entries.len();
    }

    /// Remove a specific value-document pair
    pub fn remove_value(&mut self, value: &str, doc_id: &DocumentId) {
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

        // Read entries
        file.seek(SeekFrom::Start(entries_offset))
            .map_err(|e| IronBaseError::IndexError(format!("Failed to seek to entries: {}", e)))?;

        let mut entries_data = vec![0u8; entries_size as usize];
        file.read_exact(&mut entries_data)
            .map_err(|e| IronBaseError::IndexError(format!("Failed to read entries: {}", e)))?;

        let entries: Vec<(String, String, DocumentId)> = serde_json::from_slice(&entries_data)
            .map_err(|e| {
                IronBaseError::IndexError(format!("Failed to deserialize fuzzy entries: {}", e))
            })?;

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
        })
    }
}
