//! Performance regression tests (CI job `Performance regression`)
//!
//! Absolute timings on shared CI runners are too noisy to gate on, so two
//! kinds of checks that are not:
//!
//! - **Work counts** (deterministic, any build): the number of HNSW distance
//!   evaluations for a fixed, seeded data set must stay under a bound. An
//!   algorithmic regression (a lost early stop, a wider neighbour heuristic)
//!   raises it; noise cannot.
//! - **Speed ratios** (release builds only, skipped in debug): a fast path
//!   measured against a plain reference in the same process — same machine,
//!   same moment, so the noise cancels. Each guards an optimisation that once
//!   went missing unnoticed: distance kernels that never vectorized (T6),
//!   documents parsed with their embedding (T2), JSON posting lists (D2).
//!
//! Every test first checks that the measured path returns the right result,
//! so a path that got "faster" by doing the wrong thing fails.
//!
//! Run locally: `cargo test --release -p ironbase-core --lib perf_ -- --test-threads=1`.
//! A bound or ratio that an intended change moves is updated here, with the
//! new measurement in the commit message.

#[allow(unused_imports)]
use crate::storage::Storage;
use std::hint::black_box;
use std::time::{Duration, Instant};

use crate::document::DocumentId;
use crate::vector::{simd, DistanceMetric, HnswIndex, VectorIndexConfig, DISTANCE_EVALS};

/// Deterministic pseudo-random vectors in [-1, 1]
fn vectors(n: usize, dim: usize, seed: u64) -> Vec<Vec<f32>> {
    let mut state = seed;
    (0..n)
        .map(|_| {
            (0..dim)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    (state % 2001) as f32 / 1000.0 - 1.0
                })
                .collect()
        })
        .collect()
}

/// Distance evaluations made by `work` on this thread
fn distance_evals(work: impl FnOnce()) -> u64 {
    DISTANCE_EVALS.with(|evals| evals.set(0));
    work();
    DISTANCE_EVALS.with(|evals| evals.get())
}

// ============================================================================
// Work counts
// ============================================================================

/// 1000 inserts and 200 top-10 searches of 16-dimensional vectors with the
/// default HNSW parameters (M=16, ef_construction=200, ef_search=50).
/// Measured (v0.3.427): cosine 5758 evaluations per insert and 590 per
/// search, euclidean 5833 / 588. The bounds leave 25% headroom.
#[test]
fn perf_hnsw_distance_evaluations() {
    const INSERT_BOUND: u64 = 7_300;
    const SEARCH_BOUND: u64 = 740;
    for metric in [DistanceMetric::Cosine, DistanceMetric::Euclidean] {
        let data = vectors(1000, 16, 42);
        let mut index = HnswIndex::new(VectorIndexConfig::new(16).with_metric(metric));
        let build = distance_evals(|| {
            for (i, v) in data.iter().enumerate() {
                index.insert(&format!("v{i}"), v).unwrap();
            }
        });
        let queries = vectors(200, 16, 7);
        let mut returned = 0;
        let search = distance_evals(|| {
            for q in &queries {
                returned += black_box(index.search(q, 10)).len();
            }
        });
        assert_eq!(index.len(), 1000, "{metric:?}: indexed vectors");
        assert_eq!(
            returned,
            200 * 10,
            "{metric:?}: results of 200 top-10 searches"
        );
        // An indexed vector finds itself (outside the counted work)
        for i in (0..1000).step_by(20) {
            let top = index.search(&data[i], 1);
            assert_eq!(
                top[0].id,
                format!("v{i}"),
                "{metric:?}: self-search of v{i}"
            );
        }
        let (per_insert, per_search) = (build / 1000, search / 200);
        assert!(
            per_insert <= INSERT_BOUND,
            "{metric:?}: {per_insert} distance evaluations per insert, bound {INSERT_BOUND}"
        );
        assert!(
            per_search <= SEARCH_BOUND,
            "{metric:?}: {per_search} distance evaluations per search, bound {SEARCH_BOUND}"
        );
    }
}

// ============================================================================
// Speed ratios (release only)
// ============================================================================

