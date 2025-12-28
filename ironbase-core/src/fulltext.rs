//! Full-Text Search Module
//!
//! Provides language-aware full-text search with:
//! - Tokenization with accent folding
//! - Stop words filtering (Hungarian, English, German)
//! - Stemming support via rust-stemmers
//! - TF-IDF scoring for relevance ranking
//!
//! # Example
//! ```rust,ignore
//! let options = FtsOptions::new(FtsLanguage::Hungarian);
//! let mut index = FulltextIndex::new("articles_content_fts", "content", options);
//! index.insert(doc_id, "Az autó nagyon szép és gyors");
//! let results = index.search("autók", 10);
//! ```

use crate::document::DocumentId;
use crate::error::{IronBaseError, Result};
use rust_stemmers::{Algorithm, Stemmer};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use unicode_normalization::UnicodeNormalization;

// ============================================================================
// Stop Words
// ============================================================================

pub mod stop_words {
    /// Hungarian stop words (~60 words)
    pub const HUNGARIAN: &[&str] = &[
        "a", "az", "egy", "be", "ki", "le", "fel", "meg", "el", "is", "sem", "de", "hogy", "ha",
        "vagy", "mint", "csak", "nem", "igen", "van", "volt", "lesz", "lett", "már", "még", "majd",
        "után", "alatt", "között", "által", "szerint", "miatt", "helyett", "nélkül", "felé",
        "ellen", "iránt", "körül", "előtt", "mögött", "mellett", "fölött", "alá", "elé", "közé",
        "mögé", "mellé", "fölé", "ez", "az", "ezt", "azt", "aki", "ami", "amely", "és", "én", "te",
        "ő", "mi", "ti", "ők", "nekem", "neked", "neki", "nekünk", "nektek", "nekik",
    ];

    /// English stop words (~175 words)
    pub const ENGLISH: &[&str] = &[
        "a",
        "an",
        "the",
        "and",
        "or",
        "but",
        "if",
        "then",
        "else",
        "when",
        "at",
        "from",
        "by",
        "for",
        "with",
        "about",
        "against",
        "between",
        "into",
        "through",
        "during",
        "before",
        "after",
        "above",
        "below",
        "to",
        "of",
        "in",
        "on",
        "off",
        "over",
        "under",
        "again",
        "further",
        "once",
        "here",
        "there",
        "where",
        "why",
        "how",
        "all",
        "each",
        "few",
        "more",
        "most",
        "other",
        "some",
        "such",
        "no",
        "nor",
        "not",
        "only",
        "own",
        "same",
        "so",
        "than",
        "too",
        "very",
        "can",
        "will",
        "just",
        "should",
        "now",
        "i",
        "me",
        "my",
        "myself",
        "we",
        "our",
        "ours",
        "ourselves",
        "you",
        "your",
        "yours",
        "yourself",
        "yourselves",
        "he",
        "him",
        "his",
        "himself",
        "she",
        "her",
        "hers",
        "herself",
        "it",
        "its",
        "itself",
        "they",
        "them",
        "their",
        "theirs",
        "themselves",
        "what",
        "which",
        "who",
        "whom",
        "this",
        "that",
        "these",
        "those",
        "am",
        "is",
        "are",
        "was",
        "were",
        "be",
        "been",
        "being",
        "have",
        "has",
        "had",
        "having",
        "do",
        "does",
        "did",
        "doing",
        "would",
        "could",
        "ought",
        "as",
        "until",
        "while",
        "because",
        "although",
        "though",
        "both",
        "either",
        "neither",
        "any",
        "every",
    ];

    /// German stop words (~120 words)
    pub const GERMAN: &[&str] = &[
        "der", "die", "das", "den", "dem", "des", "ein", "eine", "einer", "einem", "einen",
        "eines", "und", "oder", "aber", "denn", "weil", "dass", "ob", "wenn", "als", "wie", "so",
        "auch", "nur", "noch", "schon", "immer", "wieder", "hier", "dort", "wo", "wann", "warum",
        "was", "wer", "wen", "wem", "wessen", "welcher", "welche", "welches", "ich", "du", "er",
        "sie", "es", "wir", "ihr", "mein", "dein", "sein", "unser", "euer", "ist", "sind", "war",
        "waren", "wird", "werden", "wurde", "wurden", "hat", "haben", "hatte", "hatten", "kann",
        "können", "konnte", "konnten", "muss", "müssen", "musste", "mussten", "soll", "sollen",
        "sollte", "sollten", "will", "wollen", "wollte", "wollten", "darf", "dürfen", "durfte",
        "durften", "mag", "mögen", "mochte", "mochten", "nicht", "kein", "keine", "keiner",
        "keinem", "keinen", "ja", "nein", "mit", "bei", "nach", "von", "zu", "aus", "in", "an",
        "auf", "für", "über", "unter", "vor", "hinter", "neben", "zwischen", "durch", "gegen",
        "ohne", "um",
    ];
}

// ============================================================================
// Language Configuration
// ============================================================================

/// Supported languages for full-text search
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FtsLanguage {
    /// Hungarian language with stemming and stop words
    Hungarian,
    /// English language with stemming and stop words
    English,
    /// German language with stemming and stop words
    German,
    /// No language processing (just tokenization and accent folding)
    None,
}

