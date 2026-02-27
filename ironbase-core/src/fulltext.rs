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
use crate::log_error;
use rust_stemmers::{Algorithm, Stemmer};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::Arc;
use unicode_normalization::UnicodeNormalization;

// ============================================================================
// Type Aliases
// ============================================================================

/// Token entry with term frequency: (document_id, term_frequency)
/// Used in V3 format where TF is embedded directly in the inverted index
pub type TokenEntry = (DocumentId, u32);

// ============================================================================
// Query Parsing (MongoDB-compatible phrase search)
// ============================================================================

/// Parsed query with phrases and regular terms
///
/// MongoDB-style query parsing:
/// - `"exact phrase"` → phrase that must appear exactly
/// - `word1 word2` → terms that are OR-ed (default)
/// - `"phrase one" word "phrase two"` → mixed query
#[derive(Debug, Clone, Default)]
pub struct ParsedQuery {
    /// Exact phrases to match (extracted from quoted strings)
    pub phrases: Vec<String>,
    /// Regular terms to OR-search
    pub terms: Vec<String>,
}

impl ParsedQuery {
    /// Check if query contains any phrases
    pub fn has_phrases(&self) -> bool {
        !self.phrases.is_empty()
    }

    /// Get all terms for initial OR search (phrases tokenized + regular terms)
    pub fn all_search_terms(&self, options: &FtsOptions) -> Vec<String> {
        let mut all_terms = self.terms.clone();
        for phrase in &self.phrases {
            all_terms.extend(tokenize(phrase, options));
        }
        all_terms
    }
}

/// Parse a query string for phrases (MongoDB-compatible syntax)
///
/// Detects quoted phrases like `"exact phrase"` and separates them from
/// regular OR-ed terms.
///
/// # Examples
/// ```rust,ignore
/// let parsed = parse_query("hello world");
/// // phrases: [], terms: ["hello", "world"]
///
/// let parsed = parse_query("\"hello world\"");
/// // phrases: ["hello world"], terms: []
///
/// let parsed = parse_query("\"exact phrase\" other words");
/// // phrases: ["exact phrase"], terms: ["other", "words"]
/// ```
///
/// # Edge Cases
///
/// - **Empty string**: Returns empty ParsedQuery
/// - **Empty quotes** (`""`): Ignored, produces no phrase
/// - **Whitespace-only quotes** (`"   "`): Ignored after trim, produces no phrase
/// - **Unclosed quote** (`"hello`): Rest of query treated as regular terms
/// - **Mixed**: `word "phrase" word2` correctly separates terms and phrases
/// - **Multiple phrases**: `"one" "two"` produces two phrases
///
/// # Note
///
/// This function does NOT apply tokenization or stop-word filtering.
/// The returned phrases/terms are raw strings. Use `ParsedQuery::all_search_terms()`
/// with FtsOptions to get processed tokens for search.
pub fn parse_query(query: &str) -> ParsedQuery {
    let mut result = ParsedQuery::default();
    let mut remaining = query.trim();

    while !remaining.is_empty() {
        // Look for quoted phrase
        if remaining.starts_with('"') {
            // Find closing quote
            if let Some(end_pos) = remaining[1..].find('"') {
                let phrase = remaining[1..=end_pos].trim().to_string();
                if !phrase.is_empty() {
                    result.phrases.push(phrase);
                }
                remaining = remaining[end_pos + 2..].trim();
            } else {
                // No closing quote - treat rest as terms
                for term in remaining[1..].split_whitespace() {
                    if !term.is_empty() {
                        result.terms.push(term.to_string());
                    }
                }
                break;
            }
        } else {
            // Regular term - take until next quote or whitespace
            let next_quote = remaining.find('"').unwrap_or(remaining.len());
            let before_quote = &remaining[..next_quote];

            for term in before_quote.split_whitespace() {
                if !term.is_empty() {
                    result.terms.push(term.to_string());
                }
            }

            remaining = remaining[next_quote..].trim();
        }
    }

    result
}

/// Build a regex pattern for phrase matching
///
/// Creates a case-insensitive regex that matches the exact phrase,
/// allowing flexible whitespace between words.
///
/// # Example
/// ```rust,ignore
/// let regex = build_phrase_regex("hello world")?;
/// assert!(regex.is_match("Hello  World")); // case insensitive, flexible space
/// assert!(!regex.is_match("world hello")); // wrong order
/// ```
pub fn build_phrase_regex(phrase: &str) -> Result<regex::Regex> {
    let pattern = phrase
        .split_whitespace()
        .map(regex::escape)
        .collect::<Vec<_>>()
        .join(r"\s+");

    regex::Regex::new(&format!(r"(?i)\b{}\b", pattern))
        .map_err(|e| IronBaseError::InvalidQuery(format!("Invalid phrase regex: {}", e)))
}

// ============================================================================
// Phrase Regex Cache
// ============================================================================

use std::cell::RefCell;
use std::collections::VecDeque;

/// Maximum number of cached phrase regexes (LRU eviction)
const PHRASE_REGEX_CACHE_SIZE: usize = 64;

thread_local! {
    /// Thread-local LRU cache for compiled phrase regexes
    /// Avoids recompiling the same phrase patterns repeatedly
    static PHRASE_REGEX_CACHE: RefCell<PhraseRegexCache> = RefCell::new(PhraseRegexCache::new());
}

struct PhraseRegexCache {
    /// LRU order: most recently used at front
    order: VecDeque<String>,
    /// Cached compiled regexes
    cache: HashMap<String, regex::Regex>,
}

impl PhraseRegexCache {
    fn new() -> Self {
        Self {
            order: VecDeque::with_capacity(PHRASE_REGEX_CACHE_SIZE),
            cache: HashMap::with_capacity(PHRASE_REGEX_CACHE_SIZE),
        }
    }

    fn get_or_compile(&mut self, phrase: &str) -> Result<regex::Regex> {
        // Check cache hit
        if let Some(regex) = self.cache.get(phrase) {
            // Move to front (most recently used)
            if let Some(pos) = self.order.iter().position(|p| p == phrase) {
                self.order.remove(pos);
                self.order.push_front(phrase.to_string());
            }
            return Ok(regex.clone());
        }

        // Cache miss: compile regex
        let regex = build_phrase_regex(phrase)?;

        // Evict LRU if at capacity
        if self.cache.len() >= PHRASE_REGEX_CACHE_SIZE {
            if let Some(evicted) = self.order.pop_back() {
                self.cache.remove(&evicted);
            }
        }

        // Insert new entry
        self.order.push_front(phrase.to_string());
        self.cache.insert(phrase.to_string(), regex.clone());

        Ok(regex)
    }
}