/// Whether ratios can be measured: a debug build neither vectorizes nor
/// inlines, so its ratios say nothing
fn release_build() -> bool {
    if cfg!(debug_assertions) {
        eprintln!("skipped: speed ratios need a release build (cargo test --release)");
        return false;
    }
    true
}

/// Best of `rounds` timings of `f`: the minimum is the least disturbed run
fn best_time(rounds: usize, mut f: impl FnMut()) -> Duration {
    (0..rounds)
        .map(|_| {
            let start = Instant::now();
            f();
            start.elapsed()
        })
        .min()
        .unwrap()
}

/// `fast` must run at least `min_speedup` times faster than `reference`
fn assert_speedup(name: &str, min_speedup: f64, reference: impl FnMut(), fast: impl FnMut()) {
    let reference = best_time(15, reference);
    let fast = best_time(15, fast);
    let speedup = reference.as_secs_f64() / fast.as_secs_f64().max(1e-9);
    eprintln!("{name}: {speedup:.2}x (reference {reference:?}, fast {fast:?})");
    assert!(
        speedup >= min_speedup,
        "{name}: only {speedup:.2}x faster than the reference (expected >= {min_speedup}x)"
    );
}

/// The serial sums the kernels used before T6: one running total, which
/// LLVM may not reorder into SIMD lanes
fn serial_dot(a: &[f32], b: &[f32]) -> f32 {
    let mut sum = 0.0f32;
    for i in 0..a.len() {
        sum += a[i] * b[i];
    }
    sum
}

fn serial_squared_euclidean(a: &[f32], b: &[f32]) -> f32 {
    let mut sum = 0.0f32;
    for i in 0..a.len() {
        let d = a[i] - b[i];
        sum += d * d;
    }
    sum
}

fn serial_cosine(a: &[f32], b: &[f32]) -> f32 {
    let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    dot / (na * nb).sqrt()
}

type Kernel = fn(&[f32], &[f32]) -> f32;

/// T6: the distance kernels keep independent SIMD lanes. Measured 2–3x on
/// x86-64 (SSE2 baseline) for 384 dimensions; a kernel back on one serial
/// sum is ~1x.
#[test]
fn perf_distance_kernels_vectorize() {
    if !release_build() {
        return;
    }
    // 256 x 384 floats: fits in L2, so the arithmetic is measured, not memory
    let data = vectors(256, 384, 3);
    let query = &vectors(1, 384, 5)[0];
    let kernels: [(&str, Kernel, Kernel); 3] = [
        ("dot_product", serial_dot, simd::dot_product),
        (
            "squared_euclidean_distance",
            serial_squared_euclidean,
            simd::squared_euclidean_distance,
        ),
        ("cosine_similarity", serial_cosine, simd::cosine_similarity),
    ];
    let data = &data;
    for (name, reference, fast) in kernels {
        for v in data {
            let (expected, actual) = (reference(query, v), fast(query, v));
            assert!(
                (expected - actual).abs() <= 1e-3 * expected.abs().max(1.0),
                "{name}: {actual} != reference {expected}"
            );
        }
        let run = |kernel: Kernel| {
            move || {
                let mut acc = 0.0f32;
                for _ in 0..20 {
                    for v in data {
                        acc += kernel(black_box(query), black_box(v));
                    }
                }
                black_box(acc);
            }
        };
        assert_speedup(name, 1.5, run(reference), run(fast));
    }
}

/// T2: a document read that skips its embedding does not parse it. Measured
/// ~2.4x on a chunk with a 768-dimensional embedding (the skipped numbers
/// are still scanned, not built); a full parse is ~1x.
#[test]
fn perf_document_parse_skips_embedding() {
    if !release_build() {
        return;
    }
    let embedding = &vectors(1, 768, 9)[0];
    let doc = serde_json::json!({
        "_id": 1, "_collection": "kb", "doc_id": "d1",
        "content": "maintenance step ".repeat(40), "embedding": embedding,
    });
    let bytes = serde_json::to_vec(&doc).unwrap();
    let skip: std::collections::HashSet<String> = ["embedding".to_string()].into_iter().collect();
    let mut without_embedding = doc.clone();
    without_embedding
        .as_object_mut()
        .unwrap()
        .remove("embedding");
    assert_eq!(
        crate::collection_core::parse_document_skipping(&bytes, &skip).unwrap(),
        without_embedding
    );
    assert_speedup(
        "parse_document_skipping",
        1.6,
        || {
            for _ in 0..50 {
                black_box(serde_json::from_slice::<serde_json::Value>(black_box(&bytes)).unwrap());
            }
        },
        || {
            for _ in 0..50 {
                black_box(
                    crate::collection_core::parse_document_skipping(black_box(&bytes), &skip)
                        .unwrap(),
                );
            }
        },
    );
}