impl FtsLanguage {
    /// Parse language from string (case-insensitive)
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "hungarian" | "hu" | "hun" => FtsLanguage::Hungarian,
            "english" | "en" | "eng" => FtsLanguage::English,
            "german" | "de" | "deu" | "ger" => FtsLanguage::German,
            _ => FtsLanguage::None,
        }
    }

    /// Get the stemmer algorithm for this language
    pub fn stemmer_algorithm(&self) -> Option<Algorithm> {
        match self {
            FtsLanguage::Hungarian => Some(Algorithm::Hungarian),
            FtsLanguage::English => Some(Algorithm::English),
            FtsLanguage::German => Some(Algorithm::German),
            FtsLanguage::None => None,
        }
    }

    /// Get stop words for this language
    pub fn stop_words(&self) -> &'static [&'static str] {
        match self {
            FtsLanguage::Hungarian => stop_words::HUNGARIAN,
            FtsLanguage::English => stop_words::ENGLISH,
            FtsLanguage::German => stop_words::GERMAN,
            FtsLanguage::None => &[],
        }
    }
}

impl std::fmt::Display for FtsLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FtsLanguage::Hungarian => write!(f, "hungarian"),
            FtsLanguage::English => write!(f, "english"),
            FtsLanguage::German => write!(f, "german"),
            FtsLanguage::None => write!(f, "none"),
        }
    }
}

// ============================================================================
// Full-Text Search Options
// ============================================================================

/// Configuration options for full-text indexing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FtsOptions {
    /// Language for stemming and stop words
    pub language: FtsLanguage,
    /// Minimum word length to index (default: 2)
    pub min_word_length: usize,
    /// Whether to apply accent folding (default: true)
    pub accent_folding: bool,
}

impl FtsOptions {
    /// Create options for a specific language with defaults
    pub fn new(language: FtsLanguage) -> Self {
        FtsOptions {
            language,
            min_word_length: 2,
            accent_folding: true,
        }
    }

    /// Create options with custom settings
    pub fn with_settings(
        language: FtsLanguage,
        min_word_length: usize,
        accent_folding: bool,
    ) -> Self {
        FtsOptions {
            language,
            min_word_length: min_word_length.max(1),
            accent_folding,
        }
    }
}

impl Default for FtsOptions {
    fn default() -> Self {
        FtsOptions::new(FtsLanguage::None)
    }
}

// ============================================================================
// Accent Folding
// ============================================================================

/// Fold accented characters to their ASCII equivalents
fn fold_accent(c: char) -> char {
    match c {
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
        'Á' | 'À' | 'Â' | 'Ä' | 'Ã' | 'Å' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'É' | 'È' | 'Ê' | 'Ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'Í' | 'Ì' | 'Î' | 'Ï' => 'i',
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'ő' | 'ø' => 'o',
        'Ó' | 'Ò' | 'Ô' | 'Ö' | 'Õ' | 'Ő' | 'Ø' => 'o',
        'ú' | 'ù' | 'û' | 'ü' | 'ű' => 'u',
        'Ú' | 'Ù' | 'Û' | 'Ü' | 'Ű' => 'u',
        'ý' | 'ÿ' => 'y',
        'Ý' | 'Ÿ' => 'y',
        'ñ' => 'n',
        'Ñ' => 'n',
        'ç' => 'c',
        'Ç' => 'c',
        // Note: ß -> ss is handled separately in fold_accents() since it expands to 2 chars
        'æ' => 'a',
        'Æ' => 'a',
        'œ' => 'o',
        'Œ' => 'o',
        _ => c,
    }
}

/// Check if a character is a Unicode combining mark (diacritical mark)
fn is_combining_mark(c: char) -> bool {
    // Unicode combining marks are in range U+0300 to U+036F (Combining Diacritical Marks)
    // and U+1AB0 to U+1AFF, U+1DC0 to U+1DFF, U+20D0 to U+20FF, U+FE20 to U+FE2F
    matches!(c,
        '\u{0300}'..='\u{036F}' |  // Combining Diacritical Marks
        '\u{1AB0}'..='\u{1AFF}' |  // Combining Diacritical Marks Extended
        '\u{1DC0}'..='\u{1DFF}' |  // Combining Diacritical Marks Supplement
        '\u{20D0}'..='\u{20FF}' |  // Combining Diacritical Marks for Symbols
        '\u{FE20}'..='\u{FE2F}'    // Combining Half Marks
    )
}

/// Apply accent folding to entire string
fn fold_accents(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    for c in text.nfd() {
        if is_combining_mark(c) {
            continue; // Skip combining diacritical marks
        }
        // Special handling for ß -> ss (eszett)
        if c == 'ß' {
            result.push_str("ss");
        } else {
            result.push(fold_accent(c));
        }
    }
    result
}

// ============================================================================
// Tokenization
// ============================================================================

/// Tokenize text according to FTS options
///
/// Pipeline:
/// 1. NFD normalization (separates accents)
/// 2. Accent folding (á → a)
/// 3. Lowercase
/// 4. Split on whitespace
/// 5. Strip punctuation
/// 6. Filter by min length
/// 7. Filter stop words (if language set)
/// 8. Apply stemming (if language set)
pub fn tokenize(text: &str, options: &FtsOptions) -> Vec<String> {
    let stop_words: HashSet<&str> = options.language.stop_words().iter().copied().collect();
    let stemmer = options.language.stemmer_algorithm().map(Stemmer::create);

    // Apply accent folding if enabled
    let processed = if options.accent_folding {
        fold_accents(text)
    } else {
        text.to_string()
    };

    processed
        .to_lowercase()
        .split_whitespace()
        .map(|s| s.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|s| !s.is_empty())
        .filter(|s| s.chars().count() >= options.min_word_length)
        .filter(|s| !stop_words.contains(*s))
        .map(|s| {
            stemmer
                .as_ref()
                .map(|st| st.stem(s).to_string())
                .unwrap_or_else(|| s.to_string())
        })
        .collect()
}

