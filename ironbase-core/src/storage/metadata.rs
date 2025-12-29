// storage/metadata.rs
// Metadata management for storage engine

use super::{CollectionMeta, Header, StorageEngine};
use crate::error::{IronBaseError, Result};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};

/// Maximum allowed size for a single collection metadata entry.
/// Protects against DoS via malicious files with corrupted length fields.
/// 64 MB allows ~2-4 million documents per collection with ObjectId keys.
const MAX_METADATA_SIZE: usize = 64 * 1024 * 1024;

impl StorageEngine {
    /// Load metadata from file (supports both legacy and dynamic formats)
    ///
    /// MIGRATION (v2 → v3): If data_end_offset is 0 (v2 database), it will be
    /// calculated from the document catalog on first write. The header is NOT
    /// updated here to avoid unnecessary I/O - migration happens lazily.
    pub(super) fn load_metadata(
        file: &mut File,
    ) -> Result<(Header, HashMap<String, CollectionMeta>)> {
        file.seek(SeekFrom::Start(0))?;

        // Read header (with dynamic size for version 2+)
        let mut header_bytes = vec![0u8; 256]; // Max header size
        file.read_exact(&mut header_bytes)?;

        let mut header: Header = bincode::deserialize(&header_bytes)
            .map_err(|e| IronBaseError::Corruption(format!("Invalid header: {}", e)))?;

        // Magic number check
        if &header.magic != b"MONGOLTE" {
            return Err(IronBaseError::Corruption("Invalid magic number".into()));
        }

        // Load collections based on version
        let collections = if header.version >= 2 && header.metadata_offset > 0 {
            // Version 2+: Dynamic metadata at end of file
            Self::load_metadata_dynamic(file, &header)?
        } else {
            // Version 1: Legacy format (metadata after header)
            Self::load_metadata_legacy(file, &header)?
        };

        // MIGRATION (v2 → v3): Calculate data_end_offset if not set
        // This ensures existing v2 databases work correctly with the new code
        if header.data_end_offset == 0 && !collections.is_empty() {
            let (max_doc_offset, has_docs) = Self::find_max_document_offset(&collections);
            if has_docs {
                // Calculate data_end from last document
                if let Ok(data_end) = Self::calculate_data_end_from_catalog(file, max_doc_offset) {
                    header.data_end_offset = data_end;
                } else {
                    // Fallback: use metadata_offset as boundary
                    header.data_end_offset = header.metadata_offset;
                }
            } else {
                // No documents - data starts right after header
                header.data_end_offset = super::HEADER_SIZE;
            }
        } else if header.data_end_offset == 0 {
            // Empty database - initialize to HEADER_SIZE
            header.data_end_offset = super::HEADER_SIZE;
        }

        Ok((header, collections))
    }

    /// Calculate data end offset from the last document in catalog
    /// Used during v2 → v3 migration
    fn calculate_data_end_from_catalog(file: &mut File, max_doc_offset: u64) -> Result<u64> {
        file.seek(SeekFrom::Start(max_doc_offset))?;

        // Read document length (4 bytes)
        let mut len_bytes = [0u8; 4];
        file.read_exact(&mut len_bytes)?;
        let doc_len = u32::from_le_bytes(len_bytes) as u64;

        // Sanity check
        if doc_len > 16 * 1024 * 1024 {
            return Err(IronBaseError::Corruption(format!(
                "Migration: suspiciously large document size: {} bytes",
                doc_len
            )));
        }

        Ok(max_doc_offset + 4 + doc_len)
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

            // DoS protection: validate metadata size before allocation
            if len == 0 || len > MAX_METADATA_SIZE {
                return Err(IronBaseError::Corruption(format!(
                    "Collection metadata size {} exceeds maximum {} bytes",
                    len, MAX_METADATA_SIZE
                )));
            }

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

            // DoS protection: validate metadata size before allocation
            if len == 0 || len > MAX_METADATA_SIZE {
                return Err(IronBaseError::Corruption(format!(
                    "Collection metadata size {} exceeds maximum {} bytes",
                    len, MAX_METADATA_SIZE
                )));
            }

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

        // Write header
        let mut header_bytes =
            bincode::serialize(header).map_err(|e| IronBaseError::Serialization(e.to_string()))?;
        if header_bytes.len() < super::HEADER_SIZE as usize {
            header_bytes.resize(super::HEADER_SIZE as usize, 0);
        }
        writer.write_all(&header_bytes)?;

        // Write collection metadata
        // IMPORTANT: JSON serialization uses custom catalog_serde module,
        // which preserves DocumentId type info in [type_tag, value, offset] format
        for meta in collections.values() {
            let meta_bytes = serde_json::to_vec(meta)?;
            let len = (meta_bytes.len() as u32).to_le_bytes();
            writer.write_all(&len)?;
            writer.write_all(&meta_bytes)?;
        }

        // Current position = end of metadata section
        let metadata_end = writer.stream_position()?;

        Ok(metadata_end)
    }

