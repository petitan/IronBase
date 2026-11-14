//! Round-trip tests: what a client serializes is what the server parses.
//!
//! For every argument struct, from a minimal and from a fully specified JSON:
//! parse → serialize → parse again gives an equal value, `None` fields are
//! omitted (never `null`), the wire keeps every field that was sent, and a fully
//! specified JSON serializes back to itself.

use ironbase_mcp_types::params::*;
use ironbase_mcp_types::tools::{self, TOOL_NAMES};
use ironbase_mcp_types::Tool;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{from_value, json, to_value, Value};
use std::fmt::Debug;

fn assert_no_null(wire: &Value, path: &str) {
    match wire {
        Value::Null => panic!("`None` must be omitted, got null at {path}"),
        Value::Object(map) => {
            for (key, value) in map {
                assert_no_null(value, &format!("{path}.{key}"));
            }
        }
        Value::Array(items) => {
            for (i, value) in items.iter().enumerate() {
                assert_no_null(value, &format!("{path}[{i}]"));
            }
        }
        _ => {}
    }
}

/// Parse `input`, serialize it and parse it again; returns the serialized form.
fn roundtrip<T>(input: Value) -> Value
where
    T: Serialize + DeserializeOwned + PartialEq + Debug,
{
    let parsed: T = from_value(input.clone()).expect("input parses");
    let wire = to_value(&parsed).expect("serializes");
    assert_no_null(&wire, "$");
    let reparsed: T = from_value(wire.clone()).expect("serialized form parses");
    assert_eq!(parsed, reparsed, "round-trip changed the value");
    assert_eq!(
        to_value(&reparsed).unwrap(),
        wire,
        "serialization is stable"
    );
    for (key, value) in input.as_object().expect("object input") {
        assert_eq!(wire.get(key), Some(value), "field `{key}` lost on the wire");
    }
    wire
}

macro_rules! roundtrip_tests {
    ($($test:ident: $ty:ty, $minimal:tt, $full:tt;)+) => {
        $(
            #[test]
            fn $test() {
                let minimal = json!($minimal);
                let minimal_wire = roundtrip::<$ty>(minimal);
                let full = json!($full);
                let full_wire = roundtrip::<$ty>(full.clone());
                assert_eq!(full_wire, full, "fully specified arguments serialize to themselves");
                // Defaults are explicit on the wire: same key set either way,
                // except the optional fields the minimal form leaves out.
                for key in minimal_wire.as_object().unwrap().keys() {
                    assert!(full_wire.get(key).is_some(), "`{key}` missing from the full form");
                }
            }
        )+
    };
}

