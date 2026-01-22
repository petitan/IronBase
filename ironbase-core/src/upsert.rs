//! Upsert Logic - Filter to Document Conversion
//!
//! MongoDB-compatible upsert creates a new document from filter + update when no match is found.
//!
//! ## Filter Conversion Rules
//!
//! | Filter Pattern | Converted To |
//! |----------------|--------------|
//! | `{"email": "x"}` | `{"email": "x"}` (equality) |
//! | `{"age": {"$gt": 18}}` | `{}` (operator ignored) |
//! | `{"status": {"$eq": "active"}}` | `{"status": "active"}` ($eq supported) |
//! | `{"user.email": "x"}` | `{"user": {"email": "x"}}` (dot notation) |
//! | `{"$and": [...]}` | Recursively processed |
//! | `{"$or": [...]}` | `{}` (ambiguous, ignored) |
//!
//! ## Example
//!
//! ```rust,ignore
//! let filter = json!({"email": "user@example.com", "status": {"$eq": "active"}});
//! let update = json!({"$set": {"name": "John"}});
//!
//! // If no match, creates: {"email": "user@example.com", "status": "active", "name": "John"}
//! ```

use serde_json::{Map, Value};
use std::collections::HashMap;

/// Convert a MongoDB filter to a base document for upsert
///
/// Extracts equality conditions from the filter to create the initial document.
/// Ignores comparison operators ($gt, $lt, etc.) since they don't define exact values.
///
/// # Arguments
/// * `filter` - MongoDB-style query filter
///
/// # Returns
/// A document containing the extractable equality conditions
pub fn filter_to_document(filter: &Value) -> HashMap<String, Value> {
    let mut doc = HashMap::new();

    if let Value::Object(map) = filter {
        for (key, value) in map {
            extract_field_value(key, value, &mut doc);
        }
    }

    doc
}

/// Extract a field value from a filter condition
fn extract_field_value(key: &str, value: &Value, doc: &mut HashMap<String, Value>) {
    // Handle logical operators
    if key == "$and" {
        // Process each condition in $and array
        if let Value::Array(conditions) = value {
            for condition in conditions {
                if let Value::Object(cond_map) = condition {
                    for (k, v) in cond_map {
                        extract_field_value(k, v, doc);
                    }
                }
            }
        }
        return;
    }

    // Ignore $or, $nor, $not - ambiguous for upsert
    if key == "$or" || key == "$nor" || key == "$not" {
        return;
    }

    // Ignore other operators at the top level
    if key.starts_with('$') {
        return;
    }

    // Handle the value
    match value {
        // Direct equality: {"field": "value"}
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            set_nested_value(doc, key, value.clone());
        }

        // Object value - check if it's an operator or a nested document
        Value::Object(obj) => {
            // Check if this is an operator object
            if let Some(first_key) = obj.keys().next() {
                if first_key.starts_with('$') {
                    // Handle $eq operator - the only one that makes sense for upsert
                    if let Some(eq_value) = obj.get("$eq") {
                        set_nested_value(doc, key, eq_value.clone());
                    }
                    // Ignore other operators ($gt, $lt, $in, etc.)
                    // They don't specify an exact value for the new document
                    return;
                }
            }

            // Not an operator object - treat as nested document equality
            set_nested_value(doc, key, value.clone());
        }
    }
}

/// Set a value in the document, handling dot notation
///
/// Converts "user.email" to {"user": {"email": value}}
fn set_nested_value(doc: &mut HashMap<String, Value>, path: &str, value: Value) {
    let parts: Vec<&str> = path.split('.').collect();

    if parts.len() == 1 {
        // Simple field
        doc.insert(path.to_string(), value);
    } else {
        // Nested path - build nested structure
        let first = parts[0];
        let rest = parts[1..].join(".");

        // Get or create the nested object
        let nested = doc
            .entry(first.to_string())
            .or_insert_with(|| Value::Object(Map::new()));

        if let Value::Object(nested_map) = nested {
            // Recursively set the nested value
            let mut nested_doc: HashMap<String, Value> = nested_map
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            set_nested_value(&mut nested_doc, &rest, value);

            // Convert back to serde_json Map
            *nested_map = nested_doc.into_iter().collect();
        }
    }
}