/// D2: V4 binary posting lists decode faster than V3 JSON. Measured ~4x on
/// 20K entries.
#[test]
fn perf_posting_list_v4_decode() {
    if !release_build() {
        return;
    }
    use crate::fulltext::{
        decode_token_entries, encode_token_entries, FTIDX_VERSION_V3, FTIDX_VERSION_V4,
    };
    let entries: Vec<(DocumentId, u32)> = (0..20_000i64)
        .map(|i| (DocumentId::Int(i * 3 + 1000), (i % 5 + 1) as u32))
        .collect();
    let json = serde_json::to_vec(&entries).unwrap();
    let binary = encode_token_entries(&entries);
    assert_eq!(
        decode_token_entries(&json, FTIDX_VERSION_V3).unwrap(),
        entries
    );
    assert_eq!(
        decode_token_entries(&binary, FTIDX_VERSION_V4).unwrap(),
        entries
    );
    assert_speedup(
        "decode_token_entries V4",
        2.0,
        || {
            black_box(decode_token_entries(black_box(&json), FTIDX_VERSION_V3).unwrap());
        },
        || {
            black_box(decode_token_entries(black_box(&binary), FTIDX_VERSION_V4).unwrap());
        },
    );
}

/// The B+ tree inserts index entries at most `MAX_SLOWDOWN` times slower
/// than std `BTreeMap` takes the same pairs. Measured ~2.5x; while the split
/// check cloned the leaf on every insert and an internal node's whole subtree
/// on every child split, it was 20–26x (100K entries).
#[test]
fn perf_btree_insert_vs_btreemap() {
    const MAX_SLOWDOWN: f64 = 6.0;
    if !release_build() {
        return;
    }
    use crate::index::{BPlusTree, IndexKey};
    let entries: Vec<(IndexKey, DocumentId)> = (0..100_000i64)
        .map(|i| {
            let key = IndexKey::String(format!("k{}", (i * 7919) % 1_000_003));
            (key, DocumentId::Int(i))
        })
        .collect();
    let mut tree = BPlusTree::new("i".into(), "f".into(), false, false);
    for (key, doc_id) in entries.iter().cloned() {
        tree.insert(key, doc_id).unwrap();
    }
    let mut stored = tree.get_all_entries();
    stored.sort();
    let mut expected = entries.clone();
    expected.sort();
    assert!(
        stored == expected,
        "the tree must hold exactly the inserted entries"
    );
    let btree = best_time(5, || {
        let mut tree = BPlusTree::new("i".into(), "f".into(), false, false);
        for (key, doc_id) in entries.iter().cloned() {
            tree.insert(key, doc_id).unwrap();
        }
        black_box(tree);
    });
    let reference = best_time(5, || {
        let mut map = std::collections::BTreeMap::new();
        for entry in entries.iter().cloned() {
            map.insert(entry, ());
        }
        black_box(map);
    });
    let slowdown = btree.as_secs_f64() / reference.as_secs_f64().max(1e-9);
    eprintln!("BPlusTree::insert: {slowdown:.2}x BTreeMap (tree {btree:?}, map {reference:?})");
    assert!(
        slowdown <= MAX_SLOWDOWN,
        "BPlusTree::insert: {slowdown:.2}x slower than BTreeMap (expected <= {MAX_SLOWDOWN}x)"
    );
}