/// Tokenize without stemming (for getting display tokens)
pub fn tokenize_raw(text: &str, options: &FtsOptions) -> Vec<String> {
    let stop_words: HashSet<&str> = options.language.stop_words().iter().copied().collect();

    let processed = if options.accent_folding {
        fold_accents(text)
    } else {
        text.to_string()
    };

    processed
        .to_lowercase()
        .split_whitespace()
        .map(|s| s.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|s| !s.is_empty())
        .filter(|s| s.chars().count() >= options.min_word_length)
        .filter(|s| !stop_words.contains(*s))
        .map(|s| s.to_string())
        .collect()
}

// ============================================================================
// Full-Text Index Metadata
// ============================================================================

/// Metadata for a fulltext index (for listing/serialization)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FulltextIndexMetadata {
    pub name: String,
    pub field: String,
    pub language: FtsLanguage,
    pub min_word_length: usize,
    pub accent_folding: bool,
    pub num_documents: usize,
    pub num_tokens: usize,
}

/// Internal metadata structure for saving to .ftidx file
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FulltextIndexMetadataForSave {
    name: String,
    field: String,
    options: FtsOptions,
}

// ============================================================================
// Full-Text Index
// ============================================================================

// ============================================================================
// Fulltext Index File Format (.ftidx)
// ============================================================================
//
// Header (64 bytes):
//   - magic: "IRONFTX\0" (8 bytes)
//   - version: u32 (4 bytes)
//   - doc_count: u64 (8 bytes)
//   - offsets_offset: u64 (8 bytes) - where offset table starts
//   - inverted_offset: u64 (8 bytes) - where inverted index starts
//   - metadata_offset: u64 (8 bytes) - where index metadata starts
//   - padding: 20 bytes
//
// Doc Tokens Data (variable):
//   For each document:
//     - doc_id length: u32 (4 bytes)
//     - doc_id bytes: variable
//     - tokens JSON length: u32 (4 bytes)
//     - tokens JSON bytes: variable (HashMap<String, u32> serialized)
//
// Offset Table (JSON):
//   HashMap<DocumentId, u64> - maps doc_id to file offset
//
// Inverted Index (JSON):
//   HashMap<String, HashSet<DocumentId>> - token to doc_ids mapping
//
// Metadata (JSON):
//   FulltextIndexMetadataForSave - name, field, options

const FTIDX_MAGIC: &[u8; 8] = b"IRONFTX\0";
const FTIDX_VERSION: u32 = 1;
const FTIDX_HEADER_SIZE: u64 = 64;

/// Full-text search index using inverted index with TF-IDF scoring
///
/// # Performance Characteristics
/// - Insert: O(t) where t = tokens in document
/// - Search: O(q * d) where q = query tokens, d = matching docs
/// - Storage: Disk-based doc_tokens, memory-only inverted_index
///
/// # Disk-Based Storage
/// The `doc_tokens` (85% of memory usage) is stored on disk in `.ftidx` files.
/// Only the `inverted_index` (15%) stays in memory for fast token→doc_id lookup.
/// During search, doc_tokens are loaded lazily only for candidate documents.
///
/// # Example
/// ```rust,ignore
/// let options = FtsOptions::new(FtsLanguage::Hungarian);
/// let mut index = FulltextIndex::new("idx", "content", options);
/// index.insert(&doc_id, "Az autó nagyon szép");
/// let results = index.search("autók", 10, 0, None);
/// ```
pub struct FulltextIndex {
    /// Index name (e.g., "articles_content_fts")
    pub name: String,
    /// Field being indexed
    pub field: String,
    /// Tokenization options
    pub options: FtsOptions,
    /// Inverted index: token -> set of document IDs (IN MEMORY - fast lookup)
    inverted_index: HashMap<String, HashSet<DocumentId>>,
    /// Path to .ftidx file for disk-based doc_tokens storage
    storage_path: Option<PathBuf>,
    /// Offset table: doc_id -> file offset (IN MEMORY - small)
    doc_tokens_offsets: HashMap<DocumentId, u64>,
    /// Current write position in the file
    write_offset: u64,
    /// File handle for writing (kept open for appending)
    file_handle: Option<File>,
    /// Memory-only doc_tokens (used when no storage_path is set)
    /// This is for backward compatibility with tests that don't use disk storage
    doc_tokens_memory: HashMap<DocumentId, HashMap<String, u32>>,
}

/// Search result with score and matched tokens
#[derive(Debug, Clone)]
pub struct FtsSearchResult {
    pub doc_id: DocumentId,
    pub score: f64,
    pub matched_tokens: Vec<String>,
}

impl FulltextIndex {
    /// Create a new fulltext index (memory-only, no disk storage)
    pub fn new(name: &str, field: &str, options: FtsOptions) -> Self {
        FulltextIndex {
            name: name.to_string(),
            field: field.to_string(),
            options,
            inverted_index: HashMap::new(),
            storage_path: None,
            doc_tokens_offsets: HashMap::new(),
            write_offset: FTIDX_HEADER_SIZE,
            file_handle: None,
            doc_tokens_memory: HashMap::new(),
        }
    }

