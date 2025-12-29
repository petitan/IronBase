// Index Manager - manages all indexes for a collection

use crate::document::DocumentId;
use crate::error::{IronBaseError, Result};
use crate::fulltext::{FtsLanguage, FtsOptions, FulltextIndex};
use crate::value_utils::get_nested_value;
use std::collections::HashMap;
use std::path::PathBuf;

use super::btree::BPlusTree;
use super::fuzzy::{FuzzyAlgorithm, FuzzyIndex};
use super::key::{IndexKey, IndexPrefixInfo};
use super::legacy::{Index, IndexDefinition};

/// Index Manager - manages all indexes for a collection
pub struct IndexManager {
    btree_indexes: HashMap<String, BPlusTree>,
    legacy_indexes: HashMap<String, Index>,
    /// Fuzzy text indexes for similarity search
    fuzzy_indexes: HashMap<String, FuzzyIndex>,
    /// Full-text search indexes with TF-IDF scoring
    fulltext_indexes: HashMap<String, FulltextIndex>,
    /// File paths for persistent indexes (for two-phase commit)
    index_file_paths: HashMap<String, PathBuf>,
}

impl IndexManager {
    pub fn new() -> Self {
        IndexManager {
            btree_indexes: HashMap::new(),
            legacy_indexes: HashMap::new(),
            fuzzy_indexes: HashMap::new(),
            fulltext_indexes: HashMap::new(),
            index_file_paths: HashMap::new(),
        }
    }

    /// Set file path for an index (required for two-phase commit)
    pub fn set_index_path(&mut self, index_name: &str, path: PathBuf) {
        self.index_file_paths.insert(index_name.to_string(), path);
    }

    /// Get file path for an index
    pub fn get_index_path(&self, index_name: &str) -> Option<&PathBuf> {
        self.index_file_paths.get(index_name)
    }

    /// Create B+ tree index (single field)
    ///
    /// # Arguments
    /// * `name` - Index name
    /// * `field` - Field to index
    /// * `unique` - Whether values must be unique
    /// * `sparse` - If true, documents missing the field are not indexed
    pub fn create_btree_index(
        &mut self,
        name: String,
        field: String,
        unique: bool,
        sparse: bool,
    ) -> Result<()> {
        if self.btree_indexes.contains_key(&name) {
            return Err(IronBaseError::IndexError(format!(
                "Index already exists: {}",
                name
            )));
        }

        let tree = BPlusTree::new(name.clone(), field, unique, sparse);
        self.btree_indexes.insert(name, tree);
        Ok(())
    }

    /// Create compound B+ tree index (multiple fields)
    ///
    /// # Arguments
    /// * `name` - Index name
    /// * `fields` - Ordered list of fields (e.g., ["country", "city"])
    /// * `unique` - Whether the compound key must be unique
    /// * `sparse` - If true, documents missing any field are not indexed
    ///
    /// # Example
    /// ```rust,ignore
    /// manager.create_compound_index(
    ///     "users_location".to_string(),
    ///     vec!["country".to_string(), "city".to_string()],
    ///     false,
    ///     false
    /// )?;
    /// ```
    pub fn create_compound_index(
        &mut self,
        name: String,
        fields: Vec<String>,
        unique: bool,
        sparse: bool,
    ) -> Result<()> {
        if self.btree_indexes.contains_key(&name) {
            return Err(IronBaseError::IndexError(format!(
                "Index already exists: {}",
                name
            )));
        }

        if fields.is_empty() {
            return Err(IronBaseError::IndexError(
                "Compound index must have at least one field".to_string(),
            ));
        }

        let tree = BPlusTree::new_compound(name.clone(), fields, unique, sparse);
        self.btree_indexes.insert(name, tree);
        Ok(())
    }

    /// Create legacy HashMap index
    pub fn create_index(&mut self, definition: IndexDefinition) -> Result<()> {
        let name = definition.name.clone();

        if self.legacy_indexes.contains_key(&name) {
            return Err(IronBaseError::IndexError(format!(
                "Index already exists: {}",
                name
            )));
        }

        self.legacy_indexes.insert(name, Index::new(definition));
        Ok(())
    }

    /// Drop index by name (supports B+ tree, legacy, and fuzzy indexes)
    pub fn drop_index(&mut self, name: &str) -> Result<()> {
        let removed = self.btree_indexes.remove(name).is_some()
            || self.legacy_indexes.remove(name).is_some()
            || self.fuzzy_indexes.remove(name).is_some()
            || self.fulltext_indexes.remove(name).is_some();

        if !removed {
            return Err(IronBaseError::IndexError(format!(
                "Index not found: {}",
                name
            )));
        }
        // Also remove file path if it exists
        self.index_file_paths.remove(name);
        Ok(())
    }

