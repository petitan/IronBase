// collection_core/update_operators.rs
// MongoDB-style update operators implementation

use crate::document::Document;
use crate::error::{MongoLiteError, Result};
use serde_json::Value;
use std::cmp::Ordering;

/// Apply MongoDB-style update operators to a document
///
/// Supported operators:
/// - `$set`: Set field values (supports dot notation)
/// - `$inc`: Increment numeric values
/// - `$unset`: Remove fields
/// - `$push`: Add to array (supports $each, $position, $slice)
/// - `$pull`: Remove from array matching condition
/// - `$addToSet`: Add unique values to array
/// - `$pop`: Remove first (-1) or last (1) array element
///
/// Returns whether the document was modified.
pub fn apply_update_operators(document: &mut Document, update_json: &Value) -> Result<bool> {
    let mut was_modified = false;

    let Value::Object(ref update_ops) = update_json else {
        return Ok(false);
    };

    for (op, fields) in update_ops {
        match op.as_str() {
            "$set" => {
                was_modified |= apply_set(document, fields)?;
            }
            "$inc" => {
                was_modified |= apply_inc(document, fields)?;
            }
            "$unset" => {
                was_modified |= apply_unset(document, fields)?;
            }
            "$push" => {
                was_modified |= apply_push(document, fields)?;
            }
            "$pull" => {
                was_modified |= apply_pull(document, fields)?;
            }
            "$addToSet" => {
                was_modified |= apply_add_to_set(document, fields)?;
            }
            "$pop" => {
                was_modified |= apply_pop(document, fields)?;
            }
            _ => {
                return Err(MongoLiteError::InvalidQuery(format!(
                    "Unsupported update operator: {}",
                    op
                )));
            }
        }
    }

    Ok(was_modified)
}

// ============================================================================
// OPERATOR IMPLEMENTATIONS
// ============================================================================

/// $set: Set field values
fn apply_set(document: &mut Document, fields: &Value) -> Result<bool> {
    let Value::Object(ref field_values) = fields else {
        return Ok(false);
    };

    let mut modified = false;
    for (field, value) in field_values {
        document.set_nested(field, value.clone());
        modified = true;
    }
    Ok(modified)
}

/// $inc: Increment numeric values
fn apply_inc(document: &mut Document, fields: &Value) -> Result<bool> {
    let Value::Object(ref field_values) = fields else {
        return Ok(false);
    };

    let mut modified = false;
    for (field, inc_value) in field_values {
        // MongoDB: if field doesn't exist, treat it as 0
        let current = document.get(field).cloned().unwrap_or(Value::from(0));

        // Try int first to preserve integer types
        if let (Some(curr_int), Some(inc_int)) = (current.as_i64(), inc_value.as_i64()) {
            // Use saturating_add to prevent overflow (BUG #2 fix)
            document.set_nested(field, Value::from(curr_int.saturating_add(inc_int)));
            modified = true;
        } else if let (Some(curr_num), Some(inc_num)) = (current.as_f64(), inc_value.as_f64()) {
            document.set_nested(field, Value::from(curr_num + inc_num));
            modified = true;
        } else if !inc_value.is_number() {
            // BUG #4 fix: Return error for non-numeric increment value
            return Err(MongoLiteError::InvalidQuery(format!(
                "$inc value must be numeric, got: {}",
                inc_value
            )));
        }
    }
    Ok(modified)
}

/// $unset: Remove fields
fn apply_unset(document: &mut Document, fields: &Value) -> Result<bool> {
    let Value::Object(ref field_values) = fields else {
        return Ok(false);
    };

    let mut modified = false;
    for (field, _) in field_values {
        document.remove_nested(field);
        modified = true;
    }
    Ok(modified)
}

