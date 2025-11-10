# Aggregation Pipeline Implementation Specification

## Overview

The aggregation pipeline is a framework for data aggregation modeled on the concept of data processing pipelines. Documents enter a multi-stage pipeline that transforms the documents into aggregated results.

## Architecture

```
Input Documents
      ↓
[$match stage] ────→ Filter documents (like find)
      ↓
[$project stage] ──→ Reshape documents, add/remove fields
      ↓
[$group stage] ────→ Group by expression, compute aggregates
      ↓
[$sort stage] ─────→ Sort documents
      ↓
[$limit stage] ────→ Limit number of documents
      ↓
[$skip stage] ─────→ Skip N documents
      ↓
Output Documents
```

## MVP Stages (Phase 1)

### 1. `$match` - Filter Documents

**Purpose:** Filters documents to pass only those that match the specified condition(s).

**Syntax:**
```json
{"$match": {"age": {"$gte": 18}}}
```

**Implementation:**
- Reuse existing `Query::matches()` logic
- Should be placed early in pipeline for performance
- Can use indexes if first stage

**Example:**
```python
# Find all adults
collection.aggregate([
    {"$match": {"age": {"$gte": 18}}}
])
```

### 2. `$project` - Reshape Documents

**Purpose:** Passes along documents with only specified fields or adds new computed fields.

**Syntax:**
```json
{
    "$project": {
        "name": 1,           # Include field
        "age": 1,            # Include field
        "_id": 0,            # Exclude field
        "year": "$birthYear" # Rename field
    }
}
```

**Implementation:**
```rust
pub struct ProjectStage {
    fields: HashMap<String, ProjectField>,
}

pub enum ProjectField {
    Include,                        // 1
    Exclude,                        // 0
    Rename(String),                 // "$fieldName"
    Computed(Expression),           // Future: computed fields
}
```

**Example:**
```python
# Show only name and age
collection.aggregate([
    {"$project": {"name": 1, "age": 1, "_id": 0}}
])
```

### 3. `$group` - Group Documents

**Purpose:** Groups documents by a specified expression and outputs one document per each distinct grouping.

**Syntax:**
```json
{
    "$group": {
        "_id": "$city",              # Group by field
        "count": {"$sum": 1},        # Count documents
        "avgAge": {"$avg": "$age"},  # Average age
        "maxAge": {"$max": "$age"}   # Maximum age
    }
}
```

**Implementation:**
```rust
pub struct GroupStage {
    id: GroupId,                          // Group by expression
    accumulators: HashMap<String, Accumulator>,
}

pub enum GroupId {
    Field(String),                        // "$city"
    Null,                                 // null (all docs in one group)
    Compound(Vec<(String, String)>),      // {"city": "$city", "country": "$country"}
}

pub enum Accumulator {
    Sum(SumExpression),
    Avg(String),        // Field name
    Min(String),
    Max(String),
    First(String),
    Last(String),
    Count,
}

pub enum SumExpression {
    Constant(i64),      // {"$sum": 1} - count
    Field(String),      // {"$sum": "$amount"} - sum field
}
```

**Example:**
```python
# Count users per city
collection.aggregate([
    {"$group": {
        "_id": "$city",
        "count": {"$sum": 1},
        "avgAge": {"$avg": "$age"}
    }}
])
```

### 4. `$sort` - Sort Documents

**Purpose:** Sorts all documents and passes them through in sorted order.

**Syntax:**
```json
{"$sort": {"age": 1, "name": -1}}
```
- `1` = ascending
- `-1` = descending

**Implementation:**
```rust
pub struct SortStage {
    fields: Vec<(String, SortDirection)>,
}

pub enum SortDirection {
    Ascending,
    Descending,
}
```

**Example:**
```python
# Sort by age ascending, then name descending
collection.aggregate([
    {"$sort": {"age": 1, "name": -1}}
])
```

### 5. `$limit` - Limit Documents

**Purpose:** Limits the number of documents passed to the next stage.