    /// Flush metadata to disk with DYNAMIC METADATA approach (version 2+)
    ///
    /// Metadata is written at the END of the file, not at fixed offset.
    ///
    /// CRITICAL FIX (v3): Uses header.data_end_offset directly instead of
    /// scanning catalogs. This prevents the old bug where find_max_document_offset()
    /// could return stale values after metadata was written.
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

        // 3. Determine metadata write position using data_end_offset (v3 approach)
        // This is simpler and more reliable than scanning catalogs
        let metadata_offset = if self.header.data_end_offset >= super::HEADER_SIZE {
            // v3: Use explicit data_end_offset
            self.header.data_end_offset
        } else {
            // Migration from v2: calculate from catalog
            let (max_doc_offset, has_documents) = Self::find_max_document_offset(&self.collections);
            let file_len = self.file.metadata()?.len();

            if has_documents {
                // Calculate end of last document
                Self::calculate_metadata_offset(&mut self.file, max_doc_offset, file_len)?
            } else {
                // No documents - write metadata right after header
                super::HEADER_SIZE
            }
        };

        // CRITICAL FIX: Update data_end_offset to AFTER metadata BEFORE writing header
        // Without this, next document write would overwrite the metadata!
        // The layout is: [HEADER][DOCUMENTS][METADATA]
        // After flush, next document should be written AFTER metadata.
        let new_data_end = metadata_offset + metadata_bytes.len() as u64;
        self.header.data_end_offset = new_data_end;