/// Building an index on a low-cardinality field (5 values, 20K documents
/// each) costs at most `MAX_SLOWDOWN` times building it on a unique field.
/// Measured ~1.3x; while finding an entry scanned every entry with its key,
/// it was ~300x (100K entries; speed review 2026-10-25, A).
#[test]
fn perf_btree_build_low_cardinality() {
    const MAX_SLOWDOWN: f64 = 3.0;
    if !release_build() {
        return;
    }
    use crate::index::{BPlusTree, IndexKey};
    let build = |distinct: i64| {
        let entries: Vec<(IndexKey, DocumentId)> = (0..100_000i64)
            .map(|i| {
                (
                    IndexKey::String(format!("k{}", i % distinct)),
                    DocumentId::Int(i),
                )
            })
            .collect();
        move || {
            let mut tree = BPlusTree::new("i".into(), "f".into(), false, false);
            tree.begin_build();
            for (key, doc_id) in entries.iter().cloned() {
                tree.build_insert(key, doc_id).unwrap();
            }
            tree.end_build();
            assert_eq!(tree.size(), 100_000);
            black_box(tree);
        }
    };
    // The low-cardinality build holds exactly its entries, in order
    let mut tree = BPlusTree::new("i".into(), "f".into(), false, false);
    tree.begin_build();
    let mut expected: Vec<(IndexKey, DocumentId)> = (0..100_000i64)
        .map(|i| (IndexKey::String(format!("k{}", i % 5)), DocumentId::Int(i)))
        .collect();
    for (key, doc_id) in expected.iter().cloned() {
        tree.build_insert(key, doc_id).unwrap();
    }
    tree.end_build();
    expected.sort();
    assert!(
        tree.get_all_entries() == expected,
        "the build must hold exactly its entries, in order"
    );
    let low = best_time(5, build(5));
    let unique = best_time(5, build(100_000));
    let slowdown = low.as_secs_f64() / unique.as_secs_f64().max(1e-9);
    eprintln!(
        "build_insert, 5 keys: {slowdown:.2}x unique keys (5 keys {low:?}, unique {unique:?})"
    );
    assert!(
        slowdown <= MAX_SLOWDOWN,
        "build_insert on 5 keys: {slowdown:.2}x slower than on unique keys (expected <= {MAX_SLOWDOWN}x)"
    );
}

/// An indexed find reads each matching document once: the index plan's
/// verification keeps the documents for the result (speed review
/// 2026-10-25, C). It read every one twice before. Deterministic (any
/// build).
#[test]
fn perf_indexed_find_reads_each_document_once() {
    use crate::collection_core::DOC_READS;
    use crate::find_options::FindOptions;
    use crate::storage::MemoryStorage;
    use crate::DatabaseCore;
    use serde_json::json;

    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    for i in 0..3000 {
        let doc = std::collections::HashMap::from([
            ("category".to_string(), json!(i % 3)),
            ("n".to_string(), json!(i)),
        ]);
        db.insert_one("c", doc).unwrap();
    }
    let coll = db.get_collection("c").unwrap();
    coll.create_index("category".to_string(), false, false)
        .unwrap();
    let reads = |work: &dyn Fn() -> usize| {
        DOC_READS.with(|reads| reads.set(0));
        let found = work();
        (found, DOC_READS.with(|reads| reads.get()))
    };

    // Distinct queries: a cached result reads once anyway
    let (found, n) = reads(&|| coll.find(&json!({"category": 1})).unwrap().len());
    assert_eq!((found, n), (1000, 1000), "find: (documents, reads)");
    let (found, n) = reads(&|| {
        coll.find_with_options(&json!({"category": 2}), FindOptions::new())
            .unwrap()
            .len()
    });
    assert_eq!(
        (found, n),
        (1000, 1000),
        "find_with_options: (documents, reads)"
    );

    // Results agree with a collection scan (`n` is not indexed), filtered here
    let indexed = coll.find(&json!({"category": 0})).unwrap();
    let scanned: Vec<serde_json::Value> = coll
        .find(&json!({"n": {"$gte": 0}}))
        .unwrap()
        .into_iter()
        .filter(|d| d["category"] == json!(0))
        .collect();
    assert_eq!(indexed.len(), 1000);
    let key = |d: &serde_json::Value| d["n"].as_i64().unwrap();
    let mut indexed_n: Vec<i64> = indexed.iter().map(key).collect();
    let mut scanned_n: Vec<i64> = scanned.iter().map(key).collect();
    indexed_n.sort();
    scanned_n.sort();
    assert_eq!(indexed_n, scanned_n);

    // Over the response cap the kept documents are dropped and the find
    // reports the cap exactly as before
    let capped = coll.find_with_options(
        &json!({"category": 1, "n": {"$gte": 0}}),
        FindOptions::new().with_max_response_bytes(10_000),
    );
    let err = capped.unwrap_err().to_string();
    assert!(err.contains("Response size limit exceeded"), "{err}");
}

