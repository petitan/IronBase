//! Wire-format safety net for MCP tool arguments.
//!
//! Every typed params struct is parsed from (a) a minimal valid JSON, (b) a JSON
//! with every optional field set and (c) its important invalid / missing-field
//! case. The parsed value (`Debug`) or the exact error (`code` + `message`) is
//! compared with `tests/golden/params_wire.txt`, so any change in what the server
//! accepts — a renamed field, a changed default, a different error text — fails
//! here. The structs are reached through their public server paths
//! (`mcp_ironbase::tools::…`), which must keep resolving.
//!
//! Regenerate (only for an intended wire-format change):
//! `UPDATE_GOLDEN=1 cargo test --test params_wire_golden`

use mcp_ironbase::tools::auto_embed::{
    AutoEmbedDisableParams, AutoEmbedEnableParams, AutoEmbedStatusParams, ChunkingParams,
};
use mcp_ironbase::tools::embedding::{
    EmbedBatchParams, EmbedDocumentParams, EmbedListModelsParams, EmbedTextParams,
};
use mcp_ironbase::tools::jobs::{JobCancelParams, JobListParams, JobStatusParams};
use mcp_ironbase::tools::params::*;
use mcp_ironbase::tools::retrieval::SearchParams;
use mcp_ironbase::tools::vector::{VectorIndexCreateParams, VectorSearchParams};
use serde_json::{json, Value};
use std::fmt::{Debug, Write};

mod common;
use common::{create_test_adapter, dispatch_err};

const GOLDEN: &str = include_str!("golden/params_wire.txt");
const GOLDEN_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/params_wire.txt");

fn record<T: ParseParams + Debug>(out: &mut String, ty: &str, label: &str, input: Value) {
    writeln!(out, "## {ty} / {label}").unwrap();
    writeln!(out, "in:  {input}").unwrap();
    match T::parse(input) {
        Ok(p) => writeln!(out, "ok:  {p:?}").unwrap(),
        Err(e) => writeln!(out, "err: {:?} {}", e.code, e.message).unwrap(),
    }
    out.push('\n');
}

macro_rules! golden {
    ($out:ident, $ty:ty, $($label:literal => $json:tt),+ $(,)?) => {
        $( record::<$ty>(&mut $out, stringify!($ty), $label, json!($json)); )+
    };
}