**Syntax:**
```json
{"$limit": 10}
```

**Implementation:**
```rust
pub struct LimitStage {
    limit: usize,
}
```

**Example:**
```python
# Get top 10
collection.aggregate([
    {"$sort": {"score": -1}},
    {"$limit": 10}
])
```

### 6. `$skip` - Skip Documents

**Purpose:** Skips over the specified number of documents.

**Syntax:**
```json
{"$skip": 20}
```

**Implementation:**
```rust
pub struct SkipStage {
    skip: usize,
}
```

**Example:**
```python
# Pagination: skip 20, take 10
collection.aggregate([
    {"$sort": {"created_at": -1}},
    {"$skip": 20},
    {"$limit": 10}
])
```

## Core Implementation

### Pipeline Structure

```rust
// src/aggregation.rs

use serde_json::Value;
use crate::document::Document;
use crate::error::Result;
use std::collections::HashMap;

/// Aggregation pipeline
pub struct Pipeline {
    stages: Vec<Stage>,
}

/// Pipeline stage
pub enum Stage {
    Match(MatchStage),
    Project(ProjectStage),
    Group(GroupStage),
    Sort(SortStage),
    Limit(LimitStage),
    Skip(SkipStage),
}

impl Pipeline {
    /// Create pipeline from JSON array
    pub fn from_json(pipeline_json: &Value) -> Result<Self> {
        // Parse pipeline stages
        let stages = parse_stages(pipeline_json)?;
        Ok(Pipeline { stages })
    }

    /// Execute pipeline on documents
    pub fn execute(&self, mut docs: Vec<Value>) -> Result<Vec<Value>> {
        for stage in &self.stages {
            docs = stage.execute(docs)?;
        }
        Ok(docs)
    }
}

impl Stage {
    /// Execute this stage
    fn execute(&self, docs: Vec<Value>) -> Result<Vec<Value>> {
        match self {
            Stage::Match(stage) => stage.execute(docs),
            Stage::Project(stage) => stage.execute(docs),
            Stage::Group(stage) => stage.execute(docs),
            Stage::Sort(stage) => stage.execute(docs),
            Stage::Limit(stage) => stage.execute(docs),
            Stage::Skip(stage) => stage.execute(docs),
        }
    }
}
```

### Match Stage Implementation

```rust
pub struct MatchStage {
    query: Query,
}

impl MatchStage {
    pub fn execute(&self, docs: Vec<Value>) -> Result<Vec<Value>> {
        let mut results = Vec::new();

        for doc in docs {
            let document = Document::from_json(&serde_json::to_string(&doc)?)?;
            if self.query.matches(&document) {
                results.push(doc);
            }
        }

        Ok(results)
    }
}
```

### Project Stage Implementation

```rust
pub struct ProjectStage {
    fields: HashMap<String, ProjectField>,
}

impl ProjectStage {
    pub fn execute(&self, docs: Vec<Value>) -> Result<Vec<Value>> {
        let mut results = Vec::new();

        for doc in docs {
            let projected = self.project_document(&doc)?;
            results.push(projected);
        }

        Ok(results)
    }

    fn project_document(&self, doc: &Value) -> Result<Value> {
        let mut result = serde_json::Map::new();

        if let Value::Object(obj) = doc {
            for (field, action) in &self.fields {
                match action {
                    ProjectField::Include => {
                        if let Some(value) = obj.get(field) {
                            result.insert(field.clone(), value.clone());
                        }
                    }
                    ProjectField::Exclude => {
                        // Skip this field
                    }
                    ProjectField::Rename(source) => {
                        // Remove $ prefix
                        let source_field = source.trim_start_matches('$');
                        if let Some(value) = obj.get(source_field) {
                            result.insert(field.clone(), value.clone());
                        }
                    }
                    ProjectField::Computed(_expr) => {
                        // Future: computed fields
                        unimplemented!("Computed fields not yet supported")
                    }
                }
            }
        }

        Ok(Value::Object(result))
    }
}
```

