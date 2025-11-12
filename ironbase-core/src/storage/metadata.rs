// storage/metadata.rs
// Metadata management for storage engine

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write, Seek, SeekFrom};
use crate::error::{Result, MongoLiteError};
use super::{StorageEngine, Header, CollectionMeta};

impl StorageEngine {
    /// Load metadata from file (supports both legacy and dynamic formats)
    pub(super) fn load_metadata(file: &mut File) -> Result<(Header, HashMap<String, CollectionMeta>)> {
        file.seek(SeekFrom::Start(0))?;

        // Read header (with dynamic size for version 2+)
        let mut header_bytes = vec![0u8; 256]; // Max header size
        file.read_exact(&mut header_bytes)?;

        let header: Header = bincode::deserialize(&header_bytes)
            .map_err(|e| MongoLiteError::Corruption(format!("Invalid header: {}", e)))?;

        // Magic number check
        if &header.magic != b"MONGOLTE" {
            return Err(MongoLiteError::Corruption("Invalid magic number".into()));
        }

        // Load collections based on version
        let collections = if header.version >= 2 && header.metadata_offset > 0 {
            // Version 2+: Dynamic metadata at end of file
            Self::load_metadata_dynamic(file, &header)?
        } else {
            // Version 1: Legacy format (metadata after header)
            Self::load_metadata_legacy(file, &header)?
        };

        Ok((header, collections))
    }

    /// Load metadata from dynamic location (version 2+)
    fn load_metadata_dynamic(file: &mut File, header: &Header) -> Result<HashMap<String, CollectionMeta>> {
        // Seek to metadata location
        file.seek(SeekFrom::Start(header.metadata_offset))?;

        // Read collection count
        let mut count_bytes = [0u8; 4];
        file.read_exact(&mut count_bytes)?;
        let collection_count = u32::from_le_bytes(count_bytes);

        // Read each collection
        let mut collections = HashMap::new();
        for _ in 0..collection_count {
            let mut len_bytes = [0u8; 4];
            file.read_exact(&mut len_bytes)?;
            let len = u32::from_le_bytes(len_bytes) as usize;

            let mut meta_bytes = vec![0u8; len];
            file.read_exact(&mut meta_bytes)?;

            let meta: CollectionMeta = serde_json::from_slice(&meta_bytes)?;
            collections.insert(meta.name.clone(), meta);
        }

        Ok(collections)
    }

    /// Load metadata from legacy fixed location (version 1)
    fn load_metadata_legacy(file: &mut File, header: &Header) -> Result<HashMap<String, CollectionMeta>> {
        // Metadata is right after header in legacy format
        file.seek(SeekFrom::Start(256))?; // After header

        let mut collections = HashMap::new();
        for _ in 0..header.collection_count {
            let mut len_bytes = [0u8; 4];
            file.read_exact(&mut len_bytes)?;
            let len = u32::from_le_bytes(len_bytes) as usize;

            let mut meta_bytes = vec![0u8; len];
            file.read_exact(&mut meta_bytes)?;

            let meta: CollectionMeta = serde_json::from_slice(&meta_bytes)?;
            collections.insert(meta.name.clone(), meta);
        }

        Ok(collections)
    }

    /// Write metadata to writer
    /// Returns the offset at the end of metadata section
    pub(super) fn write_metadata<W: Write + Seek>(
        writer: &mut W,
        header: &Header,
        collections: &HashMap<String, CollectionMeta>,
    ) -> Result<u64> {
        writer.seek(SeekFrom::Start(0))?;

        // Header kiírása
        let header_bytes = bincode::serialize(header)
            .map_err(|e| MongoLiteError::Serialization(e.to_string()))?;
        writer.write_all(&header_bytes)?;

        // Collection metaadatok kiírása
        // FONTOS: JSON serialization használja a custom catalog_serde modult,
        // ami megőrzi a DocumentId típusinformációt [type_tag, value, offset] formátumban
        for meta in collections.values() {
            let meta_bytes = serde_json::to_vec(meta)?;
            let len = (meta_bytes.len() as u32).to_le_bytes();
            writer.write_all(&len)?;
            writer.write_all(&meta_bytes)?;
        }

        // Jelenlegi pozíció = metadat szakasz vége
        let metadata_end = writer.stream_position()?;

        Ok(metadata_end)
    }

    /// Flush metadata to disk with DYNAMIC METADATA approach (version 2+)
    /// Metadata is written at the END of the file, not at fixed offset
    pub(crate) fn flush_metadata(&mut self) -> Result<()> {
        use std::io::Cursor;

        // Documents start right after header (no reserved space!)
        let data_offset = super::HEADER_SIZE;

        // Update all collection data_offset
        for meta in self.collections.values_mut() {
            meta.data_offset = data_offset;
            meta.index_offset = data_offset;
        }

        // Serialize metadata to buffer first to know its size
        let mut metadata_buffer = Cursor::new(Vec::new());
        Self::write_metadata_body(&mut metadata_buffer, &self.collections)?;
        let metadata_bytes = metadata_buffer.into_inner();
        let metadata_size = metadata_bytes.len() as u64;

        // CRITICAL FIX: Find actual end of document data by scanning catalog
        // Documents are written starting at HEADER_SIZE, we need to find where they end

        // Find the highest offset in all document catalogs
        let mut max_doc_offset: u64 = super::HEADER_SIZE;

        for coll_meta in self.collections.values() {
            for &doc_offset in coll_meta.document_catalog.values() {
                if doc_offset > max_doc_offset {
                    max_doc_offset = doc_offset;
                }
            }
        }

        // If we found any documents, read the last one to find its size
        let metadata_offset = if max_doc_offset > super::HEADER_SIZE {
            // Seek to the last document to read its size
            self.file.seek(SeekFrom::Start(max_doc_offset))?;

            // Read document length (4 bytes)
            let mut len_bytes = [0u8; 4];
            if self.file.read_exact(&mut len_bytes).is_ok() {
                let doc_len = u32::from_le_bytes(len_bytes) as u64;
                // Metadata starts after: offset + 4 (length) + doc_len
                max_doc_offset + 4 + doc_len
            } else {
                // Failed to read - use current file end
                self.file.metadata()?.len()
            }
        } else {
            // No documents yet - metadata right after header
            super::HEADER_SIZE
        };

        // Truncate file at metadata position to remove any old metadata
        self.file.set_len(metadata_offset)?;

        // Seek to metadata write position
        self.file.seek(SeekFrom::Start(metadata_offset))?;

        // Write metadata at end of file
        self.file.write_all(&metadata_bytes)?;

        // Update header with metadata location
        self.header.metadata_offset = metadata_offset;
        self.header.metadata_size = metadata_size;

        // Rewrite header with new metadata pointer
        self.file.seek(SeekFrom::Start(0))?;
        let header_bytes = bincode::serialize(&self.header)
            .map_err(|e| MongoLiteError::Serialization(e.to_string()))?;
        self.file.write_all(&header_bytes)?;

        self.file.sync_all()?;

        Ok(())
    }

    /// Write only the metadata body (collections), not header
    fn write_metadata_body<W: Write>(
        writer: &mut W,
        collections: &HashMap<String, CollectionMeta>,
    ) -> Result<()> {
        // Write collection count
        let count = (collections.len() as u32).to_le_bytes();
        writer.write_all(&count)?;

        // Write each collection metadata
        for meta in collections.values() {
            let meta_bytes = serde_json::to_vec(meta)?;
            let len = (meta_bytes.len() as u32).to_le_bytes();
            writer.write_all(&len)?;
            writer.write_all(&meta_bytes)?;
        }

        Ok(())
    }
}
