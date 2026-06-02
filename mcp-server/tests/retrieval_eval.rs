//! Retrieval evaluation harness (redesign §12) — the measurement foundation that
//! gates every staged change (B/C/D/E). See `docs/HYBRID_RETRIEVAL_REDESIGN.md`.
//!
//! Because P3 forbids unvalidated magic constants, retrieval quality MUST be
//! measured, not asserted. This file provides:
//!   1. Correct, unit-tested metric implementations (nDCG@k, Recall@k, and
//!      abstention precision/recall for Stage D).
//!   2. A `LabeledQuery` data format.
//!   3. A no-regression gate: `search` must not score below `hybrid_search`
//!      grouped on a controlled labeled set (the Stage-B/A gate mechanism).
//!
//! Real per-corpus labels (rdocs + the long manual) are a separate data step;
//! the synthetic seed here proves the harness and serves as a regression guard.

use mcp_ironbase::{dispatch_tool, IronBaseAdapter};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;
use tempfile::TempDir;

// ============================================================================
// Metric implementations (pure — unit-tested below against hand-computed values)
// ============================================================================

/// Recall@k = |relevant ∩ retrieved[..k]| / |relevant|.
/// Empty `relevant` → 1.0 (vacuous: nothing to recall).
fn recall_at_k(retrieved: &[String], relevant: &HashSet<String>, k: usize) -> f64 {
    if relevant.is_empty() {
        return 1.0;
    }
    let hits = retrieved
        .iter()
        .take(k)
        .filter(|id| relevant.contains(*id))
        .count();
    hits as f64 / relevant.len() as f64
}

/// Binary-relevance nDCG@k. DCG = Σ rel_i / log2(i+2); IDCG = ideal ordering.
/// Returns 0.0 when there are no relevant docs (IDCG = 0).
fn ndcg_at_k(retrieved: &[String], relevant: &HashSet<String>, k: usize) -> f64 {
    let dcg: f64 = retrieved
        .iter()
        .take(k)
        .enumerate()
        .map(|(i, id)| {
            let rel = if relevant.contains(id) { 1.0 } else { 0.0 };
            rel / ((i as f64) + 2.0).log2()
        })
        .sum();

    let ideal_hits = relevant.len().min(k);
    let idcg: f64 = (0..ideal_hits)
        .map(|i| 1.0 / ((i as f64) + 2.0).log2())
        .sum();

    if idcg == 0.0 {
        0.0
    } else {
        dcg / idcg
    }
}

/// Abstention precision/recall for the `none`/`unknown` verdict (Stage D).
/// `predicted_abstain` and `actually_unanswerable` are aligned per query.
/// Returns (precision, recall). Defined here so the gate exists before Stage D.
fn abstention_metrics(predicted_abstain: &[bool], actually_unanswerable: &[bool]) -> (f64, f64) {
    let mut tp = 0usize; // abstained AND unanswerable
    let mut fp = 0usize; // abstained BUT answerable
    let mut fn_ = 0usize; // did not abstain BUT unanswerable
    for (&pred, &truth) in predicted_abstain.iter().zip(actually_unanswerable.iter()) {
        match (pred, truth) {
            (true, true) => tp += 1,
            (true, false) => fp += 1,
            (false, true) => fn_ += 1,
            (false, false) => {}
        }
    }
    let precision = if tp + fp == 0 {
        1.0
    } else {
        tp as f64 / (tp + fp) as f64
    };
    let recall = if tp + fn_ == 0 {
        1.0
    } else {
        tp as f64 / (tp + fn_) as f64
    };
    (precision, recall)
}

// ============================================================================
// Metric unit tests (hand-computed — a buggy metric makes the gate meaningless)
// ============================================================================

#[test]
fn test_recall_at_k_basic() {
    let relevant: HashSet<String> = ["a", "b"].iter().map(|s| s.to_string()).collect();
    let retrieved: Vec<String> = ["a", "x", "b"].iter().map(|s| s.to_string()).collect();
    assert!((recall_at_k(&retrieved, &relevant, 2) - 0.5).abs() < 1e-9); // top2 = a,x → {a}
    assert!((recall_at_k(&retrieved, &relevant, 3) - 1.0).abs() < 1e-9); // top3 → a,b
    assert!((recall_at_k(&retrieved, &relevant, 1) - 0.5).abs() < 1e-9);
}

#[test]
fn test_recall_at_k_empty_relevant_is_vacuous() {
    let relevant: HashSet<String> = HashSet::new();
    let retrieved: Vec<String> = vec!["a".to_string()];
    assert!((recall_at_k(&retrieved, &relevant, 5) - 1.0).abs() < 1e-9);
}

#[test]
fn test_ndcg_perfect_ranking_is_one() {
    let relevant: HashSet<String> = ["a", "b"].iter().map(|s| s.to_string()).collect();
    let retrieved: Vec<String> = ["a", "b", "x"].iter().map(|s| s.to_string()).collect();
    assert!((ndcg_at_k(&retrieved, &relevant, 3) - 1.0).abs() < 1e-9);
}

#[test]
fn test_ndcg_interleaved_hand_computed() {
    // retrieved [a, x, b], relevant {a, b}:
    // DCG  = 1/log2(2) + 0 + 1/log2(4) = 1.0 + 0.5 = 1.5
    // IDCG = 1/log2(2) + 1/log2(3)     = 1.0 + 0.63092975 = 1.63092975
    // nDCG = 1.5 / 1.63092975 = 0.91972...
    let relevant: HashSet<String> = ["a", "b"].iter().map(|s| s.to_string()).collect();
    let retrieved: Vec<String> = ["a", "x", "b"].iter().map(|s| s.to_string()).collect();
    let got = ndcg_at_k(&retrieved, &relevant, 3);
    assert!((got - 0.919_721_4).abs() < 1e-6, "ndcg = {got}");
}

