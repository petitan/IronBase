// Limit, Skip and Cursor E2E Integration Tests
// Tests for: FindOptions (limit, skip, sort), FindCursor, pagination patterns

use ironbase_core::FindOptions;
use ironbase_core::StorageEngine;
use serde_json::json;
use std::collections::HashMap;
use tempfile::TempDir;

type DatabaseCore = ironbase_core::DatabaseCore<StorageEngine>;

fn insert_doc(db: &DatabaseCore, coll: &str, doc: serde_json::Value) {
    if let serde_json::Value::Object(map) = doc {
        let fields: HashMap<String, serde_json::Value> = map.into_iter().collect();
        db.insert_one(coll, fields).unwrap();
    }
}

fn setup_test_db(name: &str) -> (DatabaseCore, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join(format!("{}.mlite", name));
    let db = DatabaseCore::open(&db_path).unwrap();
    (db, temp_dir)
}

// ============================================================================
// BASIC LIMIT TESTS
// ============================================================================

#[test]
fn test_limit_basic() {
    let (db, _temp) = setup_test_db("limit_basic");

    for i in 1..=10 {
        insert_doc(&db, "items", json!({"n": i, "name": format!("Item{}", i)}));
    }

    let collection = db.collection("items").unwrap();
    let options = FindOptions::new().with_limit(5);
    let results = collection.find_with_options(&json!({}), options).unwrap();

    assert_eq!(results.len(), 5);
}

#[test]
fn test_limit_exceeds_total() {
    let (db, _temp) = setup_test_db("limit_exceeds");

    for i in 1..=5 {
        insert_doc(&db, "items", json!({"n": i}));
    }

    let collection = db.collection("items").unwrap();
    let options = FindOptions::new().with_limit(100);
    let results = collection.find_with_options(&json!({}), options).unwrap();

    assert_eq!(results.len(), 5); // Returns all 5, not 100
}

/// MongoDB compatibility: limit(0) means "no limit" (returns all documents)
/// See: https://www.mongodb.com/docs/manual/reference/method/cursor.limit/
#[test]
fn test_limit_zero_returns_all() {
    let (db, _temp) = setup_test_db("limit_zero");

    for i in 1..=5 {
        insert_doc(&db, "items", json!({"n": i}));
    }

    let collection = db.collection("items").unwrap();
    let options = FindOptions::new().with_limit(0);
    let results = collection.find_with_options(&json!({}), options).unwrap();

    // limit(0) in MongoDB means "no limit" - returns all documents
    assert_eq!(results.len(), 5);
}

#[test]
fn test_limit_one() {
    let (db, _temp) = setup_test_db("limit_one");

    for i in 1..=10 {
        insert_doc(&db, "items", json!({"n": i}));
    }

    let collection = db.collection("items").unwrap();
    let options = FindOptions::new().with_limit(1);
    let results = collection.find_with_options(&json!({}), options).unwrap();

    assert_eq!(results.len(), 1);
}

// ============================================================================
// BASIC SKIP TESTS
// ============================================================================

#[test]
fn test_skip_basic() {
    let (db, _temp) = setup_test_db("skip_basic");

    for i in 1..=10 {
        insert_doc(&db, "items", json!({"n": i}));
    }

    let collection = db.collection("items").unwrap();
    let options = FindOptions::new()
        .with_sort(vec![("n".to_string(), 1)])
        .with_skip(3);
    let results = collection.find_with_options(&json!({}), options).unwrap();

    assert_eq!(results.len(), 7);
    assert_eq!(results[0].get("n").unwrap().as_i64().unwrap(), 4);
}

#[test]
fn test_skip_all() {
    let (db, _temp) = setup_test_db("skip_all");

    for i in 1..=5 {
        insert_doc(&db, "items", json!({"n": i}));
    }

    let collection = db.collection("items").unwrap();
    let options = FindOptions::new().with_skip(10);
    let results = collection.find_with_options(&json!({}), options).unwrap();

    assert_eq!(results.len(), 0);
}

