//! Compact response contract for the `search` tool (Stage A of the hybrid
//! retrieval redesign — see `docs/HYBRID_RETRIEVAL_REDESIGN.md`).
//!
//! Builds a small-model-friendly evidence package directly from the shared
//! pipeline's `DocGroup`s (no JSON round-trip through `hybrid_search`):
//! document-anchored passages, no `embedding`, no `_`-prefixed engine metadata,
//! no chunk-tracking fields. Stage A makes no calibration/abstention claims, so
//! `verdict` is always `"unknown"` (the four-state contract of the redesign §9).

use crate::error::Result;
use crate::tools::hybrid::DocGroup;
use serde_json::{json, Map, Value};

/// Chunk-level structural / RAG-bookkeeping keys never surfaced to the model.
const CHUNK_TRACKING_KEYS: &[&str] = &[
    "embedding",
    "chunk_index",
    "chunk_total",
    "start_char",
    "end_char",
    "table_header",
    "chunk_merged",
    "chunks_in_merge",
];

/// Default context budget (characters). A budget, not a relevance decision —
/// out of scope of the "no unvalidated magic constants" rule (redesign P3).
/// ~12k chars ≈ ~3k tokens, a conservative fit for a small local model's window.
const DEFAULT_MAX_CONTEXT_CHARS: usize = 12_000;

fn is_stripped_chunk_key(k: &str) -> bool {
    k.starts_with('_') || CHUNK_TRACKING_KEYS.contains(&k)
}

