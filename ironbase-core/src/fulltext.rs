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
// Type Aliases
// ============================================================================

/// Token entry with term frequency: (document_id, term_frequency)
/// Used in V3 format where TF is embedded directly in the inverted index
pub type TokenEntry = (DocumentId, u32);

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
const FTIDX_VERSION_V1: u32 = 1;
const FTIDX_VERSION_V2: u32 = 2; // Lazy loading support
const FTIDX_VERSION_V3: u32 = 3; // TF embedded in inverted index entries
const FTIDX_HEADER_SIZE: u64 = 64;

// Header layout (64 bytes):
// V1: magic(8) + version(4) + doc_count(8) + offsets_offset(8) + inverted_offset(8) + metadata_offset(8) + padding(20)
// V2: magic(8) + version(4) + doc_count(8) + offsets_offset(8) + token_entries_offset(8) + token_offsets_offset(8) + metadata_offset(8) + padding(12)

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
    /// Inverted index: token -> list of (doc_id, term_frequency) entries (IN MEMORY - fast lookup)
    /// V3 format: TF is embedded directly, eliminating disk I/O during search
    /// In lazy mode, this only contains tokens inserted since last flush
    inverted_index: HashMap<String, Vec<TokenEntry>>,
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

    // === Lazy Loading Support (v2/v3 format) ===
    /// Token offsets table: token -> (file_offset, doc_count)
    /// Used for lazy loading - only loaded at startup, actual entries loaded on demand
    token_offsets: HashMap<String, (u64, u32)>,
    /// Lazy loading mode flag
    /// When true: inverted_index only contains new inserts, token_offsets used for disk lookup
    /// When false: inverted_index contains all data (backward compatible mode)
    lazy_mode: bool,
    /// File format version (2 or 3). Used to determine how to read token entries.
    /// V2: token -> HashSet<DocumentId> (TF loaded separately from doc_tokens)
    /// V3: token -> Vec<(DocumentId, u32)> (TF embedded in inverted index)
    file_version: u32,
    /// Deleted document IDs (tracked for lazy mode)
    /// In lazy mode, removed docs can't be deleted from disk immediately,
    /// so we track them here and filter them out during search.
    deleted_doc_ids: HashSet<DocumentId>,
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
            token_offsets: HashMap::new(),
            lazy_mode: false,
            file_version: FTIDX_VERSION_V3, // New indexes use V3 format
            deleted_doc_ids: HashSet::new(),
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
            token_offsets: HashMap::new(),
            lazy_mode: false,
            file_version: FTIDX_VERSION_V3, // New indexes use V3 format
            deleted_doc_ids: HashSet::new(),
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

        // Write V2 header
        writer.write_all(FTIDX_MAGIC)?;
        writer.write_all(&FTIDX_VERSION_V2.to_le_bytes())?;
        writer.write_all(&0u64.to_le_bytes())?; // doc_count (placeholder)
        writer.write_all(&0u64.to_le_bytes())?; // offsets_offset (placeholder)
        writer.write_all(&0u64.to_le_bytes())?; // token_entries_offset (placeholder)
        writer.write_all(&0u64.to_le_bytes())?; // token_offsets_offset (placeholder)
        writer.write_all(&0u64.to_le_bytes())?; // metadata_offset (placeholder)
        writer.write_all(&[0u8; 12])?; // padding (64 - 8 - 4 - 8*5 = 12)

        writer.flush()?;

        self.write_offset = FTIDX_HEADER_SIZE;
        self.file_handle = Some(
            writer
                .into_inner()
                .map_err(|e| std::io::Error::other(e.to_string()))?,
        );

        Ok(())
    }

    /// Open the storage file for read/write without truncating
    ///
    /// Used when flushing an index that was loaded from disk.
    fn open_storage_file_rw(&mut self) -> Result<()> {
        let path = self
            .storage_path
            .as_ref()
            .ok_or_else(|| IronBaseError::IndexError("No storage path set".to_string()))?;

        let file = OpenOptions::new().read(true).write(true).open(path)?;

        self.file_handle = Some(file);
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

    // =========================================================================
    // Lazy Loading: Token-Level Disk I/O (V2 Format)
    // =========================================================================

    /// Write a single token's doc_ids to disk (for lazy loading)
    ///
    /// Format: [token_len:u32][token:bytes][doc_ids_len:u32][doc_ids:json]
    #[allow(dead_code)]
    fn write_token_entry(&mut self, token: &str, doc_ids: &HashSet<DocumentId>) -> Result<u64> {
        let file = self.file_handle.as_mut().ok_or_else(|| {
            IronBaseError::IndexError("No file handle for disk storage".to_string())
        })?;

        let offset = self.write_offset;
        file.seek(SeekFrom::Start(offset))?;

        // Serialize token
        let token_bytes = token.as_bytes();
        let token_len = token_bytes.len() as u32;

        // Serialize doc_ids as JSON (preserves DocumentId type info)
        let doc_ids_bytes = serde_json::to_vec(doc_ids)?;
        let doc_ids_len = doc_ids_bytes.len() as u32;

        // Write: token_len + token + doc_ids_len + doc_ids
        file.write_all(&token_len.to_le_bytes())?;
        file.write_all(token_bytes)?;
        file.write_all(&doc_ids_len.to_le_bytes())?;
        file.write_all(&doc_ids_bytes)?;

        self.write_offset = offset + 4 + token_bytes.len() as u64 + 4 + doc_ids_bytes.len() as u64;

        Ok(offset)
    }

    /// Read a single token's doc_ids from disk (for lazy loading)
    fn read_token_entry(&self, offset: u64) -> Result<HashSet<DocumentId>> {
        let path = self
            .storage_path
            .as_ref()
            .ok_or_else(|| IronBaseError::IndexError("No storage path set".to_string()))?;

        let mut file = File::open(path)?;
        file.seek(SeekFrom::Start(offset))?;

        let mut reader = BufReader::new(file);
        let mut len_buf = [0u8; 4];

        // Read and skip token (we already know which token we're looking for)
        reader.read_exact(&mut len_buf)?;
        let token_len = u32::from_le_bytes(len_buf) as usize;
        let mut token_buf = vec![0u8; token_len];
        reader.read_exact(&mut token_buf)?;

        // Read doc_ids
        reader.read_exact(&mut len_buf)?;
        let doc_ids_len = u32::from_le_bytes(len_buf) as usize;
        let mut doc_ids_buf = vec![0u8; doc_ids_len];
        reader.read_exact(&mut doc_ids_buf)?;

        let doc_ids: HashSet<DocumentId> = serde_json::from_slice(&doc_ids_buf)?;

        Ok(doc_ids)
    }

    /// Write a single token's entries to disk (V3 format with TF embedded)
    ///
    /// Format: [token_len:u32][token:bytes][entries_len:u32][entries:json]
    /// where entries is Vec<(DocumentId, u32)> - (doc_id, term_frequency)
    #[allow(dead_code)]
    fn write_token_entry_v3(&mut self, token: &str, entries: &[TokenEntry]) -> Result<u64> {
        let file = self.file_handle.as_mut().ok_or_else(|| {
            IronBaseError::IndexError("No file handle for disk storage".to_string())
        })?;

        let offset = self.write_offset;
        file.seek(SeekFrom::Start(offset))?;

        // Serialize token
        let token_bytes = token.as_bytes();
        let token_len = token_bytes.len() as u32;

        // Serialize entries as JSON: Vec<(DocumentId, u32)>
        let entries_bytes = serde_json::to_vec(entries)?;
        let entries_len = entries_bytes.len() as u32;

        // Write: token_len + token + entries_len + entries
        file.write_all(&token_len.to_le_bytes())?;
        file.write_all(token_bytes)?;
        file.write_all(&entries_len.to_le_bytes())?;
        file.write_all(&entries_bytes)?;

        self.write_offset = offset + 4 + token_bytes.len() as u64 + 4 + entries_bytes.len() as u64;

        Ok(offset)
    }

    /// Read a single token's entries from disk (V3 format with TF embedded)
    fn read_token_entry_v3(&self, offset: u64) -> Result<Vec<TokenEntry>> {
        let path = self
            .storage_path
            .as_ref()
            .ok_or_else(|| IronBaseError::IndexError("No storage path set".to_string()))?;

        let mut file = File::open(path)?;
        file.seek(SeekFrom::Start(offset))?;

        let mut reader = BufReader::new(file);
        let mut len_buf = [0u8; 4];

        // Read and skip token (we already know which token we're looking for)
        reader.read_exact(&mut len_buf)?;
        let token_len = u32::from_le_bytes(len_buf) as usize;
        let mut token_buf = vec![0u8; token_len];
        reader.read_exact(&mut token_buf)?;

        // Read entries: Vec<(DocumentId, u32)>
        reader.read_exact(&mut len_buf)?;
        let entries_len = u32::from_le_bytes(len_buf) as usize;
        let mut entries_buf = vec![0u8; entries_len];
        reader.read_exact(&mut entries_buf)?;

        let entries: Vec<TokenEntry> = serde_json::from_slice(&entries_buf)?;

        Ok(entries)
    }

    /// Get token entries from memory only (V3 format: doc_id + TF)
    ///
    /// This is the core lazy loading method:
    /// 1. First checks in-memory inverted_index (new inserts since last flush)
    /// 2. If not found and lazy_mode is active, loads from disk via token_offsets
    /// 3. Returns None if token doesn't exist anywhere
    #[allow(dead_code)]
    fn get_token_entries(&self, token: &str) -> Option<Vec<TokenEntry>> {
        // First check in-memory (for inserts since last flush, or non-lazy mode)
        if let Some(entries) = self.inverted_index.get(token) {
            return Some(entries.clone());
        }

        // If lazy mode, load from disk (V2 format: HashSet → Vec with TF=1)
        if self.lazy_mode {
            if let Some((offset, _count)) = self.token_offsets.get(token) {
                if let Ok(doc_ids) = self.read_token_entry(*offset) {
                    // Convert V2 HashSet to V3 Vec<TokenEntry> with TF=1 (placeholder)
                    return Some(doc_ids.into_iter().map(|id| (id, 1)).collect());
                }
            }
        }

        None
    }

    /// Get token entries, merging memory and disk data (V3 format: doc_id + TF)
    ///
    /// In lazy mode, a token may have entries in both:
    /// - token_offsets (from previous flush, on disk - V2 or V3 format)
    /// - inverted_index (new inserts since last flush, in memory - V3 format)
    ///
    /// This method merges both sources.
    /// V3 format: TF is embedded in disk entries (no placeholder needed)
    /// V2 format: TF=1 placeholder (backward compatible)
    fn get_token_entries_merged(&self, token: &str) -> Option<Vec<TokenEntry>> {
        let mem_entries = self.inverted_index.get(token);
        let disk_entries: Option<Vec<TokenEntry>> = if self.lazy_mode {
            self.token_offsets.get(token).and_then(|(offset, _)| {
                if self.file_version >= FTIDX_VERSION_V3 {
                    // V3: Read entries with TF embedded
                    self.read_token_entry_v3(*offset).ok()
                } else {
                    // V2: Convert HashSet to Vec<TokenEntry> with TF=1 placeholder
                    self.read_token_entry(*offset)
                        .ok()
                        .map(|doc_ids| doc_ids.into_iter().map(|id| (id, 1)).collect())
                }
            })
        } else {
            None
        };

        let result = match (mem_entries, disk_entries) {
            (Some(mem), Some(disk)) => {
                // Merge with deduplication: memory entries take priority (have accurate TF)
                let mem_doc_ids: std::collections::HashSet<_> =
                    mem.iter().map(|(id, _)| id.clone()).collect();
                let mut merged = mem.clone();
                // Only add disk entries for doc_ids NOT already in memory
                for (doc_id, tf) in disk {
                    if !mem_doc_ids.contains(&doc_id) {
                        merged.push((doc_id, tf));
                    }
                }
                Some(merged)
            }
            (Some(mem), None) => Some(mem.clone()),
            (None, Some(disk)) => Some(disk),
            (None, None) => None,
        };

        // Filter out deleted documents (important for lazy mode correctness)
        result.map(|entries| {
            if self.deleted_doc_ids.is_empty() {
                entries
            } else {
                entries
                    .into_iter()
                    .filter(|(doc_id, _)| !self.deleted_doc_ids.contains(doc_id))
                    .collect()
            }
        })
    }

    /// Check if a token exists in the index (memory or disk)
    #[allow(dead_code)]
    fn has_token(&self, token: &str) -> bool {
        self.inverted_index.contains_key(token)
            || (self.lazy_mode && self.token_offsets.contains_key(token))
    }

    /// Get total unique token count (memory + disk, may overlap)
    #[allow(dead_code)]
    fn total_token_count(&self) -> usize {
        if self.lazy_mode {
            // Union of memory and disk tokens
            let mem_tokens: std::collections::HashSet<&str> =
                self.inverted_index.keys().map(|s| s.as_str()).collect();
            let disk_tokens: std::collections::HashSet<&str> =
                self.token_offsets.keys().map(|s| s.as_str()).collect();
            mem_tokens.union(&disk_tokens).count()
        } else {
            self.inverted_index.len()
        }
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

        // Add to inverted index with TF (V3 format: doc_id + term_frequency)
        for (token, count) in &token_counts {
            self.inverted_index
                .entry(token.clone())
                .or_default()
                .push((doc_id.clone(), *count));
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

        // Track deleted doc_id for lazy mode filtering
        // This ensures the doc won't appear in search results even if still on disk
        if self.lazy_mode {
            self.deleted_doc_ids.insert(doc_id.clone());
        }

        // Update inverted_index (V3: Vec.retain instead of HashSet.remove)
        if let Some(tokens) = token_counts {
            for token in tokens.keys() {
                if let Some(entries) = self.inverted_index.get_mut(token) {
                    entries.retain(|(id, _)| id != doc_id);
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

        // V3 Optimized Search: TF is embedded in inverted index, NO disk I/O for TF!
        //
        // Phase 1+2 Combined: Get candidates WITH TF in one pass
        // - get_token_entries_merged returns Vec<(DocumentId, TF)>
        // - Calculate IDF and accumulate scores directly
        // - NO read_doc_tokens_from_disk calls needed!

        let mut doc_scores: HashMap<DocumentId, f64> = HashMap::new();
        let mut matched: HashMap<DocumentId, Vec<String>> = HashMap::new();

        for token in &query_tokens {
            if let Some(entries) = self.get_token_entries_merged(token) {
                // IDF = log(1 + total_docs / docs_with_token)
                let idf = (1.0 + total_docs / entries.len() as f64).ln();

                for (doc_id, tf) in entries {
                    // TF-IDF score: TF comes directly from inverted index (V3 format)
                    *doc_scores.entry(doc_id.clone()).or_default() += (tf as f64) * idf;

                    matched
                        .entry(doc_id.clone())
                        .or_default()
                        .push(token.clone());
                }
            }
        }

        if doc_scores.is_empty() {
            return Vec::new();
        }

        // Old Phase 2 code removed - TF now comes from inverted index entries!
        // This eliminates the 7GB disk I/O for doc_tokens during search.
        let scores = doc_scores;

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
            // Primary: score descending
            // Secondary: doc_id ascending (for deterministic ordering when scores are equal)
            match b.score.partial_cmp(&a.score) {
                Some(std::cmp::Ordering::Equal) | None => a.doc_id.cmp(&b.doc_id),
                Some(ord) => ord,
            }
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

    /// Flush and finalize the index file (V3 format with TF embedded)
    ///
    /// V3 format writes token entries with TF embedded: token -> Vec<(DocumentId, u32)>
    /// After flush, inverted_index is cleared and token_offsets is populated.
    pub fn flush(&mut self) -> Result<()> {
        if self.storage_path.is_none() {
            return Ok(());
        }

        // Open file handle if not already open (e.g., index was loaded from disk)
        if self.file_handle.is_none() {
            self.open_storage_file_rw()?;
        }

        // Step 1: Collect all token data BEFORE we start writing
        // This avoids borrow checker issues with read_token_entry during file writes
        let mut all_tokens: HashMap<String, Vec<TokenEntry>> = HashMap::new();

        // IMPORTANT: Memory entries (new) take priority over disk entries (old)
        // Add memory entries FIRST (they have accurate/updated TF values)
        for (token, entries) in &self.inverted_index {
            all_tokens.insert(token.clone(), entries.clone());
        }

        // Add tokens from disk (if in lazy mode with existing token_offsets)
        // Only add doc_ids that are NOT already in memory (to preserve updated TF)
        if self.lazy_mode {
            for (token, (offset, _count)) in &self.token_offsets {
                // Read using appropriate format based on file_version
                let disk_entries = if self.file_version >= FTIDX_VERSION_V3 {
                    self.read_token_entry_v3(*offset).ok()
                } else {
                    // V2 format: convert HashSet to Vec<TokenEntry> with TF=1
                    self.read_token_entry(*offset)
                        .ok()
                        .map(|doc_ids| doc_ids.into_iter().map(|id| (id, 1u32)).collect())
                };
                if let Some(disk_entries) = disk_entries {
                    let merged = all_tokens.entry(token.clone()).or_default();
                    // Only add disk entries for doc_ids NOT already in memory AND not deleted
                    let existing_doc_ids: std::collections::HashSet<_> =
                        merged.iter().map(|(id, _)| id.clone()).collect();
                    for (doc_id, tf) in disk_entries {
                        if !existing_doc_ids.contains(&doc_id)
                            && !self.deleted_doc_ids.contains(&doc_id)
                        {
                            merged.push((doc_id, tf));
                        }
                    }
                }
            }
        }

        // Step 2: Now start writing - get file handle
        let file = self.file_handle.as_mut().unwrap();
        file.flush()?;

        // Write doc_tokens offset table (as Vec to preserve DocumentId type)
        let offsets_offset = self.write_offset;
        let offsets_vec: Vec<(&DocumentId, &u64)> = self.doc_tokens_offsets.iter().collect();
        let offsets_bytes = serde_json::to_vec(&offsets_vec)?;
        file.seek(SeekFrom::Start(offsets_offset))?;
        file.write_all(&offsets_bytes)?;

        // V3: Write each token entry with TF embedded and build token_offsets table
        let token_entries_offset = offsets_offset + offsets_bytes.len() as u64;
        let mut current_offset = token_entries_offset;
        let mut new_token_offsets: HashMap<String, (u64, u32)> = HashMap::new();

        for (token, entries) in &all_tokens {
            // Write token entry with TF (V3 format)
            let token_bytes = token.as_bytes();
            let entries_bytes = serde_json::to_vec(entries)?;

            file.seek(SeekFrom::Start(current_offset))?;
            file.write_all(&(token_bytes.len() as u32).to_le_bytes())?;
            file.write_all(token_bytes)?;
            file.write_all(&(entries_bytes.len() as u32).to_le_bytes())?;
            file.write_all(&entries_bytes)?;

            new_token_offsets.insert(token.clone(), (current_offset, entries.len() as u32));
            current_offset += 4 + token_bytes.len() as u64 + 4 + entries_bytes.len() as u64;
        }

        // Write token_offsets table
        let token_offsets_offset = current_offset;
        let token_offsets_vec: Vec<(&String, &(u64, u32))> = new_token_offsets.iter().collect();
        let token_offsets_bytes = serde_json::to_vec(&token_offsets_vec)?;
        file.seek(SeekFrom::Start(token_offsets_offset))?;
        file.write_all(&token_offsets_bytes)?;

        // Write metadata (name, field, options)
        let metadata_offset = token_offsets_offset + token_offsets_bytes.len() as u64;
        let metadata = FulltextIndexMetadataForSave {
            name: self.name.clone(),
            field: self.field.clone(),
            options: self.options.clone(),
        };
        let metadata_bytes = serde_json::to_vec(&metadata)?;
        file.write_all(&metadata_bytes)?;

        // Update V3 header with final offsets
        file.seek(SeekFrom::Start(8))?; // After magic
        file.write_all(&FTIDX_VERSION_V3.to_le_bytes())?;
        file.write_all(&(self.doc_tokens_offsets.len() as u64).to_le_bytes())?;
        file.write_all(&offsets_offset.to_le_bytes())?;
        file.write_all(&token_entries_offset.to_le_bytes())?;
        file.write_all(&token_offsets_offset.to_le_bytes())?;
        file.write_all(&metadata_offset.to_le_bytes())?;

        file.flush()?;
        file.sync_all()?;

        // Step 3: Switch to lazy mode with V3 format
        self.write_offset = metadata_offset + metadata_bytes.len() as u64;
        self.token_offsets = new_token_offsets;
        self.inverted_index.clear();
        self.lazy_mode = true;
        self.file_version = FTIDX_VERSION_V3; // Upgraded to V3
        self.deleted_doc_ids.clear(); // Deleted docs are now permanently removed from disk

        Ok(())
    }

    /// Save index to file (for persistence)
    pub fn save_to_file(&mut self) -> Result<()> {
        self.flush()
    }

    /// Load index from file (supports both V1 and V2 formats)
    ///
    /// V1: Loads entire inverted_index into memory (backward compatible)
    /// V2: Loads only token_offsets table, lazy loads tokens on demand
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

        if version != FTIDX_VERSION_V1 && version != FTIDX_VERSION_V2 && version != FTIDX_VERSION_V3
        {
            return Err(IronBaseError::IndexError(format!(
                "Unsupported .ftidx version: {} (supported: 1, 2, 3)",
                version
            )));
        }

        file.read_exact(&mut buf8)?;
        let _doc_count = u64::from_le_bytes(buf8);

        file.read_exact(&mut buf8)?;
        let offsets_offset = u64::from_le_bytes(buf8);

        if version == FTIDX_VERSION_V1 {
            // V1 format: inverted_offset, metadata_offset
            file.read_exact(&mut buf8)?;
            let inverted_offset = u64::from_le_bytes(buf8);

            file.read_exact(&mut buf8)?;
            let metadata_offset = u64::from_le_bytes(buf8);

            // Read offset table
            file.seek(SeekFrom::Start(offsets_offset))?;
            let offsets_size = (inverted_offset - offsets_offset) as usize;
            let mut offsets_buf = vec![0u8; offsets_size];
            file.read_exact(&mut offsets_buf)?;
            let offsets_vec: Vec<(DocumentId, u64)> = serde_json::from_slice(&offsets_buf)?;
            let doc_tokens_offsets: HashMap<DocumentId, u64> = offsets_vec.into_iter().collect();

            // Read inverted index (entire blob - V1 behavior)
            file.seek(SeekFrom::Start(inverted_offset))?;
            let inverted_size = (metadata_offset - inverted_offset) as usize;
            let mut inverted_buf = vec![0u8; inverted_size];
            file.read_exact(&mut inverted_buf)?;
            // V1 format stored HashSet<DocumentId>, convert to V3 Vec<TokenEntry> with TF=1
            let v1_index: HashMap<String, HashSet<DocumentId>> =
                serde_json::from_slice(&inverted_buf)?;
            let inverted_index: HashMap<String, Vec<TokenEntry>> = v1_index
                .into_iter()
                .map(|(token, doc_ids)| (token, doc_ids.into_iter().map(|id| (id, 1)).collect()))
                .collect();

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
                write_offset: offsets_offset,
                file_handle: None,
                doc_tokens_memory: HashMap::new(),
                token_offsets: HashMap::new(),
                lazy_mode: false,               // V1: full in-memory mode
                file_version: FTIDX_VERSION_V1, // Will upgrade to V3 on next flush
                deleted_doc_ids: HashSet::new(),
            })
        } else {
            // V2/V3 format: token_entries_offset, token_offsets_offset, metadata_offset
            file.read_exact(&mut buf8)?;
            let token_entries_offset = u64::from_le_bytes(buf8);

            file.read_exact(&mut buf8)?;
            let token_offsets_offset = u64::from_le_bytes(buf8);

            file.read_exact(&mut buf8)?;
            let metadata_offset = u64::from_le_bytes(buf8);

            // Read doc_tokens offset table
            file.seek(SeekFrom::Start(offsets_offset))?;
            let offsets_size = (token_entries_offset - offsets_offset) as usize;
            let mut offsets_buf = vec![0u8; offsets_size];
            file.read_exact(&mut offsets_buf)?;
            let offsets_vec: Vec<(DocumentId, u64)> = serde_json::from_slice(&offsets_buf)?;
            let doc_tokens_offsets: HashMap<DocumentId, u64> = offsets_vec.into_iter().collect();

            // Read token_offsets table (small, stays in memory)
            file.seek(SeekFrom::Start(token_offsets_offset))?;
            let token_offsets_size = (metadata_offset - token_offsets_offset) as usize;
            let mut token_offsets_buf = vec![0u8; token_offsets_size];
            file.read_exact(&mut token_offsets_buf)?;
            let token_offsets_vec: Vec<(String, (u64, u32))> =
                serde_json::from_slice(&token_offsets_buf)?;
            let token_offsets: HashMap<String, (u64, u32)> =
                token_offsets_vec.into_iter().collect();

            // Read metadata
            file.seek(SeekFrom::Start(metadata_offset))?;
            let metadata_reader = BufReader::new(&file);
            let metadata: FulltextIndexMetadataForSave = serde_json::from_reader(metadata_reader)?;

            Ok(FulltextIndex {
                name: metadata.name,
                field: metadata.field,
                options: metadata.options,
                inverted_index: HashMap::new(), // V2/V3: empty, use lazy loading
                storage_path: Some(path),
                doc_tokens_offsets,
                write_offset: offsets_offset,
                file_handle: None,
                doc_tokens_memory: HashMap::new(),
                token_offsets,
                lazy_mode: true,       // V2/V3: lazy loading mode
                file_version: version, // V2 will upgrade to V3 on next flush, V3 stays V3
                deleted_doc_ids: HashSet::new(),
            })
        }
    }

    /// Get storage path
    pub fn storage_path(&self) -> Option<&PathBuf> {
        self.storage_path.as_ref()
    }

    /// Check if lazy loading mode is active
    ///
    /// Returns true if the index was loaded from a V2 file and is using
    /// on-demand token loading from disk.
    pub fn is_lazy_mode(&self) -> bool {
        self.lazy_mode
    }

    /// Get the number of unique tokens in the index
    ///
    /// This returns the total count from both in-memory and lazy-loaded sources.
    pub fn unique_token_count(&self) -> usize {
        if self.lazy_mode {
            self.token_offsets.len() + self.inverted_index.len()
        } else {
            self.inverted_index.len()
        }
    }

    /// Get memory usage estimate in bytes (for monitoring)
    pub fn memory_usage_bytes(&self) -> usize {
        // inverted_index: each entry is ~100 bytes on average (token + HashSet overhead)
        let inverted_mem = self.inverted_index.len() * 100;
        // token_offsets: each entry is ~32 bytes (String + (u64, u32))
        let offsets_mem = self.token_offsets.len() * 32;
        // doc_tokens_offsets: each entry is ~24 bytes
        let doc_offsets_mem = self.doc_tokens_offsets.len() * 24;
        // doc_tokens_memory: variable, estimate ~200 bytes per doc
        let doc_tokens_mem = self.doc_tokens_memory.len() * 200;

        inverted_mem + offsets_mem + doc_offsets_mem + doc_tokens_mem
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

    // =========================================================================
    // Lazy Loading Tests (V2 format)
    // =========================================================================

    #[test]
    fn test_lazy_mode_after_save_load() {
        let options = FtsOptions::new(FtsLanguage::English);
        let temp_path = std::env::temp_dir().join("fts_test_lazy_mode.ftidx");

        // Create, populate, and save
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
            index.insert(&DocumentId::Int(3), "A fast red cat").unwrap();

            // Before save: not in lazy mode
            assert!(!index.is_lazy_mode());
            assert!(index.unique_token_count() > 0);

            index.save_to_file().unwrap();

            // After save: switched to lazy mode
            assert!(index.is_lazy_mode());
        }

        // Load and verify lazy mode is active
        {
            let loaded = FulltextIndex::load_from_file(temp_path.clone()).unwrap();

            // V2 format: lazy mode should be active
            assert!(loaded.is_lazy_mode());
            assert_eq!(loaded.len(), 3);

            // Token count should be from token_offsets (lazy source)
            assert!(loaded.unique_token_count() > 0);
        }

        let _ = std::fs::remove_file(&temp_path);
    }

    #[test]
    fn test_lazy_mode_search_works() {
        let options = FtsOptions::new(FtsLanguage::English);
        let temp_path = std::env::temp_dir().join("fts_test_lazy_search.ftidx");

        // Create and save
        {
            let mut index = FulltextIndex::new_with_storage(
                "test_idx",
                "content",
                options.clone(),
                temp_path.clone(),
            )
            .unwrap();

            index
                .insert(&DocumentId::Int(1), "The quick brown fox jumps")
                .unwrap();
            index
                .insert(&DocumentId::Int(2), "The lazy brown dog sleeps")
                .unwrap();
            index
                .insert(&DocumentId::Int(3), "A quick red fox runs")
                .unwrap();

            index.save_to_file().unwrap();
        }

        // Load in lazy mode and search
        {
            let loaded = FulltextIndex::load_from_file(temp_path.clone()).unwrap();
            assert!(loaded.is_lazy_mode());

            // Single term search
            let results = loaded.search("fox", 10, 0, None);
            assert_eq!(results.len(), 2);
            let doc_ids: Vec<_> = results.iter().map(|r| &r.doc_id).collect();
            assert!(doc_ids.contains(&&DocumentId::Int(1)));
            assert!(doc_ids.contains(&&DocumentId::Int(3)));

            // Multi-term search
            let results = loaded.search("brown dog", 10, 0, None);
            assert_eq!(results.len(), 2); // Both docs have "brown"

            // Term only in one doc
            let results = loaded.search("sleeps", 10, 0, None);
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].doc_id, DocumentId::Int(2));
        }

        let _ = std::fs::remove_file(&temp_path);
    }

    #[test]
    fn test_lazy_mode_insert_after_load() {
        let options = FtsOptions::new(FtsLanguage::English);
        let temp_path = std::env::temp_dir().join("fts_test_lazy_insert.ftidx");

        // Create initial index
        {
            let mut index = FulltextIndex::new_with_storage(
                "test_idx",
                "content",
                options.clone(),
                temp_path.clone(),
            )
            .unwrap();

            index
                .insert(&DocumentId::Int(1), "The original document")
                .unwrap();
            index.save_to_file().unwrap();
        }

        // Load, insert new doc, search should find both
        {
            let mut loaded = FulltextIndex::load_from_file(temp_path.clone()).unwrap();
            assert!(loaded.is_lazy_mode());

            // Insert new document (goes to in-memory inverted_index)
            loaded
                .insert(&DocumentId::Int(2), "A new document added")
                .unwrap();

            // Search for "document" should find both (disk + memory merged)
            let results = loaded.search("document", 10, 0, None);
            assert_eq!(results.len(), 2);

            // Search for "original" (disk only)
            let results = loaded.search("original", 10, 0, None);
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].doc_id, DocumentId::Int(1));

            // Search for "new" (memory only)
            let results = loaded.search("new", 10, 0, None);
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].doc_id, DocumentId::Int(2));
        }

        let _ = std::fs::remove_file(&temp_path);
    }

    #[test]
    fn test_lazy_mode_memory_reduction() {
        let options = FtsOptions::new(FtsLanguage::English);
        let temp_path = std::env::temp_dir().join("fts_test_lazy_memory.ftidx");

        // Create index with many tokens
        {
            let mut index = FulltextIndex::new_with_storage(
                "test_idx",
                "content",
                options.clone(),
                temp_path.clone(),
            )
            .unwrap();

            // Insert documents with varied vocabulary
            for i in 0..100 {
                let content = format!(
                    "Document {} contains word{} and term{} with unique{} vocabulary{}",
                    i, i, i, i, i
                );
                index.insert(&DocumentId::Int(i), &content).unwrap();
            }

            let initial_token_count = index.token_count();
            assert!(initial_token_count > 400); // Should have many unique tokens

            index.save_to_file().unwrap();
        }

        // Load in lazy mode - inverted_index should be empty
        {
            let loaded = FulltextIndex::load_from_file(temp_path.clone()).unwrap();
            assert!(loaded.is_lazy_mode());

            // inverted_index is empty (accessed via token_count which uses inverted_index)
            assert_eq!(loaded.token_count(), 0);

            // But unique_token_count includes lazy tokens
            assert!(loaded.unique_token_count() > 0);

            // Search still works
            let results = loaded.search("document", 10, 0, None);
            assert!(!results.is_empty());
        }

        let _ = std::fs::remove_file(&temp_path);
    }
}