#[test]
fn test_ndcg_no_relevant_is_zero() {
    let relevant: HashSet<String> = HashSet::new();
    let retrieved: Vec<String> = ["a"].iter().map(|s| s.to_string()).collect();
    assert_eq!(ndcg_at_k(&retrieved, &relevant, 3), 0.0);
}

#[test]
fn test_abstention_metrics_hand_computed() {
    // predicted: [T, T, F, F], truth(unanswerable): [T, F, T, F]
    // tp=1 (q0), fp=1 (q1), fn=1 (q2)
    // precision = 1/(1+1)=0.5 ; recall = 1/(1+1)=0.5
    let pred = [true, true, false, false];
    let truth = [true, false, true, false];
    let (p, r) = abstention_metrics(&pred, &truth);
    assert!((p - 0.5).abs() < 1e-9, "precision = {p}");
    assert!((r - 0.5).abs() < 1e-9, "recall = {r}");
}

// ============================================================================
// Test harness plumbing
// ============================================================================

fn create_test_adapter() -> (Arc<IronBaseAdapter>, TempDir) {
    let temp_dir = TempDir::new().expect("tempdir");
    let db_path = temp_dir.path().join("eval.mlite");
    let adapter = IronBaseAdapter::new(db_path.to_string_lossy().to_string()).expect("adapter");
    (Arc::new(adapter), temp_dir)
}

fn dispatch_ok(adapter: &Arc<IronBaseAdapter>, name: &str, params: Value) -> Value {
    dispatch_tool(name, params, adapter, None, None, None, None, &None, &None)
        .unwrap_or_else(|e| panic!("Tool '{}' should succeed: {:?}", name, e))
}

/// A labeled query for the eval set: which documents are relevant, and whether
/// the corpus can answer it at all (the abstention target for Stage D).
struct LabeledQuery {
    query: &'static str,
    relevant: &'static [&'static str],
    #[allow(dead_code)] // consumed by abstention metrics in Stage D
    answerable: bool,
}

/// Seed a labeled RAG-style collection: 3 documents, each two chunks, with a
/// shared query token so retrieval is non-trivial.
fn seed_labeled_kb(adapter: &Arc<IronBaseAdapter>) {
    let docs = [
        ("alpha", 0, "fékpad PEF-35 berendezés leírás"),
        ("alpha", 1, "fékpad PEF-35 ár"),
        ("beta", 0, "kombinált vizsgasori berendezés árajánlat"),
        ("beta", 1, "vizsgasori garancia"),
        ("gamma", 0, "iroda bútor szék asztal"),
        ("gamma", 1, "iroda világítás"),
    ];
    for (doc_id, idx, content) in docs {
        dispatch_ok(
            adapter,
            "insert_one",
            json!({"collection": "kb", "document": {
                "doc_id": doc_id, "chunk_index": idx, "content": content,
                "title": format!("{} címe", doc_id),
            }}),
        );
    }
    dispatch_ok(
        adapter,
        "index_create_fulltext",
        json!({"collection": "kb", "field": "content"}),
    );
}

/// Run `search` and return its document order.
fn search_doc_order(adapter: &Arc<IronBaseAdapter>, query: &str) -> Vec<String> {
    let res = dispatch_ok(
        adapter,
        "search",
        json!({"collection": "kb", "query": query}),
    );
    res["documents"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|d| d["doc_id"].as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// Stage-A gate (redesign §12 metric 1): `search` must retrieve the labeled
/// relevant documents with non-trivial nDCG@k / Recall@k on the eval set. The
/// former vs-`hybrid_search` no-regression baseline was dropped when that tool
/// was removed (`search` is now the sole retrieval surface), so this is an
/// absolute quality floor rather than a relative comparison.
#[test]
fn test_search_quality_on_labeled_set() {
    let (adapter, _tmp) = create_test_adapter();
    seed_labeled_kb(&adapter);

    let eval_set = [
        LabeledQuery {
            query: "fékpad PEF-35",
            relevant: &["alpha"],
            answerable: true,
        },
        LabeledQuery {
            query: "vizsgasori berendezés",
            relevant: &["beta"],
            answerable: true,
        },
        LabeledQuery {
            query: "berendezés",
            relevant: &["alpha", "beta"],
            answerable: true,
        },
        LabeledQuery {
            query: "iroda bútor",
            relevant: &["gamma"],
            answerable: true,
        },
    ];

    let k = 5;
    let mut search_ndcg_sum = 0.0;
    let mut search_recall_sum = 0.0;

    for lq in &eval_set {
        let relevant: HashSet<String> = lq.relevant.iter().map(|s| s.to_string()).collect();

        let s_order = search_doc_order(&adapter, lq.query);

        // The labeled relevant docs must actually be retrievable (sanity: the
        // harness is exercising real retrieval, not an empty set).
        assert!(
            !s_order.is_empty(),
            "search returned nothing for '{}'",
            lq.query
        );

        search_ndcg_sum += ndcg_at_k(&s_order, &relevant, k);
        search_recall_sum += recall_at_k(&s_order, &relevant, k);
    }

    let n = eval_set.len() as f64;
    let (s_ndcg, s_recall) = (search_ndcg_sum / n, search_recall_sum / n);

    // Absolute floors: the labeled relevant docs are retrieved (recall) and
    // ranked sensibly (nDCG). The clean single/two-token queries should rank the
    // relevant docs at or near the top, so a >0.5 mean is a non-vacuous gate.
    assert!(
        s_recall > 0.5,
        "labeled recall implausibly low: {s_recall:.4}"
    );
    assert!(s_ndcg > 0.5, "labeled nDCG implausibly low: {s_ndcg:.4}");
}