    /// Create a new fulltext index with disk-based storage
    pub fn new_with_storage(
        name: &str,
        field: &str,
        options: FtsOptions,
        path: PathBuf,
    ) -> Result<Self> {
        let mut index = FulltextIndex {
            name: name.to_string(),
            field: field.to_string(),
            options,
            inverted_index: HashMap::new(),
            storage_path: Some(path.clone()),
            doc_tokens_offsets: HashMap::new(),
            write_offset: FTIDX_HEADER_SIZE,
            file_handle: None,
            doc_tokens_memory: HashMap::new(),
        };

        // Create and initialize the file with header
        index.init_storage_file()?;

        Ok(index)
    }

    /// Set storage path for an existing index (enables disk storage)
    pub fn set_storage_path(&mut self, path: PathBuf) -> Result<()> {
        self.storage_path = Some(path);
        self.init_storage_file()
    }

    /// Initialize storage file with header
    fn init_storage_file(&mut self) -> Result<()> {
        let path = self
            .storage_path
            .as_ref()
            .ok_or_else(|| IronBaseError::IndexError("No storage path set".to_string()))?;

        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .read(true)
            .truncate(true)
            .open(path)?;

        let mut writer = BufWriter::new(file);

        // Write header
        writer.write_all(FTIDX_MAGIC)?;
        writer.write_all(&FTIDX_VERSION.to_le_bytes())?;
        writer.write_all(&0u64.to_le_bytes())?; // doc_count (placeholder)
        writer.write_all(&0u64.to_le_bytes())?; // offsets_offset (placeholder)
        writer.write_all(&0u64.to_le_bytes())?; // inverted_offset (placeholder)
        writer.write_all(&0u64.to_le_bytes())?; // metadata_offset (placeholder)
        writer.write_all(&[0u8; 20])?; // padding (64 - 8 - 4 - 8*4 = 20)

        writer.flush()?;

        self.write_offset = FTIDX_HEADER_SIZE;
        self.file_handle = Some(
            writer
                .into_inner()
                .map_err(|e| std::io::Error::other(e.to_string()))?,
        );

        Ok(())
    }

    /// Get metadata for this index
    pub fn metadata(&self) -> FulltextIndexMetadata {
        FulltextIndexMetadata {
            name: self.name.clone(),
            field: self.field.clone(),
            language: self.options.language,
            min_word_length: self.options.min_word_length,
            accent_folding: self.options.accent_folding,
            num_documents: self.doc_tokens_offsets.len(),
            num_tokens: self.inverted_index.len(),
        }
    }

    /// Write doc_tokens to disk and return the file offset
    fn write_doc_tokens_to_disk(
        &mut self,
        doc_id: &DocumentId,
        tokens: &HashMap<String, u32>,
    ) -> Result<u64> {
        let file = self.file_handle.as_mut().ok_or_else(|| {
            IronBaseError::IndexError("No file handle for disk storage".to_string())
        })?;

        let offset = self.write_offset;
        file.seek(SeekFrom::Start(offset))?;

        // Serialize doc_id
        let doc_id_bytes = serde_json::to_vec(doc_id)?;
        let doc_id_len = doc_id_bytes.len() as u32;

        // Serialize tokens
        let tokens_bytes = serde_json::to_vec(tokens)?;
        let tokens_len = tokens_bytes.len() as u32;

        // Write: doc_id_len + doc_id + tokens_len + tokens
        file.write_all(&doc_id_len.to_le_bytes())?;
        file.write_all(&doc_id_bytes)?;
        file.write_all(&tokens_len.to_le_bytes())?;
        file.write_all(&tokens_bytes)?;

        self.write_offset = offset + 4 + doc_id_bytes.len() as u64 + 4 + tokens_bytes.len() as u64;

        Ok(offset)
    }

    /// Read doc_tokens from disk for a specific document
    fn read_doc_tokens_from_disk(&self, doc_id: &DocumentId) -> Result<HashMap<String, u32>> {
        let offset = self.doc_tokens_offsets.get(doc_id).ok_or_else(|| {
            IronBaseError::IndexError(format!("Document {:?} not in index", doc_id))
        })?;

        let path = self
            .storage_path
            .as_ref()
            .ok_or_else(|| IronBaseError::IndexError("No storage path set".to_string()))?;

        let mut file = File::open(path)?;
        file.seek(SeekFrom::Start(*offset))?;

        let mut reader = BufReader::new(file);

        // Read doc_id (skip it, we already know it)
        let mut len_buf = [0u8; 4];
        reader.read_exact(&mut len_buf)?;
        let doc_id_len = u32::from_le_bytes(len_buf) as usize;
        let mut doc_id_buf = vec![0u8; doc_id_len];
        reader.read_exact(&mut doc_id_buf)?;

        // Read tokens
        reader.read_exact(&mut len_buf)?;
        let tokens_len = u32::from_le_bytes(len_buf) as usize;
        let mut tokens_buf = vec![0u8; tokens_len];
        reader.read_exact(&mut tokens_buf)?;

        let tokens: HashMap<String, u32> = serde_json::from_slice(&tokens_buf)?;

        Ok(tokens)
    }