/// Build a phrase regex with caching (recommended for repeated searches)
///
/// Uses a thread-local LRU cache to avoid recompiling the same phrase patterns.
/// Cache size is limited to 64 entries with LRU eviction.
pub fn build_phrase_regex_cached(phrase: &str) -> Result<regex::Regex> {
    PHRASE_REGEX_CACHE.with(|cache| cache.borrow_mut().get_or_compile(phrase))
}

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
    /// True while index is being built - query planner should ignore this index
    #[serde(default)]
    pub building: bool,
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
    /// True while index is being built - search operations should return empty results
    /// to prevent using partially populated indexes
    pub building: bool,

    // === Two-Phase Flush Support ===
    /// Frozen inverted index snapshot during background flush.
    /// When a flush is in progress, inverted_index is moved here (via Arc) so that
    /// search can still find entries while the flush serializes WITHOUT holding a lock.
    /// Set to None when no flush is in progress.
    frozen_inverted: Option<Arc<HashMap<String, Vec<TokenEntry>>>>,
}

// ============================================================================
// Two-Phase Flush Types
// ============================================================================

/// Snapshot data for two-phase fulltext flush.
///
/// Contains all data needed to serialize the .ftidx file WITHOUT holding
/// the IndexManager write lock. Created by `take_flush_snapshot()`,
/// consumed by `serialize_flush()`.
pub(crate) struct FulltextFlushSnapshot {
    /// Inverted index entries (moved from FulltextIndex via Arc)
    pub inverted: Arc<HashMap<String, Vec<TokenEntry>>>,
    /// Token offsets for lazy mode disk entries (cloned)
    pub token_offsets: HashMap<String, (u64, u32)>,
    /// Document token offsets (cloned)
    pub doc_tokens_offsets: HashMap<DocumentId, u64>,
    /// Deleted document IDs for filtering (cloned)
    pub deleted_doc_ids: HashSet<DocumentId>,
    /// Current write position (start of index data region)
    pub write_offset: u64,
    /// Whether lazy mode was active
    pub lazy_mode: bool,
    /// File format version (for reading old entries)
    pub file_version: u32,
    /// Index metadata
    pub name: String,
    pub field: String,
    pub options: FtsOptions,
    /// Path to .ftidx file
    pub storage_path: PathBuf,
}

/// Result of a two-phase flush, to be committed under write lock.
pub(crate) struct FulltextFlushResult {
    /// New token offsets table (pointing to freshly written data)
    pub new_token_offsets: HashMap<String, (u64, u32)>,
    /// New write offset (end of all written data)
    pub new_write_offset: u64,
}

/// Search result with score and matched tokens
#[derive(Debug, Clone)]
pub struct FtsSearchResult {
    pub doc_id: DocumentId,
    pub score: f64,
    pub matched_tokens: Vec<String>,
}

// ============================================================================
// Highlight Support
// ============================================================================

/// Options for generating highlights/snippets
#[derive(Debug, Clone)]
pub struct HighlightOptions {
    /// Number of characters to include before and after match (default: 100)
    pub context_chars: usize,
    /// Maximum number of snippets to return per field (default: 3)
    pub max_snippets: usize,
    /// Tag to wrap matched terms (default: "<mark>")
    pub highlight_start: String,
    /// Closing tag for matched terms (default: "</mark>")
    pub highlight_end: String,
}

impl Default for HighlightOptions {
    fn default() -> Self {
        Self {
            context_chars: 100,
            max_snippets: 3,
            highlight_start: "<mark>".to_string(),
            highlight_end: "</mark>".to_string(),
        }
    }
}

/// Highlight result for a single field
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HighlightResult {
    /// Field name that was searched
    pub field: String,
    /// Snippets with highlighted matches
    pub snippets: Vec<String>,
}

// ============================================================================
// Extended Fulltext Search API (Core-level options and results)
// ============================================================================

use serde_json::Value;
use std::collections::HashMap as StdHashMap;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

/// Unified fulltext search options - CORE LEVEL
///
/// This struct consolidates all fulltext search parameters including:
/// - Pagination (limit, skip)
/// - Scoring (min_score)
/// - Projection (field filtering)
/// - Post-filter (MongoDB-style query on results)
/// - Highlighting (snippet generation)
/// - Cancellation/timeout support
#[derive(Debug, Clone, Default)]
pub struct FulltextSearchOptions {
    /// Maximum results to return (default: 10)
    pub limit: Option<usize>,
    /// Results to skip for pagination
    pub skip: Option<usize>,
    /// Minimum TF-IDF score threshold
    pub min_score: Option<f64>,
    /// Field projection (include/exclude): {"field": 1} or {"field": 0}
    pub projection: Option<StdHashMap<String, i32>>,
    /// MongoDB-style post-filter applied to fulltext results
    /// Example: {"from.email": {"$regex": "@company\\.com$"}}
    pub filter: Option<Value>,
    /// Enable highlight/snippet generation (default: false)
    pub highlight: bool,
    /// Highlight configuration (context chars, max snippets)
    pub highlight_options: Option<HighlightOptions>,
    /// AND mode: ALL query tokens must match (default: false = OR mode)
    /// When true, only documents containing every query token are returned.
    pub and_mode: bool,
    /// Cancellation flag for cooperative cancellation
    pub cancel_flag: Option<Arc<AtomicBool>>,
    /// Deadline for timeout support
    pub deadline: Option<Instant>,
}