fn render() -> String {
    let mut out = String::new();

    // ---- CRUD ----
    golden!(out, FindParams,
        "minimal" => {"collection": "users"},
        "full" => {"collection": "users", "query": {"age": {"$gt": 18}}, "projection": {"name": 1},
                   "sort": {"age": -1}, "limit": 10, "skip": 5, "include_total": true},
        "filter alias" => {"collection": "users", "filter": {"a": 1}},
        "query and filter" => {"collection": "users", "query": {"a": 1}, "filter": {"b": 2}},
        "null optionals" => {"collection": "users", "projection": null, "limit": null},
        "unknown field ignored" => {"collection": "users", "bogus": 1},
        "missing collection" => {"query": {}},
        "wrong type" => {"collection": 5},
        "negative limit" => {"collection": "users", "limit": -1},
        "not an object" => "users",
        "null" => null,
        "array" => ["users"],
    );
    golden!(out, FindOneParams,
        "minimal" => {"collection": "users"},
        "full" => {"collection": "users", "query": {"_id": 1}, "projection": {"name": 1}},
        "filter alias" => {"collection": "users", "filter": {"_id": 1}},
        "missing collection" => {},
    );
    golden!(out, InsertOneParams,
        "minimal" => {"collection": "users", "document": {"name": "Alice"}},
        "non-object document" => {"collection": "users", "document": 5},
        "missing document" => {"collection": "users"},
        "missing collection" => {"document": {}},
    );
    golden!(out, InsertManyParams,
        "minimal" => {"collection": "users", "documents": []},
        "full" => {"collection": "users", "documents": [{"a": 1}, {"a": 2}]},
        "missing documents" => {"collection": "users"},
        "documents not array" => {"collection": "users", "documents": {"a": 1}},
    );
    golden!(out, UpdateOneParams,
        "minimal" => {"collection": "users", "filter": {"a": 1}, "update": {"$set": {"b": 2}}},
        "full" => {"collection": "users", "filter": {"a": 1}, "update": {"$set": {"b": 2}}, "upsert": true},
        "query alias" => {"collection": "users", "query": {"a": 1}, "update": {}},
        "missing filter" => {"collection": "users", "update": {}},
        "missing update" => {"collection": "users", "filter": {}},
        "upsert wrong type" => {"collection": "users", "filter": {}, "update": {}, "upsert": "yes"},
    );
    golden!(out, UpdateManyParams,
        "minimal" => {"collection": "users", "filter": {"a": 1}, "update": {"$set": {"b": 2}}},
        "query alias, upsert ignored" => {"collection": "users", "query": {}, "update": {}, "upsert": true},
        "missing filter" => {"collection": "users", "update": {}},
    );
    golden!(out, DeleteParams,
        "minimal" => {"collection": "users", "filter": {"a": 1}},
        "query alias" => {"collection": "users", "query": {"a": 1}},
        "missing filter" => {"collection": "users"},
    );
    golden!(out, CountParams,
        "minimal" => {"collection": "users"},
        "full" => {"collection": "users", "query": {"a": 1}},
        "filter alias" => {"collection": "users", "filter": {"a": 1}},
        "missing collection" => {},
    );
    golden!(out, DistinctParams,
        "minimal" => {"collection": "users", "field": "city"},
        "full" => {"collection": "users", "field": "city", "query": {"a": 1}, "limit": 100},
        "filter alias" => {"collection": "users", "field": "city", "filter": {"a": 1}},
        "missing field" => {"collection": "users"},
    );
    golden!(out, AggregateParams,
        "minimal" => {"collection": "users", "pipeline": []},
        "full" => {"collection": "users", "pipeline": [{"$match": {"a": 1}}, {"$limit": 5}]},
        "missing pipeline" => {"collection": "users"},
        "pipeline not array" => {"collection": "users", "pipeline": {"$match": {}}},
    );

    // ---- Index ----
    golden!(out, IndexCreateParams,
        "minimal" => {"collection": "users"},
        "field" => {"collection": "users", "field": "age"},
        "full" => {"collection": "users", "field": "age", "fields": ["a", "b"], "unique": true, "sparse": true},
        "type ignored" => {"collection": "users", "field": "age", "type": "btree"},
        "missing collection" => {"field": "age"},
        "fields wrong type" => {"collection": "users", "fields": "age"},
    );
    golden!(out, IndexListParams,
        "minimal" => {"collection": "users"},
        "with type" => {"collection": "users", "type": "vector"},
        "non-string type ignored" => {"collection": "users", "type": 7},
        "null type" => {"collection": "users", "type": null},
        "missing collection" => {},
    );
    golden!(out, IndexDropParams,
        "minimal" => {"collection": "users", "index_name": "users_age"},
        "missing index_name" => {"collection": "users"},
    );
    golden!(out, FuzzyIndexParams,
        "minimal" => {"collection": "users", "field": "name"},
        "full" => {"collection": "users", "field": "name", "algorithm": "levenshtein", "threshold": 0.65},
        "integer threshold" => {"collection": "users", "field": "name", "threshold": 1},
        "missing field" => {"collection": "users"},
    );
    golden!(out, FulltextIndexParams,
        "minimal" => {"collection": "docs", "field": "content"},
        "full" => {"collection": "docs", "field": "content", "language": "hungarian",
                   "min_word_length": 3, "accent_folding": false},
        "missing field" => {"collection": "docs"},
    );
    golden!(out, FuzzySearchParams,
        "minimal" => {"collection": "users", "field": "name", "query": "jon"},
        "full" => {"collection": "users", "field": "name", "query": "jon", "threshold": 0.7,
                   "algorithm": "jaro_winkler", "limit": 5, "skip": 1, "projection": {"name": 1},
                   "filter": {"active": true}, "highlight": true},
        "empty filter is none" => {"collection": "users", "field": "name", "query": "jon", "filter": {}},
        "null filter" => {"collection": "users", "field": "name", "query": "jon", "filter": null},
        "non-object filter kept" => {"collection": "users", "field": "name", "query": "jon", "filter": []},
        "missing query" => {"collection": "users", "field": "name"},
    );
    golden!(out, FulltextSearchParams,
        "minimal" => {"collection": "docs", "query": "rust"},
        "full" => {"collection": "docs", "field": "title", "fields": ["title", "content"], "query": "rust",
                   "limit": 5, "skip": 2, "min_score": 0.5, "projection": {"title": 1}, "mode": "and",
                   "match_scope": "chunk", "filter": {"status": "published"}, "highlight": true,
                   "highlight_context": 200, "highlight_max_snippets": 5},
        "empty filter is none" => {"collection": "docs", "query": "rust", "filter": {}},
        "missing query" => {"collection": "docs"},
        "missing collection" => {"query": "rust"},
    );
    golden!(out, FulltextAnalyzeParams,
        "minimal" => {"text": "A király"},
        "full" => {"text": "A király", "language": "hungarian", "accent_folding": false,
                   "min_word_length": 3, "collection": "docs", "field": "content"},
        "missing text" => {"language": "hungarian"},
    );
    golden!(out, ExplainParams,
        "minimal" => {"collection": "users"},
        "full" => {"collection": "users", "query": {"age": 3}},
        "filter is not an alias" => {"collection": "users", "filter": {"age": 3}},
        "missing collection" => {},
    );
    golden!(out, FindWithHintParams,
        "minimal" => {"collection": "users", "hint": "users_age"},
        "full" => {"collection": "users", "query": {"age": 3}, "hint": "users_age",
                   "projection": {"age": 1}, "sort": {"age": 1}, "limit": 3, "skip": 1},
        "missing hint" => {"collection": "users"},
    );

    // ---- Collection / schema ----
    golden!(out, ListCollectionsParams,
        "minimal" => {},
        "unknown field ignored" => {"x": 1},
        "null" => null,
    );
    golden!(out, CollectionCreateParams,
        "minimal" => {"collection": "users"},
        "missing collection" => {},
    );
    golden!(out, CollectionDropParams,
        "minimal" => {"collection": "users"},
        "missing collection" => {},
    );
    golden!(out, CollectionRenameParams,
        "minimal" => {"old_collection": "a", "new_collection": "b"},
        "missing new_collection" => {"old_collection": "a"},
        "missing old_collection" => {"new_collection": "b"},
    );
    golden!(out, CollectionStatsParams,
        "minimal" => {"collection": "users"},
        "missing collection" => {},
    );
    golden!(out, SchemaSetParams,
        "minimal" => {"collection": "users"},
        "full" => {"collection": "users", "schema": {"type": "object"}},
        "null schema" => {"collection": "users", "schema": null},
        "missing collection" => {"schema": {}},
    );
    golden!(out, SchemaGetParams,
        "minimal" => {"collection": "users"},
        "missing collection" => {},
    );

    // ---- Transaction ----
    golden!(out, TransactionIdParams,
        "minimal" => {"transaction_id": "42"},
        "numeric id rejected" => {"transaction_id": 42},
        "missing transaction_id" => {},
    );
    golden!(out, TransactionInsertParams,
        "minimal" => {"transaction_id": "42", "collection": "users", "document": {"a": 1}},
        "missing document" => {"transaction_id": "42", "collection": "users"},
        "missing transaction_id" => {"collection": "users", "document": {}},
    );
    golden!(out, TransactionUpdateParams,
        "minimal" => {"transaction_id": "42", "collection": "users", "filter": {"a": 1}, "update": {"$set": {"a": 2}}},
        "query alias" => {"transaction_id": "42", "collection": "users", "query": {"a": 1}, "update": {}},
        "missing update" => {"transaction_id": "42", "collection": "users", "filter": {}},
    );
    golden!(out, TransactionDeleteParams,
        "minimal" => {"transaction_id": "42", "collection": "users", "filter": {"a": 1}},
        "query alias" => {"transaction_id": "42", "collection": "users", "query": {"a": 1}},
        "missing filter" => {"transaction_id": "42", "collection": "users"},
    );

    // ---- Script ----
    golden!(out, ScriptSaveParams,
        "minimal" => {"name": "s", "code": "42"},
        "full" => {"name": "s", "code": "42", "description": "d", "tags": ["a", "b"], "dependencies": ["x"]},
        "missing code" => {"name": "s"},
        "tags wrong type" => {"name": "s", "code": "42", "tags": "a"},
    );
    golden!(out, ScriptListParams,
        "minimal" => {},
        "full" => {"tags": ["a"], "match_all": true},
        "match_all wrong type" => {"match_all": 1},
    );
    golden!(out, ScriptNameParams,
        "minimal" => {"name": "s"},
        "missing name" => {},
    );
    golden!(out, ScriptRunParams,
        "minimal" => {"name": "s"},
        "full" => {"name": "s", "params": {"x": 1}, "max_operations": 5000},
        "missing name" => {"params": {}},
        "negative max_operations" => {"name": "s", "max_operations": -1},
    );
    golden!(out, ScriptExecParams,
        "minimal" => {"code": "42"},
        "full" => {"code": "42", "params": {"x": 1}, "max_operations": 5000},
        "missing code" => {},
    );
    golden!(out, ScriptHistoryParams,
        "minimal" => {"name": "s"},
        "full" => {"name": "s", "limit": 3},
        "missing name" => {"limit": 3},
    );
    golden!(out, ScriptVersionParams,
        "minimal" => {"name": "s", "version": 2},
        "missing version" => {"name": "s"},
        "string version" => {"name": "s", "version": "2"},
    );
    golden!(out, ScriptTagsParams,
        "minimal" => {"name": "s", "tags": []},
        "full" => {"name": "s", "tags": ["a", "b"]},
        "missing tags" => {"name": "s"},
    );

    // ---- ACL / listener ----
    golden!(out, AclSetParams,
        "minimal" => {"collection": "users", "rules": []},
        "full" => {"collection": "users", "rules": [{"ip": "10.0.0.0/8", "permission": "read"}]},
        "missing rules" => {"collection": "users"},
    );
    golden!(out, AclCollectionParams,
        "minimal" => {"collection": "users"},
        "missing collection" => {},
    );
    golden!(out, ListenerIdParams,
        "minimal" => {"id": "lan"},
        "missing id" => {},
    );
    golden!(out, ListenerAddParams,
        "minimal" => {"id": "lan", "bind": "0.0.0.0:8081"},
        "full" => {"id": "lan", "bind": "0.0.0.0:8443", "tls": true, "cert_path": "/c.pem",
                   "key_path": "/k.pem", "description": "LAN"},
        "missing bind" => {"id": "lan"},
    );

    // ---- Admin ----
    golden!(out, DbOpenParams,
        "minimal" => {"path": "/tmp/x.mlite"},
        "full" => {"path": "/tmp/x.mlite", "create": true},
        "missing path" => {},
    );
    golden!(out, AdminCompactParams,
        "minimal" => {},
        "full" => {"force": true},
        "force wrong type" => {"force": "yes"},
    );
    golden!(out, AdminKeyParams,
        "minimal" => {},
        "full" => {"admin_key": "k"},
        "admin_key wrong type" => {"admin_key": 1},
    );
    golden!(out, AdminCollectionParams,
        "minimal" => {"collection": "_x"},
        "full" => {"collection": "_x", "admin_key": "k"},
        "missing collection" => {"admin_key": "k"},
    );
    golden!(out, AdminFlagsParams,
        "minimal" => {"collection": "x"},
        "full" => {"collection": "x", "is_system": true, "protected": false, "hidden": true, "admin_key": "k"},
        "missing collection" => {"hidden": true},
    );
    golden!(out, AdminApiKeyCreateParams,
        "minimal" => {"name": "ci"},
        "full" => {"name": "ci", "admin_key": "k"},
        "missing name" => {},
    );
    golden!(out, AdminApiKeyIdParams,
        "minimal" => {"id": 7},
        "full" => {"id": 7, "admin_key": "k"},
        "missing id" => {},
        "string id" => {"id": "7"},
    );
    golden!(out, ApiKeyCreateParams,
        "minimal" => {"name": "ci", "admin_key": "k"},
        "full" => {"name": "ci", "admin_key": "k", "expires_days": 30},
        "missing admin_key" => {"name": "ci"},
    );
    golden!(out, ApiKeyRevokeParams,
        "minimal" => {"key": "abc", "admin_key": "k"},
        "missing key" => {"admin_key": "k"},
    );
    golden!(out, ApiKeyListParams,
        "minimal" => {"admin_key": "k"},
        "missing admin_key" => {},
    );

    // ---- Hybrid search / RAG ----
    golden!(out, HybridSearchParams,
        "minimal" => {"collection": "kb", "query": "q"},
        "full" => {"collection": "kb", "vector_field": "vec", "text_field": "body", "vector": [0.1, 0.25],
                   "query": "q", "limit": 7, "vector_weight": 0.6, "fulltext_weight": 0.4,
                   "projection": {"title": 1}, "search_mode": "semantic", "provider": "ollama",
                   "rerank": false, "deduplicate": true, "mmr_lambda": 0.3, "filter": {"year": 2026},
                   "rrf_k": 60.0, "title_field": "title", "mode": "and", "match_scope": "chunk",
                   "text_fields": ["body", "title"], "merge_chunks": false, "group_by_document": true,
                   "max_chunks_per_doc": 2},
        "empty filter is none" => {"collection": "kb", "query": "q", "filter": {}},
        "integer vector and rrf_k" => {"collection": "kb", "query": "q", "vector": [1, 2], "rrf_k": 60},
        "missing query" => {"collection": "kb"},
        "missing collection" => {"query": "q"},
        "vector wrong type" => {"collection": "kb", "query": "q", "vector": "x"},
    );
    golden!(out, RagCollectionCreateParams,
        "minimal" => {"collection": "kb"},
        "full" => {"collection": "kb", "embedding_field": "vec", "text_field": "body",
                   "text_fields": ["title"], "context_fields": ["customer"], "provider": "ollama",
                   "language": "hungarian"},
        "missing collection" => {},
    );
    golden!(out, RagDocumentImportParams,
        "minimal" => {"collection": "kb", "content": "text"},
        "full" => {"collection": "kb", "content": "text", "title": "T", "metadata": {"year": 2026},
                   "doc_id": "d1", "chunk_size": 500, "overlap": 50, "mode": "markdown",
                   "provider": "ollama", "if_exists": "skip", "language": "english",
                   "text_fields": ["title"], "context_fields": ["customer"]},
        "missing content" => {"collection": "kb"},
        "chunk_size wrong type" => {"collection": "kb", "content": "text", "chunk_size": "big"},
    );
    golden!(out, RagCollectionStatsParams,
        "minimal" => {"collection": "kb"},
        "missing collection" => {},
    );
    golden!(out, RagLoadAllChunksParams,
        "minimal" => {"collection": "kb", "doc_ids": []},
        "full" => {"collection": "kb", "doc_ids": ["d1", "d2"], "query": "q", "text_field": "body",
                   "projection": {"content": 1}, "merge_chunks": false, "max_chunks_per_doc": 3},
        "missing doc_ids" => {"collection": "kb"},
    );

    // ---- Structs defined next to their handlers ----
    golden!(out, VectorIndexCreateParams,
        "minimal" => {"collection": "kb", "field": "embedding", "dim": 384},
        "full" => {"collection": "kb", "field": "embedding", "dim": 384, "metric": "euclidean",
                   "max_vectors": 5000, "m": 8, "ef_construction": 100, "ef_search": 20},
        "missing dim" => {"collection": "kb", "field": "embedding"},
    );
    golden!(out, VectorSearchParams,
        "minimal" => {"collection": "kb", "vector": [0.1, 0.2]},
        "full" => {"collection": "kb", "field": "vec", "vector": [0.1, 0.2], "limit": 3, "projection": {"t": 1}},
        "missing vector" => {"collection": "kb"},
    );
    golden!(out, SearchParams,
        "minimal" => {"collection": "kb", "query": "q"},
        "full" => {"collection": "kb", "query": "q", "filter": {"year": 2026}, "limit": 9,
                   "format": "context_block", "debug": true},
        "empty filter is none" => {"collection": "kb", "query": "q", "filter": {}},
        "missing query" => {"collection": "kb"},
    );
    golden!(out, EmbedTextParams,
        "minimal" => {"text": "t"},
        "full" => {"text": "t", "provider": "ollama"},
        "missing text" => {},
    );
    golden!(out, EmbedBatchParams,
        "minimal" => {"texts": []},
        "full" => {"texts": ["a", "b"], "provider": "ollama"},
        "missing texts" => {},
    );
    golden!(out, EmbedListModelsParams,
        "minimal" => {},
    );
    golden!(out, EmbedDocumentParams,
        "minimal" => {"collection": "kb", "content": "text"},
        "full" => {"collection": "kb", "content": "text", "doc_id": "d1", "title": "T",
                   "metadata": {"a": 1}, "mode": "plain", "chunk_size": 400, "overlap": 40,
                   "provider": "ollama", "create_vector_index": false, "if_exists": "error"},
        "missing content" => {"collection": "kb"},
    );
    golden!(out, AutoEmbedEnableParams,
        "minimal" => {"collection": "a", "source_field": "content", "target_field": "emb"},
        "full" => {"collection": "a", "source_field": "content", "target_field": "emb",
                   "provider": "ollama", "model": "m", "dimension": 384, "skip_if_exists": true,
                   "chunking": {"mode": "markdown", "chunk_size": 300, "overlap": 30}},
        "empty chunking" => {"collection": "a", "source_field": "content", "target_field": "emb", "chunking": {}},
        "missing target_field" => {"collection": "a", "source_field": "content"},
        "chunking wrong type" => {"collection": "a", "source_field": "c", "target_field": "e", "chunking": 5},
    );
    golden!(out, ChunkingParams,
        "minimal" => {},
        "full" => {"mode": "markdown", "chunk_size": 300, "overlap": 30},
        "chunk_size wrong type" => {"chunk_size": "x"},
    );
    golden!(out, AutoEmbedDisableParams,
        "minimal" => {"collection": "a"},
        "missing collection" => {},
    );
    golden!(out, AutoEmbedStatusParams,
        "minimal" => {"collection": "a"},
        "missing collection" => {},
    );
    golden!(out, JobStatusParams,
        "minimal" => {"job_id": "j1"},
        "missing job_id" => {},
    );
    golden!(out, JobListParams,
        "minimal" => {},
        "full" => {"active_only": true},
        "active_only wrong type" => {"active_only": "yes"},
    );
    golden!(out, JobCancelParams,
        "minimal" => {"job_id": "j1"},
        "missing job_id" => {},
    );

    out
}

