// storage/compaction.rs
// Storage compaction functionality

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use serde_json::Value;
use crate::error::{Result, MongoLiteError};
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
        let mut all_docs: HashMap<String, HashMap<String, Value>> = HashMap::new();

        // Clone collections to avoid borrow conflicts
        let collections_snapshot = self.collections.clone();
        let file_len = self.file_len()?;

        // First pass: collect all latest document versions from ALL collections
        for (coll_name, coll_meta) in &collections_snapshot {
            let mut current_offset = coll_meta.data_offset;
            let mut docs_by_id: HashMap<String, Value> = HashMap::new();

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
                                    let id_key = serde_json::to_string(id_value)
                                        .unwrap_or_else(|_| "unknown".to_string());
                                    docs_by_id.insert(id_key, doc);
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

        // Second pass: Calculate final metadata size by doing a dry run
        let mut new_collections = self.collections.clone();

        // First, calculate where each collection's data will start and how many docs
        // We need to know this to calculate exact metadata size
        let mut collection_info: Vec<(String, u64, u64)> = Vec::new(); // (name, offset, count)

        for (coll_name, docs_by_id) in &all_docs {
            let doc_count = docs_by_id.iter()
                .filter(|(_, doc)| !doc.get("_tombstone").and_then(|v| v.as_bool()).unwrap_or(false))
                .count() as u64;
            collection_info.push((coll_name.clone(), 0, doc_count)); // offset will be calculated
        }

        // Update new_collections with document counts (offsets are still placeholder)
        for (coll_name, _, doc_count) in &collection_info {
            if let Some(coll_meta) = new_collections.get_mut(coll_name) {
                coll_meta.document_count = *doc_count;
            }
        }

        // Create temporary new file
        let mut new_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temp_path)?;

        // Write metadata with correct document counts to get exact metadata size
        let metadata_end = Self::write_metadata(&mut new_file, &self.header, &new_collections)?;

        // Now we know the exact metadata size, calculate collection offsets
        let mut write_offset = metadata_end;
        for (coll_name, _, _) in &collection_info {
            if let Some(coll_meta) = new_collections.get_mut(coll_name) {
                coll_meta.data_offset = write_offset;
                // Calculate how much space this collection's documents will take
                if let Some(docs_by_id) = all_docs.get(coll_name) {
                    for (_, doc) in docs_by_id {
                        if !doc.get("_tombstone").and_then(|v| v.as_bool()).unwrap_or(false) {
                            let doc_bytes = serde_json::to_vec(&doc)?;
                            write_offset += 4 + doc_bytes.len() as u64;
                        }
                    }
                }
            }
        }

        // Rewrite metadata with correct offsets
        new_file.seek(SeekFrom::Start(0))?;
        let final_metadata_end = Self::write_metadata(&mut new_file, &self.header, &new_collections)?;

        // Verify metadata size is stable
        if final_metadata_end != metadata_end {
            return Err(MongoLiteError::Corruption(
                format!("Metadata size unstable during compaction: {} -> {}", metadata_end, final_metadata_end)
            ));
        }

        // Third pass: write documents to new file
        write_offset = metadata_end;
        for (_coll_name, docs_by_id) in &all_docs {
            for (_, doc) in docs_by_id {
                // Skip tombstones (deleted documents)
                if doc.get("_tombstone").and_then(|v| v.as_bool()).unwrap_or(false) {
                    stats.tombstones_removed += 1;
                    continue;
                }

                // Write document to new file
                let doc_bytes = serde_json::to_vec(&doc)?;
                let len = doc_bytes.len() as u32;

                new_file.write_all(&len.to_le_bytes())?;
                new_file.write_all(&doc_bytes)?;

                write_offset += 4 + doc_bytes.len() as u64;
                stats.documents_kept += 1;
            }
        }

        new_file.sync_all()?;

        // Get new file size
        stats.size_after = new_file.metadata()?.len();

        // Close old file
        drop(std::mem::replace(&mut self.file, new_file));
        drop(self.mmap.take());

        // Replace old file with new file
        fs::rename(&temp_path, &self.file_path)?;

        // Reopen the compacted file
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.file_path)?;

        // Reload metadata
        let (header, collections) = Self::load_metadata(&mut file)?;

        // Update self
        self.file = file;
        self.header = header;
        self.collections = collections;
        self.mmap = None; // Reset mmap

        Ok(stats)
    }
}
