// src/storage.rs
use std::fs::{File, OpenOptions};
use std::io::{Read, Write, Seek, SeekFrom};
use std::path::Path;
use std::collections::HashMap;
use memmap2::{MmapMut, MmapOptions};
use serde::{Serialize, Deserialize};
use crate::error::{Result, MongoLiteError};

/// Adatbázis fájl fejléc
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Header {
    pub magic: [u8; 8],           // "MONGOLTE"
    pub version: u32,              // Verzió szám
    pub page_size: u32,            // Oldal méret (default: 4KB)
    pub collection_count: u32,     // Collection-ök száma
    pub free_list_head: u64,       // Szabad blokkok lista kezdete
}

impl Default for Header {
    fn default() -> Self {
        Header {
            magic: *b"MONGOLTE",
            version: 1,
            page_size: 4096,
            collection_count: 0,
            free_list_head: 0,
        }
    }
}

/// Collection metaadatok
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CollectionMeta {
    pub name: String,
    pub document_count: u64,
    pub data_offset: u64,          // Adatok kezdő pozíciója
    pub index_offset: u64,         // Indexek kezdő pozíciója
    pub last_id: u64,              // Utolsó _id
}

/// Storage engine - fájl alapú tárolás
pub struct StorageEngine {
    file: File,
    mmap: Option<MmapMut>,
    header: Header,
    collections: HashMap<String, CollectionMeta>,
    file_path: String,
}

