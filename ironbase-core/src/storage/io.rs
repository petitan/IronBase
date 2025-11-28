// storage/io.rs
// Low-level I/O operations for storage engine

use super::StorageEngine;
use crate::error::Result;
use std::io::{Read, Seek, SeekFrom, Write};

impl StorageEngine {
    /// Write data to end of file
    /// Returns the offset where data was written
    pub fn write_data(&mut self, data: &[u8]) -> Result<u64> {
        let offset = self.file.seek(SeekFrom::End(0))?;

        // Méret + adat írása
        let len = (data.len() as u32).to_le_bytes();
        self.file.write_all(&len)?;
        self.file.write_all(data)?;

        self.metadata_dirty = true;
        Ok(offset)
    }

    /// Read data from specified offset
    pub fn read_data(&mut self, offset: u64) -> Result<Vec<u8>> {
        use crate::error::MongoLiteError;

        // CRITICAL FIX: Validate offset is within file bounds BEFORE reading
        // Prevents race condition where flush_metadata() truncates file while reading
        let file_len = self.file.metadata()?.len();

        if offset >= file_len {
            return Err(MongoLiteError::Corruption(format!(
                "Attempted to read at offset {} but file is only {} bytes (likely metadata truncation race)",
                offset, file_len
            )));
        }

        // Additional validation: Ensure we can read at least the length header (4 bytes)
        if offset + 4 > file_len {
            return Err(MongoLiteError::Corruption(format!(
                "Insufficient space to read length header at offset {} (file: {} bytes)",
                offset, file_len
            )));
        }

        self.file.seek(SeekFrom::Start(offset))?;

        // Méret olvasása
        let mut len_bytes = [0u8; 4];
        self.file.read_exact(&mut len_bytes)?;
        let len = u32::from_le_bytes(len_bytes) as usize;

        // Validate document length is sane
        if len == 0 {
            return Err(MongoLiteError::Corruption(format!(
                "Document at offset {} has zero length (corrupted or truncated)",
                offset
            )));
        }

        // Validate we can read the full document
        if offset + 4 + (len as u64) > file_len {
            return Err(MongoLiteError::Corruption(format!(
                "Document at offset {} claims length {} but would exceed file boundary (file: {} bytes)",
                offset, len, file_len
            )));
        }

        // Adat olvasása
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
    pub fn write_document(
        &mut self,
        collection: &str,
        doc_id: &crate::document::DocumentId,
        data: &[u8],
    ) -> Result<u64> {
        use crate::error::MongoLiteError;

        // Append document after existing data
        let absolute_offset = self.file.seek(SeekFrom::End(0))?;

        // Write length + data (same format as write_data)
        let len = (data.len() as u32).to_le_bytes();
        self.file.write_all(&len)?;
        self.file.write_all(data)?;

        self.metadata_dirty = true;
        // Update catalog in metadata with ABSOLUTE offset
        // Direct insert using DocumentId (no serialization overhead!)
        let meta = self
            .get_collection_meta_mut(collection)
            .ok_or_else(|| MongoLiteError::CollectionNotFound(collection.to_string()))?;

        meta.document_catalog
            .insert(doc_id.clone(), absolute_offset);
        meta.document_count += 1; // CRITICAL: increment document count!

        if self.header.metadata_offset > super::HEADER_SIZE {
            self.metadata_dirty = true;
        }

        Ok(absolute_offset)
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
        use crate::error::MongoLiteError;

        // Append document after existing data
        let absolute_offset = self.file.seek(SeekFrom::End(0))?;

        // Write length + data (same format as write_data)
        let len = (data.len() as u32).to_le_bytes();
        self.file.write_all(&len)?;
        self.file.write_all(data)?;

        self.metadata_dirty = true;

        // Update ALL metadata fields in collection
        let meta = self
            .get_collection_meta_mut(collection)
            .ok_or_else(|| MongoLiteError::CollectionNotFound(collection.to_string()))?;

        // Check if this is an update (doc already exists in catalog)
        let is_update = meta.document_catalog.contains_key(doc_id);

        // Update catalog with new offset
        meta.document_catalog
            .insert(doc_id.clone(), absolute_offset);

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

        Ok(absolute_offset)
    }

    /// Write tombstone with full metadata update
    ///
    /// Used for deletes - writes a tombstone marker and updates all metadata
    pub fn write_tombstone_full(
        &mut self,
        collection: &str,
        doc_id: &crate::document::DocumentId,
    ) -> Result<()> {
        use crate::error::MongoLiteError;

        // Create tombstone document
        let tombstone = serde_json::json!({
            "_id": doc_id,
            "_collection": collection,
            "_tombstone": true
        });
        let tombstone_json = serde_json::to_string(&tombstone)
            .map_err(|e| MongoLiteError::Serialization(e.to_string()))?;

        // Write tombstone to file
        let _offset = self.file.seek(SeekFrom::End(0))?;
        let len = (tombstone_json.len() as u32).to_le_bytes();
        self.file.write_all(&len)?;
        self.file.write_all(tombstone_json.as_bytes())?;

        self.metadata_dirty = true;

        // Update metadata
        let meta = self
            .get_collection_meta_mut(collection)
            .ok_or_else(|| MongoLiteError::CollectionNotFound(collection.to_string()))?;

        // Remove from catalog
        if meta.document_catalog.remove(doc_id).is_some() {
            // Decrement live count only if document existed
            if meta.live_document_count > 0 {
                meta.live_document_count -= 1;
            }
        }

        Ok(())
    }
}
