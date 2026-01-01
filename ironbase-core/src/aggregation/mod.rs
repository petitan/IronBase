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

mod helpers;
mod pipeline;
mod stages;
mod types;

// Re-export public types
pub use types::AggregationLimits;
pub use types::Pipeline;

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
        assert_eq!(limits.max_docs_without_match, 100_000);
        assert_eq!(limits.max_group_count, 50_000);
        assert_eq!(limits.max_memory_mb, 512);
    }

    #[test]
    fn test_aggregation_limits_low_memory() {
        let limits = AggregationLimits::low_memory();
        assert_eq!(limits.max_docs_without_match, 10_000);
        assert_eq!(limits.max_group_count, 5_000);
        assert_eq!(limits.max_memory_mb, 128);
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
            max_group_count: 50_000,
            max_memory_mb: 512,
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
            max_group_count: 50_000,
            max_memory_mb: 512,
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
            max_memory_mb: 512,
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
}
