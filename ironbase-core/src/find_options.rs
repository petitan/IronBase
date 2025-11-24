// ironbase-core/src/find_options.rs
// Find query options: projection, sort, limit, skip

use serde_json::Value;
use std::collections::HashMap;

/// Options for find queries
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

/// Apply projection to a document
pub fn apply_projection(doc: &Value, projection: &HashMap<String, i32>) -> Value {
    if projection.is_empty() {
        return doc.clone();
    }

    // Detect mode
    let has_inclusions = projection.values().any(|&v| v == 1);
    let has_non_id_exclusions = projection
        .iter()
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

/// Apply sort to documents
pub fn apply_sort(docs: &mut [Value], sort: &[(String, i32)]) {
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

/// Compare two JSON values for sorting
fn compare_values(a: Option<&Value>, b: Option<&Value>) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less, // null < any value
        (Some(_), None) => Ordering::Greater,

        (Some(Value::Number(n1)), Some(Value::Number(n2))) => {
            let f1 = n1.as_f64().unwrap_or(0.0);
            let f2 = n2.as_f64().unwrap_or(0.0);
            f1.partial_cmp(&f2).unwrap_or(Ordering::Equal)
        }

        (Some(Value::String(s1)), Some(Value::String(s2))) => s1.cmp(s2),

        (Some(Value::Bool(b1)), Some(Value::Bool(b2))) => b1.cmp(b2),

        // Type priority: null < number < string < bool < object < array
        (Some(a_val), Some(b_val)) => type_priority(a_val).cmp(&type_priority(b_val)),
    }
}

/// Get type priority for mixed-type sorting
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

/// Apply limit and skip to documents
pub fn apply_limit_skip(docs: Vec<Value>, limit: Option<usize>, skip: Option<usize>) -> Vec<Value> {
    let skip_count = skip.unwrap_or(0);

    if skip_count >= docs.len() {
        return Vec::new();
    }

    let start = skip_count;
    let end = if let Some(limit_count) = limit {
        (start + limit_count).min(docs.len())
    } else {
        docs.len()
    };

    docs[start..end].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_projection_include_mode() {
        let doc = json!({"name": "Alice", "age": 30, "city": "NYC", "_id": 1});
        let projection = HashMap::from([("name".to_string(), 1), ("age".to_string(), 1)]);

        let result = apply_projection(&doc, &projection);
        assert!(result.get("name").is_some());
        assert!(result.get("age").is_some());
        assert!(result.get("_id").is_some()); // Included by default
        assert!(result.get("city").is_none());
    }

    #[test]
    fn test_projection_exclude_id() {
        let doc = json!({"name": "Alice", "age": 30, "_id": 1});
        let projection = HashMap::from([
            ("name".to_string(), 1),
            ("_id".to_string(), 0), // Explicit exclude
        ]);

        let result = apply_projection(&doc, &projection);
        assert!(result.get("name").is_some());
        assert!(result.get("_id").is_none()); // Excluded
    }

    #[test]
    fn test_projection_exclude_mode() {
        let doc = json!({"name": "Alice", "age": 30, "city": "NYC", "_id": 1});
        let projection = HashMap::from([("city".to_string(), 0)]);

        let result = apply_projection(&doc, &projection);
        assert!(result.get("name").is_some());
        assert!(result.get("age").is_some());
        assert!(result.get("_id").is_some());
        assert!(result.get("city").is_none()); // Excluded
    }

    #[test]
    fn test_sort_single_field() {
        let mut docs = vec![json!({"age": 30}), json!({"age": 25}), json!({"age": 35})];

        let sort = vec![("age".to_string(), 1)]; // Ascending

        apply_sort(&mut docs, &sort);

        assert_eq!(docs[0].get("age").unwrap(), 25);
        assert_eq!(docs[1].get("age").unwrap(), 30);
        assert_eq!(docs[2].get("age").unwrap(), 35);
    }

    #[test]
    fn test_sort_descending() {
        let mut docs = vec![json!({"age": 30}), json!({"age": 25}), json!({"age": 35})];

        let sort = vec![("age".to_string(), -1)]; // Descending

        apply_sort(&mut docs, &sort);

        assert_eq!(docs[0].get("age").unwrap(), 35);
        assert_eq!(docs[1].get("age").unwrap(), 30);
        assert_eq!(docs[2].get("age").unwrap(), 25);
    }

    #[test]
    fn test_sort_multi_field() {
        let mut docs = vec![
            json!({"age": 30, "name": "Bob"}),
            json!({"age": 25, "name": "Alice"}),
            json!({"age": 30, "name": "Carol"}),
        ];

        let sort = vec![
            ("age".to_string(), 1),   // Age ascending
            ("name".to_string(), -1), // Name descending
        ];

        apply_sort(&mut docs, &sort);

        assert_eq!(docs[0].get("name").unwrap(), "Alice"); // age=25
        assert_eq!(docs[1].get("name").unwrap(), "Carol"); // age=30, name=C
        assert_eq!(docs[2].get("name").unwrap(), "Bob"); // age=30, name=B
    }

    #[test]
    fn test_sort_string() {
        let mut docs = vec![
            json!({"name": "Charlie"}),
            json!({"name": "Alice"}),
            json!({"name": "Bob"}),
        ];

        let sort = vec![("name".to_string(), 1)];

        apply_sort(&mut docs, &sort);

        assert_eq!(docs[0].get("name").unwrap(), "Alice");
        assert_eq!(docs[1].get("name").unwrap(), "Bob");
        assert_eq!(docs[2].get("name").unwrap(), "Charlie");
    }

    #[test]
    fn test_limit() {
        let docs = vec![
            json!({"n": 1}),
            json!({"n": 2}),
            json!({"n": 3}),
            json!({"n": 4}),
            json!({"n": 5}),
        ];

        let result = apply_limit_skip(docs, Some(3), None);

        assert_eq!(result.len(), 3);
        assert_eq!(result[0].get("n").unwrap(), 1);
        assert_eq!(result[1].get("n").unwrap(), 2);
        assert_eq!(result[2].get("n").unwrap(), 3);
    }

    #[test]
    fn test_skip() {
        let docs = vec![
            json!({"n": 1}),
            json!({"n": 2}),
            json!({"n": 3}),
            json!({"n": 4}),
            json!({"n": 5}),
        ];

        let result = apply_limit_skip(docs, None, Some(2));

        assert_eq!(result.len(), 3);
        assert_eq!(result[0].get("n").unwrap(), 3);
        assert_eq!(result[1].get("n").unwrap(), 4);
        assert_eq!(result[2].get("n").unwrap(), 5);
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

    #[test]
    fn test_skip_beyond_length() {
        let docs = vec![json!({"n": 1}), json!({"n": 2})];

        let result = apply_limit_skip(docs, None, Some(10));

        assert_eq!(result.len(), 0);
    }
}
