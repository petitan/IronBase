# Kritikus Persistence Bug Javítása - IronBase v0.2.1

## 🐛 Probléma

**Dokumentumok elvesznek az adatbázis újranyitásakor**

Jelenleg a dokumentumok csak a session alatt elérhetőek. A `close()` után, amikor újra megnyitjuk az adatbázist, minden dokumentum elvész, csak a collection metaadatok maradnak meg.

## 🔍 Gyökérokok (Agent Analízis Alapján)

### 1. **Nincs Document Catalog**
- File offset-ek nincsenek követve
- Dokumentumok a fájl végére íródnak, de nincs map ami mondaná hol vannak

### 2. **Index-ek Nem Perzisztálódnak**
- B+ tree index-ek csak memóriában vannak
- `flush()` nem menti le az index struktúrákat

### 3. **Nincs Index Rebuild**
- Újranyitáskor üres index-ekkel indulunk
- `CollectionCore::new()` nem tölti vissza az adatokat

### 4. **Ineffektív Teljes Scan**
- Minden query végigolvassa az egész fájlt
- Nincs direct document lookup by offset

### 5. **Jelenlegi Architektúra Hibái**

```rust
// CollectionMeta - csak metadata, nincs document lista
pub struct CollectionMeta {
    pub name: String,
    pub document_count: u64,    // ← Csak számláló
    pub data_offset: u64,       // ← Hol kezdődik a data
    pub index_offset: u64,
    pub last_id: u64,
    // HIÁNYZIK: document_catalog
}

// CollectionCore - nincs document storage
pub struct CollectionCore {
    pub name: String,
    pub storage: Arc<RwLock<StorageEngine>>,
    pub indexes: Arc<RwLock<IndexManager>>,  // ← Csak indexek
    // HIÁNYZIK: documents HashMap
}
```

### 6. **Mi Történik Most**

**Write Flow (működik):**
1. `insert_one()` → `storage.write_data()` → dokumentum fájl végére írva
2. In-memory index frissül (DocumentId → IndexKey mapping)
3. Session alatt query-k működnek (index + full scan)

**Close/Reopen Flow (elveszik minden):**
1. `flush()` → csak metadata-t ment (collection név, számláló)
2. Close → dokumentumok a fájlban, de nincs katalógus
3. Reopen → `load_metadata()` → csak metadata betöltés
4. `CollectionCore::new()` → **ÜRES index-ekkel indul**
5. Query → üres index → nincs találat → dokumentumok elvesztek ❌

## 📋 Javítási Terv

### **Fázis 1: Document Catalog Infrastruktúra**

#### 1.1. CollectionMeta Bővítése
**Fájl**: `ironbase-core/src/storage/mod.rs`

```rust
pub struct CollectionMeta {
    pub name: String,
    pub document_count: u64,
    pub data_offset: u64,
    pub index_offset: u64,
    pub last_id: u64,

    // ÚJ: Document catalog - DocumentId → file offset mapping
    pub document_catalog: HashMap<String, u64>,  // id_key → offset
}
```

**Miért**: Ez az alapja a perzisztens document tracking-nek.

#### 1.2. write_data() → write_document() Refaktor
**Fájl**: `ironbase-core/src/storage/io.rs`

Új metódus catalog tracking-gel:

```rust
impl StorageEngine {
    /// Write document and update catalog
    pub fn write_document(
        &mut self,
        collection: &str,
        doc_id: &DocumentId,
        data: &[u8]
    ) -> Result<u64> {
        // Get current file end position
        let offset = self.file.seek(SeekFrom::End(0))?;

        // Write length + data
        let len = (data.len() as u32).to_le_bytes();
        self.file.write_all(&len)?;
        self.file.write_all(data)?;

        // Update catalog in metadata
        let id_key = serde_json::to_string(doc_id)?;
        let meta = self.get_collection_meta_mut(collection)
            .ok_or_else(|| MongoLiteError::CollectionNotFound(collection.to_string()))?;
        meta.document_catalog.insert(id_key, offset);

        Ok(offset)
    }

    /// Read document by offset (for catalog-based retrieval)
    pub fn read_document_at(&mut self, offset: u64) -> Result<Vec<u8>> {
        self.read_data(offset)  // Uses existing read_data
    }
}
```