#[test]
fn test_skip_zero() {
    let (db, _temp) = setup_test_db("skip_zero");

    for i in 1..=5 {
        insert_doc(&db, "items", json!({"n": i}));
    }

    let collection = db.collection("items").unwrap();
    let options = FindOptions::new().with_skip(0);
    let results = collection.find_with_options(&json!({}), options).unwrap();

    assert_eq!(results.len(), 5);
}

// ============================================================================
// LIMIT + SKIP PAGINATION TESTS
// ============================================================================

#[test]
fn test_pagination_first_page() {
    let (db, _temp) = setup_test_db("pagination_first");

    for i in 1..=100 {
        insert_doc(&db, "items", json!({"n": i}));
    }

    let collection = db.collection("items").unwrap();
    let page_size = 10;
    let page = 0;

    let options = FindOptions::new()
        .with_sort(vec![("n".to_string(), 1)])
        .with_skip(page * page_size)
        .with_limit(page_size);

    let results = collection.find_with_options(&json!({}), options).unwrap();

    assert_eq!(results.len(), 10);
    assert_eq!(results[0].get("n").unwrap().as_i64().unwrap(), 1);
    assert_eq!(results[9].get("n").unwrap().as_i64().unwrap(), 10);
}

#[test]
fn test_pagination_middle_page() {
    let (db, _temp) = setup_test_db("pagination_middle");

    for i in 1..=100 {
        insert_doc(&db, "items", json!({"n": i}));
    }

    let collection = db.collection("items").unwrap();
    let page_size = 10;
    let page = 5; // 6th page (0-indexed)

    let options = FindOptions::new()
        .with_sort(vec![("n".to_string(), 1)])
        .with_skip(page * page_size)
        .with_limit(page_size);

    let results = collection.find_with_options(&json!({}), options).unwrap();

    assert_eq!(results.len(), 10);
    assert_eq!(results[0].get("n").unwrap().as_i64().unwrap(), 51);
    assert_eq!(results[9].get("n").unwrap().as_i64().unwrap(), 60);
}

#[test]
fn test_pagination_last_page_partial() {
    let (db, _temp) = setup_test_db("pagination_last");

    for i in 1..=95 {
        insert_doc(&db, "items", json!({"n": i}));
    }

    let collection = db.collection("items").unwrap();
    let page_size = 10;
    let page = 9; // 10th page (0-indexed)

    let options = FindOptions::new()
        .with_sort(vec![("n".to_string(), 1)])
        .with_skip(page * page_size)
        .with_limit(page_size);

    let results = collection.find_with_options(&json!({}), options).unwrap();

    assert_eq!(results.len(), 5); // Only 5 items left
    assert_eq!(results[0].get("n").unwrap().as_i64().unwrap(), 91);
    assert_eq!(results[4].get("n").unwrap().as_i64().unwrap(), 95);
}

#[test]
fn test_pagination_beyond_total() {
    let (db, _temp) = setup_test_db("pagination_beyond");

    for i in 1..=50 {
        insert_doc(&db, "items", json!({"n": i}));
    }

    let collection = db.collection("items").unwrap();
    let page_size = 10;
    let page = 10; // Beyond the data

    let options = FindOptions::new()
        .with_sort(vec![("n".to_string(), 1)])
        .with_skip(page * page_size)
        .with_limit(page_size);

    let results = collection.find_with_options(&json!({}), options).unwrap();

    assert_eq!(results.len(), 0);
}

#[test]
fn test_pagination_iterate_all_pages() {
    let (db, _temp) = setup_test_db("pagination_iterate");

    for i in 1..=57 {
        insert_doc(&db, "items", json!({"n": i}));
    }

    let collection = db.collection("items").unwrap();
    let page_size = 10;
    let mut all_items = Vec::new();
    let mut page = 0;

    loop {
        let options = FindOptions::new()
            .with_sort(vec![("n".to_string(), 1)])
            .with_skip(page * page_size)
            .with_limit(page_size);

        let results = collection.find_with_options(&json!({}), options).unwrap();

        if results.is_empty() {
            break;
        }

        all_items.extend(results);
        page += 1;
    }

    assert_eq!(all_items.len(), 57);
    assert_eq!(page, 6); // 6 pages: 10+10+10+10+10+7
}

