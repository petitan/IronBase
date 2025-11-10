# Delete Műveletek és Compaction - Részletes Implementációs Terv

## Tartalomjegyzék
1. [Áttekintés](#áttekintés)
2. [Stratégiai Döntés: Delete Mechanizmus](#stratégiai-döntés-delete-mechanizmus)
3. [Delete Műveletek Specifikációja](#delete-műveletek-specifikációja)
4. [Tombstone Pattern Részletei](#tombstone-pattern-részletei)
5. [Compaction (Garbage Collection)](#compaction-garbage-collection)
6. [Algoritmusok és Pszeudokód](#algoritmusok-és-pszeudokód)
7. [Adatstruktúrák](#adatstruktúrák)
8. [Implementációs Példák](#implementációs-példák)
9. [Edge Case-ek és Hibakezelés](#edge-case-ek-és-hibakezelés)
10. [Teljesítmény és Optimalizálás](#teljesítmény-és-optimalizálás)

---

## Áttekintés

A delete műveletek eltávolítják a dokumentumokat a collection-ből. Az append-only storage stratégia miatt fizikai törlés helyett **logikai törlést (tombstone pattern)** használunk, majd később **compaction** során visszanyerjük a helyet.

### Célok
- ✅ MongoDB-kompatibilis delete API
- ✅ Atomi delete műveletek
- ✅ Index konzisztencia
- ✅ Free space visszanyerés (compaction)
- ✅ Crash-safe működés

---

## Stratégiai Döntés: Delete Mechanizmus

### Lehetőségek Elemzése

#### Option 1: Fizikai Törlés (Immediate Delete)

**Működés:**
- Azonnal felülírja a dokumentumot / free list-re teszi

**Előnyök:**
- ✅ Azonnali hely felszabadulás
- ✅ Nincs szükség compaction-re
- ✅ Fájl méret nem nő

**Hátrányok:**
- ❌ Bonyolult free space management
- ❌ Fragmentáció
- ❌ Crash recovery nehéz
- ❌ Nincs rollback/undo lehetőség
- ❌ In-place módosítás (nem append-only)

**Verdict:** ❌ NEM alkalmas append-only stratégiához

---

#### Option 2: Logikai Törlés - Tombstone Flag (✅ Választott)

**Működés:**
- Dokumentumot "megjelöli" töröltként
- Tombstone record írása
- Compaction később visszanyeri a helyet

**Előnyök:**
- ✅ Append-only stratégiához illeszkedik
- ✅ Crash-safe (WAL jellegű)
- ✅ Egyszerű implementáció
- ✅ Rollback/undo lehetséges (későbbi feature)
- ✅ MVCC alapok (multi-version concurrency control)

**Hátrányok:**
- ❌ Fájl méret növekedés (tombstone-ok)
- ❌ Compaction szükséges
- ❌ Read performance csökkenés (tombstone filter)

**Verdict:** ✅ **MVP választás**

---

#### Option 3: Immediate Compaction (Delete = Rewrite)

**Működés:**
- Delete trigger azonnal compaction-t
- Teljes fájl újraírása törölt dokumentumok nélkül

**Előnyök:**
- ✅ Nincs tombstone felhalmozódás
- ✅ Fájl mindig optimális méretű

**Hátrányok:**
- ❌ Nagyon lassú (teljes fájl írás minden delete-nél)
- ❌ Nem skálázható
- ❌ Nem atomi (crash esetén corruption)

**Verdict:** ❌ NEM praktikus

---

### **DÖNTÉS: Tombstone Pattern + Deferred Compaction**

**Indoklás:**
1. **Append-only konzisztencia**: Update is így működik
2. **Crash safety**: Write-ahead log jellegű
3. **Teljesítmény**: Delete gyors (csak tombstone írás)
4. **Flexibilitás**: Compaction időzíthető (background job)
5. **MVCC alap**: Későbbi transaction támogatáshoz

**Compaction stratégia:**
- **Trigger:** 30% tombstone arány VAGY explicit compact() hívás
- **Időzítés:** Foreground (MVP) → Background (v0.3.0)
- **Algoritmus:** Copy-on-compaction (új fájl írása)

---

## Delete Műveletek Specifikációja

### 1. delete_one() - Egy dokumentum törlése

**API:**
```python
result = collection.delete_one(query: dict)
```

**Paraméterek:**
- `query`: MongoDB-szerű query (ugyanaz mint find-nál)

**Visszatérési érték:**
```python
{
    "acknowledged": True,
    "deleted_count": 1  # vagy 0, ha nem talált
}
```

**Szemantika:**
- Megkeresi az **első** matching dokumentumot
- Tombstone-t ír rá
- Index-ből eltávolítja
- Metadata frissítése (document_count csökkentés)

**Példa:**
```python
# Egy dokumentum törlése ID alapján
result = collection.delete_one({"_id": 123})
print(result)  # {"acknowledged": True, "deleted_count": 1}

# Query-vel
result = collection.delete_one({"name": "János", "age": {"$gt": 30}})
```

---

### 2. delete_many() - Több dokumentum törlése

**API:**
```python
result = collection.delete_many(query: dict)
```

**Paraméterek:**
- `query`: MongoDB-szerű query
- `{}` üres query = **összes dokumentum törlése**

**Visszatérési érték:**
```python
{
    "acknowledged": True,
    "deleted_count": <count>  # törölt dokumentumok száma
}
```

**Szemantika:**
- Megkeresi az **összes** matching dokumentumot
- Mindegyikre tombstone-t ír
- Index frissítés
- Metadata frissítés

**Példa:**
```python
# Összes János nevű törlése
result = collection.delete_many({"name": "János"})
print(result)  # {"acknowledged": True, "deleted_count": 5}

# Összes dokumentum törlése
result = collection.delete_many({})
print(result)  # {"acknowledged": True, "deleted_count": 100}
```

**Figyelem:**
- `delete_many({})` **veszélyes** - confirmation kérhető (opt-in)
- Large delete: compaction trigger valószínű

---

### 3. drop() - Teljes collection törlése

**API:**
```python
db.drop_collection("collection_name")
# vagy
collection.drop()
```

**Szemantika:**
- Törli a collection metadatát
- Dokumentumok fizikailag maradnak (compaction során törlődnek)
- Indexek törlése
- Azonnali művelet (nincs tombstone)

**Példa:**
```python
db.drop_collection("users")  # Teljes users collection törlése
```

**Compaction hatás:**
- Drop után mindenképp compaction ajánlott (nagy hely felszabadulás)

---

## Tombstone Pattern Részletei

### Tombstone Struktúra

```rust
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Tombstone {
    /// Marker flag (mindig true)
    pub _tombstone: bool,

    /// Eredeti dokumentum ID
    pub _id: DocumentId,

    /// Törlés típusa
    pub _delete_type: DeleteType,

    /// Törlés időbélyege (epoch ms)
    pub _deleted_at: u64,

    /// Opcionális: Ki törölte (user/process ID)
    pub _deleted_by: Option<String>,

    /// Opcionális: Törlés oka (logging)
    pub _reason: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum DeleteType {
    /// delete_one() vagy delete_many() művelet
    Explicit,

    /// Update művelet (régi verzió superseded)
    Superseded { superseded_by: u64 },

    /// Collection drop
    CollectionDropped,

    /// TTL expiration (későbbi feature)
    Expired,
}
```

### Tombstone Fájlban

**Formátum:**
```
[u32 length][JSON tombstone bytes]
```

**Példa JSON:**
```json
{
    "_tombstone": true,
    "_id": 123,
    "_delete_type": "Explicit",
    "_deleted_at": 1699876543210,
    "_deleted_by": "user:admin",
    "_reason": "cleanup old records"
}
```

**Méret:**
- Minimum: ~100 bytes (egyszerű tombstone)
- Tipikus: ~150-200 bytes
- Sokkal kisebb mint eredeti dokumentum

---

### Tombstone vs. Eredeti Dokumentum

**Tombstone előnyök:**
- Kis méret (csak metadata)
- Gyors írás
- Kompakt tárolás

**Alternatív: Soft Delete Flag**
```json
// Eredeti dokumentum megtartása "_deleted: true" flag-gel
{
    "_id": 123,
    "_deleted": true,
    "_deleted_at": 1699876543210,
    "name": "János",
    "age": 30,
    // ... összes többi mező
}
```

**Miért NEM ezt választjuk:**
- ❌ Nagyobb fájlméret (teljes dokumentum)
- ❌ Lassabb írás
- ❌ Query-kben filter szükséges (`{_deleted: {$ne: true}}`)
- ❌ Index bloat (törölt rekordok indexben maradnak)

**VÁLASZTÁS: Külön tombstone rekord** (kisebb, gyorsabb, tisztább)

---

## Compaction (Garbage Collection)

### Mi az a Compaction?

**Cél:**
- Tombstone-ok eltávolítása
- Fájlméret csökkentése
- Storage optimalizálás

**Működés:**
1. Új fájl létrehozása (`<database>.mlite.compact`)
2. Csak élő dokumentumok másolása
3. Indexek újraépítése
4. Atomic swap (régi fájl cseréje újra)
5. Régi fájl törlése

**Trigger feltételek:**
- Tombstone arány > 30% (konfiguálható)
- Explicit `compact()` hívás
- Scheduled compaction (cron job)
- Fájlméret > threshold ÉS tombstone arány > 10%

---

### Compaction Stratégiák

#### Option 1: Full Compaction (✅ MVP)

**Működés:**
- Teljes fájl újraírása
- Minden collection compaction egyszerre

**Előnyök:**
- ✅ Egyszerű implementáció
- ✅ Maximális hely visszanyerés
- ✅ Index rebuild tiszta állapotból

**Hátrányok:**
- ❌ Lassú (teljes fájl írás)
- ❌ Fájl lock szükséges (read-only mode)
- ❌ Nem skálázható nagy fájlokra

**MVP választás:** Full Compaction (elegendő kis-közepes fájlokra)

---

#### Option 2: Incremental Compaction (Későbbi)

**Működés:**
- Csak egy collection compaction egyszerre
- Részleges fájl újraírás

**Előnyök:**
- ✅ Kisebb I/O burst
- ✅ Részleges lock (collection szinten)

**Hátrányok:**
- ❌ Bonyolultabb implementáció
- ❌ Fájl fragmentáció maradhat

**v0.3.0 feature**

---

#### Option 3: Background Compaction (v1.0)

**Működés:**
- Külön thread/process
- Nem blokkolja a write műveleteket
- Copy-on-write jellegű

**Előnyök:**
- ✅ Zero downtime
- ✅ Production-ready

**Hátrányok:**
- ❌ Komplex implementáció
- ❌ Memory overhead

**v1.0 feature** (production readiness)

---

### Compaction Algoritmus (Full, MVP)

**Előfeltétel ellenőrzés:**
```
FUNCTION should_compact(collection_meta):
    total_docs = collection_meta.document_count
    tombstone_count = count_tombstones(collection_meta)

    IF tombstone_count == 0:
        RETURN False
    END IF

    tombstone_ratio = tombstone_count / (total_docs + tombstone_count)

    IF tombstone_ratio > COMPACTION_THRESHOLD:  // 0.3 (30%)
        RETURN True
    END IF

    RETURN False
END FUNCTION
```

**Compaction végrehajtás:**
```
FUNCTION compact_database(db_path):
    // 1. Read lock (létező műveletek befejezése)
    storage.lock_write()  // Exkluzív lock

    PRINT "Compaction started..."

    // 2. Új fájl létrehozása
    compact_path = db_path + ".compact"
    compact_file = create_new_file(compact_path)

    // 3. Header és metadata másolása (üres collection-ökkel)
    new_header = storage.header.clone()
    new_collections = {}

    // 4. Collection-önként compaction
    FOR collection_name, old_meta IN storage.collections:
        new_meta = compact_collection(
            storage,
            collection_name,
            old_meta,
            compact_file
        )
        new_collections[collection_name] = new_meta
    END FOR

    // 5. Metadata írása az új fájlba
    write_metadata(compact_file, new_header, new_collections)

    // 6. Index újraépítése
    FOR collection_name IN new_collections:
        rebuild_indexes(compact_file, collection_name)
    END FOR

    // 7. Sync és flush
    compact_file.sync_all()
    compact_file.close()

    // 8. Atomic swap (rename)
    old_backup_path = db_path + ".old"
    rename(db_path, old_backup_path)  // Backup
    rename(compact_path, db_path)     // Új fájl aktiválás

    // 9. Storage reload
    storage.reload(db_path)

    // 10. Régi fájl törlése
    delete_file(old_backup_path)

    storage.unlock_write()

    PRINT "Compaction completed."
    RETURN compaction_stats
END FUNCTION
```

**Collection compaction:**
```
FUNCTION compact_collection(storage, collection_name, old_meta, new_file):
    new_meta = CollectionMeta {
        name: collection_name,
        document_count: 0,
        data_offset: new_file.current_position(),
        index_offset: 0,  // Később
        last_id: old_meta.last_id,
    }

    // Összes dokumentum olvasása
    FOR offset IN old_meta.data_offsets:
        doc_bytes = storage.read_data(offset)
        doc = deserialize_json(doc_bytes)

        // Tombstone skip
        IF doc["_tombstone"] == true:
            CONTINUE  // Skip
        END IF

        // Élő dokumentum másolása
        new_offset = new_file.write_data(doc_bytes)
        new_meta.document_count += 1
    END FOR

    RETURN new_meta
END FUNCTION
```

---

## Algoritmusok és Pszeudokód

### Delete_One Algoritmus

```
FUNCTION delete_one(collection_name, query):
    // 1. Query matching - dokumentum keresése
    storage.lock_read()
    meta = storage.get_collection_meta(collection_name)

    found_doc = NULL
    found_offset = NULL

    FOR offset IN meta.data_offsets:
        doc_bytes = storage.read_data(offset)
        doc = deserialize_json(doc_bytes)

        // Skip tombstones
        IF doc["_tombstone"] == true:
            CONTINUE
        END IF

        // Query matching
        IF query_matches(doc, query):
            found_doc = doc
            found_offset = offset
            BREAK  // Csak az első
        END IF
    END FOR

    storage.unlock_read()

    // 2. Nem találtuk
    IF found_doc == NULL:
        RETURN {acknowledged: true, deleted_count: 0}
    END IF

    // 3. Tombstone írása
    storage.lock_write()

    tombstone = Tombstone {
        _tombstone: true,
        _id: found_doc["_id"],
        _delete_type: DeleteType::Explicit,
        _deleted_at: current_timestamp_ms(),
        _deleted_by: None,
        _reason: None,
    }

    tombstone_bytes = serialize_json(tombstone)
    storage.write_data(tombstone_bytes)  // Append-only

    // 4. Index eltávolítás
    remove_from_indexes(collection_name, found_doc)

    // 5. Metadata frissítés
    meta.document_count -= 1
    storage.update_collection_meta(collection_name, meta)

    storage.unlock_write()

    // 6. Compaction trigger ellenőrzés
    IF should_compact(meta):
        // Opcionális: auto-compact
        // compact_database(storage.db_path)
        LOG "Compaction recommended (tombstone threshold reached)"
    END IF

    RETURN {acknowledged: true, deleted_count: 1}
END FUNCTION
```

---

### Delete_Many Algoritmus

```
FUNCTION delete_many(collection_name, query):
    storage.lock_write()  // Exkluzív lock

    meta = storage.get_collection_meta(collection_name)
    deleted_count = 0
    tombstones = []

    // 1. Összes matching dokumentum keresése
    FOR offset IN meta.data_offsets:
        doc_bytes = storage.read_data(offset)
        doc = deserialize_json(doc_bytes)

        // Skip tombstones
        IF doc["_tombstone"] == true:
            CONTINUE
        END IF

        // Query matching
        IF NOT query_matches(doc, query):
            CONTINUE
        END IF

        // Tombstone létrehozása
        tombstone = create_tombstone(doc["_id"], DeleteType::Explicit)
        tombstones.append(tombstone)

        // Index eltávolítás
        remove_from_indexes(collection_name, doc)

        deleted_count += 1
    END FOR

    // 2. Batch tombstone írás
    FOR tombstone IN tombstones:
        tombstone_bytes = serialize_json(tombstone)
        storage.write_data(tombstone_bytes)
    END FOR

    // 3. Metadata frissítés
    meta.document_count -= deleted_count
    storage.update_collection_meta(collection_name, meta)

    storage.unlock_write()

    // 4. Compaction trigger
    IF should_compact(meta):
        LOG "Compaction recommended"
    END IF

    RETURN {acknowledged: true, deleted_count: deleted_count}
END FUNCTION
```

---

### Tombstone Count Algoritmus

```
FUNCTION count_tombstones(collection_meta):
    tombstone_count = 0

    FOR offset IN collection_meta.data_offsets:
        doc_bytes = storage.read_data(offset)
        doc = deserialize_json(doc_bytes)

        IF doc.get("_tombstone") == true:
            tombstone_count += 1
        END IF
    END FOR

    RETURN tombstone_count
END FUNCTION
```

**Optimalizáció (későbbi):**
- Metadata-ban tárolni tombstone count-ot
- Nem kell minden dokumentumot scan-nelni

---

## Adatstruktúrák

### CollectionMeta Bővítés

```rust
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CollectionMeta {
    pub name: String,
    pub document_count: u64,      // Élő dokumentumok
    pub tombstone_count: u64,     // ✅ ÚJ: Tombstone-ok száma
    pub data_offset: u64,
    pub index_offset: u64,
    pub last_id: u64,
    pub last_compaction: Option<u64>,  // ✅ ÚJ: Utolsó compaction timestamp
}
```

**Előnyök:**
- Gyors tombstone arány számítás
- Compaction trigger gyorsabb
- Statisztikák

---

### CompactionStats

```rust
#[derive(Debug, Clone, Serialize)]
pub struct CompactionStats {
    /// Compaction kezdete
    pub started_at: u64,

    /// Compaction vége
    pub completed_at: u64,

    /// Időtartam (ms)
    pub duration_ms: u64,

    /// Eredeti fájlméret (bytes)
    pub original_size: u64,

    /// Compaction utáni méret
    pub compacted_size: u64,

    /// Felszabadított hely
    pub space_saved: u64,

    /// Törölt tombstone-ok száma
    pub tombstones_removed: u64,

    /// Megtartott dokumentumok
    pub documents_kept: u64,

    /// Collection-önkénti statisztikák
    pub collections: Vec<CollectionCompactionStats>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CollectionCompactionStats {
    pub name: String,
    pub documents_before: u64,
    pub tombstones_before: u64,
    pub documents_after: u64,
}
```

---

## Implementációs Példák

### Delete_One Rust Implementáció

```rust
// src/collection.rs
impl Collection {
    /// Egy dokumentum törlése
    pub fn delete_one(&self, query: &PyDict) -> PyResult<PyObject> {
        // 1. Query parsing
        let query_json = python_dict_to_json(query)?;

        // 2. Első matching dokumentum keresése
        let (found_doc, found_offset) = {
            let storage = self.storage.read();
            let meta = storage.get_collection_meta(&self.name)
                .ok_or_else(|| PyErr::new::<PyRuntimeError, _>("Collection not found"))?;

            let mut result = None;

            for offset in &meta.data_offsets {
                let doc_bytes = storage.read_data(*offset)
                    .map_err(|e| PyErr::new::<PyIOError, _>(e.to_string()))?;

                let doc: Value = serde_json::from_slice(&doc_bytes)
                    .map_err(|e| PyErr::new::<PyValueError, _>(e.to_string()))?;

                // Skip tombstones
                if doc.get("_tombstone").and_then(|v| v.as_bool()).unwrap_or(false) {
                    continue;
                }

                // Query matching
                if query::matches(&doc, &query_json)? {
                    result = Some((doc, *offset));
                    break;
                }
            }

            result
        };

        // Nem találtuk
        if found_doc.is_none() {
            return Python::with_gil(|py| {
                let result = PyDict::new(py);
                result.set_item("acknowledged", true)?;
                result.set_item("deleted_count", 0)?;
                Ok(result.into())
            });
        }

        let (doc, _offset) = found_doc.unwrap();

        // 3. Tombstone írása
        {
            let mut storage = self.storage.write();

            let tombstone = json!({
                "_tombstone": true,
                "_id": doc["_id"],
                "_delete_type": "Explicit",
                "_deleted_at": current_timestamp_ms(),
            });

            let tombstone_bytes = serde_json::to_vec(&tombstone)
                .map_err(|e| PyErr::new::<PyValueError, _>(e.to_string()))?;

            storage.write_data(&tombstone_bytes)
                .map_err(|e| PyErr::new::<PyIOError, _>(e.to_string()))?;

            // 4. Metadata frissítés
            let mut meta = storage.get_collection_meta(&self.name).unwrap().clone();
            meta.document_count -= 1;
            meta.tombstone_count += 1;

            storage.update_collection_meta(&self.name, meta)
                .map_err(|e| PyErr::new::<PyRuntimeError, _>(e.to_string()))?;

            // 5. Index frissítés
            // TODO: remove_from_indexes(&doc);
        }

        // 6. Eredmény visszaadása
        Python::with_gil(|py| {
            let result = PyDict::new(py);
            result.set_item("acknowledged", true)?;
            result.set_item("deleted_count", 1)?;
            Ok(result.into())
        })
    }
}
```

---

### Compaction Implementáció

```rust
// src/compaction.rs
use std::fs::{File, rename, remove_file};
use std::path::{Path, PathBuf};
use crate::storage::{StorageEngine, CollectionMeta};
use crate::error::Result;

pub struct Compactor {
    db_path: PathBuf,
    threshold: f64,  // 0.3 = 30%
}

impl Compactor {
    pub fn new<P: AsRef<Path>>(db_path: P) -> Self {
        Compactor {
            db_path: db_path.as_ref().to_path_buf(),
            threshold: 0.3,
        }
    }

    /// Compaction szükséges-e ellenőrzés
    pub fn should_compact(&self, storage: &StorageEngine) -> bool {
        for meta in storage.collections.values() {
            let total = meta.document_count + meta.tombstone_count;
            if total == 0 {
                continue;
            }

            let tombstone_ratio = meta.tombstone_count as f64 / total as f64;
            if tombstone_ratio > self.threshold {
                return true;
            }
        }

        false
    }

    /// Compaction végrehajtása
    pub fn compact(&self, storage: &mut StorageEngine) -> Result<CompactionStats> {
        let start_time = current_timestamp_ms();

        // 1. Új fájl létrehozása
        let compact_path = self.db_path.with_extension("mlite.compact");
        let mut compact_file = File::create(&compact_path)?;

        // 2. Header másolása
        let new_header = storage.header.clone();
        let mut new_collections = HashMap::new();

        // 3. Collection-önként compaction
        let original_size = storage.file.metadata()?.len();
        let mut stats = CompactionStats {
            started_at: start_time,
            completed_at: 0,
            duration_ms: 0,
            original_size,
            compacted_size: 0,
            space_saved: 0,
            tombstones_removed: 0,
            documents_kept: 0,
            collections: Vec::new(),
        };

        for (name, old_meta) in &storage.collections {
            let collection_stats = self.compact_collection(
                storage,
                name,
                old_meta,
                &mut compact_file,
            )?;

            stats.tombstones_removed += collection_stats.tombstones_before;
            stats.documents_kept += collection_stats.documents_after;
            stats.collections.push(collection_stats.clone());

            // Új meta
            let new_meta = CollectionMeta {
                name: name.clone(),
                document_count: collection_stats.documents_after,
                tombstone_count: 0,
                data_offset: /* calculated */,
                index_offset: 0,
                last_id: old_meta.last_id,
                last_compaction: Some(current_timestamp_ms()),
            };

            new_collections.insert(name.clone(), new_meta);
        }

        // 4. Metadata írása
        StorageEngine::write_metadata(&mut compact_file, &new_header, &new_collections)?;
        compact_file.sync_all()?;

        let compacted_size = compact_file.metadata()?.len();
        stats.compacted_size = compacted_size;
        stats.space_saved = original_size.saturating_sub(compacted_size);

        // 5. Atomic swap
        let backup_path = self.db_path.with_extension("mlite.old");
        rename(&self.db_path, &backup_path)?;
        rename(&compact_path, &self.db_path)?;

        // 6. Storage reload
        *storage = StorageEngine::open(&self.db_path)?;

        // 7. Backup törlése
        remove_file(backup_path)?;

        stats.completed_at = current_timestamp_ms();
        stats.duration_ms = stats.completed_at - stats.started_at;

        Ok(stats)
    }

    fn compact_collection(
        &self,
        storage: &StorageEngine,
        name: &str,
        old_meta: &CollectionMeta,
        new_file: &mut File,
    ) -> Result<CollectionCompactionStats> {
        let mut stats = CollectionCompactionStats {
            name: name.to_string(),
            documents_before: old_meta.document_count,
            tombstones_before: old_meta.tombstone_count,
            documents_after: 0,
        };

        // Scan all documents
        for offset in &old_meta.data_offsets {
            let doc_bytes = storage.read_data(*offset)?;
            let doc: Value = serde_json::from_slice(&doc_bytes)?;

            // Skip tombstones
            if doc.get("_tombstone").and_then(|v| v.as_bool()).unwrap_or(false) {
                continue;
            }

            // Write live document to new file
            new_file.write_all(&(doc_bytes.len() as u32).to_le_bytes())?;
            new_file.write_all(&doc_bytes)?;

            stats.documents_after += 1;
        }

        Ok(stats)
    }
}

fn current_timestamp_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
```

---

### Python API - Explicit Compaction

```python
# Python API
class MongoLite:
    def compact(self) -> dict:
        """
        Manuális compaction trigger

        Returns:
            CompactionStats dict
        """
        pass

# Használat
db = MongoLite("myapp.mlite")

# Sok törlés után
collection.delete_many({"status": "old"})

# Explicit compaction
stats = db.compact()
print(f"Space saved: {stats['space_saved'] / 1024 / 1024:.2f} MB")
print(f"Duration: {stats['duration_ms']}ms")
```

---

## Edge Case-ek és Hibakezelés

### 1. Delete nem létező dokumentumot

**Eset:**
```python
result = collection.delete_one({"_id": 999999})  # Nincs ilyen
```

**Viselkedés:**
- No-op (nincs hiba)
- `deleted_count: 0`
- MongoDB kompatibilis

---

### 2. Delete ugyanazt kétszer

**Eset:**
```python
collection.delete_one({"_id": 1})
collection.delete_one({"_id": 1})  # Már törölve
```

**Viselkedés:**
- Első: `deleted_count: 1`
- Második: `deleted_count: 0` (tombstone skip)

---

### 3. Delete_many üres query

**Eset:**
```python
collection.delete_many({})  # VESZÉLYES!
```

**Viselkedés:**
- ⚠️ Törli az **összes** dokumentumot
- Opcionális: confirmation prompt (későbbi feature)
- MongoDB viselkedés: törli mindent

**Védelem:**
```python
# Opt-in safe mode
db = MongoLite("app.mlite", safe_mode=True)
collection.delete_many({})  # Raises SafeModeError
```

---

### 4. Compaction közben crash

**Eset:**
- Compaction során áramszünet / process kill

**Védelem:**
1. **Atomic rename:** Régi fájl csak sikeres compaction után törlődik
2. **Backup file:** `.mlite.old` megmarad recovery-hez
3. **Partial file:** `.mlite.compact` nem kerül felhasználásra

**Recovery:**
```rust
fn recover_from_failed_compaction(db_path: &Path) -> Result<()> {
    let compact_path = db_path.with_extension("mlite.compact");
    let backup_path = db_path.with_extension("mlite.old");

    // Ha compact file létezik, de nem lett rename-elve
    if compact_path.exists() {
        remove_file(compact_path)?;  // Törlés
    }

    // Ha backup létezik, vissza kell állítani
    if backup_path.exists() && !db_path.exists() {
        rename(backup_path, db_path)?;
    }

    Ok(())
}
```

---

### 5. Concurrent delete

**Eset:**
- Thread A és B egyszerre törli ugyanazt

**Megoldás:**
- Write lock (már megvan)
- First-come-first-served
- Második thread: `deleted_count: 0`

---

### 6. Delete indexelt dokumentumot

**Eset:**
- Dokumentum indexekben van
- Delete után index inconsistency?

**Megoldás:**
- Delete során index frissítés kötelező
- Atomi művelet (write lock alatt)

**Implementáció:**
```rust
fn delete_with_index_update(doc: &Value, indexes: &mut IndexManager) {
    // 1. Dokumentum törlése index-ből
    for (field, index) in indexes.iter_mut() {
        if let Some(field_value) = doc.get(field) {
            index.remove(field_value, doc["_id"]);
        }
    }

    // 2. Tombstone írása
    write_tombstone(doc);
}
```

---

## Teljesítmény és Optimalizálás

### Delete Teljesítmény

**delete_one():**
- Query matching: O(n) - full scan (index nélkül)
- Tombstone írás: O(1) - append
- Index update: O(log n) per index
- **Összesen: O(n) + O(k log n)** (k = indexek száma)

**delete_many():**
- Query matching: O(n)
- Tombstone írás: O(m) - m deleted docs
- Index update: O(m k log n)
- **Összesen: O(n) + O(mk log n)**

**Optimalizálás:**
- Index használat query-hez: O(log n) + O(m)
- Batch tombstone write: csökkent I/O

---

### Compaction Teljesítmény

**Full compaction:**
- Read all: O(n) documents
- Write live: O(n - t) (t = tombstones)
- Sync: O(1)
- **Összesen: O(n)** lineáris

**Downtime:**
- Write lock alatt: read-only mode
- Tipikus: 100ms per 1000 docs
- 10K docs: ~1 sec
- 100K docs: ~10 sec

**Optimalizálás (v0.3.0):**
- Background compaction: zero downtime
- Incremental compaction: kisebb burst
- Parallel collection compaction: gyorsabb

---

### Compaction Trigger Stratégia

**Option 1: Auto-compact (Agresszív)**
```rust
// Minden delete_many után
if tombstone_ratio > 0.3 {
    compact_database();
}
```
- ✅ Mindig optimális fájlméret
- ❌ Sok compaction (lassú)

**Option 2: Deferred Compact (✅ MVP)**
```rust
// Csak figyelmeztetés
if tombstone_ratio > 0.3 {
    log::warn!("Compaction recommended");
}

// User explicit compact
db.compact();
```
- ✅ User control
- ❌ Lehet elfelejti

**Option 3: Scheduled Compact (v0.3.0)**
```python
# Cron job
@schedule.every().day.at("02:00")
def nightly_compaction():
    db.compact()
```
- ✅ Predictable maintenance window
- ✅ Background task

**MVP választás:** Option 2 + Optional auto-compact flag

---

### Memory Használat

**Delete:**
- 1 dokumentum deserializálás (~2x doc size)
- Tombstone: ~200 bytes
- Index update: O(log n) stack space
- **Összesen: ~2x doc size + 200 bytes**

**Compaction:**
- Full file read: streaming (constant memory)
- Buffer: 4KB page
- Metadata: O(collections)
- **Összesen: ~10MB típikus**

---

## Tesztelési Stratégia

### Unit tesztek

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delete_one_existing() {
        let mut db = setup_test_db();
        let collection = db.collection("users");

        collection.insert_one(json!({"_id": 1, "name": "János"}));

        let result = collection.delete_one(json!({"_id": 1}));
        assert_eq!(result.deleted_count, 1);

        // Verify tombstone
        let count = collection.count_documents(json!({}));
        assert_eq!(count, 0);
    }

    #[test]
    fn test_delete_one_nonexistent() {
        let collection = setup_test_collection();

        let result = collection.delete_one(json!({"_id": 999}));
        assert_eq!(result.deleted_count, 0);
    }

    #[test]
    fn test_delete_many_all() {
        let collection = setup_test_collection();
        collection.insert_many(vec![
            json!({"_id": 1, "status": "old"}),
            json!({"_id": 2, "status": "old"}),
            json!({"_id": 3, "status": "new"}),
        ]);

        let result = collection.delete_many(json!({"status": "old"}));
        assert_eq!(result.deleted_count, 2);

        let count = collection.count_documents(json!({}));
        assert_eq!(count, 1);
    }

    #[test]
    fn test_compaction() {
        let mut db = setup_test_db();
        let collection = db.collection("users");

        // Insert 100 docs
        for i in 0..100 {
            collection.insert_one(json!({"_id": i, "value": i}));
        }

        // Delete 50
        collection.delete_many(json!({"value": {"$lt": 50}}));

        // Check tombstone ratio
        let meta = db.storage.get_collection_meta("users").unwrap();
        assert_eq!(meta.tombstone_count, 50);

        // Compact
        let stats = db.compact().unwrap();
        assert_eq!(stats.tombstones_removed, 50);
        assert_eq!(stats.documents_kept, 50);

        // Verify
        let meta_after = db.storage.get_collection_meta("users").unwrap();
        assert_eq!(meta_after.tombstone_count, 0);
        assert_eq!(meta_after.document_count, 50);
    }

    #[test]
    fn test_compaction_crash_recovery() {
        // Simulate crash during compaction
        let db_path = "test_crash.mlite";
        let compact_path = "test_crash.mlite.compact";

        // Create partial compact file
        std::fs::write(compact_path, b"partial data").unwrap();

        // Recovery
        recover_from_failed_compaction(Path::new(db_path)).unwrap();

        // Verify compact file removed
        assert!(!Path::new(compact_path).exists());
    }
}
```

---

## Roadmap

### MVP (v0.2.0) - 1-2 hét
- ✅ `delete_one()` implementáció
- ✅ `delete_many()` implementáció
- ✅ Tombstone pattern
- ✅ CollectionMeta tombstone_count mező
- ✅ Alapvető compaction (manual trigger)

### v0.2.1 - 1 hét
- ✅ Auto-compact threshold (opt-in)
- ✅ Compaction stats API
- ✅ Recovery from failed compaction

### v0.3.0 - 2 hét
- ✅ Background compaction (separate thread)
- ✅ Incremental compaction (collection-by-collection)
- ✅ Scheduled compaction (cron-like)
- ✅ Safe mode (confirmation for delete_many({}))

### v1.0.0 - Production
- ✅ Zero-downtime compaction
- ✅ Compaction telemetry
- ✅ Automatic threshold tuning
- ✅ Compaction pause/resume

---

## Összefoglalás

### Kulcs Döntések

1. **Tombstone Pattern** (logikai törlés)
   - Append-only konzisztens
   - Crash-safe
   - Compaction később

2. **Full Compaction** (MVP)
   - Teljes fájl újraírás
   - Egyszerű implementáció
   - Elfogadható kis-közepes fájloknál

3. **Manual Trigger** (MVP)
   - User kontroll
   - Opt-in auto-compact
   - Scheduled későbbi feature

4. **30% Threshold**
   - Tombstone arány > 30% → compaction ajánlott
   - Konfigurálható
   - Trade-off: performance vs. disk space

### Implementációs Sorrend

1. **Hét 1:** delete_one + tombstone
2. **Hét 2:** delete_many + batch tombstone
3. **Hét 3:** Compaction algoritmus
4. **Hét 4:** Recovery + tesztelés

### Sikerkritériumok

- ✅ MongoDB-kompatibilis delete API
- ✅ < 2ms delete latency (single doc)
- ✅ Compaction < 10ms per 1000 docs
- ✅ Crash recovery működik
- ✅ 50%+ disk space savings compaction után

---

**Következő lépés:** `IMPLEMENTATION_INDEX.md` - B-tree indexelés részletes terv
