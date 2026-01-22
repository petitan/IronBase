//! Registry for language preprocessors
//!
//! Provides a global registry of preprocessors that can be accessed by language name.
//! New preprocessors can be registered at startup.

use super::{HungarianPreprocessor, QueryPreprocessor};
use std::collections::HashMap;
use std::sync::OnceLock;

/// Global registry of preprocessors
static REGISTRY: OnceLock<PreprocessorRegistry> = OnceLock::new();

/// Registry holding all available preprocessors
pub struct PreprocessorRegistry {
    preprocessors: HashMap<&'static str, Box<dyn QueryPreprocessor>>,
}

impl PreprocessorRegistry {
    /// Create a new registry with built-in preprocessors
    fn new() -> Self {
        let mut registry = Self {
            preprocessors: HashMap::new(),
        };

        // Register built-in preprocessors
        registry.register(Box::new(HungarianPreprocessor));

        // Future: register more languages here
        // registry.register(Box::new(EnglishPreprocessor));
        // registry.register(Box::new(GermanPreprocessor));

        registry
    }

    /// Register a new preprocessor
    pub fn register(&mut self, preprocessor: Box<dyn QueryPreprocessor>) {
        self.preprocessors
            .insert(preprocessor.language(), preprocessor);
    }

    /// Get a preprocessor by language name
    pub fn get(&self, language: &str) -> Option<&dyn QueryPreprocessor> {
        self.preprocessors.get(language).map(|b| b.as_ref())
    }

    /// List all available language names
    pub fn available_languages(&self) -> Vec<&'static str> {
        let mut langs: Vec<_> = self.preprocessors.keys().copied().collect();
        langs.sort();
        langs
    }
}

/// Get preprocessor by language name
///
/// Returns None if the language is not registered.
///
/// # Example
/// ```ignore
/// if let Some(preprocessor) = get_preprocessor("hungarian") {
///     let result = preprocessor.preprocess("Milyen jellemzői vannak?");
///     println!("Processed: {}", result.processed);
/// }
/// ```
pub fn get_preprocessor(language: &str) -> Option<&'static dyn QueryPreprocessor> {
    REGISTRY
        .get_or_init(PreprocessorRegistry::new)
        .get(language)
}

/// List all available preprocessor languages
///
/// # Example
/// ```ignore
/// let langs = available_languages();
/// // Returns: ["hungarian"]
/// ```
pub fn available_languages() -> Vec<&'static str> {
    REGISTRY
        .get_or_init(PreprocessorRegistry::new)
        .available_languages()
}