    /// Insert a document's field value into the index
    pub fn insert(&mut self, doc_id: &DocumentId, text: &str) -> Result<()> {
        let tokens = tokenize(text, &self.options);
        if tokens.is_empty() {
            return Ok(());
        }

        // Count token frequencies for this document
        let mut token_counts: HashMap<String, u32> = HashMap::new();
        for token in &tokens {
            *token_counts.entry(token.clone()).or_insert(0) += 1;
        }

        // Add to inverted index (always in memory for fast lookup)
        for token in token_counts.keys() {
            self.inverted_index
                .entry(token.clone())
                .or_default()
                .insert(doc_id.clone());
        }

        // Store token frequencies - disk or memory
        if self.storage_path.is_some() && self.file_handle.is_some() {
            let offset = self.write_doc_tokens_to_disk(doc_id, &token_counts)?;
            self.doc_tokens_offsets.insert(doc_id.clone(), offset);
        } else {
            // Memory-only mode: store in doc_tokens_memory
            self.doc_tokens_memory.insert(doc_id.clone(), token_counts);
            self.doc_tokens_offsets.insert(doc_id.clone(), 0);
        }

        Ok(())
    }

    /// Remove a document from the index
    /// Note: This marks the document as removed but doesn't reclaim disk space.
    /// Disk space is reclaimed during compaction/rebuild.
    pub fn remove(&mut self, doc_id: &DocumentId) -> Result<()> {
        // Get tokens to update inverted_index
        let token_counts = if self.storage_path.is_some() {
            // Disk-based: try to read tokens
            self.read_doc_tokens_from_disk(doc_id).ok()
        } else {
            // Memory-based: get from memory
            self.doc_tokens_memory.remove(doc_id)
        };

        // Remove from offsets
        self.doc_tokens_offsets.remove(doc_id);

        // Update inverted_index
        if let Some(tokens) = token_counts {
            for token in tokens.keys() {
                if let Some(doc_ids) = self.inverted_index.get_mut(token) {
                    doc_ids.remove(doc_id);
                }
            }
        }

        Ok(())
    }

    /// Update a document in the index
    pub fn update(&mut self, doc_id: &DocumentId, text: &str) -> Result<()> {
        self.remove(doc_id)?;
        self.insert(doc_id, text)
    }

    /// Search for documents matching the query using TF-IDF scoring
    ///
    /// # Arguments
    /// * `query` - Search query text
    /// * `limit` - Maximum number of results
    /// * `skip` - Number of results to skip (for pagination)
    /// * `min_score` - Minimum score threshold (None = no threshold)
    ///
    /// # Returns
    /// Vector of search results sorted by score (descending)
    ///
    /// # Disk-Based Search
    /// For disk-based indexes, doc_tokens are loaded lazily only for candidate
    /// documents that match at least one query token. This dramatically reduces
    /// memory usage for large indexes.
    pub fn search(
        &self,
        query: &str,
        limit: usize,
        skip: usize,
        min_score: Option<f64>,
    ) -> Vec<FtsSearchResult> {
        let query_tokens = tokenize(query, &self.options);
        if query_tokens.is_empty() {
            return Vec::new();
        }

        let total_docs = self.doc_tokens_offsets.len() as f64;
        if total_docs == 0.0 {
            return Vec::new();
        }

        // Phase 1: Find candidate documents from inverted_index (memory-only, fast)
        let mut candidate_docs: HashSet<DocumentId> = HashSet::new();
        let mut matched: HashMap<DocumentId, Vec<String>> = HashMap::new();

        for token in &query_tokens {
            if let Some(doc_ids) = self.inverted_index.get(token) {
                for doc_id in doc_ids {
                    candidate_docs.insert(doc_id.clone());
                    matched
                        .entry(doc_id.clone())
                        .or_default()
                        .push(token.clone());
                }
            }
        }

        if candidate_docs.is_empty() {
            return Vec::new();
        }

        // Phase 2: Calculate TF-IDF scores (with lazy loading from disk or memory)
        let mut scores: HashMap<DocumentId, f64> = HashMap::new();

        for doc_id in &candidate_docs {
            // Load doc_tokens: disk-based or memory-based
            let doc_tokens = if self.storage_path.is_some() {
                // Disk-based: load from file
                match self.read_doc_tokens_from_disk(doc_id) {
                    Ok(tokens) => tokens,
                    Err(_) => continue, // Skip documents we can't read
                }
            } else {
                // Memory-based: get from doc_tokens_memory
                match self.doc_tokens_memory.get(doc_id) {
                    Some(tokens) => tokens.clone(),
                    None => continue, // Skip documents not in memory
                }
            };

            let mut doc_score = 0.0;
            for token in &query_tokens {
                if let Some(doc_ids) = self.inverted_index.get(token) {
                    // Smoothed IDF = log(1 + total_docs / docs_with_token)
                    let idf = (1.0 + total_docs / doc_ids.len() as f64).ln();

                    // TF = token count in doc
                    let tf = doc_tokens.get(token).copied().unwrap_or(0) as f64;

                    doc_score += tf * idf;
                }
            }

            if doc_score > 0.0 {
                scores.insert(doc_id.clone(), doc_score);
            }
        }

        // Apply min_score filter
        let min = min_score.unwrap_or(0.0);

        // Sort by score descending
        let mut results: Vec<_> = scores
            .into_iter()
            .filter(|(_, score)| *score >= min)
            .map(|(doc_id, score)| FtsSearchResult {
                matched_tokens: matched.remove(&doc_id).unwrap_or_default(),
                doc_id,
                score,
            })
            .collect();

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Apply skip and limit
        results.into_iter().skip(skip).take(limit).collect()
    }

    /// Check if a document is in the index
    pub fn contains(&self, doc_id: &DocumentId) -> bool {
        self.doc_tokens_offsets.contains_key(doc_id)
    }

    /// Get number of indexed documents
    pub fn len(&self) -> usize {
        self.doc_tokens_offsets.len()
    }

