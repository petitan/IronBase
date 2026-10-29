//! Adjacent chunk merge.

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

use super::FusedResult;

/// Merge adjacent chunks from the same source document into single results.
///
/// RAG chunking with overlap (~100 chars) can cause the same text to appear in
/// consecutive chunks. When both match, they waste top-K slots with redundant
/// content. This post-processing step merges runs of consecutive chunk_index
/// values (same doc_id) into a single result with concatenated text (overlap removed).
///
/// Placement: after reranking, before MMR. This ensures merged results carry the
/// best score from the run, and MMR sees fewer near-duplicates.
///
/// Returns the number of results removed by merging.
pub fn merge_adjacent_chunks(results: &mut Vec<FusedResult>, text_field: &str) -> usize {
    merge_adjacent_runs(
        results,
        text_field,
        |r| &r.doc,
        |r| &mut r.doc,
        |_, r| r.final_score,
        None,
    )
}

/// [`merge_adjacent_chunks`] for one document group's chunks
/// (`build_doc_groups`), which are already best-first: each run keeps its
/// earliest chunk. Grouped mode never merged, so overlapping neighbours
/// reached the `search` response twice (hybrid review H1).
///
/// A merged passage holds at most [`MAX_MERGED_PASSAGE_CHARS`] around its best
/// chunk; the rest of a longer run stays in further passages. Unbounded, the
/// consecutive matching chunks of a table merged into one passage of tens of
/// thousands of characters, which the `search` context budget could not take,
/// and the whole document was left out (docs/SEARCH_CONTEXT_BUDGET_BUG.md).
/// The best chunk's byte offset in the merged text is kept under
/// [`MERGE_BEST_OFFSET_KEY`] so a shortened passage can keep it.
pub fn merge_adjacent_values(chunks: &mut Vec<Value>, text_field: &str) -> usize {
    merge_adjacent_runs(
        chunks,
        text_field,
        |c| c,
        |c| c,
        |i, _| -(i as f64),
        Some(MAX_MERGED_PASSAGE_CHARS),
    )
}

/// The largest merged passage (bytes) of a grouped result: a quarter of the
/// `search` context budget, so one document's run cannot take it over
pub const MAX_MERGED_PASSAGE_CHARS: usize = 3_000;

/// Internal key (stripped from every response, never lifted) holding the byte
/// offset of a merged passage's best chunk
pub const MERGE_BEST_OFFSET_KEY: &str = "_merge_best_offset";

