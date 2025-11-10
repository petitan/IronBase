// src/query_planner.rs
// Query planner and optimizer - index selection

use serde_json::Value;
use crate::index::IndexKey;

/// Query plan - describes how to execute a query
#[derive(Debug, Clone)]
pub enum QueryPlan {
    /// Full collection scan (no index used)
    CollectionScan,

    /// Index scan for equality match
    IndexScan {
        index_name: String,
        field: String,
        key: IndexKey,
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
    pub fn analyze_query(query_json: &Value, available_indexes: &[String]) -> Option<(String, QueryPlan)> {
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
                    }
                ));
            }
        }

        None
    }

    /// Analyze query for range operators ($gt, $gte, $lt, $lte)
    fn analyze_range_query(query_json: &Value, available_indexes: &[String]) -> Option<(String, QueryPlan)> {
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
                            }
                        ));
                    }
                }
            }
        }

        None
    }

    /// Find an index for a given field
    fn find_index_for_field(field: &str, available_indexes: &[String]) -> Option<String> {
        // Look for index ending with _{field}
        available_indexes.iter()
            .find(|idx| idx.ends_with(&format!("_{}", field)))
            .cloned()
    }

    /// Create a query plan description for explain output
    pub fn explain_query(query_json: &Value, available_indexes: &[String]) -> Value {
        use serde_json::json;

        if let Some((field, plan)) = Self::analyze_query(query_json, available_indexes) {
            // Index-based plan
            match plan {
                QueryPlan::IndexScan { ref index_name, ref key, .. } => {
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
                QueryPlan::IndexRangeScan { ref index_name, ref start, ref end, inclusive_start, inclusive_end, .. } => {
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
                QueryPlan::CollectionScan => {
                    json!({
                        "queryPlan": "CollectionScan",
                        "indexUsed": null,
                        "stage": "FULL_SCAN",
                        "reason": "No suitable index",
                        "estimatedCost": "O(n)",
                    })
                }
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
}

#[cfg(test)]
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
            QueryPlan::IndexScan { index_name, field, key } => {
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
            QueryPlan::IndexRangeScan { index_name, start, end, inclusive_start, inclusive_end, .. } => {
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