**Miért**: Automatikusan trackeli minden document offset-jét.

### **Fázis 2: Catalog Perzisztálás**

#### 2.1. Metadata Flush Frissítése
**Fájl**: `ironbase-core/src/storage/metadata.rs`

Update `flush_metadata()`:

```rust
pub(super) fn flush_metadata(&mut self) -> Result<()> {
    // ... existing header write ...

    // Write CollectionMeta (now includes document_catalog)
    for (name, meta) in collections.iter() {
        let meta_bytes = serde_json::to_vec(meta)?;  // ← Includes catalog
        let len = (meta_bytes.len() as u32).to_le_bytes();
        writer.write_all(&len)?;
        writer.write_all(&meta_bytes)?;
    }

    // ... convergence loop ...
}
```

**Változás**: Semmit nem kell csinálni, a `serde_json` automatikusan szerializálja a HashMap-et!

#### 2.2. Metadata Load Frissítése
**Fájl**: `ironbase-core/src/storage/metadata.rs`

Update `load_metadata()`:

```rust
pub(super) fn load_metadata(file: &mut File) -> Result<(Header, HashMap<String, CollectionMeta>)> {
    // ... existing header read ...

    // Read CollectionMeta (now includes document_catalog)
    for _ in 0..header.collection_count {
        let mut len_bytes = [0u8; 4];
        file.read_exact(&mut len_bytes)?;
        let len = u32::from_le_bytes(len_bytes) as usize;

        let mut meta_bytes = vec![0u8; len];
        file.read_exact(&mut meta_bytes)?;

        let meta: CollectionMeta = serde_json::from_slice(&meta_bytes)?;  // ← Includes catalog
        collections.insert(meta.name.clone(), meta);
    }

    Ok((header, collections))
}
```

**Változás**: Már működik, serde automatikusan deszerializálja a catalog HashMap-et!

### **Fázis 3: Index Rebuild on Open**

#### 3.1. CollectionCore::new() Refaktor
**Fájl**: `ironbase-core/src/collection_core.rs`

```rust
pub fn new(name: String, storage: Arc<RwLock<StorageEngine>>) -> Result<Self> {
    // Create collection if not exists
    {
        let mut storage_guard = storage.write();
        if storage_guard.get_collection_meta(&name).is_none() {
            storage_guard.create_collection(&name)?;
        }
    }

    // Initialize index manager
    let mut index_manager = IndexManager::new();

    // Create _id index
    let id_index_name = format!("{}_id", name);
    index_manager.create_btree_index(id_index_name.clone(), "_id".to_string(), true)?;

    // ÚJ: Rebuild indexes from persisted document catalog
    {
        let mut storage_guard = storage.write();
        let meta = storage_guard.get_collection_meta(&name)
            .ok_or_else(|| MongoLiteError::CollectionNotFound(name.clone()))?;

        // Iterate over document catalog
        for (id_key, offset) in &meta.document_catalog {
            // Read document from disk
            let doc_bytes = storage_guard.read_document_at(*offset)?;
            let doc: Value = serde_json::from_slice(&doc_bytes)?;

            // Skip tombstones
            if doc.get("_tombstone").and_then(|v| v.as_bool()).unwrap_or(false) {
                continue;
            }

            // Rebuild _id index
            if let Some(id_value) = doc.get("_id") {
                let doc_id = DocumentId::from(id_value);
                let id_key = IndexKey::from(id_value);
                index_manager.get_btree_index_mut(&id_index_name)?
                    .insert(id_key, doc_id)?;
            }

            // TODO: Rebuild other indexes if they exist
        }
    }

    Ok(CollectionCore {
        name,
        storage,
        indexes: Arc::new(RwLock::new(index_manager)),
    })
}
```

**Miért**: Index-ek automatikusan újraépülnek a perzisztált catalog-ból.

### **Fázis 4: Insert/Update/Delete Frissítése**

#### 4.1. insert_one() Frissítése
**Fájl**: `ironbase-core/src/collection_core.rs`