    /// Check if index is empty
    pub fn is_empty(&self) -> bool {
        self.doc_tokens_offsets.is_empty()
    }

    /// Get number of indexed documents
    pub fn doc_count(&self) -> usize {
        self.doc_tokens_offsets.len()
    }

    /// Get number of unique tokens
    pub fn token_count(&self) -> usize {
        self.inverted_index.len()
    }

    /// Clear all entries from the index
    pub fn clear(&mut self) {
        self.inverted_index.clear();
        self.doc_tokens_offsets.clear();
        self.doc_tokens_memory.clear();
        self.write_offset = FTIDX_HEADER_SIZE;
        // Re-initialize storage file if we have one
        if self.storage_path.is_some() {
            let _ = self.init_storage_file();
        }
    }

    /// Flush and finalize the index file
    /// Writes the offset table, inverted index, and metadata to disk
    pub fn flush(&mut self) -> Result<()> {
        if self.storage_path.is_none() || self.file_handle.is_none() {
            return Ok(());
        }

        let file = self.file_handle.as_mut().unwrap();
        file.flush()?;

        // Write offset table (as Vec to preserve DocumentId type)
        let offsets_offset = self.write_offset;
        let offsets_vec: Vec<(&DocumentId, &u64)> = self.doc_tokens_offsets.iter().collect();
        let offsets_bytes = serde_json::to_vec(&offsets_vec)?;
        file.seek(SeekFrom::Start(offsets_offset))?;
        file.write_all(&offsets_bytes)?;

        // Write inverted index
        let inverted_offset = offsets_offset + offsets_bytes.len() as u64;
        let inverted_bytes = serde_json::to_vec(&self.inverted_index)?;
        file.write_all(&inverted_bytes)?;

        // Write metadata (name, field, options)
        let metadata_offset = inverted_offset + inverted_bytes.len() as u64;
        let metadata = FulltextIndexMetadataForSave {
            name: self.name.clone(),
            field: self.field.clone(),
            options: self.options.clone(),
        };
        let metadata_bytes = serde_json::to_vec(&metadata)?;
        file.write_all(&metadata_bytes)?;

        // Update header with final offsets
        file.seek(SeekFrom::Start(8))?; // After magic
        file.write_all(&FTIDX_VERSION.to_le_bytes())?;
        file.write_all(&(self.doc_tokens_offsets.len() as u64).to_le_bytes())?;
        file.write_all(&offsets_offset.to_le_bytes())?;
        file.write_all(&inverted_offset.to_le_bytes())?;
        file.write_all(&metadata_offset.to_le_bytes())?;

        file.flush()?;
        file.sync_all()?;

        Ok(())
    }

    /// Save index to file (for persistence)
    pub fn save_to_file(&mut self) -> Result<()> {
        self.flush()
    }

    /// Load index from file
    pub fn load_from_file(path: PathBuf) -> Result<Self> {
        let mut file = File::open(&path)?;

        // Read and validate header
        let mut magic = [0u8; 8];
        file.read_exact(&mut magic)?;
        if &magic != FTIDX_MAGIC {
            return Err(IronBaseError::IndexError(
                "Invalid .ftidx magic".to_string(),
            ));
        }

        let mut buf4 = [0u8; 4];
        let mut buf8 = [0u8; 8];

        file.read_exact(&mut buf4)?;
        let version = u32::from_le_bytes(buf4);
        if version != FTIDX_VERSION {
            return Err(IronBaseError::IndexError(format!(
                "Unsupported .ftidx version: {}",
                version
            )));
        }

        file.read_exact(&mut buf8)?;
        let _doc_count = u64::from_le_bytes(buf8);

        file.read_exact(&mut buf8)?;
        let offsets_offset = u64::from_le_bytes(buf8);

        file.read_exact(&mut buf8)?;
        let inverted_offset = u64::from_le_bytes(buf8);

        file.read_exact(&mut buf8)?;
        let metadata_offset = u64::from_le_bytes(buf8);

        // Read offset table (stored as Vec to preserve DocumentId type)
        file.seek(SeekFrom::Start(offsets_offset))?;
        let offsets_size = (inverted_offset - offsets_offset) as usize;
        let mut offsets_buf = vec![0u8; offsets_size];
        file.read_exact(&mut offsets_buf)?;
        let offsets_vec: Vec<(DocumentId, u64)> = serde_json::from_slice(&offsets_buf)?;
        let doc_tokens_offsets: HashMap<DocumentId, u64> = offsets_vec.into_iter().collect();

        // Read inverted index
        file.seek(SeekFrom::Start(inverted_offset))?;
        let inverted_size = (metadata_offset - inverted_offset) as usize;
        let mut inverted_buf = vec![0u8; inverted_size];
        file.read_exact(&mut inverted_buf)?;
        let inverted_index: HashMap<String, HashSet<DocumentId>> =
            serde_json::from_slice(&inverted_buf)?;

        // Read metadata
        file.seek(SeekFrom::Start(metadata_offset))?;
        let metadata_reader = BufReader::new(&file);
        let metadata: FulltextIndexMetadataForSave = serde_json::from_reader(metadata_reader)?;

        Ok(FulltextIndex {
            name: metadata.name,
            field: metadata.field,
            options: metadata.options,
            inverted_index,
            storage_path: Some(path),
            doc_tokens_offsets,
            write_offset: offsets_offset, // New data would go before offset table
            file_handle: None,            // Opened read-only, need to reopen for writes
            doc_tokens_memory: HashMap::new(), // Disk-based, not used
        })
    }

