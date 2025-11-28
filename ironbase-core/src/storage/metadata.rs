// storage/metadata.rs
// Metadata management for storage engine

use super::{CollectionMeta, Header, StorageEngine};
use crate::error::{MongoLiteError, Result};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};

impl StorageEngine {
    /// Load metadata from file (supports both legacy and dynamic formats)
    pub(super) fn load_metadata(
        file: &mut File,
    ) -> Result<(Header, HashMap<String, CollectionMeta>)> {
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
    fn load_metadata_dynamic(
        file: &mut File,
        header: &Header,
    ) -> Result<HashMap<String, CollectionMeta>> {
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
    fn load_metadata_legacy(
        file: &mut File,
        header: &Header,
    ) -> Result<HashMap<String, CollectionMeta>> {
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
        let mut header_bytes =
            bincode::serialize(header).map_err(|e| MongoLiteError::Serialization(e.to_string()))?;
        if header_bytes.len() < super::HEADER_SIZE as usize {
            header_bytes.resize(super::HEADER_SIZE as usize, 0);
        }
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
    ///
    /// Metadata is written at the END of the file, not at fixed offset.
    /// This function uses helper methods for cleaner separation of concerns:
    /// - find_max_document_offset(): Scan catalogs for document region end
    /// - serialize_metadata(): Convert collections to bytes
    /// - determine_metadata_offset(): Calculate write position (idempotent logic)
    /// - write_metadata_and_header(): Atomic write of metadata + header
    ///
    /// CRITICAL: No file truncation to prevent race conditions with concurrent reads.
    pub(crate) fn flush_metadata(&mut self) -> Result<()> {
        // 1. Update collection offsets (documents start right after header)
        let data_offset = super::HEADER_SIZE;
        for meta in self.collections.values_mut() {
            meta.data_offset = data_offset;
            meta.index_offset = data_offset;
        }

        // 2. Serialize metadata to buffer
        let metadata_bytes = Self::serialize_metadata(&self.collections)?;

        // 3. Find document region end
        let (max_doc_offset, has_documents) = Self::find_max_document_offset(&self.collections);

        // 4. Determine write position (idempotent logic)
        let file_len = self.file.metadata()?.len();
        let metadata_offset = Self::determine_metadata_offset(
            &mut self.file,
            &self.header,
            max_doc_offset,
            has_documents,
            file_len,
            self.metadata_dirty,
        )?;

        // 5. Write metadata and header atomically
        Self::write_metadata_and_header(
            &mut self.file,
            &mut self.header,
            &metadata_bytes,
            metadata_offset,
        )?;

        self.metadata_dirty = false;
        Ok(())
    }

    /// Calculate metadata offset by reading the last document's size
    /// Returns the offset where metadata should start
    fn calculate_metadata_offset(
        file: &mut File,
        max_doc_offset: u64,
        file_len: u64,
    ) -> Result<u64> {
        // Seek to the last document to read its size
        file.seek(SeekFrom::Start(max_doc_offset))?;

        // Read document length (4 bytes)
        let mut len_bytes = [0u8; 4];
        match file.read_exact(&mut len_bytes) {
            Ok(_) => {
                let doc_len = u32::from_le_bytes(len_bytes) as u64;
                let calculated_offset = max_doc_offset + 4 + doc_len;

                // VALIDATION: Ensure calculated offset is sane
                if calculated_offset > file_len {
                    return Err(MongoLiteError::Corruption(format!(
                        "Invalid metadata offset calculation: {} > file_len {}",
                        calculated_offset, file_len
                    )));
                }

                // Additional sanity check: doc_len should be reasonable (< 16MB)
                if doc_len > 16 * 1024 * 1024 {
                    return Err(MongoLiteError::Corruption(format!(
                        "Suspiciously large document size: {} bytes at offset {}",
                        doc_len, max_doc_offset
                    )));
                }

                Ok(calculated_offset)
            }
            Err(e) => {
                // Failed to read document - file might be corrupt
                Err(MongoLiteError::Corruption(format!(
                    "Failed to read document at offset {}: {}",
                    max_doc_offset, e
                )))
            }
        }
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

    // =========================================================================
    // REFACTORED HELPER FUNCTIONS (for flush_metadata decomposition)
    // =========================================================================

    /// Scan all collection catalogs to find the highest document offset
    ///
    /// Returns (max_offset, has_documents) tuple:
    /// - max_offset: The highest byte offset of any document in any collection
    /// - has_documents: true if at least one document exists
    fn find_max_document_offset(collections: &HashMap<String, CollectionMeta>) -> (u64, bool) {
        let mut max_offset: u64 = 0;
        let mut has_documents = false;

        for coll_meta in collections.values() {
            for &doc_offset in coll_meta.document_catalog.values() {
                has_documents = true;
                max_offset = max_offset.max(doc_offset);
            }
        }

        (max_offset, has_documents)
    }

    /// Serialize collection metadata to bytes
    ///
    /// Uses write_metadata_body() internally with a Cursor buffer.
    fn serialize_metadata(collections: &HashMap<String, CollectionMeta>) -> Result<Vec<u8>> {
        use std::io::Cursor;

        let mut buffer = Cursor::new(Vec::new());
        Self::write_metadata_body(&mut buffer, collections)?;
        Ok(buffer.into_inner())
    }

    /// Determine where metadata should be written
    ///
    /// This function encapsulates the idempotent offset calculation logic:
    /// - If metadata_dirty is false and valid metadata exists, reuse existing offset
    /// - If metadata_dirty is true, recalculate based on document region end
    /// - If no documents exist, append at file end (or HEADER_SIZE minimum)
    fn determine_metadata_offset(
        file: &mut File,
        header: &Header,
        max_doc_offset: u64,
        has_documents: bool,
        file_len: u64,
        metadata_dirty: bool,
    ) -> Result<u64> {
        // Check if we have existing valid metadata
        let has_valid_metadata = header.metadata_offset > 0
            && header.metadata_offset >= super::HEADER_SIZE
            && header.metadata_offset <= file_len;

        if has_valid_metadata {
            if metadata_dirty {
                // Metadata changed - recalculate position
                if has_documents {
                    Self::calculate_metadata_offset(file, max_doc_offset, file_len)
                } else {
                    Ok(file_len.max(super::HEADER_SIZE))
                }
            } else {
                // Metadata unchanged - reuse existing offset
                Ok(header.metadata_offset)
            }
        } else if has_documents {
            // No existing metadata - calculate from last document
            Self::calculate_metadata_offset(file, max_doc_offset, file_len)
        } else {
            // No documents yet - append at file end (at least HEADER_SIZE)
            Ok(file_len.max(super::HEADER_SIZE))
        }
    }

    /// Write metadata body and update header atomically
    ///
    /// Performs the following steps:
    /// 1. Seek to metadata position and write metadata bytes
    /// 2. Update header struct with new metadata location
    /// 3. Rewrite header at file start
    /// 4. Sync all changes to disk
    fn write_metadata_and_header(
        file: &mut File,
        header: &mut Header,
        metadata_bytes: &[u8],
        metadata_offset: u64,
    ) -> Result<()> {
        // 1. Seek to metadata position
        file.seek(SeekFrom::Start(metadata_offset))?;

        // 2. Write metadata body
        file.write_all(metadata_bytes)?;

        // 3. Update header with new metadata location
        header.metadata_offset = metadata_offset;
        header.metadata_size = metadata_bytes.len() as u64;

        // 4. Rewrite header at file start
        file.seek(SeekFrom::Start(0))?;
        let header_bytes =
            bincode::serialize(header).map_err(|e| MongoLiteError::Serialization(e.to_string()))?;
        file.write_all(&header_bytes)?;

        // 5. Sync all changes to disk
        file.sync_all()?;

        Ok(())
    }
}