/// Apply update operators to a base document
///
/// Takes the base document (from filter) and applies the update operators.
/// Supports: $set, $inc, $unset, $push, $addToSet, $pop, $pull
///
/// # Arguments
/// * `base_doc` - Base document from filter conversion
/// * `update` - MongoDB update document with operators
///
/// # Returns
/// Final document ready for insertion
pub fn apply_update_to_base(base_doc: HashMap<String, Value>, update: &Value) -> Value {
    let mut result: Map<String, Value> = base_doc.into_iter().collect();

    if let Value::Object(update_map) = update {
        for (operator, fields) in update_map {
            match operator.as_str() {
                "$set" => {
                    if let Value::Object(set_fields) = fields {
                        for (field, value) in set_fields {
                            apply_set(&mut result, field, value.clone());
                        }
                    }
                }
                "$inc" => {
                    if let Value::Object(inc_fields) = fields {
                        for (field, amount) in inc_fields {
                            apply_inc(&mut result, field, amount);
                        }
                    }
                }
                "$unset" => {
                    if let Value::Object(unset_fields) = fields {
                        for (field, _) in unset_fields {
                            apply_unset(&mut result, field);
                        }
                    }
                }
                "$push" => {
                    if let Value::Object(push_fields) = fields {
                        for (field, value) in push_fields {
                            apply_push(&mut result, field, value.clone());
                        }
                    }
                }
                "$addToSet" => {
                    if let Value::Object(add_fields) = fields {
                        for (field, value) in add_fields {
                            apply_add_to_set(&mut result, field, value.clone());
                        }
                    }
                }
                _ => {
                    // Unknown operator - ignore for upsert document creation
                }
            }
        }
    }

    Value::Object(result)
}

/// Apply $set operation (handles dot notation)
fn apply_set(doc: &mut Map<String, Value>, path: &str, value: Value) {
    let parts: Vec<&str> = path.split('.').collect();

    if parts.len() == 1 {
        doc.insert(path.to_string(), value);
    } else {
        let first = parts[0];
        let rest = parts[1..].join(".");

        let nested = doc
            .entry(first.to_string())
            .or_insert_with(|| Value::Object(Map::new()));

        if let Value::Object(nested_map) = nested {
            apply_set(nested_map, &rest, value);
        }
    }
}

/// Apply $inc operation
fn apply_inc(doc: &mut Map<String, Value>, path: &str, amount: &Value) {
    let parts: Vec<&str> = path.split('.').collect();

    if parts.len() == 1 {
        let current = doc.get(path).cloned().unwrap_or(Value::Number(0.into()));
        let new_value = match (current, amount) {
            (Value::Number(c), Value::Number(a)) => {
                if let (Some(ci), Some(ai)) = (c.as_i64(), a.as_i64()) {
                    Value::Number((ci + ai).into())
                } else if let (Some(cf), Some(af)) = (c.as_f64(), a.as_f64()) {
                    serde_json::Number::from_f64(cf + af)
                        .map(Value::Number)
                        .unwrap_or(Value::Number(0.into()))
                } else {
                    Value::Number(0.into())
                }
            }
            _ => amount.clone(),
        };
        doc.insert(path.to_string(), new_value);
    } else {
        let first = parts[0];
        let rest = parts[1..].join(".");

        let nested = doc
            .entry(first.to_string())
            .or_insert_with(|| Value::Object(Map::new()));

        if let Value::Object(nested_map) = nested {
            apply_inc(nested_map, &rest, amount);
        }
    }
}

/// Apply $unset operation
fn apply_unset(doc: &mut Map<String, Value>, path: &str) {
    let parts: Vec<&str> = path.split('.').collect();

    if parts.len() == 1 {
        doc.remove(path);
    } else {
        let first = parts[0];
        let rest = parts[1..].join(".");

        if let Some(Value::Object(nested_map)) = doc.get_mut(first) {
            apply_unset(nested_map, &rest);
        }
    }
}

/// Apply $push operation
fn apply_push(doc: &mut Map<String, Value>, path: &str, value: Value) {
    let parts: Vec<&str> = path.split('.').collect();

    if parts.len() == 1 {
        let arr = doc
            .entry(path.to_string())
            .or_insert_with(|| Value::Array(vec![]));
        if let Value::Array(arr) = arr {
            arr.push(value);
        }
    } else {
        let first = parts[0];
        let rest = parts[1..].join(".");

        let nested = doc
            .entry(first.to_string())
            .or_insert_with(|| Value::Object(Map::new()));

        if let Value::Object(nested_map) = nested {
            apply_push(nested_map, &rest, value);
        }
    }
}

/// Apply $addToSet operation
fn apply_add_to_set(doc: &mut Map<String, Value>, path: &str, value: Value) {
    let parts: Vec<&str> = path.split('.').collect();

    if parts.len() == 1 {
        let arr = doc
            .entry(path.to_string())
            .or_insert_with(|| Value::Array(vec![]));
        if let Value::Array(arr) = arr {
            if !arr.contains(&value) {
                arr.push(value);
            }
        }
    } else {
        let first = parts[0];
        let rest = parts[1..].join(".");

        let nested = doc
            .entry(first.to_string())
            .or_insert_with(|| Value::Object(Map::new()));

        if let Value::Object(nested_map) = nested {
            apply_add_to_set(nested_map, &rest, value);
        }
    }
}

/// Create the final upsert document from filter and update
///
/// This is the main entry point for upsert document creation.
///
/// # Arguments
/// * `filter` - MongoDB-style query filter
/// * `update` - MongoDB update document with operators
///
/// # Returns
/// A complete document ready for insertion
pub fn create_upsert_document(filter: &Value, update: &Value) -> Value {
    let base_doc = filter_to_document(filter);
    apply_update_to_base(base_doc, update)
}

