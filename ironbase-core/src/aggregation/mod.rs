// src/aggregation/mod.rs
// Aggregation pipeline implementation
//
// This module provides MongoDB-style aggregation pipelines for data transformation
// and analysis. Supported stages: $match, $project, $group, $sort, $limit, $skip, $unwind

mod helpers;
mod pipeline;
mod stages;
mod types;

// Re-export public types
pub use types::Pipeline;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ========== Pipeline tests ==========

    #[test]
    fn test_pipeline_not_array() {
        let result = Pipeline::from_json(&json!({"$match": {}}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("must be an array"));
    }

    #[test]
    fn test_pipeline_empty() {
        let result = Pipeline::from_json(&json!([]));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cannot be empty"));
    }

    // ========== $match tests ==========

    #[test]
    fn test_match_simple() {
        let docs = vec![
            json!({"name": "Alice", "age": 30}),
            json!({"name": "Bob", "age": 25}),
            json!({"name": "Charlie", "age": 35}),
        ];

        let pipeline = Pipeline::from_json(&json!([
            {"$match": {"age": {"$gt": 28}}}
        ]))
        .unwrap();

        let results = pipeline.execute(docs).unwrap();
        assert_eq!(results.len(), 2);
    }

    // ========== $limit and $skip tests ==========

    #[test]
    fn test_limit() {
        let docs = vec![
            json!({"x": 1}),
            json!({"x": 2}),
            json!({"x": 3}),
            json!({"x": 4}),
            json!({"x": 5}),
        ];

        let pipeline = Pipeline::from_json(&json!([{"$limit": 3}])).unwrap();
        let results = pipeline.execute(docs).unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0]["x"], 1);
        assert_eq!(results[2]["x"], 3);
    }

    #[test]
    fn test_skip() {
        let docs = vec![
            json!({"x": 1}),
            json!({"x": 2}),
            json!({"x": 3}),
            json!({"x": 4}),
            json!({"x": 5}),
        ];

        let pipeline = Pipeline::from_json(&json!([{"$skip": 2}])).unwrap();
        let results = pipeline.execute(docs).unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0]["x"], 3);
    }

    // ========== $sort tests ==========

    #[test]
    fn test_sort_ascending() {
        let docs = vec![
            json!({"name": "Charlie", "age": 35}),
            json!({"name": "Alice", "age": 30}),
            json!({"name": "Bob", "age": 25}),
        ];

        let pipeline = Pipeline::from_json(&json!([{"$sort": {"age": 1}}])).unwrap();
        let results = pipeline.execute(docs).unwrap();
        assert_eq!(results[0]["name"], "Bob");
        assert_eq!(results[1]["name"], "Alice");
        assert_eq!(results[2]["name"], "Charlie");
    }

    #[test]
    fn test_sort_descending() {
        let docs = vec![
            json!({"name": "Alice", "age": 30}),
            json!({"name": "Bob", "age": 25}),
            json!({"name": "Charlie", "age": 35}),
        ];

        let pipeline = Pipeline::from_json(&json!([{"$sort": {"age": -1}}])).unwrap();
        let results = pipeline.execute(docs).unwrap();
        assert_eq!(results[0]["name"], "Charlie");
        assert_eq!(results[1]["name"], "Alice");
        assert_eq!(results[2]["name"], "Bob");
    }

    // ========== $group tests ==========

    #[test]
    fn test_group_count() {
        let docs = vec![
            json!({"city": "NYC", "amount": 100}),
            json!({"city": "LA", "amount": 200}),
            json!({"city": "NYC", "amount": 150}),
        ];

        let pipeline = Pipeline::from_json(&json!([
            {"$group": {"_id": "$city", "count": {"$sum": 1}}}
        ]))
        .unwrap();

        let results = pipeline.execute(docs).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_group_sum() {
        let docs = vec![
            json!({"city": "NYC", "amount": 100}),
            json!({"city": "LA", "amount": 200}),
            json!({"city": "NYC", "amount": 150}),
        ];

        let pipeline = Pipeline::from_json(&json!([
            {"$group": {"_id": "$city", "total": {"$sum": "$amount"}}}
        ]))
        .unwrap();

        let results = pipeline.execute(docs).unwrap();

        for result in &results {
            if result["_id"] == "NYC" {
                assert_eq!(result["total"], 250);
            } else if result["_id"] == "LA" {
                assert_eq!(result["total"], 200);
            }
        }
    }

    // ========== $project tests ==========

    #[test]
    fn test_project_include() {
        let docs = vec![json!({"name": "Alice", "age": 30, "city": "NYC"})];

        let pipeline = Pipeline::from_json(&json!([{"$project": {"name": 1, "age": 1}}])).unwrap();

        let results = pipeline.execute(docs).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].get("name").is_some());
        assert!(results[0].get("age").is_some());
        assert!(results[0].get("city").is_none());
    }

    #[test]
    fn test_project_rename() {
        let docs = vec![json!({"name": "Alice", "age": 30})];

        let pipeline =
            Pipeline::from_json(&json!([{"$project": {"fullName": "$name", "years": "$age"}}]))
                .unwrap();

        let results = pipeline.execute(docs).unwrap();
        assert_eq!(results[0]["fullName"], "Alice");
        assert_eq!(results[0]["years"], 30);
    }

    // ========== $unwind tests ==========

    #[test]
    fn test_unwind_simple() {
        let docs = vec![json!({"name": "Alice", "tags": ["a", "b", "c"]})];

        let pipeline = Pipeline::from_json(&json!([{"$unwind": "$tags"}])).unwrap();

        let results = pipeline.execute(docs).unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0]["tags"], "a");
        assert_eq!(results[1]["tags"], "b");
        assert_eq!(results[2]["tags"], "c");
    }

    #[test]
    fn test_unwind_with_index() {
        let docs = vec![json!({"name": "Alice", "items": [10, 20, 30]})];

        let pipeline = Pipeline::from_json(&json!([
            {"$unwind": {"path": "$items", "includeArrayIndex": "idx"}}
        ]))
        .unwrap();

        let results = pipeline.execute(docs).unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0]["idx"], 0);
        assert_eq!(results[1]["idx"], 1);
        assert_eq!(results[2]["idx"], 2);
    }
}