// ============================================================================
// LIMIT + SKIP WITH QUERY FILTER
// ============================================================================

#[test]
fn test_limit_with_filter() {
    let (db, _temp) = setup_test_db("limit_filter");

    for i in 1..=20 {
        let category = if i % 2 == 0 { "even" } else { "odd" };
        insert_doc(&db, "items", json!({"n": i, "category": category}));
    }

    let collection = db.collection("items").unwrap();
    let options = FindOptions::new()
        .with_sort(vec![("n".to_string(), 1)])
        .with_limit(3);

    let results = collection
        .find_with_options(&json!({"category": "even"}), options)
        .unwrap();

    assert_eq!(results.len(), 3);
    assert_eq!(results[0].get("n").unwrap().as_i64().unwrap(), 2);
    assert_eq!(results[1].get("n").unwrap().as_i64().unwrap(), 4);
    assert_eq!(results[2].get("n").unwrap().as_i64().unwrap(), 6);
}

#[test]
fn test_skip_with_filter() {
    let (db, _temp) = setup_test_db("skip_filter");

    for i in 1..=20 {
        let category = if i % 2 == 0 { "even" } else { "odd" };
        insert_doc(&db, "items", json!({"n": i, "category": category}));
    }

    let collection = db.collection("items").unwrap();
    let options = FindOptions::new()
        .with_sort(vec![("n".to_string(), 1)])
        .with_skip(5);

    let results = collection
        .find_with_options(&json!({"category": "even"}), options)
        .unwrap();

    assert_eq!(results.len(), 5); // 10 even numbers, skip 5, get 5
    assert_eq!(results[0].get("n").unwrap().as_i64().unwrap(), 12);
}

#[test]
fn test_pagination_with_filter() {
    let (db, _temp) = setup_test_db("pagination_filter");

    for i in 1..=100 {
        let status = if i % 3 == 0 { "active" } else { "inactive" };
        insert_doc(&db, "items", json!({"n": i, "status": status}));
    }

    let collection = db.collection("items").unwrap();
    let page_size = 5;
    let page = 2;

    let options = FindOptions::new()
        .with_sort(vec![("n".to_string(), 1)])
        .with_skip(page * page_size)
        .with_limit(page_size);

    // 33 active items (multiples of 3)
    let results = collection
        .find_with_options(&json!({"status": "active"}), options)
        .unwrap();

    assert_eq!(results.len(), 5);
    // Skip 10, so start at 11th active (n=33)
    assert_eq!(results[0].get("n").unwrap().as_i64().unwrap(), 33);
}

// ============================================================================
// CURSOR BASIC TESTS
// ============================================================================

#[test]
fn test_cursor_next() {
    let (db, _temp) = setup_test_db("cursor_next");

    for i in 1..=5 {
        insert_doc(&db, "items", json!({"n": i}));
    }

    let collection = db.collection("items").unwrap();
    let mut cursor = collection.find_streaming(&json!({})).unwrap();

    let mut count = 0;
    while let Some(_doc) = cursor.next().unwrap() {
        count += 1;
    }

    assert_eq!(count, 5);
}

#[test]
fn test_cursor_total_and_remaining() {
    let (db, _temp) = setup_test_db("cursor_total");

    for i in 1..=10 {
        insert_doc(&db, "items", json!({"n": i}));
    }

    let collection = db.collection("items").unwrap();
    let mut cursor = collection.find_streaming(&json!({})).unwrap();

    assert_eq!(cursor.total(), 10);
    assert_eq!(cursor.remaining(), 10);
    assert_eq!(cursor.position(), 0);

    cursor.next().unwrap();
    cursor.next().unwrap();
    cursor.next().unwrap();

    assert_eq!(cursor.total(), 10);
    assert_eq!(cursor.remaining(), 7);
    assert_eq!(cursor.position(), 3);
}