/// $push: Add to array (supports $each, $position, $slice modifiers)
fn apply_push(document: &mut Document, fields: &Value) -> Result<bool> {
    let Value::Object(ref field_values) = fields else {
        return Ok(false);
    };

    let mut modified = false;
    for (field, value) in field_values {
        // Parse modifiers: $each, $position, $slice
        let (items, position, slice) = parse_push_modifiers(value);

        // Get or create array
        let mut array = get_array_field(document, field, "$push")?;

        // Insert items at position or append
        if let Some(pos) = position {
            let insert_pos = pos.min(array.len());
            for (i, item) in items.into_iter().enumerate() {
                array.insert(insert_pos + i, item);
            }
        } else {
            array.extend(items);
        }

        // Apply $slice if specified
        if let Some(slice_val) = slice {
            apply_slice(&mut array, slice_val);
        }

        document.set_nested(field, Value::Array(array));
        modified = true;
    }
    Ok(modified)
}

/// $pull: Remove array elements matching condition
fn apply_pull(document: &mut Document, fields: &Value) -> Result<bool> {
    let Value::Object(ref field_values) = fields else {
        return Ok(false);
    };

    let mut modified = false;
    for (field, condition) in field_values {
        if let Some(Value::Array(ref arr)) = document.get(field) {
            // Filter out matching elements
            let filtered: Vec<Value> = arr
                .iter()
                .filter(|item| !value_matches_condition(item, condition))
                .cloned()
                .collect();

            if filtered.len() != arr.len() {
                document.set_nested(field, Value::Array(filtered));
                modified = true;
            }
        } else if document.get(field).is_some() {
            return Err(MongoLiteError::InvalidQuery(format!(
                "$pull: field '{}' is not an array",
                field
            )));
        }
    }
    Ok(modified)
}

/// $addToSet: Add unique values to array
fn apply_add_to_set(document: &mut Document, fields: &Value) -> Result<bool> {
    let Value::Object(ref field_values) = fields else {
        return Ok(false);
    };

    let mut modified = false;
    for (field, value) in field_values {
        // Handle $each modifier
        let items = parse_each_modifier(value);

        // Get or create array
        let mut array = get_array_field(document, field, "$addToSet")?;

        // Add items if not already present
        for item in items {
            if !array.contains(&item) {
                array.push(item);
                modified = true;
            }
        }

        document.set_nested(field, Value::Array(array));
    }
    Ok(modified)
}

