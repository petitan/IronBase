//! IronBase Speed Benchmarks
//!
//! Comprehensive performance tests for CRUD, index, and aggregation operations.
//! Run with: cargo test -p ironbase-core --release speed_benchmark -- --nocapture --ignored

#![allow(dead_code)]

use ironbase_core::{storage::MemoryStorage, DatabaseCore};
use serde_json::json;
use std::collections::HashMap;
use std::time::{Duration, Instant};

const DOC_COUNT: usize = 100_000;
const BATCH_SIZE: usize = 1000;

fn format_rate(count: usize, duration: Duration) -> String {
    let ops_per_sec = count as f64 / duration.as_secs_f64();
    if ops_per_sec >= 1_000_000.0 {
        format!("{:.2}M ops/sec", ops_per_sec / 1_000_000.0)
    } else if ops_per_sec >= 1_000.0 {
        format!("{:.2}K ops/sec", ops_per_sec / 1_000.0)
    } else {
        format!("{:.2} ops/sec", ops_per_sec)
    }
}

fn format_duration(d: Duration) -> String {
    if d.as_secs() > 0 {
        format!("{:.2}s", d.as_secs_f64())
    } else if d.as_millis() > 0 {
        format!("{}ms", d.as_millis())
    } else {
        format!("{}µs", d.as_micros())
    }
}

fn generate_doc(i: usize) -> HashMap<String, serde_json::Value> {
    let categories = ["electronics", "clothing", "food", "books", "toys"];
    let cities = [
        "New York", "London", "Tokyo", "Paris", "Berlin", "Sydney", "Toronto",
    ];

    HashMap::from([
        ("name".to_string(), json!(format!("User_{}", i))),
        ("email".to_string(), json!(format!("user{}@example.com", i))),
        ("age".to_string(), json!((i % 60) + 18)),
        ("score".to_string(), json!((i * 7) % 1000)),
        (
            "category".to_string(),
            json!(categories[i % categories.len()]),
        ),
        ("city".to_string(), json!(cities[i % cities.len()])),
        ("active".to_string(), json!(i.is_multiple_of(2))),
        ("balance".to_string(), json!((i as f64) * 1.5)),
        (
            "profile".to_string(),
            json!({
                "level": (i % 10) + 1,
                "points": (i * 13) % 10000,
                "rank": format!("rank_{}", i % 100)
            }),
        ),
        (
            "tags".to_string(),
            json!(vec![
                format!("tag_{}", i % 10),
                format!("tag_{}", (i + 1) % 10),
                format!("tag_{}", (i + 2) % 10)
            ]),
        ),
    ])
}