#[test]
fn test_cursor_is_finished() {
    let (db, _temp) = setup_test_db("cursor_finished");

    for i in 1..=3 {
        insert_doc(&db, "items", json!({"n": i}));
    }

    let collection = db.collection("items").unwrap();
    let mut cursor = collection.find_streaming(&json!({})).unwrap();

    assert!(!cursor.is_finished());
    cursor.next().unwrap();
    assert!(!cursor.is_finished());
    cursor.next().unwrap();
    assert!(!cursor.is_finished());
    cursor.next().unwrap();
    assert!(cursor.is_finished());
}

#[test]
fn test_cursor_rewind() {
    let (db, _temp) = setup_test_db("cursor_rewind");

    for i in 1..=5 {
        insert_doc(&db, "items", json!({"n": i}));
    }

    let collection = db.collection("items").unwrap();
    let mut cursor = collection.find_streaming(&json!({})).unwrap();

    // Consume all
    while cursor.next().unwrap().is_some() {}
    assert!(cursor.is_finished());

    // Rewind
    cursor.rewind();
    assert!(!cursor.is_finished());
    assert_eq!(cursor.position(), 0);
    assert_eq!(cursor.remaining(), 5);

    // Can iterate again
    let mut count = 0;
    while cursor.next().unwrap().is_some() {
        count += 1;
    }
    assert_eq!(count, 5);
}

// ============================================================================
// CURSOR BATCH/CHUNK TESTS
// ============================================================================

#[test]
fn test_cursor_next_chunk() {
    let (db, _temp) = setup_test_db("cursor_chunk");

    for i in 1..=25 {
        insert_doc(&db, "items", json!({"n": i}));
    }

    let collection = db.collection("items").unwrap();
    let mut cursor = collection.find_streaming(&json!({})).unwrap();

    let batch1 = cursor.next_chunk(10).unwrap();
    assert_eq!(batch1.len(), 10);

    let batch2 = cursor.next_chunk(10).unwrap();
    assert_eq!(batch2.len(), 10);

    let batch3 = cursor.next_chunk(10).unwrap();
    assert_eq!(batch3.len(), 5); // Only 5 remaining

    let batch4 = cursor.next_chunk(10).unwrap();
    assert_eq!(batch4.len(), 0); // Empty
}

#[test]
fn test_cursor_next_batch_default_size() {
    let (db, _temp) = setup_test_db("cursor_batch");

    for i in 1..=150 {
        insert_doc(&db, "items", json!({"n": i}));
    }

    let collection = db.collection("items").unwrap();
    let mut cursor = collection.find_streaming(&json!({})).unwrap();

    let batch = cursor.next_batch().unwrap();
    assert!(!batch.is_empty());
    assert!(batch.len() <= 100); // Default batch size is 100
}

#[test]
fn test_cursor_with_batch_size() {
    let (db, _temp) = setup_test_db("cursor_custom_batch");

    for i in 1..=50 {
        insert_doc(&db, "items", json!({"n": i}));
    }

    let collection = db.collection("items").unwrap();
    let mut cursor = collection
        .find_streaming(&json!({}))
        .unwrap()
        .with_batch_size(15);

    let batch = cursor.next_batch().unwrap();
    assert_eq!(batch.len(), 15);
}

#[test]
fn test_cursor_batch_iterate_all() {
    let (db, _temp) = setup_test_db("cursor_batch_all");

    for i in 1..=73 {
        insert_doc(&db, "items", json!({"n": i}));
    }

    let collection = db.collection("items").unwrap();
    let mut cursor = collection
        .find_streaming(&json!({}))
        .unwrap()
        .with_batch_size(20);

    let mut total = 0;
    let mut batch_count = 0;

    while !cursor.is_finished() {
        let batch = cursor.next_batch().unwrap();
        total += batch.len();
        batch_count += 1;
    }

    assert_eq!(total, 73);
    assert_eq!(batch_count, 4); // 20+20+20+13
}

