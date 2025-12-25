// storage/io.rs
// Low-level I/O operations for storage engine

use super::StorageEngine;
use crate::error::Result;
use std::io::{Read, Seek, SeekFrom, Write};

impl StorageEngine {
    /// Write data to end of document region
    /// Returns the offset where data was written
    ///
    /// CRITICAL FIX: Uses header.data_end_offset instead of SeekFrom::End(0)
    /// to prevent overwriting metadata that was previously flushed.
    pub fn write_data(&mut self, data: &[u8]) -> Result<u64> {
        // Determine write position from data_end_offset
        let write_offset = if self.header.data_end_offset >= super::HEADER_SIZE {
            self.header.data_end_offset
        } else {
            super::HEADER_SIZE
        };

        // Seek to write position
        self.file.seek(SeekFrom::Start(write_offset))?;

        // Write length + data
        let len = (data.len() as u32).to_le_bytes();
        self.file.write_all(&len)?;
        self.file.write_all(data)?;

        // Update data_end_offset
        self.header.data_end_offset = self.file.stream_position()?;

        self.metadata_dirty = true;
        Ok(write_offset)
    }

    /// Read data from specified offset
    pub fn read_data(&mut self, offset: u64) -> Result<Vec<u8>> {
        use crate::error::IronBaseError;

        // CRITICAL FIX: Validate offset is within file bounds BEFORE reading
        // Prevents race condition where flush_metadata() truncates file while reading
        let file_len = self.file.metadata()?.len();

        if offset >= file_len {
            return Err(IronBaseError::Corruption(format!(
                "Attempted to read at offset {} but file is only {} bytes (likely metadata truncation race)",
                offset, file_len
            )));
        }

        // Additional validation: Ensure we can read at least the length header (4 bytes)
        if offset + 4 > file_len {
            return Err(IronBaseError::Corruption(format!(
                "Insufficient space to read length header at offset {} (file: {} bytes)",
                offset, file_len
            )));
        }

        self.file.seek(SeekFrom::Start(offset))?;

        // Read length
        let mut len_bytes = [0u8; 4];
        self.file.read_exact(&mut len_bytes)?;
        let len = u32::from_le_bytes(len_bytes) as usize;

        // Validate document length is sane
        if len == 0 {
            return Err(IronBaseError::Corruption(format!(
                "Document at offset {} has zero length (corrupted or truncated)",
                offset
            )));
        }

        // Validate we can read the full document
        if offset + 4 + (len as u64) > file_len {
            return Err(IronBaseError::Corruption(format!(
                "Document at offset {} claims length {} but would exceed file boundary (file: {} bytes)",
                offset, len, file_len
            )));
        }

        // Read data
        let mut data = vec![0u8; len];
        self.file.read_exact(&mut data)?;

        Ok(data)
    }

    /// Get file length
    pub fn file_len(&self) -> Result<u64> {
        Ok(self.file.metadata()?.len())
    }

    /// Write document and update catalog
    /// This is the new persistent write method that tracks document offsets
    /// Stores ABSOLUTE offsets in catalog for simplicity and correctness
    ///
    /// CRITICAL FIX: Uses header.data_end_offset instead of SeekFrom::End(0)
    /// to prevent writing documents into old metadata regions.
    pub fn write_document(
        &mut self,
        collection: &str,
        doc_id: &crate::document::DocumentId,
        data: &[u8],
    ) -> Result<u64> {
        use crate::error::IronBaseError;

        // Determine write position from data_end_offset (not SeekFrom::End!)
        // Migration: if data_end_offset is 0 (v2 database), use file end
        let write_offset = if self.header.data_end_offset >= super::HEADER_SIZE {
            self.header.data_end_offset
        } else {
            // Migration from v2: calculate from catalog or use file end
            let (max_offset, has_docs) = Self::find_max_document_offset(&self.collections);
            if has_docs {
                // Calculate end of last document
                Self::calculate_data_end_from_last_doc(&mut self.file, max_offset)
                    .unwrap_or(self.file.seek(SeekFrom::End(0))?)
            } else {
                super::HEADER_SIZE
            }
        };

        // Seek to write position
        self.file.seek(SeekFrom::Start(write_offset))?;

        // Write length + data (same format as write_data)
        let len = (data.len() as u32).to_le_bytes();
        self.file.write_all(&len)?;
        self.file.write_all(data)?;

        // Update data_end_offset to point after this document
        self.header.data_end_offset = self.file.stream_position()?;

        self.metadata_dirty = true;

        // Update catalog in metadata with ABSOLUTE offset
        let meta = self
            .get_collection_meta_mut(collection)
            .ok_or_else(|| IronBaseError::CollectionNotFound(collection.to_string()))?;

        meta.document_catalog.insert(doc_id.clone(), write_offset);
        meta.document_count += 1;

        Ok(write_offset)
    }

    /// Read document by offset (catalog-based retrieval)
    /// Takes an ABSOLUTE offset directly from catalog
    pub fn read_document_at(&mut self, _collection: &str, absolute_offset: u64) -> Result<Vec<u8>> {
        self.read_data(absolute_offset)
    }

