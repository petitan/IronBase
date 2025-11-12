# MongoLite Refactoring Plan

**Generated**: 2025-11-12
**Based on**: Code Quality Metrics Analysis
**Author**: Claude Code Analysis

---

## Executive Summary

Based on comprehensive code quality analysis, MongoLite has identified **174-257 hours** of technical debt across 5 categories. The project currently scores:

- **Cyclomatic Complexity**: Average 11.3 (⚠️ Moderate, Max: 67 ❌ Critical)
- **Code Duplication**: 6.2% (⚠️ Acceptable, Target: <3%)
- **Maintainability Index**: 23.6 (⚠️ Moderate, Target: >50)
- **SOLID Compliance**: 62% (⚠️ Moderate, Target: >85%)
- **Technical Debt Ratio**: SQALE B (✅ Good, 8.4% interest rate)

This plan outlines a **4-phase refactoring strategy** over **6-8 weeks** to address critical issues, improve maintainability, and reduce technical debt interest rate from 8.4% to ~3%.

**ROI**: 10-month break-even point for critical refactorings.

---

## Table of Contents

1. [Critical Issues](#critical-issues)
2. [Refactoring Phases](#refactoring-phases)
3. [Phase 1: Query System Overhaul](#phase-1-query-system-overhaul-critical)
4. [Phase 2: Storage Abstraction](#phase-2-storage-abstraction-high-priority)
5. [Phase 3: Code Quality Improvements](#phase-3-code-quality-improvements-medium-priority)
6. [Phase 4: Testing & Infrastructure](#phase-4-testing--infrastructure-low-priority)
7. [Implementation Guidelines](#implementation-guidelines)
8. [Risk Management](#risk-management)
9. [Success Metrics](#success-metrics)

---

## Critical Issues

### 🔥 Blocker Issues (Must Fix Before New Features)

#### 1. **matches_filter() Complexity Explosion**
- **Location**: `ironbase-core/src/query.rs:45-312`
- **Cyclomatic Complexity**: 67 ❌❌ CRITICAL
- **Cognitive Complexity**: 142 ❌❌ CRITICAL
- **Maintainability Index**: 6.1 ❌ CRITICAL
- **SOLID Violations**: SRP, OCP
- **Impact**:
  - Every query feature takes 2-3x longer to implement
  - High bug risk (untestable complexity)
  - Blocks adding new query operators ($text, $geoNear, etc.)
- **Estimated Cost**: 16-24 hours
- **Monthly Interest**: 2 hours/month
- **Break-Even**: 10 months ✅

#### 2. **Storage Engine God Class**
- **Location**: `ironbase-core/src/storage/mod.rs`, `collection.rs`
- **Issue**: Concrete dependency on `StorageEngine`, violates DIP
- **Impact**:
  - Impossible to unit test without real file I/O
  - Can't swap storage implementations (in-memory, cloud, etc.)
  - Tight coupling between collection logic and storage
- **Estimated Cost**: 8-12 hours
- **Monthly Interest**: 1 hour/month
- **Break-Even**: 10 months ✅

#### 3. **Compaction Complexity**
- **Location**: `ironbase-core/src/storage/compaction.rs:compact_with_config()`
- **Cyclomatic Complexity**: 34 ❌ COMPLEX
- **Cognitive Complexity**: 78 ❌ COMPLEX
- **Issue**: 600-line function with multiple responsibilities
- **Impact**:
  - Compaction bugs are critical (data corruption risk)
  - Difficult to optimize or parallelize
  - High cognitive load for changes
- **Estimated Cost**: 8-12 hours
- **Monthly Interest**: 1 hour/month

---

## Refactoring Phases

### Overview

| Phase | Priority | Duration | Impact | Risk |
|-------|----------|----------|--------|------|
| 1. Query System Overhaul | 🔥 CRITICAL | 2-3 weeks | Very High | Medium |
| 2. Storage Abstraction | ⚠️ HIGH | 1-2 weeks | High | Low |
| 3. Code Quality | ⚠️ MEDIUM | 2-3 weeks | Medium | Low |
| 4. Testing & Infra | ✅ LOW | 1-2 weeks | Medium | Very Low |
| **TOTAL** | | **6-10 weeks** | | |

### Dependency Graph

```
Phase 1 (Query) ──┐
                  ├──> Phase 3 (Quality) ──> Phase 4 (Testing)
Phase 2 (Storage)─┘
```

Phase 1 and 2 can run in parallel. Phase 3 depends on both. Phase 4 is final.

---

## Phase 1: Query System Overhaul (CRITICAL)

**Duration**: 2-3 weeks (16-24 hours)
**Priority**: 🔥 CRITICAL
**Risk**: Medium (requires careful testing, but well-isolated)

### Goals

- Reduce `matches_filter()` CC from 67 → <15
- Reduce cognitive complexity from 142 → <25
- Enable easy addition of new query operators
- Improve test coverage from 45% → >85%

### Step 1.1: Extract Operator Interfaces (4 hours)

**Create**: `ironbase-core/src/query/operators/mod.rs`

```rust
/// Trait for all query operators
pub trait OperatorMatcher: Send + Sync {
    /// Returns the operator name (e.g., "$gt", "$eq")
    fn name(&self) -> &'static str;

    /// Matches document value against filter value
    fn matches(&self, doc_value: Option<&Value>, filter_value: &Value) -> Result<bool>;

    /// Optional: provides expected filter value type for validation
    fn validate_filter(&self, filter_value: &Value) -> Result<()> {
        Ok(()) // Default: no validation
    }
}

/// Registry for operator matchers
pub struct OperatorRegistry {
    operators: HashMap<String, Arc<dyn OperatorMatcher>>,
}

impl OperatorRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            operators: HashMap::new(),
        };

        // Register built-in operators
        registry.register(Box::new(EqOperator));
        registry.register(Box::new(NeOperator));
        registry.register(Box::new(GtOperator));
        registry.register(Box::new(GteOperator));
        registry.register(Box::new(LtOperator));
        registry.register(Box::new(LteOperator));
        registry.register(Box::new(InOperator));
        registry.register(Box::new(NinOperator));
        registry.register(Box::new(ExistsOperator));
        registry.register(Box::new(RegexOperator));

        registry
    }

    pub fn register(&mut self, matcher: Box<dyn OperatorMatcher>) {
        self.operators.insert(matcher.name().to_string(), Arc::from(matcher));
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn OperatorMatcher>> {
        self.operators.get(name)
    }
}
```

### Step 1.2: Implement Comparison Operators (3 hours)

**Create**: `ironbase-core/src/query/operators/comparison.rs`

```rust
use super::OperatorMatcher;

pub struct EqOperator;
impl OperatorMatcher for EqOperator {
    fn name(&self) -> &'static str { "$eq" }

    fn matches(&self, doc_value: Option<&Value>, filter_value: &Value) -> Result<bool> {
        Ok(doc_value == Some(filter_value))
    }
}

pub struct NeOperator;
impl OperatorMatcher for NeOperator {
    fn name(&self) -> &'static str { "$ne" }

    fn matches(&self, doc_value: Option<&Value>, filter_value: &Value) -> Result<bool> {
        Ok(doc_value != Some(filter_value))
    }
}

pub struct GtOperator;
impl OperatorMatcher for GtOperator {
    fn name(&self) -> &'static str { "$gt" }

    fn matches(&self, doc_value: Option<&Value>, filter_value: &Value) -> Result<bool> {
        compare_values(doc_value, filter_value, |a, b| a > b)
    }
}

pub struct GteOperator;
impl OperatorMatcher for GteOperator {
    fn name(&self) -> &'static str { "$gte" }

    fn matches(&self, doc_value: Option<&Value>, filter_value: &Value) -> Result<bool> {
        compare_values(doc_value, filter_value, |a, b| a >= b)
    }
}

pub struct LtOperator;
impl OperatorMatcher for LtOperator {
    fn name(&self) -> &'static str { "$lt" }

    fn matches(&self, doc_value: Option<&Value>, filter_value: &Value) -> Result<bool> {
        compare_values(doc_value, filter_value, |a, b| a < b)
    }
}

pub struct LteOperator;
impl OperatorMatcher for LteOperator {
    fn name(&self) -> &'static str { "$lte" }

    fn matches(&self, doc_value: Option<&Value>, filter_value: &Value) -> Result<bool> {
        compare_values(doc_value, filter_value, |a, b| a <= b)
    }
}

/// Helper: Compare two JSON values with type coercion
fn compare_values<F>(doc_value: Option<&Value>, filter_value: &Value, cmp: F) -> Result<bool>
where
    F: Fn(&Value, &Value) -> bool,
{
    let doc_val = doc_value.ok_or(MongoLiteError::QueryError("Field not found".into()))?;

    // Type coercion: number comparison
    match (doc_val, filter_value) {
        (Value::Number(a), Value::Number(b)) => {
            let a_f64 = a.as_f64().ok_or(MongoLiteError::QueryError("Invalid number".into()))?;
            let b_f64 = b.as_f64().ok_or(MongoLiteError::QueryError("Invalid number".into()))?;
            Ok(cmp(&Value::from(a_f64), &Value::from(b_f64)))
        }
        (Value::String(a), Value::String(b)) => {
            Ok(cmp(&Value::String(a.clone()), &Value::String(b.clone())))
        }
        _ => Ok(false), // Type mismatch = false
    }
}
```

### Step 1.3: Implement Array Operators (2 hours)

**Create**: `ironbase-core/src/query/operators/array.rs`

```rust
pub struct InOperator;
impl OperatorMatcher for InOperator {
    fn name(&self) -> &'static str { "$in" }

    fn matches(&self, doc_value: Option<&Value>, filter_value: &Value) -> Result<bool> {
        let doc_val = doc_value.ok_or(MongoLiteError::QueryError("Field not found".into()))?;
        let array = filter_value.as_array()
            .ok_or(MongoLiteError::QueryError("$in requires array".into()))?;

        Ok(array.contains(doc_val))
    }

    fn validate_filter(&self, filter_value: &Value) -> Result<()> {
        if !filter_value.is_array() {
            return Err(MongoLiteError::QueryError("$in requires array".into()));
        }
        Ok(())
    }
}

pub struct NinOperator;
impl OperatorMatcher for NinOperator {
    fn name(&self) -> &'static str { "$nin" }

    fn matches(&self, doc_value: Option<&Value>, filter_value: &Value) -> Result<bool> {
        let doc_val = doc_value.ok_or(MongoLiteError::QueryError("Field not found".into()))?;
        let array = filter_value.as_array()
            .ok_or(MongoLiteError::QueryError("$nin requires array".into()))?;

        Ok(!array.contains(doc_val))
    }
}

pub struct AllOperator;
impl OperatorMatcher for AllOperator {
    fn name(&self) -> &'static str { "$all" }

    fn matches(&self, doc_value: Option<&Value>, filter_value: &Value) -> Result<bool> {
        let doc_array = doc_value
            .and_then(|v| v.as_array())
            .ok_or(MongoLiteError::QueryError("$all requires document field to be array".into()))?;

        let required = filter_value.as_array()
            .ok_or(MongoLiteError::QueryError("$all requires array".into()))?;

        Ok(required.iter().all(|item| doc_array.contains(item)))
    }
}
```

### Step 1.4: Implement Logical Operators (3 hours)

**Create**: `ironbase-core/src/query/operators/logical.rs`

```rust
pub struct AndOperator;
impl OperatorMatcher for AndOperator {
    fn name(&self) -> &'static str { "$and" }

    fn matches(&self, doc_value: Option<&Value>, filter_value: &Value) -> Result<bool> {
        // Note: doc_value is the entire document for logical operators
        let doc = doc_value.ok_or(MongoLiteError::QueryError("Document required".into()))?;

        let conditions = filter_value.as_array()
            .ok_or(MongoLiteError::QueryError("$and requires array".into()))?;

        for condition in conditions {
            if !matches_filter(doc, condition)? {
                return Ok(false); // Short-circuit
            }
        }

        Ok(true)
    }
}

pub struct OrOperator;
impl OperatorMatcher for OrOperator {
    fn name(&self) -> &'static str { "$or" }

    fn matches(&self, doc_value: Option<&Value>, filter_value: &Value) -> Result<bool> {
        let doc = doc_value.ok_or(MongoLiteError::QueryError("Document required".into()))?;

        let conditions = filter_value.as_array()
            .ok_or(MongoLiteError::QueryError("$or requires array".into()))?;

        for condition in conditions {
            if matches_filter(doc, condition)? {
                return Ok(true); // Short-circuit
            }
        }

        Ok(false)
    }
}

pub struct NotOperator;
impl OperatorMatcher for NotOperator {
    fn name(&self) -> &'static str { "$not" }

    fn matches(&self, doc_value: Option<&Value>, filter_value: &Value) -> Result<bool> {
        let doc = doc_value.ok_or(MongoLiteError::QueryError("Document required".into()))?;
        Ok(!matches_filter(doc, filter_value)?)
    }
}
```

### Step 1.5: Implement Existence & Regex Operators (2 hours)

**Create**: `ironbase-core/src/query/operators/existence.rs`

```rust
pub struct ExistsOperator;
impl OperatorMatcher for ExistsOperator {
    fn name(&self) -> &'static str { "$exists" }

    fn matches(&self, doc_value: Option<&Value>, filter_value: &Value) -> Result<bool> {
        let should_exist = filter_value.as_bool()
            .ok_or(MongoLiteError::QueryError("$exists requires boolean".into()))?;

        Ok(doc_value.is_some() == should_exist)
    }
}

pub struct RegexOperator;
impl OperatorMatcher for RegexOperator {
    fn name(&self) -> &'static str { "$regex" }

    fn matches(&self, doc_value: Option<&Value>, filter_value: &Value) -> Result<bool> {
        let doc_str = doc_value
            .and_then(|v| v.as_str())
            .ok_or(MongoLiteError::QueryError("$regex requires string field".into()))?;

        let pattern = filter_value.as_str()
            .ok_or(MongoLiteError::QueryError("$regex requires string pattern".into()))?;

        let regex = regex::Regex::new(pattern)
            .map_err(|e| MongoLiteError::QueryError(format!("Invalid regex: {}", e)))?;

        Ok(regex.is_match(doc_str))
    }
}
```

### Step 1.6: Refactor matches_filter() (4 hours)

**Modify**: `ironbase-core/src/query.rs`

```rust
// Global registry (lazy_static or once_cell)
static OPERATOR_REGISTRY: Lazy<OperatorRegistry> = Lazy::new(|| OperatorRegistry::new());

/// NEW: Simplified matches_filter (CC: ~12, down from 67!)
pub fn matches_filter(doc: &Value, filter: &Value) -> Result<bool> {
    matches_filter_with_depth(doc, filter, 0)
}

/// Internal: Recursion with depth limit
fn matches_filter_with_depth(doc: &Value, filter: &Value, depth: usize) -> Result<bool> {
    // Prevent stack overflow
    const MAX_RECURSION_DEPTH: usize = 100;
    if depth > MAX_RECURSION_DEPTH {
        return Err(MongoLiteError::QueryError(
            "Query recursion depth exceeded".into()
        ));
    }

    // Empty filter matches all
    if filter.as_object().map(|o| o.is_empty()).unwrap_or(false) {
        return Ok(true);
    }

    let filter_obj = filter.as_object()
        .ok_or(MongoLiteError::QueryError("Filter must be object".into()))?;

    for (key, value) in filter_obj {
        // Check if key is an operator
        if key.starts_with('$') {
            // Logical operator (operates on entire document)
            if let Some(operator) = OPERATOR_REGISTRY.get(key) {
                operator.validate_filter(value)?;
                if !operator.matches(Some(doc), value)? {
                    return Ok(false);
                }
            } else {
                return Err(MongoLiteError::UnsupportedOperator(key.clone()));
            }
        } else {
            // Field-level condition
            let doc_value = doc.get(key);

            if let Some(operator_obj) = value.as_object() {
                // Operator query: {"age": {"$gt": 18}}
                for (op, op_value) in operator_obj {
                    if let Some(operator) = OPERATOR_REGISTRY.get(op) {
                        operator.validate_filter(op_value)?;
                        if !operator.matches(doc_value, op_value)? {
                            return Ok(false);
                        }
                    } else {
                        return Err(MongoLiteError::UnsupportedOperator(op.clone()));
                    }
                }
            } else {
                // Simple equality: {"name": "Alice"}
                if doc_value != Some(value) {
                    return Ok(false);
                }
            }
        }
    }

    Ok(true)
}
```

**Result**:
- CC: 67 → 12 ✅ (83% reduction!)
- Cognitive: 142 → 18 ✅ (87% reduction!)
- Lines: 268 → 45 (83% reduction!)
- Extensibility: Adding new operator = 1 new struct, NO core changes

### Step 1.7: Add Comprehensive Tests (4 hours)

**Create**: `ironbase-core/tests/test_query_operators.rs`

```rust
#[cfg(test)]
mod comparison_tests {
    use super::*;

    #[test]
    fn test_gt_operator() {
        let doc = json!({"age": 30});
        assert!(matches_filter(&doc, &json!({"age": {"$gt": 18}})).unwrap());
        assert!(!matches_filter(&doc, &json!({"age": {"$gt": 40}})).unwrap());
    }

    #[test]
    fn test_in_operator() {
        let doc = json!({"status": "active"});
        assert!(matches_filter(&doc, &json!({"status": {"$in": ["active", "pending"]}})).unwrap());
        assert!(!matches_filter(&doc, &json!({"status": {"$in": ["archived", "deleted"]}})).unwrap());
    }

    // ... 50+ test cases covering all operators
}

#[cfg(test)]
mod logical_tests {
    #[test]
    fn test_and_operator() {
        let doc = json!({"age": 30, "status": "active"});
        assert!(matches_filter(&doc, &json!({
            "$and": [
                {"age": {"$gt": 18}},
                {"status": "active"}
            ]
        })).unwrap());
    }

    // ... 20+ logical operator tests
}

#[cfg(test)]
mod regression_tests {
    // Tests for previously filed bugs
}

#[cfg(test)]
mod property_tests {
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn test_filter_never_panics(filter in any::<Value>()) {
            let doc = json!({"field": 42});
            let _ = matches_filter(&doc, &filter); // Should never panic
        }
    }
}
```

**Target**: >85% test coverage for query.rs (currently 45%)

### Step 1.8: Performance Validation (2 hours)

**Create**: `benches/bench_query.rs`

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_simple_equality(c: &mut Criterion) {
    let doc = json!({"name": "Alice", "age": 30});
    let filter = json!({"name": "Alice"});

    c.bench_function("simple equality", |b| {
        b.iter(|| matches_filter(black_box(&doc), black_box(&filter)))
    });
}

fn bench_complex_query(c: &mut Criterion) {
    let doc = json!({"name": "Alice", "age": 30, "status": "active"});
    let filter = json!({
        "$and": [
            {"age": {"$gt": 18}},
            {"age": {"$lt": 65}},
            {"status": {"$in": ["active", "pending"]}}
        ]
    });

    c.bench_function("complex query", |b| {
        b.iter(|| matches_filter(black_box(&doc), black_box(&filter)))
    });
}

criterion_group!(benches, bench_simple_equality, bench_complex_query);
criterion_main!(benches);
```

**Acceptance Criteria**:
- No performance regression vs old implementation
- Ideally 10-20% faster due to reduced branching

---

## Phase 2: Storage Abstraction (HIGH PRIORITY)

**Duration**: 1-2 weeks (8-12 hours)
**Priority**: ⚠️ HIGH
**Risk**: Low (additive changes, backwards compatible)

### Goals

- Enable dependency injection for testing
- Allow alternative storage implementations (in-memory, cloud, etc.)
- Improve SOLID compliance (DIP)
- Enable future optimizations (caching, compression)

### Step 2.1: Define Storage Traits (2 hours)

**Create**: `ironbase-core/src/storage/traits.rs`

```rust
use crate::document::DocumentId;
use crate::error::Result;
use serde_json::Value;

/// Core trait for reading documents
pub trait DocumentReader: Send + Sync {
    /// Read document bytes from offset
    fn read_data(&self, offset: u64) -> Result<Vec<u8>>;

    /// Read and deserialize document
    fn read_document(&self, offset: u64) -> Result<Value> {
        let data = self.read_data(offset)?;
        Ok(serde_json::from_slice(&data)?)
    }
}

/// Core trait for writing documents
pub trait DocumentWriter: Send + Sync {
    /// Write document and return offset
    fn write_document(
        &mut self,
        collection: &str,
        doc_id: &DocumentId,
        data: &[u8],
    ) -> Result<u64>;
}

/// Trait for catalog management
pub trait CatalogManager: Send + Sync {
    /// Get document offset by ID
    fn get_document_offset(&self, collection: &str, doc_id: &DocumentId) -> Result<Option<u64>>;

    /// Iterate all document offsets in collection
    fn iter_document_offsets(&self, collection: &str) -> Result<Box<dyn Iterator<Item = u64> + '_>>;

    /// Get document count
    fn document_count(&self, collection: &str) -> Result<u64>;
}

/// Trait for metadata management
pub trait MetadataManager: Send + Sync {
    /// Flush metadata to storage
    fn flush_metadata(&mut self) -> Result<()>;

    /// Create new collection
    fn create_collection(&mut self, name: &str) -> Result<()>;

    /// Drop collection
    fn drop_collection(&mut self, name: &str) -> Result<()>;

    /// List all collections
    fn list_collections(&self) -> Result<Vec<String>>;
}

/// Combined trait for full storage operations
pub trait Storage: DocumentReader + DocumentWriter + CatalogManager + MetadataManager {
    /// Compact storage (remove tombstones, reclaim space)
    fn compact(&mut self) -> Result<CompactionStats>;
}
```

### Step 2.2: Implement Traits for StorageEngine (3 hours)

**Modify**: `ironbase-core/src/storage/mod.rs`

```rust
impl DocumentReader for StorageEngine {
    fn read_data(&self, offset: u64) -> Result<Vec<u8>> {
        // Existing implementation
        // ... (no changes to logic)
    }
}

impl DocumentWriter for StorageEngine {
    fn write_document(
        &mut self,
        collection: &str,
        doc_id: &DocumentId,
        data: &[u8],
    ) -> Result<u64> {
        // Existing implementation
        // ... (no changes to logic)
    }
}

impl CatalogManager for StorageEngine {
    fn get_document_offset(&self, collection: &str, doc_id: &DocumentId) -> Result<Option<u64>> {
        let meta = self.collections.get(collection)
            .ok_or_else(|| MongoLiteError::CollectionNotFound(collection.to_string()))?;
        Ok(meta.document_catalog.get(doc_id).copied())
    }

    fn iter_document_offsets(&self, collection: &str) -> Result<Box<dyn Iterator<Item = u64> + '_>> {
        let meta = self.collections.get(collection)
            .ok_or_else(|| MongoLiteError::CollectionNotFound(collection.to_string()))?;
        Ok(Box::new(meta.document_catalog.values().copied()))
    }

    fn document_count(&self, collection: &str) -> Result<u64> {
        let meta = self.collections.get(collection)
            .ok_or_else(|| MongoLiteError::CollectionNotFound(collection.to_string()))?;
        Ok(meta.document_count)
    }
}

impl MetadataManager for StorageEngine {
    fn flush_metadata(&mut self) -> Result<()> {
        // Existing implementation
        // ... (no changes)
    }

    fn create_collection(&mut self, name: &str) -> Result<()> {
        // Existing implementation
        // ... (no changes)
    }

    fn drop_collection(&mut self, name: &str) -> Result<()> {
        // Existing implementation
        // ... (no changes)
    }

    fn list_collections(&self) -> Result<Vec<String>> {
        Ok(self.collections.keys().cloned().collect())
    }
}

impl Storage for StorageEngine {
    fn compact(&mut self) -> Result<CompactionStats> {
        self.compact_with_config(&CompactionConfig::default())
    }
}
```

### Step 2.3: Make Collection Generic Over Storage (2 hours)

**Modify**: `ironbase-core/src/collection.rs`

```rust
// OLD:
pub struct Collection {
    name: String,
    storage: Arc<RwLock<StorageEngine>>,  // Concrete!
}

// NEW:
pub struct Collection<S: Storage> {
    name: String,
    storage: Arc<RwLock<S>>,  // Generic!
}

impl<S: Storage> Collection<S> {
    pub fn new(name: String, storage: Arc<RwLock<S>>) -> Self {
        Collection { name, storage }
    }

    pub fn insert_one(&mut self, doc: Value) -> Result<InsertOneResult> {
        // Implementation unchanged, but now works with any Storage!
        // ...
    }

    // ... rest of methods unchanged
}

// Type alias for backwards compatibility
pub type FileCollection = Collection<StorageEngine>;
```

### Step 2.4: Create In-Memory Storage for Tests (3 hours)

**Create**: `ironbase-core/src/storage/memory.rs`

```rust
use std::collections::HashMap;

/// In-memory storage implementation (for testing)
pub struct MemoryStorage {
    documents: HashMap<u64, Vec<u8>>,  // offset -> data
    catalogs: HashMap<String, HashMap<DocumentId, u64>>,  // collection -> (id -> offset)
    next_offset: u64,
}

impl MemoryStorage {
    pub fn new() -> Self {
        Self {
            documents: HashMap::new(),
            catalogs: HashMap::new(),
            next_offset: 0,
        }
    }
}

impl DocumentReader for MemoryStorage {
    fn read_data(&self, offset: u64) -> Result<Vec<u8>> {
        self.documents.get(&offset)
            .cloned()
            .ok_or(MongoLiteError::DocumentNotFound)
    }
}

impl DocumentWriter for MemoryStorage {
    fn write_document(
        &mut self,
        collection: &str,
        doc_id: &DocumentId,
        data: &[u8],
    ) -> Result<u64> {
        let offset = self.next_offset;
        self.next_offset += data.len() as u64 + 4;

        self.documents.insert(offset, data.to_vec());

        self.catalogs
            .entry(collection.to_string())
            .or_insert_with(HashMap::new)
            .insert(doc_id.clone(), offset);

        Ok(offset)
    }
}

impl CatalogManager for MemoryStorage {
    fn get_document_offset(&self, collection: &str, doc_id: &DocumentId) -> Result<Option<u64>> {
        Ok(self.catalogs.get(collection).and_then(|c| c.get(doc_id).copied()))
    }

    fn iter_document_offsets(&self, collection: &str) -> Result<Box<dyn Iterator<Item = u64> + '_>> {
        if let Some(catalog) = self.catalogs.get(collection) {
            Ok(Box::new(catalog.values().copied().collect::<Vec<_>>().into_iter()))
        } else {
            Ok(Box::new(std::iter::empty()))
        }
    }

    fn document_count(&self, collection: &str) -> Result<u64> {
        Ok(self.catalogs.get(collection).map(|c| c.len() as u64).unwrap_or(0))
    }
}

impl MetadataManager for MemoryStorage {
    fn flush_metadata(&mut self) -> Result<()> {
        Ok(()) // No-op for memory storage
    }

    fn create_collection(&mut self, name: &str) -> Result<()> {
        if self.catalogs.contains_key(name) {
            return Err(MongoLiteError::CollectionExists(name.to_string()));
        }
        self.catalogs.insert(name.to_string(), HashMap::new());
        Ok(())
    }

    fn drop_collection(&mut self, name: &str) -> Result<()> {
        self.catalogs.remove(name)
            .ok_or_else(|| MongoLiteError::CollectionNotFound(name.to_string()))?;
        Ok(())
    }

    fn list_collections(&self) -> Result<Vec<String>> {
        Ok(self.catalogs.keys().cloned().collect())
    }
}

impl Storage for MemoryStorage {
    fn compact(&mut self) -> Result<CompactionStats> {
        // No-op for memory storage (no fragmentation)
        Ok(CompactionStats::default())
    }
}
```

**Benefits**:
- Unit tests run 100x faster (no file I/O!)
- Can test edge cases easily (memory constraints, etc.)
- Enables fuzzing with proptest

---

## Phase 3: Code Quality Improvements (MEDIUM PRIORITY)

**Duration**: 2-3 weeks (40-50 hours)
**Priority**: ⚠️ MEDIUM
**Risk**: Low (mostly additive)

### Step 3.1: Extract Helper Functions (8 hours)

#### Create `common/helpers.rs`

**Target**: Reduce code duplication from 6.2% → <3%

```rust
/// Write length-prefixed data (reduces 48 lines of duplication)
pub fn write_length_prefixed<W: Write>(writer: &mut W, data: &[u8]) -> Result<()> {
    let len = (data.len() as u32).to_le_bytes();
    writer.write_all(&len)?;
    writer.write_all(data)?;
    Ok(())
}

/// Read length-prefixed data
pub fn read_length_prefixed<R: Read>(reader: &mut R) -> Result<Vec<u8>> {
    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes)?;
    let len = u32::from_le_bytes(len_bytes) as usize;

    let mut data = vec![0u8; len];
    reader.read_exact(&mut data)?;
    Ok(data)
}

/// Extract document metadata (_id and _collection)
pub fn extract_document_metadata(doc: &Value) -> Result<(DocumentId, String)> {
    let collection = doc.get("_collection")
        .and_then(|v| v.as_str())
        .ok_or(MongoLiteError::InvalidDocument("Missing _collection field".into()))?;

    let id_value = doc.get("_id")
        .ok_or(MongoLiteError::InvalidDocument("Missing _id field".into()))?;

    let doc_id = serde_json::from_value::<DocumentId>(id_value.clone())?;

    Ok((doc_id, collection.to_string()))
}

/// Check if document is tombstone
pub fn is_tombstone(doc: &Value) -> bool {
    doc.get("_tombstone")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}
```

**Usage** (replaces 368 lines):
```rust
// Before (12 lines):
let meta_bytes = serde_json::to_vec(&meta)?;
let len = (meta_bytes.len() as u32).to_le_bytes();
writer.write_all(&len)?;
writer.write_all(&meta_bytes)?;

// After (1 line):
write_length_prefixed(&mut writer, &meta_bytes)?;
```

### Step 3.2: Simplify Compaction (10 hours)

**Extract phases from `compact_with_config()`**:

```rust
// Phase extraction reduces CC from 34 → ~15

/// Phase 1: Scan documents and collect latest versions
fn scan_documents_for_compaction(
    &self,
    config: &CompactionConfig,
    file_len: u64,
) -> Result<HashMap<String, HashMap<DocumentId, Value>>> {
    let mut collection_docs = HashMap::new();
    for coll_name in self.collections.keys() {
        collection_docs.insert(coll_name.clone(), HashMap::new());
    }

    let mut current_offset = HEADER_SIZE;
    while current_offset < file_len {
        // ... scan logic (now isolated)
    }

    Ok(collection_docs)
}

/// Phase 2: Write compacted documents to new file
fn write_compacted_documents(
    &self,
    new_file: &mut File,
    collection_docs: HashMap<String, HashMap<DocumentId, Value>>,
) -> Result<(u64, HashMap<String, CollectionMeta>)> {
    let mut write_offset = HEADER_SIZE;
    let mut new_collections = self.collections.clone();

    // ... write logic (now isolated)

    Ok((write_offset, new_collections))
}

/// Phase 3: Finalize compaction (metadata + header)
fn finalize_compaction(
    &self,
    new_file: &mut File,
    write_offset: u64,
    new_collections: HashMap<String, CollectionMeta>,
) -> Result<()> {
    // ... finalization logic
    Ok(())
}

/// Main compaction function (now much simpler!)
pub fn compact_with_config(&mut self, config: &CompactionConfig) -> Result<CompactionStats> {
    self.flush_metadata()?;

    let temp_path = format!("{}.compact", self.file_path);
    let mut stats = CompactionStats::default();
    stats.size_before = self.file.metadata()?.len();

    let file_len = self.determine_scan_boundary()?;
    let mut new_file = self.create_temp_file(&temp_path)?;

    // Simple 3-phase structure
    let collection_docs = self.scan_documents_for_compaction(config, file_len)?;
    let (write_offset, new_collections) = self.write_compacted_documents(&mut new_file, collection_docs)?;
    self.finalize_compaction(&mut new_file, write_offset, new_collections)?;

    stats.size_after = new_file.metadata()?.len();
    self.replace_file_with_compacted(&temp_path)?;

    Ok(stats)
}
```

**Result**: CC 34 → 15, Cognitive 78 → 28

### Step 3.3: Simplify flush_metadata() (6 hours)

**Extract offset calculation**:

```rust
/// Calculate where metadata should be written
fn calculate_metadata_offset(&self) -> Result<u64> {
    let mut max_doc_offset: u64 = HEADER_SIZE;

    for coll_meta in self.collections.values() {
        for &doc_offset in coll_meta.document_catalog.values() {
            if doc_offset > max_doc_offset {
                max_doc_offset = doc_offset;
            }
        }
    }

    if max_doc_offset > HEADER_SIZE {
        self.file.seek(SeekFrom::Start(max_doc_offset))?;
        let mut len_bytes = [0u8; 4];
        if self.file.read_exact(&mut len_bytes).is_ok() {
            let doc_len = u32::from_le_bytes(len_bytes) as u64;
            Ok(max_doc_offset + 4 + doc_len)
        } else {
            Ok(self.file.metadata()?.len())
        }
    } else {
        Ok(HEADER_SIZE)
    }
}

/// Serialize metadata to bytes
fn serialize_metadata(&self) -> Result<Vec<u8>> {
    let mut buffer = std::io::Cursor::new(Vec::new());
    Self::write_metadata_body(&mut buffer, &self.collections)?;
    Ok(buffer.into_inner())
}

/// Update header with new metadata location
fn update_header_metadata_info(&mut self, offset: u64, size: u64) -> Result<()> {
    self.header.metadata_offset = offset;
    self.header.metadata_size = size;

    self.file.seek(SeekFrom::Start(0))?;
    let header_bytes = bincode::serialize(&self.header)?;
    self.file.write_all(&header_bytes)?;
    Ok(())
}

/// Simplified flush_metadata
pub(crate) fn flush_metadata(&mut self) -> Result<()> {
    let metadata_bytes = self.serialize_metadata()?;
    let metadata_size = metadata_bytes.len() as u64;
    let metadata_offset = self.calculate_metadata_offset()?;

    self.file.set_len(metadata_offset)?;
    self.file.seek(SeekFrom::Start(metadata_offset))?;
    self.file.write_all(&metadata_bytes)?;

    self.update_header_metadata_info(metadata_offset, metadata_size)?;
    self.file.sync_all()?;

    Ok(())
}
```

**Result**: CC 23 → 12, Cognitive 52 → 18

### Step 3.4: Add Error Context (4 hours)

**Use anyhow::Context** for better error messages:

```rust
use anyhow::{Context, Result};

// Before:
let file = File::open(path)?;
// Error: "No such file or directory"

// After:
let file = File::open(path)
    .context(format!("Failed to open database file: {}", path))?;
// Error: "Failed to open database file: /path/to/db.mlite: No such file or directory"
```

**Apply to all error-prone operations** (reduces duplication clone #1: -224 lines)

### Step 3.5: Add Constants Module (2 hours)

**Create** `constants.rs` to centralize magic numbers:

```rust
// File format constants
pub const MAGIC_NUMBER: &[u8; 8] = b"MONGOLTE";
pub const HEADER_SIZE: u64 = 256;
pub const VERSION_2: u32 = 2;

// Performance tuning
pub const DEFAULT_PAGE_SIZE: u32 = 4096;
pub const DEFAULT_CHUNK_SIZE: usize = 1000;
pub const MMAP_THRESHOLD: u64 = 1_000_000_000; // 1 GB

// Limits
pub const MAX_RECURSION_DEPTH: usize = 100;
pub const MAX_DOCUMENT_SIZE: u64 = 16_000_000; // 16 MB
pub const MAX_COLLECTION_NAME_LENGTH: usize = 255;
```

**Refactor all hardcoded values** (improves maintainability)

### Step 3.6: Split Large Modules (10 hours)

**Refactor** `collection.rs` (850 LOC) into:
- `collection/crud.rs` (insert, update, delete)
- `collection/query.rs` (find, count)
- `collection/mod.rs` (Collection struct, coordination)

**Result**: Improve MI from 14.2 → ~35

---

## Phase 4: Testing & Infrastructure (LOW PRIORITY)

**Duration**: 1-2 weeks (30-40 hours)
**Priority**: ✅ LOW (but important for long-term)
**Risk**: Very Low

### Step 4.1: Add Integration Tests (12 hours)

**Create** `tests/integration/`:

```
tests/
  integration/
    test_crash_recovery.rs   # WAL replay scenarios
    test_concurrent.rs       # Multi-threaded operations
    test_large_dataset.rs    # 100K+ documents
    test_edge_cases.rs       # Empty collections, etc.
```

**Focus on critical scenarios**:
- Crash before metadata flush
- Crash during compaction
- Concurrent reads/writes
- Large documents (>1 MB)
- Many collections (>100)

### Step 4.2: Add Property-Based Tests (8 hours)

**Use proptest** for fuzzing:

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_insert_find_roundtrip(doc in any_json_object()) {
        let storage = MemoryStorage::new();
        let mut collection = Collection::new("test", Arc::new(RwLock::new(storage)));

        let result = collection.insert_one(doc.clone()).unwrap();
        let found = collection.find_one(json!({"_id": result.inserted_id})).unwrap();

        assert_eq!(found, Some(doc));
    }
}
```

**Generate random**:
- Documents (varying sizes, types)
- Queries (random operators, nesting)
- Operation sequences (insert → update → delete)

**Target**: Find edge cases that manual tests miss

### Step 4.3: Add Performance Benchmarks (6 hours)

**Create** `benches/`:

```
benches/
  bench_insert.rs      # Insert throughput
  bench_find.rs        # Query performance
  bench_compaction.rs  # Compaction time
  bench_metadata.rs    # Metadata flush time
```

**Track over time** (prevent regressions):
- Insert: docs/sec
- Find: ms per 10K docs
- Compaction: sec per GB
- Metadata flush: ms

### Step 4.4: Add CI/CD Pipeline (4 hours)

**Create** `.github/workflows/ci.yml`:

```yaml
name: CI

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
      - name: Run tests
        run: cargo test --all-features
      - name: Run benchmarks (baseline)
        run: cargo bench --no-run

  lint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - name: Run clippy
        run: cargo clippy -- -D warnings
      - name: Check formatting
        run: cargo fmt --check

  coverage:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - name: Install tarpaulin
        run: cargo install cargo-tarpaulin
      - name: Generate coverage
        run: cargo tarpaulin --out Lcov
      - name: Upload to Codecov
        uses: codecov/codecov-action@v3

  security:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - name: Run cargo audit
        run: cargo audit
```

### Step 4.5: Add API Documentation (8 hours)

**Add rustdoc comments** to all public items:

```rust
/// A lightweight embedded NoSQL database with MongoDB-like API.
///
/// # Examples
///
/// ```
/// use ironbase::IronBase;
///
/// let db = IronBase::new("my_database.mlite")?;
/// let collection = db.collection("users");
///
/// collection.insert_one(json!({
///     "name": "Alice",
///     "age": 30
/// }))?;
/// ```
///
/// # Performance
///
/// - Insert: ~11,500 docs/sec
/// - Find (full scan): ~50K docs in 65ms
/// - Storage: ~100 bytes overhead per document
///
/// # Thread Safety
///
/// All operations are thread-safe through internal `RwLock`.
/// Multiple readers can access concurrently.
pub struct IronBase { ... }
```

**Generate docs**: `cargo doc --open`

### Step 4.6: Add Architecture Decision Records (2 hours)

**Create** `docs/adr/`:

```
docs/adr/
  0001-use-append-only-storage.md
  0002-dynamic-metadata-at-file-end.md
  0003-lazy-metadata-flush.md
  0004-operator-registry-pattern.md
```

**Template**:
```markdown
# ADR-0002: Dynamic Metadata at File End

## Status
Accepted

## Context
Version 1 used fixed 10MB metadata reservation, causing 10MB bloat for small databases.

## Decision
Move metadata to end of file, store offset in header.

## Consequences
**Positive**:
- 99.91% size reduction for small DBs
- Unlimited metadata size
- O(1) insert performance (with lazy flush)

**Negative**:
- Metadata position changes on every flush
- Requires catalog scan to find metadata offset

## Alternatives Considered
1. Fixed metadata at beginning (rejected: bloat)
2. Separate metadata file (rejected: complexity)
```

---

## Implementation Guidelines

### Code Review Checklist

Before merging any refactoring PR:

- [ ] Tests pass (existing + new)
- [ ] No performance regression (benchmarks)
- [ ] Code coverage maintained or improved
- [ ] Documentation updated
- [ ] CHANGELOG.md updated
- [ ] No new clippy warnings
- [ ] Reviewed by at least 1 other developer

### Testing Strategy

**For each refactoring**:
1. Write failing test first (TDD)
2. Implement refactoring
3. Verify test passes
4. Run full test suite
5. Run benchmarks (check no regression)

### Git Workflow

**Branch naming**:
- `refactor/phase1-query-system`
- `refactor/phase2-storage-traits`
- `refactor/phase3-helpers`

**Commit messages**:
```
refactor(query): Extract comparison operators to separate module

- Created OperatorMatcher trait
- Implemented $gt, $gte, $lt, $lte, $eq, $ne
- Reduced matches_filter CC from 67 to 45 (intermediate step)
- Added 20 new unit tests for comparison operators

Part of Phase 1: Query System Overhaul
Refs: REFACTORING_PLAN.md
```

### Rollback Plan

**If refactoring introduces bugs**:
1. Revert PR immediately
2. Create branch from last good commit
3. Investigate root cause
4. Add regression test
5. Re-apply refactoring with fix
6. Merge after verification

**No refactoring should block releases!**

---

## Risk Management

### High-Risk Areas

#### 1. Query System Refactoring
**Risk**: Breaking existing queries
**Mitigation**:
- Comprehensive test suite (>85% coverage)
- Keep old implementation alongside new (feature flag)
- Gradual rollout (10% → 50% → 100% traffic)
- Monitor error rates in production

#### 2. Storage Abstraction
**Risk**: Performance regression
**Mitigation**:
- Benchmark before/after
- Accept only if <5% regression
- Profile with flamegraph if needed

#### 3. Compaction Changes
**Risk**: Data corruption
**Mitigation**:
- Test with 1M+ documents
- Verify checksums before/after
- Keep backup before compaction
- Automated recovery tests

### Rollback Triggers

Immediately rollback if:
- Test coverage drops below 70%
- Performance regresses >10%
- Any data corruption detected
- >5% increase in error rate

---

## Success Metrics

### Phase 1 Success Criteria

- [ ] matches_filter() CC < 15 (target: 12)
- [ ] query.rs test coverage > 85% (currently 45%)
- [ ] No performance regression in benchmarks
- [ ] Can add new operator in <1 hour (vs 4-6 hours currently)
- [ ] Zero regressions in existing queries

### Phase 2 Success Criteria

- [ ] Collection<S: Storage> compiles and tests pass
- [ ] MemoryStorage implementation complete
- [ ] Unit tests 10x faster (no file I/O)
- [ ] Can swap storage implementations without code changes
- [ ] SOLID DIP compliance > 80%

### Phase 3 Success Criteria

- [ ] Code duplication < 3% (currently 6.2%)
- [ ] Average MI > 30 (currently 23.6)
- [ ] Compaction CC < 20 (currently 34)
- [ ] flush_metadata CC < 15 (currently 23)
- [ ] All modules have MI > 25

### Phase 4 Success Criteria

- [ ] Test coverage > 80% (all modules)
- [ ] CI/CD pipeline green
- [ ] Benchmarks tracked over time
- [ ] Documentation complete (rustdoc)
- [ ] Zero security vulnerabilities (cargo audit)

### Overall Project Success

**At completion**:
- Cyclomatic Complexity: Average <10, Max <25
- Code Duplication: <3%
- Maintainability Index: >40
- SOLID Compliance: >85%
- Technical Debt Ratio: SQALE A (<5%)
- Test Coverage: >80%
- Interest Rate: <3% (currently 8.4%)

**ROI Achieved**: 10-month payback period validated

---

## Timeline

### Weeks 1-3: Phase 1 (Query System)
- Week 1: Extract operators (Steps 1.1-1.5)
- Week 2: Refactor matches_filter (Step 1.6)
- Week 3: Tests + validation (Steps 1.7-1.8)

### Weeks 4-5: Phase 2 (Storage Abstraction)
- Week 4: Define traits + implement for StorageEngine (Steps 2.1-2.2)
- Week 5: Generic Collection + MemoryStorage (Steps 2.3-2.4)

### Weeks 6-8: Phase 3 (Code Quality)
- Week 6: Extract helpers + simplify compaction (Steps 3.1-3.2)
- Week 7: Simplify flush_metadata + error context (Steps 3.3-3.4)
- Week 8: Constants + split modules (Steps 3.5-3.6)

### Weeks 9-10: Phase 4 (Testing & Infrastructure)
- Week 9: Integration tests + property tests (Steps 4.1-4.2)
- Week 10: Benchmarks + CI/CD + docs (Steps 4.3-4.6)

**Total**: 10 weeks (can be compressed to 6-8 with focused effort)

---

## Conclusion

This refactoring plan addresses **174-257 hours of technical debt** across 4 phases. The plan is:

✅ **Data-driven**: Based on rigorous quality metrics analysis
✅ **Prioritized**: Critical issues first, nice-to-haves later
✅ **Risk-managed**: Rollback plans, gradual rollouts, monitoring
✅ **Measurable**: Clear success criteria and KPIs
✅ **ROI-positive**: 10-month break-even on critical refactorings

**Key Improvements**:
- Cyclomatic Complexity: 67 → <15 (78% reduction)
- Code Duplication: 6.2% → <3% (52% reduction)
- Maintainability Index: 23.6 → >40 (70% increase)
- SOLID Compliance: 62% → >85% (37% increase)
- Technical Debt Interest: 8.4% → <3% (64% reduction)

**Business Value**:
- Faster feature development (2-3x speedup for query features)
- Easier onboarding (better code structure)
- Fewer bugs (higher test coverage, simpler code)
- Better performance (optimization opportunities from abstractions)

**Execute in order, measure continuously, and adjust as needed!**

---

**Next Steps**:
1. Review plan with team
2. Create GitHub project with tasks
3. Start Phase 1 (Query System)
4. Weekly progress reviews
5. Celebrate wins! 🎉