#[test]
fn params_wire_format_matches_golden() {
    let actual = render();
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(GOLDEN_PATH, &actual).expect("write golden file");
        return;
    }
    // Compared line by line, so a CRLF checkout of the golden file still matches.
    for (i, (a, g)) in actual.lines().zip(GOLDEN.lines()).enumerate() {
        assert_eq!(a, g, "first difference at golden line {}", i + 1);
    }
    assert_eq!(
        actual.lines().count(),
        GOLDEN.lines().count(),
        "golden line count differs"
    );
}

/// `auto_embed_enable` argument rules that live outside serde
/// (`AutoEmbedEnableParams::validate`), observed through the tool itself.
#[test]
fn auto_embed_enable_validate_messages() {
    let (adapter, _dir) = create_test_adapter();
    let check = |extra: Value| -> String {
        let mut v = json!({"collection": "a", "source_field": "s", "target_field": "t"});
        for (k, val) in extra.as_object().unwrap() {
            v[k] = val.clone();
        }
        let e = dispatch_err(&adapter, "auto_embed_enable", v);
        format!("{:?} {}", e.code, e.message)
    };
    // Valid arguments get past validation, to the missing embedding manager.
    let passed = check(json!({}));
    assert_eq!(check(json!({"collection": "_chunks"})), passed);
    assert_eq!(
        check(json!({"collection": ""})),
        "InvalidParams collection name cannot be empty"
    );
    assert_eq!(
        check(json!({"collection": "_private"})),
        "InvalidParams collection names starting with '_' are reserved for system collections"
    );
    assert_eq!(
        check(json!({"collection": "a/b"})),
        "InvalidParams collection name contains invalid characters (null or /)"
    );
    assert_eq!(
        check(json!({"source_field": ""})),
        "InvalidParams source_field cannot be empty"
    );
    assert_eq!(
        check(json!({"target_field": ""})),
        "InvalidParams target_field cannot be empty"
    );
    assert_eq!(
        check(json!({"target_field": "s"})),
        "InvalidParams source_field and target_field cannot be the same"
    );
    assert_eq!(
        check(json!({"provider": ""})),
        "InvalidParams provider cannot be empty"
    );
    assert_eq!(
        check(json!({"dimension": 0})),
        "InvalidParams dimension must be greater than 0"
    );
    assert_eq!(
        check(json!({"dimension": 4097})),
        "InvalidParams dimension cannot exceed 4096 (practical limit for most models)"
    );
    assert_eq!(
        check(json!({"chunking": {"chunk_size": 0}})),
        "InvalidParams chunk_size must be greater than 0"
    );
    assert_eq!(
        check(json!({"chunking": {"chunk_size": 10, "overlap": 10}})),
        "InvalidParams overlap must be less than chunk_size to prevent infinite loops"
    );
    assert!(!passed.contains("cannot be"), "unexpected: {passed}");
}
