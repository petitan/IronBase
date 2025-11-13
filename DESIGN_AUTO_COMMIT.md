# Auto-Commit Mode Design

## Problem Statement

Currently, IronBase is **unsafe by default**:
- Normal operations (`insert_one`, `update_one`, etc.) do NOT use WAL
- Data may be lost on power failure if metadata not flushed
- Only explicit transactions provide durability guarantees

**This is contrary to SQL database behavior**, where every statement is implicitly a transaction.

## Goal

Make IronBase **safe by default** like SQL databases:
- Every operation wrapped in auto-transaction
- WAL protects all operations
- Explicit opt-in for unsafe (fast) mode

## Architecture

### Durability Modes

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DurabilityMode {
    /// Safe mode: Every operation is auto-committed (like SQL)
    /// - WAL written for every operation
    /// - fsync after every commit
    /// - Slow but guaranteed durability
    Safe,

    /// Batch mode: Operations batched, periodic auto-commit
    /// - WAL written every N operations
    /// - Bounded data loss (max N operations)
    /// - Good balance of safety and performance
    Batch { batch_size: usize },

    /// Unsafe mode: No auto-commit, manual checkpoint required
    /// - No WAL for normal operations
    /// - Fast but data loss on crash
    /// - User must explicitly call checkpoint()
    Unsafe,
}
```

### DatabaseCore Changes

```rust
pub struct DatabaseCore<S: Storage + RawStorage> {
    storage: Arc<RwLock<S>>,
    db_path: String,
    active_transactions: Arc<RwLock<HashMap<TransactionId, Transaction>>>,
    next_transaction_id: Arc<RwLock<TransactionId>>,

    // NEW: Durability mode
    durability_mode: DurabilityMode,

    // NEW: Batch buffer (for Batch mode)
    batch_buffer: Arc<RwLock<Vec<Operation>>>,
}
```

### CollectionCore insert_one() Flow

#### Mode 1: Safe (auto-commit every operation)

```rust
pub fn insert_one(&self, fields: HashMap<String, Value>) -> Result<DocumentId> {
    match self.db.durability_mode {
        DurabilityMode::Safe => {
            // 1. Auto-BEGIN
            let auto_tx = self.db.begin_auto_transaction()?;

            // 2. Prepare operation
            let op = Operation::Insert {
                collection: self.name.clone(),
                doc: fields.clone()
            };

            // 3. Execute operation (write to DB)
            let doc_id = self.insert_one_impl(fields)?;

            // 4. Add operation to auto-transaction
            auto_tx.add_operation(op);

            // 5. Auto-COMMIT (WAL write + fsync)
            self.db.commit_auto_transaction(auto_tx)?;

            Ok(doc_id)
        }

        DurabilityMode::Batch { batch_size } => {
            // Add to batch buffer, flush when full
            self.insert_one_batched(fields, batch_size)
        }

        DurabilityMode::Unsafe => {
            // Fast path (current implementation)
            self.insert_one_impl(fields)
        }
    }
}
```

#### Mode 2: Batch (auto-commit every N operations)

```rust
fn insert_one_batched(&self, fields: HashMap<String, Value>, batch_size: usize) -> Result<DocumentId> {
    let mut batch = self.db.batch_buffer.write();

    // 1. Add to batch
    let op = Operation::Insert {
        collection: self.name.clone(),
        doc: fields.clone()
    };
    batch.push(op);

    // 2. Execute operation immediately (eager write)
    let doc_id = self.insert_one_impl(fields)?;

    // 3. Flush batch if full
    if batch.len() >= batch_size {
        self.db.flush_batch(&batch)?;
        batch.clear();
    }

    Ok(doc_id)
}
```

### Auto-Transaction Implementation

```rust
impl DatabaseCore {
    /// Begin an auto-transaction (internal use only)
    fn begin_auto_transaction(&self) -> Result<Transaction> {
        let mut next_id = self.next_transaction_id.write();
        let tx_id = *next_id;
        *next_id += 1;

        Ok(Transaction::new_auto(tx_id))
    }

    /// Commit auto-transaction with WAL
    fn commit_auto_transaction(&self, mut transaction: Transaction) -> Result<()> {
        let mut storage = self.storage.write();

        // 1. Write to WAL (BEGIN + OPERATIONS + COMMIT)
        storage.commit_transaction(&mut transaction)?;

        // 2. WAL is automatically flushed in commit_transaction()

        // 3. Metadata flush
        storage.flush()?;  // This also clears WAL

        Ok(())
    }

