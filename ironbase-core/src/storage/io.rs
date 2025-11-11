// storage/io.rs
// Low-level I/O operations for storage engine

use std::io::{Read, Write, Seek, SeekFrom};
use crate::error::Result;
use super::StorageEngine;

impl StorageEngine {
    /// Write data to end of file
    /// Returns the offset where data was written
    pub fn write_data(&mut self, data: &[u8]) -> Result<u64> {
        let offset = self.file.seek(SeekFrom::End(0))?;

        // Méret + adat írása
        let len = (data.len() as u32).to_le_bytes();
        self.file.write_all(&len)?;
        self.file.write_all(data)?;

        Ok(offset)
    }

    /// Read data from specified offset
    pub fn read_data(&mut self, offset: u64) -> Result<Vec<u8>> {
        self.file.seek(SeekFrom::Start(offset))?;

        // Méret olvasása
        let mut len_bytes = [0u8; 4];
        self.file.read_exact(&mut len_bytes)?;
        let len = u32::from_le_bytes(len_bytes) as usize;

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
        data: &[u8]
    ) -> Result<u64> {
        use crate::error::MongoLiteError;

        // Ensure we write AFTER the reserved metadata space
        let file_end = self.file.seek(SeekFrom::End(0))?;
        let write_pos = std::cmp::max(file_end, super::DATA_START_OFFSET);
        let absolute_offset = self.file.seek(SeekFrom::Start(write_pos))?;

        // Write length + data (same format as write_data)
        let len = (data.len() as u32).to_le_bytes();
        self.file.write_all(&len)?;
        self.file.write_all(data)?;

        // Update catalog in metadata with ABSOLUTE offset
        let id_key = serde_json::to_string(doc_id)
            .map_err(|e| MongoLiteError::Serialization(e.to_string()))?;

        let meta = self.get_collection_meta_mut(collection)
            .ok_or_else(|| MongoLiteError::CollectionNotFound(collection.to_string()))?;

        meta.document_catalog.insert(id_key, absolute_offset);

        Ok(absolute_offset)
    }

    /// Read document by offset (catalog-based retrieval)
    /// Takes an ABSOLUTE offset directly from catalog
    pub fn read_document_at(&mut self, _collection: &str, absolute_offset: u64) -> Result<Vec<u8>> {
        self.read_data(absolute_offset)
    }
}