impl FulltextSearchOptions {
    /// Create new options with defaults
    pub fn new() -> Self {
        Self::default()
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

    /// Set minimum score threshold
    pub fn with_min_score(mut self, score: f64) -> Self {
        self.min_score = Some(score);
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

    /// Enable highlighting with custom options
    pub fn with_highlight(mut self, context_chars: usize, max_snippets: usize) -> Self {
        self.highlight = true;
        self.highlight_options = Some(HighlightOptions {
            context_chars,
            max_snippets,
            ..Default::default()
        });
        self
    }

    /// Set cancellation flag
    pub fn with_cancel_flag(mut self, flag: Arc<AtomicBool>) -> Self {
        self.cancel_flag = Some(flag);
        self
    }

    /// Set timeout deadline
    pub fn with_deadline(mut self, deadline: Instant) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// Calculate candidate limit for fulltext search
    ///
    /// When filters or phrase matching is enabled, we request more candidates
    /// from the TF-IDF search to account for post-filtering that may eliminate
    /// some results.
    ///
    /// # Arguments
    /// * `effective_limit` - The actual limit requested by the user
    /// * `has_filter_or_phrase` - Whether post-filtering will be applied
    ///
    /// # Returns
    /// The number of candidates to request from TF-IDF search
    pub fn calculate_candidate_limit(effective_limit: usize, has_filter_or_phrase: bool) -> usize {
        if has_filter_or_phrase {
            // When a filter is present, the relevant documents may appear at ANY
            // position in the TF-IDF ranking. Example: "ajánlat" has 6766 matches,
            // but year=2026 docs are ranked at positions 6735-6766 (lowest TF-IDF).
            // The TF-IDF search is already O(N) internally (scores all matching docs),
            // so a large candidate_limit only affects the output vector size (~100
            // bytes/result). Cap at 100K to prevent pathological cases (~10MB).
            // Document loading in the post-filter loop has early termination.
            100_000
        } else {
            effective_limit
        }
    }
}

/// Extended fulltext search result - CORE LEVEL
///
/// Includes document, score, matched tokens, and optional highlights.
/// This is the return type for `fulltext_search_ext()`.
#[derive(Debug, Clone)]
pub struct FulltextSearchResultExt {
    /// The matched document (with projection applied if specified)
    pub document: Value,
    /// TF-IDF relevance score
    pub score: f64,
    /// Query tokens that matched in the document
    pub matched_tokens: Vec<String>,
    /// Optional highlight snippets (if highlight=true)
    pub highlights: Option<Vec<HighlightResult>>,
}

/// Generate highlighted snippets from text containing matched tokens
///
/// # Arguments
/// * `text` - The source text to extract snippets from
/// * `query_tokens` - Tokens to highlight (stemmed form)
/// * `field` - Field name for the result
/// * `options` - FTS options (for tokenization settings)
/// * `highlight_opts` - Highlight configuration
///
/// # Returns
/// `HighlightResult` with snippets containing `<mark>` tags around matches
pub fn generate_highlights(
    text: &str,
    query_tokens: &[String],
    field: &str,
    options: &FtsOptions,
    highlight_opts: &HighlightOptions,
) -> HighlightResult {
    if text.is_empty() || query_tokens.is_empty() {
        return HighlightResult {
            field: field.to_string(),
            snippets: Vec::new(),
        };
    }

    // Find all match positions in the text
    // We need to match against normalized/stemmed tokens
    let stemmer = options.language.stemmer_algorithm().map(Stemmer::create);
    let query_set: HashSet<&str> = query_tokens.iter().map(|s| s.as_str()).collect();

    // Track match positions: (start_byte, end_byte, original_word)
    let mut matches: Vec<(usize, usize, String)> = Vec::new();

    // Find word positions in original text
    let mut word_start_byte = 0;
    let mut in_word = false;
    let mut current_word = String::new();

    for (i, c) in text.char_indices() {
        let is_word_char = c.is_alphanumeric();

        if is_word_char {
            if !in_word {
                word_start_byte = i;
                in_word = true;
                current_word.clear();
            }
            current_word.push(c);
        } else if in_word {
            // End of word - check if it matches
            let word_end_byte = i;
            let normalized = if options.accent_folding {
                fold_accents(&current_word).to_lowercase()
            } else {
                current_word.to_lowercase()
            };

            // Skip short words
            if normalized.chars().count() >= options.min_word_length {
                // Apply stemming
                let stemmed = stemmer
                    .as_ref()
                    .map(|st| st.stem(&normalized).to_string())
                    .unwrap_or_else(|| normalized.clone());

                // Check if this word matches any query token
                if query_set.contains(stemmed.as_str()) {
                    matches.push((word_start_byte, word_end_byte, current_word.clone()));
                }
            }

            in_word = false;
            current_word.clear();
        }
    }

    // Handle last word if text doesn't end with separator
    if in_word && !current_word.is_empty() {
        let word_end_byte = text.len();
        let normalized = if options.accent_folding {
            fold_accents(&current_word).to_lowercase()
        } else {
            current_word.to_lowercase()
        };

        if normalized.chars().count() >= options.min_word_length {
            let stemmed = stemmer
                .as_ref()
                .map(|st| st.stem(&normalized).to_string())
                .unwrap_or_else(|| normalized.clone());

            if query_set.contains(stemmed.as_str()) {
                matches.push((word_start_byte, word_end_byte, current_word.clone()));
            }
        }
    }

    if matches.is_empty() {
        return HighlightResult {
            field: field.to_string(),
            snippets: Vec::new(),
        };
    }

    // Generate snippets around matches
    let mut snippets: Vec<String> = Vec::new();
    let mut used_ranges: Vec<(usize, usize)> = Vec::new();

    for (match_start, match_end, _original_word) in &matches {
        if snippets.len() >= highlight_opts.max_snippets {
            break;
        }

        // Calculate snippet boundaries (in bytes)
        let raw_start = match_start.saturating_sub(highlight_opts.context_chars);
        let raw_end = (match_end + highlight_opts.context_chars).min(text.len());

        // Adjust to character boundaries using safe iteration
        // Find the largest valid char boundary <= raw_start
        let snippet_start = text
            .char_indices()
            .take_while(|(i, _)| *i <= raw_start)
            .last()
            .map(|(i, _)| i)
            .unwrap_or(0);

        // Find the smallest valid char boundary >= raw_end
        let snippet_end = text
            .char_indices()
            .find(|(i, _)| *i >= raw_end)
            .map(|(i, _)| i)
            .unwrap_or(text.len());

        // Check if this range overlaps with already used ranges
        let overlaps = used_ranges
            .iter()
            .any(|(start, end)| snippet_start < *end && snippet_end > *start);

        if overlaps {
            continue;
        }

        used_ranges.push((snippet_start, snippet_end));

        // Build snippet with highlighting
        let mut snippet = String::new();

        // Add ellipsis if not at start
        if snippet_start > 0 {
            snippet.push_str("...");
        }

        // Find all matches within this snippet range
        let matches_in_range: Vec<_> = matches
            .iter()
            .filter(|(s, e, _)| *s >= snippet_start && *e <= snippet_end)
            .collect();

        let snippet_text = &text[snippet_start..snippet_end];
        let mut last_end = 0;

        for (ms, me, _) in matches_in_range {
            let rel_start = ms - snippet_start;
            let rel_end = me - snippet_start;

            // Add text before match
            if rel_start > last_end {
                snippet.push_str(&snippet_text[last_end..rel_start]);
            }

            // Add highlighted match
            snippet.push_str(&highlight_opts.highlight_start);
            snippet.push_str(&snippet_text[rel_start..rel_end]);
            snippet.push_str(&highlight_opts.highlight_end);

            last_end = rel_end;
        }

        // Add remaining text
        if last_end < snippet_text.len() {
            snippet.push_str(&snippet_text[last_end..]);
        }

        // Add ellipsis if not at end
        if snippet_end < text.len() {
            snippet.push_str("...");
        }

        snippets.push(snippet);
    }

    HighlightResult {
        field: field.to_string(),
        snippets,
    }
}

// ============================================================================
// Text Analysis (Token Debug)
// ============================================================================

/// Analysis result for a single token
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenAnalysis {
    /// Original token as it appears in text
    pub original: String,
    /// After accent folding and lowercase
    pub normalized: String,
    /// After stemming (if language set)
    pub stemmed: String,
    /// Whether the token is kept (not a stop word, meets min length)
    pub kept: bool,
    /// Reason if not kept
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filtered_reason: Option<String>,
}

/// Result of analyzing text for token processing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextAnalysisResult {
    /// Original input text
    pub input: String,
    /// Language used for analysis
    pub language: String,
    /// Whether accent folding was applied
    pub accent_folding: bool,
    /// Minimum word length filter
    pub min_word_length: usize,
    /// Detailed analysis of each token
    pub tokens: Vec<TokenAnalysis>,
    /// List of stop words that were removed
    pub stop_words_removed: Vec<String>,
    /// Final tokens that would be used for indexing/search
    pub final_tokens: Vec<String>,
}

/// Analyze text to show tokenization steps
///
/// This function is useful for debugging and understanding how text
/// is processed for fulltext search. It shows each step of the pipeline:
/// 1. Original word extraction
/// 2. Accent folding and lowercase
/// 3. Stemming
/// 4. Stop word filtering
///
/// # Example
/// ```ignore
/// let result = analyze_text("Kálmán Péter ügyvezető", &FtsOptions::hungarian());
/// // Shows: "Kálmán" -> "kalman" -> "kalman" (kept)
/// //        "Péter" -> "peter" -> "peter" (kept)
/// //        "ügyvezető" -> "ugyvezeto" -> "ugyvezet" (kept)
/// ```
pub fn analyze_text(text: &str, options: &FtsOptions) -> TextAnalysisResult {
    let stop_words: HashSet<&str> = options.language.stop_words().iter().copied().collect();
    let stemmer = options.language.stemmer_algorithm().map(Stemmer::create);

    let mut tokens: Vec<TokenAnalysis> = Vec::new();
    let mut stop_words_removed: Vec<String> = Vec::new();
    let mut final_tokens: Vec<String> = Vec::new();

    // Split into words
    for word in text.split_whitespace() {
        let original = word
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_string();
        if original.is_empty() {
            continue;
        }

        // Normalize: accent fold + lowercase
        let normalized = if options.accent_folding {
            fold_accents(&original).to_lowercase()
        } else {
            original.to_lowercase()
        };

        // Check min length
        if normalized.chars().count() < options.min_word_length {
            tokens.push(TokenAnalysis {
                original: original.clone(),
                normalized: normalized.clone(),
                stemmed: normalized.clone(),
                kept: false,
                filtered_reason: Some(format!(
                    "Too short (min: {} chars)",
                    options.min_word_length
                )),
            });
            continue;
        }

        // Check stop words
        if stop_words.contains(normalized.as_str()) {
            stop_words_removed.push(original.clone());
            tokens.push(TokenAnalysis {
                original: original.clone(),
                normalized: normalized.clone(),
                stemmed: normalized.clone(),
                kept: false,
                filtered_reason: Some("Stop word".to_string()),
            });
            continue;
        }

        // Apply stemming
        let stemmed = stemmer
            .as_ref()
            .map(|st| st.stem(&normalized).to_string())
            .unwrap_or_else(|| normalized.clone());

        tokens.push(TokenAnalysis {
            original,
            normalized,
            stemmed: stemmed.clone(),
            kept: true,
            filtered_reason: None,
        });

        final_tokens.push(stemmed);
    }

    TextAnalysisResult {
        input: text.to_string(),
        language: format!("{:?}", options.language),
        accent_folding: options.accent_folding,
        min_word_length: options.min_word_length,
        tokens,
        stop_words_removed,
        final_tokens,
    }
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
            building: false,
            frozen_inverted: None,
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
            building: false,
            frozen_inverted: None,
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
        // NOTE: If into_inner() fails, BufWriter::drop still flushes to disk,
        // but file_handle remains None. The file exists on disk with valid data,
        // however subsequent appends will fail until next initialize_disk_storage().
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
            num_tokens: self.unique_token_count(),
            building: self.building,
        }
    }

    /// Check if this index is currently being built
    pub fn is_building(&self) -> bool {
        self.building
    }

    /// Set the building flag (for index creation lifecycle)
    pub fn set_building(&mut self, building: bool) {
        self.building = building;
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
    // Lazy Loading: Token-Level Disk I/O
    // =========================================================================
    // Note: Token writing is done inline in flush() for efficiency.
    // Only read functions are needed for lazy loading.

    /// Read a single token's doc_ids from disk (V2 format, for backward compatibility)
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

    /// Read a single token's entries from disk (V3 format with TF embedded)
    ///
    /// Format: [token_len:u32][token:bytes][entries_len:u32][entries:json]
    /// where entries is Vec<(DocumentId, u32)> - (doc_id, term_frequency)
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

    // NOTE: get_token_entries() removed - superseded by get_token_entries_merged()
    // which properly handles merging memory and disk data with deduplication.

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
        // During two-phase flush, frozen_inverted holds entries moved from inverted_index
        let frozen_entries = self.frozen_inverted.as_ref().and_then(|f| f.get(token));
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

        // Merge all sources: mem (new inserts) > frozen (snapshot) > disk (previous flush)
        let mut seen_doc_ids: HashSet<DocumentId> = HashSet::new();
        let mut merged: Vec<TokenEntry> = Vec::new();

        // Memory entries take priority (most recent, accurate TF)
        if let Some(mem) = mem_entries {
            for entry in mem {
                seen_doc_ids.insert(entry.0.clone());
                merged.push(entry.clone());
            }
        }
        // Frozen entries next (snapshot from current flush, also accurate TF)
        if let Some(frozen) = frozen_entries {
            for entry in frozen {
                if !seen_doc_ids.contains(&entry.0) {
                    seen_doc_ids.insert(entry.0.clone());
                    merged.push(entry.clone());
                }
            }
        }
        // Disk entries last (from previous flush)
        if let Some(disk) = disk_entries {
            for (doc_id, tf) in disk {
                if !seen_doc_ids.contains(&doc_id) {
                    merged.push((doc_id, tf));
                }
            }
        }

        if merged.is_empty() {
            return None;
        }

        // Filter out deleted documents (important for lazy mode correctness)
        if self.deleted_doc_ids.is_empty() {
            Some(merged)
        } else {
            let filtered: Vec<_> = merged
                .into_iter()
                .filter(|(doc_id, _)| !self.deleted_doc_ids.contains(doc_id))
                .collect();
            if filtered.is_empty() {
                None
            } else {
                Some(filtered)
            }
        }
    }

    /// Check if a token exists in the index (memory or disk)
    ///
    /// This is a fast existence check that doesn't load the full entry.
    /// Useful for query planning or diagnostics.
    ///
    /// Note: Currently private, but kept for potential future public API use.
    #[allow(dead_code)]
    fn has_token(&self, token: &str) -> bool {
        self.inverted_index.contains_key(token)
            || self
                .frozen_inverted
                .as_ref()
                .is_some_and(|f| f.contains_key(token))
            || (self.lazy_mode && self.token_offsets.contains_key(token))
    }

    // NOTE: total_token_count() removed - superseded by unique_token_count()
    // which correctly computes the union without double-counting.

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

    /// Threshold for warning about unbounded deleted_doc_ids growth
    /// When exceeded, a warning is logged suggesting a flush() call
    const DELETED_DOC_IDS_WARNING_THRESHOLD: usize = 10_000;

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

            // Warn if deleted_doc_ids is growing too large (memory leak potential)
            // User should call flush() periodically to clear this
            if self.deleted_doc_ids.len() == Self::DELETED_DOC_IDS_WARNING_THRESHOLD {
                tracing::warn!(
                    index = %self.name,
                    deleted_count = self.deleted_doc_ids.len(),
                    "Fulltext index deleted_doc_ids set is large. Consider calling flush() to reclaim memory."
                );
            }
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

        results.sort_unstable_by(Self::compare_search_results);

        // Apply skip and limit
        results.into_iter().skip(skip).take(limit).collect()
    }

    /// Search for documents with ExecutionContext for cancellation support
    ///
    /// This is the cancellation-aware version of `search`. It checks the
    /// ExecutionContext during the expensive iterations to allow early termination.
    pub fn search_with_ctx(
        &self,
        query: &str,
        limit: usize,
        skip: usize,
        min_score: Option<f64>,
        ctx: Option<&crate::execution::ExecutionContext>,
    ) -> Result<Vec<FtsSearchResult>> {
        let query_tokens = tokenize(query, &self.options);
        if query_tokens.is_empty() {
            return Ok(Vec::new());
        }

        let total_docs = self.doc_tokens_offsets.len() as f64;
        if total_docs == 0.0 {
            return Ok(Vec::new());
        }

        let mut doc_scores: HashMap<DocumentId, f64> = HashMap::new();
        let mut matched: HashMap<DocumentId, Vec<String>> = HashMap::new();
        let mut iteration: usize = 0;

        for token in &query_tokens {
            // Check cancellation before loading token entries (disk I/O)
            if let Some(exec_ctx) = ctx {
                exec_ctx.maybe_check(iteration)?;
            }
            iteration += 1;

            if let Some(entries) = self.get_token_entries_merged(token) {
                let idf = (1.0 + total_docs / entries.len() as f64).ln();

                for (doc_id, tf) in entries {
                    // Check cancellation periodically during iteration
                    if let Some(exec_ctx) = ctx {
                        exec_ctx.maybe_check(iteration)?;
                    }
                    iteration += 1;

                    *doc_scores.entry(doc_id.clone()).or_default() += (tf as f64) * idf;
                    matched
                        .entry(doc_id.clone())
                        .or_default()
                        .push(token.clone());
                }
            }
        }

        if doc_scores.is_empty() {
            return Ok(Vec::new());
        }

        let min = min_score.unwrap_or(0.0);

        let mut results: Vec<_> = doc_scores
            .into_iter()
            .filter(|(_, score)| *score >= min)
            .map(|(doc_id, score)| FtsSearchResult {
                matched_tokens: matched.remove(&doc_id).unwrap_or_default(),
                doc_id,
                score,
            })
            .collect();

        results.sort_unstable_by(Self::compare_search_results);

        Ok(results.into_iter().skip(skip).take(limit).collect())
    }

    /// Compare two search results for sorting (score descending, doc_id ascending)
    ///
    /// Handles NaN scores by treating them as lowest relevance (sorted to end).
    /// This ensures deterministic ordering even with corrupted/invalid scores.
    fn compare_search_results(a: &FtsSearchResult, b: &FtsSearchResult) -> std::cmp::Ordering {
        use std::cmp::Ordering;

        match (a.score.is_nan(), b.score.is_nan()) {
            // Both NaN: fall back to doc_id ordering
            (true, true) => a.doc_id.cmp(&b.doc_id),
            // a is NaN: a goes to end (is "greater" in descending sort)
            (true, false) => Ordering::Greater,
            // b is NaN: b goes to end (a is "less" = comes first)
            (false, true) => Ordering::Less,
            // Normal comparison: score descending, doc_id ascending as tiebreaker
            (false, false) => b
                .score
                .partial_cmp(&a.score)
                .unwrap_or(Ordering::Equal)
                .then_with(|| a.doc_id.cmp(&b.doc_id)),
        }
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

    /// Check if a document is already indexed
    ///
    /// Returns true if the document ID exists in the index (either in memory or loaded from disk).
    /// Used during startup rebuild to skip documents that are already indexed from .ftidx file.
    pub fn contains_doc(&self, doc_id: &DocumentId) -> bool {
        self.doc_tokens_offsets.contains_key(doc_id)
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

        // Step 1: Build a sorted token list without materializing all postings.
        let mut tokens: Vec<String> = self.inverted_index.keys().cloned().collect();
        if self.lazy_mode {
            tokens.extend(self.token_offsets.keys().cloned());
        }
        tokens.sort();
        tokens.dedup();

        // Write doc_tokens offset table (as Vec to preserve DocumentId type)
        let offsets_offset = self.write_offset;
        let offsets_vec: Vec<(&DocumentId, &u64)> = self.doc_tokens_offsets.iter().collect();
        let offsets_bytes = serde_json::to_vec(&offsets_vec)?;
        {
            let file = self.file_handle.as_mut().unwrap();
            file.flush()?;
            file.seek(SeekFrom::Start(offsets_offset))?;
            file.write_all(&offsets_bytes)?;
        }

        // V3: Write each token entry with TF embedded and build token_offsets table
        let token_entries_offset = offsets_offset + offsets_bytes.len() as u64;
        let mut current_offset = token_entries_offset;
        let mut new_token_offsets: HashMap<String, (u64, u32)> = HashMap::new();

        for token in &tokens {
            let mut entries: Vec<TokenEntry> = Vec::new();

            if let Some(mem_entries) = self.inverted_index.get(token) {
                entries.extend(mem_entries.iter().cloned());
            }

            if self.lazy_mode {
                if let Some((offset, _count)) = self.token_offsets.get(token) {
                    let disk_entries = if self.file_version >= FTIDX_VERSION_V3 {
                        self.read_token_entry_v3(*offset).ok()
                    } else {
                        self.read_token_entry(*offset)
                            .ok()
                            .map(|doc_ids| doc_ids.into_iter().map(|id| (id, 1u32)).collect())
                    };
                    if let Some(disk_entries) = disk_entries {
                        let existing_doc_ids: std::collections::HashSet<_> =
                            entries.iter().map(|(id, _)| id.clone()).collect();
                        for (doc_id, tf) in disk_entries {
                            if !existing_doc_ids.contains(&doc_id)
                                && !self.deleted_doc_ids.contains(&doc_id)
                            {
                                entries.push((doc_id, tf));
                            }
                        }
                    }
                }
            }

            if entries.is_empty() {
                continue;
            }

            // Write token entry with TF (V3 format)
            let token_bytes = token.as_bytes();
            let entries_bytes = serde_json::to_vec(&entries)?;

            {
                let file = self.file_handle.as_mut().unwrap();
                file.seek(SeekFrom::Start(current_offset))?;
                file.write_all(&(token_bytes.len() as u32).to_le_bytes())?;
                file.write_all(token_bytes)?;
                file.write_all(&(entries_bytes.len() as u32).to_le_bytes())?;
                file.write_all(&entries_bytes)?;
            }

            new_token_offsets.insert(token.clone(), (current_offset, entries.len() as u32));
            current_offset += 4 + token_bytes.len() as u64 + 4 + entries_bytes.len() as u64;
        }

        // Write token_offsets table
        let token_offsets_offset = current_offset;
        let token_offsets_vec: Vec<(&String, &(u64, u32))> = new_token_offsets.iter().collect();
        let token_offsets_bytes = serde_json::to_vec(&token_offsets_vec)?;
        {
            let file = self.file_handle.as_mut().unwrap();
            file.seek(SeekFrom::Start(token_offsets_offset))?;
            file.write_all(&token_offsets_bytes)?;
        }

        // Write metadata (name, field, options)
        let metadata_offset = token_offsets_offset + token_offsets_bytes.len() as u64;
        let metadata = FulltextIndexMetadataForSave {
            name: self.name.clone(),
            field: self.field.clone(),
            options: self.options.clone(),
        };
        let metadata_bytes = serde_json::to_vec(&metadata)?;
        {
            let file = self.file_handle.as_mut().unwrap();
            file.write_all(&metadata_bytes)?;
        }

        // Update V3 header with final offsets
        {
            let file = self.file_handle.as_mut().unwrap();
            file.seek(SeekFrom::Start(8))?; // After magic
            file.write_all(&FTIDX_VERSION_V3.to_le_bytes())?;
            file.write_all(&(self.doc_tokens_offsets.len() as u64).to_le_bytes())?;
            file.write_all(&offsets_offset.to_le_bytes())?;
            file.write_all(&token_entries_offset.to_le_bytes())?;
            file.write_all(&token_offsets_offset.to_le_bytes())?;
            file.write_all(&metadata_offset.to_le_bytes())?;

            file.flush()?;
            file.sync_all()?;
        }

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

    // ========================================================================
    // Two-Phase Flush (lock-free serialization for checkpoint)
    // ========================================================================
    //
    // The standard flush() holds the IndexManager write lock for the entire
    // serialize + file write + fsync cycle (13+ seconds for 130K docs).
    // Two-phase flush splits this into:
    //   Phase 1: take_flush_snapshot() — brief write lock (~µs)
    //   Phase 2: serialize_flush()     — NO lock (13+ seconds)
    //   Phase 3: commit_flush()        — brief write lock (~ms)
    //
    // During Phase 2, search still works via frozen_inverted + disk.
    // Inserts buffer doc_tokens in memory (file_handle closed).

    /// Phase 1: Take a snapshot for two-phase flush (under write lock — brief).
    ///
    /// Moves inverted_index into frozen_inverted (for search visibility),
    /// clones metadata, and closes file_handle so inserts use doc_tokens_memory.
    ///
    /// Returns None if no storage_path is set (memory-only index).
    pub(crate) fn take_flush_snapshot(&mut self) -> Option<FulltextFlushSnapshot> {
        let storage_path = self.storage_path.as_ref()?.clone();

        // Move inverted_index → frozen_inverted (search still finds entries)
        let taken = std::mem::take(&mut self.inverted_index);
        let inverted_arc = Arc::new(taken);
        self.frozen_inverted = Some(Arc::clone(&inverted_arc));

        // Close file_handle → inserts fall back to doc_tokens_memory
        // (insert() checks: if storage_path.is_some() && file_handle.is_some())
        self.file_handle = None;

        // Enable lazy_mode so remove() during Phase 2 tracks deleted_doc_ids.
        // Safe: frozen_inverted provides all entries for search during Phase 2,
        // regardless of whether we were in lazy mode before.
        self.lazy_mode = true;

        Some(FulltextFlushSnapshot {
            inverted: inverted_arc,
            token_offsets: self.token_offsets.clone(),
            doc_tokens_offsets: self.doc_tokens_offsets.clone(),
            deleted_doc_ids: self.deleted_doc_ids.clone(),
            write_offset: self.write_offset,
            lazy_mode: self.lazy_mode,
            file_version: self.file_version,
            name: self.name.clone(),
            field: self.field.clone(),
            options: self.options.clone(),
            storage_path,
        })
    }

    /// Phase 2: Serialize flush snapshot to .ftidx file (NO lock held).
    ///
    /// Opens its own file handle, writes the complete index data (V3 format),
    /// and returns the new offsets for commit. Uses the same serialization
    /// logic as flush() but operates on snapshot data.
    pub(crate) fn serialize_flush(snapshot: &FulltextFlushSnapshot) -> Result<FulltextFlushResult> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&snapshot.storage_path)?;

        // Build sorted token list from memory snapshot + disk offsets
        let mut tokens: Vec<String> = snapshot.inverted.keys().cloned().collect();
        if snapshot.lazy_mode {
            tokens.extend(snapshot.token_offsets.keys().cloned());
        }
        tokens.sort();
        tokens.dedup();

        // Write doc_tokens offset table
        let offsets_offset = snapshot.write_offset;
        let offsets_vec: Vec<(&DocumentId, &u64)> = snapshot.doc_tokens_offsets.iter().collect();
        let offsets_bytes = serde_json::to_vec(&offsets_vec)?;
        file.seek(SeekFrom::Start(offsets_offset))?;
        file.write_all(&offsets_bytes)?;

        // Write V3 token entries with TF embedded
        let token_entries_offset = offsets_offset + offsets_bytes.len() as u64;
        let mut current_offset = token_entries_offset;
        let mut new_token_offsets: HashMap<String, (u64, u32)> = HashMap::new();

        for token in &tokens {
            let mut entries: Vec<TokenEntry> = Vec::new();

            if let Some(mem_entries) = snapshot.inverted.get(token) {
                entries.extend(mem_entries.iter().cloned());
            }

            if snapshot.lazy_mode {
                if let Some((offset, _count)) = snapshot.token_offsets.get(token) {
                    let disk_entries = Self::read_token_entries_from_file(
                        &mut file,
                        *offset,
                        snapshot.file_version,
                    );
                    if let Ok(disk) = disk_entries {
                        let existing_doc_ids: HashSet<_> =
                            entries.iter().map(|(id, _)| id.clone()).collect();
                        for (doc_id, tf) in disk {
                            if !existing_doc_ids.contains(&doc_id)
                                && !snapshot.deleted_doc_ids.contains(&doc_id)
                            {
                                entries.push((doc_id, tf));
                            }
                        }
                    }
                }
            }

            if entries.is_empty() {
                continue;
            }

            let token_bytes = token.as_bytes();
            let entries_bytes = serde_json::to_vec(&entries)?;
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

        // Write metadata
        let metadata_offset = token_offsets_offset + token_offsets_bytes.len() as u64;
        let metadata = FulltextIndexMetadataForSave {
            name: snapshot.name.clone(),
            field: snapshot.field.clone(),
            options: snapshot.options.clone(),
        };
        let metadata_bytes = serde_json::to_vec(&metadata)?;
        file.write_all(&metadata_bytes)?;

        // Update V3 header
        file.seek(SeekFrom::Start(8))?; // After magic
        file.write_all(&FTIDX_VERSION_V3.to_le_bytes())?;
        file.write_all(&(snapshot.doc_tokens_offsets.len() as u64).to_le_bytes())?;
        file.write_all(&offsets_offset.to_le_bytes())?;
        file.write_all(&token_entries_offset.to_le_bytes())?;
        file.write_all(&token_offsets_offset.to_le_bytes())?;
        file.write_all(&metadata_offset.to_le_bytes())?;

        file.flush()?;
        file.sync_all()?;

        let new_write_offset = metadata_offset + metadata_bytes.len() as u64;

        Ok(FulltextFlushResult {
            new_token_offsets,
            new_write_offset,
        })
    }

    /// Phase 3: Commit flush results (under write lock — brief).
    ///
    /// Installs new token_offsets, re-opens file_handle, flushes buffered
    /// doc_tokens from concurrent inserts, and clears frozen state.
    pub(crate) fn commit_flush(&mut self, result: FulltextFlushResult) -> Result<()> {
        // 1. Install new token_offsets and switch to lazy mode
        self.token_offsets = result.new_token_offsets;
        self.write_offset = result.new_write_offset;
        self.lazy_mode = true;
        self.file_version = FTIDX_VERSION_V3;
        // NOTE: Do NOT clear deleted_doc_ids here.
        // - Snapshot's deleted entries are already filtered out of the flushed data
        //   (serialize_flush checks snapshot.deleted_doc_ids), so they're harmless in memory.
        // - Phase 2 deletions (added to self.deleted_doc_ids during the flush) are NEEDED
        //   because those docs are still in the freshly written token_offsets on disk.
        // - The next full flush (close/compact via flush()) will clear deleted_doc_ids.

        // 2. Re-open file_handle BEFORE clearing frozen_inverted.
        //    If this fails, rollback_flush() can still restore frozen → inverted_index.
        self.open_storage_file_rw()?;

        // 3. Clear frozen_inverted (entries are now on disk via token_offsets)
        self.frozen_inverted = None;
        // inverted_index has only new entries from concurrent inserts — keep them

        // 4. Flush buffered doc_tokens (inserts during Phase 2 used doc_tokens_memory)
        //    doc_tokens_offsets already has old entries (not moved, only cloned to snapshot).
        //    New entries from concurrent inserts have offset=0 in doc_tokens_offsets;
        //    write them to disk now and update with real offsets.
        let buffered: Vec<(DocumentId, HashMap<String, u32>)> =
            self.doc_tokens_memory.drain().collect();
        for (doc_id, tokens) in buffered {
            let offset = self.write_doc_tokens_to_disk(&doc_id, &tokens)?;
            self.doc_tokens_offsets.insert(doc_id, offset);
        }

        Ok(())
    }

    /// Check if the index has entries in the in-memory inverted_index
    /// that have not yet been flushed to disk.
    ///
    /// Used by IndexManager after commit_flush() to decide whether to
    /// clear the dirty flag. If Phase 2 concurrent inserts added entries
    /// to inverted_index, they still need a subsequent flush to persist.
    pub(crate) fn has_pending_entries(&self) -> bool {
        !self.inverted_index.is_empty()
    }

    /// Rollback a failed two-phase flush (under write lock — brief).
    ///
    /// Restores inverted_index from frozen_inverted and re-opens file_handle.
    /// Called when serialize_flush() returns an error, to prevent data loss
    /// on the next checkpoint (which would overwrite frozen_inverted).
    pub(crate) fn rollback_flush(&mut self) {
        if let Some(frozen) = self.frozen_inverted.take() {
            // Restore: merge frozen entries back into inverted_index
            // (inverted_index may have new entries from concurrent inserts during Phase 2)
            match Arc::try_unwrap(frozen) {
                Ok(entries) => {
                    for (token, mut vec) in entries {
                        self.inverted_index
                            .entry(token)
                            .or_default()
                            .append(&mut vec);
                    }
                }
                Err(arc) => {
                    // Snapshot still referenced (shouldn't happen, but be safe)
                    for (token, entries) in arc.as_ref() {
                        self.inverted_index
                            .entry(token.clone())
                            .or_default()
                            .extend(entries.iter().cloned());
                    }
                }
            }
        }
        // Re-open file_handle (best effort — if this fails, next insert uses memory)
        let _ = self.open_storage_file_rw();
    }

    /// Read token entries from a file at a given offset (standalone, no &self).
    ///
    /// Handles both V2 (HashSet<DocumentId> → TokenEntry with TF=1) and V3 (Vec<TokenEntry>).
    /// Used by serialize_flush() which operates on its own file handle.
    fn read_token_entries_from_file(
        file: &mut File,
        offset: u64,
        file_version: u32,
    ) -> Result<Vec<TokenEntry>> {
        file.seek(SeekFrom::Start(offset))?;

        let mut len_buf = [0u8; 4];

        // Read and skip token name
        file.read_exact(&mut len_buf)?;
        let token_len = u32::from_le_bytes(len_buf) as usize;
        let mut token_buf = vec![0u8; token_len];
        file.read_exact(&mut token_buf)?;

        // Read entries
        file.read_exact(&mut len_buf)?;
        let entries_len = u32::from_le_bytes(len_buf) as usize;
        let mut entries_buf = vec![0u8; entries_len];
        file.read_exact(&mut entries_buf)?;

        if file_version >= FTIDX_VERSION_V3 {
            let entries: Vec<TokenEntry> = serde_json::from_slice(&entries_buf)?;
            Ok(entries)
        } else {
            // V2: HashSet<DocumentId> → convert to TokenEntry with TF=1
            let doc_ids: HashSet<DocumentId> = serde_json::from_slice(&entries_buf)?;
            Ok(doc_ids.into_iter().map(|id| (id, 1u32)).collect())
        }
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
                building: false, // Loaded from disk = already complete
                frozen_inverted: None,
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

            // FIX #25: Set write_offset to END of file (after metadata), NOT offsets_offset!
            // Previously, write_offset was set to offsets_offset, which caused flush()
            // to overwrite existing token entries before they could be read for merging.
            // Now we append new content at the end, preserving old data until merge completes.
            let file_end = file.seek(SeekFrom::End(0))?;

            Ok(FulltextIndex {
                name: metadata.name,
                field: metadata.field,
                options: metadata.options,
                inverted_index: HashMap::new(), // V2/V3: empty, use lazy loading
                storage_path: Some(path),
                doc_tokens_offsets,
                write_offset: file_end, // FIX #25: Use end of file, not offsets_offset
                file_handle: None,
                doc_tokens_memory: HashMap::new(),
                token_offsets,
                lazy_mode: true,       // V2/V3: lazy loading mode
                file_version: version, // V2 will upgrade to V3 on next flush, V3 stays V3
                deleted_doc_ids: HashSet::new(),
                building: false, // Loaded from disk = already complete
                frozen_inverted: None,
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
    /// In lazy mode, tokens may exist in both disk (token_offsets) and memory
    /// (inverted_index), so we compute the union to avoid double-counting.
    pub fn unique_token_count(&self) -> usize {
        if self.lazy_mode || self.frozen_inverted.is_some() {
            // Compute union of all token sources to avoid double-counting
            let mut all_tokens: HashSet<&str> =
                self.inverted_index.keys().map(|s| s.as_str()).collect();
            if let Some(ref frozen) = self.frozen_inverted {
                all_tokens.extend(frozen.keys().map(|s| s.as_str()));
            }
            all_tokens.extend(self.token_offsets.keys().map(|s| s.as_str()));
            all_tokens.len()
        } else {
            self.inverted_index.len()
        }
    }

    /// Memory usage estimate constants (in bytes)
    /// These are rough estimates based on typical HashMap overhead and value sizes
    const EST_BYTES_PER_INVERTED_ENTRY: usize = 100; // String ~24 + Vec overhead ~40 + avg entries ~36
    const EST_BYTES_PER_TOKEN_OFFSET: usize = 32; // String ~24 + (u64, u32) tuple ~12
    const EST_BYTES_PER_DOC_OFFSET: usize = 24; // DocumentId ~16 + u64 ~8
    const EST_BYTES_PER_DOC_TOKENS: usize = 200; // HashMap with ~10 tokens avg

    /// Get memory usage estimate in bytes (for monitoring)
    ///
    /// Note: These are rough estimates. Actual memory usage depends on
    /// token lengths, document counts, and HashMap load factors.
    pub fn memory_usage_bytes(&self) -> usize {
        let inverted_mem = self.inverted_index.len() * Self::EST_BYTES_PER_INVERTED_ENTRY;
        let offsets_mem = self.token_offsets.len() * Self::EST_BYTES_PER_TOKEN_OFFSET;
        let doc_offsets_mem = self.doc_tokens_offsets.len() * Self::EST_BYTES_PER_DOC_OFFSET;
        let doc_tokens_mem = self.doc_tokens_memory.len() * Self::EST_BYTES_PER_DOC_TOKENS;

        inverted_mem + offsets_mem + doc_offsets_mem + doc_tokens_mem
    }
}

// ============================================================================
// LazyLoadable Trait Implementation
// ============================================================================

use crate::index::traits::{IndexTrait, LazyLoadable};

impl IndexTrait for FulltextIndex {
    fn name(&self) -> &str {
        &self.name
    }

    fn fields(&self) -> Vec<&str> {
        vec![&self.field]
    }

    fn entry_count(&self) -> usize {
        self.unique_token_count()
    }

    fn memory_usage_bytes(&self) -> usize {
        // Delegate to existing method
        FulltextIndex::memory_usage_bytes(self)
    }

    fn is_disk_backed(&self) -> bool {
        self.storage_path.is_some()
    }

    fn flush(&mut self) -> crate::error::Result<()> {
        // Fulltext indexes are written via save_to_file(), not incremental flush
        Ok(())
    }
}

impl LazyLoadable for FulltextIndex {
    fn is_lazy_mode(&self) -> bool {
        self.lazy_mode
    }

    fn ensure_fully_loaded(&mut self) -> crate::error::Result<()> {
        // Fulltext lazy mode is different - it loads tokens on-demand during search
        // There's no "load all" operation because tokens are streamed from disk
        // For "Opció A" consistency, we mark as non-lazy after first full access
        // But fulltext doesn't need preloading - it's efficient as-is
        Ok(())
    }

    fn persisted_size_bytes(&self) -> Option<u64> {
        self.storage_path
            .as_ref()
            .and_then(|path| std::fs::metadata(path).ok().map(|m| m.len()))
    }

    fn hot_ratio(&self) -> f32 {
        if self.lazy_mode {
            // In lazy mode, only token_offsets (metadata) is in memory
            // Actual token entries are loaded on-demand
            let total_tokens = self.token_offsets.len() + self.inverted_index.len();
            if total_tokens == 0 {
                return 1.0;
            }
            // inverted_index contains loaded/new tokens
            self.inverted_index.len() as f32 / total_tokens as f32
        } else {
            1.0
        }
    }
}

// ============================================================================
// Drop Implementation
// ============================================================================

impl Drop for FulltextIndex {
    fn drop(&mut self) {
        if let Some(ref mut file) = self.file_handle {
            if let Err(e) = file.sync_all() {
                log_error!(
                    "FulltextIndex::drop sync_all failed for '{}': {}",
                    self.name,
                    e
                );
            }
        }
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

    // ========================================================================
    // Query Parsing Tests (MongoDB-compatible phrase search)
    // ========================================================================

    #[test]
    fn test_parse_query_simple_terms() {
        let parsed = super::parse_query("hello world");
        assert!(parsed.phrases.is_empty());
        assert_eq!(parsed.terms, vec!["hello", "world"]);
    }

    #[test]
    fn test_parse_query_single_phrase() {
        let parsed = super::parse_query("\"hello world\"");
        assert_eq!(parsed.phrases, vec!["hello world"]);
        assert!(parsed.terms.is_empty());
    }

    #[test]
    fn test_parse_query_phrase_and_terms() {
        let parsed = super::parse_query("\"exact phrase\" other words");
        assert_eq!(parsed.phrases, vec!["exact phrase"]);
        assert_eq!(parsed.terms, vec!["other", "words"]);
    }

    #[test]
    fn test_parse_query_multiple_phrases() {
        let parsed = super::parse_query("\"first phrase\" middle \"second phrase\"");
        assert_eq!(parsed.phrases, vec!["first phrase", "second phrase"]);
        assert_eq!(parsed.terms, vec!["middle"]);
    }

    #[test]
    fn test_parse_query_unclosed_quote() {
        // Unclosed quote treats rest as terms
        let parsed = super::parse_query("\"unclosed phrase");
        assert!(parsed.phrases.is_empty());
        assert_eq!(parsed.terms, vec!["unclosed", "phrase"]);
    }

    #[test]
    fn test_parse_query_empty_phrase() {
        // Empty quotes should be ignored
        let parsed = super::parse_query("\"\" word");
        assert!(parsed.phrases.is_empty());
        assert_eq!(parsed.terms, vec!["word"]);
    }

    #[test]
    fn test_parse_query_has_phrases() {
        let no_phrases = super::parse_query("hello world");
        assert!(!no_phrases.has_phrases());

        let with_phrases = super::parse_query("\"hello world\"");
        assert!(with_phrases.has_phrases());
    }

    #[test]
    fn test_build_phrase_regex_simple() {
        let regex = super::build_phrase_regex("hello world").unwrap();
        assert!(regex.is_match("hello world"));
        assert!(regex.is_match("Hello World")); // case insensitive
        assert!(regex.is_match("hello  world")); // flexible whitespace
        assert!(!regex.is_match("world hello")); // wrong order
        assert!(!regex.is_match("helloworld")); // no word boundary
    }

    #[test]
    fn test_build_phrase_regex_special_chars() {
        // Special regex characters should be escaped
        let regex = super::build_phrase_regex("C++ programming").unwrap();
        assert!(regex.is_match("C++ programming"));
        assert!(regex.is_match("c++  Programming")); // case insensitive
    }

    #[test]
    fn test_build_phrase_regex_hungarian() {
        let regex = super::build_phrase_regex("Projectin Kft").unwrap();
        assert!(regex.is_match("Projectin Kft"));
        assert!(regex.is_match("projectin kft")); // case insensitive
        assert!(regex.is_match("PROJECTIN  KFT")); // flexible whitespace
        assert!(!regex.is_match("Kft Projectin")); // wrong order
    }

    #[test]
    fn test_parsed_query_all_search_terms() {
        let options = super::FtsOptions::new(super::FtsLanguage::English);
        let parsed = super::parse_query("\"hello world\" other");
        let terms = parsed.all_search_terms(&options);
        // Should contain tokenized phrase + regular terms
        // "hello world" tokenizes to ["hello", "world"]
        assert!(terms.contains(&"hello".to_string()));
        assert!(terms.contains(&"world".to_string()));
        assert!(terms.contains(&"other".to_string()));
    }
}