/// Build the compact `search` response from the shared pipeline's grouped output.
///
/// - `text_field` is the collection's resolved primary text field (passage body).
/// - `total_chunks` / `qualified_doc_count` feed the debug diagnostics only.
/// - `format` is `"structured"` (default) or `"context_block"`.
pub(crate) fn assemble(
    groups: &[DocGroup],
    total_chunks: usize,
    qualified_doc_count: Option<usize>,
    text_field: &str,
    format: &str,
    debug: bool,
) -> Result<Value> {
    let mut used_chars: usize = 0;
    let mut trimmed = false;
    // Groups whose chunks carried no body text under `text_field` (body under a
    // different field) — surfaced in the response (P6), never silently dropped.
    let mut dropped_empty: usize = 0;
    // Groups that matched and HAD body text but did not fit the char budget — the
    // count silently shrinking the result set is exactly the bug this fixes, so it
    // is surfaced too (P6).
    let mut dropped_due_to_budget: usize = 0;

    // A staged document: doc-level fields + its ordered passage bodies. The char
    // budget is spent in two passes — Pass 1 reserves ONE passage per document
    // (breadth), Pass 2 fills the remainder with further passages (depth). This
    // stops a single large document from monopolizing the budget and starving the
    // rest (the depth-first allocation collapsed broad queries to a few docs).
    struct Staged {
        doc_id: String,
        relevance: f64,
        doc_fields: Map<String, Value>,
        texts: Vec<String>,
        emitted: usize,
    }

    // Extract doc-level fields + non-empty passage bodies per group.
    let mut staged: Vec<Staged> = Vec::with_capacity(groups.len());
    for g in groups {
        // Doc-level fields: the lifted set (already free of engine keys — lift
        // never promotes doc_id/best_score/chunk_count/chunks or `_`-prefixed/
        // tracking keys). Exclude the text field: a non-"content" body field can
        // lift when all chunks share it; it must stay in passages, not duplicate.
        let mut doc_fields: Map<String, Value> = Map::new();
        for (k, v) in &g.lifted {
            if k != text_field {
                doc_fields.insert(k.clone(), v.clone());
            }
        }

        // Single-chunk groups are not lifted (#69) — pull their doc-level fields
        // from the chunk so the document shape stays consistent.
        if g.chunks.len() == 1 {
            if let Some(c) = g.chunks[0].as_object() {
                for (k, v) in c {
                    if k != text_field && !is_stripped_chunk_key(k) {
                        doc_fields.entry(k.clone()).or_insert_with(|| v.clone());
                    }
                }
            }
        }

        let texts: Vec<String> = g
            .chunks
            .iter()
            .filter_map(|c| c.get(text_field).and_then(|v| v.as_str()))
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();

        if texts.is_empty() {
            // Chunks present but none carried body text under the resolved field.
            if !g.chunks.is_empty() {
                dropped_empty += 1;
            }
            continue;
        }

        staged.push(Staged {
            doc_id: g.doc_id.clone(),
            relevance: g.best_score,
            doc_fields,
            texts,
            emitted: 0,
        });
    }

    // Pass 1 — breadth: include each document (relevance order) with its first
    // passage if it fits. The very first document is always included so a
    // non-empty corpus never yields an empty result, even if its single passage
    // exceeds the budget.
    let mut included: Vec<usize> = Vec::with_capacity(staged.len());
    for (i, s) in staged.iter_mut().enumerate() {
        let first_len = s.texts[0].len();
        let nothing_yet = included.is_empty();
        if !nothing_yet && used_chars + first_len > DEFAULT_MAX_CONTEXT_CHARS {
            dropped_due_to_budget += 1;
            continue;
        }
        used_chars += first_len;
        s.emitted = 1;
        included.push(i);
    }

    // Pass 2 — depth: spend the remaining budget on further passages of the
    // already-included documents, in relevance order. A passage that doesn't fit
    // is dropped (`trimmed`); we move on so a later, smaller passage can still fit.
    for &i in &included {
        let s = &mut staged[i];
        while s.emitted < s.texts.len() {
            let len = s.texts[s.emitted].len();
            if used_chars + len > DEFAULT_MAX_CONTEXT_CHARS {
                trimmed = true;
                break;
            }
            used_chars += len;
            s.emitted += 1;
        }
    }
    if dropped_due_to_budget > 0 {
        trimmed = true;
    }

    // Build the document objects (relevance order preserved).
    let mut documents: Vec<Value> = Vec::with_capacity(included.len());
    for &i in &included {
        let s = &staged[i];
        let mut doc = Map::new();
        doc.insert("doc_id".to_string(), json!(s.doc_id));
        for (k, v) in &s.doc_fields {
            doc.insert(k.clone(), v.clone());
        }
        doc.insert("relevance".to_string(), json!(s.relevance));
        let passages: Vec<Value> = s.texts[..s.emitted]
            .iter()
            .map(|t| json!({ "text": t }))
            .collect();
        doc.insert("passages".to_string(), json!(passages));
        documents.push(Value::Object(doc));
    }

    let mut out = Map::new();
    // Stage A: no calibration → honest "unknown" (redesign §9 four-state contract).
    out.insert("verdict".to_string(), json!("unknown"));

    if format == "context_block" {
        out.insert(
            "context".to_string(),
            json!(render_context_block(&documents)),
        );
    } else {
        out.insert("documents".to_string(), json!(documents));
    }
    out.insert("count".to_string(), json!(documents.len()));
    out.insert("trimmed".to_string(), json!(trimmed));
    if dropped_empty > 0 {
        // Surfaced anomaly (P6): qualified documents whose body text was not under
        // the resolved text field — silently shrinking the result set is forbidden.
        out.insert("dropped_empty_documents".to_string(), json!(dropped_empty));
    }
    if dropped_due_to_budget > 0 {
        // Surfaced (P6): matched documents dropped because the context budget was
        // already full — the caller sees that more matches exist beyond `count`.
        out.insert(
            "dropped_due_to_budget".to_string(),
            json!(dropped_due_to_budget),
        );
    }

    if debug {
        out.insert(
            "diagnostics".to_string(),
            json!({
                "stage": "A",
                "fusion": "rrf (existing)",
                "calibration": "off",
                "hyde": "off",
                "total_chunks": total_chunks,
                "qualified_doc_ids": qualified_doc_count,
            }),
        );
    }

    Ok(Value::Object(out))
}

