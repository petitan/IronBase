# Collection Module Design Document
**Project:** IronBase (MongoLite)
**Version:** 0.2.0
**Author:** Claude Code (mérnöki design)
**Date:** 2025-11-11

---

## Current State Analysis

### File: `ironbase-core/src/collection_core.rs`
- **Size:** 1,244 lines
- **Responsibilities:** CRUD, Query, Index, Transaction operations
- **Status:** ✅ Működik, jól strukturált, de nagy

### Function Categories (Current)

#### 1. CRUD Operations (8 methods, ~450 lines)
```rust
pub fn insert_one(&self, fields) -> Result<DocumentId>
pub fn update_one(&self, query, update) -> Result<(u64, u64)>
pub fn update_many(&self, query, update) -> Result<(u64, u64)>
pub fn delete_one(&self, query) -> Result<u64>
pub fn delete_many(&self, query) -> Result<u64>

// Transaction variants
pub fn insert_one_tx(&self, doc, tx) -> Result<DocumentId>
pub fn update_one_tx(&self, query, new_doc, tx) -> Result<(u64, u64)>
pub fn delete_one_tx(&self, query, tx) -> Result<u64>
```

#### 2. Query Operations (6 methods, ~350 lines)
```rust
pub fn find(&self, query) -> Result<Vec<Value>>
pub fn find_with_options(&self, query, options) -> Result<Vec<Value>>
pub fn find_one(&self, query) -> Result<Option<Value>>
pub fn count_documents(&self, query) -> Result<u64>
pub fn distinct(&self, field, query) -> Result<Vec<Value>>
pub fn explain(&self, query) -> Result<Value>
pub fn find_with_hint(&self, query, hint) -> Result<Vec<Value>>
```

#### 3. Index Operations (3 methods, ~80 lines)
```rust
pub fn create_index(&self, field, unique) -> Result<String>
pub fn drop_index(&self, index_name) -> Result<()>
pub fn list_indexes(&self) -> Vec<String>
```

#### 4. Aggregation (1 method, ~20 lines)
```rust
pub fn aggregate(&self, pipeline) -> Result<Vec<Value>>
```

#### 5. Private Helpers (7 methods, ~300 lines)
```rust
fn read_document_by_id(&self, id_str) -> Result<Option<Value>>
fn scan_documents_via_catalog(&self) -> Result<HashMap<String, Value>>
fn filter_documents(&self, docs, query) -> Result<Vec<Value>>
fn find_with_index(&self, query, plan) -> Result<Vec<Value>>
fn apply_update_operators(&self, doc, update) -> Result<bool>
fn extract_field_from_index_name(&self, name) -> String
fn create_plan_for_hint(&self, query, hint, field) -> Result<QueryPlan>
```

---

## Proposed Modular Architecture

### Directory Structure (Future Refactor)

```
ironbase-core/src/collection/
├── mod.rs              # Public API re-exports + CollectionCore struct
├── crud.rs             # Insert, Update, Delete operations
├── query.rs            # Find, Count, Distinct operations
├── index_ops.rs        # Index management
├── helpers.rs          # Private utility functions
└── transaction.rs      # Transaction-aware CRUD operations
```

### Module Breakdown

#### `mod.rs` (Public API - ~100 lines)
**Purpose:** Entry point, struct definition, re-exports

```rust
// ironbase-core/src/collection/mod.rs

mod crud;
mod query;
mod index_ops;
mod helpers;
mod transaction;

use std::sync::Arc;
use parking_lot::RwLock;
use crate::storage::StorageEngine;
use crate::index::IndexManager;

/// Pure Rust Collection - language-independent core logic
pub struct CollectionCore {
    pub name: String,
    pub storage: Arc<RwLock<StorageEngine>>,
    pub indexes: Arc<RwLock<IndexManager>>,
}

impl CollectionCore {
    pub fn new(name: String, storage: Arc<RwLock<StorageEngine>>) -> Result<Self> {
        // Constructor logic (stays here)
    }
}

// Re-export all public APIs
pub use crud::{CrudOps};
pub use query::{QueryOps};
pub use index_ops::{IndexOps};
```

#### `crud.rs` (Insert/Update/Delete - ~300 lines)
**Purpose:** Data modification operations