// ============================================================================
// CURSOR WITH QUERY FILTER
// ============================================================================

#[test]
fn test_cursor_with_filter() {
    let (db, _temp) = setup_test_db("cursor_filter");

    for i in 1..=30 {
        let category = if i % 2 == 0 { "even" } else { "odd" };
        insert_doc(&db, "items", json!({"n": i, "category": category}));
    }

    let collection = db.collection("items").unwrap();
    let mut cursor = collection
        .find_streaming(&json!({"category": "even"}))
        .unwrap();

    assert_eq!(cursor.total(), 15);

    let mut count = 0;
    while cursor.next().unwrap().is_some() {
        count += 1;
    }
    assert_eq!(count, 15);
}

#[test]
fn test_cursor_with_complex_filter() {
    let (db, _temp) = setup_test_db("cursor_complex");

    for i in 1..=100 {
        insert_doc(
            &db,
            "items",
            json!({
                "n": i,
                "score": i * 10,
                "active": i % 3 == 0
            }),
        );
    }

    let collection = db.collection("items").unwrap();

    // Find active items with score > 500
    let query = json!({
        "$and": [
            {"active": true},
            {"score": {"$gt": 500}}
        ]
    });

    let mut cursor = collection.find_streaming(&query).unwrap();

    let mut results = Vec::new();
    while let Some(doc) = cursor.next().unwrap() {
        results.push(doc);
    }

    // Active: multiples of 3 (3,6,9,...,99)
    // Score > 500: n > 50, so n in {51,54,57,60,63,...,99}
    // That's 17 items (51/3=17 through 99/3=33, so 33-16=17 items)
    assert_eq!(results.len(), 17);
}

// ============================================================================
// SORT + LIMIT + SKIP COMBINED
// ============================================================================

#[test]
fn test_sort_limit_skip_combined() {
    let (db, _temp) = setup_test_db("combined");

    insert_doc(&db, "users", json!({"name": "Alice", "score": 85}));
    insert_doc(&db, "users", json!({"name": "Bob", "score": 92}));
    insert_doc(&db, "users", json!({"name": "Charlie", "score": 78}));
    insert_doc(&db, "users", json!({"name": "Diana", "score": 95}));
    insert_doc(&db, "users", json!({"name": "Eve", "score": 88}));

    let collection = db.collection("users").unwrap();

    // Sort by score descending, skip 1, limit 2
    let options = FindOptions::new()
        .with_sort(vec![("score".to_string(), -1)])
        .with_skip(1)
        .with_limit(2);

    let results = collection.find_with_options(&json!({}), options).unwrap();

    // Sorted: Diana(95), Bob(92), Eve(88), Alice(85), Charlie(78)
    // Skip 1: Bob(92), Eve(88), Alice(85), Charlie(78)
    // Limit 2: Bob(92), Eve(88)

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].get("name").unwrap().as_str().unwrap(), "Bob");
    assert_eq!(results[1].get("name").unwrap().as_str().unwrap(), "Eve");
}

#[test]
fn test_multi_sort_limit_skip() {
    let (db, _temp) = setup_test_db("multi_sort");

    insert_doc(
        &db,
        "users",
        json!({"dept": "Sales", "name": "Alice", "score": 85}),
    );
    insert_doc(
        &db,
        "users",
        json!({"dept": "Sales", "name": "Bob", "score": 90}),
    );
    insert_doc(
        &db,
        "users",
        json!({"dept": "Engineering", "name": "Charlie", "score": 88}),
    );
    insert_doc(
        &db,
        "users",
        json!({"dept": "Engineering", "name": "Diana", "score": 92}),
    );
    insert_doc(
        &db,
        "users",
        json!({"dept": "Sales", "name": "Eve", "score": 80}),
    );

    let collection = db.collection("users").unwrap();

    // Sort by dept ascending, then score descending; skip 1, limit 3
    let options = FindOptions::new()
        .with_sort(vec![("dept".to_string(), 1), ("score".to_string(), -1)])
        .with_skip(1)
        .with_limit(3);

    let results = collection.find_with_options(&json!({}), options).unwrap();

    // Sorted: Engineering-Diana(92), Engineering-Charlie(88), Sales-Bob(90), Sales-Alice(85), Sales-Eve(80)
    // Skip 1: Engineering-Charlie(88), Sales-Bob(90), Sales-Alice(85), Sales-Eve(80)
    // Limit 3: Engineering-Charlie(88), Sales-Bob(90), Sales-Alice(85)

    assert_eq!(results.len(), 3);
    assert_eq!(results[0].get("name").unwrap().as_str().unwrap(), "Charlie");
    assert_eq!(results[1].get("name").unwrap().as_str().unwrap(), "Bob");
    assert_eq!(results[2].get("name").unwrap().as_str().unwrap(), "Alice");
}