        // 4. Write metadata and header atomically (header now includes correct data_end_offset)
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
                    return Err(IronBaseError::Corruption(format!(
                        "Invalid metadata offset calculation: {} > file_len {}",
                        calculated_offset, file_len
                    )));
                }

                // Additional sanity check: doc_len should be reasonable (< 16MB)
                if doc_len > 16 * 1024 * 1024 {
                    return Err(IronBaseError::Corruption(format!(
                        "Suspiciously large document size: {} bytes at offset {}",
                        doc_len, max_doc_offset
                    )));
                }

                Ok(calculated_offset)
            }
            Err(e) => {
                // Failed to read document - file might be corrupt
                Err(IronBaseError::Corruption(format!(
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
    pub(crate) fn find_max_document_offset(
        collections: &HashMap<String, CollectionMeta>,
    ) -> (u64, bool) {
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
            bincode::serialize(header).map_err(|e| IronBaseError::Serialization(e.to_string()))?;
        file.write_all(&header_bytes)?;

        // 5. Sync all changes to disk
        file.sync_all()?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DocumentId;
    use crate::storage::{CollectionFlags, CollectionMeta, Header, HEADER_SIZE};
    use std::io::Cursor;
    use tempfile::NamedTempFile;

    fn create_test_header() -> Header {
        Header {
            magic: *b"MONGOLTE",
            version: 4,
            page_size: 4096,
            collection_count: 0,
            free_list_head: 0,
            index_section_offset: 0,
            metadata_offset: 0,
            metadata_size: 0,
            data_end_offset: HEADER_SIZE,
            clean_shutdown: false,
        }
    }

    fn create_test_collection(name: &str) -> CollectionMeta {
        let mut meta = CollectionMeta {
            name: name.to_string(),
            document_count: 0,
            live_document_count: 0,
            data_offset: 0,
            index_offset: 0,
            last_id: 0,
            document_catalog: HashMap::new(),
            indexes: Vec::new(),
            fuzzy_indexes: Vec::new(),
            fulltext_indexes: Vec::new(),
            schema: None,
            flags: CollectionFlags::default(),
        };
        meta.document_catalog.insert(DocumentId::Int(1), 1000);
        meta.document_catalog.insert(DocumentId::Int(2), 2000);
        meta
    }

    #[test]
    fn test_find_max_document_offset_empty() {
        let collections: HashMap<String, CollectionMeta> = HashMap::new();
        let (max_offset, has_docs) = StorageEngine::find_max_document_offset(&collections);
        assert_eq!(max_offset, 0);
        assert!(!has_docs);
    }

    #[test]
    fn test_find_max_document_offset_with_docs() {
        let mut collections = HashMap::new();
        let mut meta = create_test_collection("test");
        meta.document_catalog.clear();
        meta.document_catalog.insert(DocumentId::Int(1), 1000);
        meta.document_catalog.insert(DocumentId::Int(2), 5000);
        meta.document_catalog.insert(DocumentId::Int(3), 3000);
        collections.insert("test".to_string(), meta);

        let (max_offset, has_docs) = StorageEngine::find_max_document_offset(&collections);
        assert_eq!(max_offset, 5000);
        assert!(has_docs);
    }

    #[test]
    fn test_find_max_document_offset_multiple_collections() {
        let mut collections = HashMap::new();

        let mut meta1 = create_test_collection("coll1");
        meta1.document_catalog.clear();
        meta1.document_catalog.insert(DocumentId::Int(1), 1000);
        meta1.document_catalog.insert(DocumentId::Int(2), 2000);
        collections.insert("coll1".to_string(), meta1);

        let mut meta2 = create_test_collection("coll2");
        meta2.document_catalog.clear();
        meta2
            .document_catalog
            .insert(DocumentId::String("a".to_string()), 8000);
        meta2
            .document_catalog
            .insert(DocumentId::ObjectId("abc123".to_string()), 3000);
        collections.insert("coll2".to_string(), meta2);

        let (max_offset, has_docs) = StorageEngine::find_max_document_offset(&collections);
        assert_eq!(max_offset, 8000);
        assert!(has_docs);
    }

    #[test]
    fn test_serialize_metadata_empty() {
        let collections: HashMap<String, CollectionMeta> = HashMap::new();
        let bytes = StorageEngine::serialize_metadata(&collections).unwrap();

        // Should contain collection count (4 bytes) = 0
        assert_eq!(bytes.len(), 4);
        assert_eq!(
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            0
        );
    }

    #[test]
    fn test_serialize_metadata_with_collections() {
        let mut collections = HashMap::new();
        collections.insert("test".to_string(), create_test_collection("test"));

        let bytes = StorageEngine::serialize_metadata(&collections).unwrap();

        // Should contain: collection count (4 bytes) + length (4 bytes) + JSON data
        assert!(bytes.len() > 8);

        // First 4 bytes = collection count = 1
        assert_eq!(
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            1
        );
    }

    #[test]
    fn test_write_metadata_body() {
        let mut collections = HashMap::new();
        collections.insert("users".to_string(), create_test_collection("users"));
        collections.insert("orders".to_string(), create_test_collection("orders"));

        let mut buffer = Cursor::new(Vec::new());
        StorageEngine::write_metadata_body(&mut buffer, &collections).unwrap();

        let data = buffer.into_inner();

        // Collection count should be 2
        let count = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        assert_eq!(count, 2);
    }

    #[test]
    fn test_write_metadata_to_cursor() {
        let header = create_test_header();
        let mut collections = HashMap::new();
        collections.insert("test".to_string(), create_test_collection("test"));

        let mut buffer = Cursor::new(vec![0u8; 1024]);
        let end_offset = StorageEngine::write_metadata(&mut buffer, &header, &collections).unwrap();

        assert!(end_offset > HEADER_SIZE as u64);
    }

    #[test]
    fn test_load_metadata_invalid_magic() {
        let temp_file = NamedTempFile::new().unwrap();
        let mut file = temp_file.reopen().unwrap();

        // Write invalid header with wrong magic
        let mut header = create_test_header();
        header.magic = *b"BADMAGIC";
        let header_bytes = bincode::serialize(&header).unwrap();

        let mut padded = vec![0u8; 256];
        padded[..header_bytes.len()].copy_from_slice(&header_bytes);

        use std::io::Write;
        file.write_all(&padded).unwrap();
        file.sync_all().unwrap();

        // Try to load - should fail
        let mut read_file = temp_file.reopen().unwrap();
        let result = StorageEngine::load_metadata(&mut read_file);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid magic"));
    }

    #[test]
    fn test_load_metadata_dynamic_format() {
        let temp_file = NamedTempFile::new().unwrap();
        let mut file = temp_file.reopen().unwrap();

        // Create header with dynamic metadata offset
        let mut header = create_test_header();
        header.version = 3;
        header.metadata_offset = 512; // Metadata at offset 512
        header.collection_count = 1;

        // Write header (padded to 256 bytes)
        let header_bytes = bincode::serialize(&header).unwrap();
        let mut padded_header = vec![0u8; 256];
        padded_header[..header_bytes.len()].copy_from_slice(&header_bytes);

        use std::io::Write;
        file.write_all(&padded_header).unwrap();

        // Write padding until metadata offset
        let padding = vec![0u8; (512 - 256) as usize];
        file.write_all(&padding).unwrap();

        // Write metadata: collection count = 1
        let count_bytes = 1u32.to_le_bytes();
        file.write_all(&count_bytes).unwrap();

        // Write collection metadata
        let meta = create_test_collection("dynamic_test");
        let meta_json = serde_json::to_vec(&meta).unwrap();
        let len_bytes = (meta_json.len() as u32).to_le_bytes();
        file.write_all(&len_bytes).unwrap();
        file.write_all(&meta_json).unwrap();
        file.sync_all().unwrap();

        // Load and verify
        let mut read_file = temp_file.reopen().unwrap();
        let (loaded_header, collections) = StorageEngine::load_metadata(&mut read_file).unwrap();

        assert_eq!(loaded_header.version, 3);
        assert_eq!(collections.len(), 1);
        assert!(collections.contains_key("dynamic_test"));
    }

    #[test]
    fn test_load_metadata_legacy_format() {
        let temp_file = NamedTempFile::new().unwrap();
        let mut file = temp_file.reopen().unwrap();

        // Create legacy header (version 1, metadata_offset = 0)
        let mut header = create_test_header();
        header.version = 1;
        header.metadata_offset = 0; // Legacy: no dynamic metadata
        header.collection_count = 1;

        // Write header (padded to 256 bytes)
        let header_bytes = bincode::serialize(&header).unwrap();
        let mut padded_header = vec![0u8; 256];
        padded_header[..header_bytes.len()].copy_from_slice(&header_bytes);

        use std::io::Write;
        file.write_all(&padded_header).unwrap();

        // Write collection metadata immediately after header (legacy format)
        let meta = create_test_collection("legacy_test");
        let meta_json = serde_json::to_vec(&meta).unwrap();
        let len_bytes = (meta_json.len() as u32).to_le_bytes();
        file.write_all(&len_bytes).unwrap();
        file.write_all(&meta_json).unwrap();
        file.sync_all().unwrap();

        // Load and verify
        let mut read_file = temp_file.reopen().unwrap();
        let (loaded_header, collections) = StorageEngine::load_metadata(&mut read_file).unwrap();

        assert_eq!(loaded_header.version, 1);
        assert_eq!(collections.len(), 1);
        assert!(collections.contains_key("legacy_test"));
    }

    #[test]
    fn test_metadata_size_limit_protection() {
        let temp_file = NamedTempFile::new().unwrap();
        let mut file = temp_file.reopen().unwrap();

        // Create header
        let mut header = create_test_header();
        header.version = 3;
        header.metadata_offset = 256;
        header.collection_count = 1;

        // Write header
        let header_bytes = bincode::serialize(&header).unwrap();
        let mut padded_header = vec![0u8; 256];
        padded_header[..header_bytes.len()].copy_from_slice(&header_bytes);

        use std::io::Write;
        file.write_all(&padded_header).unwrap();

        // Write metadata with oversized length (DoS protection test)
        let count_bytes = 1u32.to_le_bytes();
        file.write_all(&count_bytes).unwrap();

        // Write invalid metadata length (larger than MAX_METADATA_SIZE)
        let huge_len = (100 * 1024 * 1024u32).to_le_bytes(); // 100MB
        file.write_all(&huge_len).unwrap();
        file.sync_all().unwrap();

        // Load should fail with size limit error
        let mut read_file = temp_file.reopen().unwrap();
        let result = StorageEngine::load_metadata(&mut read_file);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("exceeds maximum"));
    }

    #[test]
    fn test_metadata_zero_size_protection() {
        let temp_file = NamedTempFile::new().unwrap();
        let mut file = temp_file.reopen().unwrap();

        // Create header
        let mut header = create_test_header();
        header.version = 3;
        header.metadata_offset = 256;
        header.collection_count = 1;

        // Write header
        let header_bytes = bincode::serialize(&header).unwrap();
        let mut padded_header = vec![0u8; 256];
        padded_header[..header_bytes.len()].copy_from_slice(&header_bytes);

        use std::io::Write;
        file.write_all(&padded_header).unwrap();

        // Write metadata with zero length
        let count_bytes = 1u32.to_le_bytes();
        file.write_all(&count_bytes).unwrap();
        let zero_len = 0u32.to_le_bytes();
        file.write_all(&zero_len).unwrap();
        file.sync_all().unwrap();

        // Load should fail with zero size error
        let mut read_file = temp_file.reopen().unwrap();
        let result = StorageEngine::load_metadata(&mut read_file);
        assert!(result.is_err());
    }

    #[test]
    fn test_calculate_data_end_from_catalog() {
        let temp_file = NamedTempFile::new().unwrap();
        let mut file = temp_file.reopen().unwrap();

        // Write a fake document at offset 1000
        // Document format: 4-byte length + data
        use std::io::{Seek, Write};
        file.seek(SeekFrom::Start(1000)).unwrap();

        let doc_data = b"{\"_id\":1,\"name\":\"test\"}";
        let doc_len = doc_data.len() as u32;
        file.write_all(&doc_len.to_le_bytes()).unwrap();
        file.write_all(doc_data).unwrap();
        file.sync_all().unwrap();

        // Calculate data end
        let mut read_file = temp_file.reopen().unwrap();
        let data_end =
            StorageEngine::calculate_data_end_from_catalog(&mut read_file, 1000).unwrap();

        // Should be: offset (1000) + length field (4) + doc data length
        assert_eq!(data_end, 1000 + 4 + doc_data.len() as u64);
    }

    #[test]
    fn test_calculate_data_end_large_doc_protection() {
        let temp_file = NamedTempFile::new().unwrap();
        let mut file = temp_file.reopen().unwrap();

        // Write a document with suspiciously large size
        use std::io::{Seek, Write};
        file.seek(SeekFrom::Start(1000)).unwrap();

        // Write a size larger than 16MB (suspicious)
        let huge_size = 20 * 1024 * 1024u32; // 20MB
        file.write_all(&huge_size.to_le_bytes()).unwrap();
        file.sync_all().unwrap();

        // Should fail with corruption error
        let mut read_file = temp_file.reopen().unwrap();
        let result = StorageEngine::calculate_data_end_from_catalog(&mut read_file, 1000);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("suspiciously large"));
    }
}