    /// Get B+ tree index
    pub fn get_btree_index(&self, name: &str) -> Option<&BPlusTree> {
        self.btree_indexes.get(name)
    }

    /// Get B+ tree index (mutable)
    pub fn get_btree_index_mut(&mut self, name: &str) -> Option<&mut BPlusTree> {
        self.btree_indexes.get_mut(name)
    }

    /// Add a pre-loaded BPlusTree index (from .idx file)
    pub fn add_loaded_index(&mut self, tree: BPlusTree) {
        let name = tree.metadata.name.clone();
        self.btree_indexes.insert(name, tree);
    }

    /// Get legacy index
    pub fn get_index(&self, name: &str) -> Option<&Index> {
        self.legacy_indexes.get(name)
    }

    /// Get legacy index (mutable)
    pub fn get_index_mut(&mut self, name: &str) -> Option<&mut Index> {
        self.legacy_indexes.get_mut(name)
    }

    /// List all index names
    pub fn list_indexes(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .btree_indexes
            .keys()
            .chain(self.legacy_indexes.keys())
            .chain(self.fuzzy_indexes.keys())
            .chain(self.fulltext_indexes.keys())
            .cloned()
            .collect();
        names.sort();
        names
    }

    /// List all indexes with their first field info (for QueryPlanner)
    ///
    /// Returns tuples of (index_name, first_field) where first_field is:
    /// - The field name for single-field indexes
    /// - The FIRST field for compound indexes (enables prefix queries!)
    ///
    /// For compound indexes, prefix queries use range scans internally.
    pub fn list_indexes_with_prefix_field(&self) -> Vec<(String, String)> {
        self.list_indexes_with_compound_info()
            .into_iter()
            .map(|info| (info.index_name, info.prefix_field))
            .collect()
    }

    /// List all indexes with full compound index information (for QueryPlanner v2)
    ///
    /// Returns `IndexPrefixInfo` for each index, including:
    /// - Single-field indexes: `is_compound = false`, `num_fields = 1`
    /// - Compound indexes: `is_compound = true`, `num_fields > 1`
    ///
    /// Compound indexes can be used for prefix queries via range scans.
    pub fn list_indexes_with_compound_info(&self) -> Vec<IndexPrefixInfo> {
        let mut result: Vec<IndexPrefixInfo> = Vec::new();

        for (name, index) in &self.btree_indexes {
            let is_compound = index.metadata.is_compound();
            let prefix_field = if is_compound {
                // For compound indexes, return the first field to enable prefix queries
                index.metadata.fields.first().cloned().unwrap_or_default()
            } else {
                index.metadata.field.clone()
            };
            let num_fields = if is_compound {
                index.metadata.fields.len()
            } else {
                1
            };
            result.push(IndexPrefixInfo {
                index_name: name.clone(),
                prefix_field,
                is_compound,
                num_fields,
            });
        }

        // Legacy indexes are single-field only
        for (name, index) in &self.legacy_indexes {
            result.push(IndexPrefixInfo {
                index_name: name.clone(),
                prefix_field: index.definition.field.clone(),
                is_compound: false,
                num_fields: 1,
            });
        }

        result.sort_by(|a, b| a.index_name.cmp(&b.index_name));
        result
    }

    // ========== FUZZY INDEX OPERATIONS ==========

    /// Create a fuzzy text index
    ///
    /// # Arguments
    /// * `name` - Index name
    /// * `field` - Field to index
    /// * `algorithm` - Similarity algorithm (JaroWinkler, Levenshtein, DamerauLevenshtein)
    /// * `threshold` - Minimum similarity threshold (0.0-1.0)
    ///
    /// # Example
    /// ```rust,ignore
    /// manager.create_fuzzy_index(
    ///     "name_fuzzy",
    ///     "name",
    ///     FuzzyAlgorithm::JaroWinkler,
    ///     0.8
    /// )?;
    /// ```
    pub fn create_fuzzy_index(
        &mut self,
        name: String,
        field: String,
        algorithm: FuzzyAlgorithm,
        threshold: f64,
    ) -> Result<()> {
        // Check if any index with this name already exists
        if self.btree_indexes.contains_key(&name)
            || self.legacy_indexes.contains_key(&name)
            || self.fuzzy_indexes.contains_key(&name)
            || self.fulltext_indexes.contains_key(&name)
        {
            return Err(IronBaseError::IndexError(format!(
                "Index already exists: {}",
                name
            )));
        }

        let index = FuzzyIndex::new(&name, &field, algorithm, threshold);
        self.fuzzy_indexes.insert(name, index);
        Ok(())
    }

