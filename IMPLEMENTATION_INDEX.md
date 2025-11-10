# B-tree Indexelés - Részletes Implementációs Terv

## Tartalomjegyzék
1. [Áttekintés](#áttekintés)
2. [Stratégiai Döntés: B-tree vs. Alternatívák](#stratégiai-döntés-b-tree-vs-alternatívák)
3. [B-tree Alapok és Paraméterek](#b-tree-alapok-és-paraméterek)
4. [Index API Specifikáció](#index-api-specifikáció)
5. [B-tree Node Struktúra](#b-tree-node-struktúra)
6. [Algoritmusok és Pszeudokód](#algoritmusok-és-pszeudokód)
7. [Fájl Formátum és Tárolás](#fájl-formátum-és-tárolás)
8. [Implementációs Példák](#implementációs-példák)
9. [Query Optimizer Integráció](#query-optimizer-integráció)
10. [Teljesítmény és Optimalizálás](#teljesítmény-és-optimalizálás)

---

## Áttekintés

Az indexelés célja a query teljesítmény javítása. Ahelyett, hogy minden query-nél az összes dokumentumot scan-nelnénk (O(n)), az index lehetővé teszi a gyors keresést (O(log n)).

### Célok
- ✅ `create_index()` - index létrehozása mezőre
- ✅ Automatikus `_id` index
- ✅ Unique index támogatás
- ✅ Composite index (több mező, későbbi)
- ✅ Index-based query optimization
- ✅ Index persistence (fájlba mentés)

---

## Stratégiai Döntés: B-tree vs. Alternatívák

### Összehasonlítás

| Adatstruktúra | Search | Insert | Delete | Space | Disk-friendly | MongoDB |
|---------------|--------|--------|--------|-------|---------------|---------|
| **B-tree** | O(log n) | O(log n) | O(log n) | O(n) | ✅ Igen (page-based) | ✅ Használja |
| HashMap | O(1) | O(1) | O(1) | O(n) | ❌ Nem (pointer-based) | ❌ Nem |
| AVL/Red-Black | O(log n) | O(log n) | O(log n) | O(n) | ❌ Nem | ❌ Nem |
| Skip List | O(log n) | O(log n) | O(log n) | O(n log n) | ⚠️ Részben | ❌ Nem |
| LSM-tree | O(log n) | O(1)* | O(log n) | O(n) | ✅ Igen | ⚠️ RocksDB |

\* Amortized

---

### B-tree Előnyök MongoLite Számára

1. **Disk-oriented design**
   - Page/block alapú tárolás
   - Memory-mapped fájlokhoz ideális
   - Kevés disk I/O (nagy fan-out)

2. **Range query támogatás**
   - `$gt`, `$gte`, `$lt`, `$lte` operátorok
   - Sequential scan (in-order traversal)

3. **MongoDB kompatibilitás**
   - MongoDB B-tree-t használ (WiredTiger)
   - Ismerős konzisztencia

4. **Egyszerű implementáció**
   - Jól dokumentált algoritmusok
   - Sok referencia implementáció

---

### **DÖNTÉS: B+ tree (B-tree variáns)**

**Miért B+ tree, nem sima B-tree?**

| Feature | B-tree | B+ tree |
|---------|--------|---------|
| Adat tárolás | Internal nodes-ban is | Csak leaf nodes-ban |
| Range scan | Lassabb | Gyors (linked leaves) |
| Search | Gyorsabb (korai stop) | Lassabb (mindig leaf) |
| Disk I/O | Több | Kevesebb (sequential) |

**B+ tree előnyök MongoLite-nak:**
- ✅ Range query optimalizálás (MongoDB operátorok)
- ✅ Sequential scan (full index scan)
- ✅ Leaf-level linked list (gyors iteration)
- ✅ Compaction-friendly (sequential writes)

**MVP paraméterek:**
- **Order (m):** 32 (max 32 keys per node)
- **Min keys:** m/2 = 16 (kivéve root)
- **Max keys:** m - 1 = 31
- **Max children:** m = 32

---

## B-tree Alapok és Paraméterek

### B+ tree Tulajdonságok

1. **Minden internal node** (nem-levél):
   - `[k/2, k]` kulcsok (k = max keys)
   - `k+1` gyerek pointer
   - Kulcsok rendezettek

2. **Minden leaf node** (levél):
   - `[k/2, k]` kulcsok
   - Adat pointerek (document offset)
   - Linked list (next leaf pointer)

3. **Root node:**
   - Minimum 1 kulcs (ha nem egyedüli node)
   - 2-k+1 gyerek

4. **Egyensúly:**
   - Minden leaf azonos mélységben
   - Balanced tree (garantált O(log n))

---

### MongoLite B+ tree Paraméterek

```rust
const BTREE_ORDER: usize = 32;           // Order m
const MAX_KEYS: usize = BTREE_ORDER - 1; // 31
const MIN_KEYS: usize = BTREE_ORDER / 2; // 16
const MAX_CHILDREN: usize = BTREE_ORDER; // 32
```

**Trade-off elemzés:**

| Order | Height (1M docs) | Node size | I/O per lookup | Fan-out |
|-------|------------------|-----------|----------------|---------|
| 16 | ~5 | 512B | 5 | Low |
| 32 | ~4 | 1KB | 4 | Good ✅ |
| 64 | ~3 | 2KB | 3 | High |
| 128 | ~3 | 4KB | 3 | Very high |

**Választás:** Order 32
- Good balance (height vs. node size)
- 1KB node = cache friendly
- 1M docs = 4 levels = 4 disk reads

---

## Index API Specifikáció

### 1. create_index() - Index létrehozása

**Python API:**
```python
collection.create_index(
    field: str,
    unique: bool = False,
    sparse: bool = False,
    name: str = None
) -> str  # Index name
```

**Paraméterek:**
- `field`: Mező neve (pl. "age", "user.profile.email")
- `unique`: Unique constraint (duplikátum tiltás)
- `sparse`: Csak dokumentumok ahol a mező létezik
- `name`: Index neve (default: `{field}_1`)

**Példák:**
```python
# Egyszerű index
collection.create_index("age")

# Unique index
collection.create_index("email", unique=True)

# Sparse index
collection.create_index("optional_field", sparse=True)

# Custom név
collection.create_index("email", name="email_unique_idx", unique=True)
```

**Visszatérés:**
```python
"age_1"  # Index név
```

---

### 2. drop_index() - Index törlése

**Python API:**
```python
collection.drop_index(name: str) -> None
```

**Példa:**
```python
collection.drop_index("age_1")
```

---

### 3. list_indexes() - Indexek listázása

**Python API:**
```python
collection.list_indexes() -> List[dict]
```

**Visszatérés:**
```python
[
    {
        "name": "_id_",
        "field": "_id",
        "unique": True,
        "sparse": False,
        "size": 1024,  # bytes
        "num_keys": 100
    },
    {
        "name": "age_1",
        "field": "age",
        "unique": False,
        "sparse": False,
        "size": 512,
        "num_keys": 100
    }
]
```

---

### 4. Automatikus _id Index

**Működés:**
- Minden collection automatikusan kap `_id` indexet
- Unique constraint
- Create collection során létrejön

**Implementáció:**
```rust
impl StorageEngine {
    pub fn create_collection(&mut self, name: &str) -> Result<()> {
        // ... collection létrehozás ...

        // Automatikus _id index
        let id_index = BPlusTree::new(BTREE_ORDER);
        self.indexes.insert(
            format!("{}._id_", name),
            id_index
        );

        Ok(())
    }
}
```

---

## B-tree Node Struktúra

### Node Típusok

```rust
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BTreeNode {
    Internal(InternalNode),
    Leaf(LeafNode),
}

/// Internal node (nem-levél)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InternalNode {
    /// Kulcsok (rendezett)
    pub keys: Vec<IndexKey>,

    /// Gyerek node offset-ek (fájlban)
    pub children: Vec<u64>,  // length = keys.len() + 1

    /// Node ID (fájl offset)
    pub offset: u64,
}

/// Leaf node (levél)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeafNode {
    /// Kulcsok (rendezett)
    pub keys: Vec<IndexKey>,

    /// Dokumentum offset-ek
    pub values: Vec<u64>,  // length = keys.len()

    /// Következő leaf pointer (linked list)
    pub next_leaf: Option<u64>,

    /// Node ID
    pub offset: u64,
}

/// Index kulcs (támogatott típusok)
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum IndexKey {
    Null,
    Int(i64),
    Float(OrderedFloat<f64>),  // Rendezett float
    String(String),
    Bool(bool),
    // Composite (later): Vec<IndexKey>
}

/// Helper: f64 összehasonlítható wrapper
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct OrderedFloat(f64);

impl Eq for OrderedFloat {}
impl Ord for OrderedFloat {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.partial_cmp(other).unwrap_or(std::cmp::Ordering::Equal)
    }
}
```

---

### B+ tree Struktúra

```rust
#[derive(Debug)]
pub struct BPlusTree {
    /// Root node offset
    pub root: u64,

    /// Tree order (m)
    pub order: usize,

    /// Index metadata
    pub meta: IndexMeta,

    /// In-memory cache (későbbi optimalizálás)
    cache: HashMap<u64, BTreeNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexMeta {
    pub name: String,
    pub field: String,
    pub unique: bool,
    pub sparse: bool,
    pub num_keys: u64,
    pub tree_height: u32,
    pub root_offset: u64,
}
```

---

## Algoritmusok és Pszeudokód

### 1. Search Algoritmus

```
FUNCTION search(tree, key) -> Option<u64>:
    node = load_node(tree.root)

    WHILE True:
        MATCH node:
            CASE InternalNode:
                // Binary search a kulcsok között
                child_index = binary_search_child(node.keys, key)
                child_offset = node.children[child_index]
                node = load_node(child_offset)

            CASE LeafNode:
                // Binary search leaf-ben
                index = binary_search(node.keys, key)

                IF index < node.keys.len() AND node.keys[index] == key:
                    RETURN Some(node.values[index])
                ELSE:
                    RETURN None
                END IF
        END MATCH
    END WHILE
END FUNCTION

FUNCTION binary_search_child(keys, search_key) -> usize:
    // Megkeresi a megfelelő gyerek indexét
    left = 0
    right = keys.len()

    WHILE left < right:
        mid = (left + right) / 2

        IF search_key < keys[mid]:
            right = mid
        ELSE:
            left = mid + 1
        END IF
    END WHILE

    RETURN left
END FUNCTION
```

**Komplexitás:**
- **Legjobb:** O(log n) - balanced tree
- **Legrosszabb:** O(log n) - balanced tree
- **Space:** O(1) - iteratív

---

### 2. Insert Algoritmus

```
FUNCTION insert(tree, key, value) -> Result<()>:
    // 1. Unique check (ha szükséges)
    IF tree.meta.unique AND search(tree, key).is_some():
        RETURN Error("Duplicate key")
    END IF

    // 2. Root node betöltése
    root = load_node(tree.root)

    // 3. Insert rekurzívan
    new_child = insert_recursive(tree, root, key, value)

    // 4. Root split kezelése
    IF new_child.is_some():
        // Root split történt, új root kell
        old_root = root
        new_root = InternalNode {
            keys: [new_child.key],
            children: [old_root.offset, new_child.offset],
        }

        tree.root = save_node(new_root)
        tree.meta.tree_height += 1
    END IF

    tree.meta.num_keys += 1
    RETURN Ok(())
END FUNCTION

FUNCTION insert_recursive(tree, node, key, value) -> Option<SplitResult>:
    MATCH node:
        CASE LeafNode:
            RETURN insert_into_leaf(tree, node, key, value)

        CASE InternalNode:
            // 1. Megkeresi a megfelelő gyereket
            child_index = binary_search_child(node.keys, key)
            child = load_node(node.children[child_index])

            // 2. Rekurzív insert
            split_result = insert_recursive(tree, child, key, value)

            // 3. Split kezelése
            IF split_result.is_some():
                RETURN insert_into_internal(tree, node, child_index, split_result)
            END IF

            RETURN None
    END MATCH
END FUNCTION

FUNCTION insert_into_leaf(tree, leaf, key, value) -> Option<SplitResult>:
    // 1. Beszúrási pozíció keresése
    insert_pos = binary_search_insert_pos(leaf.keys, key)

    // 2. Beszúrás
    leaf.keys.insert(insert_pos, key)
    leaf.values.insert(insert_pos, value)

    // 3. Split szükséges?
    IF leaf.keys.len() <= MAX_KEYS:
        save_node(leaf)
        RETURN None  // Nincs split
    END IF

    // 4. Leaf split
    mid = leaf.keys.len() / 2

    // Bal oldali node (meglévő)
    left_leaf = LeafNode {
        keys: leaf.keys[0..mid],
        values: leaf.values[0..mid],
        next_leaf: Some(right_offset),  // Later
        offset: leaf.offset,
    }

    // Jobb oldali node (új)
    right_leaf = LeafNode {
        keys: leaf.keys[mid..],
        values: leaf.values[mid..],
        next_leaf: leaf.next_leaf,
        offset: allocate_new_offset(),
    }

    // Linked list frissítés
    left_leaf.next_leaf = Some(right_leaf.offset)

    save_node(left_leaf)
    right_offset = save_node(right_leaf)

    // Split eredmény: jobb oldal első kulcsa
    RETURN Some(SplitResult {
        key: right_leaf.keys[0],
        offset: right_offset,
    })
END FUNCTION

FUNCTION insert_into_internal(tree, internal, child_index, split_result) -> Option<SplitResult>:
    // 1. Új kulcs és gyerek beszúrása
    internal.keys.insert(child_index, split_result.key)
    internal.children.insert(child_index + 1, split_result.offset)

    // 2. Split szükséges?
    IF internal.keys.len() <= MAX_KEYS:
        save_node(internal)
        RETURN None
    END IF

    // 3. Internal node split
    mid = internal.keys.len() / 2

    // Bal oldali (meglévő)
    left_internal = InternalNode {
        keys: internal.keys[0..mid],
        children: internal.children[0..mid+1],
        offset: internal.offset,
    }

    // Jobb oldali (új)
    right_internal = InternalNode {
        keys: internal.keys[mid+1..],  // Mid kulcs felfelé megy!
        children: internal.children[mid+1..],
        offset: allocate_new_offset(),
    }

    save_node(left_internal)
    right_offset = save_node(right_internal)

    // Split eredmény: középső kulcs
    RETURN Some(SplitResult {
        key: internal.keys[mid],  // Középső felfelé
        offset: right_offset,
    })
END FUNCTION

STRUCT SplitResult {
    key: IndexKey,
    offset: u64,
}
```

**Komplexitás:**
- **Search path:** O(log n)
- **Split cascade:** O(log n) worst case
- **Összesen:** O(log n)

---

### 3. Delete Algoritmus (Egyszerűsített MVP)

**MVP stratégia:** Lazy delete (nem merge/redistribute)

```
FUNCTION delete(tree, key) -> Result<bool>:
    root = load_node(tree.root)
    deleted = delete_recursive(tree, root, key)

    IF deleted:
        tree.meta.num_keys -= 1
    END IF

    RETURN Ok(deleted)
END FUNCTION

FUNCTION delete_recursive(tree, node, key) -> bool:
    MATCH node:
        CASE LeafNode:
            index = binary_search(node.keys, key)

            IF index < node.keys.len() AND node.keys[index] == key:
                node.keys.remove(index)
                node.values.remove(index)
                save_node(node)
                RETURN True
            END IF

            RETURN False

        CASE InternalNode:
            child_index = binary_search_child(node.keys, key)
            child = load_node(node.children[child_index])

            RETURN delete_recursive(tree, child, key)
    END MATCH
END FUNCTION
```

**Megjegyzés:**
- **Nem végez merge/redistribute** (egyszerűsítés)
- Node lehet under-utilized (< min keys)
- Compaction később újraépíti az indexet (balanced)

**Teljes delete (későbbi v0.3.0):**
- Merge under-utilized nodes
- Redistribute keys
- Decrease height ha szükséges

---

### 4. Range Scan Algoritmus

**Használat:** `$gt`, `$gte`, `$lt`, `$lte` operátorok

```
FUNCTION range_scan(tree, start_key, end_key, inclusive_start, inclusive_end) -> Vec<u64>:
    results = []

    // 1. Keresés: kezdő leaf megtalálása
    leaf = find_leaf_for_key(tree, start_key)

    // 2. Linked list traversal
    WHILE leaf.is_some():
        current_leaf = leaf.unwrap()

        FOR i IN 0..current_leaf.keys.len():
            key = current_leaf.keys[i]
            value = current_leaf.values[i]

            // Start check
            IF key < start_key OR (NOT inclusive_start AND key == start_key):
                CONTINUE
            END IF

            // End check
            IF key > end_key OR (NOT inclusive_end AND key == end_key):
                RETURN results  // Done
            END IF

            // In range
            results.push(value)
        END FOR

        // Következő leaf
        leaf = current_leaf.next_leaf.map(|offset| load_node(offset))
    END WHILE

    RETURN results
END FUNCTION

FUNCTION find_leaf_for_key(tree, key) -> LeafNode:
    node = load_node(tree.root)

    WHILE True:
        MATCH node:
            CASE InternalNode:
                child_index = binary_search_child(node.keys, key)
                node = load_node(node.children[child_index])

            CASE LeafNode:
                RETURN node
        END MATCH
    END WHILE
END FUNCTION
```

**Komplexitás:**
- **Find start:** O(log n)
- **Scan:** O(k) - k találatok
- **Összesen:** O(log n + k)

---

## Fájl Formátum és Tárolás

### Index Fájl Struktúra

**Fájlnév:** `<database>.mlite.idx`

```
┌─────────────────────────────────────┐
│  Index File Header (256 bytes)      │
│  - Magic: "MLITEIDX" (8 bytes)      │
│  - Version: u32                     │
│  - Index count: u32                 │
│  - Free list head: u64              │
├─────────────────────────────────────┤
│  Index Metadata Array               │
│  [IndexMeta 1]                      │
│  [IndexMeta 2]                      │
│  ...                                │
├─────────────────────────────────────┤
│  B-tree Nodes (variable size)       │
│  [Node 1: offset 0]                 │
│  [Node 2: offset X]                 │
│  [Node 3: offset Y]                 │
│  ...                                │
└─────────────────────────────────────┘
```

---

### Node Serialization

**Node formátum:**
```
[u32 length][u8 node_type][bincode serialized node]
```

- **length:** Node size bytes
- **node_type:** 0 = Internal, 1 = Leaf
- **data:** Bincode serialized

**Példa (Leaf Node):**
```
[00 00 03 2A]  // Length: 810 bytes
[01]           // Type: Leaf
[... bincode data ...]
```

---

### IndexMeta Serialization

```rust
#[derive(Serialize, Deserialize)]
pub struct IndexMetaOnDisk {
    pub name: String,
    pub collection_name: String,
    pub field: String,
    pub unique: bool,
    pub sparse: bool,
    pub num_keys: u64,
    pub tree_height: u32,
    pub root_offset: u64,
    pub created_at: u64,
}
```

**Fájlban:** JSON (length-prefixed)
```
[u32 length][JSON bytes]
```

---

## Implementációs Példák

### Rust B+ tree Insert

```rust
// src/btree.rs
impl BPlusTree {
    pub fn insert(&mut self, key: IndexKey, value: u64, storage: &mut IndexStorage) -> Result<()> {
        // Unique check
        if self.meta.unique && self.search(&key, storage)?.is_some() {
            return Err(MongoLiteError::DuplicateKey(format!("{:?}", key)));
        }

        // Load root
        let root = storage.load_node(self.root)?;

        // Insert recursive
        let split_result = self.insert_recursive(root, key, value, storage)?;

        // Handle root split
        if let Some(split) = split_result {
            // Create new root
            let new_root = BTreeNode::Internal(InternalNode {
                keys: vec![split.key],
                children: vec![self.root, split.offset],
                offset: storage.allocate_offset(),
            });

            self.root = storage.save_node(new_root)?;
            self.meta.tree_height += 1;
        }

        self.meta.num_keys += 1;
        Ok(())
    }

    fn insert_recursive(
        &self,
        node: BTreeNode,
        key: IndexKey,
        value: u64,
        storage: &mut IndexStorage,
    ) -> Result<Option<SplitResult>> {
        match node {
            BTreeNode::Leaf(mut leaf) => {
                // Binary search insert position
                let insert_pos = leaf.keys.binary_search(&key)
                    .unwrap_or_else(|pos| pos);

                // Insert
                leaf.keys.insert(insert_pos, key.clone());
                leaf.values.insert(insert_pos, value);

                // Check split
                if leaf.keys.len() <= MAX_KEYS {
                    storage.save_node(BTreeNode::Leaf(leaf))?;
                    return Ok(None);
                }

                // Split leaf
                let mid = leaf.keys.len() / 2;

                let right_leaf = LeafNode {
                    keys: leaf.keys.split_off(mid),
                    values: leaf.values.split_off(mid),
                    next_leaf: leaf.next_leaf,
                    offset: storage.allocate_offset(),
                };

                let right_offset = storage.save_node(BTreeNode::Leaf(right_leaf.clone()))?;

                leaf.next_leaf = Some(right_offset);
                storage.save_node(BTreeNode::Leaf(leaf))?;

                Ok(Some(SplitResult {
                    key: right_leaf.keys[0].clone(),
                    offset: right_offset,
                }))
            },

            BTreeNode::Internal(mut internal) => {
                // Find child
                let child_index = self.find_child_index(&internal.keys, &key);
                let child = storage.load_node(internal.children[child_index])?;

                // Recursive insert
                let split_result = self.insert_recursive(child, key, value, storage)?;

                if let Some(split) = split_result {
                    // Insert into internal
                    internal.keys.insert(child_index, split.key.clone());
                    internal.children.insert(child_index + 1, split.offset);

                    // Check split
                    if internal.keys.len() <= MAX_KEYS {
                        storage.save_node(BTreeNode::Internal(internal))?;
                        return Ok(None);
                    }

                    // Split internal
                    let mid = internal.keys.len() / 2;
                    let mid_key = internal.keys[mid].clone();

                    let right_internal = InternalNode {
                        keys: internal.keys.split_off(mid + 1),
                        children: internal.children.split_off(mid + 1),
                        offset: storage.allocate_offset(),
                    };

                    // Remove mid key (it goes up)
                    internal.keys.pop();

                    let right_offset = storage.save_node(BTreeNode::Internal(right_internal))?;
                    storage.save_node(BTreeNode::Internal(internal))?;

                    return Ok(Some(SplitResult {
                        key: mid_key,
                        offset: right_offset,
                    }));
                }

                Ok(None)
            }
        }
    }

    fn find_child_index(&self, keys: &[IndexKey], search_key: &IndexKey) -> usize {
        keys.binary_search(search_key)
            .unwrap_or_else(|pos| pos)
    }
}

struct SplitResult {
    key: IndexKey,
    offset: u64,
}
```

---

### Python API Példa

```python
# create_index használat
collection = db.collection("users")

# Egyszerű index
collection.create_index("age")

# Query gyorsítás
users_over_30 = collection.find({"age": {"$gt": 30}})
# Index használat: O(log n) + O(k) találatok

# Unique email constraint
collection.create_index("email", unique=True)

# Insert hibát dob ha duplikált email
try:
    collection.insert_one({"email": "test@example.com"})
    collection.insert_one({"email": "test@example.com"})  # DuplicateKeyError
except Exception as e:
    print(f"Error: {e}")

# Index lista
indexes = collection.list_indexes()
for idx in indexes:
    print(f"{idx['name']}: {idx['field']} ({idx['num_keys']} keys)")

# Index törlés
collection.drop_index("age_1")
```

---

## Query Optimizer Integráció

### Index Selection (Egyszerű Heurisztika MVP)

```rust
pub fn select_index_for_query(
    query: &Query,
    available_indexes: &HashMap<String, BPlusTree>,
) -> Option<String> {
    // Priority ranking
    let mut candidates = Vec::new();

    for (index_name, index) in available_indexes {
        let field = &index.meta.field;

        // 1. Equality match ($eq vagy direct value)
        if query.has_equality_on(field) {
            candidates.push((index_name, 100));  // Highest priority
        }

        // 2. Range operators ($gt, $lt, stb.)
        else if query.has_range_on(field) {
            candidates.push((index_name, 50));
        }

        // 3. Exists check
        else if query.has_exists_on(field) {
            candidates.push((index_name, 10));
        }
    }

    // Sort by priority
    candidates.sort_by_key(|(_, priority)| -*priority);

    // Return best
    candidates.first().map(|(name, _)| name.to_string())
}
```

**Query példák:**

```python
# Equality - _id index használat (highest priority)
collection.find({"_id": 123})
# Index: _id_

# Range - age index használat
collection.find({"age": {"$gt": 30}})
# Index: age_1 (ha van)

# Compound - első mező indexe
collection.find({"age": 25, "city": "Budapest"})
# Index: age_1 (ha van), vagy full scan
```

---

## Teljesítmény és Optimalizálás

### Index vs. Full Scan Költség

**Full scan:**
- Read all documents: O(n)
- Deserialize: O(n)
- Filter: O(n)
- **Total: O(n)**

**Index scan:**
- B-tree search: O(log n)
- Document reads: O(k) - k találatok
- **Total: O(log n + k)**

**Breakeven point:**
- Ha k (találatok) < n/10 → index gyorsabb
- Ha k ≈ n → full scan gyorsabb (sequential I/O)

---

### Index Méret

**Node size:**
- Order 32 → ~1KB per node
- 1M keys → ~31K nodes → 31MB index

**Összehasonlítás:**
- Documents: 1M × 1KB = 1GB
- Index: 31MB
- **Overhead: 3%** (elfogadható)

---

### Build Performance

**Index build:**
- Bulk insert: 1M keys → ~10 sec
- Optimization: Batch bottom-up build (későbbi)

**Rebuild strategy (compaction után):**
```
FUNCTION rebuild_index(collection, field):
    new_tree = BPlusTree::new(32)

    FOR doc IN collection.all_documents():
        IF field IN doc:
            key = extract_key(doc, field)
            value = doc.offset
            new_tree.insert(key, value)
        END IF
    END FOR

    RETURN new_tree
END FUNCTION
```

---

## Tesztelési Stratégia

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_btree_insert_search() {
        let mut tree = BPlusTree::new(4);  // Small order for testing

        tree.insert(IndexKey::Int(10), 100);
        tree.insert(IndexKey::Int(20), 200);
        tree.insert(IndexKey::Int(5), 50);

        assert_eq!(tree.search(&IndexKey::Int(10)), Some(100));
        assert_eq!(tree.search(&IndexKey::Int(20)), Some(200));
        assert_eq!(tree.search(&IndexKey::Int(5)), Some(50));
        assert_eq!(tree.search(&IndexKey::Int(99)), None);
    }

    #[test]
    fn test_btree_split() {
        let mut tree = BPlusTree::new(4);  // Max 3 keys

        // Force split
        for i in 0..10 {
            tree.insert(IndexKey::Int(i), i as u64 * 10);
        }

        // Verify all
        for i in 0..10 {
            assert_eq!(tree.search(&IndexKey::Int(i)), Some(i as u64 * 10));
        }

        // Check height
        assert!(tree.meta.tree_height >= 2);
    }

    #[test]
    fn test_unique_constraint() {
        let mut tree = BPlusTree::new_unique(4);

        tree.insert(IndexKey::String("test".into()), 100).unwrap();

        let result = tree.insert(IndexKey::String("test".into()), 200);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), MongoLiteError::DuplicateKey(_)));
    }

    #[test]
    fn test_range_scan() {
        let mut tree = BPlusTree::new(4);

        for i in 0..100 {
            tree.insert(IndexKey::Int(i), i as u64);
        }

        let results = tree.range_scan(
            &IndexKey::Int(10),
            &IndexKey::Int(20),
            true,  // inclusive start
            false, // exclusive end
        );

        assert_eq!(results.len(), 10);  // 10..19
        assert_eq!(results[0], 10);
        assert_eq!(results[9], 19);
    }
}
```

---

## Roadmap

### MVP (v0.2.5) - 2 hét
- ✅ B+ tree implementáció
- ✅ `create_index()`, `drop_index()`
- ✅ Automatikus `_id` index
- ✅ Unique constraint
- ✅ Search, Insert, Delete (lazy)
- ✅ Fájlba mentés/betöltés

### v0.3.0 - 1-2 hét
- ✅ Range scan optimalizálás
- ✅ Index rebuild (compaction után)
- ✅ Delete with merge/redistribute
- ✅ Sparse index
- ✅ Query optimizer alapok

### v0.4.0 - Later
- ✅ Composite index (multi-field)
- ✅ Covered index (no doc lookup)
- ✅ Background index build
- ✅ Index statistics
- ✅ Partial index (filter expression)

---

## Összefoglalás

### Kulcs Döntések

1. **B+ tree (nem sima B-tree)**
   - Range query optimalizálás
   - Sequential scan (linked leaves)
   - Disk-friendly

2. **Order 32**
   - Good balance (height vs. node size)
   - 4 levels for 1M docs
   - 1KB node size (cache-friendly)

3. **Lazy Delete (MVP)**
   - Egyszerű implementáció
   - Compaction rebuild index
   - Teljes delete később

4. **Automatic _id index**
   - MongoDB kompatibilitás
   - Unique constraint
   - Gyors ID lookup

### Implementációs Sorrend

1. **Hét 1:** B+ tree core (insert, search)
2. **Hét 2:** Node serialization, file storage
3. **Hét 3:** create_index API, unique constraint
4. **Hét 4:** Range scan, query integration

### Sikerkritériumok

- ✅ 1M keys: < 5ms search
- ✅ Insert: < 10ms (including disk sync)
- ✅ Index overhead: < 5% of data size
- ✅ Unique constraint enforcement
- ✅ Range scan: O(log n + k)

---

**Következő:** `IMPLEMENTATION_QUERY_OPTIMIZER.md` - Query optimization stratégia
