//! Test for bug: $match fails when combining $in with comparison operators on different fields

use ironbase_core::{storage::MemoryStorage, DatabaseCore};
use serde_json::{json, Value};
use std::collections::HashMap;

fn json_to_hashmap(v: Value) -> HashMap<String, Value> {
    match v {
        Value::Object(map) => map.into_iter().collect(),
        _ => panic!("Expected object"),
    }
}

#[test]
fn test_match_in_combined_with_lt() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("test").unwrap();

    // Insert test data similar to tisza_vizallas
    for year in 2000..2010 {
        for month in 1..=12 {
            db.insert_one(
                "test",
                json_to_hashmap(json!({
                    "year": year,
                    "month": month,
                    "water_level_cm": month * 100 + year % 100
                })),
            )
            .unwrap();
        }
    }

    let total = coll.count_documents(&json!({})).unwrap();
    assert_eq!(total, 120); // 10 years * 12 months

    // Test 1: Single $in - should match months 1, 2, 3 = 30 docs
    let result1 = coll
        .aggregate(&json!([
            {"$match": {"month": {"$in": [1, 2, 3]}}},
            {"$group": {"_id": null, "count": {"$sum": 1}}}
        ]))
        .unwrap();
    let count1 = result1[0]["count"].as_i64().unwrap();
    assert_eq!(count1, 30, "Single $in should match 30 docs");

    // Test 2: Single $lt - water_level_cm < 500
    // water_level_cm = month * 100 + year % 100
    // For month=1: 100+0..100+9 = 100-109 (all < 500)
    // For month=2: 200+0..200+9 = 200-209 (all < 500)
    // For month=3: 300+0..300+9 = 300-309 (all < 500)
    // For month=4: 400+0..400+9 = 400-409 (all < 500)
    // For month=5+: 500+ (>= 500, not matched)
    // So 4 months * 10 years = 40 docs
    let result2 = coll
        .aggregate(&json!([
            {"$match": {"water_level_cm": {"$lt": 500}}},
            {"$group": {"_id": null, "count": {"$sum": 1}}}
        ]))
        .unwrap();
    let count2 = result2[0]["count"].as_i64().unwrap();
    assert_eq!(count2, 40, "Single $lt should match 40 docs");

    // Test 3: Combined $in + $lt (THE BUG)
    // Should match: month in [1,2,3] AND water_level_cm < 500
    // All month 1,2,3 have water_level < 500, so should be 30 docs
    let result3 = coll
        .aggregate(&json!([
            {"$match": {"month": {"$in": [1, 2, 3]}, "water_level_cm": {"$lt": 500}}}
        ]))
        .unwrap();

    // Test 4: Separate $match stages (workaround)
    let result4 = coll
        .aggregate(&json!([
            {"$match": {"month": {"$in": [1, 2, 3]}}},
            {"$match": {"water_level_cm": {"$lt": 500}}}
        ]))
        .unwrap();

    // Both should return the same number of documents
    assert_eq!(
        result3.len(),
        result4.len(),
        "BUG: Combined $match returned {} results, separate $match returned {} results",
        result3.len(),
        result4.len()
    );

    // Both should return 30 documents (all months 1,2,3 have water_level < 500)
    assert_eq!(result3.len(), 30, "Combined $match should return 30 docs");
    assert_eq!(result4.len(), 30, "Separate $match should return 30 docs");
}

#[test]
fn test_match_in_combined_with_gt() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("test").unwrap();

    for year in 2000..2010 {
        for month in 1..=12 {
            db.insert_one(
                "test",
                json_to_hashmap(json!({
                    "year": year,
                    "month": month,
                    "water_level_cm": month * 100 + year % 100
                })),
            )
            .unwrap();
        }
    }

    // Combined $in + $gt
    let result_combined = coll
        .aggregate(&json!([
            {"$match": {"month": {"$in": [10, 11, 12]}, "water_level_cm": {"$gt": 1000}}}
        ]))
        .unwrap();

    // Separate $match stages
    let result_separate = coll
        .aggregate(&json!([
            {"$match": {"month": {"$in": [10, 11, 12]}}},
            {"$match": {"water_level_cm": {"$gt": 1000}}}
        ]))
        .unwrap();

    assert_eq!(
        result_combined.len(),
        result_separate.len(),
        "BUG: Combined returned {}, separate returned {}",
        result_combined.len(),
        result_separate.len()
    );
}

