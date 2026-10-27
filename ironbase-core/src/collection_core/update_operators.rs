// collection_core/update_operators.rs
// MongoDB-style update operators implementation

use crate::document::Document;
use crate::error::{IronBaseError, Result};
use crate::value_utils::{values_equal, values_identical};
use serde_json::Value;

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
/// Returns whether the document was modified: an update that leaves it as it
/// was (`$set` to the current value, `$unset` of a missing field, ...) is not
/// a modification and is not written (MongoDB `nModified`).
pub fn apply_update_operators(document: &mut Document, update_json: &Value) -> Result<bool> {
    let mut was_modified = false;

    let Value::Object(ref update_ops) = update_json else {
        return Ok(false);
    };

    reject_id_modification(document, update_ops)?;

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
                return Err(IronBaseError::InvalidQuery(format!(
                    "Unsupported update operator: {}",
                    op
                )));
            }
        }
    }

    Ok(was_modified)
}

/// `_id` is immutable (MongoDB semantics). The operators edit only the field
/// map, while storage, the catalog and the `_id` index stay keyed by
/// `Document::id`, so changing `_id` gave two documents the same visible `_id`
/// and made `_id` lookups hit the wrong document (audit 2026-10-06 #17).
/// `$set` to the current value is a no-op and stays allowed.
fn reject_id_modification(
    document: &Document,
    update_ops: &serde_json::Map<String, Value>,
) -> Result<()> {
    for (op, fields) in update_ops {
        let Value::Object(field_values) = fields else {
            continue;
        };
        for (field, value) in field_values {
            if field != "_id" && !field.starts_with("_id.") {
                continue;
            }
            let unchanged = op == "$set"
                && field == "_id"
                && serde_json::to_value(&document.id).ok().as_ref() == Some(value);
            if !unchanged {
                return Err(IronBaseError::InvalidQuery(format!(
                    "Performing an update on the path '{}' would modify the immutable field '_id'",
                    field
                )));
            }
        }
    }
    Ok(())
}

