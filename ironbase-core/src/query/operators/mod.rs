// src/query/operators/mod.rs
//! Query operator trait definitions and implementations
//!
//! This module implements the Strategy pattern for MongoDB query operators.
//! Each operator is implemented as a separate type that implements the `OperatorMatcher` trait.
//!
//! # Architecture
//!
//! ```text
//! OperatorMatcher trait
//!     ↓
//! ┌────────────────┬────────────────┬────────────────┐
//! │ Comparison     │ Logical        │ Element        │
//! │ ($eq, $gt...)  │ ($and, $or...) │ ($exists...)   │
//! └────────────────┴────────────────┴────────────────┘
//! ```
//!
//! # Benefits
//!
//! - **Extensibility**: Add new operators without modifying existing code
//! - **Testability**: Each operator can be tested independently
//! - **Reduced Complexity**: Each operator has CC ~2-4 instead of one giant function
//! - **Type Safety**: Compile-time guarantees for operator implementations

mod array;
mod comparison;
mod element;
mod expression;
pub(crate) mod filter;
mod helpers;
mod logical;
mod text_search;
mod traits;

use lazy_static::lazy_static;
use std::collections::HashMap;

// Re-export public types
pub use array::{AllOperator, ElemMatchOperator, InOperator, NinOperator, SizeOperator};
pub use comparison::{EqOperator, GtOperator, GteOperator, LtOperator, LteOperator, NeOperator};
pub use element::{ExistsOperator, TypeOperator};
pub use expression::ExprOperator;
pub use filter::{matches_filter, matches_filter_value};
pub use logical::{AndOperator, NorOperator, NotOperator, OrOperator};
pub use text_search::{regex_match_with_options, FuzzyAlgorithm, FuzzyOperator, RegexOperator};
pub use traits::OperatorMatcher;

// ============================================================================
// OPERATOR REGISTRY
// ============================================================================