```rust
// ironbase-core/src/collection/crud.rs

use super::CollectionCore;
use crate::error::Result;
use crate::document::DocumentId;
use std::collections::HashMap;
use serde_json::Value;

/// CRUD operations trait
pub trait CrudOps {
    fn insert_one(&self, fields: HashMap<String, Value>) -> Result<DocumentId>;
    fn update_one(&self, query: &Value, update: &Value) -> Result<(u64, u64)>;
    fn update_many(&self, query: &Value, update: &Value) -> Result<(u64, u64)>;
    fn delete_one(&self, query: &Value) -> Result<u64>;
    fn delete_many(&self, query: &Value) -> Result<u64>;
}

impl CrudOps for CollectionCore {
    fn insert_one(&self, mut fields: HashMap<String, Value>) -> Result<DocumentId> {
        // Current insert_one implementation
    }

    fn update_one(&self, query_json: &Value, update_json: &Value) -> Result<(u64, u64)> {
        // Current update_one implementation
    }

    // ... other CRUD methods
}
```

#### `query.rs` (Find/Count/Distinct - ~400 lines)
**Purpose:** Data retrieval and query operations

```rust
// ironbase-core/src/collection/query.rs

use super::CollectionCore;
use super::helpers::QueryHelpers;
use crate::error::Result;
use crate::find_options::FindOptions;
use serde_json::Value;

/// Query operations trait
pub trait QueryOps {
    fn find(&self, query: &Value) -> Result<Vec<Value>>;
    fn find_with_options(&self, query: &Value, options: FindOptions) -> Result<Vec<Value>>;
    fn find_one(&self, query: &Value) -> Result<Option<Value>>;
    fn count_documents(&self, query: &Value) -> Result<u64>;
    fn distinct(&self, field: &str, query: &Value) -> Result<Vec<Value>>;
    fn explain(&self, query: &Value) -> Result<Value>;
    fn find_with_hint(&self, query: &Value, hint: &str) -> Result<Vec<Value>>;
}

impl QueryOps for CollectionCore {
    fn find(&self, query_json: &Value) -> Result<Vec<Value>> {
        // Current find implementation
        // Uses helpers::find_with_index, helpers::scan_documents_via_catalog
    }

    // ... other query methods
}
```

#### `index_ops.rs` (Index Management - ~100 lines)
**Purpose:** Index creation, deletion, listing

```rust
// ironbase-core/src/collection/index_ops.rs

use super::CollectionCore;
use crate::error::Result;

/// Index operations trait
pub trait IndexOps {
    fn create_index(&self, field: String, unique: bool) -> Result<String>;
    fn drop_index(&self, index_name: &str) -> Result<()>;
    fn list_indexes(&self) -> Vec<String>;
}

impl IndexOps for CollectionCore {
    fn create_index(&self, field: String, unique: bool) -> Result<String> {
        // Current create_index implementation
    }

    // ... other index methods
}
```

#### `helpers.rs` (Private Utilities - ~300 lines)
**Purpose:** Internal helper functions

```rust
// ironbase-core/src/collection/helpers.rs

use super::CollectionCore;
use crate::error::Result;
use crate::query::Query;
use crate::query_planner::QueryPlan;
use serde_json::Value;
use std::collections::HashMap;

/// Internal helper methods
pub(super) trait QueryHelpers {
    fn read_document_by_id(&self, id_str: &str) -> Result<Option<Value>>;
    fn scan_documents_via_catalog(&self) -> Result<HashMap<String, Value>>;
    fn filter_documents(&self, docs: HashMap<String, Value>, query: &Query) -> Result<Vec<Value>>;
    fn find_with_index(&self, query: Query, plan: QueryPlan) -> Result<Vec<Value>>;
}

pub(super) trait UpdateHelpers {
    fn apply_update_operators(&self, doc: &mut Document, update: &Value) -> Result<bool>;
}

impl QueryHelpers for CollectionCore {
    fn read_document_by_id(&self, id_str: &str) -> Result<Option<Value>> {
        // Current implementation
    }

    // ... other helpers
}
```

#### `transaction.rs` (TX Operations - ~150 lines)
**Purpose:** Transaction-aware CRUD operations