```rust
pub fn insert_one(&self, mut fields: HashMap<String, Value>) -> Result<DocumentId> {
    let mut storage = self.storage.write();

    let meta = storage.get_collection_meta_mut(&self.name)
        .ok_or_else(|| MongoLiteError::CollectionNotFound(self.name.clone()))?;

    let doc_id = DocumentId::new_auto(meta.last_id);
    meta.last_id += 1;

    fields.insert("_collection".to_string(), Value::String(self.name.clone()));
    let doc = Document::new(doc_id.clone(), fields);

    // Update indexes
    { /* ... existing index update code ... */ }

    // VÁLTOZÁS: Use new write_document instead of write_data
    let doc_json = doc.to_json()?;
    storage.write_document(&self.name, &doc_id, doc_json.as_bytes())?;

    Ok(doc_id)
}
```

#### 4.2. update_one() Frissítése
**Fájl**: `ironbase-core/src/collection_core.rs`

```rust
// Update után új verzió append (MVCC)
let new_doc_json = serde_json::to_string(&new_doc)?;
storage.write_document(&self.name, &doc_id, new_doc_json.as_bytes())?;
```

#### 4.3. delete_one() Frissítése
**Fájl**: `ironbase-core/src/collection_core.rs`

```rust
// Tombstone append
let tombstone_json = serde_json::to_string(&tombstone)?;
storage.write_document(&self.name, &doc_id, tombstone_json.as_bytes())?;
```

### **Fázis 5: ACD Transaction Layer Update**

#### 5.1. apply_operations() Frissítése
**Fájl**: `ironbase-core/src/storage/mod.rs`

```rust
fn apply_operations(&mut self, transaction: &Transaction) -> Result<()> {
    for operation in transaction.operations() {
        match operation {
            Operation::Insert { collection, doc, .. } => {
                let doc_json = serde_json::to_string(doc)?;
                if let Some(id) = doc.get("_id") {
                    let doc_id = DocumentId::from(id);
                    self.write_document(collection, &doc_id, doc_json.as_bytes())?;
                }
            }
            Operation::Update { collection, doc_id, new_doc, .. } => {
                let doc_json = serde_json::to_string(new_doc)?;
                self.write_document(collection, doc_id, doc_json.as_bytes())?;
            }
            Operation::Delete { collection, doc_id, .. } => {
                let tombstone = json!({
                    "_id": doc_id,
                    "_collection": collection,
                    "_tombstone": true
                });
                let tombstone_json = serde_json::to_string(&tombstone)?;
                self.write_document(collection, doc_id, tombstone_json.as_bytes())?;
            }
        }
    }
    Ok(())
}
```

### **Fázis 6: Query Optimization (Opcionális)**

#### 6.1. Catalog-based Direct Lookup
**Fájl**: `ironbase-core/src/collection_core.rs`

Új helper metódus:

```rust
fn get_documents_by_ids(&self, doc_ids: &[DocumentId]) -> Result<Vec<Value>> {
    let mut storage = self.storage.write();
    let meta = storage.get_collection_meta(&self.name)
        .ok_or_else(|| MongoLiteError::CollectionNotFound(self.name.clone()))?;

    let mut docs = Vec::new();

    for doc_id in doc_ids {
        let id_key = serde_json::to_string(doc_id)?;

        // Direct catalog lookup instead of full scan
        if let Some(offset) = meta.document_catalog.get(&id_key) {
            let doc_bytes = storage.read_document_at(*offset)?;
            let doc: Value = serde_json::from_slice(&doc_bytes)?;

            // Skip tombstones
            if !doc.get("_tombstone").and_then(|v| v.as_bool()).unwrap_or(false) {
                docs.push(doc);
            }
        }
    }

    Ok(docs)
}
```

**Miért**: Teljes file scan helyett direct offset-based retrieval.

### **Fázis 7: Backward Compatibility (Opcionális)**

#### 7.1. Migration Support
**Fájl**: `ironbase-core/src/storage/mod.rs`