    /// Get fuzzy index
    pub fn get_fuzzy_index(&self, name: &str) -> Option<&FuzzyIndex> {
        self.fuzzy_indexes.get(name)
    }

    /// Get fuzzy index (mutable)
    pub fn get_fuzzy_index_mut(&mut self, name: &str) -> Option<&mut FuzzyIndex> {
        self.fuzzy_indexes.get_mut(name)
    }

    /// Get fuzzy index for a field (if one exists)
    pub fn get_fuzzy_index_for_field(&self, field: &str) -> Option<&FuzzyIndex> {
        self.fuzzy_indexes
            .values()
            .find(|idx| idx.metadata.field == field)
    }

    /// List all fuzzy indexes
    pub fn list_fuzzy_indexes(&self) -> Vec<&FuzzyIndex> {
        self.fuzzy_indexes.values().collect()
    }

    /// Add a pre-loaded FuzzyIndex
    pub fn add_loaded_fuzzy_index(&mut self, index: FuzzyIndex) {
        let name = index.metadata.name.clone();
        self.fuzzy_indexes.insert(name, index);
    }

    // ========== FULL-TEXT INDEX METHODS ==========

    /// Create full-text search index with language support
    ///
    /// # Arguments
    /// * `name` - Unique index name
    /// * `field` - Field to index (supports dot notation)
    /// * `language` - Language for stemming and stop words
    /// * `min_word_length` - Minimum word length to index (default: 2)
    /// * `accent_folding` - Whether to apply accent folding (default: true)
    pub fn create_fulltext_index(
        &mut self,
        name: String,
        field: String,
        language: FtsLanguage,
        min_word_length: Option<usize>,
        accent_folding: Option<bool>,
    ) -> Result<()> {
        self.create_fulltext_index_with_storage(
            name,
            field,
            language,
            min_word_length,
            accent_folding,
            None,
        )
    }

    /// Create full-text search index with disk-based storage
    ///
    /// # Arguments
    /// * `name` - Unique index name
    /// * `field` - Field to index (supports dot notation)
    /// * `language` - Language for stemming and stop words
    /// * `min_word_length` - Minimum word length to index (default: 2)
    /// * `accent_folding` - Whether to apply accent folding (default: true)
    /// * `storage_path` - Path to store the .ftidx file (None = memory-only)
    pub fn create_fulltext_index_with_storage(
        &mut self,
        name: String,
        field: String,
        language: FtsLanguage,
        min_word_length: Option<usize>,
        accent_folding: Option<bool>,
        storage_path: Option<PathBuf>,
    ) -> Result<()> {
        // Check if any index with this name already exists
        if self.btree_indexes.contains_key(&name)
            || self.legacy_indexes.contains_key(&name)
            || self.fuzzy_indexes.contains_key(&name)
            || self.fulltext_indexes.contains_key(&name)
        {
            return Err(IronBaseError::IndexError(format!(
                "Index already exists: {}",
                name
            )));
        }

        let options = FtsOptions::with_settings(
            language,
            min_word_length.unwrap_or(2),
            accent_folding.unwrap_or(true),
        );

        let index = if let Some(path) = storage_path {
            self.index_file_paths.insert(name.clone(), path.clone());
            FulltextIndex::new_with_storage(&name, &field, options, path)?
        } else {
            FulltextIndex::new(&name, &field, options)
        };
        self.fulltext_indexes.insert(name, index);
        Ok(())
    }

    /// Get fulltext index by name
    pub fn get_fulltext_index(&self, name: &str) -> Option<&FulltextIndex> {
        self.fulltext_indexes.get(name)
    }

    /// Get fulltext index by name (mutable)
    pub fn get_fulltext_index_mut(&mut self, name: &str) -> Option<&mut FulltextIndex> {
        self.fulltext_indexes.get_mut(name)
    }

    /// Get fulltext index for a field (if one exists)
    pub fn get_fulltext_index_for_field(&self, field: &str) -> Option<&FulltextIndex> {
        self.fulltext_indexes
            .values()
            .find(|idx| idx.field == field)
    }