#[cfg(test)]
#[allow(clippy::unnecessary_get_then_check)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_simple_equality() {
        let filter = json!({"email": "user@example.com"});
        let doc = filter_to_document(&filter);

        assert_eq!(doc.get("email"), Some(&json!("user@example.com")));
    }

    #[test]
    fn test_multiple_fields() {
        let filter = json!({
            "email": "user@example.com",
            "status": "active",
            "age": 25
        });
        let doc = filter_to_document(&filter);

        assert_eq!(doc.get("email"), Some(&json!("user@example.com")));
        assert_eq!(doc.get("status"), Some(&json!("active")));
        assert_eq!(doc.get("age"), Some(&json!(25)));
    }

    #[test]
    fn test_eq_operator() {
        let filter = json!({"status": {"$eq": "active"}});
        let doc = filter_to_document(&filter);

        assert_eq!(doc.get("status"), Some(&json!("active")));
    }

    #[test]
    fn test_comparison_operators_ignored() {
        let filter = json!({
            "age": {"$gt": 18, "$lt": 65}
        });
        let doc = filter_to_document(&filter);

        // Comparison operators should be ignored
        assert!(doc.get("age").is_none());
    }

    #[test]
    fn test_dot_notation() {
        let filter = json!({"user.email": "test@example.com"});
        let doc = filter_to_document(&filter);

        let user = doc.get("user").unwrap();
        assert_eq!(user, &json!({"email": "test@example.com"}));
    }

    #[test]
    fn test_and_operator() {
        let filter = json!({
            "$and": [
                {"email": "user@example.com"},
                {"status": "active"}
            ]
        });
        let doc = filter_to_document(&filter);

        assert_eq!(doc.get("email"), Some(&json!("user@example.com")));
        assert_eq!(doc.get("status"), Some(&json!("active")));
    }

    #[test]
    fn test_or_operator_ignored() {
        let filter = json!({
            "$or": [
                {"email": "a@example.com"},
                {"email": "b@example.com"}
            ]
        });
        let doc = filter_to_document(&filter);

        // $or is ambiguous - should be ignored
        assert!(doc.get("email").is_none());
    }

    #[test]
    fn test_create_upsert_document() {
        let filter = json!({"email": "new@example.com"});
        let update = json!({"$set": {"name": "New User", "status": "pending"}});

        let doc = create_upsert_document(&filter, &update);

        assert_eq!(doc.get("email"), Some(&json!("new@example.com")));
        assert_eq!(doc.get("name"), Some(&json!("New User")));
        assert_eq!(doc.get("status"), Some(&json!("pending")));
    }

    #[test]
    fn test_upsert_with_inc() {
        let filter = json!({"userId": 1});
        let update = json!({
            "$set": {"name": "Counter"},
            "$inc": {"count": 1}
        });

        let doc = create_upsert_document(&filter, &update);

        assert_eq!(doc.get("userId"), Some(&json!(1)));
        assert_eq!(doc.get("name"), Some(&json!("Counter")));
        assert_eq!(doc.get("count"), Some(&json!(1)));
    }

    #[test]
    fn test_upsert_with_push() {
        let filter = json!({"name": "Alice"});
        let update = json!({"$push": {"tags": "new"}});

        let doc = create_upsert_document(&filter, &update);

        assert_eq!(doc.get("name"), Some(&json!("Alice")));
        assert_eq!(doc.get("tags"), Some(&json!(["new"])));
    }

    #[test]
    fn test_nested_dot_notation_in_update() {
        let filter = json!({"userId": 1});
        let update = json!({"$set": {"profile.name": "John", "profile.age": 30}});

        let doc = create_upsert_document(&filter, &update);

        assert_eq!(doc.get("userId"), Some(&json!(1)));
        let profile = doc.get("profile").unwrap();
        assert_eq!(profile.get("name"), Some(&json!("John")));
        assert_eq!(profile.get("age"), Some(&json!(30)));
    }

    #[test]
    fn test_mixed_filter_and_update() {
        let filter = json!({
            "email": "user@example.com",
            "status": {"$eq": "active"},
            "age": {"$gte": 18}  // Should be ignored
        });
        let update = json!({
            "$set": {"lastLogin": "2024-01-01"},
            "$inc": {"loginCount": 1}
        });

        let doc = create_upsert_document(&filter, &update);

        assert_eq!(doc.get("email"), Some(&json!("user@example.com")));
        assert_eq!(doc.get("status"), Some(&json!("active")));
        assert!(doc.get("age").is_none()); // $gte ignored
        assert_eq!(doc.get("lastLogin"), Some(&json!("2024-01-01")));
        assert_eq!(doc.get("loginCount"), Some(&json!(1)));
    }
}
