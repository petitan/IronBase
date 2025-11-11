# Phase 1.1: Index Atomicity Fix - Implementation Status

## Overview

**Goal:** Ensure index updates are atomic with data changes to prevent data/index inconsistency after crashes.

**Approach:** Two-phase commit protocol for index updates
- Phase 1 (PREPARE): Write changes to temp files + WAL
- Phase 2 (COMMIT): Atomic rename temp → final

**Progress:** 4/8 steps completed (infrastructure ready)

---

## ✅ COMPLETED STEPS (1-4)

### Step 1: WAL IndexChange Support ✅
**File:** `ironbase-core/src/wal.rs`
**Commit:** 5227b28

**Changes:**
- Added `WALEntryType::IndexChange = 0x05` enum variant
- Updated `from_u8()` deserializer to handle new entry type

**Purpose:** WAL can now store index change entries for durability

---

### Step 2: Two-Phase Commit Infrastructure ✅
**File:** `ironbase-core/src/index.rs`
**Commit:** 5227b28

**Changes:**
1. **IndexManager** (lines 468-493):
   - Added `index_file_paths: HashMap<String, PathBuf>` for tracking persistent index locations
   - Added `set_index_path()` and `get_index_path()` helper methods
   - Updated `drop_index()` to clean up file path tracking

2. **BPlusTree** (lines 400-456):
   - `prepare_changes(base_path) -> PathBuf`: Creates `.idx.tmp` file with current index state
   - `commit_prepared_changes(temp_path, final_path)`: Atomic rename via POSIX rename()
   - `rollback_prepared_changes(temp_path)`: Cleanup on failure

**Purpose:** Enables atomic index updates through temp files + atomic rename

---

### Step 3: WAL IndexChange Integration ✅
**File:** `ironbase-core/src/storage/mod.rs`
**Commit:** e1e4e93

**Changes:**
- Step 2.5 in `commit_transaction()` (lines 248-274):
  ```rust
  for (index_name, changes) in transaction.index_changes() {
      for change in changes {
          // Serialize to JSON: {index_name, operation, key, doc_id}
          let index_entry = WALEntry::new(tx.id, WALEntryType::IndexChange, json_bytes);
          self.wal.append(&index_entry)?;
      }
  }
  ```

- Updated Step 6 comments (lines 286-307) with comprehensive two-phase commit design

**Purpose:** Index changes are written to WAL for crash recovery

---

### Step 4: WAL Recovery for IndexChange ✅
**File:** `ironbase-core/src/storage/mod.rs`
**Commit:** fec10ff

**Changes:**
1. Added `RecoveredIndexChange` struct (lines 20-27):
   ```rust
   pub struct RecoveredIndexChange {
       pub index_name: String,
       pub operation: IndexOperation,
       pub key: IndexKey,
       pub doc_id: DocumentId,
   }
   ```

2. Modified `recover_from_wal()` return type (line 382):
   - Old: `Result<()>`
   - New: `Result<(Vec<WALEntry>, Vec<RecoveredIndexChange>)>`

3. Added IndexChange parsing in recovery loop (lines 425-452):
   - Parses JSON: `{index_name, operation, key, doc_id}`
   - Returns parsed changes for higher-level replay

**Purpose:** WAL recovery extracts index changes for Database/CollectionCore to apply

---

## 🚧 REMAINING STEPS (5-8) - TODO

### Step 5: Collection Index Tracking
**File:** `ironbase-core/src/collection_core.rs`
**Status:** ❌ NOT IMPLEMENTED

**Required Changes:**
1. Update `insert_one_tx()` (line 1109):
   ```rust
   // Current: // TODO: Track index changes (future: two-phase commit)

   // Required:
   for (index_name, index) in self.indexes.read().list_indexes() {
       if let Some(key_value) = doc.get(&index.field) {
           let key = IndexKey::from(key_value);
           tx.add_index_change(index_name, IndexChange {
               operation: IndexOperation::Insert,
               key,
               doc_id: doc_id.clone(),
           })?;
       }
   }
   ```