/// Render a citation-marked plain-text block ready to paste into a prompt.
fn render_context_block(documents: &[Value]) -> String {
    let mut s = String::new();
    for doc in documents {
        let Some(o) = doc.as_object() else { continue };
        let doc_id = o.get("doc_id").and_then(|v| v.as_str()).unwrap_or("?");
        let title = o.get("title").and_then(|v| v.as_str()).unwrap_or("");
        if title.is_empty() {
            s.push_str(&format!("[{}]\n", doc_id));
        } else {
            s.push_str(&format!("[{}] {}\n", doc_id, title));
        }
        if let Some(passages) = o.get("passages").and_then(|v| v.as_array()) {
            for p in passages {
                if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                    s.push_str(t);
                    s.push_str("\n\n");
                }
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::hybrid::DocGroup;

    const F: &str = "content_text";

    /// A group with `passage_lens.len()` chunks, each carrying a `content_text`
    /// body of the given length.
    fn group(doc_id: &str, score: f64, passage_lens: &[usize]) -> DocGroup {
        let chunks: Vec<Value> = passage_lens
            .iter()
            .enumerate()
            .map(|(i, &n)| json!({"doc_id": doc_id, F: "x".repeat(n), "chunk_index": i}))
            .collect();
        DocGroup {
            doc_id: doc_id.to_string(),
            best_score: score,
            lifted: Map::new(),
            chunks,
        }
    }

    fn docs(out: &Value) -> &Vec<Value> {
        out.get("documents").unwrap().as_array().unwrap()
    }

    /// The breadth-first regression guard: a single large document must not starve
    /// the rest. Depth-first allocation (the bug) would return only ~1 document
    /// here; breadth-first keeps many. A mutation back to depth-first fails this.
    #[test]
    fn breadth_first_keeps_many_documents() {
        // Doc A: 10 large passages (~1000 each) = 10_000 chars (depth-first eats
        // the 12_000 budget on A alone). Then 60 small one-passage docs.
        let mut groups = vec![group("A", 1.0, &[1000; 10])];
        for i in 0..60 {
            groups.push(group(&format!("d{i}"), 0.9 - i as f64 * 0.001, &[300]));
        }
        let out = assemble(&groups, 1000, None, F, "structured", false).unwrap();

        let count = out.get("count").unwrap().as_u64().unwrap();
        assert!(
            count >= 30,
            "breadth-first must keep many docs, got {count}"
        );
        // Every returned document carries at least its top passage.
        for d in docs(&out) {
            assert!(!d["passages"].as_array().unwrap().is_empty());
        }
        // The tail that did not fit is surfaced, not silently dropped (P6).
        assert!(
            out.get("dropped_due_to_budget")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                > 0,
            "documents dropped for budget must be surfaced"
        );
    }

    /// The total emitted passage text stays within the budget (except the
    /// never-empty first-passage guarantee, not exercised here).
    #[test]
    fn total_passage_chars_within_budget() {
        let mut groups = Vec::new();
        for i in 0..100 {
            groups.push(group(&format!("d{i}"), 1.0 - i as f64 * 0.001, &[500, 500]));
        }
        let out = assemble(&groups, 200, None, F, "structured", false).unwrap();
        let total: usize = docs(&out)
            .iter()
            .flat_map(|d| d["passages"].as_array().unwrap())
            .map(|p| p["text"].as_str().unwrap().len())
            .sum();
        assert!(
            total <= DEFAULT_MAX_CONTEXT_CHARS,
            "total {total} exceeds budget"
        );
    }

    /// Never-empty: the first document is included even if its single passage
    /// exceeds the whole budget.
    #[test]
    fn never_empty_for_oversized_first_doc() {
        let groups = vec![group("big", 1.0, &[DEFAULT_MAX_CONTEXT_CHARS + 5000])];
        let out = assemble(&groups, 1, None, F, "structured", false).unwrap();
        assert_eq!(out.get("count").unwrap().as_u64().unwrap(), 1);
        assert_eq!(docs(&out)[0]["passages"].as_array().unwrap().len(), 1);
    }

    /// A group whose chunks carry no body text under `text_field` is counted as
    /// `dropped_empty_documents`, distinct from a budget drop.
    #[test]
    fn body_less_group_is_dropped_empty() {
        let g = DocGroup {
            doc_id: "x".into(),
            best_score: 1.0,
            lifted: Map::new(),
            chunks: vec![json!({"doc_id": "x", "other": "y"})],
        };
        let out = assemble(&[g], 1, None, F, "structured", false).unwrap();
        assert_eq!(out.get("count").unwrap().as_u64().unwrap(), 0);
        assert_eq!(
            out.get("dropped_empty_documents").and_then(|v| v.as_u64()),
            Some(1)
        );
        assert!(out.get("dropped_due_to_budget").is_none());
    }

    /// Engine internals are stripped, doc-level fields lifted, verdict honest.
    #[test]
    fn strips_internals_and_marks_unknown() {
        let g = DocGroup {
            doc_id: "a".into(),
            best_score: 0.5,
            lifted: Map::new(),
            chunks: vec![json!({
                "doc_id": "a", F: "hello", "embedding": [0.1],
                "chunk_index": 2, "_text_score": 3.0, "title": "T"
            })],
        };
        let out = assemble(&[g], 5, None, F, "structured", false).unwrap();
        assert_eq!(out.get("verdict").unwrap(), &json!("unknown"));
        let obj = docs(&out)[0].as_object().unwrap();
        assert!(!obj.contains_key("embedding"));
        assert!(!obj.contains_key("chunk_index"));
        assert!(!obj.keys().any(|k| k.starts_with('_')));
        assert_eq!(obj.get("title").unwrap(), &json!("T"));
        assert_eq!(obj["passages"][0]["text"], json!("hello"));
    }

    /// `context_block` format renders citation-marked text with the doc_id.
    #[test]
    fn context_block_contains_doc_id() {
        let out = assemble(
            &[group("alpha", 1.0, &[100])],
            5,
            None,
            F,
            "context_block",
            false,
        )
        .unwrap();
        assert!(out
            .get("context")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("alpha"));
    }
}