#[test]
fn test_match_multiple_fields_no_in() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("test").unwrap();

    for i in 0..100 {
        db.insert_one(
            "test",
            json_to_hashmap(json!({
                "a": i % 10,
                "b": i % 5
            })),
        )
        .unwrap();
    }

    // Combined conditions without $in
    let result_combined = coll
        .aggregate(&json!([
            {"$match": {"a": {"$lt": 5}, "b": {"$gt": 2}}}
        ]))
        .unwrap();

    let result_separate = coll
        .aggregate(&json!([
            {"$match": {"a": {"$lt": 5}}},
            {"$match": {"b": {"$gt": 2}}}
        ]))
        .unwrap();

    assert_eq!(
        result_combined.len(),
        result_separate.len(),
        "Combined without $in should work: combined={}, separate={}",
        result_combined.len(),
        result_separate.len()
    );
}

#[test]
fn test_match_in_combined_with_index() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("test").unwrap();

    // Create index on month field (similar to real-world scenario)
    coll.create_index("month".to_string(), false, false)
        .unwrap();

    // Insert test data
    for year in 2000..2010 {
        for month in 1..=12 {
            db.insert_one(
                "test",
                json_to_hashmap(json!({
                    "year": year,
                    "month": month,
                    "water_level_cm": month * 100 + year % 100
                })),
            )
            .unwrap();
        }
    }

    // Combined $in + $lt with index on month
    let result_combined = coll
        .aggregate(&json!([
            {"$match": {"month": {"$in": [1, 2, 3]}, "water_level_cm": {"$lt": 500}}}
        ]))
        .unwrap();

    // Separate $match stages
    let result_separate = coll
        .aggregate(&json!([
            {"$match": {"month": {"$in": [1, 2, 3]}}},
            {"$match": {"water_level_cm": {"$lt": 500}}}
        ]))
        .unwrap();

    assert_eq!(
        result_combined.len(),
        result_separate.len(),
        "BUG WITH INDEX: Combined returned {}, separate returned {}",
        result_combined.len(),
        result_separate.len()
    );

    assert_eq!(
        result_combined.len(),
        30,
        "Combined with index should return 30 docs"
    );
}

#[test]
fn test_match_in_combined_with_compound_index() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("test").unwrap();

    // Create compound index (month, water_level_cm)
    coll.create_compound_index(
        vec!["month".to_string(), "water_level_cm".to_string()],
        false,
        false,
    )
    .unwrap();

    // Insert test data
    for year in 2000..2010 {
        for month in 1..=12 {
            db.insert_one(
                "test",
                json_to_hashmap(json!({
                    "year": year,
                    "month": month,
                    "water_level_cm": month * 100 + year % 100
                })),
            )
            .unwrap();
        }
    }

    // Combined $in + $lt with compound index
    let result_combined = coll
        .aggregate(&json!([
            {"$match": {"month": {"$in": [1, 2, 3]}, "water_level_cm": {"$lt": 500}}}
        ]))
        .unwrap();

    // Separate $match stages
    let result_separate = coll
        .aggregate(&json!([
            {"$match": {"month": {"$in": [1, 2, 3]}}},
            {"$match": {"water_level_cm": {"$lt": 500}}}
        ]))
        .unwrap();

    assert_eq!(
        result_combined.len(),
        result_separate.len(),
        "BUG WITH COMPOUND INDEX: Combined returned {}, separate returned {}",
        result_combined.len(),
        result_separate.len()
    );
}