roundtrip_tests! {
    // ---- CRUD ----
    find: FindParams,
        {"collection": "users"},
        {"collection": "users", "query": {"age": {"$gt": 18}}, "projection": {"name": 1},
         "sort": {"age": -1}, "limit": 10, "skip": 5, "include_total": true};
    find_one: FindOneParams,
        {"collection": "users"},
        {"collection": "users", "query": {"_id": 1}, "projection": {"name": 1}};
    insert_one: InsertOneParams,
        {"collection": "users", "document": {}},
        {"collection": "users", "document": {"name": "Alice", "tags": ["a"]}};
    insert_many: InsertManyParams,
        {"collection": "users", "documents": []},
        {"collection": "users", "documents": [{"a": 1}, {"a": 2}]};
    update_one: UpdateOneParams,
        {"collection": "users", "filter": {}, "update": {}},
        {"collection": "users", "filter": {"a": 1}, "update": {"$set": {"b": 2}}, "upsert": true};
    update_many: UpdateManyParams,
        {"collection": "users", "filter": {}, "update": {}},
        {"collection": "users", "filter": {"a": 1}, "update": {"$set": {"b": 2}}};
    delete: DeleteParams,
        {"collection": "users", "filter": {}},
        {"collection": "users", "filter": {"a": 1}};
    count: CountParams,
        {"collection": "users"},
        {"collection": "users", "query": {"a": 1}};
    distinct: DistinctParams,
        {"collection": "users", "field": "city"},
        {"collection": "users", "field": "city", "query": {"a": 1}, "limit": 100};
    aggregate: AggregateParams,
        {"collection": "users", "pipeline": []},
        {"collection": "users", "pipeline": [{"$match": {"a": 1}}, {"$limit": 5}]};

    // ---- Index ----
    index_create: IndexCreateParams,
        {"collection": "users"},
        {"collection": "users", "field": "age", "fields": ["a", "b"], "unique": true, "sparse": true};
    index_list: IndexListParams,
        {"collection": "users"},
        {"collection": "users", "type": "vector"};
    index_drop: IndexDropParams,
        {"collection": "users", "index_name": "users_age"},
        {"collection": "users", "index_name": "users_age"};
    fuzzy_index: FuzzyIndexParams,
        {"collection": "users", "field": "name"},
        {"collection": "users", "field": "name", "algorithm": "levenshtein", "threshold": 0.65};
    fulltext_index: FulltextIndexParams,
        {"collection": "docs", "field": "content"},
        {"collection": "docs", "field": "content", "language": "hungarian",
         "min_word_length": 3, "accent_folding": false};
    fuzzy_search: FuzzySearchParams,
        {"collection": "users", "field": "name", "query": "jon"},
        {"collection": "users", "field": "name", "query": "jon", "threshold": 0.7,
         "algorithm": "jaro_winkler", "limit": 5, "skip": 1, "projection": {"name": 1},
         "filter": {"active": true}, "highlight": true};
    fulltext_search: FulltextSearchParams,
        {"collection": "docs", "query": "rust"},
        {"collection": "docs", "field": "title", "fields": ["title", "content"], "query": "rust",
         "limit": 5, "skip": 2, "min_score": 0.5, "projection": {"title": 1}, "mode": "and",
         "match_scope": "chunk", "filter": {"status": "published"}, "highlight": true,
         "highlight_context": 200, "highlight_max_snippets": 5};
    fulltext_analyze: FulltextAnalyzeParams,
        {"text": "A király"},
        {"text": "A király", "language": "hungarian", "accent_folding": false,
         "min_word_length": 3, "collection": "docs", "field": "content"};
    explain: ExplainParams,
        {"collection": "users"},
        {"collection": "users", "query": {"age": 3}};
    find_with_hint: FindWithHintParams,
        {"collection": "users", "hint": "users_age"},
        {"collection": "users", "query": {"age": 3}, "hint": "users_age",
         "projection": {"age": 1}, "sort": {"age": 1}, "limit": 3, "skip": 1};

    // ---- Collection / schema ----
    list_collections: ListCollectionsParams, {}, {};
    collection_create: CollectionCreateParams, {"collection": "users"}, {"collection": "users"};
    collection_drop: CollectionDropParams, {"collection": "users"}, {"collection": "users"};
    collection_rename: CollectionRenameParams,
        {"old_collection": "a", "new_collection": "b"},
        {"old_collection": "a", "new_collection": "b"};
    collection_stats: CollectionStatsParams, {"collection": "users"}, {"collection": "users"};
    schema_set: SchemaSetParams,
        {"collection": "users"},
        {"collection": "users", "schema": {"type": "object"}};
    schema_get: SchemaGetParams, {"collection": "users"}, {"collection": "users"};

    // ---- Transaction ----
    transaction_id: TransactionIdParams, {"transaction_id": "42"}, {"transaction_id": "42"};
    transaction_insert: TransactionInsertParams,
        {"transaction_id": "42", "collection": "users", "document": {}},
        {"transaction_id": "42", "collection": "users", "document": {"a": 1}};
    transaction_update: TransactionUpdateParams,
        {"transaction_id": "42", "collection": "users", "filter": {}, "update": {}},
        {"transaction_id": "42", "collection": "users", "filter": {"a": 1}, "update": {"$set": {"a": 2}}};
    transaction_delete: TransactionDeleteParams,
        {"transaction_id": "42", "collection": "users", "filter": {}},
        {"transaction_id": "42", "collection": "users", "filter": {"a": 1}};

    // ---- Script ----
    script_save: ScriptSaveParams,
        {"name": "s", "code": "42"},
        {"name": "s", "code": "42", "description": "d", "tags": ["a", "b"], "dependencies": ["x"]};
    script_list: ScriptListParams,
        {},
        {"tags": ["a"], "match_all": true};
    script_name: ScriptNameParams, {"name": "s"}, {"name": "s"};
    script_run: ScriptRunParams,
        {"name": "s"},
        {"name": "s", "params": {"x": 1}, "max_operations": 5000};
    script_exec: ScriptExecParams,
        {"code": "42"},
        {"code": "42", "params": {"x": 1}, "max_operations": 5000};
    script_history: ScriptHistoryParams,
        {"name": "s"},
        {"name": "s", "limit": 3};
    script_version: ScriptVersionParams, {"name": "s", "version": 2}, {"name": "s", "version": 2};
    script_tags: ScriptTagsParams,
        {"name": "s", "tags": []},
        {"name": "s", "tags": ["a", "b"]};

    // ---- ACL / listener ----
    acl_set: AclSetParams,
        {"collection": "users", "rules": []},
        {"collection": "users", "rules": [{"ip": "10.0.0.0/8", "permission": "read"}]};
    acl_collection: AclCollectionParams, {"collection": "users"}, {"collection": "users"};
    listener_id: ListenerIdParams, {"id": "lan"}, {"id": "lan"};
    listener_add: ListenerAddParams,
        {"id": "lan", "bind": "0.0.0.0:8081"},
        {"id": "lan", "bind": "0.0.0.0:8443", "tls": true, "cert_path": "/c.pem",
         "key_path": "/k.pem", "description": "LAN"};

    // ---- Admin ----
    db_open: DbOpenParams,
        {"path": "/tmp/x.mlite"},
        {"path": "/tmp/x.mlite", "create": true};
    admin_compact: AdminCompactParams, {}, {"force": true};
    admin_key: AdminKeyParams, {}, {"admin_key": "k"};
    admin_collection: AdminCollectionParams,
        {"collection": "_x"},
        {"collection": "_x", "admin_key": "k"};
    admin_flags: AdminFlagsParams,
        {"collection": "x"},
        {"collection": "x", "is_system": true, "protected": false, "hidden": true, "admin_key": "k"};
    admin_apikey_create: AdminApiKeyCreateParams,
        {"name": "ci"},
        {"name": "ci", "admin_key": "k"};
    admin_apikey_id: AdminApiKeyIdParams,
        {"id": 7},
        {"id": 7, "admin_key": "k"};
    api_key_create: ApiKeyCreateParams,
        {"name": "ci", "admin_key": "k"},
        {"name": "ci", "admin_key": "k", "expires_days": 30};
    api_key_revoke: ApiKeyRevokeParams,
        {"key": "abc", "admin_key": "k"},
        {"key": "abc", "admin_key": "k"};
    api_key_list: ApiKeyListParams, {"admin_key": "k"}, {"admin_key": "k"};

    // ---- Hybrid search / RAG ----
    hybrid_search: HybridSearchParams,
        {"collection": "kb", "query": "q"},
        {"collection": "kb", "vector_field": "vec", "text_field": "body", "vector": [0.1, 0.25],
         "query": "q", "limit": 7, "vector_weight": 0.6, "fulltext_weight": 0.4,
         "projection": {"title": 1}, "search_mode": "semantic", "provider": "ollama",
         "rerank": false, "deduplicate": true, "mmr_lambda": 0.3, "filter": {"year": 2026},
         "rrf_k": 60.5, "title_field": "title", "mode": "and", "match_scope": "chunk",
         "text_fields": ["body", "title"], "merge_chunks": false, "group_by_document": true,
         "max_chunks_per_doc": 2};
    rag_collection_create: RagCollectionCreateParams,
        {"collection": "kb"},
        {"collection": "kb", "embedding_field": "vec", "text_field": "body",
         "text_fields": ["title"], "context_fields": ["customer"], "provider": "ollama",
         "language": "hungarian"};
    rag_document_import: RagDocumentImportParams,
        {"collection": "kb", "content": "text"},
        {"collection": "kb", "content": "text", "title": "T", "metadata": {"year": 2026},
         "doc_id": "d1", "chunk_size": 500, "overlap": 50, "mode": "markdown",
         "provider": "ollama", "if_exists": "skip", "language": "english",
         "text_fields": ["title"], "context_fields": ["customer"]};
    rag_collection_stats: RagCollectionStatsParams, {"collection": "kb"}, {"collection": "kb"};
    rag_chunks_load: RagLoadAllChunksParams,
        {"collection": "kb", "doc_ids": []},
        {"collection": "kb", "doc_ids": ["d1", "d2"], "query": "q", "text_field": "body",
         "projection": {"content": 1}, "merge_chunks": false, "max_chunks_per_doc": 3};

    // ---- Vector / retrieval ----
    vector_index_create: VectorIndexCreateParams,
        {"collection": "kb", "field": "embedding", "dim": 384},
        {"collection": "kb", "field": "embedding", "dim": 384, "metric": "euclidean",
         "max_vectors": 5000, "m": 8, "ef_construction": 100, "ef_search": 20};
    vector_search: VectorSearchParams,
        {"collection": "kb", "vector": [0.1, 0.2]},
        {"collection": "kb", "field": "vec", "vector": [0.1, 0.2], "limit": 3, "projection": {"t": 1}};
    search: SearchParams,
        {"collection": "kb", "query": "q"},
        {"collection": "kb", "query": "q", "filter": {"year": 2026}, "limit": 9,
         "format": "context_block", "debug": true};

    // ---- Embedding / auto-embedding / jobs ----
    embed_text: EmbedTextParams,
        {"text": "t"},
        {"text": "t", "provider": "ollama"};
    embed_batch: EmbedBatchParams,
        {"texts": []},
        {"texts": ["a", "b"], "provider": "ollama"};
    embed_list_models: EmbedListModelsParams, {}, {};
    embed_document: EmbedDocumentParams,
        {"collection": "kb", "content": "text"},
        {"collection": "kb", "content": "text", "doc_id": "d1", "title": "T",
         "metadata": {"a": 1}, "mode": "plain", "chunk_size": 400, "overlap": 40,
         "provider": "ollama", "create_vector_index": false, "if_exists": "error"};
    auto_embed_enable: AutoEmbedEnableParams,
        {"collection": "a", "source_field": "content", "target_field": "emb"},
        {"collection": "a", "source_field": "content", "target_field": "emb",
         "provider": "ollama", "model": "m", "dimension": 384, "skip_if_exists": true,
         "chunking": {"mode": "markdown", "chunk_size": 300, "overlap": 30}};
    chunking: ChunkingParams,
        {},
        {"mode": "markdown", "chunk_size": 300, "overlap": 30};
    auto_embed_disable: AutoEmbedDisableParams, {"collection": "a"}, {"collection": "a"};
    auto_embed_status: AutoEmbedStatusParams, {"collection": "a"}, {"collection": "a"};
    job_status: JobStatusParams, {"job_id": "j1"}, {"job_id": "j1"};
    job_list: JobListParams, {}, {"active_only": true};
    job_cancel: JobCancelParams, {"job_id": "j1"}, {"job_id": "j1"};
}

