//! # Aggregation Pipeline Modul
//!
//! ## Cél
//!
//! MongoDB-kompatibilis aggregation pipeline implementáció dokumentumok
//! transzformálására és analízisére. A pipeline stage-ek láncolhatók,
//! minden stage az előző kimenetén dolgozik.
//!
//! ## Architektúra
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                 PIPELINE EXECUTION MODEL                        │
//! ├─────────────────────────────────────────────────────────────────┤
//! │                                                                 │
//! │   Input Documents                                               │
//! │   ┌─────┐ ┌─────┐ ┌─────┐ ┌─────┐                              │
//! │   │Doc1 │ │Doc2 │ │Doc3 │ │...  │                              │
//! │   └──┬──┘ └──┬──┘ └──┬──┘ └──┬──┘                              │
//! │      │       │       │       │                                  │
//! │      ▼       ▼       ▼       ▼                                  │
//! │   ┌─────────────────────────────────┐                          │
//! │   │         $match (szűrés)         │ ◄── STREAMING            │
//! │   │    (csak illeszkedők tovább)    │     (1 doc → 0/1 doc)    │
//! │   └──────────────┬──────────────────┘                          │
//! │                  │                                              │
//! │                  ▼                                              │
//! │   ┌─────────────────────────────────┐                          │
//! │   │       $project (alakítás)       │ ◄── STREAMING            │
//! │   │   (mező kiválasztás/átnevezés)  │     (1 doc → 1 doc)      │
//! │   └──────────────┬──────────────────┘                          │
//! │                  │                                              │
//! │                  ▼                                              │
//! │   ┌─────────────────────────────────┐                          │
//! │   │     $group (csoportosítás)      │ ◄── ACCUMULATING         │
//! │   │   (aggregátumok számítása)      │     (N doc → G csoport)  │
//! │   └──────────────┬──────────────────┘                          │
//! │                  │                                              │
//! │                  ▼                                              │
//! │   ┌─────────────────────────────────┐                          │
//! │   │        $sort (rendezés)         │ ◄── MATERIALIZING        │
//! │   │  (összes doc kell egyszerre)    │     (requires all)       │
//! │   └──────────────┬──────────────────┘                          │
//! │                  │                                              │
//! │                  ▼                                              │
//! │   ┌─────────────────────────────────┐                          │
//! │   │    $limit / $skip (lapozás)     │                          │
//! │   └──────────────┬──────────────────┘                          │
//! │                  │                                              │
//! │                  ▼                                              │
//! │            Output Documents                                     │
//! │                                                                 │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Támogatott Stage-ek
//!
//! | Stage      | Leírás                                    | Memória típus   |
//! |------------|-------------------------------------------|-----------------|
//! | `$match`   | Dokumentumok szűrése query alapján        | Streaming       |
//! | `$project` | Mezők kiválasztása/átnevezése/számítása   | Streaming       |
//! | `$group`   | Csoportosítás és aggregátumok             | Accumulating    |
//! | `$sort`    | Rendezés mező(k) szerint                  | Materializing   |
//! | `$limit`   | Első N dokumentum megtartása              | Pass-through    |
//! | `$skip`    | Első N dokumentum kihagyása               | Pass-through    |
//! | `$unwind`  | Tömb mező "kiterítése" (1→N doc)          | Expanding       |
//!
//! ## Memória Optimalizáció
//!
//! ```text
//! ┌────────────────────────────────────────────────────────────────┐
//! │            EXECUTE vs EXECUTE_STREAMING                        │
//! ├────────────────────────────────────────────────────────────────┤
//! │                                                                │
//! │  execute() - LEGACY                                            │
//! │  ┌─────────────────────────────────────────────────────┐      │
//! │  │  Minden stage TELJES Vec<Value>-t kap és ad vissza  │      │
//! │  │  Memória: O(N × doc_size) minden stage után         │      │
//! │  │  Példa: 650K email × 800B = ~500MB mindvégig        │      │
//! │  └─────────────────────────────────────────────────────┘      │
//! │                                                                │
//! │  execute_streaming() - OPTIMALIZÁLT                            │
//! │  ┌─────────────────────────────────────────────────────┐      │
//! │  │  Phase 1: $match/$project → Iterator (0 alloc)      │      │
//! │  │  Phase 2: $group → AccumulatorState (O(G) memory)   │      │
//! │  │  Phase 3: $sort/$limit → Materialize eredményt      │      │
//! │  │  Memória: O(G × state_size) ahol G = csoportok száma│      │
//! │  │  Példa: 650K email → 10K csoport = ~640KB           │      │
//! │  └─────────────────────────────────────────────────────┘      │
//! │                                                                │
//! └────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Accumulator Típusok ($group)
//!
//! | Accumulator | Működés                          | State méret     |
//! |-------------|----------------------------------|-----------------|
//! | `$sum`      | Összeg számítás                  | 16 byte (i64+f64)|
//! | `$avg`      | Átlag (sum/count)                | 16 byte         |
//! | `$min`      | Legkisebb érték                  | ~32 byte        |
//! | `$max`      | Legnagyobb érték                 | ~32 byte        |
//! | `$first`    | Első dokumentum értéke           | ~32 byte        |
//! | `$last`     | Utolsó dokumentum értéke         | ~32 byte        |
//! | `$push`     | Összes érték tömbben             | O(N) - NEM OPT  |
//! | `$addToSet` | Egyedi értékek tömbben           | O(unique)       |
//!
//! ## $project Kifejezések
//!
//! ```text
//! Include/Exclude:  {"name": 1, "password": 0}
//! Rename:           {"fullName": "$name"}
//! $size:            {"tagCount": {"$size": "$tags"}}
//! $reduce:          {"total": {"$reduce": {...}}}
//! Aritmetika:       {"$add", "$subtract", "$multiply", "$divide", "$mod"}
//!                   {"$abs", "$ceil", "$floor", "$round"}
//! ```
//!
//! ## Invariánsok
//!
//! 1. **Pipeline Sorrend**: Stage-ek sorrendben hajtódnak végre
//! 2. **Stage Izoláció**: Minden stage az előző kimenetén dolgozik
//! 3. **_id Mező**: `$group` után mindig van `_id` mező
//! 4. **Null Kezelés**: Hiányzó mező → null (nem hiba)
//! 5. **Típus Biztonság**: Numerikus műveletek nem-számra → skip
//!
//! ## Index Optimalizáció
//!
//! A `Pipeline::extract_leading_match()` metódus lehetővé teszi,
//! hogy a vezető `$match` stage-t indexelt `find()`-al hajtsuk végre
//! teljes collection scan helyett:
//!
//! ```rust,ignore
//! let mut pipeline = Pipeline::from_json(&json_pipeline)?;
//! if let Some(match_query) = pipeline.extract_leading_match() {
//!     // Use indexed find() instead of full scan
//!     let docs = collection.find(&match_query)?;
//!     pipeline.execute_streaming(docs.into_iter().map(Ok))
//! } else {
//!     // No leading $match - full scan
//!     let docs = collection.find(&json!({}))?;
//!     pipeline.execute_streaming(docs.into_iter().map(Ok))
//! }
//! ```
//!
//! ## Kapcsolódó Modulok
//!
//! - [`crate::query`] - Query operátorok ($match stage-hez)
//! - [`crate::collection_core`] - `aggregate()` API
//! - [`crate::find_options`] - Projection logika ($project-hez hasonló)
//!
//! ## Példa Használat
//!
//! ```rust,ignore
//! use serde_json::json;
//!
//! // Email statisztikák: hány email érkezett feladónként
//! let pipeline = Pipeline::from_json(&json!([
//!     {"$match": {"folder": "inbox"}},
//!     {"$project": {"from": "$from.email", "_id": 0}},
//!     {"$group": {"_id": "$from", "count": {"$sum": 1}}},
//!     {"$sort": {"count": -1}},
//!     {"$limit": 10}
//! ]))?;
//!
//! let results = pipeline.execute_streaming(docs.into_iter().map(Ok))?;
//! // Eredmény: [{"_id": "boss@example.com", "count": 42}, ...]
//! ```
//!
//! ## Submodulok
//!
//! - [`pipeline`] - Pipeline és Stage implementáció
//! - [`types`] - Típus definíciók (Stage, Accumulator, stb.)
//! - [`stages`] - Egyedi stage implementációk
//! - [`helpers`] - Segédfüggvények (dot notation, mező kinyerés)