```rust
pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
    // ... existing code ...

    let (header, mut collections) = if exists && file.metadata()?.len() > 0 {
        let (h, c) = Self::load_metadata(&mut file)?;

        // Check if catalog needs migration
        let needs_migration = c.values().any(|meta| meta.document_catalog.is_empty());

        if needs_migration {
            eprintln!("⚠️  Old format detected, rebuilding document catalog...");
            // Rebuild catalog by scanning file
            Self::rebuild_catalog(&mut file, &mut collections)?;
        }

        (h, c)
    } else {
        // ... new database ...
    };

    // ...
}

fn rebuild_catalog(file: &mut File, collections: &mut HashMap<String, CollectionMeta>) -> Result<()> {
    // Full file scan to rebuild catalog
    // Similar to current scan_documents logic
    // ...
}
```

## ✅ Tesztelési Terv

### 1. **Alapvető Persistence Test**
```python
# Test 1: Insert, close, reopen
db = ironbase.IronBase("test.db")
coll = db.collection("test")
coll.insert_one({"name": "Alice"})
db.close()

db = ironbase.IronBase("test.db")  # Reopen
coll = db.collection("test")
assert coll.count_documents({}) == 1  # Should be 1, not 0!
assert coll.find_one({"name": "Alice"}) is not None
```

### 2. **Transaction Persistence Test**
```python
# Test 2: Transactional insert + commit
db = ironbase.IronBase("test.db")
tx_id = db.begin_transaction()
db.insert_one_tx("test", {"name": "Bob"}, tx_id)
db.commit_transaction(tx_id)
db.close()

db = ironbase.IronBase("test.db")  # Reopen
assert db.collection("test").count_documents({}) == 1
```

### 3. **Multi-Collection Test**
```python
# Test 3: Multiple collections
db = ironbase.IronBase("test.db")
db.collection("users").insert_one({"name": "Alice"})
db.collection("posts").insert_one({"title": "Hello"})
db.close()

db = ironbase.IronBase("test.db")  # Reopen
assert db.collection("users").count_documents({}) == 1
assert db.collection("posts").count_documents({}) == 1
```

### 4. **Update/Delete Persistence Test**
```python
# Test 4: Updates and deletes persist
db = ironbase.IronBase("test.db")
coll = db.collection("test")
coll.insert_one({"_id": 1, "count": 0})
coll.update_one({"_id": 1}, {"$set": {"count": 5}})
db.close()

db = ironbase.IronBase("test.db")  # Reopen
doc = db.collection("test").find_one({"_id": 1})
assert doc["count"] == 5  # Updated value should persist
```

### 5. **Large Dataset Test**
```python
# Test 5: Many documents (chunks import)
db = ironbase.IronBase("test.db")
coll = db.collection("chunks")
coll.insert_many([{"n": i} for i in range(1000)])
db.close()

db = ironbase.IronBase("test.db")  # Reopen
assert db.collection("chunks").count_documents({}) == 1000
```

## 📊 Várható Eredmények

### Előtte (v0.2.0 - Broken):
- ❌ Dokumentumok elvesznek close után
- ❌ Csak session alatt működik
- ❌ Teljes file scan minden query-nél

### Utána (v0.2.1 - Fixed):
- ✅ Dokumentumok perzisztálnak
- ✅ Reopen után is elérhetőek
- ✅ Catalog-based direct lookup (gyorsabb)
- ✅ ACD tranzakciók megbízhatóak

## 🎯 Implementációs Sorrend

1. ✅ **CollectionMeta bővítése** (document_catalog HashMap)
2. ✅ **write_document() létrehozása** (catalog tracking)
3. ✅ **Index rebuild** (CollectionCore::new() frissítése)
4. ✅ **insert_one/update_one/delete_one** (write_document használata)
5. ✅ **Transaction layer** (apply_operations frissítése)
6. ✅ **Tesztelés** (import_and_query.py → import → close → query külön)
7. 🔄 **Optimalizálás** (catalog-based direct lookup - opcionális)
8. 🔄 **Migration** (backward compatibility - opcionális)

## 🚀 Kész!

Utána a chunks import teszt működni fog:
```bash
python import_chunks.py  # Import 81 chunks
# Database closed
python query_chunks.py   # Query works! 81 documents found ✅
```