### Group Stage Implementation

```rust
pub struct GroupStage {
    id: GroupId,
    accumulators: HashMap<String, Accumulator>,
}

impl GroupStage {
    pub fn execute(&self, docs: Vec<Value>) -> Result<Vec<Value>> {
        // Step 1: Group documents by _id expression
        let mut groups: HashMap<String, Vec<Value>> = HashMap::new();

        for doc in docs {
            let group_key = self.extract_group_key(&doc)?;
            groups.entry(group_key).or_insert_with(Vec::new).push(doc);
        }

        // Step 2: Compute accumulators for each group
        let mut results = Vec::new();

        for (key, group_docs) in groups {
            let mut result = serde_json::Map::new();

            // Set _id
            result.insert("_id".to_string(), self.parse_group_key(&key)?);

            // Compute each accumulator
            for (field, accumulator) in &self.accumulators {
                let value = accumulator.compute(&group_docs)?;
                result.insert(field.clone(), value);
            }

            results.push(Value::Object(result));
        }

        Ok(results)
    }

    fn extract_group_key(&self, doc: &Value) -> Result<String> {
        match &self.id {
            GroupId::Null => Ok("__all__".to_string()),
            GroupId::Field(field) => {
                let field_name = field.trim_start_matches('$');
                if let Some(value) = doc.get(field_name) {
                    Ok(serde_json::to_string(value)?)
                } else {
                    Ok("null".to_string())
                }
            }
            GroupId::Compound(_fields) => {
                // Future: compound group keys
                unimplemented!("Compound group keys not yet supported")
            }
        }
    }
}
```

### Accumulator Implementation

```rust
impl Accumulator {
    pub fn compute(&self, docs: &[Value]) -> Result<Value> {
        match self {
            Accumulator::Count => {
                Ok(Value::from(docs.len() as i64))
            }

            Accumulator::Sum(expr) => {
                let mut sum = 0i64;
                match expr {
                    SumExpression::Constant(n) => {
                        sum = (*n) * (docs.len() as i64);
                    }
                    SumExpression::Field(field) => {
                        for doc in docs {
                            if let Some(value) = doc.get(field) {
                                if let Some(n) = value.as_i64() {
                                    sum += n;
                                } else if let Some(f) = value.as_f64() {
                                    return Ok(Value::from(f * docs.len() as f64));
                                }
                            }
                        }
                    }
                }
                Ok(Value::from(sum))
            }

            Accumulator::Avg(field) => {
                let mut sum = 0.0;
                let mut count = 0;

                for doc in docs {
                    if let Some(value) = doc.get(field) {
                        if let Some(n) = value.as_f64() {
                            sum += n;
                            count += 1;
                        } else if let Some(n) = value.as_i64() {
                            sum += n as f64;
                            count += 1;
                        }
                    }
                }

                if count > 0 {
                    Ok(Value::from(sum / count as f64))
                } else {
                    Ok(Value::Null)
                }
            }

            Accumulator::Min(field) => {
                let mut min: Option<f64> = None;

                for doc in docs {
                    if let Some(value) = doc.get(field) {
                        let num = if let Some(n) = value.as_f64() {
                            n
                        } else if let Some(n) = value.as_i64() {
                            n as f64
                        } else {
                            continue;
                        };

                        min = Some(min.map_or(num, |m| m.min(num)));
                    }
                }

                Ok(min.map(Value::from).unwrap_or(Value::Null))
            }

            Accumulator::Max(field) => {
                let mut max: Option<f64> = None;

                for doc in docs {
                    if let Some(value) = doc.get(field) {
                        let num = if let Some(n) = value.as_f64() {
                            n
                        } else if let Some(n) = value.as_i64() {
                            n as f64
                        } else {
                            continue;
                        };

                        max = Some(max.map_or(num, |m| m.max(num)));
                    }
                }

                Ok(max.map(Value::from).unwrap_or(Value::Null))
            }

            Accumulator::First(field) => {
                docs.first()
                    .and_then(|doc| doc.get(field))
                    .cloned()
                    .ok_or_else(|| MongoLiteError::AggregationError("No documents".to_string()))
            }

            Accumulator::Last(field) => {
                docs.last()
                    .and_then(|doc| doc.get(field))
                    .cloned()
                    .ok_or_else(|| MongoLiteError::AggregationError("No documents".to_string()))
            }
        }
    }
}
```