impl StorageEngine {
    /// Adatbázis megnyitása vagy létrehozása
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path_str = path.as_ref().to_string_lossy().to_string();
        let exists = path.as_ref().exists();
        
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)?;
        
        let (header, collections) = if exists && file.metadata()?.len() > 0 {
            // Meglévő adatbázis betöltése
            Self::load_metadata(&mut file)?
        } else {
            // Új adatbázis inicializálása
            let header = Header::default();
            let collections = HashMap::new();
            let _ = Self::write_metadata(&mut file, &header, &collections)?;
            (header, collections)
        };
        
        // Memory-mapped fájl (ha elég kicsi a fájl)
        let mmap = if file.metadata()?.len() < 1_000_000_000 {  // 1GB alatt használjuk az mmap-et
            let mmap = unsafe { MmapOptions::new().map_mut(&file).ok() };
            mmap
        } else {
            None
        };
        
        Ok(StorageEngine {
            file,
            mmap,
            header,
            collections,
            file_path: path_str,
        })
    }
    
    /// Metaadatok betöltése
    fn load_metadata(file: &mut File) -> Result<(Header, HashMap<String, CollectionMeta>)> {
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
    
    /// Metaadatok kiírása
    /// Visszaadja a metadat szakasz végének offsetjét
    fn write_metadata(
        file: &mut File,
        header: &Header,
        collections: &HashMap<String, CollectionMeta>,
    ) -> Result<u64> {
        file.seek(SeekFrom::Start(0))?;

        // Header kiírása
        let header_bytes = bincode::serialize(header)
            .map_err(|e| MongoLiteError::Serialization(e.to_string()))?;
        file.write_all(&header_bytes)?;

        // Collection metaadatok kiírása
        for meta in collections.values() {
            let meta_bytes = serde_json::to_vec(meta)?;
            let len = (meta_bytes.len() as u32).to_le_bytes();
            file.write_all(&len)?;
            file.write_all(&meta_bytes)?;
        }

        // Jelenlegi pozíció = metadat szakasz vége
        let metadata_end = file.stream_position()?;

        file.sync_all()?;
        Ok(metadata_end)
    }
    
    /// Collection létrehozása
    pub fn create_collection(&mut self, name: &str) -> Result<()> {
        if self.collections.contains_key(name) {
            return Err(MongoLiteError::CollectionExists(name.to_string()));
        }

        // Create new collection with placeholder offset (will be corrected by flush_metadata)
        let meta = CollectionMeta {
            name: name.to_string(),
            document_count: 0,
            data_offset: 0,  // Will be set correctly by flush_metadata
            index_offset: 0,
            last_id: 0,
        };

        self.collections.insert(name.to_string(), meta);
        self.header.collection_count += 1;

        // Flush metadata with proper convergence
        self.flush_metadata()?;

        Ok(())
    }
    
    /// Collection törlése
    pub fn drop_collection(&mut self, name: &str) -> Result<()> {
        if !self.collections.contains_key(name) {
            return Err(MongoLiteError::CollectionNotFound(name.to_string()));
        }

        self.collections.remove(name);
        self.header.collection_count -= 1;

        // Flush metadata with proper convergence
        self.flush_metadata()?;

        Ok(())
    }
    
    /// Collection-ök listája
    pub fn list_collections(&self) -> Vec<String> {
        self.collections.keys().cloned().collect()
    }
    
    /// Collection metaadatok lekérése (immutable)
    pub fn get_collection_meta(&self, name: &str) -> Option<&CollectionMeta> {
        self.collections.get(name)
    }

    /// Collection metaadatok lekérése (mutable)
    /// Metadata changes are persisted only when flush() is called (typically on database close)
    pub fn get_collection_meta_mut(&mut self, name: &str) -> Option<&mut CollectionMeta> {
        self.collections.get_mut(name)
    }
    
    /// Adatok írása (append-only)
    pub fn write_data(&mut self, data: &[u8]) -> Result<u64> {
        let offset = self.file.seek(SeekFrom::End(0))?;
        
        // Méret + adat írása
        let len = (data.len() as u32).to_le_bytes();
        self.file.write_all(&len)?;
        self.file.write_all(data)?;
        
        Ok(offset)
    }
    
    /// Adatok olvasása
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
    
    /// Flush - változások lemezre írása (beleértve a metadata-t is)
    pub fn flush(&mut self) -> Result<()> {
        // Flush metadata to disk with proper convergence
        self.flush_metadata()?;
        self.file.sync_all()?;
        Ok(())
    }

    /// Metadata flush with iterative convergence (internal use)
    fn flush_metadata(&mut self) -> Result<()> {
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
    
    /// Get file length
    pub fn file_len(&self) -> Result<u64> {
        Ok(self.file.metadata()?.len())
    }

    /// Statisztikák
    pub fn stats(&self) -> serde_json::Value {
        serde_json::json!({
            "file_path": self.file_path,
            "file_size": self.file.metadata().map(|m| m.len()).unwrap_or(0),
            "page_size": self.header.page_size,
            "collection_count": self.header.collection_count,
            "collections": self.collections.iter().map(|(name, meta)| {
                serde_json::json!({
                    "name": name,
                    "document_count": meta.document_count,
                    "last_id": meta.last_id,
                })
            }).collect::<Vec<_>>(),
        })
    }
}

// Automatikus bezárás
impl Drop for StorageEngine {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_test_db() -> (TempDir, StorageEngine) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let storage = StorageEngine::open(&db_path).unwrap();
        (temp_dir, storage)
    }

    #[test]
    fn test_create_new_database() {
        let (_temp, storage) = setup_test_db();

        assert_eq!(storage.header.magic, *b"MONGOLTE");
        assert_eq!(storage.header.version, 1);
        assert_eq!(storage.header.page_size, 4096);
        assert_eq!(storage.header.collection_count, 0);
        assert_eq!(storage.collections.len(), 0);
    }

    #[test]
    fn test_open_existing_database() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");

        // Create database
        {
            let mut storage = StorageEngine::open(&db_path).unwrap();
            storage.create_collection("users").unwrap();
            storage.flush().unwrap();
        }

        // Reopen database
        let storage = StorageEngine::open(&db_path).unwrap();
        assert_eq!(storage.header.collection_count, 1);
        assert!(storage.collections.contains_key("users"));
    }

    #[test]
    fn test_magic_number_validation() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("corrupt.mlite");

        // Create corrupt file with wrong magic number
        let mut file = fs::File::create(&db_path).unwrap();
        use std::io::Write;
        file.write_all(b"WRONGMAG").unwrap(); // Wrong magic
        file.sync_all().unwrap();
        drop(file);

        // Try to open should fail
        let result = StorageEngine::open(&db_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_create_collection() {
        let (_temp, mut storage) = setup_test_db();

        storage.create_collection("users").unwrap();

        assert_eq!(storage.header.collection_count, 1);
        assert!(storage.collections.contains_key("users"));

        let meta = storage.get_collection_meta("users").unwrap();
        assert_eq!(meta.name, "users");
        assert_eq!(meta.document_count, 0);
        assert_eq!(meta.last_id, 0);
    }

    #[test]
    fn test_create_duplicate_collection() {
        let (_temp, mut storage) = setup_test_db();

        storage.create_collection("users").unwrap();
        let result = storage.create_collection("users");

        assert!(result.is_err());
        match result {
            Err(MongoLiteError::CollectionExists(_)) => (),
            _ => panic!("Expected CollectionExists error"),
        }
    }

    #[test]
    fn test_create_multiple_collections() {
        let (_temp, mut storage) = setup_test_db();

        storage.create_collection("users").unwrap();
        storage.create_collection("posts").unwrap();
        storage.create_collection("comments").unwrap();

        assert_eq!(storage.header.collection_count, 3);
        assert_eq!(storage.list_collections().len(), 3);

        let collections = storage.list_collections();
        assert!(collections.contains(&"users".to_string()));
        assert!(collections.contains(&"posts".to_string()));
        assert!(collections.contains(&"comments".to_string()));
    }

    #[test]
    fn test_drop_collection() {
        let (_temp, mut storage) = setup_test_db();

        storage.create_collection("users").unwrap();
        storage.create_collection("posts").unwrap();

        storage.drop_collection("users").unwrap();

        assert_eq!(storage.header.collection_count, 1);
        assert!(!storage.collections.contains_key("users"));
        assert!(storage.collections.contains_key("posts"));
    }

    #[test]
    fn test_drop_nonexistent_collection() {
        let (_temp, mut storage) = setup_test_db();

        let result = storage.drop_collection("nonexistent");

        assert!(result.is_err());
        match result {
            Err(MongoLiteError::CollectionNotFound(_)) => (),
            _ => panic!("Expected CollectionNotFound error"),
        }
    }

    #[test]
    fn test_write_and_read_data() {
        let (_temp, mut storage) = setup_test_db();

        let test_data = b"Hello, MongoLite!";
        let offset = storage.write_data(test_data).unwrap();

        let read_data = storage.read_data(offset).unwrap();
        assert_eq!(read_data, test_data);
    }

    #[test]
    fn test_write_multiple_data_blocks() {
        let (_temp, mut storage) = setup_test_db();

        let data1 = b"First block";
        let data2 = b"Second block";
        let data3 = b"Third block";

        let offset1 = storage.write_data(data1).unwrap();
        let offset2 = storage.write_data(data2).unwrap();
        let offset3 = storage.write_data(data3).unwrap();

        assert_eq!(storage.read_data(offset1).unwrap(), data1);
        assert_eq!(storage.read_data(offset2).unwrap(), data2);
        assert_eq!(storage.read_data(offset3).unwrap(), data3);

        // Offsets should be different
        assert_ne!(offset1, offset2);
        assert_ne!(offset2, offset3);
    }

    #[test]
    fn test_collection_metadata_persistence() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");

        // Create and modify collection
        {
            let mut storage = StorageEngine::open(&db_path).unwrap();
            storage.create_collection("users").unwrap();

            // Modify metadata
            let meta = storage.get_collection_meta_mut("users").unwrap();
            meta.document_count = 42;
            meta.last_id = 100;

            storage.flush().unwrap();
        }

        // Reopen and verify
        let storage = StorageEngine::open(&db_path).unwrap();
        let meta = storage.get_collection_meta("users").unwrap();
        assert_eq!(meta.document_count, 42);
        assert_eq!(meta.last_id, 100);
    }

    #[test]
    fn test_flush_metadata_convergence() {
        let (_temp, mut storage) = setup_test_db();

        // Create multiple collections
        for i in 0..5 {
            storage.create_collection(&format!("collection_{}", i)).unwrap();
        }

        // All collections should have correct data_offset
        let first_offset = storage.get_collection_meta("collection_0").unwrap().data_offset;

        for i in 1..5 {
            let offset = storage.get_collection_meta(&format!("collection_{}", i)).unwrap().data_offset;
            assert_eq!(offset, first_offset, "All collections should have same data_offset after convergence");
        }
    }

    #[test]
    fn test_get_collection_meta() {
        let (_temp, mut storage) = setup_test_db();

        storage.create_collection("users").unwrap();

        let meta = storage.get_collection_meta("users");
        assert!(meta.is_some());
        assert_eq!(meta.unwrap().name, "users");

        let nonexistent = storage.get_collection_meta("nonexistent");
        assert!(nonexistent.is_none());
    }

    #[test]
    fn test_get_collection_meta_mut() {
        let (_temp, mut storage) = setup_test_db();

        storage.create_collection("users").unwrap();

        {
            let meta = storage.get_collection_meta_mut("users").unwrap();
            meta.last_id = 999;
        }

        let meta = storage.get_collection_meta("users").unwrap();
        assert_eq!(meta.last_id, 999);
    }

    #[test]
    fn test_stats() {
        let (_temp, mut storage) = setup_test_db();

        storage.create_collection("users").unwrap();
        storage.create_collection("posts").unwrap();

        let stats = storage.stats();

        assert!(stats["file_path"].is_string());
        assert_eq!(stats["collection_count"], 2);
        assert_eq!(stats["page_size"], 4096);

        let collections = stats["collections"].as_array().unwrap();
        assert_eq!(collections.len(), 2);
    }

    #[test]
    fn test_file_len() {
        let (_temp, mut storage) = setup_test_db();

        let initial_len = storage.file_len().unwrap();
        assert!(initial_len > 0, "File should have header");

        storage.write_data(b"Some test data").unwrap();

        let new_len = storage.file_len().unwrap();
        assert!(new_len > initial_len, "File should grow after write");
    }

    #[test]
    fn test_data_persistence_after_reopen() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");

        let offset;

        // Write data
        {
            let mut storage = StorageEngine::open(&db_path).unwrap();
            storage.create_collection("test").unwrap();
            offset = storage.write_data(b"Persistent data").unwrap();
            storage.flush().unwrap();
        }

        // Reopen and read
        {
            let mut storage = StorageEngine::open(&db_path).unwrap();
            let data = storage.read_data(offset).unwrap();
            assert_eq!(data, b"Persistent data");
        }
    }

    #[test]
    fn test_empty_data_write() {
        let (_temp, mut storage) = setup_test_db();

        let offset = storage.write_data(b"").unwrap();
        let data = storage.read_data(offset).unwrap();
        assert_eq!(data, b"");
    }

    #[test]
    fn test_large_data_write() {
        let (_temp, mut storage) = setup_test_db();

        // Create 1MB data block
        let large_data = vec![0xAB; 1024 * 1024];
        let offset = storage.write_data(&large_data).unwrap();

        let read_data = storage.read_data(offset).unwrap();
        assert_eq!(read_data.len(), large_data.len());
        assert_eq!(read_data, large_data);
    }

    #[test]
    fn test_collection_isolation_metadata() {
        let (_temp, mut storage) = setup_test_db();

        storage.create_collection("users").unwrap();
        storage.create_collection("posts").unwrap();

        // Modify users metadata
        {
            let meta = storage.get_collection_meta_mut("users").unwrap();
            meta.last_id = 42;
            meta.document_count = 100;
        }

        // Verify posts metadata not affected
        let posts_meta = storage.get_collection_meta("posts").unwrap();
        assert_eq!(posts_meta.last_id, 0);
        assert_eq!(posts_meta.document_count, 0);
    }

    #[test]
    fn test_header_defaults() {
        let header = Header::default();

        assert_eq!(header.magic, *b"MONGOLTE");
        assert_eq!(header.version, 1);
        assert_eq!(header.page_size, 4096);
        assert_eq!(header.collection_count, 0);
        assert_eq!(header.free_list_head, 0);
    }
}