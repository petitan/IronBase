//! The `mode` / `match_scope` parameters of the hybrid and fulltext search
//! tools; the document-level AND they select runs in the engine
//! (`CollectionCore::fulltext_document_scope`).
//!
//! The post-fusion steps (rerank, adjacent-chunk merge, MMR) live in the
//! engine: `ironbase_core::search::hybrid`.

use crate::adapter::IronBaseAdapter;
use crate::error::{McpError, Result};
use ironbase_core::QualificationOutcome;
use serde_json::Value;
use std::sync::Arc;

// ============================================================================
// Document qualification orchestration (shared between hybrid & fulltext)
// ============================================================================

/// Validate `match_scope` parameter value.
/// Returns error for invalid values; None and valid values pass through.
pub(crate) fn validate_match_scope(match_scope: Option<&str>) -> Result<()> {
    if let Some(scope) = match_scope {
        match scope {
            "document" | "chunk" => Ok(()),
            other => Err(McpError::invalid_params(format!(
                "Invalid match_scope '{}'. Use: 'document' or 'chunk'",
                other
            ))),
        }
    } else {
        Ok(())
    }
}

/// Resolve the lexical retrieval conjunction mode from the `mode` parameter.
///
/// Default (None) is DISJUNCTIVE (OR, returns `false`) — the industry-standard
/// hybrid/RRF design where relevance is ranked by BM25 score, not gated by a
/// binary all-terms requirement. Only an explicit `mode="and"` opts into the
/// conjunctive all-terms filter. Any other value (incl. "or") → OR.
pub(crate) fn resolve_and_mode(mode: Option<&str>) -> bool {
    mode == Some("and")
}

/// Apply document-level AND qualification if needed
/// (`CollectionCore::fulltext_document_scope`), from the tools' `mode` and
/// `match_scope` parameters.
///
/// `fields`: all text fields to qualify across (union of doc_ids).
/// For single-field, pass a slice with one element.
pub(crate) fn apply_document_qualification(
    adapter: &Arc<IronBaseAdapter>,
    collection: &str,
    fields: &[&str],
    query: &str,
    mode: Option<&str>,
    match_scope: Option<&str>,
    user_filter: Option<Value>,
) -> Result<QualificationOutcome> {
    validate_match_scope(match_scope)?;
    adapter.fulltext_document_scope(
        collection,
        fields,
        query,
        resolve_and_mode(mode),
        match_scope != Some("chunk"),
        user_filter,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_and_mode_default_is_disjunctive() {
        // Industry-standard default: None → OR (disjunctive). Only explicit "and" → AND.
        assert!(!resolve_and_mode(None)); // default = OR
        assert!(!resolve_and_mode(Some("or")));
        assert!(resolve_and_mode(Some("and"))); // opt-in precision
        assert!(!resolve_and_mode(Some("anything-else"))); // unknown → OR (safe default)
    }
}