#[test]
#[ignore] // Run with --ignored flag
fn speed_benchmark_full_suite() {
    println!("\n");
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║           IRONBASE SPEED BENCHMARK SUITE                     ║");
    println!("║                   100K Documents                             ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("benchmark").unwrap();

    // ═══════════════════════════════════════════════════════════════
    // SECTION 1: INSERT BENCHMARKS
    // ═══════════════════════════════════════════════════════════════
    println!("┌──────────────────────────────────────────────────────────────┐");
    println!("│  1. INSERT BENCHMARKS                                        │");
    println!("└──────────────────────────────────────────────────────────────┘");

    // 1a. Bulk insert (insert_many)
    let docs: Vec<_> = (0..DOC_COUNT).map(generate_doc).collect();

    let start = Instant::now();
    for chunk in docs.chunks(BATCH_SIZE) {
        coll.insert_many(chunk.to_vec()).unwrap();
    }
    let insert_time = start.elapsed();

    println!(
        "  insert_many ({} docs, batch {}): {} ({})",
        DOC_COUNT,
        BATCH_SIZE,
        format_duration(insert_time),
        format_rate(DOC_COUNT, insert_time)
    );

    // Verify count
    let count = coll.count_documents(&json!({})).unwrap();
    assert_eq!(count as usize, DOC_COUNT);
    println!("  ✓ Verified: {} documents inserted", count);
    println!();

    // ═══════════════════════════════════════════════════════════════
    // SECTION 2: FIND BENCHMARKS (NO INDEX)
    // ═══════════════════════════════════════════════════════════════
    println!("┌──────────────────────────────────────────────────────────────┐");
    println!("│  2. FIND BENCHMARKS (Collection Scan)                        │");
    println!("└──────────────────────────────────────────────────────────────┘");

    // 2a. Find all (empty filter)
    let start = Instant::now();
    let all = coll.find(&json!({})).unwrap();
    let find_all_time = start.elapsed();
    println!(
        "  find({{}}) - all docs: {} ({} docs)",
        format_duration(find_all_time),
        all.len()
    );

    // 2b. Find by equality (high selectivity - ~1 result)
    let start = Instant::now();
    let result = coll.find(&json!({"name": "User_50000"})).unwrap();
    let find_eq_time = start.elapsed();
    println!(
        "  find({{name: exact}}) - 1 match: {} ({} docs)",
        format_duration(find_eq_time),
        result.len()
    );

    // 2c. Find by range (medium selectivity - ~10% results)
    let start = Instant::now();
    let result = coll.find(&json!({"score": {"$gte": 900}})).unwrap();
    let find_range_time = start.elapsed();
    println!(
        "  find({{score: $gte 900}}) - ~10%: {} ({} docs)",
        format_duration(find_range_time),
        result.len()
    );

    // 2d. Find by category (low selectivity - ~20% results)
    let start = Instant::now();
    let result = coll.find(&json!({"category": "electronics"})).unwrap();
    let find_cat_time = start.elapsed();
    println!(
        "  find({{category: electronics}}) - ~20%: {} ({} docs)",
        format_duration(find_cat_time),
        result.len()
    );

    // 2e. Complex query with $and
    let start = Instant::now();
    let result = coll
        .find(&json!({
            "$and": [
                {"age": {"$gte": 30}},
                {"age": {"$lt": 40}},
                {"active": true}
            ]
        }))
        .unwrap();
    let find_complex_time = start.elapsed();
    println!(
        "  find($and: age 30-40, active) - complex: {} ({} docs)",
        format_duration(find_complex_time),
        result.len()
    );

    // 2f. Nested field query
    let start = Instant::now();
    let result = coll.find(&json!({"profile.level": 5})).unwrap();
    let find_nested_time = start.elapsed();
    println!(
        "  find({{profile.level: 5}}) - nested: {} ({} docs)",
        format_duration(find_nested_time),
        result.len()
    );
    println!();

    // ═══════════════════════════════════════════════════════════════
    // SECTION 3: INDEX CREATION & INDEXED QUERIES
    // ═══════════════════════════════════════════════════════════════
    println!("┌──────────────────────────────────────────────────────────────┐");
    println!("│  3. INDEX BENCHMARKS                                         │");
    println!("└──────────────────────────────────────────────────────────────┘");

    // 3a. Create index on 'score'
    let start = Instant::now();
    coll.create_index("score".to_string(), false).unwrap();
    let idx_score_time = start.elapsed();
    println!("  create_index(score): {}", format_duration(idx_score_time));

    // 3b. Create index on 'category'
    let start = Instant::now();
    coll.create_index("category".to_string(), false).unwrap();
    let idx_cat_time = start.elapsed();
    println!(
        "  create_index(category): {}",
        format_duration(idx_cat_time)
    );

    // 3c. Create index on nested field
    let start = Instant::now();
    coll.create_index("profile.level".to_string(), false)
        .unwrap();
    let idx_nested_time = start.elapsed();
    println!(
        "  create_index(profile.level): {}",
        format_duration(idx_nested_time)
    );

    // 3d. Create compound index
    let start = Instant::now();
    coll.create_compound_index(vec!["category".to_string(), "score".to_string()], false)
        .unwrap();
    let idx_compound_time = start.elapsed();
    println!(
        "  create_compound_index(category, score): {}",
        format_duration(idx_compound_time)
    );

    println!();
    println!("  --- Indexed Queries (vs Collection Scan) ---");

    // 3e. Indexed range query
    let start = Instant::now();
    let result = coll.find(&json!({"score": {"$gte": 900}})).unwrap();
    let find_idx_range_time = start.elapsed();
    let speedup = find_range_time.as_nanos() as f64 / find_idx_range_time.as_nanos() as f64;
    println!(
        "  find({{score: $gte 900}}) indexed: {} ({} docs) - {:.1}x faster",
        format_duration(find_idx_range_time),
        result.len(),
        speedup
    );

    // 3f. Indexed equality query
    let start = Instant::now();
    let result = coll.find(&json!({"category": "electronics"})).unwrap();
    let find_idx_cat_time = start.elapsed();
    let speedup = find_cat_time.as_nanos() as f64 / find_idx_cat_time.as_nanos() as f64;
    println!(
        "  find({{category: electronics}}) indexed: {} ({} docs) - {:.1}x faster",
        format_duration(find_idx_cat_time),
        result.len(),
        speedup
    );

    // 3g. Indexed nested field query
    let start = Instant::now();
    let result = coll.find(&json!({"profile.level": 5})).unwrap();
    let find_idx_nested_time = start.elapsed();
    let speedup = find_nested_time.as_nanos() as f64 / find_idx_nested_time.as_nanos() as f64;
    println!(
        "  find({{profile.level: 5}}) indexed: {} ({} docs) - {:.1}x faster",
        format_duration(find_idx_nested_time),
        result.len(),
        speedup
    );
    println!();

    // ═══════════════════════════════════════════════════════════════
    // SECTION 4: UPDATE BENCHMARKS
    // ═══════════════════════════════════════════════════════════════
    println!("┌──────────────────────────────────────────────────────────────┐");
    println!("│  4. UPDATE BENCHMARKS                                        │");
    println!("└──────────────────────────────────────────────────────────────┘");

    // 4a. Update one document
    let start = Instant::now();
    let (matched, modified) = coll
        .update_one(
            &json!({"name": "User_1000"}),
            &json!({"$set": {"updated": true}}),
        )
        .unwrap();
    let update_one_time = start.elapsed();
    println!(
        "  update_one(name: User_1000): {} (matched: {}, modified: {})",
        format_duration(update_one_time),
        matched,
        modified
    );

    // 4b. Update many (10% of documents)
    let start = Instant::now();
    let (matched, modified) = coll
        .update_many(
            &json!({"score": {"$gte": 900}}),
            &json!({"$set": {"high_score": true}}),
        )
        .unwrap();
    let update_many_time = start.elapsed();
    println!(
        "  update_many(score >= 900): {} (matched: {}, modified: {})",
        format_duration(update_many_time),
        matched,
        modified
    );

    // 4c. Increment operation
    let start = Instant::now();
    let (matched, modified) = coll
        .update_many(
            &json!({"category": "electronics"}),
            &json!({"$inc": {"balance": 100.0}}),
        )
        .unwrap();
    let update_inc_time = start.elapsed();
    println!(
        "  update_many($inc balance): {} (matched: {}, modified: {})",
        format_duration(update_inc_time),
        matched,
        modified
    );
    println!();

    // ═══════════════════════════════════════════════════════════════
    // SECTION 5: AGGREGATION BENCHMARKS
    // ═══════════════════════════════════════════════════════════════
    println!("┌──────────────────────────────────────────────────────────────┐");
    println!("│  5. AGGREGATION BENCHMARKS                                   │");
    println!("└──────────────────────────────────────────────────────────────┘");

    // 5a. Simple group by
    let start = Instant::now();
    let result = coll
        .aggregate(&json!([
            {"$group": {"_id": "$category", "count": {"$sum": 1}}}
        ]))
        .unwrap();
    let agg_group_time = start.elapsed();
    println!(
        "  $group by category (count): {} ({} groups)",
        format_duration(agg_group_time),
        result.len()
    );

    // 5b. Group with multiple accumulators
    let start = Instant::now();
    let result = coll
        .aggregate(&json!([
            {"$group": {
                "_id": "$category",
                "count": {"$sum": 1},
                "avg_score": {"$avg": "$score"},
                "max_score": {"$max": "$score"},
                "min_age": {"$min": "$age"}
            }}
        ]))
        .unwrap();
    let agg_multi_time = start.elapsed();
    println!(
        "  $group (sum, avg, max, min): {} ({} groups)",
        format_duration(agg_multi_time),
        result.len()
    );

    // 5c. Match + Group pipeline
    let start = Instant::now();
    let result = coll
        .aggregate(&json!([
            {"$match": {"active": true}},
            {"$group": {"_id": "$city", "total": {"$sum": "$balance"}}}
        ]))
        .unwrap();
    let agg_match_group_time = start.elapsed();
    println!(
        "  $match + $group: {} ({} groups)",
        format_duration(agg_match_group_time),
        result.len()
    );

    // 5d. Full pipeline with sort and limit
    let start = Instant::now();
    let result = coll
        .aggregate(&json!([
            {"$match": {"score": {"$gte": 500}}},
            {"$group": {"_id": "$category", "avg_score": {"$avg": "$score"}}},
            {"$sort": {"avg_score": -1}},
            {"$limit": 3}
        ]))
        .unwrap();
    let agg_full_time = start.elapsed();
    println!(
        "  $match + $group + $sort + $limit: {} ({} results)",
        format_duration(agg_full_time),
        result.len()
    );

    // 5e. Nested field aggregation
    let start = Instant::now();
    let result = coll
        .aggregate(&json!([
            {"$group": {
                "_id": "$profile.level",
                "count": {"$sum": 1},
                "avg_points": {"$avg": "$profile.points"}
            }},
            {"$sort": {"_id": 1}}
        ]))
        .unwrap();
    let agg_nested_time = start.elapsed();
    println!(
        "  $group by profile.level (nested): {} ({} groups)",
        format_duration(agg_nested_time),
        result.len()
    );
    println!();

    // ═══════════════════════════════════════════════════════════════
    // SECTION 6: DELETE BENCHMARKS
    // ═══════════════════════════════════════════════════════════════
    println!("┌──────────────────────────────────────────────────────────────┐");
    println!("│  6. DELETE BENCHMARKS                                        │");
    println!("└──────────────────────────────────────────────────────────────┘");

    // 6a. Delete one
    let start = Instant::now();
    let deleted = coll.delete_one(&json!({"name": "User_99999"})).unwrap();
    let delete_one_time = start.elapsed();
    println!(
        "  delete_one(name: User_99999): {} (deleted: {})",
        format_duration(delete_one_time),
        deleted
    );

    // 6b. Delete many (~10%)
    let start = Instant::now();
    let deleted = coll.delete_many(&json!({"score": {"$lt": 100}})).unwrap();
    let delete_many_time = start.elapsed();
    println!(
        "  delete_many(score < 100): {} (deleted: {})",
        format_duration(delete_many_time),
        deleted
    );

    let count_after = coll.count_documents(&json!({})).unwrap();
    println!("  Remaining documents: {}", count_after);
    println!();

    // ═══════════════════════════════════════════════════════════════
    // SUMMARY
    // ═══════════════════════════════════════════════════════════════
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║                        SUMMARY                               ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!("  Documents: {}", DOC_COUNT);
    println!("  Insert rate: {}", format_rate(DOC_COUNT, insert_time));
    println!(
        "  Index speedup (range query): {:.1}x",
        find_range_time.as_nanos() as f64 / find_idx_range_time.as_nanos() as f64
    );
    println!(
        "  Index speedup (equality): {:.1}x",
        find_cat_time.as_nanos() as f64 / find_idx_cat_time.as_nanos() as f64
    );
    println!();
}

#[test]
#[ignore]
fn speed_benchmark_insert_rates() {
    println!("\n");
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║           INSERT RATE COMPARISON                             ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

    // Test different batch sizes
    for batch_size in [1, 10, 100, 1000, 10000] {
        let coll = db
            .collection(&format!("bench_batch_{}", batch_size))
            .unwrap();
        let doc_count = 50_000.min(batch_size * 100); // Scale down for small batches

        let docs: Vec<_> = (0..doc_count).map(generate_doc).collect();

        let start = Instant::now();
        if batch_size == 1 {
            for doc in docs {
                coll.insert_one(doc).unwrap();
            }
        } else {
            for chunk in docs.chunks(batch_size) {
                coll.insert_many(chunk.to_vec()).unwrap();
            }
        }
        let elapsed = start.elapsed();

        println!(
            "  Batch size {:>5}: {} ({} docs in {})",
            batch_size,
            format_rate(doc_count, elapsed),
            doc_count,
            format_duration(elapsed)
        );
    }
    println!();
}

#[test]
#[ignore]
fn speed_benchmark_query_selectivity() {
    println!("\n");
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║           QUERY SELECTIVITY IMPACT                           ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("selectivity").unwrap();

    // Insert 100K documents
    let docs: Vec<_> = (0..DOC_COUNT).map(generate_doc).collect();
    for chunk in docs.chunks(BATCH_SIZE) {
        coll.insert_many(chunk.to_vec()).unwrap();
    }

    // Create index
    coll.create_index("score".to_string(), false).unwrap();

    println!(
        "  Testing query selectivity (score >= X) on {} docs:",
        DOC_COUNT
    );
    println!();

    // Test different selectivities
    for threshold in [0, 100, 250, 500, 750, 900, 950, 990, 999] {
        let start = Instant::now();
        let result = coll.find(&json!({"score": {"$gte": threshold}})).unwrap();
        let elapsed = start.elapsed();
        let pct = (result.len() as f64 / DOC_COUNT as f64) * 100.0;

        println!(
            "  score >= {:>3}: {:>6} docs ({:>5.1}%) in {}",
            threshold,
            result.len(),
            pct,
            format_duration(elapsed)
        );
    }
    println!();
}
