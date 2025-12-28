// Fuzzy Text Index Implementation

use crate::document::DocumentId;
use crate::value_utils::get_nested_value;
use serde::{Deserialize, Serialize};
use strsim::{damerau_levenshtein, jaro_winkler, normalized_levenshtein};

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
        }
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
}
