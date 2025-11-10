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
        // FONTOS: Bincode a Header-t 28 byte-ra szerializálja (8+4+4+4+8),
        // std::mem::size_of::<Header>() viszont 32-t mondana Rust struct padding miatt!
        // Ezért fix 28 byte-ot olvasunk, ami megfelel a bincode szerializált méretének.
        const HEADER_SIZE: usize = 28; // 8 (magic) + 4 (version) + 4 (page_size) + 4 (collection_count) + 8 (free_list_head)
        let mut header_bytes = vec![0u8; HEADER_SIZE];
        file.read_exact(&mut header_bytes)?;

        let header: Header = bincode::deserialize(&header_bytes)
            .map_err(|e| MongoLiteError::Corruption(format!("Invalid header: {}", e)))?;

        // Magic number ellenőrzése
        if &header.magic != b"MONGOLTE" {
            return Err(MongoLiteError::Corruption("Invalid magic number".into()));
        }

        // Collection-ök metaadatainak beolvasása
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

    /// Flush metadata to disk with iterative convergence
    pub(super) fn flush_metadata(&mut self) -> Result<()> {
        // Get current file size to preserve existing data
        let original_file_size = self.file.metadata()?.len();

        // Use iterative convergence to handle circular dependency
        let mut current_metadata_end = Self::write_metadata(&mut self.file, &self.header, &self.collections)?;

        // Iterate until convergence (max 5 iterations)
        for _ in 0..5 {
            // Update all collection data_offset values
            for meta in self.collections.values_mut() {
                meta.data_offset = current_metadata_end;
                meta.index_offset = current_metadata_end;
            }

            // Rewrite metadata with updated offsets
            let new_metadata_end = Self::write_metadata(&mut self.file, &self.header, &self.collections)?;

            // Check convergence
            if new_metadata_end == current_metadata_end {
                break;
            }

            current_metadata_end = new_metadata_end;
        }

        // Only truncate if there's no data yet (file size <= metadata end)
        // This preserves existing documents while removing metadata remnants during initial setup
        if original_file_size <= current_metadata_end {
            self.file.set_len(current_metadata_end)?;
        }

        self.file.sync_all()?;

        Ok(())
    }
}
