// storage/compaction.rs
// Storage compaction functionality

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use serde_json::Value;
use crate::error::{Result};
use super::StorageEngine;

/// Compaction statistics
#[derive(Debug, Clone, Default)]
pub struct CompactionStats {
    pub size_before: u64,
    pub size_after: u64,
    pub documents_scanned: u64,
    pub documents_kept: u64,
    pub tombstones_removed: u64,
}

impl CompactionStats {
    pub fn space_saved(&self) -> u64 {
        self.size_before.saturating_sub(self.size_after)
    }

    pub fn compression_ratio(&self) -> f64 {
        if self.size_before == 0 {
            0.0
        } else {
            (self.size_after as f64 / self.size_before as f64) * 100.0
        }
    }
}

impl StorageEngine {
    /// Storage compaction - removes tombstones and old document versions
    /// Creates a new file with only current, non-deleted documents
    pub fn compact(&mut self) -> Result<CompactionStats> {
        let temp_path = format!("{}.compact", self.file_path);
        let mut stats = CompactionStats::default();

        // Get current file size
        stats.size_before = self.file.metadata()?.len();

        // Track latest versions of each document by collection and ID
        let mut all_docs: HashMap<String, HashMap<crate::document::DocumentId, Value>> = HashMap::new();

        // Clone collections to avoid borrow conflicts
        let collections_snapshot = self.collections.clone();
        let file_len = self.file_len()?;

        // First pass: collect all latest document versions from ALL collections
        for (coll_name, coll_meta) in &collections_snapshot {
            let mut current_offset = coll_meta.data_offset;
            let mut docs_by_id: HashMap<crate::document::DocumentId, Value> = HashMap::new();

            // Scan all documents in this collection
            while current_offset < file_len {
                match self.read_data(current_offset) {
                    Ok(doc_bytes) => {
                        stats.documents_scanned += 1;

                        if let Ok(doc) = serde_json::from_slice::<Value>(&doc_bytes) {
                            // Check if this document belongs to this collection
                            let doc_collection = doc.get("_collection")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");

                            if doc_collection == coll_name {
                                if let Some(id_value) = doc.get("_id") {
                                    // Deserialize directly to DocumentId (no string conversion!)
                                    if let Ok(doc_id) = serde_json::from_value::<crate::document::DocumentId>(id_value.clone()) {
                                        docs_by_id.insert(doc_id, doc);
                                    }
                                }
                            }
                        }

                        current_offset += 4 + doc_bytes.len() as u64;
                    }
                    Err(_) => break,
                }
            }

            all_docs.insert(coll_name.clone(), docs_by_id);
        }

        // Second pass: Calculate final metadata size using iterative convergence
        let mut new_collections = self.collections.clone();

        // Calculate document counts for each collection
        for (coll_name, docs_by_id) in &all_docs {
            let doc_count = docs_by_id.iter()
                .filter(|(_, doc)| !doc.get("_tombstone").and_then(|v| v.as_bool()).unwrap_or(false))
                .count() as u64;

            if let Some(coll_meta) = new_collections.get_mut(coll_name) {
                coll_meta.document_count = doc_count;
            }
        }

        // Create temporary new file
        let mut new_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temp_path)?;

        // Use RESERVED SPACE architecture: metadata in first 256KB, documents after DATA_START_OFFSET
        // This matches the normal storage layout and avoids complex offset calculations

        // Write metadata first (placeholder with document_count but empty catalog)
        for coll_meta in new_collections.values_mut() {
            coll_meta.data_offset = super::DATA_START_OFFSET;
            coll_meta.document_catalog.clear();
        }

        new_file.seek(SeekFrom::Start(0))?;
        Self::write_metadata(&mut new_file, &self.header, &new_collections)?;

        // Write documents starting at DATA_START_OFFSET and build document_catalog
        new_file.seek(SeekFrom::Start(super::DATA_START_OFFSET))?;
        let mut write_offset = super::DATA_START_OFFSET;

        for (coll_name, docs_by_id) in &all_docs {
            for (doc_id, doc) in docs_by_id {
                // Skip tombstones (deleted documents)
                if doc.get("_tombstone").and_then(|v| v.as_bool()).unwrap_or(false) {
                    stats.tombstones_removed += 1;
                    continue;
                }

                // Write document to new file
                let doc_offset = write_offset;
                let doc_bytes = serde_json::to_vec(&doc)?;
                let len = doc_bytes.len() as u32;

                new_file.write_all(&len.to_le_bytes())?;
                new_file.write_all(&doc_bytes)?;

                write_offset += 4 + doc_bytes.len() as u64;
                stats.documents_kept += 1;

                // Update document_catalog with actual offset (direct DocumentId insert!)
                if let Some(coll_meta) = new_collections.get_mut(coll_name) {
                    coll_meta.document_catalog.insert(doc_id.clone(), doc_offset);
                }
            }
        }

        new_file.sync_all()?;

        // Now rewrite metadata with the populated document_catalog
        new_file.seek(SeekFrom::Start(0))?;
        Self::write_metadata(&mut new_file, &self.header, &new_collections)?;
        new_file.sync_all()?;

        // Get new file size
        stats.size_after = new_file.metadata()?.len();

        // Close new file before renaming
        drop(new_file);

        // Close old file and mmap
        drop(self.mmap.take());
        // Don't close self.file yet - we'll replace it after rename

        // Replace old file with new file
        fs::rename(&temp_path, &self.file_path)?;

        // Reopen the compacted file
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.file_path)?;

        // Reload metadata
        let (header, collections) = Self::load_metadata(&mut file)?;

        // Update self (this closes the old file)
        self.file = file;
        self.header = header;
        self.collections = collections;
        self.mmap = None; // Reset mmap

        Ok(stats)
    }
}