2. Similar updates for `update_one_tx()` and `delete_one_tx()`
   - Update: Delete old key, Insert new key
   - Delete: Delete key from index

**Effort:** 2-3 hours

---

### Step 6: Database Index Recovery
**File:** `ironbase-core/src/database.rs` or new helper
**Status:** ❌ NOT IMPLEMENTED

**Required Changes:**
1. Update `Database::open()` to call index recovery:
   ```rust
   pub fn open(path: impl AsRef<Path>) -> Result<Self> {
       // ... existing open logic ...

       // NEW: Recover indexes from WAL
       let (_, recovered_index_changes) = storage.recover_from_wal()?;

       // Apply index changes to IndexManagers
       for change in recovered_index_changes {
           let collection = self.collection(&change.collection)?;
           collection.apply_recovered_index_change(change)?;
       }

       Ok(db)
   }
   ```

2. Add `CollectionCore::apply_recovered_index_change()`:
   ```rust
   fn apply_recovered_index_change(&self, change: RecoveredIndexChange) -> Result<()> {
       let mut indexes = self.indexes.write();
       if let Some(index) = indexes.get_btree_index_mut(&change.index_name) {
           match change.operation {
               IndexOperation::Insert => index.insert(change.key, change.doc_id)?,
               IndexOperation::Delete => index.delete(&change.key, &change.doc_id)?,
           }
       }
       Ok(())
   }
   ```

**Effort:** 3-4 hours

---

### Step 7: Full Two-Phase Commit
**Files:** `ironbase-core/src/database.rs`, `collection_core.rs`
**Status:** ❌ NOT IMPLEMENTED

**Required Changes:**
1. New `Database::commit_transaction_with_indexes()`:
   ```rust
   pub fn commit_transaction_with_indexes(&self, tx_id: TransactionId) -> Result<()> {
       let mut tx = self.get_transaction(tx_id)?;

       // PHASE 1: PREPARE all indexes
       let mut prepared_temps = Vec::new();
       for (collection_name, index_changes) in group_by_collection(&tx.index_changes()) {
           let collection = self.collection(collection_name)?;

           for (index_name, changes) in group_by_index(index_changes) {
               // Apply changes to in-memory index
               let mut indexes = collection.indexes.write();
               let index = indexes.get_btree_index_mut(index_name)?;

               for change in changes {
                   match change.operation {
                       IndexOperation::Insert => index.insert(change.key, change.doc_id)?,
                       IndexOperation::Delete => index.delete(&change.key, &change.doc_id)?,
                   }
               }

               // Prepare temp file
               let base_path = get_index_path(collection_name, index_name);
               let temp_path = index.prepare_changes(&base_path)?;
               prepared_temps.push((temp_path, base_path));
           }
       }

       // PHASE 2: COMMIT (via existing storage commit)
       self.storage.write().commit_transaction(&mut tx)?;

       // PHASE 3: ATOMIC RENAME all indexes
       for (temp_path, final_path) in prepared_temps {
           BPlusTree::commit_prepared_changes(&temp_path, &final_path)?;
       }

       Ok(())
   }
   ```

2. Update all call sites to use new method instead of `commit_transaction()`

**Effort:** 6-8 hours

---

### Step 8: Tests
**File:** New test file or existing test module
**Status:** ❌ NOT IMPLEMENTED

**Required Tests:**
1. **Basic two-phase commit:**
   ```rust
   #[test]
   fn test_index_atomicity_basic() {
       let db = Database::open("test.db")?;
       let coll = db.collection("users")?;
       coll.create_index("age", unique=false)?;

       let tx = db.begin_transaction()?;
       coll.insert_one_tx({"name": "Alice", "age": 30}, &mut tx)?;
       db.commit_transaction_with_indexes(tx.id)?;

       // Verify index contains entry
       let index = coll.indexes.read().get_btree_index("age")?;
       assert!(index.search(&IndexKey::Int(30)).contains(&doc_id));
   }
   ```

