//! Query preprocessing plugin system
//!
//! Pluggable language preprocessors for RAG queries.
//! New languages can be added by implementing the QueryPreprocessor trait.

mod hungarian;
mod registry;

pub use hungarian::HungarianPreprocessor;
pub use registry::{available_languages, get_preprocessor};

/// Trait for language-specific query preprocessing
pub trait QueryPreprocessor: Send + Sync {
    /// Language identifier (e.g., "hungarian", "english")
    fn language(&self) -> &'static str;

    /// Preprocess query - returns cleaned/stemmed query
    fn preprocess(&self, query: &str) -> PreprocessResult;

    /// Optional: get stop words for this language
    fn stop_words(&self) -> &[&str] {
        &[]
    }

    /// Optional: get question words for this language
    fn question_words(&self) -> &[&str] {
        &[]
    }
}

/// Result of preprocessing
#[derive(Debug, Clone)]
pub struct PreprocessResult {
    /// Processed/cleaned query
    pub processed: String,
    /// Original query
    pub original: String,
    /// Words that were removed (stop words, question words, too short)
    pub removed_words: Vec<String>,
    /// Words that were stemmed: (original, stemmed)
    pub stemmed_words: Vec<(String, String)>,
}

impl PreprocessResult {
    /// Create a passthrough result (no preprocessing)
    pub fn passthrough(query: &str) -> Self {
        Self {
            processed: query.to_string(),
            original: query.to_string(),
            removed_words: vec![],
            stemmed_words: vec![],
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
    fn test_passthrough() {
        let result = PreprocessResult::passthrough("test query");
        assert_eq!(result.processed, "test query");
        assert_eq!(result.original, "test query");
        assert!(result.removed_words.is_empty());
        assert!(result.stemmed_words.is_empty());
    }

    #[test]
    fn test_available_languages() {
        let langs = available_languages();
        assert!(langs.contains(&"hungarian"));
    }

    #[test]
    fn test_get_preprocessor_hungarian() {
        let preprocessor = get_preprocessor("hungarian");
        assert!(preprocessor.is_some());
        assert_eq!(preprocessor.unwrap().language(), "hungarian");
    }

    #[test]
    fn test_get_preprocessor_unknown() {
        let preprocessor = get_preprocessor("klingon");
        assert!(preprocessor.is_none());
    }
}
