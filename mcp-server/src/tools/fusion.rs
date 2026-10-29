//! Document-level AND orchestration shared by the hybrid and fulltext search
//! tools (match_scope, filter merge, OR switch).
//!
//! The post-fusion steps (rerank, adjacent-chunk merge, MMR) live in the
//! engine: `ironbase_core::search::hybrid`.

use crate::adapter::IronBaseAdapter;
use crate::error::{McpError, Result};
use ironbase_core::DocumentQualification;
use serde_json::{json, Value};
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

/// Result of document qualification orchestration
#[derive(Debug)]
pub(crate) struct QualificationOutcome {
    /// Whether document-level scope is active
    pub is_doc_scope: bool,
    /// Effective AND mode (false if doc qualification switched to OR)
    pub effective_and_mode: bool,
    /// Effective filter (merged user filter + doc qualification filter)
    pub effective_filter: Option<Value>,
    /// Number of qualified documents (if doc scope active)
    pub qualified_doc_count: Option<usize>,
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

/// Apply document-level AND qualification if needed.
///
/// Shared orchestration logic used by both `hybrid_search` and `fulltext_search`.
/// Handles: match_scope validation, multi-field qualification (union), filter merge,
/// and OR mode switch.
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

    // Default = DISJUNCTIVE (OR), aligning with the industry-standard hybrid /
    // RRF design: the lexical lane retrieves disjunctively and relevance is decided
    // by the (graded) BM25 score + RRF + rerank — NOT by a binary all-terms gate.
    // A hard AND qualification before fusion drops strong partial (BM25 rank-1)
    // matches and is non-standard for RRF (it re-binarizes BM25's graded scoring).
    // AND is now OPT-IN (`mode="and"`) for callers that explicitly want all-terms
    // precision. BREAKING CHANGE vs the prior AND default (clients may need updating).
    let and_mode = resolve_and_mode(mode);
    let is_doc_scope = and_mode && match_scope != Some("chunk");

    if !is_doc_scope || fields.is_empty() {
        return Ok(QualificationOutcome {
            is_doc_scope,
            effective_and_mode: and_mode,
            effective_filter: user_filter,
            qualified_doc_count: None,
        });
    }

    // Skip qualification if the collection has no fulltext indexes (non-RAG collection)
    let ft_fields = adapter
        .get_fulltext_field_names(collection)
        .unwrap_or_default();
    if ft_fields.is_empty() {
        return Ok(QualificationOutcome {
            is_doc_scope: false,
            effective_and_mode: and_mode,
            effective_filter: user_filter,
            qualified_doc_count: None,
        });
    }

    // A document qualifies if ALL query tokens appear across its chunks in ANY
    // of the fields (union over the fields).
    let qualify_start = std::time::Instant::now();
    let qualification = adapter.fulltext_qualify_documents(collection, fields, query)?;
    let qualified_doc_count = match &qualification {
        DocumentQualification::NotRequired => None,
        DocumentQualification::Documents { ids, .. } => Some(ids.len()),
    };
    if let Some(count) = qualified_doc_count {
        tracing::info!(
            collection = collection,
            qualified = count,
            elapsed_ms = qualify_start.elapsed().as_millis() as u64,
            "document-level AND qualification"
        );
    }
    let doc_qualification_filter = qualification.to_filter();

    // If document-level AND is active, switch to OR mode (doc qualification
    // already ensures all tokens appear across the document's chunks) and merge filters.
    let (effective_and_mode, effective_filter) =
        if let Some(ref qual_filter) = doc_qualification_filter {
            let merged = match user_filter {
                Some(user_f) => Some(json!({"$and": [user_f, qual_filter]})),
                None => Some(qual_filter.clone()),
            };
            (false, merged) // OR mode — doc-level AND already satisfied
        } else {
            (and_mode, user_filter)
        };

    Ok(QualificationOutcome {
        is_doc_scope,
        effective_and_mode,
        effective_filter,
        qualified_doc_count,
    })
}

// ============================================================================
// Tests
// ============================================================================

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
