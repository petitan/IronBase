// storage/metadata.rs
// Metadata management for storage engine

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write, Seek, SeekFrom};
use crate::error::{Result, MongoLiteError};
use super::{StorageEngine, Header, CollectionMeta};

impl StorageEngine {
    /// Load metadata from file
    pub(super) fn load_metadata(file: &mut File) -> Result<(Header, HashMap<String, CollectionMeta>)> {
        file.seek(SeekFrom::Start(0))?;

        // Header beolvasása
        // FONTOS: Bincode a Header-t szerializálja:
        // 8 (magic) + 4 (version) + 4 (page_size) + 4 (collection_count) + 8 (free_list_head) + 8 (index_section_offset) = 36 bytes
        const HEADER_SIZE: usize = 36;
        let mut header_bytes = vec![0u8; HEADER_SIZE];
        file.read_exact(&mut header_bytes)?;

        let header: Header = bincode::deserialize(&header_bytes)
            .map_err(|e| MongoLiteError::Corruption(format!("Invalid header: {}", e)))?;

        // Magic number ellenőrzése
        if &header.magic != b"MONGOLTE" {
            return Err(MongoLiteError::Corruption("Invalid magic number".into()));
        }

        // Collection-ök metaadatainak beolvasása
        // FONTOS: JSON serialization használja a custom catalog_serde modult,
        // ami megőrzi a DocumentId típusinformációt [type_tag, value, offset] formátumban
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

        Ok((header, collections))
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

    /// Flush metadata to disk with RESERVED SPACE approach
    pub(super) fn flush_metadata(&mut self) -> Result<()> {
        // Use FIXED data offset = HEADER + RESERVED_METADATA_SIZE
        // This prevents documents from being overwritten when metadata grows
        let data_offset = super::DATA_START_OFFSET;

        // Update all collection data_offset to the FIXED start position
        for meta in self.collections.values_mut() {
            meta.data_offset = data_offset;
            meta.index_offset = data_offset;
        }

        // Write metadata (will fit in reserved space or error if too large)
        let metadata_end = Self::write_metadata(&mut self.file, &self.header, &self.collections)?;

        // Verify metadata fits in reserved space
        if metadata_end > data_offset {
            return Err(MongoLiteError::Corruption(
                format!("Metadata size {} exceeds reserved space {}", metadata_end, data_offset)
            ));
        }

        // Ensure file is at least DATA_START_OFFSET long (fills reserved space with zeros if needed)
        let current_size = self.file.metadata()?.len();
        if current_size < data_offset {
            self.file.set_len(data_offset)?;
        }

        self.file.sync_all()?;

        Ok(())
    }
}
