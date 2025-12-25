// src/query/operators/traits.rs
// OperatorMatcher trait definition

use crate::document::Document;
use crate::error::Result;
use serde_json::Value;

/// Trait for all query operators
///
/// Each MongoDB query operator ($eq, $gt, $and, etc.) implements this trait.
/// The trait provides a uniform interface for matching documents against filter criteria.
///
/// # Examples
///
/// ```rust
/// use serde_json::json;
/// use ironbase_core::query::operators::EqOperator;
/// use ironbase_core::query::operators::OperatorMatcher;
///
/// let eq_op = EqOperator;
/// let matches = eq_op.matches(Some(&json!("Alice")), &json!("Alice"), None).unwrap();
/// assert!(matches);
/// ```
pub trait OperatorMatcher: Send + Sync {
    /// Returns the operator name (e.g., "$eq", "$gt", "$and")
    fn name(&self) -> &'static str;

    /// Checks if a document value matches the filter criteria
    ///
    /// # Arguments
    ///
    /// - `doc_value`: The value from the document field (None if field doesn't exist)
    /// - `filter_value`: The expected value from the query filter
    /// - `document`: Optional reference to the full document (for logical operators that recurse)
    ///
    /// # Returns
    ///
    /// - `Ok(true)` if the document matches
    /// - `Ok(false)` if the document doesn't match
    /// - `Err(...)` if there's a validation error (e.g., wrong type for operator)
    fn matches(
        &self,
        doc_value: Option<&Value>,
        filter_value: &Value,
        document: Option<&Document>,
    ) -> Result<bool>;
}