### Sort Stage Implementation

```rust
pub struct SortStage {
    fields: Vec<(String, SortDirection)>,
}

impl SortStage {
    pub fn execute(&self, mut docs: Vec<Value>) -> Result<Vec<Value>> {
        docs.sort_by(|a, b| {
            for (field, direction) in &self.fields {
                let val_a = a.get(field);
                let val_b = b.get(field);

                let cmp = compare_values(val_a, val_b);
                let cmp = match direction {
                    SortDirection::Ascending => cmp,
                    SortDirection::Descending => cmp.reverse(),
                };

                if cmp != std::cmp::Ordering::Equal {
                    return cmp;
                }
            }
            std::cmp::Ordering::Equal
        });

        Ok(docs)
    }
}

fn compare_values(a: Option<&Value>, b: Option<&Value>) -> std::cmp::Ordering {
    match (a, b) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(a), Some(b)) => {
            // String comparison
            if let (Some(s1), Some(s2)) = (a.as_str(), b.as_str()) {
                return s1.cmp(s2);
            }

            // Number comparison
            if let (Some(n1), Some(n2)) = (a.as_f64(), b.as_f64()) {
                return n1.partial_cmp(&n2).unwrap_or(std::cmp::Ordering::Equal);
            }

            // Boolean comparison
            if let (Some(b1), Some(b2)) = (a.as_bool(), b.as_bool()) {
                return b1.cmp(&b2);
            }

            std::cmp::Ordering::Equal
        }
    }
}
```

### Limit and Skip Stages

```rust
pub struct LimitStage {
    limit: usize,
}

impl LimitStage {
    pub fn execute(&self, docs: Vec<Value>) -> Result<Vec<Value>> {
        Ok(docs.into_iter().take(self.limit).collect())
    }
}

pub struct SkipStage {
    skip: usize,
}

impl SkipStage {
    pub fn execute(&self, docs: Vec<Value>) -> Result<Vec<Value>> {
        Ok(docs.into_iter().skip(self.skip).collect())
    }
}
```

## CollectionCore Integration

```rust
// In collection_core.rs

impl CollectionCore {
    /// Execute aggregation pipeline
    pub fn aggregate(&self, pipeline_json: &Value) -> Result<Vec<Value>> {
        // Parse pipeline
        let pipeline = Pipeline::from_json(pipeline_json)?;

        // Get all documents (optimize: use index if $match is first stage)
        let docs = self.find(&serde_json::json!({}))?;

        // Execute pipeline
        pipeline.execute(docs)
    }
}
```

## Python Binding

```python
# collection.aggregate()
def aggregate(self, pipeline: list) -> list:
    """
    Execute aggregation pipeline.

    Args:
        pipeline: List of stage dictionaries

    Returns:
        List of result documents

    Example:
        results = collection.aggregate([
            {"$match": {"age": {"$gte": 18}}},
            {"$group": {"_id": "$city", "count": {"$sum": 1}}},
            {"$sort": {"count": -1}}
        ])
    """
```

## Testing Strategy

### Unit Tests