2. **Crash recovery:**
   ```rust
   #[test]
   fn test_index_recovery_after_crash() {
       // Insert with index, simulate crash before WAL clear
       // Reopen database
       // Verify index recovered correctly
   }
   ```

3. **Rollback on index failure:**
   ```rust
   #[test]
   fn test_index_unique_violation_rollback() {
       // Create unique index
       // Insert duplicate → should rollback transaction
       // Verify data not persisted
   }
   ```

**Effort:** 4-6 hours

---

## Architecture Summary

```
┌─────────────────────────────────────────────────────────────┐
│ USER CODE                                                   │
│   db.begin_transaction()                                    │
│   collection.insert_one_tx(doc, &mut tx)  ← Step 5        │
│   db.commit_transaction_with_indexes(tx)  ← Step 7        │
└────────────────────────┬────────────────────────────────────┘
                         │
┌────────────────────────▼────────────────────────────────────┐
│ DATABASE LAYER                                              │
│   commit_transaction_with_indexes() {                       │
│     // Phase 1: Prepare all indexes                         │
│     for index: apply changes + prepare_changes()  ← Step 2 │
│                                                              │
│     // Phase 2: Commit data + write WAL                     │
│     storage.commit_transaction()           ← Step 3        │
│                                                              │
│     // Phase 3: Atomic commit indexes                       │
│     BPlusTree::commit_prepared_changes()   ← Step 2        │
│   }                                                          │
│                                                              │
│   open() {                                                   │
│     storage.recover_from_wal()             ← Step 4        │
│     apply_recovered_index_changes()        ← Step 6        │
│   }                                                          │
└─────────────────────────────────────────────────────────────┘
                         │
┌────────────────────────▼────────────────────────────────────┐
│ STORAGE LAYER                                               │
│   commit_transaction() {                                    │
│     wal.append(Operations)                                  │
│     wal.append(IndexChanges)               ← Step 3        │
│     wal.flush()                                             │
│     apply_operations()                                      │
│   }                                                          │
│                                                              │
│   recover_from_wal() {                                      │
│     parse Operations → apply                                │
│     parse IndexChanges → return            ← Step 4        │
│   }                                                          │
└─────────────────────────────────────────────────────────────┘
                         │
┌────────────────────────▼────────────────────────────────────┐
│ WAL LAYER                                                   │
│   WALEntryType::IndexChange                ← Step 1        │
│   append(), recover(), clear()                              │
└─────────────────────────────────────────────────────────────┘
```

---

## Next Actions

### Immediate (for full atomicity):
1. Implement Step 5 (2-3h): Track index changes in collection methods
2. Implement Step 6 (3-4h): Index recovery on database open
3. Implement Step 7 (6-8h): Full two-phase commit coordination
4. Implement Step 8 (4-6h): Comprehensive tests

**Total effort:** 15-21 hours

### Alternative (defer to Phase 2):
- Infrastructure is ready (Steps 1-4)
- Can be completed later without breaking changes
- Focus on other Phase 1 priorities first

---

## Benefits Already Achieved

Even with Steps 1-4 only:
1. ✅ WAL infrastructure supports index changes
2. ✅ BPlusTree has atomic commit capability (prepare/commit/rollback)
3. ✅ WAL recovery can parse index changes
4. ✅ Clean architecture for future completion

**Impact:** Foundation is solid. Steps 5-8 are "plumbing" to connect everything.

---

## Files Modified

1. `ironbase-core/src/wal.rs` - WAL IndexChange support
2. `ironbase-core/src/index.rs` - Two-phase commit methods
3. `ironbase-core/src/storage/mod.rs` - WAL integration + recovery

**Lines added:** ~250 lines
**Tests added:** 0 (TODO in Step 8)

---

## References

- **Design Document:** `IMPLEMENTATION_IMPROVEMENTS.md` (Phase 1.1 section)
- **Commits:**
  - Step 1: (part of Step 2 commit 5227b28)
  - Step 2: 5227b28 "Two-phase commit infrastructure"
  - Step 3: e1e4e93 "WAL IndexChange integration"
  - Step 4: fec10ff "WAL recovery for IndexChange entries"
