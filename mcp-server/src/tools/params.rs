//! Typed parameter structs for MCP tools
//!
//! These structs provide compile-time type safety and automatic validation
//! via serde deserialization, replacing manual Value parsing.
//!
//! The structs themselves live in `ironbase_mcp_types::params` (shared with
//! clients) and are re-exported here; this module keeps what needs the server's
//! error type: `ParseParams` and the search-mode weight resolution.

use serde::Deserialize;
use serde_json::Value;

pub use ironbase_mcp_types::params::*;

// ============================================================================
// Search Mode Presets
// ============================================================================

/// Named search mode presets for LLM-friendly weight configuration
///
/// Instead of tuning numerical weights (which LLMs are bad at),
/// the LLM picks a semantic mode name and the server resolves it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    /// Equal weight for vector and fulltext (0.5 / 0.5)
    Balanced,
    /// Favor vector similarity (0.8 / 0.2) — conceptual queries
    Semantic,
    /// Favor fulltext matching (0.2 / 0.8) — specific term queries
    Keyword,
}

impl SearchMode {
    /// Parse search mode from string
    pub fn parse_mode(s: &str) -> Option<Self> {
        match s {
            "balanced" => Some(Self::Balanced),
            "semantic" => Some(Self::Semantic),
            "keyword" => Some(Self::Keyword),
            _ => None,
        }
    }

    /// Get (vector_weight, fulltext_weight) for this mode
    pub fn weights(&self) -> (f64, f64) {
        match self {
            Self::Balanced => (0.5, 0.5),
            Self::Semantic => (0.8, 0.2),
            Self::Keyword => (0.2, 0.8),
        }
    }
}

/// Resolve actual weights from search_mode + explicit overrides
///
/// Priority: explicit weights > search_mode preset > balanced default
pub fn resolve_weights(
    search_mode: Option<&str>,
    explicit_vector_weight: Option<f64>,
    explicit_fulltext_weight: Option<f64>,
) -> Result<(f64, f64)> {
    // Explicit weights take priority (either one triggers explicit mode)
    if explicit_vector_weight.is_some() || explicit_fulltext_weight.is_some() {
        let vw = explicit_vector_weight.unwrap_or(0.5);
        let fw = explicit_fulltext_weight.unwrap_or(0.5);
        if vw.abs() < f64::EPSILON && fw.abs() < f64::EPSILON {
            return Err(McpError::invalid_params(
                "At least one of vector_weight or fulltext_weight must be positive.",
            ));
        }
        // Normalize so weights sum to 1.0
        let total = vw + fw;
        return Ok((vw / total, fw / total));
    }

    // Search mode presets
    if let Some(mode_str) = search_mode {
        let mode = SearchMode::parse_mode(mode_str).ok_or_else(|| {
            McpError::invalid_params(format!(
                "Invalid search_mode '{}'. Use: 'balanced', 'semantic', or 'keyword'",
                mode_str
            ))
        })?;
        return Ok(mode.weights());
    }

    // Default: balanced
    Ok((0.5, 0.5))
}

// ============================================================================
// Helper trait for param parsing
// ============================================================================

use crate::error::{McpError, Result};

/// Helper trait for parsing params from Value
pub trait ParseParams: Sized {
    fn parse(params: Value) -> Result<Self>;
}