    /// List all fulltext indexes
    pub fn list_fulltext_indexes(&self) -> Vec<&FulltextIndex> {
        self.fulltext_indexes.values().collect()
    }

    /// Drop a fulltext index by name
    ///
    /// Returns Ok(()) if the index was removed, Err if not found.
    /// Used for cleanup on failed index creation.
    pub fn drop_fulltext_index(&mut self, name: &str) -> Result<()> {
        if self.fulltext_indexes.remove(name).is_some() {
            self.index_file_paths.remove(name);
            Ok(())
        } else {
            Err(IronBaseError::IndexError(format!(
                "Fulltext index not found: {}",
                name
            )))
        }
    }

    /// Check if any index in this manager has unique constraint
    ///
    /// Used for hybrid locking: collections with unique indexes use collection-level lock,
    /// collections without unique indexes can use per-document locking for better concurrency.
    pub fn has_unique_index(&self) -> bool {
        self.btree_indexes.values().any(|idx| idx.metadata.unique)
    }

    /// Add a pre-loaded FulltextIndex
    pub fn add_loaded_fulltext_index(&mut self, index: FulltextIndex) {
        let name = index.name.clone();
        self.fulltext_indexes.insert(name, index);
    }

    /// Flush all fulltext indexes to disk
    ///
    /// This should be called before database close or during checkpoint
    /// to persist fulltext index changes to .ftidx files.
    pub fn flush_fulltext_indexes(&mut self) -> Result<()> {
        for index in self.fulltext_indexes.values_mut() {
            index.save_to_file()?;
        }
        Ok(())
    }

    // ========== CENTRALIZED INDEX OPERATIONS (FIX #19) ==========

    /// Add a document to all indexes (B+ tree and fuzzy)
    ///
    /// Properly handles both single-field and compound indexes using extract_key().
    /// For unique indexes: includes null keys (MongoDB treats null as a value).
    /// For non-unique indexes: skips null keys (no query benefit).
    /// For fuzzy indexes: only indexes string values.
    ///
    /// # Arguments
    /// * `doc` - The document as JSON Value
    /// * `doc_id` - The document ID
    /// * `exclude_index` - Optional index name to skip (e.g., "_id" index handled separately)
    pub fn add_document_to_indexes(
        &mut self,
        doc: &serde_json::Value,
        doc_id: &DocumentId,
        exclude_index: Option<&str>,
    ) -> Result<()> {
        // B+ tree indexes
        let index_names: Vec<String> = self.btree_indexes.keys().cloned().collect();

        for index_name in index_names {
            if let Some(excluded) = exclude_index {
                if index_name == excluded {
                    continue;
                }
            }

            if let Some(index) = self.btree_indexes.get_mut(&index_name) {
                let index_key = index.extract_key(doc);
                let is_null = Self::is_key_all_null(&index_key);

                // Sparse indexes: NEVER include null keys (that's the whole point)
                // Non-sparse unique indexes: include null keys (null is a value, enforce uniqueness)
                // Non-sparse non-unique indexes: skip null keys (no query benefit)
                let should_index = !is_null || (index.metadata.unique && !index.metadata.sparse);
                if should_index {
                    index.insert(index_key, doc_id.clone())?;
                }
            }
        }

        // Fuzzy indexes
        let fuzzy_names: Vec<String> = self.fuzzy_indexes.keys().cloned().collect();

        for index_name in fuzzy_names {
            if let Some(excluded) = exclude_index {
                if index_name == excluded {
                    continue;
                }
            }

            if let Some(index) = self.fuzzy_indexes.get_mut(&index_name) {
                // Get field value - only index string values
                if let Some(value) = get_nested_value(doc, &index.metadata.field) {
                    if let Some(s) = value.as_str() {
                        index.insert(s, doc_id.clone());
                    }
                }
            }
        }

        // Fulltext indexes
        let fulltext_names: Vec<String> = self.fulltext_indexes.keys().cloned().collect();

        for index_name in fulltext_names {
            if let Some(excluded) = exclude_index {
                if index_name == excluded {
                    continue;
                }
            }

            if let Some(index) = self.fulltext_indexes.get_mut(&index_name) {
                // Get field value - only index string values
                if let Some(value) = get_nested_value(doc, &index.field) {
                    if let Some(s) = value.as_str() {
                        let _ = index.insert(doc_id, s);
                    }
                }
            }
        }

        Ok(())
    }