// ============================================================================
// PROJECTION WITH LIMIT/SKIP
// ============================================================================

#[test]
fn test_projection_with_limit() {
    let (db, _temp) = setup_test_db("proj_limit");

    for i in 1..=10 {
        insert_doc(
            &db,
            "items",
            json!({
                "n": i,
                "name": format!("Item{}", i),
                "description": "Long description here...",
                "secret": "hidden"
            }),
        );
    }

    let collection = db.collection("items").unwrap();

    let mut projection = HashMap::new();
    projection.insert("name".to_string(), 1);
    projection.insert("n".to_string(), 1);

    let options = FindOptions::new()
        .with_projection(projection)
        .with_sort(vec![("n".to_string(), 1)])
        .with_limit(3);

    let results = collection.find_with_options(&json!({}), options).unwrap();

    assert_eq!(results.len(), 3);

    for doc in &results {
        assert!(doc.get("name").is_some());
        assert!(doc.get("n").is_some());
        assert!(doc.get("description").is_none());
        assert!(doc.get("secret").is_none());
    }
}

// ============================================================================
// INCLUDE_TOTAL FOR PAGINATION
// ============================================================================

#[test]
fn test_pagination_with_total_count() {
    let (db, _temp) = setup_test_db("pagination_total");

    for i in 1..=50 {
        insert_doc(&db, "items", json!({"n": i}));
    }

    let collection = db.collection("items").unwrap();

    // Get total count first (typical pagination pattern)
    let total = collection.count_documents(&json!({})).unwrap();
    assert_eq!(total, 50);

    // Then get paginated results
    let options = FindOptions::new()
        .with_sort(vec![("n".to_string(), 1)])
        .with_skip(10)
        .with_limit(5);
    let results = collection.find_with_options(&json!({}), options).unwrap();

    assert_eq!(results.len(), 5);
    // Should be items 11-15
    assert_eq!(results[0].get("n").unwrap().as_i64().unwrap(), 11);
    assert_eq!(results[4].get("n").unwrap().as_i64().unwrap(), 15);
}

#[test]
fn test_pagination_with_filtered_total_count() {
    let (db, _temp) = setup_test_db("pagination_filter_total");

    for i in 1..=100 {
        let status = if i % 4 == 0 { "active" } else { "inactive" };
        insert_doc(&db, "items", json!({"n": i, "status": status}));
    }

    let collection = db.collection("items").unwrap();
    let filter = json!({"status": "active"});

    // Get filtered total count first
    let total = collection.count_documents(&filter).unwrap();
    // 25 active items (multiples of 4: 4, 8, 12, ..., 100)
    assert_eq!(total, 25);

    // Then get paginated filtered results
    let options = FindOptions::new()
        .with_sort(vec![("n".to_string(), 1)])
        .with_skip(5)
        .with_limit(10);
    let results = collection.find_with_options(&filter, options).unwrap();

    assert_eq!(results.len(), 10);
    // Verify all returned items are active
    for doc in &results {
        assert_eq!(doc.get("status").unwrap().as_str().unwrap(), "active");
    }
}

// ============================================================================
// EDGE CASES
// ============================================================================

