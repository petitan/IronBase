# Index Consistency in ACD Transactions

## Current Status

Index consistency is **partially implemented** in the ACD transaction system. This document explains the current architecture and future optimization path.

## Architecture Overview

### Separation of Concerns

```
DatabaseCore
    ├── StorageEngine (data persistence)
    │   └── commit_transaction() - Steps 1-9
    └── CollectionCore (business logic)
        └── IndexManager (index management)
```

**Key Design Decision**: IndexManager is owned by CollectionCore, not StorageEngine. This maintains clean separation:
- **StorageEngine**: Low-level data persistence, WAL, recovery
- **CollectionCore**: Business logic, CRUD operations, index management

## Current Implementation

### Step 6 in commit_transaction()

```rust
// Step 6: Apply index changes
// NOTE: Index changes are tracked in transaction.index_changes()
// The actual index updates happen at a higher level (CollectionCore)
// where the IndexManager is accessible.
```

### Transaction Tracking

The `Transaction` struct tracks index changes:

```rust
pub struct Transaction {
    index_changes: HashMap<String, Vec<IndexChange>>,
    // ...
}

pub struct IndexChange {
    pub operation: IndexOperation,  // Insert or Delete
    pub key: IndexKey,
    pub doc_id: DocumentId,
}
```

## ACD Guarantees

### Current Guarantees ✅

1. **Atomicity**: All operations in a transaction are applied together or not at all
   - Buffered in memory until commit
   - Applied atomically in commit_transaction()

2. **Consistency**: Data constraints are maintained
   - Index changes tracked alongside data operations
   - Metadata (last_id) updated atomically

3. **Durability**: Changes survive crashes
   - WAL ensures recovery after crash
   - Fsync guarantees persistence

### Index Atomicity ⚠️

**Current**: Index updates happen in CollectionCore.insert_one/update_one/delete_one
**Issue**: Not part of the atomic commit in StorageEngine

**Why This Works for MVP**:
- Single-writer model (write lock during entire transaction)
- No concurrent index updates possible
- Index updates happen immediately after data write, before transaction completion

**Future Optimization** (when adding concurrent transactions):
- Move IndexManager to StorageEngine
- Apply index changes in Step 6 of commit
- Include index updates in WAL for crash recovery

## Usage Pattern (Current)

```rust
// 1. Begin transaction
let tx_id = db.begin_transaction();

// 2. Add operations (index changes tracked automatically)
let mut tx = db.get_transaction(tx_id).unwrap();
tx.add_operation(Operation::Insert { ... }).unwrap();
tx.add_index_change("users_age".to_string(), IndexChange { ... }).unwrap();
db.update_transaction(tx_id, tx).unwrap();

// 3. Commit (applies data + metadata, index updates happen in CollectionCore)
db.commit_transaction(tx_id).unwrap();
```

## Future Implementation (Phase 7+)

### Option 1: Move IndexManager to StorageEngine

**Pros**:
- True atomic index updates in commit
- Simpler transaction API

**Cons**:
- Breaks separation of concerns
- StorageEngine becomes more complex

### Option 2: Callback-based Index Updates

```rust
pub fn commit_transaction<F>(
    &mut self,
    transaction: &mut Transaction,
    apply_indexes: F
) -> Result<()>
where F: FnOnce(&Transaction) -> Result<()>
{
    // ... Steps 1-5 ...

    // Step 6: Apply index changes via callback
    apply_indexes(transaction)?;

    // ... Steps 7-9 ...
}
```

**Pros**:
- Maintains separation of concerns
- Index updates atomic with data

**Cons**:
- More complex API
- Callback must not fail after WAL commit

### Option 3: Two-Phase Commit (Recommended)

```rust
// Phase 1: Prepare (validate, log to WAL)
let commit_handle = storage.prepare_commit(transaction)?;

// Phase 2: Apply (data + indexes atomically)
storage.apply_commit(commit_handle, |tx| {
    // Apply index changes here
    index_manager.apply_changes(tx.index_changes())?;
    Ok(())
})?;
```

**Pros**:
- Clean separation of concerns
- True atomicity for data + indexes
- Follows database transaction protocols

**Cons**:
- More complex implementation

## Testing

Index consistency is tested indirectly:
- CollectionCore tests verify index updates with data operations
- Transaction tests verify operation buffering
- Recovery tests verify data persistence

**TODO**: Add explicit index consistency tests when implementing Phase 7.

## Conclusion

The current implementation provides **practical ACD guarantees** for the single-writer model. Index updates are tracked in transactions and applied consistently, though not within the same atomic operation as data writes.

For production use with concurrent transactions, implement **Two-Phase Commit** (Option 3) to ensure true atomicity of data + index updates.