impl<T: for<'de> Deserialize<'de>> ParseParams for T {
    fn parse(params: Value) -> Result<Self> {
        serde_json::from_value(params).map_err(|e| McpError::invalid_params(e.to_string()))
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_find_params_full() {
        let params = json!({
            "collection": "users",
            "query": {"age": {"$gt": 18}},
            "projection": {"name": 1},
            "sort": {"age": -1},
            "limit": 10,
            "skip": 5,
            "include_total": true
        });
        let p: FindParams = FindParams::parse(params).unwrap();
        assert_eq!(p.collection, "users");
        assert_eq!(p.limit, Some(10));
        assert!(p.include_total);
    }

    #[test]
    fn test_find_params_minimal() {
        let params = json!({"collection": "users"});
        let p: FindParams = FindParams::parse(params).unwrap();
        assert_eq!(p.collection, "users");
        assert_eq!(p.query, json!({}));
        assert_eq!(p.limit, None);
        assert!(!p.include_total);
    }

    #[test]
    fn test_find_params_missing_collection() {
        let params = json!({"query": {}});
        let result = FindParams::parse(params);
        assert!(result.is_err());
    }

    #[test]
    fn test_find_params_filter_alias() {
        // Issue #26: "filter" should work as alias for "query"
        let params = json!({
            "collection": "emails",
            "filter": {"from.email": "test@example.com"},
            "limit": 10
        });
        let p: FindParams = FindParams::parse(params).unwrap();
        assert_eq!(p.collection, "emails");
        assert_eq!(p.query["from.email"], "test@example.com");
        assert_eq!(p.limit, Some(10));
    }

    #[test]
    fn test_find_one_params_filter_alias() {
        let params = json!({
            "collection": "users",
            "filter": {"_id": 123}
        });
        let p: FindOneParams = FindOneParams::parse(params).unwrap();
        assert_eq!(p.query["_id"], 123);
    }

    #[test]
    fn test_count_params_filter_alias() {
        let params = json!({
            "collection": "orders",
            "filter": {"status": "pending"}
        });
        let p: CountParams = CountParams::parse(params).unwrap();
        assert_eq!(p.query["status"], "pending");
    }

    #[test]
    fn test_insert_one_params() {
        let params = json!({
            "collection": "users",
            "document": {"name": "Alice", "age": 30}
        });
        let p: InsertOneParams = InsertOneParams::parse(params).unwrap();
        assert_eq!(p.collection, "users");
        assert_eq!(p.document["name"], "Alice");
    }

    #[test]
    fn test_update_one_params() {
        let params = json!({
            "collection": "users",
            "filter": {"name": "Alice"},
            "update": {"$set": {"age": 31}},
            "upsert": true
        });
        let p: UpdateOneParams = UpdateOneParams::parse(params).unwrap();
        assert!(p.upsert);
    }

    #[test]
    fn test_update_many_params() {
        let params = json!({
            "collection": "users",
            "filter": {"status": "inactive"},
            "update": {"$set": {"archived": true}}
        });
        let p: UpdateManyParams = UpdateManyParams::parse(params).unwrap();
        assert_eq!(p.collection, "users");
        assert_eq!(p.filter["status"], "inactive");
    }

    #[test]
    fn test_update_many_no_upsert_field() {
        // Verify UpdateManyParams doesn't have upsert field
        let params = json!({
            "collection": "users",
            "filter": {},
            "update": {"$set": {"x": 1}},
            "upsert": true  // This field should be ignored
        });
        let p: UpdateManyParams = UpdateManyParams::parse(params).unwrap();
        // If we could access upsert, the test would fail at compile time
        assert_eq!(p.collection, "users");
    }

    #[test]
    fn test_fuzzy_index_defaults() {
        let params = json!({
            "collection": "users",
            "field": "name"
        });
        let p: FuzzyIndexParams = FuzzyIndexParams::parse(params).unwrap();
        assert_eq!(p.algorithm, "jaro_winkler");
        assert_eq!(p.threshold, 0.8);
    }

    #[test]
    fn test_script_exec_params() {
        let params = json!({
            "code": "return 42;",
            "params": {"x": 10},
            "max_operations": 5000
        });
        let p: ScriptExecParams = ScriptExecParams::parse(params).unwrap();
        assert_eq!(p.code, "return 42;");
        assert_eq!(p.params.as_ref().and_then(|v| v.get("x")), Some(&json!(10)));
        assert_eq!(p.max_operations, Some(5000));
    }

    #[test]
    fn test_fulltext_search_params_defaults() {
        let params = json!({
            "collection": "articles",
            "query": "rust programming"
        });
        let p: FulltextSearchParams = FulltextSearchParams::parse(params).unwrap();
        assert_eq!(p.collection, "articles");
        assert_eq!(p.field, "content"); // default
        assert!(p.fields.is_none());
        assert_eq!(p.query, "rust programming");
        assert!(p.limit.is_none());
        assert!(p.skip.is_none());
        assert!(p.min_score.is_none());
        assert!(p.mode.is_none());
        assert!(p.filter.is_none());
        assert!(!p.highlight);
    }

    #[test]
    fn test_fulltext_search_params_single_field() {
        let params = json!({
            "collection": "articles",
            "field": "title",
            "query": "rust"
        });
        let p: FulltextSearchParams = FulltextSearchParams::parse(params).unwrap();
        assert_eq!(p.field, "title");
        assert!(p.fields.is_none());
    }

    #[test]
    fn test_fulltext_search_params_multi_fields() {
        let params = json!({
            "collection": "articles",
            "fields": ["title", "body"],
            "query": "rust"
        });
        let p: FulltextSearchParams = FulltextSearchParams::parse(params).unwrap();
        let fields = p.fields.unwrap();
        assert_eq!(fields, vec!["title", "body"]);
    }

    #[test]
    fn test_fulltext_search_params_fields_overrides_field() {
        // When both field and fields are provided, fields should be present
        let params = json!({
            "collection": "articles",
            "field": "content",
            "fields": ["title", "body"],
            "query": "rust"
        });
        let p: FulltextSearchParams = FulltextSearchParams::parse(params).unwrap();
        assert!(p.fields.is_some());
        assert_eq!(p.fields.unwrap(), vec!["title", "body"]);
    }

    #[test]
    fn test_fulltext_search_params_with_all_options() {
        let params = json!({
            "collection": "articles",
            "fields": ["title", "content"],
            "query": "rust python",
            "mode": "and",
            "limit": 5,
            "skip": 2,
            "min_score": 0.5,
            "filter": {"status": "published"},
            "highlight": true,
            "highlight_context": 200,
            "highlight_max_snippets": 5
        });
        let p: FulltextSearchParams = FulltextSearchParams::parse(params).unwrap();
        assert_eq!(p.fields.unwrap(), vec!["title", "content"]);
        assert_eq!(p.mode.as_deref(), Some("and"));
        assert_eq!(p.limit, Some(5));
        assert_eq!(p.skip, Some(2));
        assert_eq!(p.min_score, Some(0.5));
        assert!(p.filter.is_some());
        assert!(p.highlight);
        assert_eq!(p.highlight_context, Some(200));
        assert_eq!(p.highlight_max_snippets, Some(5));
    }

    #[test]
    fn test_fulltext_search_params_empty_fields_array() {
        let params = json!({
            "collection": "articles",
            "fields": [],
            "query": "rust"
        });
        let p: FulltextSearchParams = FulltextSearchParams::parse(params).unwrap();
        let fields = p.fields.unwrap();
        assert!(fields.is_empty());
    }

    // =========================================================================
    // Search Mode Preset Tests
    // =========================================================================

    #[test]
    fn test_search_mode_from_str() {
        assert_eq!(
            SearchMode::parse_mode("balanced"),
            Some(SearchMode::Balanced)
        );
        assert_eq!(
            SearchMode::parse_mode("semantic"),
            Some(SearchMode::Semantic)
        );
        assert_eq!(SearchMode::parse_mode("keyword"), Some(SearchMode::Keyword));
        assert_eq!(SearchMode::parse_mode("invalid"), None);
        assert_eq!(SearchMode::parse_mode(""), None);
    }

    #[test]
    fn test_search_mode_weights() {
        assert_eq!(SearchMode::Balanced.weights(), (0.5, 0.5));
        assert_eq!(SearchMode::Semantic.weights(), (0.8, 0.2));
        assert_eq!(SearchMode::Keyword.weights(), (0.2, 0.8));
    }

    #[test]
    fn test_resolve_weights_default() {
        // No mode, no explicit weights → balanced
        let (vw, fw) = resolve_weights(None, None, None).unwrap();
        assert_eq!(vw, 0.5);
        assert_eq!(fw, 0.5);
    }

    #[test]
    fn test_resolve_weights_search_mode_semantic() {
        let (vw, fw) = resolve_weights(Some("semantic"), None, None).unwrap();
        assert_eq!(vw, 0.8);
        assert_eq!(fw, 0.2);
    }

    #[test]
    fn test_resolve_weights_search_mode_keyword() {
        let (vw, fw) = resolve_weights(Some("keyword"), None, None).unwrap();
        assert_eq!(vw, 0.2);
        assert_eq!(fw, 0.8);
    }

    #[test]
    fn test_resolve_weights_explicit_overrides_mode() {
        // Explicit weights take priority over search_mode
        let (vw, fw) = resolve_weights(Some("semantic"), Some(0.3), Some(0.7)).unwrap();
        assert_eq!(vw, 0.3);
        assert_eq!(fw, 0.7);
    }

    #[test]
    fn test_resolve_weights_partial_explicit() {
        // Only one explicit weight → other defaults to 0.5, then normalized to sum=1.0
        let (vw, fw) = resolve_weights(None, Some(0.9), None).unwrap();
        let total = 0.9 + 0.5;
        assert!((vw - 0.9 / total).abs() < 1e-10);
        assert!((fw - 0.5 / total).abs() < 1e-10);
    }

    #[test]
    fn test_resolve_weights_partial_explicit_overrides_mode() {
        // One explicit weight triggers explicit mode (ignores search_mode), normalized
        let (vw, fw) = resolve_weights(Some("keyword"), None, Some(0.9)).unwrap();
        let total = 0.5 + 0.9;
        assert!((vw - 0.5 / total).abs() < 1e-10);
        assert!((fw - 0.9 / total).abs() < 1e-10);
    }

    #[test]
    fn test_resolve_weights_invalid_mode() {
        let result = resolve_weights(Some("turbo"), None, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_hybrid_params_with_search_mode() {
        let params = json!({
            "collection": "test",
            "vector": [0.1, 0.2],
            "query": "test",
            "search_mode": "semantic"
        });
        let p: HybridSearchParams = HybridSearchParams::parse(params).unwrap();
        assert_eq!(p.search_mode.as_deref(), Some("semantic"));
        assert!(p.vector_weight.is_none());
        assert!(p.fulltext_weight.is_none());
    }

    #[test]
    fn test_hybrid_params_auto_embed_mode() {
        // No vector → auto-embed mode
        let params = json!({
            "collection": "docs",
            "query": "semantic search test"
        });
        let p: HybridSearchParams = HybridSearchParams::parse(params).unwrap();
        assert_eq!(p.collection, "docs");
        assert_eq!(p.query, "semantic search test");
        assert!(p.vector.is_none()); // auto-embed mode
        assert!(p.provider.is_none()); // use collection config
    }

    #[test]
    fn test_hybrid_params_auto_embed_with_provider() {
        let params = json!({
            "collection": "docs",
            "query": "keresés",
            "provider": "ollama"
        });
        let p: HybridSearchParams = HybridSearchParams::parse(params).unwrap();
        assert!(p.vector.is_none());
        assert_eq!(p.provider.as_deref(), Some("ollama"));
    }

    #[test]
    fn test_hybrid_params_explicit_weights_with_mode() {
        // Explicit weights should be parsed even alongside search_mode
        let params = json!({
            "collection": "test",
            "vector": [0.1, 0.2],
            "query": "test",
            "search_mode": "semantic",
            "vector_weight": 0.6,
            "fulltext_weight": 0.4
        });
        let p: HybridSearchParams = HybridSearchParams::parse(params).unwrap();
        assert_eq!(p.search_mode.as_deref(), Some("semantic"));
        assert_eq!(p.vector_weight, Some(0.6));
        assert_eq!(p.fulltext_weight, Some(0.4));
    }
}
