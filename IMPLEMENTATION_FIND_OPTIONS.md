# Find Options Implementation

## Áttekintés

Ez a dokumentum a `find()` metódus kiegészítését specifikálja projection, sort, limit, és skip paraméterekkel.

## Célok

1. **Projection**: Field filtering (include/exclude mode)
2. **Sort**: Single és multi-field sorting
3. **Limit**: Maximum results count
4. **Skip**: Pagination support

## API Design

### Rust API (CollectionCore)

```rust
pub struct FindOptions {
    pub projection: Option<HashMap<String, i32>>,  // 1 = include, 0 = exclude
    pub sort: Option<Vec<(String, i32)>>,          // field, direction (1/-1)
    pub limit: Option<usize>,
    pub skip: Option<usize>,
}

impl CollectionCore {
    // New method with options
    pub fn find_with_options(
        &self,
        query_json: &Value,
        options: FindOptions
    ) -> Result<Vec<Value>>;

    // Keep old method for backwards compatibility
    pub fn find(&self, query_json: &Value) -> Result<Vec<Value>> {
        self.find_with_options(query_json, FindOptions::default())
    }
}
```

### Python API

```python
collection.find(
    query,
    projection=None,   # {"name": 1, "age": 1, "_id": 0}
    sort=None,         # [("age", -1), ("name", 1)]
    limit=None,        # 10
    skip=None          # 5
)
```

## Implementáció

### 1. FindOptions struktúra

**Fájl:** `ironbase-core/src/find_options.rs` (új fájl)

```rust
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct FindOptions {
    /// Projection: field → 1 (include) or 0 (exclude)
    /// Special case: _id can be excluded in include mode
    pub projection: Option<HashMap<String, i32>>,

    /// Sort: [(field, direction)], direction: 1 (asc) or -1 (desc)
    pub sort: Option<Vec<(String, i32)>>,

    /// Limit: maximum number of documents to return
    pub limit: Option<usize>,

    /// Skip: number of documents to skip (for pagination)
    pub skip: Option<usize>,
}

impl FindOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_projection(mut self, projection: HashMap<String, i32>) -> Self {
        self.projection = Some(projection);
        self
    }

    pub fn with_sort(mut self, sort: Vec<(String, i32)>) -> Self {
        self.sort = Some(sort);
        self
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn with_skip(mut self, skip: usize) -> Self {
        self.skip = Some(skip);
        self
    }
}
```

### 2. Projection Logic

**Algoritmus:**
1. Detect mode (include vagy exclude)
   - Include mode: Van legalább egy field: 1
   - Exclude mode: Csak field: 0 értékek (kivéve _id)
2. Include mode: Csak a specified fieldek + _id (unless explicitly excluded)
3. Exclude mode: Minden field kivéve a specified

**Implementáció:**

```rust
fn apply_projection(doc: &Value, projection: &HashMap<String, i32>) -> Value {
    if projection.is_empty() {
        return doc.clone();
    }

    // Detect mode
    let has_inclusions = projection.values().any(|&v| v == 1);
    let has_non_id_exclusions = projection.iter()
        .any(|(field, &action)| action == 0 && field != "_id");

    let include_mode = has_inclusions && !has_non_id_exclusions;

    if let Value::Object(obj) = doc {
        let mut result = serde_json::Map::new();

        if include_mode {
            // Include specified fields
            for (field, &action) in projection {
                if action == 1 {
                    if let Some(value) = obj.get(field) {
                        result.insert(field.clone(), value.clone());
                    }
                }
            }

            // Include _id unless explicitly excluded
            if projection.get("_id") != Some(&0) {
                if let Some(id) = obj.get("_id") {
                    result.insert("_id".to_string(), id.clone());
                }
            }
        } else {
            // Exclude mode: copy all except excluded
            for (key, value) in obj {
                if projection.get(key) != Some(&0) {
                    result.insert(key.clone(), value.clone());
                }
            }
        }

        Value::Object(result)
    } else {
        doc.clone()
    }
}
```

### 3. Sort Logic

**Algoritmus:**
1. Multi-field comparison: Compare by first field, if equal compare by second, etc.
2. Handle different types (int, float, string, null)
3. Direction: 1 = ascending, -1 = descending

**Implementáció:**