/// A minimal request spells the defaults out, with the values the server uses.
#[test]
fn defaults_are_explicit_on_the_wire() {
    assert_eq!(
        roundtrip::<FindParams>(json!({"collection": "users"})),
        json!({"collection": "users", "query": {}, "include_total": false})
    );
    assert_eq!(
        roundtrip::<HybridSearchParams>(json!({"collection": "kb", "query": "q"})),
        json!({"collection": "kb", "vector_field": "embedding", "text_field": "content",
               "query": "q", "limit": 10, "rerank": true, "deduplicate": false,
               "mmr_lambda": 0.7, "rrf_k": 20.0, "merge_chunks": true,
               "group_by_document": false})
    );
    assert_eq!(
        roundtrip::<RagDocumentImportParams>(json!({"collection": "kb", "content": "text"})),
        json!({"collection": "kb", "content": "text", "chunk_size": 1000, "overlap": 100,
               "mode": "auto", "if_exists": "replace", "language": "none"})
    );
}

/// The `query`/`filter` aliases are accepted on input; the canonical name is sent.
#[test]
fn aliases_serialize_under_the_canonical_name() {
    let p: FindParams = from_value(json!({"collection": "c", "filter": {"a": 1}})).unwrap();
    assert_eq!(to_value(&p).unwrap()["query"], json!({"a": 1}));
    assert!(to_value(&p).unwrap().get("filter").is_none());

    let p: UpdateOneParams =
        from_value(json!({"collection": "c", "query": {"a": 1}, "update": {}})).unwrap();
    assert_eq!(to_value(&p).unwrap()["filter"], json!({"a": 1}));
    assert!(to_value(&p).unwrap().get("query").is_none());
}