    /// Write document with FULL metadata update - the unified path for both runtime and recovery
    ///
    /// This function updates ALL metadata fields:
    /// - document_catalog: doc_id → offset mapping
    /// - document_count: total document writes
    /// - live_document_count: count of live (non-tombstone) documents
    /// - last_id: tracks highest auto-increment ID (prevents _id collisions after recovery)
    ///
    /// CRITICAL FIX: Uses header.data_end_offset instead of SeekFrom::End(0)
    /// to prevent writing documents into old metadata regions.
    ///
    /// This is the ONLY function that should be used for writing documents during:
    /// - Normal runtime inserts/updates
    /// - WAL recovery
    /// - Transaction commit
    pub fn write_document_full(
        &mut self,
        collection: &str,
        doc_id: &crate::document::DocumentId,
        data: &[u8],
    ) -> Result<u64> {
        use crate::error::IronBaseError;

        // Determine write position from data_end_offset (not SeekFrom::End!)
        // Migration: if data_end_offset is 0 (v2 database), use file end
        let write_offset = if self.header.data_end_offset >= super::HEADER_SIZE {
            self.header.data_end_offset
        } else {
            // Migration from v2: calculate from catalog or use file end
            let (max_offset, has_docs) = Self::find_max_document_offset(&self.collections);
            if has_docs {
                Self::calculate_data_end_from_last_doc(&mut self.file, max_offset)
                    .unwrap_or(self.file.seek(SeekFrom::End(0))?)
            } else {
                super::HEADER_SIZE
            }
        };

        // Seek to write position
        self.file.seek(SeekFrom::Start(write_offset))?;

        // Write length + data (same format as write_data)
        let len = (data.len() as u32).to_le_bytes();
        self.file.write_all(&len)?;
        self.file.write_all(data)?;

        // Update data_end_offset to point after this document
        self.header.data_end_offset = self.file.stream_position()?;

        self.metadata_dirty = true;

        // Update ALL metadata fields in collection
        let meta = self
            .get_collection_meta_mut(collection)
            .ok_or_else(|| IronBaseError::CollectionNotFound(collection.to_string()))?;

        // Check if this is an update (doc already exists in catalog)
        let is_update = meta.document_catalog.contains_key(doc_id);

        // Update catalog with new offset
        meta.document_catalog.insert(doc_id.clone(), write_offset);

        // Update document_count (total writes)
        meta.document_count += 1;

        // Update live_document_count (only increment for new inserts, not updates)
        if !is_update {
            meta.live_document_count += 1;
        }

        // Update last_id to prevent _id collisions after recovery
        if let crate::document::DocumentId::Int(id_num) = doc_id {
            if (*id_num as u64) > meta.last_id {
                meta.last_id = *id_num as u64;
            }
        }

        Ok(write_offset)
    }

    /// Write tombstone with full metadata update
    ///
    /// Used for deletes - writes a tombstone marker and updates all metadata
    ///
    /// CRITICAL FIX: Uses header.data_end_offset instead of SeekFrom::End(0)
    pub fn write_tombstone_full(
        &mut self,
        collection: &str,
        doc_id: &crate::document::DocumentId,
    ) -> Result<()> {
        use crate::error::IronBaseError;

        // Create tombstone document
        let tombstone = serde_json::json!({
            "_id": doc_id,
            "_collection": collection,
            "_tombstone": true
        });
        let tombstone_json = serde_json::to_string(&tombstone)
            .map_err(|e| IronBaseError::Serialization(e.to_string()))?;

        // Determine write position from data_end_offset (not SeekFrom::End!)
        let write_offset = if self.header.data_end_offset >= super::HEADER_SIZE {
            self.header.data_end_offset
        } else {
            // Migration from v2: calculate from catalog or use file end
            let (max_offset, has_docs) = Self::find_max_document_offset(&self.collections);
            if has_docs {
                Self::calculate_data_end_from_last_doc(&mut self.file, max_offset)
                    .unwrap_or(self.file.seek(SeekFrom::End(0))?)
            } else {
                super::HEADER_SIZE
            }
        };

        // Seek to write position
        self.file.seek(SeekFrom::Start(write_offset))?;

        // Write tombstone
        let len = (tombstone_json.len() as u32).to_le_bytes();
        self.file.write_all(&len)?;
        self.file.write_all(tombstone_json.as_bytes())?;

        // Update data_end_offset
        self.header.data_end_offset = self.file.stream_position()?;

        self.metadata_dirty = true;

        // Update metadata
        let meta = self
            .get_collection_meta_mut(collection)
            .ok_or_else(|| IronBaseError::CollectionNotFound(collection.to_string()))?;

        // Remove from catalog
        if meta.document_catalog.remove(doc_id).is_some() {
            // Decrement live count only if document existed
            if meta.live_document_count > 0 {
                meta.live_document_count -= 1;
            }
        }

        Ok(())
    }

    /// Calculate data end position from the last document's offset
    /// Used for migration from v2 databases that don't have data_end_offset
    pub(crate) fn calculate_data_end_from_last_doc(
        file: &mut std::fs::File,
        max_doc_offset: u64,
    ) -> Result<u64> {
        use crate::error::IronBaseError;

        file.seek(SeekFrom::Start(max_doc_offset))?;

        // Read document length (4 bytes)
        let mut len_bytes = [0u8; 4];
        file.read_exact(&mut len_bytes).map_err(|e| {
            IronBaseError::Corruption(format!(
                "Failed to read document length at offset {}: {}",
                max_doc_offset, e
            ))
        })?;

        let doc_len = u32::from_le_bytes(len_bytes) as u64;

        // Sanity check: doc_len should be reasonable (< 16MB)
        if doc_len > 16 * 1024 * 1024 {
            return Err(IronBaseError::Corruption(format!(
                "Suspiciously large document size: {} bytes at offset {}",
                doc_len, max_doc_offset
            )));
        }

        // data_end = offset + 4 (length header) + doc_len
        Ok(max_doc_offset + 4 + doc_len)
    }
}