    /// Remove a document from all indexes (B+ tree and fuzzy)
    ///
    /// Properly handles both single-field and compound indexes.
    /// For unique indexes: removes null keys (they were inserted).
    /// For non-unique indexes: skips null keys (they weren't inserted).
    /// For fuzzy indexes: removes by document ID.
    ///
    /// # Arguments
    /// * `doc` - The document as JSON Value
    /// * `doc_id` - The document ID
    /// * `exclude_index` - Optional index name to skip
    pub fn remove_document_from_indexes(
        &mut self,
        doc: &serde_json::Value,
        doc_id: &DocumentId,
        exclude_index: Option<&str>,
    ) -> Result<()> {
        // B+ tree indexes
        let index_names: Vec<String> = self.btree_indexes.keys().cloned().collect();

        for index_name in index_names {
            if let Some(excluded) = exclude_index {
                if index_name == excluded {
                    continue;
                }
            }

            if let Some(index) = self.btree_indexes.get_mut(&index_name) {
                let index_key = index.extract_key(doc);
                let is_null = Self::is_key_all_null(&index_key);

                // Mirror the insert logic: only delete what was inserted
                // Sparse indexes: null keys were never inserted
                // Non-sparse unique indexes: null keys were inserted
                // Non-sparse non-unique indexes: null keys were not inserted
                let was_indexed = !is_null || (index.metadata.unique && !index.metadata.sparse);
                if was_indexed {
                    index.delete(&index_key, doc_id)?;
                }
            }
        }

        // Fuzzy indexes - remove by document ID
        let fuzzy_names: Vec<String> = self.fuzzy_indexes.keys().cloned().collect();

        for index_name in fuzzy_names {
            if let Some(excluded) = exclude_index {
                if index_name == excluded {
                    continue;
                }
            }

            if let Some(index) = self.fuzzy_indexes.get_mut(&index_name) {
                index.remove(doc_id);
            }
        }

        // Fulltext indexes - remove by document ID
        let fulltext_names: Vec<String> = self.fulltext_indexes.keys().cloned().collect();

        for index_name in fulltext_names {
            if let Some(excluded) = exclude_index {
                if index_name == excluded {
                    continue;
                }
            }

            if let Some(index) = self.fulltext_indexes.get_mut(&index_name) {
                let _ = index.remove(doc_id);
            }
        }

        Ok(())
    }

    /// Check if a document would violate unique constraints
    ///
    /// Checks all unique indexes to see if the document's values already exist.
    /// Properly handles compound unique indexes.
    /// MongoDB behavior: null is a value, so duplicate nulls are rejected.
    ///
    /// # Arguments
    /// * `doc` - The document as JSON Value
    /// * `exclude_doc_id` - Optional document ID to exclude (for updates)
    /// * `exclude_index` - Optional index name to skip (e.g., "_id" handled separately)
    pub fn check_unique_constraints(
        &self,
        doc: &serde_json::Value,
        exclude_doc_id: Option<&DocumentId>,
        exclude_index: Option<&str>,
    ) -> Result<()> {
        for (index_name, index) in &self.btree_indexes {
            if let Some(excluded) = exclude_index {
                if index_name == excluded {
                    continue;
                }
            }

            // Only check unique indexes
            if !index.metadata.unique {
                continue;
            }

            let index_key = index.extract_key(doc);

            // MongoDB behavior: null IS a value for unique constraint purposes
            // Do NOT skip null keys - duplicate nulls should be rejected

            // Check if key already exists
            if let Some(existing_id) = index.search(&index_key) {
                // Allow update to same document (exclude_doc_id matches)
                if exclude_doc_id != Some(&existing_id) {
                    // Format field names for error message
                    let fields_str = if index.metadata.is_compound() {
                        index.metadata.fields.join(", ")
                    } else {
                        index.metadata.field.clone()
                    };
                    return Err(IronBaseError::IndexError(format!(
                        "Duplicate key: {:?} in field(s) '{}' (unique index)",
                        index_key, fields_str
                    )));
                }
            }
        }

        Ok(())
    }

    /// Helper: Check if an IndexKey is all Null
    pub fn is_key_all_null(key: &IndexKey) -> bool {
        match key {
            IndexKey::Null => true,
            IndexKey::Compound(keys) => keys.iter().all(|k| matches!(k, IndexKey::Null)),
            _ => false,
        }
    }
}

impl Default for IndexManager {
    fn default() -> Self {
        Self::new()
    }
}
