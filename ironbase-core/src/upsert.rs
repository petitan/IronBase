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

use crate::collection_core::apply_update_operators;
use crate::document::{Document, DocumentId};
use crate::error::{IronBaseError, Result};
use serde::Deserialize;
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
/// Takes the base document (from filter) and applies the update operators
/// with the same code, and so the same semantics and errors, as an update of
/// an existing document (`apply_update_operators`). This module used to have
/// its own copy of every operator, which had drifted: `$inc` to infinity
/// stored 0, `$inc` on a string overwrote it, `$push` on a scalar and unknown
/// operators were ignored (property-based testing plan, P4).
///
/// An upsert may set the `_id` of the document it inserts (`$set: {_id: v}`),
/// unless the filter fixes a different one.
///
/// # Arguments
/// * `base_doc` - Base document from filter conversion
/// * `update` - MongoDB update document with operators
///
/// # Returns
/// Final document ready for insertion
///
/// # Errors
/// What the update of an existing document rejects (`InvalidQuery`), and an
/// `_id` that is not a valid document id.
pub fn apply_update_to_base(mut base_doc: HashMap<String, Value>, update: &Value) -> Result<Value> {
    if let Some(id) = update.get("$set").and_then(|set| set.get("_id")) {
        if base_doc.get("_id").is_some_and(|existing| existing != id) {
            return Err(IronBaseError::InvalidQuery(
                "Performing an update on the path '_id' would modify the immutable field '_id'"
                    .to_string(),
            ));
        }
        base_doc.insert("_id".to_string(), id.clone());
    }
    let has_id = base_doc.contains_key("_id");
    let id = match base_doc.get("_id") {
        Some(id) => DocumentId::deserialize(id)
            .map_err(|e| IronBaseError::InvalidQuery(format!("Invalid upsert _id: {}", e)))?,
        // Not stored: the insert assigns the id; `_id` stays out of the fields
        None => DocumentId::Int(0),
    };
    let mut document = Document::new(id, base_doc);
    apply_update_operators(&mut document, update)?;
    if !has_id {
        document.fields.remove("_id");
    }
    Ok(Value::Object(document.fields.into_iter().collect()))
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
///
/// # Errors
/// See [`apply_update_to_base`].
pub fn create_upsert_document(filter: &Value, update: &Value) -> Result<Value> {
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

        let user = doc
            .get("user")
            .expect("dot notation should create nested 'user' object");
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

        let doc = create_upsert_document(&filter, &update).unwrap();

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

        let doc = create_upsert_document(&filter, &update).unwrap();

        assert_eq!(doc.get("userId"), Some(&json!(1)));
        assert_eq!(doc.get("name"), Some(&json!("Counter")));
        assert_eq!(doc.get("count"), Some(&json!(1)));
    }

    #[test]
    fn test_upsert_with_push() {
        let filter = json!({"name": "Alice"});
        let update = json!({"$push": {"tags": "new"}});

        let doc = create_upsert_document(&filter, &update).unwrap();

        assert_eq!(doc.get("name"), Some(&json!("Alice")));
        assert_eq!(doc.get("tags"), Some(&json!(["new"])));
    }

    #[test]
    fn test_nested_dot_notation_in_update() {
        let filter = json!({"userId": 1});
        let update = json!({"$set": {"profile.name": "John", "profile.age": 30}});

        let doc = create_upsert_document(&filter, &update).unwrap();

        assert_eq!(doc.get("userId"), Some(&json!(1)));
        let profile = doc
            .get("profile")
            .expect("dot notation should create nested 'profile' object");
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

        let doc = create_upsert_document(&filter, &update).unwrap();

        assert_eq!(doc.get("email"), Some(&json!("user@example.com")));
        assert_eq!(doc.get("status"), Some(&json!("active")));
        assert!(doc.get("age").is_none()); // $gte ignored
        assert_eq!(doc.get("lastLogin"), Some(&json!("2024-01-01")));
        assert_eq!(doc.get("loginCount"), Some(&json!(1)));
    }

    #[test]
    fn test_upsert_with_pop() {
        // Test $pop on existing array from filter (no dependency on operator order)
        let filter = json!({"name": "Alice", "tags": ["a", "b", "c"]});
        let update = json!({
            "$pop": {"tags": 1}  // Remove last
        });

        let doc = create_upsert_document(&filter, &update).unwrap();

        assert_eq!(doc.get("name"), Some(&json!("Alice")));
        assert_eq!(doc.get("tags"), Some(&json!(["a", "b"])));
    }

    #[test]
    fn test_upsert_with_pop_first() {
        // Test $pop with -1 (remove first element)
        let filter = json!({"name": "Alice", "tags": ["a", "b", "c"]});
        let update = json!({
            "$pop": {"tags": -1}  // Remove first
        });

        let doc = create_upsert_document(&filter, &update).unwrap();

        assert_eq!(doc.get("tags"), Some(&json!(["b", "c"])));
    }

    #[test]
    fn test_upsert_with_pull() {
        // Test $pull removes all matching values
        let filter = json!({"name": "Alice", "tags": ["a", "b", "c", "b"]});
        let update = json!({
            "$pull": {"tags": "b"}  // Remove all "b"
        });

        let doc = create_upsert_document(&filter, &update).unwrap();

        assert_eq!(doc.get("tags"), Some(&json!(["a", "c"])));
    }

    #[test]
    fn test_inc_overflow_is_rejected() {
        // An int64 overflow is an error (MongoDB), not a panic or a
        // silently saturated value
        let filter = json!({"id": 1, "counter": i64::MAX - 5});
        let update = json!({
            "$inc": {"counter": 10}  // Would overflow
        });

        assert!(create_upsert_document(&filter, &update).is_err());
    }

    #[test]
    fn test_inc_underflow_is_rejected() {
        let filter = json!({"id": 1, "counter": i64::MIN + 5});
        let update = json!({
            "$inc": {"counter": -10}  // Would underflow
        });

        assert!(create_upsert_document(&filter, &update).is_err());
    }

    #[test]
    fn test_pop_on_empty_array() {
        // Test $pop on empty array does nothing
        let filter = json!({"tags": []});
        let update = json!({"$pop": {"tags": 1}});

        let doc = create_upsert_document(&filter, &update).unwrap();

        assert_eq!(doc.get("tags"), Some(&json!([])));
    }

    #[test]
    fn test_pull_nonexistent_value() {
        // Test $pull with value that doesn't exist
        let filter = json!({"tags": ["a", "b", "c"]});
        let update = json!({"$pull": {"tags": "x"}});

        let doc = create_upsert_document(&filter, &update).unwrap();

        assert_eq!(doc.get("tags"), Some(&json!(["a", "b", "c"])));
    }

    #[test]
    fn test_addtoset_object_key_order_independent() {
        // If array already contains {"a":1,"b":2}, adding {"b":2,"a":1} should NOT add duplicate
        let filter = json!({"items": [{"a": 1, "b": 2}]});
        let update = json!({"$addToSet": {"items": {"b": 2, "a": 1}}});

        let doc = create_upsert_document(&filter, &update).unwrap();

        // Should still have only one item (no duplicate added)
        let items = doc
            .get("items")
            .expect("'items' field should exist")
            .as_array()
            .expect("'items' should be an array");
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn test_addtoset_object_different_values_added() {
        // If array contains {"a":1,"b":2}, adding {"a":1,"b":3} SHOULD add (different value)
        let filter = json!({"items": [{"a": 1, "b": 2}]});
        let update = json!({"$addToSet": {"items": {"a": 1, "b": 3}}});

        let doc = create_upsert_document(&filter, &update).unwrap();

        let items = doc
            .get("items")
            .expect("'items' field should exist")
            .as_array()
            .expect("'items' should be an array");
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn test_addtoset_nested_object_key_order() {
        // Test with nested objects
        let filter = json!({"items": [{"user": {"name": "Alice", "age": 30}}]});
        let update = json!({"$addToSet": {"items": {"user": {"age": 30, "name": "Alice"}}}});

        let doc = create_upsert_document(&filter, &update).unwrap();

        let items = doc
            .get("items")
            .expect("'items' field should exist")
            .as_array()
            .expect("'items' should be an array");
        assert_eq!(
            items.len(),
            1,
            "Nested objects with same values but different key order should be treated as equal"
        );
    }
}
