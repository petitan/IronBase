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
    let mut documents: Vec<Value> = Vec::with_capacity(groups.len());
    let mut used_chars: usize = 0;
    let mut trimmed = false;
    // Groups that had chunks but yielded no usable passage text (body under a
    // different field) — surfaced in the response (P6), never silently dropped.
    let mut dropped_empty: usize = 0;

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

        // Passages: the text body of each chunk (no internals). Hard char budget;
        // a passage that would exceed it is dropped and `trimmed` surfaced (P6) —
        // EXCEPT at least one passage is always included in the whole response so a
        // non-empty corpus never yields an empty result.
        let mut passages: Vec<Value> = Vec::with_capacity(g.chunks.len());
        for c in &g.chunks {
            let text = c.get(text_field).and_then(|v| v.as_str()).unwrap_or("");
            if text.is_empty() {
                continue;
            }
            let nothing_yet = documents.is_empty() && passages.is_empty();
            if !nothing_yet && used_chars + text.len() > DEFAULT_MAX_CONTEXT_CHARS {
                trimmed = true;
                break;
            }
            used_chars += text.len();
            passages.push(json!({ "text": text }));
        }

        if passages.is_empty() {
            // Distinguish a budget trim (chunk had text but didn't fit) from an
            // anomaly (chunks present but none carried body text under text_field).
            let any_text = g.chunks.iter().any(|c| {
                c.get(text_field)
                    .and_then(|v| v.as_str())
                    .map(|s| !s.is_empty())
                    .unwrap_or(false)
            });
            if any_text {
                trimmed = true;
            } else if !g.chunks.is_empty() {
                dropped_empty += 1;
            }
            continue;
        }

        let mut doc = Map::new();
        doc.insert("doc_id".to_string(), json!(g.doc_id));
        for (k, v) in doc_fields {
            doc.insert(k, v);
        }
        doc.insert("relevance".to_string(), json!(g.best_score));
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
