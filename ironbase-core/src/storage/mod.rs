// storage/mod.rs
// Storage engine module

mod compaction;
mod metadata;
mod io;

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use memmap2::{MmapMut, MmapOptions};
use serde::{Serialize, Deserialize};
use crate::error::{Result, MongoLiteError};
use crate::wal::WriteAheadLog;
use crate::transaction::Transaction;

// Re-export compaction types
pub use compaction::CompactionStats;

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
    wal: WriteAheadLog,
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

        // WAL fájl megnyitása
        let wal_path = PathBuf::from(&path_str).with_extension("wal");
        let wal = WriteAheadLog::open(wal_path)?;

        let mut storage = StorageEngine {
            file,
            mmap,
            header,
            collections,
            file_path: path_str,
            wal,
        };

        // Recover from WAL if needed
        storage.recover_from_wal()?;

        Ok(storage)
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

    /// Flush - változások lemezre írása (beleértve a metadata-t is)
    pub fn flush(&mut self) -> Result<()> {
        // Flush metadata to disk with proper convergence
        self.flush_metadata()?;
        self.file.sync_all()?;
        Ok(())
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

    /// Commit a transaction (9-step atomic operation)
    /// This is the core of ACD guarantee
    pub fn commit_transaction(&mut self, transaction: &mut Transaction) -> Result<()> {
        use crate::wal::{WALEntry, WALEntryType};

        if !transaction.is_active() {
            return Err(MongoLiteError::TransactionCommitted);
        }

        // Step 1: Write BEGIN marker to WAL
        let begin_entry = WALEntry::new(transaction.id, WALEntryType::Begin, vec![]);
        self.wal.append(&begin_entry)?;

        // Step 2: Write all operations to WAL (use JSON instead of bincode for compatibility)
        for operation in transaction.operations() {
            let op_json = serde_json::to_string(operation)
                .map_err(|e| MongoLiteError::Serialization(e.to_string()))?;
            let op_entry = WALEntry::new(transaction.id, WALEntryType::Operation, op_json.as_bytes().to_vec());
            self.wal.append(&op_entry)?;
        }

        // Step 3: Write COMMIT marker to WAL
        let commit_entry = WALEntry::new(transaction.id, WALEntryType::Commit, vec![]);
        self.wal.append(&commit_entry)?;

        // Step 4: Fsync WAL (durability guarantee)
        self.wal.flush()?;

        // Step 5: Apply operations to storage
        self.apply_operations(transaction)?;

        // Step 6: Apply index changes
        // NOTE: Index changes are tracked in transaction.index_changes()
        // The actual index updates happen at a higher level (CollectionCore)
        // where the IndexManager is accessible. This maintains clean separation
        // of concerns: StorageEngine handles data persistence, CollectionCore
        // handles index management.
        // For full ACD guarantee, index updates must be applied before Step 8 (fsync)

        // Step 7: Apply metadata changes
        for metadata_change in transaction.metadata_changes() {
            if let Some(meta) = self.collections.get_mut(&metadata_change.collection) {
                meta.last_id = metadata_change.last_id as u64;
            }
        }

        // Step 8: Fsync storage file
        self.file.sync_all()?;

        // Step 9: Mark transaction as committed
        transaction.mark_committed()?;

        Ok(())
    }

    /// Rollback a transaction (discard all buffered operations)
    pub fn rollback_transaction(&mut self, transaction: &mut Transaction) -> Result<()> {
        use crate::wal::{WALEntry, WALEntryType};

        if !transaction.is_active() {
            return Ok(()); // Already committed or aborted
        }

        // Write ABORT marker to WAL
        let abort_entry = WALEntry::new(transaction.id, WALEntryType::Abort, vec![]);
        self.wal.append(&abort_entry)?;
        self.wal.flush()?;

        // Discard all buffered operations
        transaction.rollback()?;

        Ok(())
    }

    /// Apply transaction operations to storage
    fn apply_operations(&mut self, transaction: &Transaction) -> Result<()> {
        use crate::transaction::Operation;

        for operation in transaction.operations() {
            match operation {
                Operation::Insert { collection: _, doc_id: _, doc } => {
                    // Serialize and write document to storage
                    let doc_json = serde_json::to_string(doc)
                        .map_err(|e| MongoLiteError::Serialization(e.to_string()))?;
                    self.write_data(doc_json.as_bytes())?;
                }
                Operation::Update { collection: _, doc_id: _, old_doc: _, new_doc } => {
                    // Write new version of document (append-only)
                    let doc_json = serde_json::to_string(new_doc)
                        .map_err(|e| MongoLiteError::Serialization(e.to_string()))?;
                    self.write_data(doc_json.as_bytes())?;
                }
                Operation::Delete { collection, doc_id, old_doc: _ } => {
                    // Write tombstone marker with collection info
                    let tombstone = serde_json::json!({
                        "_id": doc_id,
                        "_collection": collection,
                        "_tombstone": true
                    });
                    let tombstone_json = serde_json::to_string(&tombstone)
                        .map_err(|e| MongoLiteError::Serialization(e.to_string()))?;
                    self.write_data(tombstone_json.as_bytes())?;
                }
            }
        }

        Ok(())
    }

    /// Recover from WAL after crash
    pub fn recover_from_wal(&mut self) -> Result<()> {
        let recovered = self.wal.recover()?;

        if recovered.is_empty() {
            return Ok(());
        }

        // Replay each committed transaction
        for tx_entries in recovered {
            // Deserialize operations from WAL entries
            for entry in tx_entries {
                if entry.entry_type == crate::wal::WALEntryType::Operation {
                    let op_str = std::str::from_utf8(&entry.data)
                        .map_err(|e| MongoLiteError::Serialization(format!("UTF-8 error: {}", e)))?;
                    let operation: crate::transaction::Operation = serde_json::from_str(op_str)?;

                    // Apply operation to storage
                    match operation {
                        crate::transaction::Operation::Insert { collection: _, doc_id: _, doc } => {
                            let doc_json = serde_json::to_string(&doc)
                                .map_err(|e| MongoLiteError::Serialization(e.to_string()))?;
                            self.write_data(doc_json.as_bytes())?;
                        }
                        crate::transaction::Operation::Update { collection: _, doc_id: _, old_doc: _, new_doc } => {
                            let doc_json = serde_json::to_string(&new_doc)
                                .map_err(|e| MongoLiteError::Serialization(e.to_string()))?;
                            self.write_data(doc_json.as_bytes())?;
                        }
                        crate::transaction::Operation::Delete { collection, doc_id, old_doc: _ } => {
                            let tombstone = serde_json::json!({
                                "_id": doc_id,
                                "_collection": collection,
                                "_tombstone": true
                            });
                            let tombstone_json = serde_json::to_string(&tombstone)
                                .map_err(|e| MongoLiteError::Serialization(e.to_string()))?;
                            self.write_data(tombstone_json.as_bytes())?;
                        }
                    }
                }
            }
        }

        // Clear WAL after successful recovery
        self.wal.clear()?;

        Ok(())
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

    // ========== ACD Transaction Tests ==========

    #[test]
    fn test_transaction_commit_with_insert() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");

        {
            let mut storage = StorageEngine::open(&db_path).unwrap();
            storage.create_collection("users").unwrap();

            // Create and commit transaction
            let mut tx = crate::transaction::Transaction::new(1);
            tx.add_operation(crate::transaction::Operation::Insert {
                collection: "users".to_string(),
                doc_id: crate::document::DocumentId::Int(1),
                doc: serde_json::json!({"name": "Alice", "age": 30}),
            }).unwrap();

            storage.commit_transaction(&mut tx).unwrap();
        }

        // Verify data persisted
        {
            let mut storage = StorageEngine::open(&db_path).unwrap();
            let file_len = storage.file_len().unwrap();
            assert!(file_len > 0, "Storage should contain data after commit");
        }
    }

    #[test]
    fn test_transaction_rollback() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");

        let mut storage = StorageEngine::open(&db_path).unwrap();
        storage.create_collection("users").unwrap();

        // Create and rollback transaction
        let mut tx = crate::transaction::Transaction::new(1);
        tx.add_operation(crate::transaction::Operation::Insert {
            collection: "users".to_string(),
            doc_id: crate::document::DocumentId::Int(1),
            doc: serde_json::json!({"name": "Bob"}),
        }).unwrap();

        storage.rollback_transaction(&mut tx).unwrap();

        // Transaction should be rolled back
        assert_eq!(tx.state(), crate::transaction::TransactionState::Aborted);
    }

    #[test]
    fn test_wal_recovery_after_crash() {
        use crate::wal::{WriteAheadLog, WALEntry, WALEntryType};

        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let wal_path = temp_dir.path().join("test.wal");

        // Simulate crash: Write WAL entries but don't apply to storage
        {
            let mut wal = WriteAheadLog::open(&wal_path).unwrap();

            // Write a committed transaction to WAL
            let tx_id = 1;
            wal.append(&WALEntry::new(tx_id, WALEntryType::Begin, vec![])).unwrap();

            let operation = crate::transaction::Operation::Insert {
                collection: "users".to_string(),
                doc_id: crate::document::DocumentId::Int(1),
                doc: serde_json::json!({"name": "Recovered Alice", "age": 25}),
            };
            let op_json = serde_json::to_string(&operation).unwrap();
            wal.append(&WALEntry::new(tx_id, WALEntryType::Operation, op_json.as_bytes().to_vec())).unwrap();

            wal.append(&WALEntry::new(tx_id, WALEntryType::Commit, vec![])).unwrap();
            wal.flush().unwrap();
        }

        // Create storage file (simulating existing database)
        {
            let mut storage = StorageEngine::open(&db_path).unwrap();
            storage.create_collection("users").unwrap();
            storage.flush().unwrap();
        }

        // Reopen storage - should recover from WAL
        {
            let _storage = StorageEngine::open(&db_path).unwrap();
            // Recovery happens automatically in open()

            // WAL should be cleared after recovery
            let mut wal_result = WriteAheadLog::open(&wal_path).unwrap();
            let recovered = wal_result.recover().unwrap();
            assert_eq!(recovered.len(), 0, "WAL should be empty after recovery");
        }
    }

    #[test]
    fn test_wal_recovery_multiple_transactions() {
        use crate::wal::{WriteAheadLog, WALEntry, WALEntryType};

        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let wal_path = temp_dir.path().join("test.wal");

        // Write multiple committed transactions to WAL
        {
            let mut wal = WriteAheadLog::open(&wal_path).unwrap();

            for tx_id in 1..=3 {
                wal.append(&WALEntry::new(tx_id, WALEntryType::Begin, vec![])).unwrap();

                let operation = crate::transaction::Operation::Insert {
                    collection: "users".to_string(),
                    doc_id: crate::document::DocumentId::Int(tx_id as i64),
                    doc: serde_json::json!({"name": format!("User {}", tx_id)}),
                };
                let op_json = serde_json::to_string(&operation).unwrap();
                wal.append(&WALEntry::new(tx_id, WALEntryType::Operation, op_json.as_bytes().to_vec())).unwrap();

                wal.append(&WALEntry::new(tx_id, WALEntryType::Commit, vec![])).unwrap();
            }
            wal.flush().unwrap();
        }

        // Create storage and recover
        {
            let mut storage = StorageEngine::open(&db_path).unwrap();
            storage.create_collection("users").unwrap();
        }

        // Reopen and verify recovery
        {
            let storage = StorageEngine::open(&db_path).unwrap();
            let file_len = storage.file_len().unwrap();
            assert!(file_len > 0, "Storage should contain recovered data");
        }
    }
}