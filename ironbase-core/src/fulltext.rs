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
use rust_stemmers::{Algorithm, Stemmer};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
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

// ============================================================================
// Full-Text Index
// ============================================================================

/// Full-text search index using inverted index with TF-IDF scoring
///
/// # Performance Characteristics
/// - Insert: O(t) where t = tokens in document
/// - Search: O(q * d) where q = query tokens, d = matching docs
/// - Storage: ~50-100% overhead per indexed field
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
    /// Inverted index: token -> set of document IDs
    inverted_index: HashMap<String, HashSet<DocumentId>>,
    /// Document token frequencies: doc_id -> (token -> count)
    doc_tokens: HashMap<DocumentId, HashMap<String, u32>>,
}

/// Search result with score and matched tokens
#[derive(Debug, Clone)]
pub struct FtsSearchResult {
    pub doc_id: DocumentId,
    pub score: f64,
    pub matched_tokens: Vec<String>,
}

impl FulltextIndex {
    /// Create a new fulltext index
    pub fn new(name: &str, field: &str, options: FtsOptions) -> Self {
        FulltextIndex {
            name: name.to_string(),
            field: field.to_string(),
            options,
            inverted_index: HashMap::new(),
            doc_tokens: HashMap::new(),
        }
    }

    /// Get metadata for this index
    pub fn metadata(&self) -> FulltextIndexMetadata {
        FulltextIndexMetadata {
            name: self.name.clone(),
            field: self.field.clone(),
            language: self.options.language,
            min_word_length: self.options.min_word_length,
            accent_folding: self.options.accent_folding,
            num_documents: self.doc_tokens.len(),
            num_tokens: self.inverted_index.len(),
        }
    }

    /// Insert a document's field value into the index
    pub fn insert(&mut self, doc_id: &DocumentId, text: &str) {
        let tokens = tokenize(text, &self.options);
        if tokens.is_empty() {
            return;
        }

        // Count token frequencies for this document
        let mut token_counts: HashMap<String, u32> = HashMap::new();
        for token in &tokens {
            *token_counts.entry(token.clone()).or_insert(0) += 1;
        }

        // Add to inverted index
        for token in token_counts.keys() {
            self.inverted_index
                .entry(token.clone())
                .or_default()
                .insert(doc_id.clone());
        }

        // Store token frequencies for TF calculation
        self.doc_tokens.insert(doc_id.clone(), token_counts);
    }

    /// Remove a document from the index
    pub fn remove(&mut self, doc_id: &DocumentId) {
        if let Some(token_counts) = self.doc_tokens.remove(doc_id) {
            // Remove from inverted index
            for token in token_counts.keys() {
                if let Some(doc_ids) = self.inverted_index.get_mut(token) {
                    doc_ids.remove(doc_id);
                    // Don't remove empty sets to avoid iterator invalidation
                }
            }
        }
    }

    /// Update a document in the index
    pub fn update(&mut self, doc_id: &DocumentId, text: &str) {
        self.remove(doc_id);
        self.insert(doc_id, text);
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

        let total_docs = self.doc_tokens.len() as f64;
        if total_docs == 0.0 {
            return Vec::new();
        }

        let mut scores: HashMap<DocumentId, f64> = HashMap::new();
        let mut matched: HashMap<DocumentId, Vec<String>> = HashMap::new();

        for token in &query_tokens {
            if let Some(doc_ids) = self.inverted_index.get(token) {
                // Smoothed IDF = log(1 + total_docs / docs_with_token)
                // This ensures IDF is always positive, even when all docs contain the term
                let idf = (1.0 + total_docs / doc_ids.len() as f64).ln();

                for doc_id in doc_ids {
                    // TF = token count in doc
                    let tf = self
                        .doc_tokens
                        .get(doc_id)
                        .and_then(|t| t.get(token))
                        .copied()
                        .unwrap_or(0) as f64;

                    *scores.entry(doc_id.clone()).or_insert(0.0) += tf * idf;
                    matched
                        .entry(doc_id.clone())
                        .or_default()
                        .push(token.clone());
                }
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
        self.doc_tokens.contains_key(doc_id)
    }

    /// Get number of indexed documents
    pub fn len(&self) -> usize {
        self.doc_tokens.len()
    }

    /// Check if index is empty
    pub fn is_empty(&self) -> bool {
        self.doc_tokens.is_empty()
    }

    /// Get number of unique tokens
    pub fn token_count(&self) -> usize {
        self.inverted_index.len()
    }

    /// Clear all entries from the index
    pub fn clear(&mut self) {
        self.inverted_index.clear();
        self.doc_tokens.clear();
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
        let mut index = FulltextIndex::new("test_idx", "content", options);

        let doc1 = DocumentId::Int(1);
        let doc2 = DocumentId::Int(2);
        let doc3 = DocumentId::Int(3);

        index.insert(&doc1, "The quick brown fox");
        index.insert(&doc2, "The lazy brown dog");
        index.insert(&doc3, "A fast red fox");

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
    }

    #[test]
    fn test_fulltext_index_tfidf_scoring() {
        let options = FtsOptions::new(FtsLanguage::None);
        let mut index = FulltextIndex::new("test_idx", "content", options);

        let doc1 = DocumentId::Int(1);
        let doc2 = DocumentId::Int(2);

        // doc1 has "fox" twice, doc2 has it once
        index.insert(&doc1, "fox fox dog");
        index.insert(&doc2, "fox cat");

        let results = index.search("fox", 10, 0, None);
        assert_eq!(results.len(), 2);
        // doc1 should have higher score (more "fox" occurrences)
        assert!(results[0].doc_id == doc1);
        assert!(results[0].score > results[1].score);
    }

    #[test]
    fn test_fulltext_index_remove() {
        let options = FtsOptions::new(FtsLanguage::None);
        let mut index = FulltextIndex::new("test_idx", "content", options);

        let doc1 = DocumentId::Int(1);
        let doc2 = DocumentId::Int(2);

        index.insert(&doc1, "hello world");
        index.insert(&doc2, "hello rust");

        assert_eq!(index.len(), 2);

        index.remove(&doc1);
        assert_eq!(index.len(), 1);

        let results = index.search("hello", 10, 0, None);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].doc_id, doc2);
    }

    #[test]
    fn test_fulltext_index_pagination() {
        let options = FtsOptions::new(FtsLanguage::None);
        let mut index = FulltextIndex::new("test_idx", "content", options);

        // Create documents with different scores (different term frequencies)
        // Doc 10 has highest score (10x "test"), doc 1 has lowest (1x "test")
        for i in 1..=10 {
            let content = "test ".repeat(i as usize);
            index.insert(&DocumentId::Int(i), &content);
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
    }

    #[test]
    fn test_fulltext_index_min_score() {
        let options = FtsOptions::new(FtsLanguage::None);
        let mut index = FulltextIndex::new("test_idx", "content", options);

        let doc1 = DocumentId::Int(1);
        let doc2 = DocumentId::Int(2);

        // doc1 has very high TF for "apple"
        index.insert(&doc1, "apple apple apple apple apple");
        index.insert(&doc2, "apple banana");

        let results = index.search("apple", 10, 0, None);
        assert_eq!(results.len(), 2);

        let high_score = results[0].score;
        // Filter with min_score just below the highest
        let filtered = index.search("apple", 10, 0, Some(high_score - 0.01));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].doc_id, doc1);
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
        let mut index = FulltextIndex::new("test_idx", "content", options);

        index.insert(&DocumentId::Int(1), "Hello world");

        // Search with only stop words should return empty
        let results = index.search("the a an", 10, 0, None);
        assert!(results.is_empty());
    }
}
