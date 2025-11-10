# ACD Transactions Implementation Specification

## Executive Summary

This document specifies the implementation of **ACD (Atomicity, Consistency, Durability)** transactions in MongoLite, **without Isolation**. This design choice significantly simplifies implementation while providing the three most critical ACID guarantees for an embedded database.

**Key Design Decisions:**
- ✅ **Atomicity**: All-or-nothing multi-operation commits via in-memory buffering
- ✅ **Consistency**: Data integrity through constraint validation and atomic index updates
- ✅ **Durability**: Crash recovery via Write-Ahead Log (WAL) with fsync
- ❌ **Isolation**: Single-writer model (write lock held during transaction)

**Estimated Effort:** 10 days, ~2,020 lines of code

**Target Use Case:** Embedded database scenarios where single-process, sequential transaction execution is acceptable.

---

## Table of Contents

1. [Current Architecture Analysis](#1-current-architecture-analysis)
2. [ACD Requirements](#2-acd-requirements)
3. [Why ACD Instead of ACID?](#3-why-acd-instead-of-acid)
4. [Implementation Approach](#4-implementation-approach)
5. [Core Components](#5-core-components)
6. [Write-Ahead Log (WAL)](#6-write-ahead-log-wal)
7. [Transaction Lifecycle](#7-transaction-lifecycle)
8. [API Design](#8-api-design)
9. [Commit Process](#9-commit-process)
10. [Crash Recovery](#10-crash-recovery)
11. [Index Consistency](#11-index-consistency)
12. [Code Examples](#12-code-examples)
13. [Implementation Roadmap](#13-implementation-roadmap)
14. [Technical Challenges](#14-technical-challenges)
15. [Testing Strategy](#15-testing-strategy)
16. [Performance Considerations](#16-performance-considerations)
17. [Future Enhancements](#17-future-enhancements)

---

## 1. Current Architecture Analysis

### 1.1 Storage Engine

**File:** `ironbase-core/src/storage/io.rs`

**Current Write Mechanism:**
- Append-only storage model
- `write_data()` always seeks to `SeekFrom::End(0)`
- Format: 4-byte length prefix + JSON data bytes
- No in-place updates (tombstone pattern for deletes/updates)

**Write Flow:**
```rust
// From collection_core.rs, insert_one() lines 53-106
1. Acquire write lock on storage: storage.write()
2. Get/increment last_id in metadata
3. Create Document with new ID
4. Update indexes BEFORE writing to storage  // ⚠️ Issue for transactions
5. Serialize document to JSON
6. Call storage.write_data() - appends to file
```

**Problem for Transactions:**
- Indexes updated before storage write
- If storage write fails, indexes are inconsistent
- No rollback mechanism currently

### 1.2 Concurrency Model

**Current Locking:**
- `Arc<RwLock<StorageEngine>>` shared across collections
- `parking_lot::RwLock` for performance
- Each operation acquires lock independently
- No cross-operation coordination

**Existing Lock Points:**
- `collection_core.rs:54` - `let mut storage = self.storage.write()`
- `collection_core.rs:72` - `let mut indexes = self.indexes.write()`

### 1.3 Durability Mechanisms

**Current Flush Behavior:**
```rust
// storage/mod.rs:159
pub fn flush(&mut self) -> Result<()> {
    self.file.sync_all()?;  // fsync
    Ok(())
}

// Automatic flush on Drop (storage/mod.rs:184-188)
impl Drop for StorageEngine {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}
```

**Gap:** No explicit fsync after individual writes - relies on OS buffering and Drop

### 1.4 Update/Delete Pattern

**Current Approach:**
```rust
// From collection_core.rs, update_one() lines 324-404
1. Scan entire file to build latest version map (docs_by_id)
2. Find matching document
3. Apply update operators
4. Write tombstone (mark old version as deleted)
5. Write new version as separate entry
```

**Observation:** Tombstone pattern is already atomic per-operation, but not across multiple operations

---

## 2. ACD Requirements

### 2.1 Atomicity (A)

**Definition:** Multiple operations execute as a single, indivisible unit.

**Requirements:**
- Either ALL operations in a transaction succeed, or NONE do
- No partial commits visible
- If any operation fails, entire transaction must roll back
- Rollback must restore system to pre-transaction state

**Example:**
```rust
// Transfer money between accounts
tx = begin_transaction();
debit(account_a, $100);  // Operation 1
credit(account_b, $100); // Operation 2
commit(tx);              // Both or neither
```

### 2.2 Consistency (C)

**Definition:** Database constraints and invariants are maintained.

**Requirements:**
- Unique indexes remain unique after commit
- Foreign key-like relationships preserved (user → profile)
- No orphaned index entries
- Metadata (last_id, collection info) stays consistent

**Example:**
```rust
// User + Profile creation
tx = begin_transaction();
user_id = insert_user({"email": "test@example.com"});
insert_profile({"user_id": user_id, ...});
commit(tx);
// If commit fails, neither user nor profile exists
```

### 2.3 Durability (D)

**Definition:** Once committed, changes survive system crashes.

**Requirements:**
- Committed transactions must persist even if power fails
- Write-Ahead Log (WAL) written before data
- WAL synced to disk (fsync) before commit returns
- Recovery mechanism to replay WAL after crash

**Example:**
```rust
tx = begin_transaction();
insert(...);
commit(tx);  // Returns success
// <CRASH HAPPENS HERE>
// After restart: transaction is recovered from WAL
```

### 2.4 NO Isolation (Why?)

**Isolation in ACID:** Concurrent transactions don't interfere with each other.

**Why We Skip It:**
- **Complexity:** Isolation requires MVCC, snapshot isolation, or strict locking
- **Use Case:** MongoLite is embedded, typically single-process
- **Simplification:** Single-writer model is acceptable for most embedded scenarios
- **Performance:** Avoiding complex concurrency control is faster

**What This Means:**
- Only one transaction can execute at a time
- Write lock held from `begin_transaction()` to `commit()`/`rollback()`
- Reads can still proceed (read lock)
- Simpler implementation, fewer bugs

---

## 3. Why ACD Instead of ACID?

### 3.1 Complexity Comparison

| Feature | ACD (This Design) | Full ACID |
|---------|-------------------|-----------|
| Atomicity | ✓ In-memory buffer | ✓ In-memory buffer |
| Consistency | ✓ Constraint validation | ✓ Constraint validation |
| Durability | ✓ WAL + fsync | ✓ WAL + fsync |
| Isolation | ✗ Single-writer | ✓ MVCC, snapshots |
| Concurrent TX | ✗ Sequential only | ✓ Parallel execution |
| Implementation | ~2,020 LOC, 10 days | ~5,000+ LOC, 30+ days |
| Complexity | Medium | High |

### 3.2 Use Case Fit

**MongoLite Typical Use Cases:**
- Desktop applications (single user)
- Mobile app backends (local storage)
- Embedded devices (IoT)
- Prototypes and MVPs
- Test databases

**Isolation Not Critical Because:**
- Single process typically
- Few concurrent writers
- Embedded context (not a server)
- Simpler = more reliable

### 3.3 Upgrade Path

**Future:** If isolation becomes necessary:
- Add MVCC (Multi-Version Concurrency Control)
- Implement snapshot isolation
- Optimize locking (row-level instead of write-lock)

**But not now.** Start simple, add later if needed.

---

## 4. Implementation Approach

### 4.1 Core Strategy

**Three-Phase Approach:**

1. **Buffer Phase:** Operations accumulate in memory (Transaction struct)
2. **Commit Phase:** Write WAL → Write storage → Update indexes → Fsync
3. **Recovery Phase:** Replay WAL on database open

**Key Insight:** Nothing hits disk until commit. Rollback = discard buffer.

### 4.2 Data Structures

```rust
pub struct Transaction {
    id: TransactionId,              // Unique ID (timestamp or UUID)
    operations: Vec<Operation>,      // Buffered operations
    index_changes: HashMap<String, Vec<IndexChange>>,  // Index updates
    metadata_changes: HashMap<String, CollectionMeta>, // last_id, etc.
    state: TransactionState,         // Active, Committed, Aborted
}

pub enum Operation {
    Insert {
        collection: String,
        doc: Document,
        doc_id: DocumentId,
    },
    Update {
        collection: String,
        doc_id: DocumentId,
        old_doc: Document,
        new_doc: Document,
    },
    Delete {
        collection: String,
        doc_id: DocumentId,
        old_doc: Document,  // For rollback
    },
}

pub enum TransactionState {
    Active,      // Accepting operations
    Committed,   // Successfully committed
    Aborted,     // Rolled back
}
```

### 4.3 Locking Strategy

**Simple Approach:**
- Acquire storage write lock at `begin_transaction()`
- Hold lock until `commit()` or `rollback()`
- No other writes can proceed during transaction
- Reads still possible (read lock)

**Why This Works:**
- Guarantees no conflicts (single writer)
- Simple to implement
- No validation needed at commit time
- Acceptable for embedded use case

**Lock Duration:**
```rust
let tx = db.begin_transaction()?;  // Acquire write lock
// ... operations buffered ...
tx.commit()?;                      // Release write lock
```

---

## 5. Core Components

### 5.1 New File: `transaction.rs`

**Location:** `ironbase-core/src/transaction.rs`

**Responsibilities:**
- Transaction lifecycle management
- Operation buffering
- Commit coordination
- Rollback handling

**Key Structures:**

```rust
pub type TransactionId = u64;

pub struct Transaction {
    pub id: TransactionId,
    operations: Vec<Operation>,
    index_changes: HashMap<String, Vec<IndexChange>>,
    metadata_changes: HashMap<String, CollectionMeta>,
    state: TransactionState,
}

impl Transaction {
    pub fn new(id: TransactionId) -> Self {
        Transaction {
            id,
            operations: Vec::new(),
            index_changes: HashMap::new(),
            metadata_changes: HashMap::new(),
            state: TransactionState::Active,
        }
    }

    pub fn add_operation(&mut self, op: Operation) -> Result<()> {
        if self.state != TransactionState::Active {
            return Err(MongoLiteError::TransactionCommitted);
        }
        self.operations.push(op);
        Ok(())
    }

    pub fn add_index_change(&mut self, index_name: String, change: IndexChange) {
        self.index_changes
            .entry(index_name)
            .or_insert_with(Vec::new)
            .push(change);
    }

    pub fn commit(
        &mut self,
        storage: &mut StorageEngine,
        indexes: &mut IndexManager,
        wal: &mut WriteAheadLog,
    ) -> Result<()> {
        // Implementation in section 9
    }

    pub fn rollback(&mut self) -> Result<()> {
        self.operations.clear();
        self.index_changes.clear();
        self.metadata_changes.clear();
        self.state = TransactionState::Aborted;
        Ok(())
    }
}
```

### 5.2 New File: `wal.rs`

**Location:** `ironbase-core/src/wal.rs`

**Responsibilities:**
- Write-Ahead Log file management
- Entry serialization/deserialization
- Crash recovery

**Structure:**

```rust
pub struct WriteAheadLog {
    file: File,
    path: PathBuf,
}

pub struct WALEntry {
    transaction_id: TransactionId,
    entry_type: WALEntryType,
    data: Vec<u8>,
    checksum: u32,
}

pub enum WALEntryType {
    Begin,      // Transaction start
    Operation,  // Insert/Update/Delete
    Commit,     // Transaction commit
    Abort,      // Transaction rollback
}

impl WriteAheadLog {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&path)?;

        Ok(WriteAheadLog {
            file,
            path: path.as_ref().to_path_buf(),
        })
    }

    pub fn append(&mut self, entry: &WALEntry) -> Result<u64> {
        let serialized = entry.serialize()?;
        let offset = self.file.seek(SeekFrom::End(0))?;
        self.file.write_all(&serialized)?;
        Ok(offset)
    }

    pub fn flush(&mut self) -> Result<()> {
        self.file.sync_all()?;  // fsync
        Ok(())
    }

    pub fn recover(&mut self) -> Result<Vec<WALEntry>> {
        // Read all entries from file
        // Group by transaction_id
        // Return only committed transactions
        // Details in section 10
    }
}
```

### 5.3 Modifications to Existing Files

**`collection_core.rs`:**
- Add transaction-aware methods:
  - `insert_one_tx(&self, fields, tx: &mut Transaction)`
  - `update_one_tx(&self, query, update, tx: &mut Transaction)`
  - `delete_one_tx(&self, query, tx: &mut Transaction)`

**`database.rs`:**
- Add transaction management:
  - `begin_transaction(&self) -> Result<Transaction>`
  - `commit_transaction(&self, tx: Transaction) -> Result<()>`
  - `rollback_transaction(&self, tx: Transaction) -> Result<()>`
- Add WAL instance: `wal: Arc<RwLock<WriteAheadLog>>`

**`storage/io.rs`:**
- Add batch write: `write_batch(&mut self, data: Vec<&[u8]>) -> Result<Vec<u64>>`
- Expose sync: `pub fn sync(&mut self) -> Result<()>`

**`error.rs`:**
```rust
#[error("Transaction already committed or aborted")]
TransactionCommitted,

#[error("Transaction aborted: {0}")]
TransactionAborted(String),

#[error("WAL corruption detected")]
WALCorruption,
```

---

## 6. Write-Ahead Log (WAL)

### 6.1 WAL Entry Format

```
┌──────────────────────────────────────────────┐
│ Transaction ID (8 bytes)                     │
├──────────────────────────────────────────────┤
│ Entry Type (1 byte)                          │
│  0x01 = Begin                                │
│  0x02 = Operation                            │
│  0x03 = Commit                               │
│  0x04 = Abort                                │
├──────────────────────────────────────────────┤
│ Data Length (4 bytes)                        │
├──────────────────────────────────────────────┤
│ Data (variable length, JSON-serialized)      │
│  - For Begin: empty                          │
│  - For Operation: serialized Operation       │
│  - For Commit: empty                         │
│  - For Abort: empty                          │
├──────────────────────────────────────────────┤
│ Checksum (4 bytes, CRC32)                    │
└──────────────────────────────────────────────┘
```

### 6.2 WAL Workflow

**Write Path:**
```
1. BEGIN entry → WAL
2. For each operation:
   - OPERATION entry → WAL
3. COMMIT entry → WAL
4. fsync(WAL)  ← Durability point
5. Apply to storage
6. Apply to indexes
7. fsync(storage)
8. Checkpoint (optional: remove committed entries)
```

**Why WAL First?**
- If crash before WAL fsync: Transaction lost (OK, never committed)
- If crash after WAL fsync, before storage write: Recovery replays WAL
- If crash after storage write: Transaction durable

### 6.3 WAL Serialization

```rust
impl WALEntry {
    pub fn serialize(&self) -> Result<Vec<u8>> {
        let mut buf = Vec::new();

        // Transaction ID (8 bytes)
        buf.extend_from_slice(&self.transaction_id.to_le_bytes());

        // Entry Type (1 byte)
        buf.push(self.entry_type as u8);

        // Data Length (4 bytes)
        let data_len = self.data.len() as u32;
        buf.extend_from_slice(&data_len.to_le_bytes());

        // Data
        buf.extend_from_slice(&self.data);

        // Checksum (CRC32 of all above)
        let checksum = crc32(&buf);
        buf.extend_from_slice(&checksum.to_le_bytes());

        Ok(buf)
    }

    pub fn deserialize(data: &[u8]) -> Result<Self> {
        // Parse fields
        // Verify checksum
        // Return entry or error if corrupt
    }
}
```

### 6.4 WAL Compaction

**Problem:** WAL grows unbounded

**Solution:** Periodic checkpointing
```rust
pub fn checkpoint(&mut self, committed_tx_ids: &[TransactionId]) -> Result<()> {
    // Read all entries
    let all_entries = self.recover()?;

    // Keep only uncommitted transactions
    let active_entries: Vec<_> = all_entries.into_iter()
        .filter(|e| !committed_tx_ids.contains(&e.transaction_id))
        .collect();

    // Rewrite WAL file
    let temp_path = self.path.with_extension("wal.tmp");
    let mut temp_wal = WriteAheadLog::open(&temp_path)?;
    for entry in active_entries {
        temp_wal.append(&entry)?;
    }
    temp_wal.flush()?;

    // Atomic rename
    std::fs::rename(&temp_path, &self.path)?;

    Ok(())
}
```

---

## 7. Transaction Lifecycle

### 7.1 State Diagram

```
       ┌─────────┐
       │  Start  │
       └────┬────┘
            │ begin_transaction()
            ▼
       ┌─────────┐
       │ Active  │◄──────┐
       └────┬────┘       │
            │            │ add_operation()
            ├────────────┘
            │
            ├────────────┬────────────┐
            │ commit()   │ rollback() │
            ▼            ▼            ▼
       ┌──────────┐ ┌──────────┐ ┌─────────┐
       │Committed │ │ Aborted  │ │  Error  │
       └──────────┘ └──────────┘ └─────────┘
```

### 7.2 Lifecycle Methods

```rust
// 1. Begin
let mut tx = db.begin_transaction()?;
// State: Active
// Lock: Write lock acquired on storage

// 2. Operations
users.insert_one_tx(doc1, &mut tx)?;
users.update_one_tx(query, update, &mut tx)?;
// State: Still Active
// Operations buffered in tx.operations

// 3. Commit
db.commit_transaction(tx)?;
// State: Committed
// Lock: Released
// Changes: Durable

// 4. Or Rollback
db.rollback_transaction(tx)?;
// State: Aborted
// Lock: Released
// Changes: Discarded
```

### 7.3 Error Handling

```rust
let mut tx = db.begin_transaction()?;

match users.insert_one_tx(doc, &mut tx) {
    Ok(_) => {
        // Continue transaction
        match users.update_one_tx(query, update, &mut tx) {
            Ok(_) => db.commit_transaction(tx)?,
            Err(e) => {
                db.rollback_transaction(tx)?;
                return Err(e);
            }
        }
    }
    Err(e) => {
        db.rollback_transaction(tx)?;
        return Err(e);
    }
}
```

---

## 8. API Design

### 8.1 Manual API (MongoDB-style)

```rust
// Explicit begin/commit/rollback
pub fn begin_transaction(&self) -> Result<Transaction> {
    let tx_id = self.next_transaction_id();
    let tx = Transaction::new(tx_id);

    // Acquire write lock (held until commit/rollback)
    // Note: Lock is managed internally by DatabaseCore

    Ok(tx)
}

pub fn commit_transaction(&self, mut tx: Transaction) -> Result<()> {
    let mut storage = self.storage.write();
    let mut indexes = self.indexes.write();
    let mut wal = self.wal.write();

    tx.commit(&mut storage, &mut indexes, &mut wal)?;

    Ok(())
    // Lock released here (storage, indexes, wal drop their locks)
}

pub fn rollback_transaction(&self, mut tx: Transaction) -> Result<()> {
    tx.rollback()?;
    Ok(())
    // Lock released
}
```

**Usage:**
```python
# Python example
tx = db.begin_transaction()
try:
    users.insert_one_tx({"name": "Alice"}, tx)
    profiles.insert_one_tx({"user": "Alice"}, tx)
    db.commit_transaction(tx)
except Exception as e:
    db.rollback_transaction(tx)
    raise
```

### 8.2 RAII API (Rust-idiomatic)

```rust
pub struct TransactionGuard<'a> {
    db: &'a DatabaseCore,
    tx: Option<Transaction>,
}

impl<'a> TransactionGuard<'a> {
    pub fn new(db: &'a DatabaseCore) -> Result<Self> {
        let tx = db.begin_transaction()?;
        Ok(TransactionGuard {
            db,
            tx: Some(tx),
        })
    }

    pub fn tx_mut(&mut self) -> &mut Transaction {
        self.tx.as_mut().unwrap()
    }

    pub fn commit(mut self) -> Result<()> {
        let tx = self.tx.take().unwrap();
        self.db.commit_transaction(tx)
    }
}

impl<'a> Drop for TransactionGuard<'a> {
    fn drop(&mut self) {
        if let Some(tx) = self.tx.take() {
            // Auto-rollback if commit not called
            let _ = self.db.rollback_transaction(tx);
        }
    }
}
```

**Usage:**
```rust
{
    let mut guard = db.transaction()?;
    users.insert_one_tx(doc, guard.tx_mut())?;
    guard.commit()?;  // Explicit commit
} // If commit not called, auto-rollback on drop
```

### 8.3 Python Context Manager

```python
# Python API (via PyO3 bindings)
with db.transaction() as tx:
    users.insert_one({"name": "Alice"}, tx)
    profiles.insert_one({"user": "Alice"}, tx)
    # Auto-commit if no exception
# Auto-rollback if exception
```

**Implementation in `bindings/python/src/lib.rs`:**
```rust
#[pymethods]
impl MongoLite {
    fn __enter__(&mut self) -> PyResult<Transaction> {
        self.begin_transaction()
    }

    fn __exit__(
        &mut self,
        exc_type: Option<&PyAny>,
        exc_value: Option<&PyAny>,
        traceback: Option<&PyAny>,
    ) -> PyResult<bool> {
        if exc_type.is_some() {
            // Exception occurred, rollback
            self.rollback_transaction()?;
        } else {
            // Success, commit
            self.commit_transaction()?;
        }
        Ok(false)  // Don't suppress exception
    }
}
```

---

## 9. Commit Process

### 9.1 Atomic Commit Steps

```rust
impl Transaction {
    pub fn commit(
        &mut self,
        storage: &mut StorageEngine,
        indexes: &mut IndexManager,
        wal: &mut WriteAheadLog,
    ) -> Result<()> {
        // 1. Validate state
        if self.state != TransactionState::Active {
            return Err(MongoLiteError::TransactionCommitted);
        }

        // 2. Write BEGIN to WAL
        wal.append(&WALEntry {
            transaction_id: self.id,
            entry_type: WALEntryType::Begin,
            data: vec![],
            checksum: 0,  // Computed in serialize()
        })?;

        // 3. Write all operations to WAL
        for op in &self.operations {
            let op_data = serde_json::to_vec(op)?;
            wal.append(&WALEntry {
                transaction_id: self.id,
                entry_type: WALEntryType::Operation,
                data: op_data,
                checksum: 0,
            })?;
        }

        // 4. Write COMMIT to WAL
        wal.append(&WALEntry {
            transaction_id: self.id,
            entry_type: WALEntryType::Commit,
            data: vec![],
            checksum: 0,
        })?;

        // 5. Fsync WAL (DURABILITY POINT)
        wal.flush()?;

        // 6. Apply to storage
        for op in &self.operations {
            match op {
                Operation::Insert { doc, .. } => {
                    let json = serde_json::to_string(doc)?;
                    storage.write_data(json.as_bytes())?;
                }
                Operation::Update { new_doc, .. } => {
                    let json = serde_json::to_string(new_doc)?;
                    storage.write_data(json.as_bytes())?;
                }
                Operation::Delete { doc_id, .. } => {
                    // Write tombstone
                    let tombstone = json!({
                        "_id": doc_id,
                        "_tombstone": true
                    });
                    storage.write_data(serde_json::to_string(&tombstone)?.as_bytes())?;
                }
            }
        }

        // 7. Apply index changes (all-or-nothing)
        self.apply_index_changes(indexes)?;

        // 8. Update metadata
        self.apply_metadata_changes(storage)?;

        // 9. Fsync storage
        storage.flush()?;

        // 10. Mark committed
        self.state = TransactionState::Committed;

        Ok(())
    }
}
```

### 9.2 Index Change Application

```rust
impl Transaction {
    fn apply_index_changes(&self, indexes: &mut IndexManager) -> Result<()> {
        let mut applied_changes = Vec::new();

        for (index_name, changes) in &self.index_changes {
            let index = indexes.get_btree_index_mut(index_name)
                .ok_or_else(|| MongoLiteError::IndexError(format!("Index not found: {}", index_name)))?;

            for change in changes {
                match change {
                    IndexChange::Insert { key, doc_id } => {
                        if let Err(e) = index.insert(key.clone(), doc_id.clone()) {
                            // Rollback all applied changes
                            self.rollback_index_changes(indexes, &applied_changes)?;
                            return Err(e);
                        }
                        applied_changes.push((index_name.clone(), change.clone()));
                    }
                    IndexChange::Delete { key, doc_id } => {
                        index.delete(key, doc_id);
                        applied_changes.push((index_name.clone(), change.clone()));
                    }
                }
            }
        }

        Ok(())
    }

    fn rollback_index_changes(
        &self,
        indexes: &mut IndexManager,
        applied: &[(String, IndexChange)],
    ) -> Result<()> {
        // Undo in reverse order
        for (index_name, change) in applied.iter().rev() {
            let index = indexes.get_btree_index_mut(index_name).unwrap();

            match change {
                IndexChange::Insert { key, doc_id } => {
                    // Undo insert = delete
                    index.delete(key, doc_id);
                }
                IndexChange::Delete { key, doc_id } => {
                    // Undo delete = insert
                    let _ = index.insert(key.clone(), doc_id.clone());
                }
            }
        }
        Ok(())
    }
}
```

---

## 10. Crash Recovery

### 10.1 Recovery Process

```rust
impl DatabaseCore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let db_path = path.as_ref();
        let wal_path = db_path.with_extension("wal");

        // 1. Open storage
        let storage = Arc::new(RwLock::new(StorageEngine::open(db_path)?));

        // 2. Open WAL
        let mut wal = WriteAheadLog::open(&wal_path)?;

        // 3. Recover from WAL
        let committed_txs = wal.recover()?;

        // 4. Replay committed transactions
        for tx_entries in committed_txs {
            Self::replay_transaction(&storage, tx_entries)?;
        }

        // 5. Clear WAL (all committed txs applied)
        wal.clear()?;

        // 6. Return database
        Ok(DatabaseCore {
            storage,
            wal: Arc::new(RwLock::new(wal)),
            // ... other fields
        })
    }
}
```

### 10.2 WAL Recovery Logic

```rust
impl WriteAheadLog {
    pub fn recover(&mut self) -> Result<Vec<Vec<WALEntry>>> {
        self.file.seek(SeekFrom::Start(0))?;

        let mut entries = Vec::new();

        // Read all entries
        loop {
            match WALEntry::deserialize_from_file(&mut self.file) {
                Ok(entry) => entries.push(entry),
                Err(MongoLiteError::Io(_)) => break,  // EOF
                Err(e) => return Err(e),  // Corruption
            }
        }

        // Group by transaction ID
        let mut txs: HashMap<TransactionId, Vec<WALEntry>> = HashMap::new();
        for entry in entries {
            txs.entry(entry.transaction_id)
                .or_insert_with(Vec::new)
                .push(entry);
        }

        // Filter to committed transactions only
        let mut committed = Vec::new();
        for (tx_id, tx_entries) in txs {
            // Check if last entry is COMMIT
            if tx_entries.last().map(|e| e.entry_type) == Some(WALEntryType::Commit) {
                committed.push(tx_entries);
            }
            // Else: uncommitted transaction, discard
        }

        Ok(committed)
    }
}
```

### 10.3 Transaction Replay

```rust
impl DatabaseCore {
    fn replay_transaction(
        storage: &Arc<RwLock<StorageEngine>>,
        entries: Vec<WALEntry>,
    ) -> Result<()> {
        let mut storage = storage.write();

        for entry in entries {
            match entry.entry_type {
                WALEntryType::Begin | WALEntryType::Commit | WALEntryType::Abort => {
                    // Metadata entries, skip
                }
                WALEntryType::Operation => {
                    let op: Operation = serde_json::from_slice(&entry.data)?;

                    match op {
                        Operation::Insert { doc, .. } => {
                            let json = serde_json::to_string(&doc)?;
                            storage.write_data(json.as_bytes())?;
                        }
                        Operation::Update { new_doc, .. } => {
                            let json = serde_json::to_string(&new_doc)?;
                            storage.write_data(json.as_bytes())?;
                        }
                        Operation::Delete { doc_id, .. } => {
                            let tombstone = json!({
                                "_id": doc_id,
                                "_tombstone": true
                            });
                            storage.write_data(serde_json::to_string(&tombstone)?.as_bytes())?;
                        }
                    }
                }
            }
        }

        storage.flush()?;
        Ok(())
    }
}
```

---

## 11. Index Consistency

### 11.1 Problem Statement

**Current Issue:**
```rust
// collection_core.rs lines 71-98
// Indexes updated BEFORE storage write
let mut indexes = self.indexes.write();
id_index.insert(id_key, doc_id.clone())?;
// ... later ...
storage.write_data(doc_json.as_bytes())?;  // Could fail!
```

If storage write fails, index is left with orphaned entry.

### 11.2 Solution: Deferred Index Updates

```rust
pub struct IndexChange {
    pub operation: IndexOperation,
    pub key: IndexKey,
    pub doc_id: DocumentId,
}

pub enum IndexOperation {
    Insert,
    Delete,
}

impl Transaction {
    pub fn buffer_index_change(&mut self, index_name: String, change: IndexChange) {
        self.index_changes
            .entry(index_name)
            .or_insert_with(Vec::new)
            .push(change);
    }
}
```

**Workflow:**
1. Operation added to transaction buffer
2. Index change buffered (not applied yet)
3. On commit:
   - Write to storage first
   - Then apply all index changes
   - If index update fails, rollback storage? (Complex)

**Alternative:** Accept index inconsistency, rebuild on next open (simpler)

### 11.3 Index Rebuild (Fallback)

```rust
impl CollectionCore {
    pub fn rebuild_indexes(&self) -> Result<()> {
        let mut indexes = self.indexes.write();
        indexes.clear();

        // Scan all documents
        let docs = self.find(&json!({}))?;

        for doc in docs {
            let doc_id = DocumentId::from_json(doc.get("_id").unwrap())?;

            // Re-index all fields
            for index_name in indexes.list_indexes() {
                let index = indexes.get_btree_index_mut(&index_name).unwrap();
                let field = &index.metadata.field;

                if let Some(value) = doc.get(field) {
                    let key = IndexKey::from(value);
                    index.insert(key, doc_id.clone())?;
                }
            }
        }

        Ok(())
    }
}
```

---

## 12. Code Examples

### 12.1 Bank Transfer (Classic)

```rust
fn transfer_money(
    db: &DatabaseCore,
    from: &str,
    to: &str,
    amount: i64,
) -> Result<()> {
    let accounts = db.collection("accounts")?;

    // Manual API
    let mut tx = db.begin_transaction()?;

    // Debit source
    accounts.update_one_tx(
        &json!({"account_id": from}),
        &json!({"$inc": {"balance": -amount}}),
        &mut tx,
    )?;

    // Credit destination
    accounts.update_one_tx(
        &json!({"account_id": to}),
        &json!({"$inc": {"balance": amount}}),
        &mut tx,
    )?;

    // Commit atomically
    db.commit_transaction(tx)?;

    Ok(())
}
```

**Failure Scenarios:**
- If debit succeeds but credit fails: rollback (money not lost)
- If commit fails: rollback (money not lost)
- If crash after WAL commit but before storage write: recovery replays transaction

### 12.2 Multi-Collection Creation

```rust
fn create_user_with_profile(
    db: &DatabaseCore,
    name: &str,
    email: &str,
    bio: &str,
) -> Result<DocumentId> {
    let users = db.collection("users")?;
    let profiles = db.collection("profiles")?;

    // RAII API
    let mut guard = db.transaction()?;

    // Insert user
    let user_id = users.insert_one_tx(
        hashmap!{
            "name" => Value::String(name.to_string()),
            "email" => Value::String(email.to_string()),
        },
        guard.tx_mut(),
    )?;

    // Insert profile with reference
    profiles.insert_one_tx(
        hashmap!{
            "user_id" => Value::from(user_id.clone()),
            "bio" => Value::String(bio.to_string()),
        },
        guard.tx_mut(),
    )?;

    guard.commit()?;

    Ok(user_id)
}
// If commit not called, auto-rollback on drop
```

### 12.3 Python Example

```python
import ironbase

db = ironbase.MongoLite("app.db")
users = db.collection("users")
orders = db.collection("orders")

# Context manager API
with db.transaction() as tx:
    # Create user
    user_id = users.insert_one({
        "name": "Alice",
        "email": "alice@example.com"
    }, tx)

    # Create initial order
    orders.insert_one({
        "user_id": user_id,
        "items": ["Widget"],
        "total": 29.99
    }, tx)

    # Auto-commit if no exception
# Auto-rollback on exception
```

---

## 13. Implementation Roadmap

### Phase 1: Core Infrastructure (Days 1-2)

**Goals:**
- Transaction and WAL basic structures
- Error types
- No actual functionality yet, just scaffolding

**Tasks:**
- [ ] Create `ironbase-core/src/transaction.rs`
  - Transaction struct
  - TransactionState enum
  - Operation enum
  - Basic add_operation()
- [ ] Create `ironbase-core/src/wal.rs`
  - WriteAheadLog struct
  - WALEntry struct
  - Serialization/deserialization
- [ ] Update `ironbase-core/src/error.rs`
  - TransactionCommitted
  - TransactionAborted
  - WALCorruption
- [ ] Update `ironbase-core/src/lib.rs`
  - Export new modules
- [ ] Write unit tests for serialization

**Deliverable:** Compiles, basic structure in place

### Phase 2: Storage Integration (Days 3-4)

**Goals:**
- WAL can write and read entries
- Storage can batch-write

**Tasks:**
- [ ] Implement `wal.rs` methods:
  - append()
  - flush()
  - recover()
- [ ] Modify `storage/io.rs`:
  - Add write_batch()
  - Add explicit sync()
- [ ] Write tests:
  - WAL write/read roundtrip
  - WAL recovery
  - Batch write

**Deliverable:** WAL works, can write and recover entries

### Phase 3: Transaction Operations (Days 5-6)

**Goals:**
- Collections can buffer operations in transactions
- Transaction commit logic (without WAL yet)

**Tasks:**
- [ ] Modify `collection_core.rs`:
  - Add insert_one_tx()
  - Add update_one_tx()
  - Add delete_one_tx()
  - Buffer operations instead of immediate write
- [ ] Implement Transaction::commit() (basic version)
  - Apply buffered operations to storage
  - Don't worry about WAL or indexes yet
- [ ] Write tests:
  - Insert in transaction
  - Update in transaction
  - Rollback

**Deliverable:** Transactions work in-memory (not durable yet)

### Phase 4: Database API (Day 7)

**Goals:**
- DatabaseCore exposes transaction methods
- RAII wrapper

**Tasks:**
- [ ] Modify `database.rs`:
  - Add WAL instance
  - begin_transaction()
  - commit_transaction()
  - rollback_transaction()
- [ ] Create TransactionGuard
  - RAII with auto-rollback on drop
- [ ] Write tests:
  - Manual API usage
  - RAII API usage
  - Multi-collection transactions

**Deliverable:** Database-level transaction API works

### Phase 5: Durability & Recovery (Days 8-9)

**Goals:**
- WAL integration in commit
- Crash recovery on open

**Tasks:**
- [ ] Integrate WAL in Transaction::commit():
  - Write BEGIN
  - Write operations
  - Write COMMIT
  - Fsync WAL
  - Apply to storage
  - Fsync storage
- [ ] Implement DatabaseCore recovery:
  - Open WAL
  - Recover committed transactions
  - Replay to storage
  - Clear WAL
- [ ] Write tests:
  - Commit with WAL
  - Simulated crash recovery
  - Corrupted WAL handling

**Deliverable:** Durable transactions with crash recovery

### Phase 6: Index Consistency (Day 9)

**Goals:**
- Index updates atomic with storage writes

**Tasks:**
- [ ] Implement index change buffering
- [ ] Apply index changes in commit
- [ ] Rollback index changes on failure
- [ ] Write tests:
  - Index consistency after commit
  - Index rollback on failure

**Deliverable:** Indexes consistent with transactions

### Phase 7: Python Bindings (Day 10)

**Goals:**
- Python API for transactions

**Tasks:**
- [ ] Modify `bindings/python/src/lib.rs`:
  - begin_transaction()
  - commit_transaction()
  - rollback_transaction()
  - insert_one_tx(), update_one_tx(), delete_one_tx()
  - Context manager support
- [ ] Build Python bindings
- [ ] Write Python tests:
  - Manual API
  - Context manager API
  - Error cases

**Deliverable:** Python transactions work

### Phase 8: Testing & Documentation (Days 10-11)

**Goals:**
- Comprehensive test coverage
- Documentation complete

**Tasks:**
- [ ] Write integration tests:
  - Multi-collection scenarios
  - Concurrent readers during transaction
  - Large transactions
  - WAL corruption recovery
- [ ] Write property-based tests (proptest):
  - Transaction invariants
  - Crash recovery correctness
- [ ] Update documentation:
  - This file (IMPLEMENTATION_ACD.md)
  - README.md examples
  - Python docstrings
- [ ] Performance benchmarks:
  - Transaction overhead vs non-transactional
  - WAL write performance

**Deliverable:** Fully tested and documented

---

## 14. Technical Challenges

### 14.1 Challenge: Long Transactions Block Writes

**Problem:** Write lock held for entire transaction duration

**Impact:**
- Other write operations wait
- Long-running transactions degrade throughput

**Mitigation:**
- Document best practices (keep transactions short)
- Consider timeout mechanism
- Future: Optimistic locking (validate at commit)

### 14.2 Challenge: Memory Usage for Large Transactions

**Problem:** All operations buffered in memory

**Impact:**
- Large transactions (1000s of operations) consume RAM
- Risk of OOM

**Mitigation:**
- Limit transaction size (e.g., max 1000 operations)
- Return error if limit exceeded
- Future: Spill to temporary file if too large

### 14.3 Challenge: WAL File Growth

**Problem:** WAL grows unbounded

**Impact:**
- Disk space exhaustion
- Recovery slowdown (more entries to replay)

**Mitigation:**
- Checkpoint after N commits (remove committed transactions)
- Configurable checkpoint interval
- Monitor WAL size

### 14.4 Challenge: Fsync Performance

**Problem:** Fsync is slow (10-100ms depending on disk)

**Impact:**
- Commit latency high
- Throughput limited by fsync

**Mitigation:**
- Batch commits if possible (group multiple transactions)
- Use SSDs for better fsync performance
- Make fsync optional with warning (for testing only)
- Future: Group commit (multiple txs in one fsync)

### 14.5 Challenge: Index Rollback Complexity

**Problem:** If storage write succeeds but index update fails, how to rollback?

**Options:**
1. **Rollback storage writes** (hard: append-only, can't undo)
2. **Accept inconsistency, rebuild indexes** (simpler)
3. **Two-phase commit** (complex)

**Chosen Solution:** Option 2 - rebuild indexes if corruption detected

---

## 15. Testing Strategy

### 15.1 Unit Tests

**Transaction Logic:**
- [ ] add_operation() succeeds when Active
- [ ] add_operation() fails when Committed/Aborted
- [ ] rollback() clears operations
- [ ] State transitions correct

**WAL:**
- [ ] Serialize/deserialize roundtrip
- [ ] Checksum validation
- [ ] Corrupted entry detection
- [ ] Recovery filters to committed transactions only

**Storage:**
- [ ] write_batch() writes all entries
- [ ] sync() fsyncs file

### 15.2 Integration Tests

**Single Collection:**
- [ ] Insert in transaction, commit, verify
- [ ] Insert in transaction, rollback, verify not present
- [ ] Update in transaction
- [ ] Delete in transaction

**Multi-Collection:**
- [ ] Insert into multiple collections, commit
- [ ] Insert into multiple collections, rollback
- [ ] User + Profile creation scenario

**Crash Recovery:**
- [ ] Write transaction to WAL, simulate crash, recover
- [ ] Uncommitted transaction discarded after recovery
- [ ] Multiple committed transactions recovered in order

### 15.3 Property-Based Tests (Proptest)

**Invariant: Atomicity**
```rust
proptest! {
    #[test]
    fn prop_transaction_atomic(ops in vec(operation(), 1..100)) {
        let db = setup_test_db();
        let mut tx = db.begin_transaction().unwrap();

        for op in ops {
            add_operation_to_tx(&mut tx, op);
        }

        // Either all operations visible or none
        match db.commit_transaction(tx) {
            Ok(_) => {
                // All operations should be present
                assert_all_ops_present(&db);
            }
            Err(_) => {
                // No operations should be present
                assert_no_ops_present(&db);
            }
        }
    }
}
```

**Invariant: Durability**
```rust
proptest! {
    #[test]
    fn prop_transaction_durable(ops in vec(operation(), 1..100)) {
        let db_path = temp_db_path();

        {
            let db = DatabaseCore::open(&db_path).unwrap();
            let mut tx = db.begin_transaction().unwrap();

            for op in ops.clone() {
                add_operation_to_tx(&mut tx, op);
            }

            db.commit_transaction(tx).unwrap();
        }
        // Database closed (simulated crash)

        // Reopen and verify
        let db = DatabaseCore::open(&db_path).unwrap();
        assert_all_ops_present(&db, &ops);
    }
}
```

### 15.4 Performance Tests

**Benchmarks:**
- [ ] Transaction overhead (tx vs non-tx insert)
- [ ] WAL write latency
- [ ] Commit latency (with/without fsync)
- [ ] Recovery time (various WAL sizes)
- [ ] Throughput (transactions per second)

---

## 16. Performance Considerations

### 16.1 Expected Performance

**Baseline (non-transactional):**
- Insert: 1.26M ops/sec
- Update: ~500K ops/sec

**With Transactions:**
- Insert in TX: ~800K ops/sec (overhead: buffering, state management)
- Commit (no fsync): ~100K commits/sec
- Commit (with fsync): ~100 commits/sec (fsync bottleneck)

**Recovery:**
- WAL replay: ~1M ops/sec (reading WAL, applying to storage)
- For 1000 committed transactions: ~1 second recovery time

### 16.2 Optimization Opportunities

**Group Commit:**
```rust
// Instead of:
commit(tx1); // fsync
commit(tx2); // fsync
commit(tx3); // fsync

// Do:
write_wal(tx1, tx2, tx3);
fsync();  // One fsync for all
```

**Async Fsync (Dangerous!):**
```rust
// Don't wait for fsync, return immediately
// Risk: If crash before fsync completes, transaction lost
// Only for non-critical data
```

**Batch Operations:**
```rust
// Instead of 1000 single-op transactions:
tx = begin();
insert(doc1);
commit();
// Repeat 1000 times

// Do one transaction with 1000 ops:
tx = begin();
for i in 1..1000 {
    insert(doc_i);
}
commit();
```

### 16.3 Tuning Parameters

**Configuration:**
```rust
pub struct TransactionConfig {
    pub max_operations: usize,      // Default: 1000
    pub wal_checkpoint_interval: usize,  // Commits before checkpoint
    pub fsync_on_commit: bool,       // Default: true
    pub transaction_timeout: Duration,  // Default: 30s
}
```

---

## 17. Future Enhancements

### 17.1 Add Isolation (Upgrade to ACID)

**When:** If concurrent transactions become necessary

**Approach:**
- Implement MVCC (Multi-Version Concurrency Control)
- Add snapshot isolation
- Read committed / repeatable read isolation levels

**Complexity:** High (~3,000 additional LOC)

### 17.2 Savepoints

**Feature:** Partial rollback within transaction

```rust
tx = begin();
insert(doc1);
savepoint("sp1");
insert(doc2);
rollback_to("sp1");  // Discard doc2, keep doc1
commit();  // Only doc1 committed
```

**Complexity:** Medium (~500 LOC)

### 17.3 Nested Transactions

**Feature:** Transactions within transactions

```rust
tx1 = begin();
insert(doc1);
    tx2 = begin_nested(tx1);  // Subtransaction
    insert(doc2);
    commit(tx2);  // Subtransaction committed
insert(doc3);
commit(tx1);  // Entire transaction committed
```

**Complexity:** High (~1,000 LOC)

### 17.4 Read-Only Transactions

**Feature:** Transactions that only read (no locking needed)

```rust
tx = begin_readonly();
docs = find_in_tx(tx, query);
// No commit needed, no write lock acquired
```

**Complexity:** Low (~200 LOC)

---

## Appendix A: File Structure

```
ironbase-core/src/
├── lib.rs                    # MODIFY: Export transaction, wal
├── transaction.rs            # NEW: Transaction logic
├── wal.rs                    # NEW: Write-Ahead Log
├── error.rs                  # MODIFY: Add transaction errors
├── database.rs               # MODIFY: Add transaction methods
├── collection_core.rs        # MODIFY: Add *_tx() methods
└── storage/
    └── io.rs                 # MODIFY: Add write_batch(), sync()

bindings/python/src/
└── lib.rs                    # MODIFY: Python transaction API

tests/
├── transaction_tests.rs      # NEW: Transaction unit tests
├── wal_tests.rs              # NEW: WAL tests
├── recovery_tests.rs         # NEW: Crash recovery tests
└── transaction_integration.rs # NEW: Integration tests
```

---

## Appendix B: Glossary

- **ACD**: Atomicity, Consistency, Durability (ACID without Isolation)
- **Atomicity**: All-or-nothing guarantee for transactions
- **Consistency**: Database constraints maintained
- **Durability**: Committed data survives crashes
- **WAL**: Write-Ahead Log (durability mechanism)
- **Fsync**: Force synchronous write to disk (durability guarantee)
- **Tombstone**: Marker for deleted documents (append-only pattern)
- **MVCC**: Multi-Version Concurrency Control (not implemented)
- **Checkpoint**: WAL compaction (removing committed transactions)

---

## Appendix C: References

**SQLite WAL:**
- https://www.sqlite.org/wal.html
- Inspiration for WAL design

**PostgreSQL MVCC:**
- https://www.postgresql.org/docs/current/mvcc.html
- Reference for future Isolation implementation

**MongoDB Transactions:**
- https://www.mongodb.com/docs/manual/core/transactions/
- API design inspiration

---

**Document Version:** 1.0
**Last Updated:** 2025-11-09
**Status:** Specification (Not Yet Implemented)