```rust
fn apply_sort(docs: &mut Vec<Value>, sort: &[(String, i32)]) {
    if sort.is_empty() {
        return;
    }

    docs.sort_by(|a, b| {
        for (field, direction) in sort {
            let val_a = a.get(field);
            let val_b = b.get(field);

            let cmp = compare_values(val_a, val_b);

            if cmp != std::cmp::Ordering::Equal {
                return if *direction == 1 { cmp } else { cmp.reverse() };
            }
        }
        std::cmp::Ordering::Equal
    });
}

fn compare_values(a: Option<&Value>, b: Option<&Value>) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,    // null < any value
        (Some(_), None) => Ordering::Greater,

        (Some(Value::Number(n1)), Some(Value::Number(n2))) => {
            let f1 = n1.as_f64().unwrap_or(0.0);
            let f2 = n2.as_f64().unwrap_or(0.0);
            f1.partial_cmp(&f2).unwrap_or(Ordering::Equal)
        }

        (Some(Value::String(s1)), Some(Value::String(s2))) => s1.cmp(s2),

        (Some(Value::Bool(b1)), Some(Value::Bool(b2))) => b1.cmp(b2),

        // Type priority: null < number < string < bool < object < array
        (Some(a_val), Some(b_val)) => {
            type_priority(a_val).cmp(&type_priority(b_val))
        }
    }
}

fn type_priority(val: &Value) -> u8 {
    match val {
        Value::Null => 0,
        Value::Number(_) => 1,
        Value::String(_) => 2,
        Value::Bool(_) => 3,
        Value::Object(_) => 4,
        Value::Array(_) => 5,
    }
}
```

### 4. Limit and Skip

**Implementáció:**

```rust
fn apply_limit_skip(docs: Vec<Value>, limit: Option<usize>, skip: Option<usize>) -> Vec<Value> {
    let mut iter = docs.into_iter();

    // Skip first N elements
    if let Some(skip_count) = skip {
        iter = iter.skip(skip_count);
    }

    // Take only limit elements
    if let Some(limit_count) = limit {
        iter.take(limit_count).collect()
    } else {
        iter.collect()
    }
}
```

### 5. find_with_options() implementáció

```rust
pub fn find_with_options(
    &self,
    query_json: &Value,
    options: FindOptions
) -> Result<Vec<Value>> {
    // 1. Get matching documents (use existing find() logic)
    let mut docs = self.find(query_json)?;

    // 2. Apply sort
    if let Some(ref sort) = options.sort {
        apply_sort(&mut docs, sort);
    }

    // 3. Apply skip and limit
    docs = apply_limit_skip(docs, options.limit, options.skip);

    // 4. Apply projection
    if let Some(ref projection) = options.projection {
        docs = docs.into_iter()
            .map(|doc| apply_projection(&doc, projection))
            .collect();
    }

    Ok(docs)
}
```

### 6. Python Bindings

**Módosítás:** `bindings/python/src/lib.rs`

```rust
#[pymethods]
impl Collection {
    fn find(
        &self,
        query: &PyDict,
        projection: Option<&PyDict>,
        sort: Option<&PyList>,
        limit: Option<usize>,
        skip: Option<usize>,
    ) -> PyResult<PyObject> {
        // Convert projection
        let projection_map = if let Some(proj) = projection {
            let mut map = HashMap::new();
            for (key, value) in proj.iter() {
                let field: String = key.extract()?;
                let action: i32 = value.extract()?;
                map.insert(field, action);
            }
            Some(map)
        } else {
            None
        };

        // Convert sort
        let sort_vec = if let Some(sort_list) = sort {
            let mut vec = Vec::new();
            for item in sort_list.iter() {
                let tuple: &PyTuple = item.downcast()?;
                let field: String = tuple.get_item(0)?.extract()?;
                let direction: i32 = tuple.get_item(1)?.extract()?;
                vec.push((field, direction));
            }
            Some(vec)
        } else {
            None
        };

        // Create FindOptions
        let options = FindOptions {
            projection: projection_map,
            sort: sort_vec,
            limit,
            skip,
        };

        // Execute query
        let query_json = python_dict_to_json_value(query)?;
        let results = self.core.find_with_options(&query_json, options)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        // Convert to Python
        Python::with_gil(|py| {
            let py_list = PyList::empty(py);
            for doc in results {
                let py_dict = json_to_python_dict(py, &doc)?;
                py_list.append(py_dict)?;
            }
            Ok(py_list.into())
        })
    }
}
```

## Execution Order

**MongoDB-kompatibilis sorrend:**

1. **Query Filtering** - Matching documents
2. **Sort** - Order results
3. **Skip** - Skip N documents
4. **Limit** - Take M documents
5. **Projection** - Filter fields