    /// Flush batch operations
    fn flush_batch(&self, operations: &[Operation]) -> Result<()> {
        let auto_tx = self.begin_auto_transaction()?;

        // Add all operations to transaction
        let mut tx = auto_tx;
        for op in operations {
            tx.add_operation(op.clone());
        }

        // Commit (WAL + fsync)
        self.commit_auto_transaction(tx)?;

        Ok(())
    }
}
```

## WAL Format (unchanged)

```
[BEGIN marker (tx_id)]
[OPERATION 1 (tx_id, insert data)]
[OPERATION 2 (tx_id, update data)]
...
[COMMIT marker (tx_id)]
```

## Python API

### Default: Safe mode

```python
# Safe mode (default)
db = IronBase("db.mlite")  # durability="safe"
col = db.collection("users")

col.insert_one({"name": "Alice"})  # ✅ Safe (auto-commit + WAL)
# Performance: ~1,000-5,000 inserts/sec
```

### Batch mode

```python
# Batch mode (good balance)
db = IronBase("db.mlite", durability="batch", batch_size=100)
col = db.collection("users")

for i in range(1000):
    col.insert_one({"value": i})
    # Auto-commit every 100 operations

# Manual flush
db.checkpoint()  # Flush remaining batch

# Performance: ~20,000-50,000 inserts/sec
```

### Unsafe mode (opt-in)

```python
# Unsafe mode (fast, manual checkpoint required)
db = IronBase("db.mlite", durability="unsafe")
col = db.collection("users")

for i in range(10000):
    col.insert_one({"value": i})

# MUST call checkpoint manually!
db.checkpoint()  # Flush metadata

# Performance: ~50,000-100,000 inserts/sec
```

## Performance Comparison

| Mode | Throughput | Data Loss Risk | Use Case |
|------|-----------|----------------|----------|
| **Safe** | ~1-5K/sec | ✅ NONE | Critical data, ACID compliance |
| **Batch (100)** | ~20-50K/sec | ⚠️ Max 100 ops | General purpose, good balance |
| **Batch (1000)** | ~40-80K/sec | ⚠️ Max 1000 ops | High throughput with bounded risk |
| **Unsafe** | ~50-100K/sec | ❌ HIGH | Analytics, temporary data, testing |

## Migration Path

### Phase 1: Add durability mode (non-breaking)

```rust
// Default: Safe (breaking change for performance, but correct behavior)
pub fn open(path: &str) -> Result<DatabaseCore> {
    Self::open_with_durability(path, DurabilityMode::Safe)
}

// Explicit durability control
pub fn open_with_durability(path: &str, mode: DurabilityMode) -> Result<DatabaseCore> {
    // ...
}
```

### Phase 2: Python bindings

```python
# Python API
IronBase(path, durability="safe")      # Default
IronBase(path, durability="batch", batch_size=100)
IronBase(path, durability="unsafe")
```

### Phase 3: Documentation

Update README with:
- Durability guarantees
- Performance trade-offs
- Migration guide for existing code

## Testing Strategy

1. **Power failure tests** for each mode
2. **Performance benchmarks** (safe vs batch vs unsafe)
3. **Crash recovery tests** (WAL replay verification)
4. **Concurrent operation tests** (multi-thread safety)

## Open Questions

1. **Background flush thread for batch mode?**
   - Option: Auto-flush every N seconds
   - Reduces bounded data loss window

2. **Per-collection durability mode?**
   - Some collections critical (users)
   - Some collections fast (logs, cache)

3. **fsync configuration?**
   - Always fsync (safe)
   - Periodic fsync (faster)
   - Let OS decide (risky)

## Implementation Checklist

- [ ] Add `DurabilityMode` enum
- [ ] Add `durability_mode` field to `DatabaseCore`
- [ ] Implement `begin_auto_transaction()`
- [ ] Implement `commit_auto_transaction()`
- [ ] Add batch buffer to `DatabaseCore`
- [ ] Implement `flush_batch()`
- [ ] Update `insert_one()` with mode switch
- [ ] Update `update_one()` with mode switch
- [ ] Update `delete_one()` with mode switch
- [ ] Add Python binding for durability parameter
- [ ] Write power failure tests
- [ ] Write performance benchmarks
- [ ] Update documentation
- [ ] Migration guide for existing users