```rust
#[test]
fn test_match_stage() {
    let docs = vec![
        json!({"name": "Alice", "age": 25}),
        json!({"name": "Bob", "age": 30}),
    ];

    let stage = MatchStage {
        query: Query::from_json(&json!({"age": {"$gte": 26}})).unwrap()
    };

    let results = stage.execute(docs).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["name"], "Bob");
}

#[test]
fn test_group_stage_count() {
    let docs = vec![
        json!({"city": "NYC", "age": 25}),
        json!({"city": "LA", "age": 30}),
        json!({"city": "NYC", "age": 35}),
    ];

    let stage = GroupStage {
        id: GroupId::Field("$city".to_string()),
        accumulators: HashMap::from([
            ("count".to_string(), Accumulator::Count),
        ]),
    };

    let results = stage.execute(docs).unwrap();
    assert_eq!(results.len(), 2);
}
```

### Integration Tests

```rust
#[test]
fn test_full_pipeline() {
    let db = DatabaseCore::open(&temp_path).unwrap();
    let users = db.collection("users").unwrap();

    // Insert test data
    for i in 0..100 {
        users.insert_one(HashMap::from([
            ("name".to_string(), json!(format!("User{}", i))),
            ("age".to_string(), json!(i % 10)),
            ("city".to_string(), json!(if i % 2 == 0 { "NYC" } else { "LA" })),
        ])).unwrap();
    }

    // Aggregation pipeline
    let results = users.aggregate(&json!([
        {"$match": {"age": {"$gte": 5}}},
        {"$group": {"_id": "$city", "count": {"$sum": 1}, "avgAge": {"$avg": "$age"}}},
        {"$sort": {"count": -1}}
    ])).unwrap();

    assert_eq!(results.len(), 2);
}
```

## Performance Considerations

1. **Early $match optimization**: If $match is first stage, use index
2. **$sort + $limit optimization**: Only keep top N in memory
3. **Memory management**: Stream large result sets
4. **Index usage**: Detect when $group _id can use index

## Future Enhancements (Phase 2+)

- `$unwind` - Deconstruct arrays
- `$lookup` - Join collections
- `$facet` - Multi-faceted aggregation
- `$bucket` - Categorize documents into buckets
- `$out` - Write results to collection
- Expression operators: `$add`, `$multiply`, `$concat`, etc.
- Computed fields in `$project`

## Error Handling

```rust
pub enum AggregationError {
    InvalidStage(String),
    InvalidAccumulator(String),
    InvalidGroupId(String),
    TypeMismatch(String),
    EmptyPipeline,
}
```

## Documentation Examples

### Example 1: Group and Count
```python
# Count users per city
results = users.aggregate([
    {"$group": {"_id": "$city", "total": {"$sum": 1}}}
])
# Result: [{"_id": "NYC", "total": 50}, {"_id": "LA", "total": 30}]
```

### Example 2: Filter, Group, Sort
```python
# Adults per city, sorted by count
results = users.aggregate([
    {"$match": {"age": {"$gte": 18}}},
    {"$group": {"_id": "$city", "count": {"$sum": 1}}},
    {"$sort": {"count": -1}}
])
```

### Example 3: Statistics
```python
# Calculate age statistics per city
results = users.aggregate([
    {"$group": {
        "_id": "$city",
        "avgAge": {"$avg": "$age"},
        "minAge": {"$min": "$age"},
        "maxAge": {"$max": "$age"},
        "count": {"$sum": 1}
    }}
])
```

### Example 4: Pagination
```python
# Get page 3 (items 20-29) sorted by age
results = users.aggregate([
    {"$sort": {"age": -1, "name": 1}},
    {"$skip": 20},
    {"$limit": 10}
])
```

## Implementation Checklist

- [ ] Create `aggregation.rs` module
- [ ] Implement `Pipeline` struct
- [ ] Implement `Stage` enum
- [ ] Implement `MatchStage`
- [ ] Implement `ProjectStage`
- [ ] Implement `GroupStage`
- [ ] Implement `Accumulator` enum
- [ ] Implement `SortStage`
- [ ] Implement `LimitStage`
- [ ] Implement `SkipStage`
- [ ] Add `aggregate()` to `CollectionCore`
- [ ] Add Python binding for `aggregate()`
- [ ] Write unit tests
- [ ] Write integration tests
- [ ] Update documentation
- [ ] Add usage examples
