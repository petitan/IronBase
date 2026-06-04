//! Vector Index Manager — separate-lock home for HNSW vector indexes.
//!
//! Carved out of [`IndexManager`](super::manager::IndexManager) so that a slow
//! HNSW batch-build (held under this manager's own `RwLock`) does NOT block
//! btree/fulltext/fuzzy reads (e.g. a filtered `count` taking the shared
//! `IndexManager` read lock).
//!
//! Lock-ordering invariant: the `IndexManager` lock and the `VectorIndexManager`
//! lock are NEVER held simultaneously. Every code path acquires one, releases it,
//! then (if needed) acquires the other. The data sets are disjoint, so no
//! cross-type snapshot under a single critical section is required.
//!
//! Like `IndexManager`, this type is NOT thread-safe on its own; `DatabaseCore`
//! wraps each per-collection instance in `Arc<RwLock<VectorIndexManager>>`
//! (the `vector_managers` map) and `CollectionCore` holds a clone of that Arc.

use crate::document::DocumentId;
use crate::error::{IronBaseError, Result};
use crate::log_error;
use crate::value_utils::get_nested_value;
use crate::vector::{HnswIndex, VectorIndexConfig};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Manages all HNSW vector indexes for a single collection.
#[derive(Default)]
pub struct VectorIndexManager {
    /// HNSW vector indexes for similarity search
    vector_indexes: HashMap<String, HnswIndex>,
    /// File paths for persistent vector caches (`.hnsw`)
    vector_file_paths: HashMap<String, PathBuf>,
    /// Dirty tracking for vector indexes (consolidates HNSW internal dirty flag)
    dirty_vector_indexes: HashSet<String>,
}