#[test]
fn test_empty_collection_limit() {
    let (db, _temp) = setup_test_db("empty_limit");

    let collection = db.collection("empty").unwrap();
    let options = FindOptions::new().with_limit(10);
    let results = collection.find_with_options(&json!({}), options).unwrap();

    assert_eq!(results.len(), 0);
}

#[test]
fn test_empty_collection_cursor() {
    let (db, _temp) = setup_test_db("empty_cursor");

    let collection = db.collection("empty").unwrap();
    let mut cursor = collection.find_streaming(&json!({})).unwrap();

    assert!(cursor.is_finished());
    assert_eq!(cursor.total(), 0);
    assert_eq!(cursor.remaining(), 0);
    assert!(cursor.next().unwrap().is_none());
}

#[test]
fn test_single_doc_pagination() {
    let (db, _temp) = setup_test_db("single_doc");

    insert_doc(&db, "items", json!({"n": 1}));

    let collection = db.collection("items").unwrap();

    let options = FindOptions::new().with_skip(0).with_limit(10);
    let results = collection.find_with_options(&json!({}), options).unwrap();
    assert_eq!(results.len(), 1);

    let options = FindOptions::new().with_skip(1).with_limit(10);
    let results = collection.find_with_options(&json!({}), options).unwrap();
    assert_eq!(results.len(), 0);
}

#[test]
fn test_cursor_empty_query_result() {
    let (db, _temp) = setup_test_db("cursor_empty_query");

    for i in 1..=10 {
        insert_doc(&db, "items", json!({"n": i, "active": false}));
    }

    let collection = db.collection("items").unwrap();
    let cursor = collection.find_streaming(&json!({"active": true})).unwrap();

    assert!(cursor.is_finished());
    assert_eq!(cursor.total(), 0);
}

#[test]
fn test_large_skip_value() {
    let (db, _temp) = setup_test_db("large_skip");

    for i in 1..=100 {
        insert_doc(&db, "items", json!({"n": i}));
    }

    let collection = db.collection("items").unwrap();
    let options = FindOptions::new().with_skip(1000000);
    let results = collection.find_with_options(&json!({}), options).unwrap();

    assert_eq!(results.len(), 0);
}

// ============================================================================
// PERFORMANCE-ORIENTED TESTS
// ============================================================================

#[test]
fn test_cursor_large_dataset_batched() {
    let (db, _temp) = setup_test_db("cursor_large");

    for i in 1..=1000 {
        insert_doc(&db, "items", json!({"n": i, "data": format!("data_{}", i)}));
    }

    let collection = db.collection("items").unwrap();
    let mut cursor = collection
        .find_streaming(&json!({}))
        .unwrap()
        .with_batch_size(100);

    let mut total = 0;
    while !cursor.is_finished() {
        let batch = cursor.next_batch().unwrap();
        total += batch.len();
    }

    assert_eq!(total, 1000);
}

#[test]
fn test_pagination_consistency() {
    let (db, _temp) = setup_test_db("pagination_consistency");

    for i in 1..=50 {
        insert_doc(&db, "items", json!({"n": i}));
    }

    let collection = db.collection("items").unwrap();

    // Get all items via pagination and verify consistency
    let mut all_ids = Vec::new();
    let page_size = 7;
    let mut page = 0;

    loop {
        let options = FindOptions::new()
            .with_sort(vec![("n".to_string(), 1)])
            .with_skip(page * page_size)
            .with_limit(page_size);

        let results = collection.find_with_options(&json!({}), options).unwrap();

        if results.is_empty() {
            break;
        }

        for doc in &results {
            let n = doc.get("n").unwrap().as_i64().unwrap();
            all_ids.push(n);
        }

        page += 1;
    }

    // Verify no duplicates and all items present
    assert_eq!(all_ids.len(), 50);

    for i in 1..=50 {
        assert!(all_ids.contains(&i), "Missing item {}", i);
    }

    // Verify sorted order
    let mut sorted = all_ids.clone();
    sorted.sort();
    assert_eq!(all_ids, sorted);
}
