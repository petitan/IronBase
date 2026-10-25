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