**Miért ez a sorrend?**
- Sort before skip/limit: Hogy a pagination konzisztens legyen
- Projection last: Minimális memory használat (csak a final results-ot project-eljük)

## Testing

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_projection_include_mode() {
        let doc = json!({"name": "Alice", "age": 30, "city": "NYC", "_id": 1});
        let projection = HashMap::from([
            ("name".to_string(), 1),
            ("age".to_string(), 1),
        ]);

        let result = apply_projection(&doc, &projection);
        assert!(result.get("name").is_some());
        assert!(result.get("age").is_some());
        assert!(result.get("_id").is_some());  // Included by default
        assert!(result.get("city").is_none());
    }

    #[test]
    fn test_projection_exclude_id() {
        let doc = json!({"name": "Alice", "age": 30, "_id": 1});
        let projection = HashMap::from([
            ("name".to_string(), 1),
            ("_id".to_string(), 0),  // Explicit exclude
        ]);

        let result = apply_projection(&doc, &projection);
        assert!(result.get("name").is_some());
        assert!(result.get("_id").is_none());  // Excluded
    }

    #[test]
    fn test_sort_multi_field() {
        let mut docs = vec![
            json!({"name": "Bob", "age": 30}),
            json!({"name": "Alice", "age": 25}),
            json!({"name": "Carol", "age": 30}),
        ];

        let sort = vec![
            ("age".to_string(), 1),   // Age ascending
            ("name".to_string(), -1), // Name descending
        ];

        apply_sort(&mut docs, &sort);

        assert_eq!(docs[0].get("name").unwrap(), "Alice");  // age=25
        assert_eq!(docs[1].get("name").unwrap(), "Carol");  // age=30, name=C
        assert_eq!(docs[2].get("name").unwrap(), "Bob");    // age=30, name=B
    }

    #[test]
    fn test_limit_skip() {
        let docs = vec![
            json!({"n": 1}),
            json!({"n": 2}),
            json!({"n": 3}),
            json!({"n": 4}),
            json!({"n": 5}),
        ];

        let result = apply_limit_skip(docs, Some(2), Some(1));

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].get("n").unwrap(), 2);
        assert_eq!(result[1].get("n").unwrap(), 3);
    }
}
```

### Integration Tests (Python)

```python
# test_find_options.py

def test_projection():
    users.insert_many([
        {"name": "Alice", "age": 30, "city": "NYC"},
        {"name": "Bob", "age": 25, "city": "LA"},
    ])

    results = users.find({}, projection={"name": 1, "age": 1, "_id": 0})

    assert "name" in results[0]
    assert "age" in results[0]
    assert "_id" not in results[0]
    assert "city" not in results[0]

def test_sort():
    users.insert_many([
        {"name": "Bob", "age": 30},
        {"name": "Alice", "age": 25},
        {"name": "Carol", "age": 35},
    ])

    results = users.find({}, sort=[("age", 1)])

    assert results[0]["name"] == "Alice"
    assert results[1]["name"] == "Bob"
    assert results[2]["name"] == "Carol"

def test_pagination():
    for i in range(20):
        users.insert_one({"n": i})

    # Page 1
    page1 = users.find({}, sort=[("n", 1)], limit=5, skip=0)
    assert len(page1) == 5
    assert page1[0]["n"] == 0

    # Page 2
    page2 = users.find({}, sort=[("n", 1)], limit=5, skip=5)
    assert len(page2) == 5
    assert page2[0]["n"] == 5
```

## MongoDB Compatibility

| Feature | MongoLite | MongoDB |
|---------|-----------|---------|
| Projection include | ✅ | ✅ |
| Projection exclude | ✅ | ✅ |
| _id in include mode | ✅ | ✅ |
| Sort single field | ✅ | ✅ |
| Sort multi field | ✅ | ✅ |
| Limit | ✅ | ✅ |
| Skip | ✅ | ✅ |
| Computed fields | ❌ | ✅ |
| Nested field projection | ❌ (future) | ✅ |

## Performance Considerations

1. **Sort before limit**: Always sort the full result set before applying limit
   - Optimization opportunity: If using index for sort, can limit during scan

2. **Projection last**: Apply projection after all filtering to minimize work

3. **Memory usage**: Large result sets + sort can use significant memory
   - Future: Streaming results, external sort for large datasets

## Future Enhancements

1. **Nested field support**: `{"user.name": 1}`
2. **Computed projections**: `{"fullName": {"$concat": ["$firstName", "$lastName"]}}`
3. **Index-optimized sort**: Use B+ tree order when index matches sort
4. **Cursor API**: Return iterator instead of Vec for large results