/// `deserialize_nonempty_filter` needs no custom serializer: an empty filter a
/// client builds by hand is sent as `{}` and parsed as "no filter", like `None`.
#[test]
fn empty_filter_is_no_filter_after_the_wire() {
    let mut p: SearchParams = from_value(json!({"collection": "kb", "query": "q"})).unwrap();
    assert_eq!(p.filter, None);
    p.filter = Some(json!({}));
    let wire = to_value(&p).unwrap();
    assert_eq!(wire["filter"], json!({}));
    let back: SearchParams = from_value(wire).unwrap();
    assert_eq!(back.filter, None);

    p.filter = Some(json!({"year": 2026}));
    let back: SearchParams = from_value(to_value(&p).unwrap()).unwrap();
    assert_eq!(back, p);
}

/// Floats survive the wire bit-for-bit.
#[test]
fn floats_round_trip_exactly() {
    let vector = vec![0.1, 1.0 / 3.0, -2.5e-7, 1e300, 0.30000000000000004];
    let p = VectorSearchParams {
        collection: "kb".to_string(),
        field: "embedding".to_string(),
        vector: vector.clone(),
        limit: 10,
        projection: None,
    };
    let text = serde_json::to_string(&p).unwrap();
    let back: VectorSearchParams = serde_json::from_str(&text).unwrap();
    assert_eq!(back.vector, vector);
}

/// A client can build a `tools/call` request from a marker alone.
#[test]
fn tool_markers_build_a_call() {
    fn call<T: Tool>(args: &T::Args) -> Value {
        json!({"name": T::NAME, "arguments": args})
    }
    let args: FindParams = from_value(json!({"collection": "users", "limit": 3})).unwrap();
    assert_eq!(
        call::<tools::Find>(&args),
        json!({"name": "find", "arguments":
               {"collection": "users", "query": {}, "limit": 3, "include_total": false}})
    );
    assert_eq!(<tools::DeleteMany as Tool>::NAME, "delete_many");
    assert_eq!(<tools::RagChunksLoad as Tool>::NAME, "rag_chunks_load");
}

#[test]
fn tool_names_are_unique() {
    let mut names = TOOL_NAMES.to_vec();
    names.sort_unstable();
    let total = names.len();
    names.dedup();
    assert_eq!(names.len(), total, "duplicate Tool::NAME");
}