lazy_static! {
    /// Global registry of all query operators
    ///
    /// This registry allows dynamic dispatch to the appropriate operator implementation
    /// based on the operator name string (e.g., "$eq", "$gt").
    ///
    /// # Thread Safety
    ///
    /// The registry is initialized once at program startup and is immutable thereafter.
    /// All operator implementations are required to be `Send + Sync`.
    pub static ref OPERATOR_REGISTRY: HashMap<&'static str, Box<dyn OperatorMatcher>> = {
        let mut registry: HashMap<&'static str, Box<dyn OperatorMatcher>> = HashMap::new();

        // Comparison operators
        registry.insert("$eq", Box::new(EqOperator));
        registry.insert("$ne", Box::new(NeOperator));
        registry.insert("$gt", Box::new(GtOperator));
        registry.insert("$gte", Box::new(GteOperator));
        registry.insert("$lt", Box::new(LtOperator));
        registry.insert("$lte", Box::new(LteOperator));

        // Array operators
        registry.insert("$in", Box::new(InOperator));
        registry.insert("$nin", Box::new(NinOperator));
        registry.insert("$all", Box::new(AllOperator));
        registry.insert("$elemMatch", Box::new(ElemMatchOperator));
        registry.insert("$size", Box::new(SizeOperator));

        // Element operators
        registry.insert("$exists", Box::new(ExistsOperator));
        registry.insert("$type", Box::new(TypeOperator));

        // Regex operators
        registry.insert("$regex", Box::new(RegexOperator));

        // Fuzzy text search operators
        registry.insert("$fuzzy", Box::new(FuzzyOperator));

        // Logical operators
        registry.insert("$and", Box::new(AndOperator));
        registry.insert("$or", Box::new(OrOperator));
        registry.insert("$nor", Box::new(NorOperator));
        registry.insert("$not", Box::new(NotOperator));

        // Expression operators
        registry.insert("$expr", Box::new(ExprOperator));

        registry
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Document, DocumentId};
    use serde_json::json;
    use std::collections::HashMap as StdHashMap;

    fn create_test_document(id: i64, fields: Vec<(&str, serde_json::Value)>) -> Document {
        let mut field_map = StdHashMap::new();
        for (k, v) in fields {
            field_map.insert(k.to_string(), v);
        }
        Document::new(DocumentId::Int(id), field_map)
    }

    // ========== Additional comparison operator tests ==========

    #[test]
    fn test_gte_operator() {
        let op = GteOperator;
        assert!(op.matches(Some(&json!(10)), &json!(5), None).unwrap());
        assert!(op.matches(Some(&json!(5)), &json!(5), None).unwrap()); // Equal
        assert!(!op.matches(Some(&json!(3)), &json!(5), None).unwrap());
        assert!(!op.matches(None, &json!(5), None).unwrap()); // Missing field
    }

    #[test]
    fn test_lt_operator() {
        let op = LtOperator;
        assert!(op.matches(Some(&json!(3)), &json!(5), None).unwrap());
        assert!(!op.matches(Some(&json!(5)), &json!(5), None).unwrap()); // Equal
        assert!(!op.matches(Some(&json!(10)), &json!(5), None).unwrap());
        assert!(!op.matches(None, &json!(5), None).unwrap()); // Missing field
    }

    #[test]
    fn test_lte_operator() {
        let op = LteOperator;
        assert!(op.matches(Some(&json!(3)), &json!(5), None).unwrap());
        assert!(op.matches(Some(&json!(5)), &json!(5), None).unwrap()); // Equal
        assert!(!op.matches(Some(&json!(10)), &json!(5), None).unwrap());
        assert!(!op.matches(None, &json!(5), None).unwrap()); // Missing field
    }

    #[test]
    fn test_gt_missing_field() {
        let op = GtOperator;
        assert!(!op.matches(None, &json!(5), None).unwrap());
    }

    #[test]
    fn test_comparison_strings() {
        let op = GtOperator;
        assert!(op.matches(Some(&json!("b")), &json!("a"), None).unwrap());
        assert!(!op.matches(Some(&json!("a")), &json!("b"), None).unwrap());
    }

    #[test]
    fn test_comparison_booleans() {
        let op = GtOperator;
        assert!(op.matches(Some(&json!(true)), &json!(false), None).unwrap());
        assert!(!op.matches(Some(&json!(false)), &json!(true), None).unwrap());
    }

    #[test]
    fn test_comparison_incompatible_types() {
        let op = GtOperator;
        // String vs number - incompatible
        assert!(!op.matches(Some(&json!("10")), &json!(5), None).unwrap());
    }

    // ========== Array operator tests ==========

    #[test]
    fn test_nin_operator() {
        let op = NinOperator;
        let array = json!(["NYC", "LA", "SF"]);
        assert!(op.matches(Some(&json!("Chicago")), &array, None).unwrap());
        assert!(!op.matches(Some(&json!("NYC")), &array, None).unwrap());
        assert!(op.matches(None, &array, None).unwrap()); // Missing field returns true
    }

    #[test]
    fn test_in_missing_field() {
        let op = InOperator;
        let array = json!(["NYC", "LA"]);
        assert!(!op.matches(None, &array, None).unwrap());
    }

    #[test]
    fn test_in_not_array_error() {
        let op = InOperator;
        let result = op.matches(Some(&json!("NYC")), &json!("not an array"), None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("requires an array"));
    }

    #[test]
    fn test_nin_not_array_error() {
        let op = NinOperator;
        let result = op.matches(Some(&json!("NYC")), &json!("not an array"), None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("requires an array"));
    }

    #[test]
    fn test_all_missing_field() {
        let op = AllOperator;
        assert!(!op.matches(None, &json!(["a"]), None).unwrap());
    }

    #[test]
    fn test_all_not_array_doc() {
        let op = AllOperator;
        // Doc value is not an array
        assert!(!op
            .matches(Some(&json!("not an array")), &json!(["a"]), None)
            .unwrap());
    }

    #[test]
    fn test_all_not_array_filter_error() {
        let op = AllOperator;
        let result = op.matches(Some(&json!(["a", "b"])), &json!("not an array"), None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("requires an array"));
    }

    // ========== Element operator tests ==========

    #[test]
    fn test_exists_not_boolean_error() {
        let op = ExistsOperator;
        let result = op.matches(Some(&json!("value")), &json!("not a boolean"), None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("requires a boolean"));
    }

    #[test]
    fn test_regex_missing_field() {
        let op = RegexOperator;
        assert!(!op.matches(None, &json!("pattern"), None).unwrap());
    }

    #[test]
    fn test_regex_not_string_doc() {
        let op = RegexOperator;
        // Doc value is not a string
        assert!(!op
            .matches(Some(&json!(123)), &json!("pattern"), None)
            .unwrap());
    }

    #[test]
    fn test_regex_not_string_filter_error() {
        let op = RegexOperator;
        let result = op.matches(Some(&json!("hello")), &json!(123), None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("requires a string pattern"));
    }

    #[test]
    fn test_type_bson_numbers() {
        let op = TypeOperator;
        // BSON type 1 = double
        assert!(op.matches(Some(&json!(1.5)), &json!(1), None).unwrap());
        // BSON type 2 = string
        assert!(op.matches(Some(&json!("hello")), &json!(2), None).unwrap());
        // BSON type 3 = object
        assert!(op.matches(Some(&json!({"a": 1})), &json!(3), None).unwrap());
        // BSON type 4 = array
        assert!(op.matches(Some(&json!([1, 2])), &json!(4), None).unwrap());
        // BSON type 8 = bool
        assert!(op.matches(Some(&json!(true)), &json!(8), None).unwrap());
        // BSON type 10 = null
        assert!(op.matches(Some(&json!(null)), &json!(10), None).unwrap());
        // BSON type 16 = int
        assert!(op.matches(Some(&json!(42)), &json!(16), None).unwrap());
        // BSON type 18 = long
        assert!(op
            .matches(Some(&json!(9223372036854775807_i64)), &json!(18), None)
            .unwrap());
    }

    #[test]
    fn test_type_unknown_bson_number() {
        let op = TypeOperator;
        let result = op.matches(Some(&json!("hello")), &json!(999), None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Unknown BSON type number"));
    }

    #[test]
    fn test_type_unknown_type_name() {
        let op = TypeOperator;
        let result = op.matches(Some(&json!("hello")), &json!("unknown_type"), None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Unknown type name"));
    }

    #[test]
    fn test_type_invalid_filter_error() {
        let op = TypeOperator;
        let result = op.matches(Some(&json!("hello")), &json!([1, 2]), None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("requires a string or number"));
    }

    #[test]
    fn test_type_missing_field() {
        let op = TypeOperator;
        assert!(!op.matches(None, &json!("string"), None).unwrap());
    }

    #[test]
    fn test_type_boolean_alias() {
        let op = TypeOperator;
        assert!(op
            .matches(Some(&json!(true)), &json!("boolean"), None)
            .unwrap());
        assert!(op
            .matches(Some(&json!(false)), &json!("bool"), None)
            .unwrap());
    }

    #[test]
    fn test_type_int_long() {
        let op = TypeOperator;
        assert!(op.matches(Some(&json!(42)), &json!("int"), None).unwrap());
        assert!(op.matches(Some(&json!(42)), &json!("long"), None).unwrap());
    }

    #[test]
    fn test_type_double_vs_int_distinction() {
        // BUG #3 regression test: $type must distinguish between int and double
        // serde_json stores 42 as PosInt/NegInt, 42.0 as Float
        let op = TypeOperator;

        // Integer stored as int (json!(42))
        let int_val = json!(42);
        // Float stored as float (json!(42.5) - has fractional part)
        let float_val = json!(42.5);
        // Float stored as float (json!(42.0) - no fractional part but stored as Float)
        let float_whole = serde_json::Number::from_f64(42.0).unwrap();
        let float_whole_val = serde_json::Value::Number(float_whole);

        // $type: "double" should match floats, NOT integers
        assert!(
            !op.matches(Some(&int_val), &json!("double"), None).unwrap(),
            "Integer 42 should NOT match $type: 'double'"
        );
        assert!(
            op.matches(Some(&float_val), &json!("double"), None)
                .unwrap(),
            "Float 42.5 should match $type: 'double'"
        );
        assert!(
            op.matches(Some(&float_whole_val), &json!("double"), None)
                .unwrap(),
            "Float 42.0 should match $type: 'double'"
        );

        // $type: "int" should match integers, NOT floats
        assert!(
            op.matches(Some(&int_val), &json!("int"), None).unwrap(),
            "Integer 42 should match $type: 'int'"
        );
        assert!(
            !op.matches(Some(&float_val), &json!("int"), None).unwrap(),
            "Float 42.5 should NOT match $type: 'int'"
        );
        assert!(
            !op.matches(Some(&float_whole_val), &json!("int"), None)
                .unwrap(),
            "Float 42.0 should NOT match $type: 'int'"
        );

        // $type: "number" should match ALL numbers
        assert!(
            op.matches(Some(&int_val), &json!("number"), None).unwrap(),
            "Integer 42 should match $type: 'number'"
        );
        assert!(
            op.matches(Some(&float_val), &json!("number"), None)
                .unwrap(),
            "Float 42.5 should match $type: 'number'"
        );
    }

    #[test]
    fn test_type_bson_double_vs_int32_distinction() {
        // BUG #3 regression test: BSON type numbers must also distinguish
        let op = TypeOperator;

        let int_val = json!(42);
        let float_val = json!(42.5);

        // BSON type 1 = double: should NOT match integers
        assert!(
            !op.matches(Some(&int_val), &json!(1), None).unwrap(),
            "Integer 42 should NOT match BSON type 1 (double)"
        );
        assert!(
            op.matches(Some(&float_val), &json!(1), None).unwrap(),
            "Float 42.5 should match BSON type 1 (double)"
        );

        // BSON type 16 = int32: should NOT match floats
        assert!(
            op.matches(Some(&int_val), &json!(16), None).unwrap(),
            "Integer 42 should match BSON type 16 (int32)"
        );
        assert!(
            !op.matches(Some(&float_val), &json!(16), None).unwrap(),
            "Float 42.5 should NOT match BSON type 16 (int32)"
        );
    }

    #[test]
    fn test_type_int32_range() {
        // int32 has range -2147483648 to 2147483647
        let op = TypeOperator;

        // Within i32 range
        assert!(op
            .matches(Some(&json!(2147483647)), &json!("int"), None)
            .unwrap());
        assert!(op
            .matches(Some(&json!(-2147483648_i64)), &json!("int"), None)
            .unwrap());

        // Outside i32 range - should NOT match "int" but should match "long"
        let large_int = json!(2147483648_i64);
        assert!(
            !op.matches(Some(&large_int), &json!("int"), None).unwrap(),
            "2147483648 exceeds i32 max, should NOT match 'int'"
        );
        assert!(
            op.matches(Some(&large_int), &json!("long"), None).unwrap(),
            "2147483648 should match 'long'"
        );
    }

    // ========== Logical operator tests ==========

    #[test]
    fn test_nor_operator() {
        let doc = create_test_document(1, vec![("age", json!(25))]);
        // age is not < 18 AND age is not > 65, so $nor should return true
        let filter = json!([{"age": {"$lt": 18}}, {"age": {"$gt": 65}}]);
        let op = NorOperator;
        assert!(op.matches(None, &filter, Some(&doc)).unwrap());
    }

    #[test]
    fn test_nor_operator_fails() {
        let doc = create_test_document(1, vec![("age", json!(15))]);
        // age < 18 is TRUE, so $nor should return false
        let filter = json!([{"age": {"$lt": 18}}, {"age": {"$gt": 65}}]);
        let op = NorOperator;
        assert!(!op.matches(None, &filter, Some(&doc)).unwrap());
    }

    #[test]
    fn test_nor_not_array_error() {
        let doc = create_test_document(1, vec![("age", json!(25))]);
        let op = NorOperator;
        let result = op.matches(None, &json!({"age": 25}), Some(&doc));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("requires an array"));
    }

    #[test]
    fn test_nor_no_document_error() {
        let op = NorOperator;
        let result = op.matches(None, &json!([{"age": 25}]), None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("requires document context"));
    }

    #[test]
    fn test_and_not_array_error() {
        let doc = create_test_document(1, vec![("age", json!(25))]);
        let op = AndOperator;
        let result = op.matches(None, &json!({"age": 25}), Some(&doc));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("requires an array"));
    }

    #[test]
    fn test_and_no_document_error() {
        let op = AndOperator;
        let result = op.matches(None, &json!([{"age": 25}]), None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("requires document context"));
    }

    #[test]
    fn test_or_not_array_error() {
        let doc = create_test_document(1, vec![("age", json!(25))]);
        let op = OrOperator;
        let result = op.matches(None, &json!({"age": 25}), Some(&doc));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("requires an array"));
    }

    #[test]
    fn test_or_no_document_error() {
        let op = OrOperator;
        let result = op.matches(None, &json!([{"age": 25}]), None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("requires document context"));
    }

    #[test]
    fn test_or_no_match() {
        let doc = create_test_document(1, vec![("age", json!(30))]);
        let filter = json!([{"age": {"$lt": 18}}, {"age": {"$gt": 65}}]);
        let op = OrOperator;
        assert!(!op.matches(None, &filter, Some(&doc)).unwrap());
    }

    #[test]
    fn test_and_fails() {
        let doc = create_test_document(1, vec![("age", json!(25)), ("city", json!("LA"))]);
        let filter = json!([{"age": {"$gt": 18}}, {"city": "NYC"}]); // city doesn't match
        let op = AndOperator;
        assert!(!op.matches(None, &filter, Some(&doc)).unwrap());
    }

    #[test]
    fn test_not_operator() {
        let doc = create_test_document(1, vec![("age", json!(25))]);
        let op = NotOperator;
        // $not: { $gt: 30 } should return true for age=25
        let filter = json!({"$gt": 30});
        assert!(op.matches(Some(&json!(25)), &filter, Some(&doc)).unwrap());
    }

    #[test]
    fn test_not_no_document_error() {
        let op = NotOperator;
        let result = op.matches(Some(&json!(25)), &json!({"$gt": 30}), None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("requires document context"));
    }

    // ========== matches_filter tests ==========

    #[test]
    fn test_matches_filter_empty() {
        let doc = create_test_document(1, vec![("name", json!("Alice"))]);
        let filter = json!({});
        assert!(matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_matches_filter_unknown_operator() {
        let doc = create_test_document(1, vec![("age", json!(25))]);
        let filter = json!({"age": {"$unknown": 25}});
        let result = matches_filter(&doc, &filter);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown operator"));
    }

    #[test]
    fn test_matches_filter_top_level_unknown_operator() {
        let doc = create_test_document(1, vec![("age", json!(25))]);
        let filter = json!({"$unknown": [{"age": 25}]});
        let result = matches_filter(&doc, &filter);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown operator"));
    }

    #[test]
    fn test_matches_filter_not_object_error() {
        let doc = create_test_document(1, vec![("name", json!("Alice"))]);
        let filter = json!("not an object");
        let result = matches_filter(&doc, &filter);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Filter must be an object"));
    }

    #[test]
    fn test_matches_filter_direct_mismatch() {
        let doc = create_test_document(1, vec![("name", json!("Alice"))]);
        let filter = json!({"name": "Bob"});
        assert!(!matches_filter(&doc, &filter).unwrap());
    }

    // ========== $elemMatch tests ==========

    #[test]
    fn test_elemmatch_operator() {
        let op = ElemMatchOperator;
        let doc_value = json!([
            {"name": "Alice", "age": 25},
            {"name": "Bob", "age": 30}
        ]);
        let filter_value = json!({"name": "Alice", "age": {"$gte": 20}});
        assert!(op.matches(Some(&doc_value), &filter_value, None).unwrap());
    }

    #[test]
    fn test_elemmatch_no_match() {
        let op = ElemMatchOperator;
        let doc_value = json!([
            {"name": "Alice", "age": 15},
            {"name": "Bob", "age": 18}
        ]);
        let filter_value = json!({"name": "Alice", "age": {"$gte": 20}});
        assert!(!op.matches(Some(&doc_value), &filter_value, None).unwrap());
    }

    #[test]
    fn test_elemmatch_missing_field() {
        let op = ElemMatchOperator;
        assert!(!op.matches(None, &json!({"name": "Alice"}), None).unwrap());
    }

    #[test]
    fn test_elemmatch_not_array() {
        let op = ElemMatchOperator;
        assert!(!op
            .matches(
                Some(&json!("not an array")),
                &json!({"name": "Alice"}),
                None
            )
            .unwrap());
    }

    #[test]
    fn test_elemmatch_non_object_elements() {
        let op = ElemMatchOperator;
        let doc_value = json!([1, 2, 3]); // Array of non-objects
        let filter_value = json!({"name": "Alice"}); // Field-based query doesn't work on scalars
        assert!(!op.matches(Some(&doc_value), &filter_value, None).unwrap());
    }

    #[test]
    fn test_elemmatch_scalar_array_with_operators() {
        // MongoDB: {scores: {$elemMatch: {$gt: 80, $lt: 85}}} matches [75, 82, 90]
        // because 82 satisfies BOTH conditions
        let op = ElemMatchOperator;
        let doc_value = json!([75, 82, 90]);
        let filter_value = json!({"$gt": 80, "$lt": 85});
        assert!(op.matches(Some(&doc_value), &filter_value, None).unwrap());
    }

    #[test]
    fn test_elemmatch_scalar_array_no_match() {
        // No element in [75, 90, 95] satisfies BOTH $gt:80 AND $lt:85
        let op = ElemMatchOperator;
        let doc_value = json!([75, 90, 95]);
        let filter_value = json!({"$gt": 80, "$lt": 85});
        assert!(!op.matches(Some(&doc_value), &filter_value, None).unwrap());
    }

    #[test]
    fn test_elemmatch_scalar_array_single_condition() {
        // {$elemMatch: {$gte: 5}} on [3, 5, 7] - 5 and 7 both match
        let op = ElemMatchOperator;
        let doc_value = json!([3, 5, 7]);
        let filter_value = json!({"$gte": 5});
        assert!(op.matches(Some(&doc_value), &filter_value, None).unwrap());
    }

    #[test]
    fn test_elemmatch_scalar_string_array() {
        // String array with $regex
        let op = ElemMatchOperator;
        let doc_value = json!(["apple", "banana", "cherry"]);
        let filter_value = json!({"$regex": "^b"});
        assert!(op.matches(Some(&doc_value), &filter_value, None).unwrap());
    }

    #[test]
    fn test_elemmatch_scalar_array_in_operator() {
        // {$elemMatch: {$in: [2, 4, 6]}} on [1, 3, 4] - 4 is in the list
        let op = ElemMatchOperator;
        let doc_value = json!([1, 3, 4]);
        let filter_value = json!({"$in": [2, 4, 6]});
        assert!(op.matches(Some(&doc_value), &filter_value, None).unwrap());
    }

    #[test]
    fn test_elemmatch_invalid_filter_value() {
        // $elemMatch with non-object filter should return error
        let op = ElemMatchOperator;
        let doc_value = json!([1, 2, 3]);
        let filter_value = json!(5); // Invalid: not an object
        let result = op.matches(Some(&doc_value), &filter_value, None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("$elemMatch requires an object"));
    }

    #[test]
    fn test_elemmatch_unknown_operator() {
        // $elemMatch with unknown operator should return error
        let op = ElemMatchOperator;
        let doc_value = json!([{"score": 85}]);
        let filter_value = json!({"score": {"$gtt": 80}}); // $gtt is typo
        let result = op.matches(Some(&doc_value), &filter_value, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown operator"));
    }

    #[test]
    fn test_elemmatch_options_without_regex() {
        // $options without $regex should return error
        let op = ElemMatchOperator;
        let doc_value = json!([{"tag": "rust"}]);
        let filter_value = json!({"tag": {"$options": "i"}});
        let result = op.matches(Some(&doc_value), &filter_value, None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("$options requires $regex"));
    }

    #[test]
    fn test_elemmatch_regex_with_options() {
        // $regex + $options should work together
        let op = ElemMatchOperator;
        let doc_value = json!([{"tag": "RUST"}]);
        let filter_value = json!({"tag": {"$regex": "rust", "$options": "i"}});
        assert!(op.matches(Some(&doc_value), &filter_value, None).unwrap());
    }

    // ========== matches_filter_value tests ==========

    #[test]
    fn test_matches_filter_value_unknown_operator() {
        let result = matches_filter_value(Some(&json!(25)), &json!({"$unknown": 25}), None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown operator"));
    }

    #[test]
    fn test_matches_filter_value_direct() {
        assert!(matches_filter_value(Some(&json!(25)), &json!(25), None).unwrap());
        assert!(!matches_filter_value(Some(&json!(25)), &json!(30), None).unwrap());
        assert!(!matches_filter_value(None, &json!(25), None).unwrap());
    }

    // ========== Existing tests ==========

    #[test]
    fn test_eq_operator() {
        let op = EqOperator;
        assert!(op
            .matches(Some(&json!("Alice")), &json!("Alice"), None)
            .unwrap());
        assert!(!op
            .matches(Some(&json!("Bob")), &json!("Alice"), None)
            .unwrap());
        assert!(!op.matches(None, &json!("Alice"), None).unwrap());
    }

    #[test]
    fn test_ne_operator() {
        let op = NeOperator;
        assert!(op
            .matches(Some(&json!("Bob")), &json!("Alice"), None)
            .unwrap());
        assert!(!op
            .matches(Some(&json!("Alice")), &json!("Alice"), None)
            .unwrap());
        assert!(op.matches(None, &json!("Alice"), None).unwrap()); // Missing field != value
    }

    #[test]
    fn test_gt_operator() {
        let op = GtOperator;
        assert!(op.matches(Some(&json!(10)), &json!(5), None).unwrap());
        assert!(!op.matches(Some(&json!(5)), &json!(10), None).unwrap());
        assert!(!op.matches(Some(&json!(5)), &json!(5), None).unwrap());
    }

    #[test]
    fn test_in_operator() {
        let op = InOperator;
        let array = json!(["NYC", "LA", "SF"]);
        assert!(op.matches(Some(&json!("NYC")), &array, None).unwrap());
        assert!(!op.matches(Some(&json!("Chicago")), &array, None).unwrap());
    }

    #[test]
    fn test_exists_operator() {
        let op = ExistsOperator;
        assert!(op
            .matches(Some(&json!("value")), &json!(true), None)
            .unwrap());
        assert!(!op.matches(None, &json!(true), None).unwrap());
        assert!(op.matches(None, &json!(false), None).unwrap());
    }

    #[test]
    fn test_and_operator() {
        let doc = create_test_document(1, vec![("age", json!(25)), ("city", json!("NYC"))]);
        let filter = json!([{"age": {"$gt": 18}}, {"city": "NYC"}]);

        let op = AndOperator;
        assert!(op.matches(None, &filter, Some(&doc)).unwrap());
    }

    #[test]
    fn test_or_operator() {
        let doc = create_test_document(1, vec![("age", json!(15))]);
        let filter = json!([{"age": {"$lt": 18}}, {"age": {"$gt": 65}}]);

        let op = OrOperator;
        assert!(op.matches(None, &filter, Some(&doc)).unwrap());
    }

    #[test]
    fn test_matches_filter_simple() {
        let doc = create_test_document(1, vec![("name", json!("Alice"))]);
        let filter = json!({"name": "Alice"});
        assert!(matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_matches_filter_with_operators() {
        let doc = create_test_document(1, vec![("age", json!(25))]);
        let filter = json!({"age": {"$gte": 18, "$lt": 30}});
        assert!(matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_matches_filter_logical_and() {
        let doc = create_test_document(1, vec![("age", json!(25)), ("city", json!("NYC"))]);
        let filter = json!({"$and": [{"age": {"$gte": 18}}, {"city": "NYC"}]});
        assert!(matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_matches_filter_nested_dot_notation() {
        let doc = create_test_document(
            1,
            vec![
                ("address", json!({"city": "Budapest", "zip": 1111})),
                ("stats", json!({"login_count": 42})),
            ],
        );
        let filter = json!({"address.city": "Budapest", "stats.login_count": {"$gte": 40}});
        assert!(matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_operator_registry() {
        assert!(OPERATOR_REGISTRY.contains_key("$eq"));
        assert!(OPERATOR_REGISTRY.contains_key("$gt"));
        assert!(OPERATOR_REGISTRY.contains_key("$and"));
        assert!(OPERATOR_REGISTRY.contains_key("$exists"));
        assert!(OPERATOR_REGISTRY.contains_key("$all"));
        assert!(OPERATOR_REGISTRY.contains_key("$elemMatch"));
        assert!(OPERATOR_REGISTRY.contains_key("$type"));
        assert!(OPERATOR_REGISTRY.contains_key("$regex"));
        assert!(OPERATOR_REGISTRY.contains_key("$fuzzy"));
        assert!(OPERATOR_REGISTRY.contains_key("$expr"));
        assert_eq!(OPERATOR_REGISTRY.len(), 20); // Total operators implemented (19 + $fuzzy)
    }

    #[test]
    fn test_all_operator() {
        let op = AllOperator;
        let doc_value = json!(["apple", "banana", "cherry"]);
        let filter_value = json!(["apple", "banana"]);
        assert!(op.matches(Some(&doc_value), &filter_value, None).unwrap());

        let filter_value_fail = json!(["apple", "grape"]);
        assert!(!op
            .matches(Some(&doc_value), &filter_value_fail, None)
            .unwrap());
    }

    #[test]
    fn test_type_operator() {
        let op = TypeOperator;
        assert!(op
            .matches(Some(&json!("hello")), &json!("string"), None)
            .unwrap());
        assert!(op
            .matches(Some(&json!(42)), &json!("number"), None)
            .unwrap());
        assert!(op.matches(Some(&json!([])), &json!("array"), None).unwrap());
        assert!(!op
            .matches(Some(&json!("hello")), &json!("number"), None)
            .unwrap());
    }

    #[test]
    fn test_regex_operator() {
        let op = RegexOperator;
        assert!(op
            .matches(Some(&json!("hello world")), &json!("world"), None)
            .unwrap());
        assert!(!op
            .matches(Some(&json!("hello world")), &json!("xyz"), None)
            .unwrap());
    }

    #[test]
    fn test_size_operator() {
        let op = SizeOperator;

        // Array with 3 elements
        let arr3 = json!(["a", "b", "c"]);
        assert!(op.matches(Some(&arr3), &json!(3), None).unwrap());
        assert!(!op.matches(Some(&arr3), &json!(2), None).unwrap());
        assert!(!op.matches(Some(&arr3), &json!(4), None).unwrap());

        // Empty array
        let empty = json!([]);
        assert!(op.matches(Some(&empty), &json!(0), None).unwrap());
        assert!(!op.matches(Some(&empty), &json!(1), None).unwrap());

        // Non-array value should not match
        let str_val = json!("hello");
        assert!(!op.matches(Some(&str_val), &json!(5), None).unwrap());

        // Missing field should not match
        assert!(!op.matches(None, &json!(0), None).unwrap());
    }

    #[test]
    fn test_size_operator_in_query() {
        // Test matches_filter with $size
        let doc = create_test_document(1, vec![("tags", json!(["a", "b", "c"]))]);
        let filter = json!({"tags": {"$size": 3}});
        assert!(matches_filter(&doc, &filter).unwrap());

        let filter_fail = json!({"tags": {"$size": 2}});
        assert!(!matches_filter(&doc, &filter_fail).unwrap());
    }

    #[test]
    fn test_regex_with_options_case_insensitive() {
        // Test case-insensitive regex matching
        let doc_upper = create_test_document(1, vec![("name", json!("ALICE"))]);
        let doc_lower = create_test_document(2, vec![("name", json!("alice"))]);
        let doc_mixed = create_test_document(3, vec![("name", json!("Alice"))]);
        let doc_other = create_test_document(4, vec![("name", json!("Bob"))]);

        // Case-insensitive query: { name: { $regex: "alice", $options: "i" } }
        let filter_ci = json!({"name": {"$regex": "alice", "$options": "i"}});

        // All "alice" variants should match
        assert!(matches_filter(&doc_upper, &filter_ci).unwrap());
        assert!(matches_filter(&doc_lower, &filter_ci).unwrap());
        assert!(matches_filter(&doc_mixed, &filter_ci).unwrap());

        // "Bob" should not match
        assert!(!matches_filter(&doc_other, &filter_ci).unwrap());
    }

    #[test]
    fn test_regex_with_options_case_sensitive() {
        // Test case-sensitive regex matching (default / no options)
        let doc_lower = create_test_document(1, vec![("name", json!("alice"))]);
        let doc_upper = create_test_document(2, vec![("name", json!("ALICE"))]);

        // Case-sensitive query: { name: { $regex: "alice", $options: "" } }
        let filter_cs = json!({"name": {"$regex": "alice", "$options": ""}});

        // Only lowercase "alice" should match
        assert!(matches_filter(&doc_lower, &filter_cs).unwrap());
        assert!(!matches_filter(&doc_upper, &filter_cs).unwrap());
    }

    #[test]
    fn test_regex_without_options() {
        // Test that $regex alone still works (case-sensitive by default)
        let doc_lower = create_test_document(1, vec![("name", json!("alice"))]);
        let doc_upper = create_test_document(2, vec![("name", json!("ALICE"))]);

        let filter = json!({"name": {"$regex": "alice"}});

        // Should be case-sensitive
        assert!(matches_filter(&doc_lower, &filter).unwrap());
        assert!(!matches_filter(&doc_upper, &filter).unwrap());
    }

    #[test]
    fn test_regex_with_options_on_array() {
        // Test case-insensitive regex on array field
        let doc = create_test_document(1, vec![("tags", json!(["Rust", "PYTHON", "javascript"]))]);

        let filter_rust = json!({"tags": {"$regex": "rust", "$options": "i"}});
        let filter_python = json!({"tags": {"$regex": "python", "$options": "i"}});
        let filter_java = json!({"tags": {"$regex": "java", "$options": "i"}});
        let filter_go = json!({"tags": {"$regex": "go", "$options": "i"}});

        assert!(matches_filter(&doc, &filter_rust).unwrap());
        assert!(matches_filter(&doc, &filter_python).unwrap());
        assert!(matches_filter(&doc, &filter_java).unwrap()); // "javascript" contains "java"
        assert!(!matches_filter(&doc, &filter_go).unwrap());
    }

    #[test]
    fn test_regex_with_invalid_options_error() {
        let doc = create_test_document(1, vec![("name", json!("Alice"))]);
        let filter = json!({"name": {"$regex": "alice", "$options": "iz"}});

        let err = matches_filter(&doc, &filter).unwrap_err();
        assert!(err.to_string().contains("Invalid regex option"));
    }

    // ========================================================================
    // $expr OPERATOR TESTS
    // ========================================================================

    #[test]
    fn test_expr_compare_two_fields() {
        // Test comparing two fields within a document
        // { "$expr": { "$gt": ["$quantity", "$reorderLevel"] } }
        let doc_above = create_test_document(
            1,
            vec![("quantity", json!(100)), ("reorderLevel", json!(50))],
        );
        let doc_below = create_test_document(
            2,
            vec![("quantity", json!(30)), ("reorderLevel", json!(50))],
        );
        let doc_equal = create_test_document(
            3,
            vec![("quantity", json!(50)), ("reorderLevel", json!(50))],
        );

        let filter = json!({"$expr": {"$gt": ["$quantity", "$reorderLevel"]}});

        assert!(matches_filter(&doc_above, &filter).unwrap()); // 100 > 50
        assert!(!matches_filter(&doc_below, &filter).unwrap()); // 30 > 50 is false
        assert!(!matches_filter(&doc_equal, &filter).unwrap()); // 50 > 50 is false
    }

    #[test]
    fn test_expr_compare_field_with_literal() {
        // Test comparing a field with a literal value
        // { "$expr": { "$gte": ["$age", 18] } }
        let doc_adult = create_test_document(1, vec![("age", json!(25))]);
        let doc_teen = create_test_document(2, vec![("age", json!(16))]);
        let doc_exact = create_test_document(3, vec![("age", json!(18))]);

        let filter = json!({"$expr": {"$gte": ["$age", 18]}});

        assert!(matches_filter(&doc_adult, &filter).unwrap()); // 25 >= 18
        assert!(!matches_filter(&doc_teen, &filter).unwrap()); // 16 >= 18 is false
        assert!(matches_filter(&doc_exact, &filter).unwrap()); // 18 >= 18
    }

    #[test]
    fn test_expr_eq_and_ne() {
        // Test $eq and $ne in $expr
        let doc = create_test_document(
            1,
            vec![("a", json!(10)), ("b", json!(10)), ("c", json!(20))],
        );

        let filter_eq = json!({"$expr": {"$eq": ["$a", "$b"]}});
        let filter_ne = json!({"$expr": {"$ne": ["$a", "$c"]}});
        let filter_eq_fail = json!({"$expr": {"$eq": ["$a", "$c"]}});

        assert!(matches_filter(&doc, &filter_eq).unwrap()); // a == b (10 == 10)
        assert!(matches_filter(&doc, &filter_ne).unwrap()); // a != c (10 != 20)
        assert!(!matches_filter(&doc, &filter_eq_fail).unwrap()); // a == c (10 == 20 is false)
    }

    #[test]
    fn test_expr_with_strings() {
        // Test $expr with string comparisons
        let doc = create_test_document(
            1,
            vec![("firstName", json!("Alice")), ("lastName", json!("Smith"))],
        );

        let filter_lt = json!({"$expr": {"$lt": ["$firstName", "$lastName"]}});
        let filter_gt = json!({"$expr": {"$gt": ["$firstName", "$lastName"]}});

        assert!(matches_filter(&doc, &filter_lt).unwrap()); // "Alice" < "Smith" alphabetically
        assert!(!matches_filter(&doc, &filter_gt).unwrap()); // "Alice" > "Smith" is false
    }

    #[test]
    fn test_expr_substr() {
        let doc = create_test_document(1, vec![("name", json!("Alice"))]);

        let filter = json!({"$expr": {"$eq": [{"$substr": ["$name", 0, 1]}, "A"]}});
        let filter_no_match = json!({"$expr": {"$eq": [{"$substr": ["$name", 0, 1]}, "B"]}});

        assert!(matches_filter(&doc, &filter).unwrap());
        assert!(!matches_filter(&doc, &filter_no_match).unwrap());
    }

    #[test]
    fn test_expr_missing_field() {
        // Test $expr when a field is missing
        let doc = create_test_document(1, vec![("quantity", json!(100))]);

        let filter = json!({"$expr": {"$gt": ["$quantity", "$reorderLevel"]}});

        // Should return false when a field is missing
        assert!(!matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_expr_nested_logical_operators() {
        // Test nested logical operators in $expr
        // { "$expr": { "$and": [{ "$gt": ["$a", 5] }, { "$lt": ["$a", 10] }] } }
        let doc_in_range = create_test_document(1, vec![("a", json!(7))]);
        let doc_too_low = create_test_document(2, vec![("a", json!(3))]);
        let doc_too_high = create_test_document(3, vec![("a", json!(15))]);

        let filter = json!({
            "$expr": {
                "$and": [
                    {"$gt": ["$a", 5]},
                    {"$lt": ["$a", 10]}
                ]
            }
        });

        assert!(matches_filter(&doc_in_range, &filter).unwrap()); // 7 > 5 AND 7 < 10
        assert!(!matches_filter(&doc_too_low, &filter).unwrap()); // 3 > 5 is false
        assert!(!matches_filter(&doc_too_high, &filter).unwrap()); // 15 < 10 is false
    }

    #[test]
    fn test_expr_or_operator() {
        // Test $or in $expr
        let doc_low = create_test_document(1, vec![("score", json!(20))]);
        let doc_high = create_test_document(2, vec![("score", json!(90))]);
        let doc_mid = create_test_document(3, vec![("score", json!(50))]);

        let filter = json!({
            "$expr": {
                "$or": [
                    {"$lt": ["$score", 30]},
                    {"$gt": ["$score", 80]}
                ]
            }
        });

        assert!(matches_filter(&doc_low, &filter).unwrap()); // 20 < 30
        assert!(matches_filter(&doc_high, &filter).unwrap()); // 90 > 80
        assert!(!matches_filter(&doc_mid, &filter).unwrap()); // 50 is neither
    }

    // ========================================================================
    // FULL REGEX TESTS (regex crate)
    // ========================================================================

    #[test]
    fn test_regex_anchor_start() {
        let doc = create_test_document(1, vec![("name", json!("Alice Smith"))]);
        let filter = json!({"name": {"$regex": "^Alice"}});
        assert!(matches_filter(&doc, &filter).unwrap());

        let filter_fail = json!({"name": {"$regex": "^Smith"}});
        assert!(!matches_filter(&doc, &filter_fail).unwrap());
    }

    #[test]
    fn test_regex_anchor_end() {
        let doc = create_test_document(1, vec![("name", json!("Alice Smith"))]);
        let filter = json!({"name": {"$regex": "Smith$"}});
        assert!(matches_filter(&doc, &filter).unwrap());

        let filter_fail = json!({"name": {"$regex": "Alice$"}});
        assert!(!matches_filter(&doc, &filter_fail).unwrap());
    }

    #[test]
    fn test_regex_anchor_full() {
        let doc = create_test_document(1, vec![("name", json!("Alice"))]);
        let filter = json!({"name": {"$regex": "^Alice$"}});
        assert!(matches_filter(&doc, &filter).unwrap());

        let doc_partial = create_test_document(2, vec![("name", json!("Alice Smith"))]);
        assert!(!matches_filter(&doc_partial, &filter).unwrap());
    }

    #[test]
    fn test_regex_character_class() {
        let doc = create_test_document(1, vec![("email", json!("test@example.com"))]);
        let filter = json!({"email": {"$regex": "[a-z]+@[a-z]+\\.[a-z]+"}});
        assert!(matches_filter(&doc, &filter).unwrap());

        let doc_invalid = create_test_document(2, vec![("email", json!("123@456.789"))]);
        assert!(!matches_filter(&doc_invalid, &filter).unwrap());
    }

    #[test]
    fn test_regex_digit_class() {
        let doc = create_test_document(1, vec![("code", json!("ABC123"))]);
        let filter = json!({"code": {"$regex": "[A-Z]+\\d+"}});
        assert!(matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_regex_quantifiers() {
        let doc = create_test_document(1, vec![("phone", json!("123-456-7890"))]);
        let filter = json!({"phone": {"$regex": "^\\d{3}-\\d{3}-\\d{4}$"}});
        assert!(matches_filter(&doc, &filter).unwrap());

        let doc_invalid = create_test_document(2, vec![("phone", json!("12-34-5678"))]);
        assert!(!matches_filter(&doc_invalid, &filter).unwrap());
    }

    #[test]
    fn test_regex_alternation() {
        let doc = create_test_document(1, vec![("lang", json!("rust"))]);
        let filter = json!({"lang": {"$regex": "^(python|javascript|rust)$"}});
        assert!(matches_filter(&doc, &filter).unwrap());

        let doc_go = create_test_document(2, vec![("lang", json!("go"))]);
        assert!(!matches_filter(&doc_go, &filter).unwrap());
    }

    #[test]
    fn test_regex_optional_quantifier() {
        let doc1 = create_test_document(1, vec![("color", json!("color"))]);
        let doc2 = create_test_document(2, vec![("color", json!("colour"))]);
        let filter = json!({"color": {"$regex": "colou?r"}});
        assert!(matches_filter(&doc1, &filter).unwrap());
        assert!(matches_filter(&doc2, &filter).unwrap());
    }

    #[test]
    fn test_regex_plus_quantifier() {
        let doc = create_test_document(1, vec![("text", json!("aaaabc"))]);
        let filter = json!({"text": {"$regex": "a+bc"}});
        assert!(matches_filter(&doc, &filter).unwrap());

        let doc_no_a = create_test_document(2, vec![("text", json!("bc"))]);
        assert!(!matches_filter(&doc_no_a, &filter).unwrap());
    }

    #[test]
    fn test_regex_star_quantifier() {
        let doc1 = create_test_document(1, vec![("text", json!("bc"))]);
        let doc2 = create_test_document(2, vec![("text", json!("aaabc"))]);
        let filter = json!({"text": {"$regex": "a*bc"}});
        assert!(matches_filter(&doc1, &filter).unwrap());
        assert!(matches_filter(&doc2, &filter).unwrap());
    }

    #[test]
    fn test_regex_multiline_option() {
        let doc = create_test_document(1, vec![("text", json!("line1\nline2\nline3"))]);
        let filter = json!({"text": {"$regex": "^line2$", "$options": "m"}});
        assert!(matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_regex_dotall_option() {
        let doc = create_test_document(1, vec![("text", json!("hello\nworld"))]);
        let filter = json!({"text": {"$regex": "hello.world", "$options": "s"}});
        assert!(matches_filter(&doc, &filter).unwrap());

        // Without 's' option, '.' doesn't match newline
        let filter_no_s = json!({"text": {"$regex": "hello.world"}});
        assert!(!matches_filter(&doc, &filter_no_s).unwrap());
    }

    #[test]
    fn test_regex_invalid_pattern_error() {
        let doc = create_test_document(1, vec![("name", json!("Alice"))]);
        let filter = json!({"name": {"$regex": "[unclosed"}});
        let result = matches_filter(&doc, &filter);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid regex"));
    }

    #[test]
    fn test_regex_word_boundary() {
        let doc = create_test_document(1, vec![("text", json!("test testing tested"))]);
        let filter = json!({"text": {"$regex": "\\btest\\b"}});
        assert!(matches_filter(&doc, &filter).unwrap());

        // Should not match "testing" when looking for whole word "test"
        let doc2 = create_test_document(2, vec![("text", json!("testing"))]);
        assert!(!matches_filter(&doc2, &filter).unwrap());
    }

    #[test]
    fn test_regex_whitespace_class() {
        let doc = create_test_document(1, vec![("text", json!("hello world"))]);
        let filter = json!({"text": {"$regex": "hello\\s+world"}});
        assert!(matches_filter(&doc, &filter).unwrap());

        let doc_tabs = create_test_document(2, vec![("text", json!("hello\t\tworld"))]);
        assert!(matches_filter(&doc_tabs, &filter).unwrap());
    }

    #[test]
    fn test_regex_combined_options() {
        let doc = create_test_document(1, vec![("text", json!("HELLO\nworld"))]);
        // Case insensitive + multiline
        let filter = json!({"text": {"$regex": "^hello$", "$options": "im"}});
        assert!(matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_regex_on_array_with_anchors() {
        let doc = create_test_document(1, vec![("tags", json!(["rust", "python", "javascript"]))]);
        let filter = json!({"tags": {"$regex": "^rust$"}});
        assert!(matches_filter(&doc, &filter).unwrap());

        let filter_not_found = json!({"tags": {"$regex": "^go$"}});
        assert!(!matches_filter(&doc, &filter_not_found).unwrap());
    }

    #[test]
    fn test_regex_cache_reuse() {
        // This test verifies that the same pattern is used multiple times
        // and should benefit from caching (can't directly test cache hit,
        // but we verify correct behavior with repeated use)
        let doc1 = create_test_document(1, vec![("name", json!("Alice"))]);
        let doc2 = create_test_document(2, vec![("name", json!("Bob"))]);
        let doc3 = create_test_document(3, vec![("name", json!("Charlie"))]);

        let filter = json!({"name": {"$regex": "^[A-C]"}});

        assert!(matches_filter(&doc1, &filter).unwrap()); // Alice starts with A
        assert!(matches_filter(&doc2, &filter).unwrap()); // Bob starts with B
        assert!(matches_filter(&doc3, &filter).unwrap()); // Charlie starts with C
    }

    // ========================================================================
    // FUZZY OPERATOR TESTS
    // ========================================================================

    #[test]
    fn test_fuzzy_operator_simple_form() {
        let op = FuzzyOperator;

        // Exact match should pass
        assert!(op
            .matches(Some(&json!("john")), &json!("john"), None)
            .unwrap());

        // Similar strings should pass (jaro_winkler default)
        assert!(op
            .matches(Some(&json!("john")), &json!("jon"), None)
            .unwrap());
        assert!(op
            .matches(Some(&json!("john")), &json!("johnn"), None)
            .unwrap());

        // Very different strings should fail
        assert!(!op
            .matches(Some(&json!("john")), &json!("xyz"), None)
            .unwrap());

        // None value should fail
        assert!(!op.matches(None, &json!("john"), None).unwrap());
    }

    #[test]
    fn test_fuzzy_operator_extended_form() {
        let op = FuzzyOperator;

        // Extended form with explicit algorithm
        let filter = json!({"value": "john", "algorithm": "jaro_winkler", "threshold": 0.8});
        assert!(op.matches(Some(&json!("john")), &filter, None).unwrap());
        assert!(op.matches(Some(&json!("jon")), &filter, None).unwrap());

        // Lower threshold allows more matches
        let filter_low = json!({"value": "john", "threshold": 0.5});
        assert!(op.matches(Some(&json!("jane")), &filter_low, None).unwrap());

        // Higher threshold is stricter
        let filter_high = json!({"value": "john", "threshold": 0.95});
        assert!(op
            .matches(Some(&json!("john")), &filter_high, None)
            .unwrap());
        assert!(!op.matches(Some(&json!("jon")), &filter_high, None).unwrap());
    }

    #[test]
    fn test_fuzzy_operator_algorithms() {
        let op = FuzzyOperator;

        // Levenshtein
        let filter_lev = json!({"value": "john", "algorithm": "levenshtein", "threshold": 0.7});
        assert!(op.matches(Some(&json!("john")), &filter_lev, None).unwrap());
        assert!(op.matches(Some(&json!("jon")), &filter_lev, None).unwrap());

        // Damerau-Levenshtein (good for transpositions)
        let filter_dl =
            json!({"value": "the", "algorithm": "damerau_levenshtein", "threshold": 0.6});
        assert!(op.matches(Some(&json!("teh")), &filter_dl, None).unwrap()); // transposition
    }

    #[test]
    fn test_fuzzy_operator_array_matching() {
        let op = FuzzyOperator;

        // Array should match if any element is similar
        let arr = json!(["alice", "bob", "charlie"]);
        assert!(op.matches(Some(&arr), &json!("bob"), None).unwrap());
        assert!(op.matches(Some(&arr), &json!("bobb"), None).unwrap()); // fuzzy match
        assert!(!op.matches(Some(&arr), &json!("xyz"), None).unwrap());
    }

    #[test]
    fn test_fuzzy_operator_invalid_filter() {
        let op = FuzzyOperator;

        // Invalid filter types
        assert!(op.matches(Some(&json!("john")), &json!(123), None).is_err());

        // Object without value field
        assert!(op
            .matches(Some(&json!("john")), &json!({"threshold": 0.8}), None)
            .is_err());

        // Invalid threshold
        assert!(op
            .matches(
                Some(&json!("john")),
                &json!({"value": "john", "threshold": 1.5}),
                None
            )
            .is_err());

        // Invalid algorithm
        assert!(op
            .matches(
                Some(&json!("john")),
                &json!({"value": "john", "algorithm": "unknown"}),
                None
            )
            .is_err());
    }

    #[test]
    fn test_fuzzy_algorithm_similarity() {
        // Test Jaro-Winkler
        let jw = FuzzyAlgorithm::JaroWinkler;
        assert!(jw.similarity("john", "john") > 0.99);
        assert!(jw.similarity("john", "jon") > 0.85);
        assert!(jw.similarity("john", "johnny") > 0.8);

        // Test Levenshtein
        let lev = FuzzyAlgorithm::Levenshtein;
        assert!(lev.similarity("john", "john") > 0.99);
        assert!(lev.similarity("john", "jon") > 0.7);

        // Test Damerau-Levenshtein
        let dl = FuzzyAlgorithm::DamerauLevenshtein;
        assert!(dl.similarity("the", "teh") > 0.6); // transposition
        assert!(dl.similarity("abc", "abc") > 0.99);
    }

    #[test]
    fn test_fuzzy_with_document_filter() {
        let doc = create_test_document(1, vec![("name", json!("John"))]);

        // Simple form - exact case should match
        let filter = json!({"name": {"$fuzzy": "john"}});
        assert!(matches_filter(&doc, &filter).unwrap());

        // Extended form with lower threshold
        let filter2 = json!({"name": {"$fuzzy": {"value": "jon", "threshold": 0.7}}});
        assert!(matches_filter(&doc, &filter2).unwrap());

        // No match with very different string
        let filter3 = json!({"name": {"$fuzzy": {"value": "xyz", "threshold": 0.5}}});
        assert!(!matches_filter(&doc, &filter3).unwrap());
    }

    // ========================================================================
    // FUZZY OPERATOR WITH NESTED DOCUMENTS TESTS
    // ========================================================================

    #[test]
    fn test_fuzzy_nested_field_simple() {
        // Document with nested structure
        let doc = create_test_document(
            1,
            vec![(
                "user",
                json!({
                    "name": "John Smith",
                    "profile": {
                        "bio": "Software engineer"
                    }
                }),
            )],
        );

        // Fuzzy search on nested field
        let filter = json!({"user.name": {"$fuzzy": "jon smith"}});
        assert!(matches_filter(&doc, &filter).unwrap());

        // Fuzzy search on deeply nested field
        let filter2 = json!({"user.profile.bio": {"$fuzzy": "software enginer"}}); // typo
        assert!(matches_filter(&doc, &filter2).unwrap());
    }

    #[test]
    fn test_fuzzy_nested_field_extended_form() {
        let doc = create_test_document(
            1,
            vec![(
                "contact",
                json!({
                    "email": "john.smith@example.com",
                    "address": {
                        "city": "New York"
                    }
                }),
            )],
        );

        // Extended form with algorithm on nested field
        // "New York" vs "new york" - case insensitive comparison
        let filter = json!({
            "contact.address.city": {
                "$fuzzy": {
                    "value": "new york",
                    "algorithm": "jaro_winkler",
                    "threshold": 0.8
                }
            }
        });
        assert!(matches_filter(&doc, &filter).unwrap());

        // Jaro-Winkler on email field
        let filter2 = json!({
            "contact.email": {
                "$fuzzy": {
                    "value": "john.smth@example.com",  // missing 'i'
                    "algorithm": "jaro_winkler",
                    "threshold": 0.85
                }
            }
        });
        assert!(matches_filter(&doc, &filter2).unwrap());
    }

    #[test]
    fn test_fuzzy_nested_field_no_match() {
        let doc = create_test_document(
            1,
            vec![(
                "company",
                json!({
                    "name": "Acme Corporation"
                }),
            )],
        );

        // Very different string should not match
        let filter = json!({"company.name": {"$fuzzy": "xyz technologies"}});
        assert!(!matches_filter(&doc, &filter).unwrap());

        // Non-existent nested path should not match
        let filter2 = json!({"company.location.city": {"$fuzzy": "test"}});
        assert!(!matches_filter(&doc, &filter2).unwrap());
    }

    #[test]
    fn test_fuzzy_nested_combined_with_other_operators() {
        let doc = create_test_document(
            1,
            vec![
                (
                    "user",
                    json!({
                        "name": "John Smith",
                        "age": 30
                    }),
                ),
                ("status", json!("active")),
            ],
        );

        // Combine fuzzy with $and and other operators
        let filter = json!({
            "$and": [
                {"user.name": {"$fuzzy": "jon smith"}},
                {"user.age": {"$gte": 25}},
                {"status": "active"}
            ]
        });
        assert!(matches_filter(&doc, &filter).unwrap());

        // Combine fuzzy with $or
        let filter2 = json!({
            "$or": [
                {"user.name": {"$fuzzy": "jane doe"}},  // no match
                {"user.name": {"$fuzzy": "john smth"}}  // match
            ]
        });
        assert!(matches_filter(&doc, &filter2).unwrap());
    }

    #[test]
    fn test_fuzzy_nested_three_levels_deep() {
        let doc = create_test_document(
            1,
            vec![(
                "organization",
                json!({
                    "department": {
                        "team": {
                            "leader": "Elizabeth Johnson"
                        }
                    }
                }),
            )],
        );

        // Fuzzy on 4-level deep field
        let filter = json!({
            "organization.department.team.leader": {
                "$fuzzy": {
                    "value": "elisabeth johnsen",  // typos
                    "algorithm": "levenshtein",
                    "threshold": 0.75
                }
            }
        });
        assert!(matches_filter(&doc, &filter).unwrap());
    }

    // ========== $not + $regex + $options tests ==========

    #[test]
    fn test_not_with_regex_options_case_insensitive() {
        let doc = create_test_document(1, vec![("name", json!("ALICE"))]);

        // $not with $regex and $options should work
        // This should NOT match because "ALICE" matches /alice/i
        let filter = json!({"name": {"$not": {"$regex": "alice", "$options": "i"}}});
        assert!(!matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_not_with_regex_options_no_match() {
        let doc = create_test_document(1, vec![("name", json!("BOB"))]);

        // $not with $regex and $options - should match because "BOB" doesn't match /alice/i
        let filter = json!({"name": {"$not": {"$regex": "alice", "$options": "i"}}});
        assert!(matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_not_with_regex_options_multiline() {
        let doc = create_test_document(1, vec![("text", json!("Hello\nWorld"))]);

        // $not with $regex and multiline option
        let filter = json!({"text": {"$not": {"$regex": "^World", "$options": "m"}}});
        // "World" is at the start of a line, so regex matches -> $not returns false
        assert!(!matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_not_with_regex_no_options() {
        let doc = create_test_document(1, vec![("name", json!("alice"))]);

        // $not with $regex but no $options - should still work
        let filter = json!({"name": {"$not": {"$regex": "bob"}}});
        assert!(matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_not_options_without_regex_error() {
        let doc = create_test_document(1, vec![("name", json!("alice"))]);

        // $options without $regex inside $not should error
        let filter = json!({"name": {"$not": {"$options": "i"}}});
        let result = matches_filter(&doc, &filter);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("$options requires $regex"));
    }

    // ========== $expr arithmetic tests ==========

    #[test]
    fn test_expr_add_basic() {
        let doc = create_test_document(1, vec![("a", json!(10)), ("b", json!(5))]);

        // { $expr: { $gt: [{ $add: ["$a", "$b"] }, 14] } }
        // 10 + 5 = 15 > 14 -> true
        let filter = json!({"$expr": {"$gt": [{"$add": ["$a", "$b"]}, 14]}});
        assert!(matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_expr_add_no_match() {
        let doc = create_test_document(1, vec![("a", json!(10)), ("b", json!(5))]);

        // 10 + 5 = 15 > 16 -> false
        let filter = json!({"$expr": {"$gt": [{"$add": ["$a", "$b"]}, 16]}});
        assert!(!matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_expr_subtract() {
        let doc = create_test_document(1, vec![("total", json!(100)), ("discount", json!(20))]);

        // { $expr: { $gte: [{ $subtract: ["$total", "$discount"] }, 80] } }
        // 100 - 20 = 80 >= 80 -> true
        let filter = json!({"$expr": {"$gte": [{"$subtract": ["$total", "$discount"]}, 80]}});
        assert!(matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_expr_multiply() {
        let doc = create_test_document(1, vec![("price", json!(10)), ("quantity", json!(5))]);

        // { $expr: { $eq: [{ $multiply: ["$price", "$quantity"] }, 50] } }
        // 10 * 5 = 50 == 50 -> true
        let filter = json!({"$expr": {"$eq": [{"$multiply": ["$price", "$quantity"]}, 50]}});
        assert!(matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_expr_divide() {
        let doc = create_test_document(1, vec![("total", json!(100)), ("count", json!(4))]);

        // { $expr: { $eq: [{ $divide: ["$total", "$count"] }, 25] } }
        // 100 / 4 = 25 == 25 -> true
        let filter = json!({"$expr": {"$eq": [{"$divide": ["$total", "$count"]}, 25]}});
        assert!(matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_expr_nested_arithmetic() {
        let doc = create_test_document(1, vec![("a", json!(10)), ("b", json!(5)), ("c", json!(3))]);

        // { $expr: { $eq: [{ $add: [{ $multiply: ["$a", "$b"] }, "$c"] }, 53] } }
        // (10 * 5) + 3 = 53 == 53 -> true
        let filter = json!({
            "$expr": {
                "$eq": [
                    {"$add": [{"$multiply": ["$a", "$b"]}, "$c"]},
                    53
                ]
            }
        });
        assert!(matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_expr_mod_operator() {
        let doc = create_test_document(1, vec![("value", json!(17))]);

        // { $expr: { $eq: [{ $mod: ["$value", 5] }, 2] } }
        // 17 % 5 = 2 == 2 -> true
        let filter = json!({"$expr": {"$eq": [{"$mod": ["$value", 5]}, 2]}});
        assert!(matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_expr_abs_operator() {
        let doc = create_test_document(1, vec![("balance", json!(-50))]);

        // { $expr: { $gt: [{ $abs: ["$balance"] }, 40] } }
        // |-50| = 50 > 40 -> true
        let filter = json!({"$expr": {"$gt": [{"$abs": ["$balance"]}, 40]}});
        assert!(matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_expr_floor_ceil() {
        let doc = create_test_document(1, vec![("value", json!(3.7))]);

        // floor(3.7) = 3
        let floor_filter = json!({"$expr": {"$eq": [{"$floor": ["$value"]}, 3]}});
        assert!(matches_filter(&doc, &floor_filter).unwrap());

        // ceil(3.7) = 4
        let ceil_filter = json!({"$expr": {"$eq": [{"$ceil": ["$value"]}, 4]}});
        assert!(matches_filter(&doc, &ceil_filter).unwrap());
    }

    #[test]
    fn test_expr_with_literals() {
        let doc = create_test_document(1, vec![("value", json!(10))]);

        // { $expr: { $gt: [{ $add: ["$value", 5] }, 14] } }
        // 10 + 5 = 15 > 14 -> true
        let filter = json!({"$expr": {"$gt": [{"$add": ["$value", 5]}, 14]}});
        assert!(matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_expr_arithmetic_missing_field() {
        let doc = create_test_document(1, vec![("a", json!(10))]);

        // Missing field "$b" should make comparison return false
        let filter = json!({"$expr": {"$gt": [{"$add": ["$a", "$b"]}, 10]}});
        assert!(!matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_expr_divide_by_zero() {
        let doc = create_test_document(1, vec![("a", json!(10)), ("b", json!(0))]);

        // Division by zero returns null, comparison with null returns false
        let filter = json!({"$expr": {"$gt": [{"$divide": ["$a", "$b"]}, 0]}});
        assert!(!matches_filter(&doc, &filter).unwrap());
    }

    // ========================================================================
    // HASHSET OPTIMIZATION TESTS FOR $in/$nin/$all
    // ========================================================================

    #[test]
    fn test_in_operator_large_array_performance() {
        // Test that $in with large array works correctly
        // This verifies the HashSet optimization doesn't break functionality
        let op = InOperator;

        // Create a large filter array with 1000 IDs
        let filter_array: Vec<serde_json::Value> =
            (0..1000).map(|i| json!(format!("id_{}", i))).collect();
        let filter = serde_json::Value::Array(filter_array);

        // Test matching value (id_500 is in the array)
        assert!(op.matches(Some(&json!("id_500")), &filter, None).unwrap());

        // Test non-matching value
        assert!(!op.matches(Some(&json!("id_9999")), &filter, None).unwrap());

        // Test with array document value
        let doc_array = json!(["id_999", "other_value"]);
        assert!(op.matches(Some(&doc_array), &filter, None).unwrap());

        // Test with array document value that doesn't match
        let doc_array_no_match = json!(["not_in_list", "also_not"]);
        assert!(!op
            .matches(Some(&doc_array_no_match), &filter, None)
            .unwrap());
    }

    #[test]
    fn test_nin_operator_large_array_performance() {
        // Test that $nin with large array works correctly
        let op = NinOperator;

        let filter_array: Vec<serde_json::Value> =
            (0..1000).map(|i| json!(format!("id_{}", i))).collect();
        let filter = serde_json::Value::Array(filter_array);

        // Test value NOT in array (should return true for $nin)
        assert!(op.matches(Some(&json!("id_9999")), &filter, None).unwrap());

        // Test value IN array (should return false for $nin)
        assert!(!op.matches(Some(&json!("id_500")), &filter, None).unwrap());
    }

    #[test]
    fn test_all_operator_large_array_performance() {
        // Test that $all with large document array works correctly
        let op = AllOperator;

        // Document with 1000 elements
        let doc_array: Vec<serde_json::Value> =
            (0..1000).map(|i| json!(format!("val_{}", i))).collect();
        let doc = serde_json::Value::Array(doc_array);

        // Required values that are all in document
        let required_present = json!(["val_0", "val_500", "val_999"]);
        assert!(op.matches(Some(&doc), &required_present, None).unwrap());

        // Required values where one is NOT in document
        let required_missing = json!(["val_0", "val_9999"]);
        assert!(!op.matches(Some(&doc), &required_missing, None).unwrap());
    }

    #[test]
    fn test_in_operator_with_numeric_ids() {
        // Test $in with numeric IDs (common for _id fields)
        let op = InOperator;

        let filter_array: Vec<serde_json::Value> = (0..100).map(|i| json!(i)).collect();
        let filter = serde_json::Value::Array(filter_array);

        // Test matching numeric value
        assert!(op.matches(Some(&json!(50)), &filter, None).unwrap());
        assert!(op.matches(Some(&json!(0)), &filter, None).unwrap());
        assert!(op.matches(Some(&json!(99)), &filter, None).unwrap());

        // Test non-matching numeric value
        assert!(!op.matches(Some(&json!(100)), &filter, None).unwrap());
        assert!(!op.matches(Some(&json!(-1)), &filter, None).unwrap());
    }

    #[test]
    fn test_in_operator_with_mixed_types() {
        // Test $in with mixed types in filter array
        let op = InOperator;

        let filter = json!([1, "two", 3.0, true, null, {"nested": "object"}, [1, 2, 3]]);

        // Test hashable types
        assert!(op.matches(Some(&json!(1)), &filter, None).unwrap());
        assert!(op.matches(Some(&json!("two")), &filter, None).unwrap());
        assert!(op.matches(Some(&json!(true)), &filter, None).unwrap());
        assert!(op.matches(Some(&json!(null)), &filter, None).unwrap());

        // Test non-hashable types (objects/arrays) - should fall back to linear search
        assert!(op
            .matches(Some(&json!({"nested": "object"})), &filter, None)
            .unwrap());
        assert!(op.matches(Some(&json!([1, 2, 3])), &filter, None).unwrap());

        // Test non-matching
        assert!(!op.matches(Some(&json!("three")), &filter, None).unwrap());
        assert!(!op.matches(Some(&json!(false)), &filter, None).unwrap());
    }
}