/// $pop: Remove first (-1) or last (1) array element
fn apply_pop(document: &mut Document, fields: &Value) -> Result<bool> {
    let Value::Object(ref field_values) = fields else {
        return Ok(false);
    };

    let mut modified = false;
    for (field, direction) in field_values {
        if let Some(Value::Array(ref arr)) = document.get(field) {
            if arr.is_empty() {
                continue; // No-op on empty array
            }

            let mut new_array = arr.clone();

            // -1 = remove first, 1 = remove last
            match direction.as_i64() {
                Some(-1) => {
                    new_array.remove(0);
                    modified = true;
                }
                Some(1) => {
                    new_array.pop();
                    modified = true;
                }
                _ => {
                    return Err(MongoLiteError::InvalidQuery(format!(
                        "$pop: value must be -1 or 1, got {:?}",
                        direction
                    )));
                }
            }

            document.set_nested(field, Value::Array(new_array));
        } else if document.get(field).is_some() {
            return Err(MongoLiteError::InvalidQuery(format!(
                "$pop: field '{}' is not an array",
                field
            )));
        }
    }
    Ok(modified)
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Parse $push modifiers: $each, $position, $slice
fn parse_push_modifiers(value: &Value) -> (Vec<Value>, Option<usize>, Option<i64>) {
    if let Value::Object(ref modifiers) = value {
        let items = if let Some(each_val) = modifiers.get("$each") {
            if let Value::Array(ref arr) = each_val {
                arr.clone()
            } else {
                vec![each_val.clone()]
            }
        } else {
            // No $each, treat entire value as single item
            vec![value.clone()]
        };

        // BUG #5 fix: Validate position is non-negative before converting to usize
        let position = modifiers
            .get("$position")
            .and_then(|v| v.as_i64())
            .and_then(|p| if p >= 0 { Some(p as usize) } else { None });

        let slice = modifiers.get("$slice").and_then(|v| v.as_i64());

        (items, position, slice)
    } else {
        // Simple push: single value
        (vec![value.clone()], None, None)
    }
}

/// Parse $each modifier for $addToSet
fn parse_each_modifier(value: &Value) -> Vec<Value> {
    if let Value::Object(ref modifiers) = value {
        if let Some(each_val) = modifiers.get("$each") {
            if let Value::Array(ref arr) = each_val {
                return arr.clone();
            } else {
                return vec![each_val.clone()];
            }
        }
    }
    vec![value.clone()]
}

/// Get array field or create empty array if field doesn't exist
fn get_array_field(document: &Document, field: &str, op_name: &str) -> Result<Vec<Value>> {
    match document.get(field) {
        Some(Value::Array(arr)) => Ok(arr.clone()),
        Some(_) => Err(MongoLiteError::InvalidQuery(format!(
            "{}: field '{}' is not an array",
            op_name, field
        ))),
        None => Ok(vec![]),
    }
}

/// Apply $slice modifier to array
fn apply_slice(array: &mut Vec<Value>, slice_val: i64) {
    if slice_val < 0 {
        // Keep last N elements
        let keep = (-slice_val) as usize;
        let len = array.len();
        if len > keep {
            *array = array.drain(..).skip(len - keep).collect();
        }
    } else {
        // Keep first N elements
        array.truncate(slice_val as usize);
    }
}

/// Check if a value matches a condition (for $pull)
///
/// Supports:
/// - Direct equality: `{"tags": "obsolete"}` removes "obsolete"
/// - Query operators: `{"score": {"$lt": 5}}` removes items < 5
pub fn value_matches_condition(value: &Value, condition: &Value) -> bool {
    // If condition is an object with operators, evaluate them
    if let Value::Object(ref cond_obj) = condition {
        // Check if it contains query operators
        let has_operators = cond_obj.keys().any(|k| k.starts_with('$'));

        if has_operators {
            // Evaluate query operators
            for (op, op_value) in cond_obj {
                let matches = match op.as_str() {
                    "$eq" => value == op_value,
                    "$ne" => value != op_value,
                    "$gt" => compare_values(value, op_value)
                        .map(|cmp| cmp == Ordering::Greater)
                        .unwrap_or(false),
                    "$gte" => compare_values(value, op_value)
                        .map(|cmp| matches!(cmp, Ordering::Greater | Ordering::Equal))
                        .unwrap_or(false),
                    "$lt" => compare_values(value, op_value)
                        .map(|cmp| cmp == Ordering::Less)
                        .unwrap_or(false),
                    "$lte" => compare_values(value, op_value)
                        .map(|cmp| matches!(cmp, Ordering::Less | Ordering::Equal))
                        .unwrap_or(false),
                    "$in" => {
                        if let Value::Array(ref arr) = op_value {
                            arr.contains(value)
                        } else {
                            false
                        }
                    }
                    "$nin" => {
                        if let Value::Array(ref arr) = op_value {
                            !arr.contains(value)
                        } else {
                            true
                        }
                    }
                    _ => false, // Unknown operator: treat as no match (safe default)
                };

                if !matches {
                    return false;
                }
            }
            return true; // All operators matched
        }
    }

    // Direct equality comparison
    value == condition
}

/// Compare two JSON values for ordering
/// BUG #6 fix: Use integer comparison when both values are integers to preserve precision
fn compare_values(a: &Value, b: &Value) -> Option<Ordering> {
    match (a, b) {
        (Value::Number(n1), Value::Number(n2)) => {
            // Try integer comparison first to preserve precision for large integers
            if let (Some(i1), Some(i2)) = (n1.as_i64(), n2.as_i64()) {
                return Some(i1.cmp(&i2));
            }
            // Fall back to float comparison
            let f1 = n1.as_f64()?;
            let f2 = n2.as_f64()?;
            f1.partial_cmp(&f2)
        }
        (Value::String(s1), Value::String(s2)) => Some(s1.cmp(s2)),
        (Value::Bool(b1), Value::Bool(b2)) => Some(b1.cmp(b2)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    fn make_doc(fields: serde_json::Value) -> Document {
        let fields_map: HashMap<String, Value> = fields
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        Document::new(crate::document::DocumentId::Int(1), fields_map)
    }

    #[test]
    fn test_set_operator() {
        let mut doc = make_doc(json!({"name": "Alice", "age": 30}));
        let update = json!({"$set": {"age": 31, "city": "NYC"}});

        let modified = apply_update_operators(&mut doc, &update).unwrap();

        assert!(modified);
        assert_eq!(doc.get("age"), Some(&json!(31)));
        assert_eq!(doc.get("city"), Some(&json!("NYC")));
    }

    #[test]
    fn test_inc_operator() {
        let mut doc = make_doc(json!({"count": 5}));
        let update = json!({"$inc": {"count": 3}});

        let modified = apply_update_operators(&mut doc, &update).unwrap();

        assert!(modified);
        assert_eq!(doc.get("count"), Some(&json!(8)));
    }

    #[test]
    fn test_unset_operator() {
        let mut doc = make_doc(json!({"name": "Alice", "temp": "value"}));
        let update = json!({"$unset": {"temp": ""}});

        let modified = apply_update_operators(&mut doc, &update).unwrap();

        assert!(modified);
        assert_eq!(doc.get("temp"), None);
    }

    #[test]
    fn test_push_operator() {
        let mut doc = make_doc(json!({"tags": ["a", "b"]}));
        let update = json!({"$push": {"tags": "c"}});

        let modified = apply_update_operators(&mut doc, &update).unwrap();

        assert!(modified);
        assert_eq!(doc.get("tags"), Some(&json!(["a", "b", "c"])));
    }

    #[test]
    fn test_pull_operator() {
        let mut doc = make_doc(json!({"tags": ["a", "b", "c"]}));
        let update = json!({"$pull": {"tags": "b"}});

        let modified = apply_update_operators(&mut doc, &update).unwrap();

        assert!(modified);
        assert_eq!(doc.get("tags"), Some(&json!(["a", "c"])));
    }

    #[test]
    fn test_add_to_set_operator() {
        let mut doc = make_doc(json!({"tags": ["a", "b"]}));

        // Add existing value - no change
        let update1 = json!({"$addToSet": {"tags": "a"}});
        let modified1 = apply_update_operators(&mut doc, &update1).unwrap();
        assert!(!modified1);
        assert_eq!(doc.get("tags"), Some(&json!(["a", "b"])));

        // Add new value
        let update2 = json!({"$addToSet": {"tags": "c"}});
        let modified2 = apply_update_operators(&mut doc, &update2).unwrap();
        assert!(modified2);
        assert_eq!(doc.get("tags"), Some(&json!(["a", "b", "c"])));
    }

    #[test]
    fn test_pop_operator() {
        let mut doc = make_doc(json!({"arr": [1, 2, 3]}));

        // Pop last
        let update1 = json!({"$pop": {"arr": 1}});
        apply_update_operators(&mut doc, &update1).unwrap();
        assert_eq!(doc.get("arr"), Some(&json!([1, 2])));

        // Pop first
        let update2 = json!({"$pop": {"arr": -1}});
        apply_update_operators(&mut doc, &update2).unwrap();
        assert_eq!(doc.get("arr"), Some(&json!([2])));
    }

    #[test]
    fn test_value_matches_condition() {
        // Direct equality
        assert!(value_matches_condition(&json!(5), &json!(5)));
        assert!(!value_matches_condition(&json!(5), &json!(6)));

        // Query operators
        assert!(value_matches_condition(&json!(5), &json!({"$gt": 3})));
        assert!(!value_matches_condition(&json!(5), &json!({"$gt": 10})));
        assert!(value_matches_condition(
            &json!(5),
            &json!({"$in": [1, 5, 10]})
        ));
    }
}