/// Shared merge: `doc` reads an item's chunk document, `score` ranks an item
/// (by its position and value) to pick the one that carries the run.
/// `max_chars`: merge at most this many bytes of a run, growing from its best
/// chunk; the chunks left out form runs of their own.
fn merge_adjacent_runs<T>(
    results: &mut Vec<T>,
    text_field: &str,
    doc: impl Fn(&T) -> &Value,
    doc_mut: impl Fn(&mut T) -> &mut Value,
    score: impl Fn(usize, &T) -> f64,
    max_chars: Option<usize>,
) -> usize {
    if results.len() < 2 {
        return 0;
    }

    // Collect (doc_id, chunk_index, results_index) for RAG chunks only
    let mut chunk_info: Vec<(String, u64, usize)> = Vec::new();
    for (i, item) in results.iter().enumerate() {
        let doc_id = doc(item).get("doc_id").and_then(|v| v.as_str());
        let chunk_index = doc(item).get("chunk_index").and_then(|v| v.as_u64());
        if let (Some(did), Some(ci)) = (doc_id, chunk_index) {
            chunk_info.push((did.to_string(), ci, i));
        }
    }

    if chunk_info.is_empty() {
        return 0;
    }

    // Group by doc_id
    let mut groups: HashMap<String, Vec<(u64, usize)>> = HashMap::with_capacity(chunk_info.len());
    for (doc_id, ci, idx) in chunk_info {
        groups.entry(doc_id).or_default().push((ci, idx));
    }

    // For each group, sort by chunk_index and find consecutive runs
    let mut indices_to_remove: HashSet<usize> = HashSet::new();

    for (_doc_id, mut entries) in groups {
        if entries.len() < 2 {
            continue;
        }
        entries.sort_by_key(|(ci, _)| *ci);

        // Detect consecutive runs; a run longer than `max_chars` is merged
        // around its best chunk, and the parts left out are runs of their own
        let mut pending: Vec<(usize, usize)> = Vec::new();
        let mut run_start = 0;
        while run_start < entries.len() {
            let mut run_end = run_start;
            while run_end + 1 < entries.len() && entries[run_end + 1].0 == entries[run_end].0 + 1 {
                run_end += 1;
            }
            pending.push((run_start, run_end));
            run_start = run_end + 1;
        }

        while let Some((range_start, range_end)) = pending.pop() {
            if range_end > range_start {
                let range = &entries[range_start..=range_end];

                // Find the entry with the best final_score in this run
                let best_pos = (0..range.len())
                    .max_by(|&a, &b| {
                        score(range[a].1, &results[range[a].1])
                            .partial_cmp(&score(range[b].1, &results[range[b].1]))
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .unwrap();
                let best_idx_in_run = range[best_pos].1;

                // The merged window: the best chunk, then its neighbours (the
                // nearer side first) while the window stays within `max_chars`
                let (mut lo, mut hi) = (best_pos, best_pos);
                if let Some(cap) = max_chars {
                    let len = |pos: usize| {
                        doc(&results[range[pos].1])
                            .get(text_field)
                            .and_then(|v| v.as_str())
                            .map_or(0, str::len)
                    };
                    let mut total = len(best_pos);
                    loop {
                        let left = (lo > 0).then(|| lo - 1).filter(|&p| total + len(p) <= cap);
                        let right = (hi + 1 < range.len())
                            .then_some(hi + 1)
                            .filter(|&p| total + len(p) <= cap);
                        match (left, right) {
                            (Some(l), Some(r)) if best_pos - l <= r - best_pos => {
                                total += len(l);
                                lo = l;
                            }
                            (_, Some(r)) => {
                                total += len(r);
                                hi = r;
                            }
                            (Some(l), None) => {
                                total += len(l);
                                lo = l;
                            }
                            (None, None) => break,
                        }
                    }
                    if lo > 0 {
                        pending.push((range_start, range_start + lo - 1));
                    }
                    if hi + 1 < range.len() {
                        pending.push((range_start + hi + 1, range_end));
                    }
                } else {
                    lo = 0;
                    hi = range.len() - 1;
                }
                if hi == lo {
                    continue; // nothing to merge with within the cap
                }
                let run = &range[lo..=hi];
                let mut best_offset: Option<usize> = None;

                // Build merged text from all chunks in the run (overlap removed)
                let mut merged_text = String::new();
                let mut min_start_char: Option<u64> = None;
                let mut max_end_char: Option<u64> = None;

                for (i, &(_, res_idx)) in run.iter().enumerate() {
                    let chunk = doc(&results[res_idx]);
                    let raw_text = chunk.get(text_field).and_then(|v| v.as_str()).unwrap_or("");
                    // Table header propagation (#63) prepends the table's header to each
                    // continuation chunk; when merging, that header would repeat. Strip it
                    // from chunks after the first, using the EXACT header recorded at chunk
                    // time (the `table_header` field) — no re-derivation, so it matches
                    // byte-for-byte (CRLF/whitespace safe). Stripping first also restores
                    // the original data text so the overlap offsets below line up.
                    let chunk_text = if i > 0 {
                        match chunk.get("table_header").and_then(|v| v.as_str()) {
                            Some(h) => raw_text
                                .strip_prefix(format!("{}\n", h).as_str())
                                .unwrap_or(raw_text),
                            None => raw_text,
                        }
                    } else {
                        raw_text
                    };

                    if res_idx == best_idx_in_run {
                        best_offset = Some(merged_text.len());
                    }
                    if i == 0 {
                        merged_text.push_str(chunk_text);
                    } else {
                        // Calculate overlap from start_char/end_char if available
                        let prev_end = doc(&results[run[i - 1].1])
                            .get("end_char")
                            .and_then(|v| v.as_u64());
                        let curr_start = chunk.get("start_char").and_then(|v| v.as_u64());

                        let overlap_chars = match (prev_end, curr_start) {
                            (Some(pe), Some(cs)) if pe > cs => (pe - cs) as usize,
                            _ => 0,
                        };

                        // Skip overlap characters from the beginning of this chunk's text
                        let text_to_append =
                            if overlap_chars > 0 && overlap_chars < chunk_text.chars().count() {
                                // Find byte offset for char boundary
                                let byte_offset = chunk_text
                                    .char_indices()
                                    .nth(overlap_chars)
                                    .map(|(idx, _)| idx)
                                    .unwrap_or(chunk_text.len());
                                &chunk_text[byte_offset..]
                            } else {
                                chunk_text
                            };

                        merged_text.push_str(text_to_append);
                    }

                    // Track start_char/end_char range
                    if let Some(sc) = chunk.get("start_char").and_then(|v| v.as_u64()) {
                        min_start_char = Some(min_start_char.map_or(sc, |m: u64| m.min(sc)));
                    }
                    if let Some(ec) = chunk.get("end_char").and_then(|v| v.as_u64()) {
                        max_end_char = Some(max_end_char.map_or(ec, |m: u64| m.max(ec)));
                    }
                }

                // Update the best-scored result's doc with merged content
                if let Value::Object(obj) = doc_mut(&mut results[best_idx_in_run]) {
                    obj.insert(text_field.to_string(), Value::String(merged_text));
                    obj.insert("chunk_index".to_string(), json!(run[0].0));
                    if let Some(sc) = min_start_char {
                        obj.insert("start_char".to_string(), json!(sc));
                    }
                    if let Some(ec) = max_end_char {
                        obj.insert("end_char".to_string(), json!(ec));
                    }
                    obj.insert("chunk_merged".to_string(), json!(true));
                    obj.insert("chunks_in_merge".to_string(), json!(run.len()));
                    if max_chars.is_some() {
                        obj.insert(MERGE_BEST_OFFSET_KEY.to_string(), json!(best_offset));
                    }
                }

                // Mark all other entries in this run for removal
                for &(_, res_idx) in run {
                    if res_idx != best_idx_in_run {
                        indices_to_remove.insert(res_idx);
                    }
                }
            }
        }
    }

    let removed = indices_to_remove.len();
    if removed > 0 {
        let mut idx = 0;
        results.retain(|_| {
            let keep = !indices_to_remove.contains(&idx);
            idx += 1;
            keep
        });
    }

    removed
}
