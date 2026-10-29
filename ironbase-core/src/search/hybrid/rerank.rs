//! Reranking of fused results by phrase match, keyword density, content
//! length and title match.

use std::collections::HashSet;

use super::FusedResult;

/// Strip punctuation for phrase matching (optimized: single allocation)
pub(super) fn strip_punctuation(s: &str) -> String {
    let filtered: String = s
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect();
    // Normalize whitespace in-place without extra Vec allocation
    let mut result = String::with_capacity(filtered.len());
    let mut prev_space = true; // Start true to skip leading spaces
    for c in filtered.chars() {
        if c.is_whitespace() {
            if !prev_space {
                result.push(' ');
                prev_space = true;
            }
        } else {
            result.push(c);
            prev_space = false;
        }
    }
    // Trim trailing space
    if result.ends_with(' ') {
        result.pop();
    }
    result
}

/// Rerank results by phrase match, keyword density, content length, and title match
///
/// Reranking boosts:
/// - Exact phrase boost: 1.5x if query found in content (punctuation ignored)
/// - Keyword density: 1.0-1.3x based on query word occurrence ratio
/// - Content length penalty: 0.8x for content < 50 chars
/// - Title match boost: up to 1.5x if query words appear in title field
pub fn rerank_results(
    results: &mut [FusedResult],
    query: &str,
    text_field: &str,
    title_field: Option<&str>,
) {
    // Build query word sets — use chars().count() for UTF-8 correctness
    let query_words: HashSet<String> = query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 3)
        .map(|s| s.to_string())
        .collect();

    // Query for exact phrase matching (punctuation stripped)
    let query_normalized = strip_punctuation(&query.to_lowercase());

    for item in results.iter_mut() {
        let mut boost = 1.0;

        let content = item
            .doc
            .get(text_field)
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let content_lower = content.to_lowercase();
        let content_normalized = strip_punctuation(&content_lower);

        // 1. Exact phrase boost (1.5x) - query found in content (punctuation ignored)
        // Use chars().count() for UTF-8 correctness
        if query_normalized.chars().count() > 10 && content_normalized.contains(&query_normalized) {
            boost *= 1.5;
        }

        // 2. Keyword density (1.0-1.3x)
        let content_words: Vec<&str> = content_lower
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .collect();

        if !content_words.is_empty() {
            #[allow(clippy::unnecessary_to_owned)]
            let matches = content_words
                .iter()
                .filter(|w| query_words.contains(&w.to_string()))
                .count();
            let density = matches as f64 / content_words.len() as f64;
            boost *= 1.0 + density.min(0.3); // Cap at 1.3x
        }

        // 3. Content length penalty (0.8x for short content)
        if content.len() < 50 {
            boost *= 0.8;
        }

        // 4. Title match boost (up to 1.5x) — query words in title
        if let Some(tf) = title_field {
            if let Some(title) = item.doc.get(tf).and_then(|v| v.as_str()) {
                let title_lower = title.to_lowercase();
                let title_matches = query_words
                    .iter()
                    .filter(|w| title_lower.contains(w.as_str()))
                    .count();
                if title_matches > 0 && !query_words.is_empty() {
                    let title_ratio = title_matches as f64 / query_words.len() as f64;
                    boost *= 1.0 + 0.5 * title_ratio; // 1.0–1.5x scale
                }
            }
        }

        item.rerank_boost = boost;
        item.final_score = item.rrf_score * boost;
    }

    // Re-sort by final_score (descending). Deterministic tie-break on the stable
    // chunk id (see the RRF sort in retrieve_and_fuse) so equal scores resolve
    // reproducibly run-to-run.
    results.sort_by(|a, b| {
        b.final_score
            .partial_cmp(&a.final_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
}