pub mod context;
mod helpers;
pub mod memory_info;
pub mod optimizer;
mod pipeline;
pub mod profiles;
mod stages;
pub mod stream;
mod types;

// Re-export public types
// Memory detection functions are re-exported for users who want to check system memory manually
#[allow(unused_imports)]
pub use memory_info::{get_available_memory_bytes, get_total_memory_bytes};
pub use types::AggregationLimits;
pub use types::Pipeline;
pub use types::Stage;

// Re-export optimizer types for fast path execution
pub use optimizer::{analyze_pipeline, FastPath, PipelineOptimization};

// Re-export context and profiles (Phase 2: will be used in pipeline.rs)
#[allow(unused_imports)]
pub use context::AggregationLimitContext;
#[allow(unused_imports)]
pub use profiles::{limits_from_system, limits_from_tier, MemoryTier};

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

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

    // ========== $count tests ==========

    #[test]
    fn test_count_stage() {
        let docs = vec![
            json!({"name": "Alice"}),
            json!({"name": "Bob"}),
            json!({"name": "Charlie"}),
        ];

        let pipeline = Pipeline::from_json(&json!([{"$count": "total"}])).unwrap();
        let results = pipeline.execute(docs).unwrap();
        assert_eq!(results, vec![json!({"total": 3})]);
    }

    #[test]
    fn test_count_stage_with_match() {
        let docs = vec![
            json!({"name": "Alice", "age": 30}),
            json!({"name": "Bob", "age": 25}),
            json!({"name": "Charlie", "age": 35}),
        ];

        let pipeline = Pipeline::from_json(&json!([
            {"$match": {"age": {"$gte": 30}}},
            {"$count": "n"}
        ]))
        .unwrap();

        let results = pipeline.execute(docs).unwrap();
        assert_eq!(results, vec![json!({"n": 2})]);
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

    // ========== Aggregation Limits tests ==========

    #[test]
    fn test_aggregation_limits_default() {
        let limits = AggregationLimits::default();
        // FIX (2026-01-25): Harmonized with scale_to_memory(4GB)
        assert_eq!(limits.max_docs_without_match, 10_000);
        assert_eq!(limits.max_group_count, 100_000); // FIX: was 50K
        assert_eq!(limits.max_memory_mb, 256); // FIX: was 512
                                               // New global limits
        assert_eq!(limits.max_total_push_elements, 1_000_000);
        assert_eq!(limits.max_total_addtoset_elements, 500_000);
    }

    #[test]
    fn test_aggregation_limits_low_memory() {
        let limits = AggregationLimits::low_memory();
        // FIX (2026-01-25): Increased max_group_count - streaming $group uses minimal memory
        assert_eq!(limits.max_docs_without_match, 1_000);
        assert_eq!(limits.max_docs_with_match, 5_000);
        assert_eq!(limits.max_group_count, 10_000); // FIX: was 500, now 10K (only 640KB)
        assert_eq!(limits.max_memory_mb, 64); // FIX: was 128
                                              // Global limits
        assert_eq!(limits.max_total_push_elements, 50_000);
        assert_eq!(limits.max_total_addtoset_elements, 25_000);
    }

    // ========== FIX (2026-01-25) New tests for limit system consistency ==========

    #[test]
    fn test_default_matches_4gb_scale() {
        // Verify that Default produces the same values as scale_to_memory with 4GB
        // This ensures consistency between constructors
        let default_limits = AggregationLimits::default();
        let scaled_limits = AggregationLimits::with_memory_budget(4096); // 4GB

        assert_eq!(
            default_limits.max_docs_without_match, scaled_limits.max_docs_without_match,
            "default() should match with_memory_budget(4GB) for max_docs_without_match"
        );
        assert_eq!(
            default_limits.max_group_count, scaled_limits.max_group_count,
            "default() should match with_memory_budget(4GB) for max_group_count"
        );
        assert_eq!(
            default_limits.max_push_elements, scaled_limits.max_push_elements,
            "default() should match with_memory_budget(4GB) for max_push_elements"
        );
        assert_eq!(
            default_limits.max_total_push_elements, scaled_limits.max_total_push_elements,
            "default() should match with_memory_budget(4GB) for max_total_push_elements"
        );
    }

    #[test]
    fn test_scaling_formula_consistency() {
        // Verify that scaling produces consistent results across different memory levels
        // All limits should scale linearly with memory
        let gb1 = AggregationLimits::with_memory_budget(1024); // 1GB
        let gb4 = AggregationLimits::with_memory_budget(4096); // 4GB
        let gb16 = AggregationLimits::with_memory_budget(16384); // 16GB

        // 1GB → 4GB is 4x, so limits should be ~4x (accounting for minimums)
        assert!(
            gb4.max_docs_without_match >= gb1.max_docs_without_match,
            "4GB limits should be >= 1GB limits"
        );
        assert!(
            gb16.max_docs_without_match >= gb4.max_docs_without_match,
            "16GB limits should be >= 4GB limits"
        );

        // Same pattern for groups
        assert!(gb4.max_group_count >= gb1.max_group_count);
        assert!(gb16.max_group_count >= gb4.max_group_count);

        // Global limits should also scale
        assert!(gb4.max_total_push_elements >= gb1.max_total_push_elements);
        assert!(gb16.max_total_push_elements >= gb4.max_total_push_elements);
    }

    #[test]
    fn test_low_memory_allows_reasonable_streaming_group() {
        // Even low_memory() should allow reasonable streaming $group usage
        // because streaming only uses ~64 bytes per group
        let limits = AggregationLimits::low_memory();

        // 10K groups × 64 bytes = 640 KB - should be allowed
        assert!(
            limits.max_group_count >= 10_000,
            "low_memory() should allow at least 10K groups (only 640KB memory)"
        );

        // Memory estimate: 10K groups × 64B = 640KB << 64MB budget
        let estimated_memory_kb = (limits.max_group_count * 64) / 1024;
        let budget_kb = limits.max_memory_mb * 1024;
        assert!(
            estimated_memory_kb < budget_kb,
            "max_group_count ({}) memory estimate ({}KB) should be < budget ({}KB)",
            limits.max_group_count,
            estimated_memory_kb,
            budget_kb
        );
    }

    #[test]
    fn test_global_push_limit_context_tracking() {
        // Verify that the context tracks global $push elements
        use crate::aggregation::context::AggregationLimitContext;

        let limits = AggregationLimits {
            max_push_elements: 100,       // per-group
            max_total_push_elements: 250, // global (2.5 groups worth)
            ..Default::default()
        };

        let ctx = AggregationLimitContext::new(limits);

        // Register 3 groups
        ctx.register_new_group(1).unwrap();
        ctx.register_new_group(2).unwrap();
        ctx.register_new_group(3).unwrap();

        // Each group pushes elements - should hit global limit
        for _ in 0..80 {
            ctx.increment_push(1).unwrap();
        }
        for _ in 0..80 {
            ctx.increment_push(2).unwrap();
        }
        // Now at 160 total, try to push 100 more - should fail at global limit
        for _ in 0..90 {
            // This should eventually fail when we hit 250
            let _ = ctx.increment_push(3);
        }

        // Should have hit the global limit
        let total = ctx.total_push_elements();
        assert!(
            total >= 250,
            "Should have tracked at least 250 elements, got {}",
            total
        );
    }

    #[test]
    fn test_aggregation_doc_limit_exceeded() {
        // Generate docs that exceed the limit
        let docs: Vec<Value> = (0..5000).map(|i| json!({"x": i})).collect();

        // Pipeline WITHOUT $match - should hit doc limit
        let pipeline = Pipeline::from_json(&json!([
            {"$group": {"_id": null, "count": {"$sum": 1}}}
        ]))
        .unwrap();

        let limits = AggregationLimits {
            max_docs_without_match: 1000, // Low limit
            ..Default::default()
        };

        let result = pipeline.execute_streaming_with_limits(docs.into_iter().map(Ok), limits);

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("exceeded document limit"),
            "Expected 'exceeded document limit', got: {}",
            err_msg
        );
    }

    #[test]
    fn test_aggregation_doc_limit_with_match_ok() {
        // Same docs, but WITH $match - should NOT hit doc limit
        // because $match filters docs before the limit check
        let docs: Vec<Value> = (0..5000).map(|i| json!({"x": i})).collect();

        // Pipeline WITH $match
        let mut pipeline = Pipeline::from_json(&json!([
            {"$match": {"x": {"$lt": 100}}},  // Only keeps first 100
            {"$group": {"_id": null, "count": {"$sum": 1}}}
        ]))
        .unwrap();

        // Mark that we had a leading match (simulating what aggregate() does)
        pipeline.set_has_leading_match(true);

        let limits = AggregationLimits {
            max_docs_without_match: 1000, // This limit won't apply due to $match
            max_docs_with_match: 10_000,  // But this one will!
            ..Default::default()
        };

        let result = pipeline.execute_streaming_with_limits(docs.into_iter().map(Ok), limits);

        assert!(result.is_ok());
        let results = result.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["count"], 100); // Only matched 100 docs
    }

    #[test]
    fn test_aggregation_group_limit_exceeded() {
        // Generate docs with unique group keys
        let docs: Vec<Value> = (0..5000)
            .map(|i| json!({"user_id": format!("user_{}", i)}))
            .collect();

        // $group by user_id - creates 5000 unique groups
        let pipeline = Pipeline::from_json(&json!([
            {"$group": {"_id": "$user_id", "count": {"$sum": 1}}}
        ]))
        .unwrap();

        let limits = AggregationLimits {
            max_docs_without_match: 100_000, // High doc limit
            max_group_count: 1000,           // Low group limit
            ..Default::default()
        };

        let result = pipeline.execute_streaming_with_limits(docs.into_iter().map(Ok), limits);

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("exceeded group limit") || err_msg.contains("unique groups"),
            "Expected group limit error, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_aggregation_limits_unlimited() {
        let limits = AggregationLimits::unlimited();
        assert_eq!(limits.max_docs_without_match, usize::MAX);
        assert_eq!(limits.max_group_count, usize::MAX);
    }

    // ========== OOM FIX tests (2026-01) ==========

    #[test]
    fn test_limit_only_pipeline_early_exit() {
        // OOM FIX: [{"$limit": 5}] should only materialize 5 docs, not all
        let docs: Vec<Value> = (0..1000).map(|i| json!({"i": i})).collect();

        let pipeline = Pipeline::from_json(&json!([{"$limit": 5}])).unwrap();

        // Use very low limit to ensure early exit is working
        let limits = AggregationLimits {
            max_docs_without_match: 100, // Would fail if all docs were loaded
            ..Default::default()
        };

        let result = pipeline
            .execute_streaming_with_limits(docs.into_iter().map(Ok), limits)
            .unwrap();

        assert_eq!(result.len(), 5);
        // Verify we got the first 5 docs
        assert_eq!(result[0]["i"], 0);
        assert_eq!(result[4]["i"], 4);
    }

    #[test]
    fn test_skip_limit_pipeline_early_exit() {
        // OOM FIX: [{"$skip": 10}, {"$limit": 5}] should only materialize 5 docs
        let docs: Vec<Value> = (0..1000).map(|i| json!({"i": i})).collect();

        let pipeline = Pipeline::from_json(&json!([{"$skip": 10}, {"$limit": 5}])).unwrap();

        let limits = AggregationLimits {
            max_docs_without_match: 100,
            ..Default::default()
        };

        let result = pipeline
            .execute_streaming_with_limits(docs.into_iter().map(Ok), limits)
            .unwrap();

        assert_eq!(result.len(), 5);
        // Verify we skipped 10 and got next 5
        assert_eq!(result[0]["i"], 10);
        assert_eq!(result[4]["i"], 14);
    }

    #[test]
    fn test_empty_pipeline_rejected() {
        // Empty pipeline is rejected at parse time
        let result = Pipeline::from_json(&json!([]));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("empty"), "Expected empty error, got: {}", err);
    }

    #[test]
    fn test_project_only_pipeline_respects_limit() {
        // OOM FIX: Simple pipeline without $group should respect doc limit
        let docs: Vec<Value> = (0..100).map(|i| json!({"i": i, "extra": "data"})).collect();

        let pipeline = Pipeline::from_json(&json!([{"$project": {"i": 1}}])).unwrap();

        let limits = AggregationLimits {
            max_docs_without_match: 50, // Lower than doc count
            ..Default::default()
        };

        let result = pipeline.execute_streaming_with_limits(docs.into_iter().map(Ok), limits);

        // Should fail because 100 docs > 50 limit
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("exceeded") && err.contains("limit"),
            "Expected limit error, got: {}",
            err
        );
    }

    #[test]
    fn test_limit_with_sort_loads_all() {
        // When $sort comes before $limit, we MUST load all docs for correct sort
        // (Top-K optimization handles this efficiently, but we can't use early exit)
        let docs: Vec<Value> = (0..100).map(|i| json!({"i": i})).collect();

        let pipeline = Pipeline::from_json(&json!([{"$sort": {"i": -1}}, {"$limit": 5}])).unwrap();

        let limits = AggregationLimits {
            max_docs_without_match: 1000, // High enough to allow all docs
            ..Default::default()
        };

        let result = pipeline
            .execute_streaming_with_limits(docs.into_iter().map(Ok), limits)
            .unwrap();

        assert_eq!(result.len(), 5);
        // Verify we got top 5 by descending order
        assert_eq!(result[0]["i"], 99);
        assert_eq!(result[4]["i"], 95);
    }

    // ========== Top-K Optimization tests ==========

    #[test]
    fn test_topk_optimization_sort_limit() {
        // Create docs with unique values
        let docs: Vec<Value> = (0..1000)
            .map(|i| json!({"id": i, "score": 1000 - i}))
            .collect();

        // Pipeline with $sort → $limit pattern
        let pipeline = Pipeline::from_json(&json!([
            {"$sort": {"score": -1}},
            {"$limit": 5}
        ]))
        .unwrap();

        let results = pipeline.execute(docs).unwrap();

        // Should return top 5 by score descending
        assert_eq!(results.len(), 5);
        assert_eq!(results[0]["score"], 1000);
        assert_eq!(results[1]["score"], 999);
        assert_eq!(results[2]["score"], 998);
        assert_eq!(results[3]["score"], 997);
        assert_eq!(results[4]["score"], 996);
    }

    #[test]
    fn test_topk_optimization_group_sort_limit() {
        // Simulates the problematic query that caused OOM
        let docs: Vec<Value> = (0..500)
            .map(|i| json!({"city": format!("city_{}", i % 100), "amount": i}))
            .collect();

        // Pipeline: $group → $sort → $limit (Top-K pattern)
        let pipeline = Pipeline::from_json(&json!([
            {"$group": {"_id": "$city", "total": {"$sum": "$amount"}}},
            {"$sort": {"total": -1}},
            {"$limit": 3}
        ]))
        .unwrap();

        let results = pipeline
            .execute_streaming(docs.into_iter().map(Ok))
            .unwrap();

        // Should return top 3 cities by total
        assert_eq!(results.len(), 3);

        // Verify descending order
        let total0 = results[0]["total"].as_i64().unwrap();
        let total1 = results[1]["total"].as_i64().unwrap();
        let total2 = results[2]["total"].as_i64().unwrap();
        assert!(total0 >= total1);
        assert!(total1 >= total2);
    }

    #[test]
    fn test_topk_ascending_sort() {
        let docs: Vec<Value> = (0..100).map(|i| json!({"value": i})).collect();

        let pipeline = Pipeline::from_json(&json!([
            {"$sort": {"value": 1}},
            {"$limit": 3}
        ]))
        .unwrap();

        let results = pipeline.execute(docs).unwrap();

        assert_eq!(results.len(), 3);
        assert_eq!(results[0]["value"], 0);
        assert_eq!(results[1]["value"], 1);
        assert_eq!(results[2]["value"], 2);
    }

    #[test]
    #[ignore] // Run with: cargo test -p ironbase-core --release speed_benchmark_topk -- --nocapture --ignored
    fn speed_benchmark_topk_vs_full_sort() {
        use std::time::Instant;

        println!("\n=== Top-K vs Full Sort Benchmark (streaming) ===\n");

        for group_count in [10_000usize, 50_000, 100_000] {
            println!("--- {} groups, limit 5 ---", group_count);

            // Simulate $group output
            let docs: Vec<Value> = (0..group_count)
                .map(|i| json!({"_id": format!("email_{}", i), "count": group_count - i}))
                .collect();

            // Top-K: $sort + $limit (uses optimization via streaming)
            let pipeline_topk = Pipeline::from_json(&json!([
                {"$sort": {"count": -1}},
                {"$limit": 5}
            ]))
            .unwrap();

            let docs_clone = docs.clone();
            let start = Instant::now();
            // Use streaming to trigger Top-K optimization
            let result = pipeline_topk
                .execute_streaming(docs_clone.into_iter().map(Ok))
                .unwrap();
            let topk_time = start.elapsed();

            assert_eq!(result.len(), 5);
            assert_eq!(result[0]["count"], group_count as i64);

            // Full sort: $sort only (no limit = no optimization)
            let pipeline_full = Pipeline::from_json(&json!([
                {"$sort": {"count": -1}}
            ]))
            .unwrap();

            let docs_clone = docs.clone();
            let start = Instant::now();
            let _result = pipeline_full
                .execute_streaming(docs_clone.into_iter().map(Ok))
                .unwrap();
            let full_time = start.elapsed();

            let speedup = full_time.as_micros() as f64 / topk_time.as_micros().max(1) as f64;

            println!(
                "Top-K (limit 5): {:>10.3}ms",
                topk_time.as_secs_f64() * 1000.0
            );
            println!(
                "Full sort:       {:>10.3}ms",
                full_time.as_secs_f64() * 1000.0
            );
            println!("Speedup:         {:>10.2}x\n", speedup);
        }
    }

    // ========== OOM Protection tests ==========

    #[test]
    fn test_push_limit_exceeded() {
        // Test that $push respects the max_push_elements limit
        let docs: Vec<Value> = (0..200).map(|i| json!({"x": i})).collect();

        let pipeline = Pipeline::from_json(&json!([
            {"$group": {"_id": null, "all": {"$push": "$x"}}}
        ]))
        .unwrap();

        let limits = AggregationLimits {
            max_push_elements: 100, // Low limit - should fail
            ..Default::default()
        };

        let result = pipeline.execute_streaming_with_limits(docs.into_iter().map(Ok), limits);

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("$push exceeded") && err_msg.contains("limit"),
            "Expected $push limit error, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_addtoset_limit_exceeded() {
        // Test that $addToSet respects the max_addtoset_elements limit
        let docs: Vec<Value> = (0..200).map(|i| json!({"x": i})).collect();

        let pipeline = Pipeline::from_json(&json!([
            {"$group": {"_id": null, "unique": {"$addToSet": "$x"}}}
        ]))
        .unwrap();

        let limits = AggregationLimits {
            max_addtoset_elements: 100, // Low limit - should fail
            ..Default::default()
        };

        let result = pipeline.execute_streaming_with_limits(docs.into_iter().map(Ok), limits);

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("$addToSet exceeded") && err_msg.contains("limit"),
            "Expected $addToSet limit error, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_match_has_limit_now() {
        // Test that $match no longer bypasses limits (was usize::MAX, now max_docs_with_match)
        let docs: Vec<Value> = (0..200).map(|i| json!({"x": i})).collect();

        let mut pipeline = Pipeline::from_json(&json!([
            {"$group": {"_id": null, "count": {"$sum": 1}}}
        ]))
        .unwrap();

        // Simulate that aggregate() extracted a $match stage
        pipeline.set_has_leading_match(true);

        let limits = AggregationLimits {
            max_docs_without_match: 50, // This one is NOT used because we have $match
            max_docs_with_match: 100,   // This one IS used - should fail at 100
            ..Default::default()
        };

        let result = pipeline.execute_streaming_with_limits(docs.into_iter().map(Ok), limits);

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("exceeded document limit"),
            "Expected document limit error even with $match, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_unwind_output_limit() {
        // Create docs with large arrays
        let docs: Vec<Value> = (0..10)
            .map(|i| {
                json!({
                    "id": i,
                    "items": (0..200).collect::<Vec<_>>()
                })
            })
            .collect();

        // 10 docs × 200 items = 2000 output docs
        // Default limit is 1,000,000 so we need to use the pipeline directly
        // Let's test via the Stage enum which is public

        let pipeline = Pipeline::from_json(&json!([
            {"$unwind": "$items"}
        ]))
        .unwrap();

        // The default unwind limit is 1M, so this should succeed
        let result = pipeline.execute(docs.clone());
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 2000); // 10 × 200

        // For the actual limit test, we use the internal constant (DEFAULT_MAX_UNWIND_OUTPUT = 1M)
        // which is tested implicitly. The try_reserve and limit checks are in place.
    }

    #[test]
    fn test_push_within_limit_succeeds() {
        // Test that $push works when within limits
        let docs: Vec<Value> = (0..50).map(|i| json!({"x": i})).collect();

        let pipeline = Pipeline::from_json(&json!([
            {"$group": {"_id": null, "all": {"$push": "$x"}}}
        ]))
        .unwrap();

        let limits = AggregationLimits {
            max_push_elements: 100, // Higher than doc count - should succeed
            ..Default::default()
        };

        let result = pipeline.execute_streaming_with_limits(docs.into_iter().map(Ok), limits);

        assert!(result.is_ok());
        let results = result.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["all"].as_array().unwrap().len(), 50);
    }

    // ========== Dynamic Memory Limits Tests ==========

    #[test]
    fn test_from_system_memory_returns_valid_limits() {
        let limits = AggregationLimits::from_system_memory();

        // Limits should be positive and reasonable
        // Note: On low-memory CI runners, scale_factor can be 0.1, giving:
        //   max_docs_without_match = (10_000 * 0.1).max(1_000) = 1_000
        //   max_group_count = (50_000 * 0.1).max(500) = 5_000
        assert!(
            limits.max_docs_without_match >= 1_000,
            "max_docs_without_match ({}) should be >= 1000",
            limits.max_docs_without_match
        );
        assert!(limits.max_memory_mb >= 64);
        assert!(limits.max_memory_mb <= 4096);
        assert!(
            limits.max_group_count >= 500,
            "max_group_count ({}) should be >= 500",
            limits.max_group_count
        );
    }

    #[test]
    fn test_with_memory_budget_scales_correctly() {
        let small = AggregationLimits::with_memory_budget(128);
        let large = AggregationLimits::with_memory_budget(1024);

        // Larger budget should give larger limits
        assert!(
            large.max_docs_without_match > small.max_docs_without_match,
            "large.max_docs_without_match ({}) should be > small ({}) ",
            large.max_docs_without_match,
            small.max_docs_without_match
        );
        assert!(
            large.max_group_count > small.max_group_count,
            "large.max_group_count ({}) should be > small ({})",
            large.max_group_count,
            small.max_group_count
        );
        assert!(large.max_memory_mb > small.max_memory_mb);
    }

    #[test]
    fn test_with_memory_budget_enforces_minimum() {
        // Even with tiny budget, should have minimum limits
        let limits = AggregationLimits::with_memory_budget(1); // 1 MB - very small

        assert!(limits.max_docs_without_match >= 1_000);
        assert!(limits.max_memory_mb >= 64);
        assert!(limits.max_group_count >= 500);
    }

    #[test]
    fn test_with_memory_budget_scales_large_budgets() {
        // Large budgets should continue scaling (no 4GB cap)
        let gb4 = AggregationLimits::with_memory_budget(4096); // 4 GB
        let gb16 = AggregationLimits::with_memory_budget(16384); // 16 GB
        let gb64 = AggregationLimits::with_memory_budget(65536); // 64 GB

        // 16GB should have ~4x the limits of 4GB
        assert!(
            gb16.max_docs_without_match > gb4.max_docs_without_match * 3,
            "16GB ({}) should be ~4x of 4GB ({})",
            gb16.max_docs_without_match,
            gb4.max_docs_without_match
        );
        assert!(
            gb16.max_group_count > gb4.max_group_count * 3,
            "16GB groups ({}) should be ~4x of 4GB ({})",
            gb16.max_group_count,
            gb4.max_group_count
        );

        // 64GB should have ~4x the limits of 16GB
        assert!(
            gb64.max_docs_without_match > gb16.max_docs_without_match * 3,
            "64GB ({}) should be ~4x of 16GB ({})",
            gb64.max_docs_without_match,
            gb16.max_docs_without_match
        );

        // Memory budget is preserved (no capping)
        assert_eq!(gb4.max_memory_mb, 4096);
        assert_eq!(gb16.max_memory_mb, 16384);
        assert_eq!(gb64.max_memory_mb, 65536);
    }

    #[test]
    fn test_with_memory_budget_exact_values() {
        // Test specific expected values at known budget points
        // FIX (2026-01-25): Now uses 4GB base (was 1GB), so values are different
        //
        // 1GB = 1024MB → scale_factor = 1024/4096 = 0.25 → 0.25x base limits
        let gb1 = AggregationLimits::with_memory_budget(1024);
        assert_eq!(gb1.max_docs_without_match, 2_500);
        assert_eq!(gb1.max_group_count, 25_000);
        assert_eq!(gb1.max_memory_mb, 1024);

        // 4GB = 4096MB → scale_factor = 1.0 → base limits (this is the reference point)
        let gb4 = AggregationLimits::with_memory_budget(4096);
        assert_eq!(gb4.max_docs_without_match, 10_000);
        assert_eq!(gb4.max_group_count, 100_000);
        assert_eq!(gb4.max_memory_mb, 4096);

        // 16GB = 16384MB → scale_factor = 4.0 → 4x base limits
        let gb16 = AggregationLimits::with_memory_budget(16384);
        assert_eq!(gb16.max_docs_without_match, 40_000);
        assert_eq!(gb16.max_group_count, 400_000);
        assert_eq!(gb16.max_memory_mb, 16384);
    }

    #[test]
    #[cfg(unix)]
    fn test_memory_detection_works_on_unix() {
        use super::memory_info::{get_available_memory_bytes, get_total_memory_bytes};

        // Test available memory
        let avail = get_available_memory_bytes();
        assert!(
            avail.is_some(),
            "Available memory detection should work on Unix"
        );
        let avail_bytes = avail.unwrap();
        assert!(avail_bytes > 0, "Available memory should be > 0");

        // Test total memory
        let total = get_total_memory_bytes();
        assert!(
            total.is_some(),
            "Total memory detection should work on Unix"
        );
        let total_bytes = total.unwrap();
        assert!(total_bytes > 0, "Total memory should be > 0");

        // Total should be >= available
        assert!(
            total_bytes >= avail_bytes,
            "Total memory ({} MB) should be >= available ({} MB)",
            total_bytes / 1024 / 1024,
            avail_bytes / 1024 / 1024
        );

        // Sanity check: at least 100MB available
        let mb = avail_bytes / 1024 / 1024;
        assert!(
            mb >= 100,
            "Should have at least 100MB available (got {}MB)",
            mb
        );
    }

    #[test]
    fn test_scale_to_memory_profiles() {
        // Test the scale_to_memory function indirectly through with_memory_budget
        // FIX (2026-01-25): Now uses 4GB base, so values changed

        // Low memory profile: 128/4096 = 0.03125 → uses minimum
        let low = AggregationLimits::with_memory_budget(128);
        assert_eq!(low.max_docs_without_match, 1_000); // minimum

        // Standard profile: 512/4096 = 0.125
        let standard = AggregationLimits::with_memory_budget(512);
        assert_eq!(standard.max_docs_without_match, 1_250);

        // High memory profile: 2048/4096 = 0.5
        let high = AggregationLimits::with_memory_budget(2048);
        assert_eq!(high.max_docs_without_match, 5_000);

        assert!(low.max_docs_without_match <= standard.max_docs_without_match);
        assert!(standard.max_docs_without_match < high.max_docs_without_match);
    }

    // ============================================================
    // BUG FIX REGRESSION TESTS (2026-01)
    // These tests verify that the specific OOM bugs are fixed
    // ============================================================

    #[test]
    fn test_bugfix_large_limit_respects_doc_limit() {
        // BUG: $limit: 1_000_000 bypassed doc_limit protection
        // FIX: effective_limit = min(user_limit, doc_limit)
        // NEW BEHAVIOR: Early exit at doc_limit, returns doc_limit docs (safe from OOM)
        let docs: Vec<Value> = (0..500).map(|i| json!({"i": i})).collect();

        let pipeline = Pipeline::from_json(&json!([{"$limit": 1_000_000}])).unwrap();

        let limits = AggregationLimits {
            max_docs_without_match: 100, // Only allow 100 docs
            ..Default::default()
        };

        let result = pipeline
            .execute_streaming_with_limits(docs.into_iter().map(Ok), limits)
            .unwrap();

        // Should return exactly doc_limit docs (early exit protects from OOM)
        assert_eq!(result.len(), 100);
    }

    #[test]
    fn test_bugfix_limit_within_doc_limit_succeeds() {
        // Verify that small $limit still works when within doc_limit
        let docs: Vec<Value> = (0..500).map(|i| json!({"i": i})).collect();

        let pipeline = Pipeline::from_json(&json!([{"$limit": 10}])).unwrap();

        let limits = AggregationLimits {
            max_docs_without_match: 100, // Allow 100 docs
            ..Default::default()
        };

        let result = pipeline
            .execute_streaming_with_limits(docs.into_iter().map(Ok), limits)
            .unwrap();

        // Should succeed with exactly 10 docs (early exit before hitting 100)
        assert_eq!(result.len(), 10);
    }

    #[test]
    fn test_bugfix_unwind_uses_custom_limit() {
        use crate::aggregation::types::UnwindStage;

        // BUG: $unwind used hardcoded 1M limit instead of AggregationLimits
        // FIX: Stage::execute_with_limits passes limits.max_unwind_output

        // Create documents with arrays
        let docs: Vec<Value> = (0..10)
            .map(|i| {
                json!({
                    "i": i,
                    "items": (0..100).map(|j| json!({"j": j})).collect::<Vec<_>>()
                })
            })
            .collect();

        let unwind = UnwindStage::from_json(&json!("$items")).unwrap();

        // With default limit (1M), this should succeed (10 docs × 100 items = 1000)
        let result = unwind.execute(docs.clone()).unwrap();
        assert_eq!(result.len(), 1000);

        // With custom low limit (500), should fail
        let result = unwind.execute_with_limit(docs, 500);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("exceed output limit"),
            "Expected unwind limit error, got: {}",
            err
        );
    }

    #[test]
    fn test_bugfix_unwind_through_pipeline_uses_limits() {
        // Test that $unwind in pipeline uses AggregationLimits.max_unwind_output
        let docs: Vec<Value> = (0..5)
            .map(|i| {
                json!({
                    "i": i,
                    "items": (0..50).map(|j| json!({"j": j})).collect::<Vec<_>>()
                })
            })
            .collect();

        let pipeline = Pipeline::from_json(&json!([{"$unwind": "$items"}])).unwrap();

        // With restrictive max_unwind_output (100), should fail (5 docs × 50 items = 250)
        let limits = AggregationLimits {
            max_docs_without_match: 10000, // High doc limit
            max_unwind_output: 100,        // Low unwind limit
            ..Default::default()
        };

        let result = pipeline.execute_streaming_with_limits(docs.into_iter().map(Ok), limits);

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("exceed output limit") || err.contains("$unwind"),
            "Expected unwind limit error, got: {}",
            err
        );
    }

    #[test]
    fn test_bugfix_with_memory_budget_scales_above_4gb() {
        // BUG: with_memory_budget() capped at 4GB, preventing scaling on large systems
        // FIX: Removed 4GB cap, allows unlimited scaling

        let gb4 = AggregationLimits::with_memory_budget(4096);
        let gb8 = AggregationLimits::with_memory_budget(8192);
        let gb16 = AggregationLimits::with_memory_budget(16384);
        let gb32 = AggregationLimits::with_memory_budget(32768);

        // Each doubling should approximately double the limits
        assert!(
            gb8.max_docs_without_match > gb4.max_docs_without_match,
            "8GB ({}) should have higher limit than 4GB ({})",
            gb8.max_docs_without_match,
            gb4.max_docs_without_match
        );
        assert!(
            gb16.max_docs_without_match > gb8.max_docs_without_match,
            "16GB ({}) should have higher limit than 8GB ({})",
            gb16.max_docs_without_match,
            gb8.max_docs_without_match
        );
        assert!(
            gb32.max_docs_without_match > gb16.max_docs_without_match,
            "32GB ({}) should have higher limit than 16GB ({})",
            gb32.max_docs_without_match,
            gb16.max_docs_without_match
        );

        // Memory budget should be preserved, not capped
        assert_eq!(gb8.max_memory_mb, 8192);
        assert_eq!(gb16.max_memory_mb, 16384);
        assert_eq!(gb32.max_memory_mb, 32768);
    }

    #[test]
    fn test_bugfix_skip_limit_respects_doc_limit() {
        // Verify $skip + $limit combination also respects doc_limit
        // NEW BEHAVIOR: Early exit at (doc_limit - skip) docs after skipping
        let docs: Vec<Value> = (0..500).map(|i| json!({"i": i})).collect();

        let pipeline = Pipeline::from_json(&json!([
            {"$skip": 50},
            {"$limit": 1_000_000}  // Very large limit
        ]))
        .unwrap();

        let limits = AggregationLimits {
            max_docs_without_match: 100, // Only allow 100 docs to be processed
            ..Default::default()
        };

        let result = pipeline
            .execute_streaming_with_limits(docs.into_iter().map(Ok), limits)
            .unwrap();

        // Should return (doc_limit - skip) = 50 docs
        // Processes 50 skipped + 50 returned = 100 total (within limit)
        assert_eq!(result.len(), 50);
        // Verify we got the correct docs (after skip)
        assert_eq!(result[0]["i"], 50);
        assert_eq!(result[49]["i"], 99);
    }

    #[test]
    fn test_bugfix_small_group_limit_respected() {
        // BUG: Group count check only every 1000 docs
        // If max_group_count=100, could create 1000+ groups before first check
        // FIX: Adaptive check interval = max(1, limit/10)

        // Create docs that each have unique group key
        let docs: Vec<Value> = (0..200).map(|i| json!({"key": i})).collect();

        let pipeline = Pipeline::from_json(&json!([
            {"$group": {"_id": "$key", "count": {"$sum": 1}}}
        ]))
        .unwrap();

        // Set small group limit
        let limits = AggregationLimits {
            max_docs_without_match: 10000,
            max_group_count: 50, // Small limit - should fail quickly
            ..Default::default()
        };

        let result = pipeline.execute_streaming_with_limits(docs.into_iter().map(Ok), limits);

        // Should fail with group limit exceeded
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("exceeded group limit") || err.contains("unique groups"),
            "Expected group limit error, got: {}",
            err
        );
    }

    #[test]
    fn test_bugfix_low_memory_is_conservative() {
        // BUG: low_memory() used same limits as default (10K docs)
        // FIX: low_memory() now uses 1K docs, 500 groups

        let default = AggregationLimits::default();
        let low = AggregationLimits::low_memory();

        // low_memory should be significantly lower than default
        assert!(
            low.max_docs_without_match < default.max_docs_without_match,
            "low_memory ({}) should have fewer docs than default ({})",
            low.max_docs_without_match,
            default.max_docs_without_match
        );
        assert!(
            low.max_group_count < default.max_group_count,
            "low_memory ({}) should have fewer groups than default ({})",
            low.max_group_count,
            default.max_group_count
        );
        assert!(
            low.max_docs_with_match < default.max_docs_with_match,
            "low_memory ({}) should have fewer docs with match than default ({})",
            low.max_docs_with_match,
            default.max_docs_with_match
        );
    }

    #[test]
    fn test_bugfix_max_docs_with_match_scales_down() {
        // BUG: max_docs_with_match had minimum 10K, too high for low memory
        // FIX: minimum reduced to 2K, and low_memory() uses 5K

        // low_memory() should have lower limit than default
        let low = AggregationLimits::low_memory();
        let default = AggregationLimits::default();

        assert!(
            low.max_docs_with_match < default.max_docs_with_match,
            "low_memory max_docs_with_match ({}) should be < default ({})",
            low.max_docs_with_match,
            default.max_docs_with_match
        );

        // low_memory() should have 5K (not 100K like before)
        assert_eq!(low.max_docs_with_match, 5_000);

        // With memory budget, minimum is 2K (enforced)
        let tiny = AggregationLimits::with_memory_budget(32);
        assert!(
            tiny.max_docs_with_match >= 2_000,
            "Should respect minimum of 2K, got {}",
            tiny.max_docs_with_match
        );
    }
}
