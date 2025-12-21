// src/query_planner.rs
// Query planner and optimizer - index selection

use crate::index::{IndexKey, IndexPrefixInfo};
use serde_json::Value;

/// Query plan - describes how to execute a query
/// NOTE: CollectionScan variant was removed - analyze_query() returns None for full scan,
/// and explain_query() handles None case by generating "CollectionScan" JSON directly.
#[derive(Debug, Clone)]
pub enum QueryPlan {
    /// Index scan for equality match
    IndexScan {
        index_name: String,
        field: String,
        key: IndexKey,
        /// If true, this is a compound index prefix query - use range scan internally
        is_compound: bool,
    },

    /// Index range scan
    IndexRangeScan {
        index_name: String,
        field: String,
        start: Option<IndexKey>,
        end: Option<IndexKey>,
        inclusive_start: bool,
        inclusive_end: bool,
    },
}

/// Query planner - analyzes queries and selects optimal execution plan
pub struct QueryPlanner;

impl QueryPlanner {
    /// Analyze a query and determine if an index can be used
    /// Returns (field_name, QueryPlan) if an index opportunity is found
    ///
    /// DEPRECATED: Use analyze_query_with_fields() for compound index support
    #[deprecated(
        since = "0.3.0",
        note = "use analyze_query_with_fields for compound index support"
    )]
    #[allow(dead_code)]
    pub fn analyze_query(
        query_json: &Value,
        available_indexes: &[String],
    ) -> Option<(String, QueryPlan)> {
        // Check for simple equality query: { "field": value }
        if let Value::Object(ref map) = query_json {
            // First try range query analysis (handles { "field": { "$gte": ... } })
            if let Some((field, plan)) = Self::analyze_range_query(query_json, available_indexes) {
                return Some((field, plan));
            }

            // Skip logical operators like $and, $or, $nor
            if map.keys().any(|k| k.starts_with('$')) {
                return None;
            }

            // Simple equality query: { "field": value }
            if let Some((field, value)) = map.iter().next() {
                // Skip if value contains operators (like {"age": {"$gt": 5}})
                if let Value::Object(ref val_map) = value {
                    if val_map.keys().any(|k| k.starts_with('$')) {
                        // Already handled by range query analysis above
                        return None;
                    }
                }

                // Check if we have an index on this field
                let index_name = Self::find_index_for_field(field, available_indexes)?;

                let key = IndexKey::from(value);
                return Some((
                    field.clone(),
                    QueryPlan::IndexScan {
                        index_name,
                        field: field.clone(),
                        key,
                        is_compound: false, // Legacy method doesn't know about compound indexes
                    },
                ));
            }
        }

        None
    }

    /// Analyze query for range operators ($gt, $gte, $lt, $lte)
    ///
    /// DEPRECATED: Used by deprecated analyze_query()
    #[allow(dead_code)]
    fn analyze_range_query(
        query_json: &Value,
        available_indexes: &[String],
    ) -> Option<(String, QueryPlan)> {
        if let Value::Object(ref map) = query_json {
            for (field, conditions) in map {
                if field.starts_with('$') {
                    continue; // Skip logical operators at root level
                }

                if let Value::Object(ref cond_map) = conditions {
                    // Check for range operators
                    let has_gt = cond_map.contains_key("$gt");
                    let has_gte = cond_map.contains_key("$gte");
                    let has_lt = cond_map.contains_key("$lt");
                    let has_lte = cond_map.contains_key("$lte");

                    if has_gt || has_gte || has_lt || has_lte {
                        // We have a range query
                        let index_name = Self::find_index_for_field(field, available_indexes)?;

                        let start = if has_gte {
                            cond_map.get("$gte").map(IndexKey::from)
                        } else if has_gt {
                            cond_map.get("$gt").map(IndexKey::from)
                        } else {
                            None
                        };

                        let end = if has_lte {
                            cond_map.get("$lte").map(IndexKey::from)
                        } else if has_lt {
                            cond_map.get("$lt").map(IndexKey::from)
                        } else {
                            None
                        };

                        let inclusive_start = has_gte || (!has_gt && !has_gte);
                        let inclusive_end = has_lte || (!has_lt && !has_lte);

                        return Some((
                            field.clone(),
                            QueryPlan::IndexRangeScan {
                                index_name,
                                field: field.clone(),
                                start,
                                end,
                                inclusive_start,
                                inclusive_end,
                            },
                        ));
                    }
                }
            }
        }

        None
    }

    /// Find an index for a given field
    ///
    /// DEPRECATED: This method has a bug with compound indexes - it matches
    /// any index ending with _{field}, even if the field is not the first
    /// (prefix) field of a compound index.
    ///
    /// Use find_index_for_field_v2 with index_fields parameter instead.
    #[allow(dead_code)]
    fn find_index_for_field(field: &str, available_indexes: &[String]) -> Option<String> {
        // Look for index ending with _{field}
        available_indexes
            .iter()
            .find(|idx| idx.ends_with(&format!("_{}", field)))
            .cloned()
    }

    /// Find an index for a given field (v2 - compound index aware)
    ///
    /// Takes a list of IndexPrefixInfo containing:
    /// - index_name: The index name
    /// - prefix_field: The first field (for compound) or only field (for single)
    /// - is_compound: Whether this is a compound index
    ///
    /// Returns (index_name, is_compound) if found.
    fn find_index_for_field_v2(
        field: &str,
        index_fields: &[IndexPrefixInfo],
    ) -> Option<(String, bool)> {
        index_fields
            .iter()
            .find(|info| info.prefix_field == field)
            .map(|info| (info.index_name.clone(), info.is_compound))
    }

    /// Analyze a query with compound-index-aware field matching (v2)
    ///
    /// This version takes IndexPrefixInfo to correctly handle compound indexes
    /// by using them for prefix field queries with range scans.
    pub fn analyze_query_with_fields(
        query_json: &Value,
        index_fields: &[IndexPrefixInfo],
    ) -> Option<(String, QueryPlan)> {
        // Check for simple equality query: { "field": value }
        if let Value::Object(ref map) = query_json {
            // First try range query analysis
            if let Some((field, plan)) = Self::analyze_range_query_v2(query_json, index_fields) {
                return Some((field, plan));
            }

            // Skip logical operators like $and, $or, $nor
            if map.keys().any(|k| k.starts_with('$')) {
                return None;
            }

            // Simple equality query: { "field": value }
            if let Some((field, value)) = map.iter().next() {
                // Skip if value contains operators (like {"age": {"$gt": 5}})
                if let Value::Object(ref val_map) = value {
                    if val_map.keys().any(|k| k.starts_with('$')) {
                        return None;
                    }
                }

                // Check if we have an index on this field (compound-aware!)
                let (index_name, is_compound) = Self::find_index_for_field_v2(field, index_fields)?;

                let key = IndexKey::from(value);
                return Some((
                    field.clone(),
                    QueryPlan::IndexScan {
                        index_name,
                        field: field.clone(),
                        key,
                        is_compound,
                    },
                ));
            }
        }

        None
    }

    /// Analyze query for range operators with compound-index-aware matching
    fn analyze_range_query_v2(
        query_json: &Value,
        index_fields: &[IndexPrefixInfo],
    ) -> Option<(String, QueryPlan)> {
        if let Value::Object(ref map) = query_json {
            for (field, conditions) in map {
                if field.starts_with('$') {
                    continue; // Skip logical operators at root level
                }

                if let Value::Object(ref cond_map) = conditions {
                    let has_gt = cond_map.contains_key("$gt");
                    let has_gte = cond_map.contains_key("$gte");
                    let has_lt = cond_map.contains_key("$lt");
                    let has_lte = cond_map.contains_key("$lte");

                    if has_gt || has_gte || has_lt || has_lte {
                        // Compound-index-aware field matching (we only need the index name for range queries)
                        let (index_name, _is_compound) =
                            Self::find_index_for_field_v2(field, index_fields)?;

                        let start = if has_gte {
                            cond_map.get("$gte").map(IndexKey::from)
                        } else if has_gt {
                            cond_map.get("$gt").map(IndexKey::from)
                        } else {
                            None
                        };

                        let end = if has_lte {
                            cond_map.get("$lte").map(IndexKey::from)
                        } else if has_lt {
                            cond_map.get("$lt").map(IndexKey::from)
                        } else {
                            None
                        };

                        let inclusive_start = has_gte || (!has_gt && !has_gte);
                        let inclusive_end = has_lte || (!has_lt && !has_lte);

                        return Some((
                            field.clone(),
                            QueryPlan::IndexRangeScan {
                                index_name,
                                field: field.clone(),
                                start,
                                end,
                                inclusive_start,
                                inclusive_end,
                            },
                        ));
                    }
                }
            }
        }

        None
    }

    /// Create a query plan description for explain output
    ///
    /// DEPRECATED: Use explain_query_with_fields() for compound index support
    #[deprecated(
        since = "0.3.0",
        note = "use explain_query_with_fields for compound index support"
    )]
    #[allow(deprecated)]
    #[allow(dead_code)]
    pub fn explain_query(query_json: &Value, available_indexes: &[String]) -> Value {
        use serde_json::json;

        #[allow(deprecated)]
        if let Some((field, plan)) = Self::analyze_query(query_json, available_indexes) {
            // Index-based plan
            match plan {
                QueryPlan::IndexScan {
                    ref index_name,
                    ref key,
                    ..
                } => {
                    json!({
                        "queryPlan": "IndexScan",
                        "indexUsed": index_name,
                        "field": field,
                        "stage": "FETCH_WITH_INDEX",
                        "indexType": "equality",
                        "searchKey": format!("{:?}", key),
                        "estimatedCost": "O(log n)",
                    })
                }
                QueryPlan::IndexRangeScan {
                    ref index_name,
                    ref start,
                    ref end,
                    inclusive_start,
                    inclusive_end,
                    ..
                } => {
                    json!({
                        "queryPlan": "IndexRangeScan",
                        "indexUsed": index_name,
                        "field": field,
                        "stage": "FETCH_WITH_INDEX",
                        "indexType": "range",
                        "range": {
                            "start": format!("{:?}", start),
                            "end": format!("{:?}", end),
                            "inclusiveStart": inclusive_start,
                            "inclusiveEnd": inclusive_end,
                        },
                        "estimatedCost": "O(log n + k)",
                    })
                } // NOTE: CollectionScan match arm removed - unreachable since analyze_query returns None for full scan
            }
        } else {
            // No index available
            json!({
                "queryPlan": "CollectionScan",
                "indexUsed": null,
                "stage": "FULL_SCAN",
                "reason": "No suitable index found for query",
                "estimatedCost": "O(n)",
                "availableIndexes": available_indexes,
            })
        }
    }

    /// Create a query plan description for explain output (v2 - compound index aware)
    ///
    /// Uses the new compound-index-aware query analysis for accurate explain output.
    pub fn explain_query_with_fields(
        query_json: &Value,
        index_fields: &[IndexPrefixInfo],
    ) -> Value {
        use serde_json::json;

        if let Some((field, plan)) = Self::analyze_query_with_fields(query_json, index_fields) {
            // Index-based plan
            match plan {
                QueryPlan::IndexScan {
                    ref index_name,
                    ref key,
                    is_compound,
                    ..
                } => {
                    json!({
                        "queryPlan": if is_compound { "CompoundIndexScan" } else { "IndexScan" },
                        "indexUsed": index_name,
                        "field": field,
                        "stage": "FETCH_WITH_INDEX",
                        "indexType": if is_compound { "compound_prefix" } else { "equality" },
                        "searchKey": format!("{:?}", key),
                        "estimatedCost": "O(log n)",
                    })
                }
                QueryPlan::IndexRangeScan {
                    ref index_name,
                    ref start,
                    ref end,
                    inclusive_start,
                    inclusive_end,
                    ..
                } => {
                    json!({
                        "queryPlan": "IndexRangeScan",
                        "indexUsed": index_name,
                        "field": field,
                        "stage": "FETCH_WITH_INDEX",
                        "indexType": "range",
                        "range": {
                            "start": format!("{:?}", start),
                            "end": format!("{:?}", end),
                            "inclusiveStart": inclusive_start,
                            "inclusiveEnd": inclusive_end,
                        },
                        "estimatedCost": "O(log n + k)",
                    })
                }
            }
        } else {
            // No index available
            let available: Vec<&str> = index_fields.iter().map(|i| i.index_name.as_str()).collect();
            json!({
                "queryPlan": "CollectionScan",
                "indexUsed": null,
                "stage": "FULL_SCAN",
                "reason": "No suitable index found for query",
                "estimatedCost": "O(n)",
                "availableIndexes": available,
            })
        }
    }
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_equality_query_analysis() {
        let query = json!({"age": 25});
        let indexes = vec!["users_age".to_string(), "users_id".to_string()];

        let result = QueryPlanner::analyze_query(&query, &indexes);
        assert!(result.is_some());

        let (field, plan) = result.unwrap();
        assert_eq!(field, "age");

        match plan {
            QueryPlan::IndexScan {
                index_name,
                field,
                key,
                ..
            } => {
                assert_eq!(index_name, "users_age");
                assert_eq!(field, "age");
                assert_eq!(key, IndexKey::Int(25));
            }
            _ => panic!("Expected IndexScan"),
        }
    }

    #[test]
    fn test_range_query_analysis() {
        let query = json!({"age": {"$gte": 18, "$lt": 65}});
        let indexes = vec!["users_age".to_string()];

        let result = QueryPlanner::analyze_query(&query, &indexes);
        assert!(result.is_some());

        let (field, plan) = result.unwrap();
        assert_eq!(field, "age");

        match plan {
            QueryPlan::IndexRangeScan {
                index_name,
                start,
                end,
                inclusive_start,
                inclusive_end,
                ..
            } => {
                assert_eq!(index_name, "users_age");
                assert_eq!(start, Some(IndexKey::Int(18)));
                assert_eq!(end, Some(IndexKey::Int(65)));
                assert!(inclusive_start);
                assert!(!inclusive_end);
            }
            _ => panic!("Expected IndexRangeScan"),
        }
    }

    #[test]
    fn test_no_index_available() {
        let query = json!({"name": "Alice"});
        let indexes = vec!["users_age".to_string()];

        let result = QueryPlanner::analyze_query(&query, &indexes);
        assert!(result.is_none());
    }

    #[test]
    fn test_complex_query_no_optimization() {
        let query = json!({"$and": [{"age": 25}, {"name": "Alice"}]});
        let indexes = vec!["users_age".to_string()];

        // Complex queries not yet supported
        let result = QueryPlanner::analyze_query(&query, &indexes);
        assert!(result.is_none());
    }
}