/// After a delete, an empty-filter `$group` count and sort still use the
/// index: a deleted document stays in the catalog (pointing at its
/// tombstone), so comparing the index with the catalog size turned the index
/// paths off for good after one delete (speed review 2026-10-25, F; the
/// benchmark's `$group` count went from ~20 ms to ~1 s). Deterministic.
#[test]
fn perf_index_paths_survive_a_delete() {
    use crate::find_options::FindOptions;
    use crate::storage::MemoryStorage;
    use crate::DatabaseCore;
    use serde_json::json;

    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    for i in 0..1000 {
        let doc = std::collections::HashMap::from([
            ("category".to_string(), json!(i % 5)),
            ("n".to_string(), json!(i)),
        ]);
        db.insert_one("c", doc).unwrap();
    }
    let coll = db.get_collection("c").unwrap();
    coll.create_index("category".to_string(), false, false)
        .unwrap();
    coll.create_index("n".to_string(), false, false).unwrap();
    assert!(coll.field_index_covers_every_doc("category"));

    db.delete_one("c", &json!({"n": 3})).unwrap();
    assert!(
        coll.field_index_covers_every_doc("category"),
        "a delete must not turn the index paths off"
    );

    // Each runs on the index: no document is read (a scan reads ~1000)
    let reads = |work: &mut dyn FnMut()| {
        crate::collection_core::DOC_READS.with(|reads| reads.set(0));
        work();
        crate::collection_core::DOC_READS.with(|reads| reads.get())
    };

    // The index-based count leaves the deleted document out
    let mut counts: Vec<(i64, i64)> = Vec::new();
    let group_reads = reads(&mut || {
        let groups = coll
            .aggregate(&json!([{"$group": {"_id": "$category", "count": {"$sum": 1}}}]))
            .unwrap();
        counts = groups
            .iter()
            .map(|g| (g["_id"].as_i64().unwrap(), g["count"].as_i64().unwrap()))
            .collect();
    });
    counts.sort();
    assert_eq!(
        counts,
        vec![(0, 200), (1, 200), (2, 200), (3, 199), (4, 200)]
    );
    assert_eq!(group_reads, 0, "$group count after a delete read documents");

    // distinct on the index leaves out a value only the deleted document had
    db.insert_one(
        "c",
        std::collections::HashMap::from([("category".to_string(), json!(99))]),
    )
    .unwrap();
    db.delete_one("c", &json!({"category": 99})).unwrap();
    let mut values: Vec<i64> = Vec::new();
    let distinct_reads = reads(&mut || {
        values = coll
            .distinct("category", &json!({}))
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap())
            .collect();
    });
    values.sort();
    assert_eq!(values, vec![0, 1, 2, 3, 4]);
    assert_eq!(distinct_reads, 0, "distinct after a delete read documents");

    // An empty-filter sort served by the index skips it too, in order, and
    // reads only the page
    let mut n: Vec<i64> = Vec::new();
    let sort_reads = reads(&mut || {
        n = coll
            .find_with_options(
                &json!({}),
                FindOptions::new()
                    .with_sort(vec![("n".to_string(), 1)])
                    .with_limit(5),
            )
            .unwrap()
            .iter()
            .map(|d| d["n"].as_i64().unwrap())
            .collect();
    });
    assert_eq!(n, vec![0, 1, 2, 4, 5]);
    assert!(
        sort_reads <= 10,
        "index sort after a delete read {sort_reads} documents"
    );
}

