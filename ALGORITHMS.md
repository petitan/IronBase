# MongoLite - Kritikus Algoritmusok Gyűjteménye

Ez a dokumentum összefoglalja a MongoLite projekt összes kritikus algoritmusát pszeudokóddal, komplexitás elemzéssel és hivatkozásokkal a részletes implementációs dokumentumokra.

---

## Tartalomjegyzék

1. [CRUD Algoritmusok](#crud-algoritmusok)
2. [Query Engine Algoritmusok](#query-engine-algoritmusok)
3. [Index Algoritmusok](#index-algoritmusok)
4. [Storage Algoritmusok](#storage-algoritmusok)
5. [Optimalizációs Algoritmusok](#optimalizációs-algoritmusok)
6. [Komplexitás Összefoglaló](#komplexitás-összefoglaló)

---

## CRUD Algoritmusok

### 1. INSERT_ONE

**Fájl:** `collection.rs`
**Részletek:** `IMPLEMENTATION_UPDATE.md`

```
FUNCTION insert_one(collection, document):
    // 1. ID generálás
    IF "_id" NOT IN document:
        document["_id"] = generate_next_id()
    END IF

    // 2. Validáció
    IF document_size(document) > 16MB:
        THROW DocumentTooLarge
    END IF

    // 3. Szerializálás
    json_bytes = serialize_json(document)

    // 4. Append-only írás
    storage.lock_write()
    offset = storage.append(json_bytes)

    // 5. Index frissítés
    FOR index IN indexes:
        IF index.field IN document:
            index.insert(document[index.field], offset)
        END IF
    END FOR

    // 6. Metadata frissítés
    meta.document_count += 1
    meta.last_id += 1
    storage.update_metadata(meta)

    storage.unlock_write()

    RETURN {acknowledged: true, inserted_id: document["_id"]}
END FUNCTION
```

**Komplexitás:**
- **Idő:** O(1) + O(k log n) - k indexek száma
- **Space:** O(doc_size)
- **I/O:** 1 write (append) + k index updates

---

### 2. FIND / FIND_ONE

**Fájl:** `collection.rs`, `query.rs`
**Részletek:** `IMPLEMENTATION_QUERY_OPTIMIZER.md`

```
FUNCTION find(collection, query):
    // 1. Query parsing
    parsed_query = parse_query(query)

    // 2. Query optimization
    plan = optimizer.optimize(parsed_query, stats, indexes)

    // 3. Plan execution
    results = execute_plan(plan)

    RETURN results
END FUNCTION

FUNCTION execute_plan(plan):
    MATCH plan.type:
        CASE FullScan:
            RETURN full_collection_scan(query)

        CASE IndexScan:
            RETURN index_based_scan(query, plan.index)
    END MATCH
END FUNCTION

FUNCTION full_collection_scan(query):
    results = []
    storage.lock_read()

    FOR offset IN collection.all_offsets:
        doc = storage.read(offset)

        // Skip tombstones
        IF doc["_tombstone"] == true:
            CONTINUE
        END IF

        // Query matching
        IF query_matches(doc, query):
            results.append(doc)
        END IF
    END FOR

    storage.unlock_read()
    RETURN results
END FUNCTION

FUNCTION index_based_scan(query, index):
    results = []

    // 1. Index lookup
    key_range = extract_key_range(query, index.field)
    offsets = index.range_scan(key_range)

    // 2. Document fetch
    FOR offset IN offsets:
        doc = storage.read(offset)

        // Additional filters (non-index fields)
        IF query_matches_all(doc, query):
            results.append(doc)
        END IF
    END FOR

    RETURN results
END FUNCTION
```

**Komplexitás:**
- **Full scan:** O(n) - n dokumentumok
- **Index scan:** O(log n + k) - k találatok
- **Space:** O(k) - eredmények

---

### 3. UPDATE_ONE

**Fájl:** `collection.rs`
**Részletek:** `IMPLEMENTATION_UPDATE.md`

```
FUNCTION update_one(collection, query, update_spec):
    // 1. Find matching document
    storage.lock_read()
    found_doc = find_one(collection, query)
    storage.unlock_read()

    IF found_doc == NULL:
        RETURN {matched: 0, modified: 0}
    END IF

    // 2. Apply update spec
    modified_doc = apply_update_spec(found_doc, update_spec)

    // 3. Check if changed
    IF modified_doc == found_doc:
        RETURN {matched: 1, modified: 0}
    END IF

    // 4. Write new version (append-only)
    storage.lock_write()

    new_offset = storage.append(serialize(modified_doc))

    // 5. Tombstone old version
    tombstone = create_tombstone(found_doc["_id"], new_offset)
    storage.append(serialize(tombstone))

    // 6. Update indexes
    update_indexes(found_doc, modified_doc)

    // 7. Metadata update
    meta.update_count += 1
    storage.update_metadata(meta)

    storage.unlock_write()

    RETURN {matched: 1, modified: 1}
END FUNCTION
```

**Komplexitás:**
- **Idő:** O(n) find + O(1) append + O(k log n) index
- **Space:** O(doc_size)
- **I/O:** 1 read + 2 writes (new doc + tombstone)

---

### 4. DELETE_ONE

**Fájl:** `collection.rs`
**Részletek:** `IMPLEMENTATION_DELETE.md`

```
FUNCTION delete_one(collection, query):
    // 1. Find document
    storage.lock_read()
    found_doc = find_one(collection, query)
    storage.unlock_read()

    IF found_doc == NULL:
        RETURN {deleted_count: 0}
    END IF

    // 2. Write tombstone
    storage.lock_write()

    tombstone = {
        _tombstone: true,
        _id: found_doc["_id"],
        _delete_type: "Explicit",
        _deleted_at: timestamp()
    }

    storage.append(serialize(tombstone))

    // 3. Remove from indexes
    FOR index IN indexes:
        IF index.field IN found_doc:
            index.delete(found_doc[index.field], found_doc._id)
        END IF
    END FOR

    // 4. Metadata update
    meta.document_count -= 1
    meta.tombstone_count += 1
    storage.update_metadata(meta)

    storage.unlock_write()

    // 5. Compaction check
    IF should_compact(meta):
        LOG "Compaction recommended"
    END IF

    RETURN {deleted_count: 1}
END FUNCTION
```

**Komplexitás:**
- **Idő:** O(n) find + O(1) append + O(k log n) index
- **Space:** O(1) - csak tombstone
- **I/O:** 1 write (tombstone)

---

## Query Engine Algoritmusok

### 5. QUERY_MATCHES (Query Evaluation)

**Fájl:** `query.rs`

```
FUNCTION query_matches(document, query) -> bool:
    // Implicit $and ha több mező van
    FOR field, condition IN query:
        IF NOT field_matches(document, field, condition):
            RETURN False
        END IF
    END FOR

    RETURN True
END FUNCTION

FUNCTION field_matches(document, field, condition) -> bool:
    doc_value = get_nested_field(document, field)

    MATCH condition:
        // Egyszerű egyenlőség
        CASE value (not object):
            RETURN doc_value == value

        // Operátor object
        CASE {operator: value}:
            RETURN evaluate_operator(doc_value, operator, value)
    END MATCH
END FUNCTION

FUNCTION evaluate_operator(doc_value, operator, operand) -> bool:
    MATCH operator:
        CASE "$eq":  RETURN doc_value == operand
        CASE "$ne":  RETURN doc_value != operand
        CASE "$gt":  RETURN doc_value > operand
        CASE "$gte": RETURN doc_value >= operand
        CASE "$lt":  RETURN doc_value < operand
        CASE "$lte": RETURN doc_value <= operand
        CASE "$in":  RETURN doc_value IN operand
        CASE "$nin": RETURN doc_value NOT IN operand

        CASE "$and":
            FOR sub_query IN operand:
                IF NOT query_matches(document, sub_query):
                    RETURN False
                END IF
            END FOR
            RETURN True

        CASE "$or":
            FOR sub_query IN operand:
                IF query_matches(document, sub_query):
                    RETURN True
                END IF
            END FOR
            RETURN False

        CASE "$not":
            RETURN NOT query_matches(document, operand)

        CASE "$exists":
            IF operand == true:
                RETURN field EXISTS in document
            ELSE:
                RETURN field NOT EXISTS in document
            END IF

        DEFAULT:
            THROW UnsupportedOperator(operator)
    END MATCH
END FUNCTION
```

**Komplexitás:**
- **Idő:** O(f) - f mezők száma query-ben
- **Space:** O(1)

---

## Index Algoritmusok

### 6. B+ TREE INSERT

**Fájl:** `btree.rs`
**Részletek:** `IMPLEMENTATION_INDEX.md`

```
FUNCTION btree_insert(tree, key, value):
    root = load_node(tree.root)
    split_result = insert_recursive(root, key, value)

    // Root split handling
    IF split_result != NULL:
        new_root = InternalNode {
            keys: [split_result.key],
            children: [tree.root, split_result.offset]
        }
        tree.root = save_node(new_root)
        tree.height += 1
    END IF
END FUNCTION

FUNCTION insert_recursive(node, key, value):
    MATCH node:
        CASE LeafNode:
            // Binary search insert position
            pos = binary_search_insert_pos(node.keys, key)

            // Insert
            node.keys.insert(pos, key)
            node.values.insert(pos, value)

            // Check overflow
            IF node.keys.length <= MAX_KEYS:
                save_node(node)
                RETURN NULL  // No split
            END IF

            // Split leaf
            mid = node.keys.length / 2

            right = LeafNode {
                keys: node.keys[mid..],
                values: node.values[mid..],
                next: node.next
            }

            node.keys = node.keys[..mid]
            node.values = node.values[..mid]
            node.next = right.offset

            save_node(node)
            right_offset = save_node(right)

            RETURN SplitResult { key: right.keys[0], offset: right_offset }

        CASE InternalNode:
            // Find child
            child_index = binary_search_child(node.keys, key)
            child = load_node(node.children[child_index])

            // Recursive insert
            split = insert_recursive(child, key, value)

            IF split == NULL:
                RETURN NULL
            END IF

            // Insert split key into this node
            node.keys.insert(child_index, split.key)
            node.children.insert(child_index + 1, split.offset)

            // Check overflow
            IF node.keys.length <= MAX_KEYS:
                save_node(node)
                RETURN NULL
            END IF

            // Split internal
            mid = node.keys.length / 2
            mid_key = node.keys[mid]

            right = InternalNode {
                keys: node.keys[mid+1..],
                children: node.children[mid+1..]
            }

            node.keys = node.keys[..mid]
            node.children = node.children[..mid+1]

            save_node(node)
            right_offset = save_node(right)

            RETURN SplitResult { key: mid_key, offset: right_offset }
    END MATCH
END FUNCTION
```

**Komplexitás:**
- **Idő:** O(log n) - tree height
- **Space:** O(log n) - recursion stack
- **I/O:** O(log n) node reads/writes

---

### 7. B+ TREE SEARCH

**Fájl:** `btree.rs`

```
FUNCTION btree_search(tree, key):
    node = load_node(tree.root)

    WHILE True:
        MATCH node:
            CASE InternalNode:
                // Binary search for child
                child_index = binary_search_child(node.keys, key)
                node = load_node(node.children[child_index])

            CASE LeafNode:
                // Binary search in leaf
                index = binary_search(node.keys, key)

                IF index < node.keys.length AND node.keys[index] == key:
                    RETURN Some(node.values[index])
                ELSE:
                    RETURN None
                END IF
        END MATCH
    END WHILE
END FUNCTION
```

**Komplexitás:**
- **Idő:** O(log n)
- **Space:** O(1)
- **I/O:** O(log n) node reads

---

### 8. B+ TREE RANGE SCAN

**Fájl:** `btree.rs`

```
FUNCTION btree_range_scan(tree, start_key, end_key):
    results = []

    // 1. Find start leaf
    leaf = find_leaf_for_key(tree, start_key)

    // 2. Linked list traversal
    WHILE leaf != NULL:
        FOR i = 0 TO leaf.keys.length - 1:
            key = leaf.keys[i]
            value = leaf.values[i]

            // Range check
            IF key < start_key:
                CONTINUE
            END IF

            IF key > end_key:
                RETURN results  // Done
            END IF

            results.append(value)
        END FOR

        // Next leaf
        IF leaf.next != NULL:
            leaf = load_node(leaf.next)
        ELSE:
            BREAK
        END IF
    END WHILE

    RETURN results
END FUNCTION
```

**Komplexitás:**
- **Idő:** O(log n + k) - k találatok
- **Space:** O(k)
- **I/O:** O(log n + k/fanout) - traverse leaves

---

## Storage Algoritmusok

### 9. COMPACTION (Garbage Collection)

**Fájl:** `compaction.rs`
**Részletek:** `IMPLEMENTATION_DELETE.md`

```
FUNCTION compact_database(db_path):
    storage.lock_write()  // Exkluzív lock

    // 1. Create new file
    compact_path = db_path + ".compact"
    new_file = create_file(compact_path)

    // 2. Compact each collection
    new_collections = {}

    FOR collection_name, old_meta IN storage.collections:
        new_meta = compact_collection(collection_name, old_meta, new_file)
        new_collections[collection_name] = new_meta
    END FOR

    // 3. Write metadata to new file
    write_metadata(new_file, header, new_collections)

    // 4. Rebuild indexes
    FOR collection_name IN new_collections:
        rebuild_indexes(new_file, collection_name)
    END FOR

    // 5. Sync and close
    new_file.sync()
    new_file.close()

    // 6. Atomic swap
    rename(db_path, db_path + ".old")
    rename(compact_path, db_path)

    // 7. Reload storage
    storage.reload(db_path)

    // 8. Delete old file
    delete(db_path + ".old")

    storage.unlock_write()
END FUNCTION

FUNCTION compact_collection(name, old_meta, new_file):
    new_meta = CollectionMeta::new(name)
    documents_written = 0

    // Scan all documents
    FOR offset IN old_meta.data_offsets:
        doc = storage.read(offset)

        // Skip tombstones
        IF doc["_tombstone"] == true:
            CONTINUE
        END IF

        // Write live document
        new_offset = new_file.append(serialize(doc))
        documents_written += 1
    END FOR

    new_meta.document_count = documents_written
    new_meta.tombstone_count = 0

    RETURN new_meta
END FUNCTION
```

**Komplexitás:**
- **Idő:** O(n) - n dokumentumok
- **Space:** O(1) - streaming
- **I/O:** O(n) reads + O(n-t) writes (t = tombstones)

---

## Optimalizációs Algoritmusok

### 10. INDEX SELECTION (Query Optimizer)

**Fájl:** `query_optimizer.rs`
**Részletek:** `IMPLEMENTATION_QUERY_OPTIMIZER.md`

```
FUNCTION select_best_index(query, available_indexes, stats):
    // 1. _id equality - highest priority
    IF query.has_equality("_id"):
        RETURN "_id_"
    END IF

    candidates = []

    // 2. Score each index
    FOR index IN available_indexes:
        score = calculate_index_score(query, index, stats)

        IF score > 0:
            candidates.append((index.name, score))
        END IF
    END FOR

    // 3. No good index
    IF candidates.empty():
        RETURN NULL  // Full scan
    END IF

    // 4. Best index
    candidates.sort_by_score_desc()
    RETURN candidates[0].name
END FUNCTION

FUNCTION calculate_index_score(query, index, stats):
    field = index.field

    // Unique index equality
    IF index.unique AND query.has_equality(field):
        RETURN 1000
    END IF

    // Regular equality
    IF query.has_equality(field):
        RETURN 500
    END IF

    // Range query
    IF query.has_range(field):
        selectivity = estimate_selectivity(query, field, stats)

        // Only if selective
        IF selectivity < 0.3:
            RETURN 100 / selectivity
        ELSE:
            RETURN 0  // Full scan better
        END IF
    END IF

    RETURN 0
END FUNCTION
```

**Komplexitás:**
- **Idő:** O(i) - i indexek száma
- **Space:** O(i)

---

### 11. APPLY UPDATE SPEC

**Fájl:** `update.rs`
**Részletek:** `IMPLEMENTATION_UPDATE.md`

```
FUNCTION apply_update_spec(document, update_spec):
    result = deep_copy(document)

    // 1. $rename
    FOR old_field, new_field IN update_spec.rename:
        IF old_field IN result:
            result[new_field] = result[old_field]
            DELETE result[old_field]
        END IF
    END FOR

    // 2. $unset
    FOR field IN update_spec.unset:
        DELETE result[field]
    END FOR

    // 3. $set
    FOR field, value IN update_spec.set:
        result[field] = value
    END FOR

    // 4. $inc
    FOR field, amount IN update_spec.inc:
        current = result.get(field, 0)
        result[field] = current + amount
    END FOR

    // 5. $push
    FOR field, value IN update_spec.push:
        array = result.get(field, [])
        array.append(value)
        result[field] = array
    END FOR

    // 6. $pull
    FOR field, condition IN update_spec.pull:
        array = result.get(field, [])
        array = array.filter(item => NOT matches(item, condition))
        result[field] = array
    END FOR

    // ... other operators

    RETURN result
END FUNCTION
```

**Komplexitás:**
- **Idő:** O(f + a) - f mezők, a tömb elemek
- **Space:** O(doc_size) - deep copy

---

## Komplexitás Összefoglaló

### CRUD Műveletek

| Művelet | Best | Average | Worst | Space | I/O |
|---------|------|---------|-------|-------|-----|
| insert_one | O(1) | O(k log n) | O(k log n) | O(d) | 1W + kW |
| find (full) | O(n) | O(n) | O(n) | O(k) | nR |
| find (index) | O(log n) | O(log n + k) | O(log n + k) | O(k) | log(n)R + kR |
| update_one | O(n) | O(n + k log n) | O(n + k log n) | O(d) | 1R + 2W + kW |
| delete_one | O(n) | O(n + k log n) | O(n + k log n) | O(1) | 1W + kW |
| delete_many | O(n) | O(n + mk log n) | O(n + mk log n) | O(m) | mW + mkW |

**Legend:**
- n = dokumentumok száma
- k = indexek száma
- d = dokumentum méret
- m = matched dokumentumok
- R = read operation
- W = write operation

---

### Index Műveletek

| Művelet | Best | Average | Worst | Space |
|---------|------|---------|-------|-------|
| btree_insert | O(log n) | O(log n) | O(log n) | O(log n) |
| btree_search | O(log n) | O(log n) | O(log n) | O(1) |
| btree_delete | O(log n) | O(log n) | O(log n) | O(log n) |
| range_scan | O(log n + k) | O(log n + k) | O(log n + k) | O(k) |

---

### Storage Műveletek

| Művelet | Best | Average | Worst | Space |
|---------|------|---------|-------|-------|
| append | O(1) | O(1) | O(1) | O(d) |
| read | O(1) | O(1) | O(1) | O(d) |
| compaction | O(n) | O(n) | O(n) | O(1) streaming |

---

## Hivatkozások

- **Update műveletek:** `IMPLEMENTATION_UPDATE.md`
- **Delete és compaction:** `IMPLEMENTATION_DELETE.md`
- **B-tree indexelés:** `IMPLEMENTATION_INDEX.md`
- **Query optimizer:** `IMPLEMENTATION_QUERY_OPTIMIZER.md`
- **Forráskód:** `*.rs` fájlok projekt gyökérben

---

**Utolsó frissítés:** 2025-11-09
**Verzió:** 0.1.0-alpha