/// A dotted path through a value that cannot hold a field (a scalar, or an
/// array stepped into by a name) cannot be created: MongoDB rejects the
/// update. `set_nested` replaced such a value with an object, silently
/// losing it (`{"o": 5}` + `$set {"o.x": 1}` gave `{"o": {"x": 1}}`).
fn reject_uncreatable_path(document: &Document, op: &str, field: &str) -> Result<()> {
    let mut parts = field.split('.');
    let Some(mut current) = parts.next().and_then(|first| document.get(first)) else {
        return Ok(());
    };
    for part in parts {
        current = match current {
            Value::Object(map) => match map.get(part) {
                Some(next) => next,
                None => return Ok(()),
            },
            Value::Array(arr) if part.parse::<usize>().is_ok() => {
                match arr.get(part.parse::<usize>().unwrap_or_default()) {
                    Some(next) => next,
                    None => return Ok(()),
                }
            }
            other => {
                return Err(IronBaseError::InvalidQuery(format!(
                    "{}: cannot create field '{}' of '{}' in {}",
                    op, part, field, other
                )))
            }
        };
    }
    Ok(())
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
        reject_uncreatable_path(document, "$set", field)?;
        if document
            .get(field)
            .is_some_and(|current| values_identical(current, value))
        {
            continue;
        }
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
        if !inc_value.is_number() {
            // BUG #4 fix: Return error for non-numeric increment value
            return Err(IronBaseError::InvalidQuery(format!(
                "$inc value must be numeric, got: {}",
                inc_value
            )));
        }
        reject_uncreatable_path(document, "$inc", field)?;

        // MongoDB: if field doesn't exist, treat it as 0
        let existing = document.get(field);
        let current = existing.cloned().unwrap_or(Value::from(0));

        // Try int first to preserve integer types
        let next = if let (Some(curr_int), Some(inc_int)) = (current.as_i64(), inc_value.as_i64()) {
            // An int64 overflow is rejected (MongoDB); it used to
            // saturate silently at i64::MAX / i64::MIN (BUG #2 fix)
            Value::from(curr_int.checked_add(inc_int).ok_or_else(|| {
                IronBaseError::InvalidQuery(format!(
                    "$inc on '{}' overflows: {} + {} does not fit in a 64-bit integer",
                    field, curr_int, inc_int
                ))
            })?)
        } else if let (Some(curr_num), Some(inc_num)) = (current.as_f64(), inc_value.as_f64()) {
            // JSON has no infinity: `Value::from(inf)` stored null
            let sum = curr_num + inc_num;
            if !sum.is_finite() {
                return Err(IronBaseError::InvalidQuery(format!(
                    "$inc on '{}' overflows: {} + {} is not a finite number",
                    field,
                    Value::from(curr_num),
                    Value::from(inc_num)
                )));
            }
            Value::from(sum)
        } else {
            // A non-numeric field: MongoDB rejects it (it was silently kept)
            return Err(IronBaseError::InvalidQuery(format!(
                "Cannot apply $inc to a value of non-numeric type: '{}' is {}",
                field, current
            )));
        };
        if existing.is_some_and(|current| values_identical(current, &next)) {
            continue;
        }
        document.set_nested(field, next);
        modified = true;
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
        modified |= document.remove_nested(field).is_some();
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
        reject_uncreatable_path(document, "$push", field)?;
        // Get or create array
        let mut array = get_array_field(document, field, "$push")?;
        let before = document.get(field).cloned();

        push_with_modifiers(&mut array, value);

        let array = Value::Array(array);
        if before.is_some_and(|before| values_identical(&before, &array)) {
            continue;
        }
        document.set_nested(field, array);
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
            // Filter out matching elements, propagating any errors from condition evaluation
            let mut filtered: Vec<Value> = Vec::with_capacity(arr.len());
            for item in arr {
                // Check if item matches condition - if error, propagate it
                let matches = value_matches_condition(item, condition)?;
                if !matches {
                    filtered.push(item.clone());
                }
            }

            if filtered.len() != arr.len() {
                document.set_nested(field, Value::Array(filtered));
                modified = true;
            }
        } else if document.get(field).is_some() {
            return Err(IronBaseError::InvalidQuery(format!(
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

        reject_uncreatable_path(document, "$addToSet", field)?;
        // Get or create array (a missing field becomes an array even when
        // nothing is added, as in MongoDB)
        let mut array = get_array_field(document, field, "$addToSet")?;
        let mut field_modified = document.get(field).is_none();

        // Add items if not already present; numbers compare by value, as in
        // `$pull` and queries (`2` is already in `[2.0]`)
        for item in items {
            if !array.iter().any(|existing| values_equal(existing, &item)) {
                array.push(item);
                field_modified = true;
            }
        }

        // Only update document if this field was actually modified
        if field_modified {
            document.set_nested(field, Value::Array(array));
            modified = true;
        }
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
                    return Err(IronBaseError::InvalidQuery(format!(
                        "$pop: value must be -1 or 1, got {:?}",
                        direction
                    )));
                }
            }

            document.set_nested(field, Value::Array(new_array));
        } else if document.get(field).is_some() {
            return Err(IronBaseError::InvalidQuery(format!(
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

/// Apply one `$push` value to `array`: `$each` items, `$position` and
/// `$slice` modifiers, or a plain single value.
fn push_with_modifiers(array: &mut Vec<Value>, value: &Value) {
    // Parse modifiers: $each, $position, $slice
    let (items, position, slice) = parse_push_modifiers(value);

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
        apply_slice(array, slice_val);
    }
}

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
        Some(_) => Err(IronBaseError::InvalidQuery(format!(
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

/// Check if an array element matches a `$pull` condition (MongoDB semantics,
/// shared with `$elemMatch` via `element_matches`):
///
/// - Scalar or array condition: equality (numbers by value):
///   `{"tags": "obsolete"}` removes "obsolete"
/// - Operator condition: applied to the element value:
///   `{"scores": {"$lt": 5}}` removes items < 5
/// - Document condition: a query on each document element, so
///   `{"items": {"name": "a"}}` removes `{"name": "a", "qty": 2}` too
///   (it used to require deep equality; audit 2026-10-07 follow-up)
///
/// # Errors
/// Returns an error for an unknown query operator, so a typo like `$gtt`
/// fails instead of being ignored.
pub fn value_matches_condition(value: &Value, condition: &Value) -> Result<bool> {
    match condition {
        Value::Object(conditions) => {
            crate::query::operators::element_matches(value, conditions, condition)
        }
        _ => Ok(values_equal(value, condition)),
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
        assert!(value_matches_condition(&json!(5), &json!(5)).unwrap());
        assert!(!value_matches_condition(&json!(5), &json!(6)).unwrap());

        // Query operators
        assert!(value_matches_condition(&json!(5), &json!({"$gt": 3})).unwrap());
        assert!(!value_matches_condition(&json!(5), &json!({"$gt": 10})).unwrap());
        assert!(value_matches_condition(&json!(5), &json!({"$in": [1, 5, 10]})).unwrap());
    }

    #[test]
    fn test_value_matches_condition_unknown_operator_error() {
        // Unknown operators should return an error, not silently fail
        let result = value_matches_condition(&json!(5), &json!({"$unknown": 5}));
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.to_lowercase().contains("unknown operator"),
            "Error should mention unknown operator: {}",
            err_msg
        );
        assert!(
            err_msg.contains("$unknown"),
            "Error should include the operator name: {}",
            err_msg
        );

        // Typo in operator name should also error
        let result2 = value_matches_condition(&json!(5), &json!({"$gtt": 3})); // typo: $gtt instead of $gt
        assert!(result2.is_err());

        // The full query operator set applies, as in MongoDB
        assert!(value_matches_condition(&json!({"a": 1}), &json!({"$exists": true})).unwrap());
        assert!(value_matches_condition(&json!("hello"), &json!({"$regex": "^hel"})).unwrap());
        assert!(!value_matches_condition(&json!("hello"), &json!({"$regex": "^x"})).unwrap());
    }

    #[test]
    fn test_pull_document_condition_is_a_query_on_each_element() {
        let mut doc = make_doc(json!({"items": [
            {"name": "a", "qty": 2},
            {"name": "b", "qty": 1},
            {"name": "a", "tags": ["x"]},
            "a",
            {"name": "c", "meta": {"color": "red"}}
        ]}));
        apply_update_operators(&mut doc, &json!({"$pull": {"items": {"name": "a"}}})).unwrap();
        assert_eq!(
            doc.get("items").unwrap(),
            &json!([{"name": "b", "qty": 1}, "a", {"name": "c", "meta": {"color": "red"}}])
        );
        // operators, dotted paths and logical operators inside
        apply_update_operators(
            &mut doc,
            &json!({"$pull": {"items": {"$or": [{"qty": {"$lt": 2}}, {"meta.color": "red"}]}}}),
        )
        .unwrap();
        assert_eq!(doc.get("items").unwrap(), &json!(["a"]));
        // scalars and arrays: equality
        let mut doc = make_doc(json!({"xs": [1, [1, 2], 2.0, {"v": 1}]}));
        apply_update_operators(&mut doc, &json!({"$pull": {"xs": [1, 2]}})).unwrap();
        apply_update_operators(&mut doc, &json!({"$pull": {"xs": 2}})).unwrap();
        assert_eq!(doc.get("xs").unwrap(), &json!([1, {"v": 1}]));
    }

    #[test]
    fn test_pull_with_unknown_operator_returns_error() {
        let mut doc = make_doc(json!({"scores": [1, 5, 10, 15]}));

        // Using unknown operator in $pull should fail
        let update = json!({"$pull": {"scores": {"$unknown": 5}}});
        let result = apply_update_operators(&mut doc, &update);
        assert!(result.is_err());

        // Document should be unchanged
        assert_eq!(doc.get("scores"), Some(&json!([1, 5, 10, 15])));
    }

    // Faults found by the update operator property (P4)

    #[test]
    fn test_path_through_a_scalar_is_rejected_not_overwritten() {
        for update in [
            json!({"$set": {"o.x": 1}}),
            json!({"$inc": {"o.x": 1}}),
            json!({"$push": {"o.x": 1}}),
            json!({"$addToSet": {"o.x": 1}}),
        ] {
            let mut doc = make_doc(json!({"o": 5}));
            assert!(
                apply_update_operators(&mut doc, &update).is_err(),
                "{update}"
            );
            assert_eq!(doc.get("o"), Some(&json!(5)), "{update}");
        }
        // Through an object, or a missing field, the path is created
        let mut doc = make_doc(json!({"o": {"y": 2}}));
        assert!(apply_update_operators(&mut doc, &json!({"$set": {"o.x": 1, "n.m": 3}})).unwrap());
        assert_eq!(doc.get("o"), Some(&json!({"y": 2, "x": 1})));
        assert_eq!(doc.get("n"), Some(&json!({"m": 3})));
    }

    #[test]
    fn test_inc_rejects_what_it_cannot_store() {
        for (doc, update) in [
            (json!({"a": 1e308}), json!({"$inc": {"a": 1e308}})), // stored null
            (json!({"a": "s"}), json!({"$inc": {"a": 1}})),       // silently kept
            (json!({"a": i64::MAX}), json!({"$inc": {"a": 1}})),  // saturated
            (json!({"a": i64::MIN}), json!({"$inc": {"a": -1}})),
        ] {
            let mut document = make_doc(doc.clone());
            assert!(
                apply_update_operators(&mut document, &update).is_err(),
                "{doc} {update}"
            );
            assert_eq!(document.get("a"), doc.get("a"));
        }
    }

    #[test]
    fn test_add_to_set_compares_numbers_by_value() {
        let mut doc = make_doc(json!({"a": [2.0]}));
        assert!(!apply_update_operators(&mut doc, &json!({"$addToSet": {"a": 2}})).unwrap());
        assert_eq!(doc.get("a"), Some(&json!([2.0])));
    }

    #[test]
    fn test_add_to_set_empty_each_creates_the_array() {
        let mut doc = make_doc(json!({}));
        let update = json!({"$addToSet": {"a": {"$each": []}}});
        assert!(apply_update_operators(&mut doc, &update).unwrap());
        assert_eq!(doc.get("a"), Some(&json!([])));
    }

    #[test]
    fn test_update_that_changes_nothing_is_not_a_modification() {
        let mut doc = make_doc(json!({"a": 1, "b": [1]}));
        for update in [
            json!({"$set": {"a": 1}}),
            json!({"$unset": {"missing": ""}}),
            json!({"$inc": {"a": 0}}),
            json!({"$push": {"b": {"$each": []}}}),
            json!({"$pull": {"b": 5}}),
            json!({"$pop": {"missing": 1}}),
        ] {
            assert!(
                !apply_update_operators(&mut doc, &update).unwrap(),
                "{update}"
            );
        }
        // A type change is a modification (stored differently)
        assert!(apply_update_operators(&mut doc, &json!({"$set": {"a": 1.0}})).unwrap());
    }
}