    /// Get storage path
    pub fn storage_path(&self) -> Option<&PathBuf> {
        self.storage_path.as_ref()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_accent_folding() {
        assert_eq!(
            fold_accents("Árvíztűrő tükörfúrógép"),
            "Arvizturo tukorfurogep"
        );
        assert_eq!(fold_accents("café résumé naïve"), "cafe resume naive");
        assert_eq!(fold_accents("Größe Müller"), "Grosse Muller");
        assert_eq!(fold_accents("hello world"), "hello world");
    }

    #[test]
    fn test_tokenize_basic() {
        let options = FtsOptions::new(FtsLanguage::None);
        let tokens = tokenize("Hello World! This is a test.", &options);
        assert_eq!(tokens, vec!["hello", "world", "this", "is", "test"]);
    }

    #[test]
    fn test_tokenize_with_accents() {
        let options = FtsOptions::new(FtsLanguage::None);
        let tokens = tokenize("Árvíztűrő tükörfúrógép", &options);
        assert_eq!(tokens, vec!["arvizturo", "tukorfurogep"]);
    }

    #[test]
    fn test_tokenize_min_length() {
        let options = FtsOptions::with_settings(FtsLanguage::None, 3, true);
        let tokens = tokenize("I am a big dog", &options);
        assert_eq!(tokens, vec!["big", "dog"]);
    }

    #[test]
    fn test_stop_words_hungarian() {
        let options = FtsOptions::new(FtsLanguage::Hungarian);
        let tokens = tokenize("Ez egy nagyon szép ház", &options);
        // "ez", "egy" are stop words
        assert!(!tokens.contains(&"ez".to_string()));
        assert!(!tokens.contains(&"egy".to_string()));
        assert!(tokens
            .iter()
            .any(|t| t.starts_with("nagy") || t.starts_with("szep") || t.starts_with("haz")));
    }

    #[test]
    fn test_stop_words_english() {
        let options = FtsOptions::new(FtsLanguage::English);
        let tokens = tokenize("The quick brown fox jumps over the lazy dog", &options);
        // "the", "over" are stop words
        assert!(!tokens.contains(&"the".to_string()));
        assert!(!tokens.contains(&"over".to_string()));
        assert!(tokens.len() >= 4); // quick, brown, fox, jump*, lazi*, dog
    }

    #[test]
    fn test_stemming_english() {
        let options = FtsOptions::new(FtsLanguage::English);
        let tokens1 = tokenize("running", &options);
        let tokens2 = tokenize("runs", &options);
        let tokens3 = tokenize("run", &options);
        // All should stem to "run"
        assert_eq!(tokens1, tokens2);
        assert_eq!(tokens2, tokens3);
    }

    #[test]
    fn test_fulltext_index_insert_search() {
        let options = FtsOptions::new(FtsLanguage::English);
        let temp_dir = std::env::temp_dir().join("fts_test_insert_search.ftidx");
        let mut index =
            FulltextIndex::new_with_storage("test_idx", "content", options, temp_dir.clone())
                .unwrap();

        let doc1 = DocumentId::Int(1);
        let doc2 = DocumentId::Int(2);
        let doc3 = DocumentId::Int(3);

        index.insert(&doc1, "The quick brown fox").unwrap();
        index.insert(&doc2, "The lazy brown dog").unwrap();
        index.insert(&doc3, "A fast red fox").unwrap();

        // Search for "fox"
        let results = index.search("fox", 10, 0, None);
        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|r| r.doc_id == doc1));
        assert!(results.iter().any(|r| r.doc_id == doc3));