impl VectorIndexManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether any vector index needs flushing.
    pub fn has_dirty(&self) -> bool {
        !self.dirty_vector_indexes.is_empty()
    }

    /// Mark a vector index as dirty (modified since last flush).
    pub fn mark_vector_dirty(&mut self, name: &str) {
        self.dirty_vector_indexes.insert(name.to_string());
    }

    /// Names of dirty vector indexes (for per-index flush).
    pub fn dirty_vector_index_names(&self) -> Vec<String> {
        self.dirty_vector_indexes.iter().cloned().collect()
    }

    /// Whether a vector index with this name exists.
    pub fn contains(&self, name: &str) -> bool {
        self.vector_indexes.contains_key(name)
    }

    /// Set the cache-file path for a vector index (two-phase commit).
    pub fn set_index_path(&mut self, index_name: &str, path: PathBuf) {
        self.vector_file_paths.insert(index_name.to_string(), path);
    }

    /// Get the cache-file path for a vector index.
    pub fn get_index_path(&self, index_name: &str) -> Option<&PathBuf> {
        self.vector_file_paths.get(index_name)
    }

    /// Create a vector (HNSW) index.
    pub fn create_vector_index(
        &mut self,
        name: String,
        _field: String,
        config: VectorIndexConfig,
        storage_path: Option<PathBuf>,
    ) -> Result<()> {
        if self.vector_indexes.contains_key(&name) {
            return Err(IronBaseError::IndexError(format!(
                "Index already exists: {}",
                name
            )));
        }

        config
            .validate()
            .map_err(|e| IronBaseError::IndexError(e.to_string()))?;

        let index = HnswIndex::new(config);

        if let Some(path) = storage_path {
            self.vector_file_paths.insert(name.clone(), path);
        }

        self.vector_indexes.insert(name, index);
        Ok(())
    }

    /// Get vector index by name.
    pub fn get_vector_index(&self, name: &str) -> Option<&HnswIndex> {
        self.vector_indexes.get(name)
    }

    /// Get vector index by name (mutable), auto-marking as dirty.
    pub fn get_vector_index_mut(&mut self, name: &str) -> Option<&mut HnswIndex> {
        if self.vector_indexes.contains_key(name) {
            self.dirty_vector_indexes.insert(name.to_string());
        }
        self.vector_indexes.get_mut(name)
    }

    /// Get vector index for a field (if one exists).
    pub fn get_vector_index_for_field(
        &self,
        collection_name: &str,
        field: &str,
    ) -> Option<&HnswIndex> {
        let expected_name = format!("{}_vec_{}", collection_name, field);
        self.vector_indexes.get(&expected_name)
    }

    /// Get vector index for a field (mutable).
    pub fn get_vector_index_for_field_mut(
        &mut self,
        collection_name: &str,
        field: &str,
    ) -> Option<&mut HnswIndex> {
        let expected_name = format!("{}_vec_{}", collection_name, field);
        if self.vector_indexes.contains_key(&expected_name) {
            self.dirty_vector_indexes.insert(expected_name.clone());
        }
        self.vector_indexes.get_mut(&expected_name)
    }

    /// List all vector indexes.
    pub fn list_vector_indexes(&self) -> Vec<&HnswIndex> {
        self.vector_indexes.values().collect()
    }

    /// All vector index names.
    pub fn vector_index_names(&self) -> Vec<String> {
        self.vector_indexes.keys().cloned().collect()
    }

    /// Add a pre-loaded `HnswIndex` (from `.hnsw` file).
    pub fn add_loaded_vector_index(&mut self, name: String, index: HnswIndex) {
        self.vector_indexes.insert(name, index);
    }

    /// Drop a vector index by name.
    pub fn drop_vector_index(&mut self, name: &str) -> Result<()> {
        if self.vector_indexes.remove(name).is_some() {
            self.vector_file_paths.remove(name);
            self.dirty_vector_indexes.remove(name);
            Ok(())
        } else {
            Err(IronBaseError::IndexError(format!(
                "Vector index not found: {}",
                name
            )))
        }
    }

    /// Add a document to all vector indexes (auto-index vector fields).
    ///
    /// Moved verbatim from `IndexManager::add_document_to_indexes` (the vector
    /// section); kept the `dirty_vector_indexes.insert()` invariant.
    pub fn add_document_to_vector_indexes(
        &mut self,
        doc: &Value,
        doc_id: &DocumentId,
        exclude_index: Option<&str>,
    ) -> Result<()> {
        let vector_names: Vec<String> = self.vector_indexes.keys().cloned().collect();

        for index_name in vector_names {
            if let Some(excluded) = exclude_index {
                if index_name == excluded {
                    continue;
                }
            }

            if let Some(index) = self.vector_indexes.get_mut(&index_name) {
                let field = &index.config().field;
                if field.is_empty() {
                    continue; // Skip if no field configured (legacy index)
                }

                if let Some(value) = get_nested_value(doc, field) {
                    if let Some(arr) = value.as_array() {
                        let vector: Vec<f32> = arr
                            .iter()
                            .filter_map(|v| v.as_f64().map(|f| f as f32))
                            .collect();

                        if vector.len() == index.config().dim {
                            let id_str = match doc_id {
                                DocumentId::Int(i) => i.to_string(),
                                DocumentId::String(s) => s.clone(),
                                DocumentId::ObjectId(oid) => oid.clone(),
                            };
                            // Insert into HNSW - log error but don't fail (document already persisted)
                            if let Err(e) = index.insert(&id_str, &vector) {
                                log_error!(
                                    "Failed to insert into vector index '{}': {:?}",
                                    index_name,
                                    e
                                );
                            } else {
                                self.dirty_vector_indexes.insert(index_name.clone());
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Remove a document from all vector indexes (by document ID).
    pub fn remove_document_from_vector_indexes(
        &mut self,
        doc_id: &DocumentId,
        exclude_index: Option<&str>,
    ) -> Result<()> {
        let vector_names: Vec<String> = self.vector_indexes.keys().cloned().collect();

        for index_name in vector_names {
            if let Some(excluded) = exclude_index {
                if index_name == excluded {
                    continue;
                }
            }

            if let Some(index) = self.vector_indexes.get_mut(&index_name) {
                let id_str = match doc_id {
                    DocumentId::Int(i) => i.to_string(),
                    DocumentId::String(s) => s.clone(),
                    DocumentId::ObjectId(oid) => oid.clone(),
                };
                index.remove(&id_str);
                self.dirty_vector_indexes.insert(index_name.clone());
            }
        }

        Ok(())
    }

    /// Atomically persist a single HNSW index to its cache path via temp file +
    /// rename. Returns the final file size in bytes.
    fn persist_hnsw_to_file(index: &HnswIndex, cache_path: &PathBuf) -> Result<u64> {
        use std::io::BufWriter;

        let temp_path = cache_path.with_extension("hnsw.tmp");
        let file = std::fs::File::create(&temp_path).map_err(|e| {
            IronBaseError::Io(std::io::Error::other(format!(
                "Failed to create HNSW temp file '{}': {}",
                temp_path.display(),
                e
            )))
        })?;
        let mut writer = BufWriter::new(file);
        index.save_to_writer(&mut writer)?;
        std::io::Write::flush(&mut writer).map_err(|e| {
            IronBaseError::Io(std::io::Error::other(format!(
                "Failed to flush HNSW temp file '{}': {}",
                temp_path.display(),
                e
            )))
        })?;
        let file = writer.into_inner().map_err(|e| {
            IronBaseError::Io(std::io::Error::other(format!(
                "BufWriter into_inner failed for '{}': {}",
                temp_path.display(),
                e
            )))
        })?;
        file.sync_all().map_err(|e| {
            IronBaseError::Io(std::io::Error::other(format!(
                "Failed to fsync HNSW temp file '{}': {}",
                temp_path.display(),
                e
            )))
        })?;
        drop(file);

        crate::fs_utils::atomic_rename_and_sync(&temp_path, cache_path)?;

        let file_size = cache_path.metadata().map(|m| m.len()).unwrap_or(0);
        Ok(file_size)
    }

    /// Flush all dirty vector indexes to `.hnsw` files. Returns the count flushed.
    pub fn flush_vector_indexes(&mut self, db_path: &str, watermark: u64) -> Result<usize> {
        let mut count = 0;
        let dirty_names: Vec<String> = self.dirty_vector_indexes.iter().cloned().collect();
        for name in dirty_names {
            if let Some(index) = self.vector_indexes.get_mut(&name) {
                index.set_flushed_tx_id(watermark);
                let cache_path = Self::build_vector_cache_path(db_path, &name);
                let file_size = Self::persist_hnsw_to_file(index, &cache_path)?;
                index.mark_clean();
                self.dirty_vector_indexes.remove(&name);
                crate::log_debug!(
                    "Flushed vector index '{}' to {} ({:.1} MB, atomic)",
                    name,
                    cache_path.display(),
                    file_size as f64 / (1024.0 * 1024.0)
                );
                count += 1;
            }
        }
        Ok(count)
    }

    /// Flush a single dirty vector index. `Ok(true)` if flushed, `Ok(false)` if
    /// not dirty / not found.
    pub fn flush_one_vector_index(
        &mut self,
        name: &str,
        db_path: &str,
        watermark: u64,
    ) -> Result<bool> {
        if !self.dirty_vector_indexes.contains(name) {
            return Ok(false);
        }
        if let Some(index) = self.vector_indexes.get_mut(name) {
            index.set_flushed_tx_id(watermark);
            let cache_path = Self::build_vector_cache_path(db_path, name);
            let file_size = Self::persist_hnsw_to_file(index, &cache_path)?;
            index.mark_clean();
            self.dirty_vector_indexes.remove(name);
            crate::log_debug!(
                "Flushed vector index '{}' to {} ({:.1} MB, atomic)",
                name,
                cache_path.display(),
                file_size as f64 / (1024.0 * 1024.0)
            );
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Rebuild vector indexes with excessive orphan nodes (>30% per
    /// `rebuild_if_needed`). Returns the number rebuilt.
    pub fn rebuild_vector_indexes_if_needed(&mut self) -> Result<usize> {
        let mut rebuilt = 0;
        let names: Vec<String> = self.vector_indexes.keys().cloned().collect();
        for name in names {
            if let Some(index) = self.vector_indexes.get_mut(&name) {
                if index.rebuild_if_needed()? {
                    self.dirty_vector_indexes.insert(name.clone());
                    rebuilt += 1;
                }
            }
        }
        Ok(rebuilt)
    }

    /// Rebuild ALL vector indexes unconditionally (used during `db_compact`).
    pub fn rebuild_all_vector_indexes(&mut self) -> Result<usize> {
        let mut rebuilt = 0;
        let names: Vec<String> = self.vector_indexes.keys().cloned().collect();
        for name in names {
            if let Some(index) = self.vector_indexes.get_mut(&name) {
                let orphans = index.orphan_count();
                if orphans > 0 {
                    let active = index.len();
                    index.rebuild()?;
                    self.dirty_vector_indexes.insert(name.clone());
                    crate::log_info!(
                        "HNSW index '{}' compacted: removed {} orphan nodes, {} active vectors remain",
                        name,
                        orphans,
                        active
                    );
                    rebuilt += 1;
                }
            }
        }
        Ok(rebuilt)
    }

    /// Build the path for an HNSW cache file.
    pub fn build_vector_cache_path(db_path: &str, index_name: &str) -> PathBuf {
        let db_path_buf = PathBuf::from(db_path);
        let parent = db_path_buf.parent().unwrap_or(&db_path_buf);
        parent.join(format!("{}.hnsw", index_name))
    }

    /// Try to load a vector index from its `.hnsw` cache file.
    pub fn try_load_vector_index(db_path: &str, index_name: &str) -> Option<HnswIndex> {
        let cache_path = Self::build_vector_cache_path(db_path, index_name);
        if !cache_path.exists() {
            return None;
        }

        match std::fs::read(&cache_path) {
            Ok(bytes) => match HnswIndex::from_bytes(&bytes) {
                Ok(index) => {
                    crate::log_debug!(
                        "Loaded vector index '{}' from {} ({} vectors)",
                        index_name,
                        cache_path.display(),
                        index.len()
                    );
                    Some(index)
                }
                Err(e) => {
                    crate::log_warn!(
                        "Failed to deserialize vector index '{}' from {}: {}",
                        index_name,
                        cache_path.display(),
                        e
                    );
                    None
                }
            },
            Err(e) => {
                crate::log_warn!(
                    "Failed to read vector index cache '{}': {}",
                    cache_path.display(),
                    e
                );
                None
            }
        }
    }
}