```rust
// ironbase-core/src/collection/transaction.rs

use super::CollectionCore;
use crate::error::Result;
use crate::document::DocumentId;
use crate::transaction::Transaction;
use std::collections::HashMap;
use serde_json::Value;

/// Transaction operations trait
pub trait TransactionOps {
    fn insert_one_tx(&self, doc: HashMap<String, Value>, tx: &mut Transaction) -> Result<DocumentId>;
    fn update_one_tx(&self, query: &Value, new_doc: Value, tx: &mut Transaction) -> Result<(u64, u64)>;
    fn delete_one_tx(&self, query: &Value, tx: &mut Transaction) -> Result<u64>;
}

impl TransactionOps for CollectionCore {
    fn insert_one_tx(&self, doc: HashMap<String, Value>, tx: &mut Transaction) -> Result<DocumentId> {
        // Current insert_one_tx implementation
    }

    // ... other TX methods
}
```

---

## Benefits of Modular Architecture

### 1. Code Organization ✅
- **Kisebb fájlok:** 100-400 sor/fájl (jelenlegi 1244 helyett)
- **Egyértelmű felelősségek:** CRUD vs Query vs Index
- **Könnyebb navigáció:** Fejlesztők gyorsabban találnak kódot

### 2. Maintainability ✅
- **Isolated changes:** Query optimalizáció nem érinti a CRUD-ot
- **Easier testing:** Trait-based testing per module
- **Better documentation:** Module-level docs

### 3. Compilation Speed ✅
- **Parallel compilation:** Rustc párhuzamosan fordítja a modulokat
- **Incremental builds:** Csak a változott modul build-elődik újra

### 4. Lock Contention Reduction 🚀
- **Specialized locking:** Query operations csak read lock, CRUD write lock
- **Fine-grained control:** Helper functions külön lock kezelése

---

## Migration Strategy

### Phase 1: Preparation (1 hour)
1. Create `collection/` directory structure
2. Copy `collection_core.rs` → `collection/mod.rs` (backup)
3. Write trait definitions in separate files

### Phase 2: Extract Helpers (2 hours)
1. Move private functions → `helpers.rs`
2. Update visibility: `pub(super)` for internal use
3. Test compilation

### Phase 3: Extract Operations (2 hours)
1. Move CRUD methods → `crud.rs` trait implementation
2. Move Query methods → `query.rs` trait implementation
3. Move Index methods → `index_ops.rs` trait implementation
4. Test compilation after each step

### Phase 4: Extract Transactions (1 hour)
1. Move TX methods → `transaction.rs`
2. Update `mod.rs` re-exports
3. Final compilation test

### Phase 5: Testing & Validation (1 hour)
1. Run full test suite: `cargo test --release`
2. Run integration tests
3. Performance validation (no regression expected)

### Phase 6: Cleanup (30 min)
1. Delete original `collection_core.rs`
2. Update lib.rs: `pub mod collection;`
3. Git commit with detailed message

**Total Estimated Time:** 6-7 hours

---

## Risks & Mitigation

### Risk 1: API Breaking Changes
**Mitigation:** Use trait re-exports in `mod.rs`, maintain backward compatibility

### Risk 2: Compilation Errors
**Mitigation:** Incremental approach, test after each module extraction

### Risk 3: Performance Regression
**Mitigation:** Trait calls are zero-cost abstractions (no runtime overhead)

### Risk 4: Test Failures
**Mitigation:** Run tests frequently, isolate issues per module

---

## Decision: Defer Implementation

**Current Decision:** Defer full modularization until:
1. Code quality issues are resolved ✅ (DONE)
2. Performance optimizations are implemented (Query Caching, Document Catalog)
3. Team bandwidth is available for 6-7 hour refactor

**Rationale:**
- Current code works well
- No immediate performance benefit
- Focus on high-ROI optimizations first (Query Caching → 10-100x improvement)

---

## Next Steps

**Immediate:**
1. ✅ Document current architecture (this file)
2. ✅ Add inline section comments to `collection_core.rs`
3. 🎯 Implement Query Caching (high priority)

**Future (Post-v0.3.0):**
1. Implement modular architecture (following this design)
2. Benchmark before/after to validate no regression
3. Update Python bindings if needed

---

## References

- Current implementation: `ironbase-core/src/collection_core.rs`
- Related modules: `storage.rs`, `index.rs`, `query.rs`, `query_planner.rs`
- Test suite: `ironbase-core/tests/integration_tests.rs`
- Python bindings: `bindings/python/src/lib.rs`

---

**Approval:** Approved by user on 2025-11-11
**Status:** Design Complete, Implementation Deferred
**Next Review:** After Query Caching implementation