        // Search for "brown"
        let results = index.search("brown", 10, 0, None);
        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|r| r.doc_id == doc1));
        assert!(results.iter().any(|r| r.doc_id == doc2));

        // Cleanup
        let _ = std::fs::remove_file(&temp_dir);
    }

    #[test]
    fn test_fulltext_index_tfidf_scoring() {
        let options = FtsOptions::new(FtsLanguage::None);
        let temp_dir = std::env::temp_dir().join("fts_test_tfidf.ftidx");
        let mut index =
            FulltextIndex::new_with_storage("test_idx", "content", options, temp_dir.clone())
                .unwrap();

        let doc1 = DocumentId::Int(1);
        let doc2 = DocumentId::Int(2);

        // doc1 has "fox" twice, doc2 has it once
        index.insert(&doc1, "fox fox dog").unwrap();
        index.insert(&doc2, "fox cat").unwrap();

        let results = index.search("fox", 10, 0, None);
        assert_eq!(results.len(), 2);
        // doc1 should have higher score (more "fox" occurrences)
        assert!(results[0].doc_id == doc1);
        assert!(results[0].score > results[1].score);

        // Cleanup
        let _ = std::fs::remove_file(&temp_dir);
    }

    #[test]
    fn test_fulltext_index_remove() {
        let options = FtsOptions::new(FtsLanguage::None);
        let temp_dir = std::env::temp_dir().join("fts_test_remove.ftidx");
        let mut index =
            FulltextIndex::new_with_storage("test_idx", "content", options, temp_dir.clone())
                .unwrap();

        let doc1 = DocumentId::Int(1);
        let doc2 = DocumentId::Int(2);

        index.insert(&doc1, "hello world").unwrap();
        index.insert(&doc2, "hello rust").unwrap();

        assert_eq!(index.len(), 2);

        index.remove(&doc1).unwrap();
        assert_eq!(index.len(), 1);

        let results = index.search("hello", 10, 0, None);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].doc_id, doc2);

        // Cleanup
        let _ = std::fs::remove_file(&temp_dir);
    }

    #[test]
    fn test_fulltext_index_pagination() {
        let options = FtsOptions::new(FtsLanguage::None);
        let temp_dir = std::env::temp_dir().join("fts_test_pagination.ftidx");
        let mut index =
            FulltextIndex::new_with_storage("test_idx", "content", options, temp_dir.clone())
                .unwrap();

        // Create documents with different scores (different term frequencies)
        // Doc 10 has highest score (10x "test"), doc 1 has lowest (1x "test")
        for i in 1..=10 {
            let content = "test ".repeat(i as usize);
            index.insert(&DocumentId::Int(i), &content).unwrap();
        }

        // All documents match "test"
        let all = index.search("test", 100, 0, None);
        assert_eq!(all.len(), 10);

        // First 3 (should be docs 10, 9, 8 - highest scores)
        let first3 = index.search("test", 3, 0, None);
        assert_eq!(first3.len(), 3);

        // Skip 3, take 3 (should be docs 7, 6, 5)
        let next3 = index.search("test", 3, 3, None);
        assert_eq!(next3.len(), 3);

        // Verify pagination works - first3 and next3 should have no overlap
        let first3_ids: std::collections::HashSet<_> = first3.iter().map(|r| &r.doc_id).collect();
        let next3_ids: std::collections::HashSet<_> = next3.iter().map(|r| &r.doc_id).collect();
        assert!(
            first3_ids.is_disjoint(&next3_ids),
            "Paginated results should not overlap"
        );

        // Skip 8, take 10 (should only get 2 remaining)
        let last2 = index.search("test", 10, 8, None);
        assert_eq!(last2.len(), 2);

        // Cleanup
        let _ = std::fs::remove_file(&temp_dir);
    }

    #[test]
    fn test_fulltext_index_min_score() {
        let options = FtsOptions::new(FtsLanguage::None);
        let temp_dir = std::env::temp_dir().join("fts_test_min_score.ftidx");
        let mut index =
            FulltextIndex::new_with_storage("test_idx", "content", options, temp_dir.clone())
                .unwrap();

        let doc1 = DocumentId::Int(1);
        let doc2 = DocumentId::Int(2);

        // doc1 has very high TF for "apple"
        index
            .insert(&doc1, "apple apple apple apple apple")
            .unwrap();
        index.insert(&doc2, "apple banana").unwrap();

        let results = index.search("apple", 10, 0, None);
        assert_eq!(results.len(), 2);

        let high_score = results[0].score;
        // Filter with min_score just below the highest
        let filtered = index.search("apple", 10, 0, Some(high_score - 0.01));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].doc_id, doc1);

        // Cleanup
        let _ = std::fs::remove_file(&temp_dir);
    }

    #[test]
    fn test_fulltext_index_save_load() {
        let options = FtsOptions::new(FtsLanguage::English);
        let temp_path = std::env::temp_dir().join("fts_test_save_load.ftidx");

        // Create and populate index
        {
            let mut index = FulltextIndex::new_with_storage(
                "test_idx",
                "content",
                options.clone(),
                temp_path.clone(),
            )
            .unwrap();

            index
                .insert(&DocumentId::Int(1), "The quick brown fox")
                .unwrap();
            index
                .insert(&DocumentId::Int(2), "The lazy brown dog")
                .unwrap();

            index.save_to_file().unwrap();
        }

        // Load index and verify
        {
            let loaded = FulltextIndex::load_from_file(temp_path.clone()).unwrap();

            assert_eq!(loaded.len(), 2);
            assert_eq!(loaded.options.language, FtsLanguage::English);

            let results = loaded.search("fox", 10, 0, None);
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].doc_id, DocumentId::Int(1));

            let results = loaded.search("brown", 10, 0, None);
            assert_eq!(results.len(), 2);
        }

        // Cleanup
        let _ = std::fs::remove_file(&temp_path);
    }

    #[test]
    fn test_fts_language_from_str() {
        assert_eq!(FtsLanguage::from_str("hungarian"), FtsLanguage::Hungarian);
        assert_eq!(FtsLanguage::from_str("HUNGARIAN"), FtsLanguage::Hungarian);
        assert_eq!(FtsLanguage::from_str("hu"), FtsLanguage::Hungarian);
        assert_eq!(FtsLanguage::from_str("english"), FtsLanguage::English);
        assert_eq!(FtsLanguage::from_str("en"), FtsLanguage::English);
        assert_eq!(FtsLanguage::from_str("german"), FtsLanguage::German);
        assert_eq!(FtsLanguage::from_str("de"), FtsLanguage::German);
        assert_eq!(FtsLanguage::from_str("unknown"), FtsLanguage::None);
    }

    #[test]
    fn test_empty_search() {
        let options = FtsOptions::new(FtsLanguage::None);
        let index = FulltextIndex::new("test_idx", "content", options);

        let results = index.search("anything", 10, 0, None);
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_only_stop_words() {
        let options = FtsOptions::new(FtsLanguage::English);
        let temp_dir = std::env::temp_dir().join("fts_test_stop_words.ftidx");
        let mut index =
            FulltextIndex::new_with_storage("test_idx", "content", options, temp_dir.clone())
                .unwrap();

        index.insert(&DocumentId::Int(1), "Hello world").unwrap();

        // Search with only stop words should return empty
        let results = index.search("the a an", 10, 0, None);
        assert!(results.is_empty());

        // Cleanup
        let _ = std::fs::remove_file(&temp_dir);
    }
}