/// E: converting a stored document for query matching moves the object's
/// entries instead of going through the derived `#[serde(flatten)]`, which
/// buffers every value through serde's intermediate representation.
#[test]
fn perf_document_from_value_skips_flatten() {
    use crate::document::Document;
    if !release_build() {
        return;
    }
    let doc = serde_json::json!({
        "_id": 12345, "name": "User_12345", "email": "user12345@example.com",
        "age": 42, "score": 415, "category": "electronics", "city": "London",
        "active": true, "balance": 18517.5,
        "profile": {"level": 5, "points": 1234, "rank": "rank_45"},
        "tags": ["tag_5", "tag_6", "tag_7"],
    });
    let owned = Document::from_value_owned(doc.clone()).unwrap();
    assert_eq!(owned.id, crate::document::DocumentId::Int(12345));
    assert_eq!(owned.fields.len(), 11);
    // Each round clones the input (both paths take it owned): the clone
    // alone is timed too and left out of both sides
    let rounds = |convert: fn(serde_json::Value)| {
        best_time(15, || {
            for _ in 0..200 {
                convert(black_box(&doc).clone());
            }
        })
    };
    let clone = rounds(|value| drop(black_box(value)));
    let derived = rounds(|value| {
        drop(black_box(
            serde_json::from_value::<Document>(value).unwrap(),
        ))
    });
    let owned = rounds(|value| drop(black_box(Document::from_value_owned(value).unwrap())));
    let speedup = derived.saturating_sub(clone).as_secs_f64()
        / owned.saturating_sub(clone).as_secs_f64().max(1e-9);
    eprintln!(
        "Document::from_value_owned: {speedup:.2}x (derived {derived:?}, owned {owned:?}, clone {clone:?})"
    );
    assert!(
        speedup >= 2.5,
        "Document::from_value_owned: only {speedup:.2}x faster than the derived path (expected >= 2.5x)"
    );
}

/// Writing a new document costs the same in a large collection as in an
/// empty one: only a rewrite of a known id removes it from
/// `document_order`. Measured ~2x (the larger catalog misses the cache);
/// while every write scanned the whole order for the id, 1000 inserts into
/// 100K documents were ~250x slower (speed review 2026-10-26, insert_many).
#[test]
fn perf_insert_into_large_collection() {
    const MAX_SLOWDOWN: f64 = 5.0;
    use crate::storage::{MemoryStorage, RawStorage};
    let order = |storage: &MemoryStorage| {
        storage
            .get_collection_meta("c")
            .unwrap()
            .document_order
            .clone()
    };
    // A rewrite moves the id to the end; a new id is appended once
    let mut storage = MemoryStorage::new();
    storage.create_collection("c").unwrap();
    for id in [1, 2, 3, 2, 4] {
        storage
            .write_document_raw("c", &DocumentId::Int(id), b"{}")
            .unwrap();
    }
    let ids = |v: &[i64]| v.iter().map(|&i| DocumentId::Int(i)).collect::<Vec<_>>();
    assert_eq!(order(&storage), ids(&[1, 3, 2, 4]));

    if !release_build() {
        return;
    }
    let filled = |n: i64| {
        let mut storage = MemoryStorage::new();
        storage.create_collection("c").unwrap();
        for id in 0..n {
            storage
                .write_document_raw("c", &DocumentId::Int(id), b"{}")
                .unwrap();
        }
        storage
    };
    let mut next_id = 1_000_000i64;
    let mut insert_1000 = |storage: &mut MemoryStorage| {
        for _ in 0..1000 {
            next_id += 1;
            storage
                .write_document_raw("c", &DocumentId::Int(next_id), b"{}")
                .unwrap();
        }
    };
    let mut large = filled(100_000);
    let into_large = best_time(5, || insert_1000(&mut large));
    let into_empty = best_time(5, || insert_1000(&mut filled(0)));
    assert_eq!(
        large.get_collection_meta("c").unwrap().document_order.len(),
        105_000
    );
    let slowdown = into_large.as_secs_f64() / into_empty.as_secs_f64().max(1e-9);
    eprintln!(
        "1000 inserts into 100K documents: {slowdown:.2}x an empty collection (large {into_large:?}, empty {into_empty:?})"
    );
    assert!(
        slowdown <= MAX_SLOWDOWN,
        "1000 inserts into 100K documents: {slowdown:.2}x slower than into an empty collection (expected <= {MAX_SLOWDOWN}x)"
    );
}
