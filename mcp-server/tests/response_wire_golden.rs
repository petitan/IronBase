//! Wire-format safety net for MCP tool responses.
//!
//! Read-side group (`collection_list`, `find`, `count`, `index_list`,
//! `index_stats`, `explain`, `db_stats`) against a seeded temporary database, then
//! the write/admin group (`insert_one`, `insert_many`, `update_one`,
//! `update_many`, `delete_one`, `delete_many`, `collection_create`,
//! `collection_drop`, `index_create` with every `type`, `index_drop`,
//! `index_stats_refresh`) against a second one, in call order (each case sees the
//! previous cases' writes). Every call runs through the real `dispatch_tool`; the
//! exact serialized JSON of each response (or the exact error) is
//! compared with `tests/golden/response_wire.txt`. Any change in what the server
//! sends — a renamed or reordered field, `null` vs. absent, a number format —
//! fails here.
//!
//! Normalised (nondeterministic) values, replaced before recording:
//! - vector index `created_at` (creation timestamp) and `cache_file` (path in the
//!   temporary directory)
//! - `db_stats`: `database.path`, `database.file_size_bytes`, `database.file_size`,
//!   the `storage_wastage` file and live sizes (byte counts of the file on disk)
//!   and `server.version` (the crate version, bumped on every
//!   release)
//! - the key order inside `find` documents (the engine stores document fields in
//!   hash-map order, different in every process): keys sorted, recursively
//! - the order of `collection_list.collections` and `db_stats.collections` (the
//!   server lists collections in hash-map order): sorted by name
//!
//! The write/admin group needs no normalisation: generated `_id`s are
//! auto-increment integers (deterministic in a fresh database) and no response
//! carries a timestamp or a path. Its auto-embedding fields (`auto_embedded`,
//! `auto_embedded_count`, `auto_embed_error`) come from a deterministic in-test
//! embedding provider.
//!
//! The search group (`fulltext_search`, `fulltext_analyze`, `fuzzy_search`,
//! `vector_search`, `search`, on a third database) normalises only the key order
//! of the documents its hits carry (hash-map order, as for `find`): a hit's own
//! document keys are sorted into the positions they occupy, while the engine
//! metadata keys (`_score`, `_matched_tokens`, `_highlights`, `_matched_value`,
//! `_highlight`; `search`: `doc_id`, `relevance`, `passages`) keep theirs, so
//! where the server puts them stays pinned (except where a "clash" document
//! has its own field of that name: the metadata then stands at the field's
//! hash-order position and is sorted with the document keys). The
//! `_matched_tokens` of a
//! multi-field `fulltext_search` (`fields`) are sorted: the server merges the
//! fields' tokens through a hash set. Scores are not normalised: the data has no
//! score ties (tied hits come back in hash order) and the BM25, fuzzy, similarity
//! and fusion arithmetic is sequential, so the floats are byte-identical from
//! run to run. `search` embeds with the same in-test provider as the
//! auto-embedding cases. A second test parses every recorded search-group
//! response, strictly, with its tool's shared response type
//! (`ironbase_mcp_types`) and serializes it back to the same bytes.
//!
//! The embedding group (`embed_text`, `embed_batch`, `embed_models_list`,
//! `embed_cache_stats`, `embed_cache_clear`, `auto_embed_enable`,
//! `auto_embed_disable`, `auto_embed_status`, on a fourth database) embeds with
//! the same in-test provider, registered as "length" (the default) and
//! "other". It normalises the order of `embed_models_list.models` (hash-map
//! order: sorted by `provider`); an error listing the providers is recorded
//! with one provider only, for the same reason. The two jobs (a backfill and
//! a re-embed) are each run to their end before the next case. The job group (`embed_job_list`,
//! `embed_job_status`, `embed_job_cancel`) runs on jobs set up directly in a
//! job manager; it normalises the jobs' `created_at_secs` and
//! `updated_at_secs` (ages in seconds, measured at the call) and the order of
//! `embed_job_list.jobs` (hash-map order: sorted by `id`). A third test parses
//! these groups' responses with the shared types, as for the search group.
//!
//! The RAG group (`rag_collection_create`, `rag_document_import`,
//! `embed_document`, `rag_collection_stats`, `rag_chunks_load`, on a fifth
//! database) embeds with the same in-test provider. It normalises a generated
//! `doc_id` (a UUID: only when the request names none), the stored RAG
//! configuration's `created_at` and the vector indexes' creation fields in
//! `rag_collection_stats` (whose index lists it sorts by name: hash-map
//! order), and the key order of the `rag_chunks_load` documents (as for
//! `find`; a `_score` keeps its position). Another test parses its responses
//! with the shared types.
//!
//! The ACL and listener group (on a sixth database) sorts the ACL documents of
//! `acl_list` by `collection` and the listeners of `listener_list` by `_id`.
//! (The stored ACL documents came out in the storage's hash-map key order
//! before 0.3.472; those cases were recorded with sorted keys, which the typed
//! response's fixed order replaced.)
//!
//! Regenerate (only for an intended wire-format change):
//! `UPDATE_GOLDEN=1 cargo test --test response_wire_golden`

use mcp_ironbase::embedding::{CacheConfig, EmbeddingResult};
use mcp_ironbase::{
    dispatch_tool, EmbeddingError, EmbeddingManager, EmbeddingProvider, IronBaseAdapter,
    JobManager, JobType, ServerInfo,
};
use serde_json::{json, Value};
use std::fmt::Write;
use std::sync::Arc;

mod common;
use common::{create_test_adapter, dispatch_ok};

const GOLDEN: &str = include_str!("golden/response_wire.txt");
const GOLDEN_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/golden/response_wire.txt"
);

/// Replaced value of a normalised field
fn placeholder(field: &str) -> Value {
    Value::String(format!("<{field}>"))
}

fn normalize_vector_index(index: &mut Value) {
    for field in ["created_at", "cache_file"] {
        if let Some(v) = index.get_mut(field) {
            *v = placeholder(field);
        }
    }
}

/// Sort the keys of every object in `v`, recursively
fn sort_keys(v: &mut Value) {
    match v {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = std::mem::take(map).into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            for (k, mut child) in entries {
                sort_keys(&mut child);
                map.insert(k, child);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(sort_keys),
        _ => {}
    }
}

/// A job's age and time since its last update (seconds, measured at the call)
fn normalize_job_ages(job: &mut Value) {
    for field in ["created_at_secs", "updated_at_secs"] {
        job[field] = placeholder(field);
    }
}

fn sort_by_key(list: &mut Value, key: &str) {
    if let Some(items) = list.as_array_mut() {
        items.sort_by(|a, b| a[key].to_string().cmp(&b[key].to_string()));
    }
}

fn sort_by_name(list: &mut Value) {
    if let Some(items) = list.as_array_mut() {
        items.sort_by(|a, b| {
            let key = |v: &Value| v.get("name").unwrap_or(v).to_string();
            key(a).cmp(&key(b))
        });
    }
}

/// Replace exactly the nondeterministic values listed in the module docs
fn normalize(tool: &str, args: &Value, out: &mut Value) {
    match tool {
        "collection_list" => sort_by_name(&mut out["collections"]),
        "find" => sort_keys(&mut out["documents"]),
        "index_list" | "index_stats" => {
            let index_type = args.get("type").and_then(Value::as_str);
            let list = match (tool, index_type) {
                ("index_list", None) => out.get_mut("vector_indexes"),
                (_, Some("vector")) => out.get_mut("indexes"),
                _ => None,
            };
            if let Some(items) = list.and_then(Value::as_array_mut) {
                items.iter_mut().for_each(normalize_vector_index);
            }
        }
        "db_stats" => {
            for field in ["path", "file_size_bytes", "file_size"] {
                out["database"][field] = placeholder(field);
            }
            for field in [
                "file_size_bytes",
                "file_size_mb",
                "estimated_live_bytes",
                "estimated_live_mb",
            ] {
                out["storage_wastage"][field] = placeholder(field);
            }
            out["server"]["version"] = placeholder("version");
            sort_by_name(&mut out["collections"]);
        }
        "embed_models_list" => sort_by_key(&mut out["models"], "provider"),
        "embed_job_list" => {
            sort_by_key(&mut out["jobs"], "id");
            if let Some(jobs) = out["jobs"].as_array_mut() {
                jobs.iter_mut().for_each(normalize_job_ages);
            }
        }
        "embed_job_status" => {
            if let Some(job) = out.get_mut("job") {
                normalize_job_ages(job);
            }
        }
        "rag_collection_stats" => {
            if let Some(created_at) = out["config"].get_mut("created_at") {
                *created_at = placeholder("created_at");
            }
            if let Some(items) = out["stats"]["vector_indexes"].as_array_mut() {
                items.iter_mut().for_each(normalize_vector_index);
            }
            sort_by_name(&mut out["stats"]["vector_indexes"]);
            sort_by_name(&mut out["stats"]["fulltext_indexes"]);
        }
        "rag_document_import" | "embed_document" if args.get("doc_id").is_none() => {
            if let Some(doc_id) = out.get_mut("doc_id") {
                *doc_id = placeholder("doc_id");
            }
        }
        "rag_chunks_load" => {
            if let Some(items) = out["results"].as_array_mut() {
                for item in items {
                    sort_document_keys(item, &["_score"]);
                }
            }
        }
        // The list in storage order
        "acl_list" => sort_by_key(&mut out["rules"], "collection"),
        "listener_list" => sort_by_key(&mut out["listeners"], "_id"),
        "fulltext_search" | "fuzzy_search" | "vector_search" | "search" => {
            normalize_search(tool, args, out);
            if tool == "fulltext_search" && args.get("fields").is_some() {
                sort_matched_tokens(out);
            }
        }
        _ => {}
    }
}

fn record(
    out: &mut String,
    adapter: &Arc<IronBaseAdapter>,
    server_info: Option<&ServerInfo>,
    tool: &str,
    label: &str,
    args: Value,
) {
    record_with(out, adapter, server_info, &None, tool, label, args);
}

fn record_with(
    out: &mut String,
    adapter: &Arc<IronBaseAdapter>,
    server_info: Option<&ServerInfo>,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
    tool: &str,
    label: &str,
    args: Value,
) {
    record_full(
        out,
        adapter,
        server_info,
        embedding_manager,
        &None,
        tool,
        label,
        args,
    );
}

#[allow(clippy::too_many_arguments)]
fn record_full(
    out: &mut String,
    adapter: &Arc<IronBaseAdapter>,
    server_info: Option<&ServerInfo>,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
    job_manager: &Option<Arc<JobManager>>,
    tool: &str,
    label: &str,
    args: Value,
) {
    writeln!(out, "## {tool} / {label}").unwrap();
    writeln!(out, "in:  {args}").unwrap();
    match dispatch_tool(
        tool,
        args.clone(),
        adapter,
        None,
        server_info,
        None,
        None,
        embedding_manager,
        job_manager,
    ) {
        Ok(mut v) => {
            normalize(tool, &args, &mut v);
            writeln!(out, "out: {}", serde_json::to_string(&v).unwrap()).unwrap();
        }
        Err(e) => writeln!(out, "err: {:?} {}", e.code, e.message).unwrap(),
    }
    out.push('\n');
}

macro_rules! golden {
    ($out:ident, $adapter:ident, $tool:literal, $($label:literal => $json:tt),+ $(,)?) => {
        $( record(&mut $out, &$adapter, None, $tool, $label, json!($json)); )+
    };
}

/// "users": nested, array, null and accented values, with a B+ tree, compound,
/// unique, fulltext, fuzzy and vector index; "empty": no documents
fn seed(adapter: &Arc<IronBaseAdapter>) {
    dispatch_ok(adapter, "collection_create", json!({"collection": "empty"}));
    let docs = [
        json!({"_id": 1, "name": "Álmos", "age": 30, "email": "almos@example.hu", "city": "Győr",
               "tags": ["rust", "db"], "address": {"zip": "9021", "geo": {"lat": 47.68, "lon": 17.63}},
               "nick": null, "bio": "Rust fejlesztő és király", "emb": [0.1, 0.2, 0.3]}),
        json!({"_id": 2, "name": "Bea", "age": 25, "email": "bea@example.com", "city": "Pécs",
               "tags": [], "address": {"zip": "7621"}, "nick": "B",
               "bio": "Adatbázis szakértő", "emb": [0.3, 0.2, 0.1]}),
        json!({"_id": 3, "name": "Csaba", "age": 41, "email": "csaba@example.com", "city": "Győr",
               "tags": ["go"], "address": null, "bio": "Király a backendben", "emb": [0.0, 1.0, 0.0]}),
        json!({"_id": 4, "name": "Dóra", "age": 25.5, "email": "dora@example.com",
               "scores": [1, 2.5, -3], "meta": {"active": true, "note": "ünnep"}}),
    ];
    for doc in docs {
        dispatch_ok(
            adapter,
            "insert_one",
            json!({"collection": "users", "document": doc}),
        );
    }
    for args in [
        json!({"collection": "users", "field": "age"}),
        json!({"collection": "users", "fields": ["city", "age"]}),
        json!({"collection": "users", "field": "email", "unique": true}),
        json!({"type": "fulltext", "collection": "users", "field": "bio", "language": "hungarian"}),
        json!({"type": "fuzzy", "collection": "users", "field": "name"}),
        json!({"type": "vector", "collection": "users", "field": "emb", "dim": 3, "metric": "dot_product"}),
    ] {
        dispatch_ok(adapter, "index_create", args);
    }
    dispatch_ok(
        adapter,
        "index_stats_refresh",
        json!({"collection": "users"}),
    );
}

fn render() -> String {
    let (adapter, _dir) = create_test_adapter();
    seed(&adapter);
    let mut out = String::new();

    golden!(out, adapter, "collection_list",
        "all" => {},
    );
    golden!(out, adapter, "find",
        "all" => {"collection": "users"},
        "empty collection" => {"collection": "empty"},
        "nested filter" => {"collection": "users", "query": {"address.zip": "9021"}},
        "null filter" => {"collection": "users", "filter": {"nick": null}},
        "no match" => {"collection": "users", "query": {"age": {"$gt": 100}}},
        "sort skip limit" => {"collection": "users", "sort": {"age": -1}, "skip": 1, "limit": 2},
        "projection" => {"collection": "users", "projection": {"name": 1, "address.geo": 1}},
        "include_total" => {"collection": "users", "query": {"city": "Győr"}, "limit": 1, "include_total": true},
        "include_total empty" => {"collection": "empty", "include_total": true},
        "missing collection" => {"collection": "nope"},
    );
    golden!(out, adapter, "count",
        "all" => {"collection": "users"},
        "filtered" => {"collection": "users", "query": {"city": "Győr"}},
        "empty collection" => {"collection": "empty"},
        "missing collection" => {"collection": "nope"},
    );
    golden!(out, adapter, "index_list",
        "all types" => {"collection": "users"},
        "btree" => {"collection": "users", "type": "btree"},
        "fulltext" => {"collection": "users", "type": "fulltext"},
        "fuzzy" => {"collection": "users", "type": "fuzzy"},
        "vector" => {"collection": "users", "type": "vector"},
        "empty collection" => {"collection": "empty"},
        "empty collection vector" => {"collection": "empty", "type": "vector"},
        "unknown type" => {"collection": "users", "type": "hash"},
    );
    golden!(out, adapter, "index_stats",
        "default" => {"collection": "users"},
        "btree" => {"collection": "users", "type": "btree"},
        "fulltext" => {"collection": "users", "type": "fulltext"},
        "fuzzy" => {"collection": "users", "type": "fuzzy"},
        "vector" => {"collection": "users", "type": "vector"},
        "empty collection" => {"collection": "empty"},
        "unknown type" => {"collection": "users", "type": "hash"},
    );
    golden!(out, adapter, "explain",
        "indexed equality" => {"collection": "users", "query": {"email": "bea@example.com"}},
        "indexed range" => {"collection": "users", "query": {"age": {"$gte": 25, "$lt": 40}}},
        "compound prefix" => {"collection": "users", "query": {"city": "Győr", "age": 30}},
        "indexed $in" => {"collection": "users", "query": {"age": {"$in": [25, 30]}}},
        "unindexed" => {"collection": "users", "query": {"name": "Bea"}},
        "empty query" => {"collection": "users", "query": {}},
        "$or" => {"collection": "users", "query": {"$or": [{"age": 25}, {"name": "Bea"}]}},
        "empty collection" => {"collection": "empty", "query": {"a": 1}},
    );
    golden!(out, adapter, "db_stats",
        "stdio" => {},
    );
    let http = ServerInfo {
        protocol: "http".to_string(),
        host: "127.0.0.1".to_string(),
        port: 8080,
        require_api_key: true,
    };
    record(
        &mut out,
        &adapter,
        Some(&http),
        "db_stats",
        "http",
        json!({}),
    );

    render_write_group(&mut out);
    render_search_group(&mut out);
    render_embedding_group(&mut out);
    render_job_group(&mut out);
    render_rag_group(&mut out);
    render_acl_listener_group(&mut out);
    out
}

/// "w": three documents with an `age` index; "u": two documents with distinct
/// `email`s, a shared `grp` and a `bio`; "drop_me": one document
fn seed_write_group(adapter: &Arc<IronBaseAdapter>) {
    dispatch_ok(
        adapter,
        "insert_many",
        json!({"collection": "w", "documents": [
            {"_id": 1, "name": "Anna", "age": 30, "team": "x", "tags": ["a"]},
            {"_id": 2, "name": "Béla", "age": 30, "team": "x"},
            {"_id": 3, "name": "Cili", "age": 41, "team": "x"},
        ]}),
    );
    dispatch_ok(
        adapter,
        "insert_many",
        json!({"collection": "u", "documents": [
            {"_id": 1, "email": "a@x.hu", "grp": 1, "bio": "Király kódoló"},
            {"_id": 2, "email": "b@x.hu", "grp": 1, "bio": "Adatbázis gazda"},
        ]}),
    );
    dispatch_ok(
        adapter,
        "index_create",
        json!({"collection": "w", "field": "age"}),
    );
    dispatch_ok(
        adapter,
        "insert_one",
        json!({"collection": "drop_me", "document": {"_id": 1}}),
    );
}

/// The write/admin tool group, on its own database (cases run in order)
fn render_write_group(out: &mut String) {
    let (adapter, _dir) = create_test_adapter();
    seed_write_group(&adapter);
    let mut out_local = String::new();
    let out_ref = &mut out_local;
    macro_rules! case {
        ($tool:literal, $($label:literal => $json:tt),+ $(,)?) => {
            $( record(out_ref, &adapter, None, $tool, $label, json!($json)); )+
        };
    }

    case!("insert_one",
        "int _id" => {"collection": "w", "document": {"_id": 10, "name": "Dani"}},
        "string _id" => {"collection": "w", "document": {"_id": "e-1", "name": "Edit"}},
        "ObjectId-like _id" => {"collection": "w", "document": {"_id": {"$oid": "507f1f77bcf86cd799439011"}, "name": "Feri"}},
        "object _id" => {"collection": "w", "document": {"_id": {"k": 1}, "name": "Gizi"}},
        "no _id" => {"collection": "w", "document": {"name": "Hugó"}},
        "no _id again" => {"collection": "w", "document": {"name": "Ilona"}},
        "null _id" => {"collection": "w", "document": {"_id": null, "name": "Jani"}},
        "duplicate _id" => {"collection": "w", "document": {"_id": 1, "name": "Anna2"}},
        "new collection" => {"collection": "fresh", "document": {"a": 1}},
        "empty document" => {"collection": "w", "document": {}},
        "non-object document" => {"collection": "w", "document": [1, 2]},
        "missing document" => {"collection": "w"},
        "empty collection name" => {"collection": "", "document": {"a": 1}},
        "invalid collection name" => {"collection": "a$b", "document": {"a": 1}},
    );
    case!("insert_many",
        "mixed _ids" => {"collection": "w", "documents": [{"_id": 20, "n": 1}, {"_id": "s-21", "n": 2}, {"n": 3}]},
        "single" => {"collection": "w", "documents": [{"_id": 22}]},
        "empty list" => {"collection": "w", "documents": []},
        "duplicate _id in batch" => {"collection": "w", "documents": [{"_id": 30}, {"_id": 30}]},
        "duplicate existing _id" => {"collection": "w", "documents": [{"_id": 31}, {"_id": 1}]},
        "non-object item" => {"collection": "w", "documents": [{"_id": 32}, 5]},
        "documents not array" => {"collection": "w", "documents": {"_id": 33}},
        "missing documents" => {"collection": "w"},
    );
    case!("update_one",
        "matched and modified" => {"collection": "w", "filter": {"_id": 1}, "update": {"$set": {"age": 31}}},
        "matched, not modified" => {"collection": "w", "filter": {"_id": 1}, "update": {"$set": {"age": 31}}},
        "query alias" => {"collection": "w", "query": {"_id": 2}, "update": {"$inc": {"age": 1}}},
        "no match" => {"collection": "w", "filter": {"_id": 999}, "update": {"$set": {"x": 1}}},
        "upsert inserts" => {"collection": "w", "filter": {"name": "Upsert"}, "update": {"$set": {"age": 5}}, "upsert": true},
        "upsert with _id inserts" => {"collection": "w", "filter": {"_id": "u-1"}, "update": {"$set": {"age": 6}}, "upsert": true},
        "upsert matches" => {"collection": "w", "filter": {"name": "Upsert"}, "update": {"$set": {"age": 7}}, "upsert": true},
        "multiple matches updates one" => {"collection": "w", "filter": {"team": "x"}, "update": {"$set": {"flag": true}}},
        "missing collection" => {"collection": "nope", "filter": {"_id": 1}, "update": {"$set": {"x": 1}}},
        "replacement document" => {"collection": "w", "filter": {"_id": 1}, "update": {"age": 1}},
        "unknown operator" => {"collection": "w", "filter": {"_id": 1}, "update": {"$bogus": {"age": 1}}},
        "empty update" => {"collection": "w", "filter": {"_id": 1}, "update": {}},
        "missing update" => {"collection": "w", "filter": {"_id": 1}},
        "update _id" => {"collection": "w", "filter": {"_id": 1}, "update": {"$set": {"_id": 99}}},
    );
    case!("update_many",
        "matched and modified" => {"collection": "w", "filter": {"age": {"$gte": 30}}, "update": {"$set": {"seen": true}}},
        "matched, not modified" => {"collection": "w", "filter": {"age": {"$gte": 30}}, "update": {"$set": {"seen": true}}},
        "no match" => {"collection": "w", "filter": {"age": 1000}, "update": {"$set": {"x": 1}}},
        "empty filter" => {"collection": "w", "filter": {}, "update": {"$inc": {"v": 1}}},
        "upsert ignored" => {"collection": "w", "filter": {"name": "Nobody"}, "update": {"$set": {"x": 1}}, "upsert": true},
        "missing collection" => {"collection": "nope", "filter": {}, "update": {"$set": {"x": 1}}},
        "replacement document" => {"collection": "w", "filter": {}, "update": {"x": 1}},
    );
    case!("delete_one",
        "deleted" => {"collection": "w", "filter": {"_id": 10}},
        "not found" => {"collection": "w", "filter": {"_id": 10}},
        "query alias" => {"collection": "w", "query": {"_id": "e-1"}},
        "string _id" => {"collection": "w", "filter": {"_id": "s-21"}},
        "multiple matches deletes one" => {"collection": "w", "filter": {"seen": true}},
        "missing collection" => {"collection": "nope", "filter": {"_id": 1}},
        "invalid filter" => {"collection": "w", "filter": {"$bogus": 1}},
    );
    case!("delete_many",
        "deleted" => {"collection": "w", "filter": {"seen": true}},
        "not found" => {"collection": "w", "filter": {"seen": true}},
        "missing collection" => {"collection": "nope", "filter": {}},
    );
    case!("collection_create",
        "new" => {"collection": "c1"},
        "already exists" => {"collection": "c1"},
        "existing with documents" => {"collection": "w"},
        "empty name" => {"collection": ""},
        "invalid name" => {"collection": "bad name!"},
        "system prefix" => {"collection": "_system_x"},
        "missing collection" => {},
    );
    case!("collection_drop",
        "existing" => {"collection": "drop_me"},
        "not found" => {"collection": "drop_me"},
        "empty collection" => {"collection": "c1"},
        "missing collection" => {},
    );
    case!("index_create",
        "btree single" => {"collection": "w", "field": "name"},
        "btree explicit type" => {"type": "btree", "collection": "w", "field": "n"},
        "btree unique" => {"collection": "u", "field": "email", "unique": true},
        "unique on a field most documents lack" => {"collection": "w", "field": "uniq_field", "unique": true},
        "btree sparse" => {"collection": "w", "field": "tags", "sparse": true},
        "btree unique sparse" => {"collection": "w", "field": "code", "unique": true, "sparse": true},
        "compound" => {"collection": "w", "fields": ["name", "age"]},
        "compound unique sparse" => {"collection": "w", "fields": ["age", "flag"], "unique": true, "sparse": true},
        "fields wins over field" => {"collection": "w", "field": "x", "fields": ["v", "age"]},
        "single-item fields" => {"collection": "w", "fields": ["v"]},
        "duplicate single" => {"collection": "w", "field": "name"},
        "duplicate compound" => {"collection": "w", "fields": ["name", "age"]},
        "unique on duplicate data" => {"collection": "u", "field": "grp", "unique": true},
        "empty fields" => {"collection": "w", "fields": []},
        "neither field nor fields" => {"collection": "w"},
        "new collection" => {"collection": "idx_new", "field": "a"},
        "fulltext defaults" => {"type": "fulltext", "collection": "w", "field": "name"},
        "fulltext full" => {"type": "fulltext", "collection": "u", "field": "bio", "language": "hungarian", "min_word_length": 3, "accent_folding": false},
        "fulltext on a field without text" => {"type": "fulltext", "collection": "w", "field": "no_text"},
        "fulltext duplicate" => {"type": "fulltext", "collection": "w", "field": "name"},
        "fulltext missing field" => {"type": "fulltext", "collection": "w"},
        "fuzzy defaults" => {"type": "fuzzy", "collection": "w", "field": "name"},
        "fuzzy full" => {"type": "fuzzy", "collection": "w", "field": "nick", "algorithm": "levenshtein", "threshold": 0.65},
        "fuzzy integer threshold" => {"type": "fuzzy", "collection": "w", "field": "city", "threshold": 1},
        "fuzzy duplicate" => {"type": "fuzzy", "collection": "w", "field": "name"},
        "vector defaults" => {"type": "vector", "collection": "w", "field": "emb", "dim": 3},
        "vector full" => {"type": "vector", "collection": "w", "field": "emb2", "dim": 4, "metric": "dot_product",
                          "max_vectors": 500, "m": 8, "ef_construction": 100, "ef_search": 20},
        "vector euclidean" => {"type": "vector", "collection": "w", "field": "emb3", "dim": 2, "metric": "euclidean"},
        "vector duplicate" => {"type": "vector", "collection": "w", "field": "emb", "dim": 3},
        "vector unknown metric" => {"type": "vector", "collection": "w", "field": "emb4", "dim": 3, "metric": "manhattan"},
        "vector zero dim" => {"type": "vector", "collection": "w", "field": "emb5", "dim": 0},
        "vector missing dim" => {"type": "vector", "collection": "w", "field": "emb6"},
        "unknown type" => {"type": "hash", "collection": "w", "field": "name"},
        "non-string type" => {"type": 7, "collection": "w", "field": "type7"},
        "empty collection name" => {"collection": "", "field": "a"},
        "wrong-typed field" => {"collection": 5, "field": "a"},
        "vector wrong-typed dim" => {"type": "vector", "collection": "w", "field": "e", "dim": "3"},
        "not an object" => "w",
        "null" => null,
    );
    case!("index_drop",
        "btree" => {"collection": "w", "index_name": "w_name"},
        "compound" => {"collection": "w", "index_name": "w_name_age"},
        "fulltext" => {"collection": "w", "index_name": "w_name_fts"},
        "fuzzy" => {"collection": "w", "index_name": "w_name_fuzzy"},
        "vector" => {"collection": "w", "index_name": "w_vec_emb"},
        "primary _id index" => {"collection": "w", "index_name": "w_id"},
        "not found" => {"collection": "w", "index_name": "w_name"},
        "missing collection" => {"collection": "nope", "index_name": "nope_age"},
        "missing index_name" => {"collection": "w"},
    );
    case!("index_stats_refresh",
        "collection" => {"collection": "w"},
        "type ignored" => {"collection": "w", "type": "vector"},
        "collection without documents" => {"collection": "c2_missing"},
        "missing collection param" => {},
    );
    out.push_str(&out_local);
    render_auto_embed_group(out);
}

/// Embeds a text as `[character count, 1.0]`; fails on a text containing `fail`
struct LengthProvider;

impl EmbeddingProvider for LengthProvider {
    fn embed(&self, text: &str) -> EmbeddingResult<Vec<f32>> {
        if text.contains("fail") {
            return Err(EmbeddingError::EmbeddingFailed(format!(
                "cannot embed '{text}'"
            )));
        }
        Ok(vec![text.chars().count() as f32, 1.0])
    }
    fn dimension(&self) -> usize {
        2
    }
    fn model_name(&self) -> &str {
        "length"
    }
    fn provider_name(&self) -> &str {
        "length"
    }
}

/// The write tools' auto-embedding fields: "kb" auto-embeds `content` into
/// `embedding` (skip_if_exists) with [`LengthProvider`]
fn render_auto_embed_group(out: &mut String) {
    let (adapter, _dir) = create_test_adapter();
    let mut manager = EmbeddingManager::new();
    manager.register_provider("length", Arc::new(LengthProvider));
    let manager = Some(Arc::new(manager));
    dispatch_ok(
        &adapter,
        "insert_many",
        json!({"collection": "kb", "documents": [
            {"_id": 1, "group": "g", "content": "abc", "embedding": [0.0, 0.0]},
            {"_id": 2, "group": "g", "content": "abcd", "embedding": [0.0, 0.0]},
        ]}),
    );
    adapter
        .set_auto_embedding_config(
            "kb",
            Some(ironbase_core::storage::AutoEmbeddingConfig {
                enabled: true,
                source_field: "content".to_string(),
                target_field: "embedding".to_string(),
                provider: "length".to_string(),
                dimension: Some(2),
                skip_if_exists: true,
                ..Default::default()
            }),
        )
        .unwrap();
    macro_rules! case {
        ($tool:literal, $($label:literal => $json:tt),+ $(,)?) => {
            $( record_with(out, &adapter, None, &manager, $tool, $label, json!($json)); )+
        };
    }

    case!("insert_one",
        "auto-embedded" => {"collection": "kb", "document": {"_id": 10, "content": "hello"}},
        "vector given" => {"collection": "kb", "document": {"_id": 11, "content": "hi", "embedding": [9.0, 9.0]}},
        "no source field" => {"collection": "kb", "document": {"_id": 12}},
        "embedding fails" => {"collection": "kb", "document": {"_id": 13, "content": "fail"}},
    );
    case!("insert_many",
        "auto-embedded" => {"collection": "kb", "documents": [{"_id": 20, "content": "a"}, {"_id": 21, "content": "bb"}, {"_id": 22}]},
        "none embedded" => {"collection": "kb", "documents": [{"_id": 23}]},
        "embedding fails" => {"collection": "kb", "documents": [{"_id": 24, "content": "fail"}]},
    );
    case!("update_one",
        "re-embedded" => {"collection": "kb", "filter": {"_id": 1}, "update": {"$set": {"content": "abcdef"}}},
        "source untouched" => {"collection": "kb", "filter": {"_id": 1}, "update": {"$set": {"group": "h"}}},
        "re-embedding fails" => {"collection": "kb", "filter": {"_id": 2}, "update": {"$set": {"content": "fail now"}}},
        "upsert auto-embedded" => {"collection": "kb", "filter": {"_id": 30}, "update": {"$set": {"content": "new"}}, "upsert": true},
        "no match" => {"collection": "kb", "filter": {"_id": 999}, "update": {"$set": {"content": "x"}}},
    );
    case!("update_many",
        "re-embedded" => {"collection": "kb", "filter": {"group": "g"}, "update": {"$set": {"content": "same"}}},
        "re-embedding fails" => {"collection": "kb", "filter": {"group": "g"}, "update": {"$set": {"content": "fail all"}}},
        "no match" => {"collection": "kb", "filter": {"group": "none"}, "update": {"$set": {"content": "x"}}},
    );
}

/// Engine metadata keys of the `fulltext_search` / `fuzzy_search` /
/// `vector_search` hits: they keep their position when [`sort_document_keys`]
/// sorts the document keys
const HIT_METADATA_KEYS: &[&str] = &[
    "_score",
    "_matched_tokens",
    "_highlights",
    "_matched_value",
    "_highlight",
];

/// The keys the `search` tool writes into each document itself
const SEARCH_DOCUMENT_KEYS: &[&str] = &["doc_id", "relevance", "passages"];

/// Sort the document keys of a search hit in place: the `fixed` keys stay
/// where they are, the document's own keys (hash-map order, see the module
/// docs) are sorted into the remaining positions, recursively
fn sort_document_keys(hit: &mut Value, fixed: &[&str]) {
    let Value::Object(map) = hit else { return };
    let entries: Vec<(String, Value)> = std::mem::take(map).into_iter().collect();
    let mut own: Vec<(String, Value)> = entries
        .iter()
        .filter(|(k, _)| !fixed.contains(&k.as_str()))
        .cloned()
        .collect();
    own.sort_by(|a, b| a.0.cmp(&b.0));
    let mut own = own.into_iter();
    for (k, v) in entries {
        if fixed.contains(&k.as_str()) {
            map.insert(k, v);
        } else {
            let (k, mut v) = own.next().unwrap();
            sort_keys(&mut v);
            map.insert(k, v);
        }
    }
}

/// Sort the `_matched_tokens` of every `fulltext_search` hit: a multi-field
/// search (`fields`) merges the fields' matched tokens through a hash set, so
/// their order differs from process to process
fn sort_matched_tokens(out: &mut Value) {
    for hit in out["results"].as_array_mut().into_iter().flatten() {
        if let Some(tokens) = hit.get_mut("_matched_tokens").and_then(Value::as_array_mut) {
            tokens.sort_by_key(|t| t.to_string());
        }
    }
}

/// The search tools' normalisation: the document keys of every hit. In the
/// "clash" collection a document's own metadata-named field is one of its keys
/// (hash-map order): where the server writes the metadata over it, the metadata
/// stands at the field's position, so it is sorted with the document keys. The
/// exception is the `_score` a `vector_search` with a `projection` writes first:
/// the document's own `_score` replaces its value there, at a fixed position.
fn normalize_search(tool: &str, args: &Value, out: &mut Value) {
    let (list, fixed) = match tool {
        "search" => (out.get_mut("documents"), SEARCH_DOCUMENT_KEYS),
        _ => (out.get_mut("results"), HIT_METADATA_KEYS),
    };
    let clash = args["collection"] == "clash";
    let score_first = tool == "vector_search" && args.get("projection").is_some();
    if let Some(items) = list.and_then(Value::as_array_mut) {
        for hit in items {
            let own: &[&str] = if clash { clash_own_keys(hit) } else { &[] };
            let fixed: Vec<&str> = fixed
                .iter()
                .copied()
                .filter(|k| !own.contains(k) || (score_first && *k == "_score"))
                .collect();
            sort_document_keys(hit, &fixed);
        }
    }
}

/// The metadata-named fields of the "clash" document `hit` came from (by its
/// `_id`, or a `search` document's `doc_id`)
fn clash_own_keys(hit: &Value) -> &'static [&'static str] {
    let id = hit["_id"].as_i64().or_else(|| {
        hit["doc_id"]
            .as_str()
            .and_then(|d| d.strip_prefix('c')?.parse().ok())
    });
    let id = id.unwrap_or_else(|| panic!("a \"clash\" hit without `_id` / `doc_id`: {hit}"));
    CLASH_OWN_KEYS
        .iter()
        .find(|(n, _)| *n == id)
        .map_or(&[], |(_, keys)| keys)
}

/// "kb": RAG-style chunks (`doc_id`, `chunk_index`) with a `content` and a
/// `title` fulltext index (English), a `heading` fuzzy index and a cosine
/// `embedding` vector index (dim 2, [`LengthProvider`]'s space); "odd": a chunk
/// whose own `doc_id` is a number; "big": 61 one-chunk documents of more than
/// 200 characters each, more than `search`'s context budget holds
fn seed_search_group(adapter: &Arc<IronBaseAdapter>) {
    dispatch_ok(
        adapter,
        "insert_many",
        json!({"collection": "kb", "documents": [
            {"_id": 1, "doc_id": "d1", "chunk_index": 0, "title": "Rust Handbook", "heading": "Rust Basics", "year": 2024,
             "content": "Rust is a safe systems programming language", "embedding": [43.0, 1.0]},
            {"_id": 2, "doc_id": "d1", "chunk_index": 1, "title": "Rust Handbook", "heading": "Ownership in Rust", "year": 2024,
             "content": "Ownership and borrowing are the core of Rust memory safety rules", "embedding": [64.0, 1.0]},
            {"_id": 3, "doc_id": "d2", "chunk_index": 0, "title": "Python Guide", "heading": "Python Guide", "year": 2023,
             "content": "Python is a dynamic language that is easy to learn for beginners", "embedding": [63.0, 1.0]},
            {"_id": 4, "doc_id": "d3", "chunk_index": 0, "title": "Embedded Databases", "heading": "Embedded Databases", "year": 2025,
             "content": "An embedded database written in Rust with fast search", "embedding": [54.0, 1.0]},
            {"_id": 5, "doc_id": "d4", "chunk_index": 0, "title": "Gardening", "heading": "Gardening", "year": 2022,
             "content": "Planting tomatoes in spring", "embedding": [27.0, 1.0]},
            {"_id": 6, "doc_id": "d5", "chunk_index": 0, "title": "Rust Without Body", "heading": "Rust Without Body", "year": 2021,
             "embedding": [2.0, 1.0]},
        ]}),
    );
    for args in [
        json!({"type": "fulltext", "collection": "kb", "field": "content", "language": "english"}),
        json!({"type": "fulltext", "collection": "kb", "field": "title", "language": "english"}),
        json!({"type": "fuzzy", "collection": "kb", "field": "heading"}),
        json!({"type": "vector", "collection": "kb", "field": "embedding", "dim": 2}),
    ] {
        dispatch_ok(adapter, "index_create", args);
    }
    dispatch_ok(
        adapter,
        "insert_one",
        json!({"collection": "odd", "document": {"_id": 1, "doc_id": 7, "content": "a rust chunk with a numeric doc id"}}),
    );
    dispatch_ok(
        adapter,
        "index_create",
        json!({"type": "fulltext", "collection": "odd", "field": "content", "language": "english"}),
    );
    let big: Vec<Value> = (0..61)
        .map(|i| {
            let filler = vec!["filler"; 29 + i].join(" ");
            json!({"_id": i + 1, "doc_id": format!("b{i:02}"), "content": format!("needle {filler}")})
        })
        .collect();
    dispatch_ok(
        adapter,
        "insert_many",
        json!({"collection": "big", "documents": big}),
    );
    dispatch_ok(
        adapter,
        "index_create",
        json!({"type": "fulltext", "collection": "big", "field": "content", "language": "english"}),
    );
    dispatch_ok(
        adapter,
        "insert_many",
        json!({"collection": "clash", "documents": clash_documents()}),
    );
    for args in [
        json!({"type": "fulltext", "collection": "clash", "field": "content", "language": "english"}),
        json!({"type": "fuzzy", "collection": "clash", "field": "heading"}),
        json!({"type": "vector", "collection": "clash", "field": "embedding", "dim": 2}),
    ] {
        dispatch_ok(adapter, "index_create", args);
    }
}

/// "clash": one-chunk documents (`_id` n, `doc_id` "cn") whose own fields are
/// named like the engine metadata of the search hits (`_highlights`, `_score`,
/// `_matched_tokens`, `_highlight`, `_matched_value`) and of the `search`
/// documents (`relevance`, `passages`); seeded after the other collections, so
/// the earlier cases do not see them
fn clash_documents() -> Value {
    json!([
        {"_id": 1, "doc_id": "c1", "heading": "Clash", "embedding": [1.0, 1.0],
         "content": "rust highlights", "_highlights": 5},
        {"_id": 2, "doc_id": "c2", "heading": "Clash Two", "embedding": [3.0, 1.0],
         "content": "rust overwritten", "_score": "own", "_matched_tokens": "own",
         "_highlight": "own", "_matched_value": "own"},
        {"_id": 3, "doc_id": "c3", "heading": "Other", "embedding": [9.0, 1.0],
         "content": "rust relevance", "relevance": "own", "passages": "own"},
    ])
}

/// The metadata-named fields of each "clash" document, by `_id`
const CLASH_OWN_KEYS: &[(i64, &[&str])] = &[
    (1, &["_highlights"]),
    (
        2,
        &["_score", "_matched_tokens", "_highlight", "_matched_value"],
    ),
    (3, &["relevance", "passages"]),
];

/// The search tool group (`fulltext_search`, `fulltext_analyze`,
/// `fuzzy_search`, `vector_search`, `search`) on its own database. `search` runs
/// with [`LengthProvider`] (hybrid) and without an embedding manager (BM25-only,
/// `degraded`).
fn render_search_group(out: &mut String) {
    let (adapter, _dir) = create_test_adapter();
    seed_search_group(&adapter);
    let mut manager = EmbeddingManager::new();
    manager.register_provider("length", Arc::new(LengthProvider));
    let manager = Some(Arc::new(manager));
    let mut local = String::new();
    macro_rules! case {
        ($manager:expr, $tool:literal, $($label:literal => $json:tt),+ $(,)?) => {
            $( record_with(&mut local, &adapter, None, $manager, $tool, $label, json!($json)); )+
        };
    }

    case!(&None, "fulltext_search",
        "or mode (default)" => {"collection": "kb", "field": "content", "query": "rust language"},
        "field defaults to content" => {"collection": "kb", "query": "tomatoes"},
        "explicit or mode" => {"collection": "kb", "field": "content", "query": "rust python", "mode": "or"},
        "and mode, document scope" => {"collection": "kb", "field": "content", "query": "safe ownership", "mode": "and"},
        "and mode, chunk scope" => {"collection": "kb", "field": "content", "query": "safe ownership", "mode": "and", "match_scope": "chunk"},
        "and mode, explicit document scope" => {"collection": "kb", "field": "content", "query": "rust search", "mode": "and", "match_scope": "document"},
        "chunk scope in or mode" => {"collection": "kb", "field": "content", "query": "rust", "match_scope": "chunk"},
        "invalid match_scope" => {"collection": "kb", "field": "content", "query": "rust", "match_scope": "page"},
        "multi-field" => {"collection": "kb", "fields": ["content", "title"], "query": "rust guide"},
        "multi-field and mode" => {"collection": "kb", "fields": ["content", "title"], "query": "python guide", "mode": "and"},
        "highlight" => {"collection": "kb", "field": "content", "query": "rust", "highlight": true},
        "highlight context and snippets" => {"collection": "kb", "field": "content", "query": "rust safe", "highlight": true, "highlight_context": 8, "highlight_max_snippets": 1},
        "min_score" => {"collection": "kb", "field": "content", "query": "rust language", "min_score": 1.0},
        "filter" => {"collection": "kb", "field": "content", "query": "rust", "filter": {"year": {"$gte": 2025}}},
        "empty filter" => {"collection": "kb", "field": "content", "query": "rust", "filter": {}},
        "limit" => {"collection": "kb", "field": "content", "query": "rust", "limit": 1},
        "limit and skip" => {"collection": "kb", "field": "content", "query": "rust", "limit": 1, "skip": 1},
        "projection" => {"collection": "kb", "field": "content", "query": "rust", "projection": {"title": 1}},
        "no match" => {"collection": "kb", "field": "content", "query": "quantum"},
        "empty query" => {"collection": "kb", "field": "content", "query": ""},
        "field without index" => {"collection": "kb", "field": "year", "query": "rust"},
        "missing collection" => {"collection": "nope", "field": "content", "query": "rust"},
        "missing query" => {"collection": "kb", "field": "content"},
    );
    case!(&None, "fulltext_analyze",
        "defaults" => {"text": "The Árvíztűrő tükörfúrógép"},
        "hungarian" => {"text": "A királyok és a királynők", "language": "hungarian"},
        "english, no folding, min length 4" => {"text": "The running dogs ran", "language": "en", "accent_folding": false, "min_word_length": 4},
        "german" => {"text": "Die Häuser und die Bäume", "language": "german"},
        "inherited from index" => {"text": "Running Rust programs", "collection": "kb", "field": "content"},
        "punctuation only" => {"text": "-- !!"},
        "collection without field" => {"text": "x", "collection": "kb"},
        "field without collection" => {"text": "x", "field": "content"},
        "index missing" => {"text": "x", "collection": "kb", "field": "year"},
        "missing text" => {"language": "english"},
    );
    case!(&None, "fuzzy_search",
        "default algorithm" => {"collection": "kb", "field": "heading", "query": "Rust Basic"},
        "levenshtein" => {"collection": "kb", "field": "heading", "query": "Pyton Guide", "algorithm": "levenshtein", "threshold": 0.5},
        "damerau_levenshtein" => {"collection": "kb", "field": "heading", "query": "Gardenign", "algorithm": "damerau_levenshtein", "threshold": 0.7},
        "jaro_winkler threshold" => {"collection": "kb", "field": "heading", "query": "Embeded Databases", "algorithm": "jaro_winkler", "threshold": 0.9},
        "unknown algorithm" => {"collection": "kb", "field": "heading", "query": "Gardening", "algorithm": "soundex"},
        "low threshold" => {"collection": "kb", "field": "heading", "query": "Rust", "threshold": 0.3},
        "filter" => {"collection": "kb", "field": "heading", "query": "Rust", "threshold": 0.3, "filter": {"year": 2024}},
        "highlight" => {"collection": "kb", "field": "heading", "query": "Gardenin", "highlight": true},
        "limit and skip" => {"collection": "kb", "field": "heading", "query": "Rust", "threshold": 0.3, "limit": 1, "skip": 1},
        "projection" => {"collection": "kb", "field": "heading", "query": "Python Guid", "projection": {"heading": 1, "year": 1}},
        "no match" => {"collection": "kb", "field": "heading", "query": "zzzz"},
        "field without index" => {"collection": "kb", "field": "content", "query": "rust"},
        "missing collection" => {"collection": "nope", "field": "heading", "query": "x"},
        "missing query" => {"collection": "kb", "field": "heading"},
    );
    case!(&None, "vector_search",
        "default limit" => {"collection": "kb", "vector": [60.0, 1.0]},
        "limit" => {"collection": "kb", "field": "embedding", "vector": [60.0, 1.0], "limit": 2},
        "include projection" => {"collection": "kb", "field": "embedding", "vector": [30.0, 1.0], "limit": 2, "projection": {"title": 1}},
        "include projection without _id" => {"collection": "kb", "field": "embedding", "vector": [30.0, 1.0], "limit": 1, "projection": {"title": 1, "_id": 0}},
        "exclude projection" => {"collection": "kb", "field": "embedding", "vector": [2.0, 1.0], "limit": 2, "projection": {"embedding": 0, "content": 0}},
        "opposite vector" => {"collection": "kb", "field": "embedding", "vector": [-1.0, 0.0], "limit": 2},
        "wrong dimension" => {"collection": "kb", "field": "embedding", "vector": [1.0, 2.0, 3.0]},
        "field without index" => {"collection": "kb", "field": "title", "vector": [1.0, 2.0]},
        "missing collection" => {"collection": "nope", "vector": [1.0, 2.0]},
        "missing vector" => {"collection": "kb"},
        "filter is not an argument" => {"collection": "kb", "vector": [60.0, 1.0], "limit": 1, "filter": {"year": 2023}},
    );
    case!(&manager, "search",
        "structured" => {"collection": "kb", "query": "rust"},
        "context_block" => {"collection": "kb", "query": "rust", "format": "context_block"},
        "debug" => {"collection": "kb", "query": "rust memory", "debug": true},
        "debug context_block" => {"collection": "kb", "query": "python", "format": "context_block", "debug": true},
        "filter" => {"collection": "kb", "query": "rust", "filter": {"year": 2025}},
        "empty filter" => {"collection": "kb", "query": "rust", "filter": {}},
        "limit" => {"collection": "kb", "query": "rust", "limit": 1},
        "no lexical match" => {"collection": "kb", "query": "quantum"},
        "mechanism parameter rejected" => {"collection": "kb", "query": "rust", "rrf_k": 60},
        "projection rejected" => {"collection": "kb", "query": "rust", "projection": {"title": 1}},
        "invalid format" => {"collection": "kb", "query": "rust", "format": "markdown"},
        "empty query" => {"collection": "kb", "query": ""},
        "missing query" => {"collection": "kb"},
        "not an object" => ["kb", "rust"],
        "missing collection" => {"collection": "nope", "query": "rust"},
        "invalid collection name" => {"collection": "a$b", "query": "rust"},
    );
    case!(&None, "search",
        "without embedding provider" => {"collection": "kb", "query": "rust", "debug": true},
        "without embedding provider, context_block" => {"collection": "kb", "query": "python", "format": "context_block"},
        "numeric doc_id field" => {"collection": "odd", "query": "rust"},
        "numeric doc_id field, context_block" => {"collection": "odd", "query": "rust", "format": "context_block"},
        "context budget" => {"collection": "big", "query": "needle", "limit": 70, "debug": true},
    );
    // A document's own fields named like the metadata ("clash")
    case!(&None, "fulltext_search",
        "own _highlights, no highlight" => {"collection": "clash", "query": "highlights"},
        "own _highlights, highlight" => {"collection": "clash", "query": "highlights", "highlight": true},
        "own _score and _matched_tokens" => {"collection": "clash", "query": "overwritten"},
        "own fields, projection" => {"collection": "clash", "query": "rust", "projection": {"_highlights": 1, "_score": 1, "_matched_tokens": 1}},
    );
    case!(&None, "fuzzy_search",
        "own _highlight, no highlight" => {"collection": "clash", "field": "heading", "query": "Clash Two"},
        "own _highlight, highlight" => {"collection": "clash", "field": "heading", "query": "Clash Two", "highlight": true},
    );
    case!(&None, "vector_search",
        "own _score, no projection" => {"collection": "clash", "vector": [3.0, 1.0], "limit": 3},
        "own _score, include projection" => {"collection": "clash", "vector": [3.0, 1.0], "limit": 3, "projection": {"_score": 1, "heading": 1}},
    );
    case!(&None, "search",
        "own relevance and passages" => {"collection": "clash", "query": "relevance"},
        "own relevance and passages, context_block" => {"collection": "clash", "query": "relevance", "format": "context_block"},
    );
    out.push_str(&local);
}

/// The embedding tools (`embed_text`, `embed_batch`, `embed_models_list`,
/// `embed_cache_stats`, `embed_cache_clear`) and the auto-embedding
/// configuration tools (`auto_embed_enable`, `auto_embed_disable`,
/// `auto_embed_status`), embedding with [`LengthProvider`]: "length" (the
/// default) and "other", without a cache; a second manager with a cache
fn render_embedding_group(out: &mut String) {
    let (adapter, _dir) = create_test_adapter();
    let mut manager = EmbeddingManager::new();
    manager.register_provider("length", Arc::new(LengthProvider));
    manager.register_provider("other", Arc::new(LengthProvider));
    let manager = Some(Arc::new(manager));
    let mut cached = EmbeddingManager::with_cache(CacheConfig {
        max_entries: 100,
        ttl: None,
    });
    cached.register_provider("length", Arc::new(LengthProvider));
    let cached = Some(Arc::new(cached));
    let empty = Some(Arc::new(EmbeddingManager::new()));
    let jobs = Some(Arc::new(JobManager::new()));
    macro_rules! case {
        ($m:expr, $tool:literal, $($label:literal => $json:tt),+ $(,)?) => {
            $( record_full(out, &adapter, None, $m, &None, $tool, $label, json!($json)); )+
        };
    }

    case!(&manager, "embed_text",
        "default provider" => {"text": "hello"},
        "explicit provider" => {"text": "hi", "provider": "other"},
        "unknown provider" => {"text": "hi", "provider": "nope"},
        "embedding fails" => {"text": "fail"},
        "missing text" => {},
    );
    case!(&None, "embed_text", "no embedding manager" => {"text": "hello"});
    case!(&empty, "embed_text", "no provider" => {"text": "hello"});
    case!(&manager, "embed_batch",
        "two texts" => {"texts": ["a", "bcd"]},
        "explicit provider" => {"texts": ["xy"], "provider": "other"},
        "empty" => {"texts": []},
        "embedding fails" => {"texts": ["ok", "fail"]},
    );
    case!(&manager, "embed_models_list", "two providers" => {});
    case!(&empty, "embed_models_list", "no provider" => {});
    case!(&None, "embed_models_list", "no embedding manager" => {});
    case!(&manager, "embed_cache_stats", "cache disabled" => {});
    case!(&cached, "embed_cache_stats", "empty cache" => {});
    case!(&cached, "embed_text", "cached miss" => {"text": "cache me"});
    case!(&cached, "embed_text", "cached hit" => {"text": "cache me"});
    case!(&cached, "embed_cache_stats", "one hit one miss" => {});
    case!(&manager, "embed_cache_clear", "cache disabled" => {});
    case!(&cached, "embed_cache_clear", "one entry" => {});
    case!(&cached, "embed_cache_stats", "after clear" => {});

    // auto-embedding: "plain" has no documents, "docs" two without a vector,
    // "vecs" one with a vector of the old space, "chunks" a chunk document
    dispatch_ok(
        &adapter,
        "collection_create",
        json!({"collection": "plain"}),
    );
    dispatch_ok(
        &adapter,
        "insert_many",
        json!({"collection": "docs", "documents": [
            {"_id": 1, "content": "abc"}, {"_id": 2, "content": "de"},
        ]}),
    );
    dispatch_ok(
        &adapter,
        "insert_one",
        json!({"collection": "vecs", "document": {"_id": 1, "content": "abc", "embedding": [3.0, 1.0]}}),
    );
    dispatch_ok(
        &adapter,
        "insert_one",
        json!({"collection": "chunks", "document": {"_id": 1, "content": "abc", "section_path": "A > B"}}),
    );
    adapter
        .set_auto_embedding_config(
            "legacy",
            Some(ironbase_core::storage::AutoEmbeddingConfig {
                enabled: true,
                source_field: "text".to_string(),
                target_field: "vec".to_string(),
                provider: "fasttext".to_string(),
                ..Default::default()
            }),
        )
        .unwrap();

    case!(&manager, "auto_embed_status",
        "not configured" => {"collection": "plain"},
        "removed provider" => {"collection": "legacy"},
        "missing collection param" => {},
    );
    case!(&manager, "auto_embed_enable",
        "empty collection" => {"collection": "plain", "source_field": "content", "target_field": "embedding"},
        "explicit provider, chunking" => {"collection": "plain", "source_field": "body", "target_field": "body_vec",
            "provider": "other", "skip_if_exists": false,
            "chunking": {"mode": "paragraph", "chunk_size": 200, "overlap": 20}},
        "chunk documents" => {"collection": "chunks", "source_field": "content", "target_field": "embedding"},
        "dimension mismatch" => {"collection": "plain", "source_field": "content", "target_field": "embedding", "dimension": 5},
        "same source and target" => {"collection": "plain", "source_field": "content", "target_field": "content"},
        "missing source_field" => {"collection": "plain", "target_field": "embedding"},
    );
    // One provider: the error lists the available ones in hash-map order
    case!(&cached, "auto_embed_enable",
        "unknown provider" => {"collection": "plain", "source_field": "content", "target_field": "embedding", "provider": "nope"},
    );
    case!(&None, "auto_embed_enable",
        "no embedding manager" => {"collection": "plain", "source_field": "content", "target_field": "embedding"},
    );
    case!(&manager, "auto_embed_status",
        "enabled" => {"collection": "plain"},
        "chunk documents" => {"collection": "chunks"},
    );
    // A backfill of "docs" (no job manager: no job is started)
    case!(&manager, "auto_embed_enable",
        "existing documents" => {"collection": "docs", "source_field": "content", "target_field": "embedding"},
    );
    // A re-embed of "vecs" into the space of another model name
    adapter
        .set_auto_embedding_config(
            "vecs",
            Some(ironbase_core::storage::AutoEmbeddingConfig {
                enabled: true,
                source_field: "content".to_string(),
                target_field: "embedding".to_string(),
                provider: "length".to_string(),
                model: Some("older".to_string()),
                dimension: Some(2),
                ..Default::default()
            }),
        )
        .unwrap();
    case!(&manager, "auto_embed_enable",
        "other model, existing vectors" => {"collection": "vecs", "source_field": "content", "target_field": "embedding"},
    );
    case!(&manager, "auto_embed_status", "re-embed pending" => {"collection": "vecs"});

    // A backfill job, run to its end before the next case
    dispatch_ok(
        &adapter,
        "insert_one",
        json!({"collection": "jobbed", "document": {"_id": 1, "content": "abc"}}),
    );
    record_full(
        out,
        &adapter,
        None,
        &manager,
        &jobs,
        "auto_embed_enable",
        "backfill job started",
        json!({"collection": "jobbed", "source_field": "content", "target_field": "embedding"}),
    );
    let job_manager = jobs.as_ref().unwrap();
    wait_for_job(job_manager, "job_1");

    // A re-embed with a job manager: the message comes before the job id
    // (the server inserts them in that order)
    dispatch_ok(
        &adapter,
        "insert_one",
        json!({"collection": "rejobbed", "document": {"_id": 1, "content": "abc", "embedding": [3.0, 1.0]}}),
    );
    adapter
        .set_auto_embedding_config(
            "rejobbed",
            Some(ironbase_core::storage::AutoEmbeddingConfig {
                enabled: true,
                source_field: "content".to_string(),
                target_field: "embedding".to_string(),
                provider: "length".to_string(),
                model: Some("older".to_string()),
                dimension: Some(2),
                ..Default::default()
            }),
        )
        .unwrap();
    record_full(
        out,
        &adapter,
        None,
        &manager,
        &jobs,
        "auto_embed_enable",
        "re-embed job started",
        json!({"collection": "rejobbed", "source_field": "content", "target_field": "embedding"}),
    );
    wait_for_job(job_manager, "job_2");
    job_manager.shutdown();

    case!(&manager, "auto_embed_disable",
        "enabled" => {"collection": "plain"},
        "not configured" => {"collection": "empty_one"},
        "missing collection param" => {},
    );
    case!(&manager, "auto_embed_status", "after disable" => {"collection": "plain"});
}

/// Waits (at most 30 s) until the job `id` has finished
fn wait_for_job(job_manager: &JobManager, id: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while job_manager
        .get_job_info(id)
        .is_some_and(|info| !info.status.is_terminal())
    {
        assert!(
            std::time::Instant::now() < deadline,
            "job {id} did not finish"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// The job tools (`embed_job_list`, `embed_job_status`, `embed_job_cancel`) on
/// jobs set up directly in a [`JobManager`]: pending, running, completed,
/// failed and cancelled, of every job type
fn render_job_group(out: &mut String) {
    let (adapter, _dir) = create_test_adapter();
    let manager = JobManager::new();
    let (pending, _) = manager.create_job(JobType::Compact);
    let (running, _) = manager.create_job(JobType::EmbedBackfill {
        collection: "kb".to_string(),
        provider: "length".to_string(),
    });
    manager.update_progress(&running, 1, Some(4), "Embedding batch 1");
    let (completed, _) = manager.create_job(JobType::EmbedDocument {
        collection: "kb".to_string(),
        doc_id: "d1".to_string(),
    });
    manager.complete_job(&completed, json!({"processed": 3, "errors": 0}));
    let (failed, _) = manager.create_job(JobType::Custom {
        name: "x".to_string(),
    });
    manager.fail_job(&failed, "Provider 'nope' not available".to_string());
    let (cancelled, _) = manager.create_job(JobType::Compact);
    manager.cancel_job(&cancelled);
    let (to_cancel, _) = manager.create_job(JobType::Compact);
    manager.update_progress(&to_cancel, 0, None, "Starting...");
    let jobs = Some(Arc::new(manager));
    macro_rules! case {
        ($m:expr, $tool:literal, $($label:literal => $json:tt),+ $(,)?) => {
            $( record_full(out, &adapter, None, &None, $m, $tool, $label, json!($json)); )+
        };
    }

    case!(&jobs, "embed_job_list",
        "all" => {},
        "active only" => {"active_only": true},
    );
    for (label, id) in [
        ("pending", &pending),
        ("running", &running),
        ("completed", &completed),
        ("failed", &failed),
        ("cancelled", &cancelled),
    ] {
        record_full(
            out,
            &adapter,
            None,
            &None,
            &jobs,
            "embed_job_status",
            label,
            json!({"job_id": id}),
        );
    }
    case!(&jobs, "embed_job_status",
        "unknown" => {"job_id": "job_999"},
        "missing job_id" => {},
    );
    record_full(
        out,
        &adapter,
        None,
        &None,
        &jobs,
        "embed_job_cancel",
        "running",
        json!({"job_id": to_cancel}),
    );
    case!(&jobs, "embed_job_cancel",
        "completed" => {"job_id": "job_3"},
        "already cancelled" => {"job_id": "job_5"},
        "unknown" => {"job_id": "job_999"},
        "missing job_id" => {},
    );
    case!(&None, "embed_job_list", "no job manager" => {});
    case!(&None, "embed_job_status", "no job manager" => {"job_id": "job_1"});
    case!(&None, "embed_job_cancel", "no job manager" => {"job_id": "job_1"});
}

/// The RAG tools (`rag_collection_create`, `rag_document_import`,
/// `rag_collection_stats`, `rag_chunks_load`) and `embed_document`, embedding
/// with [`LengthProvider`] ("length", the default)
fn render_rag_group(out: &mut String) {
    let (adapter, _dir) = create_test_adapter();
    let mut manager = EmbeddingManager::new();
    manager.register_provider("length", Arc::new(LengthProvider));
    let manager = Some(Arc::new(manager));
    macro_rules! case {
        ($m:expr, $tool:literal, $($label:literal => $json:tt),+ $(,)?) => {
            $( record_full(out, &adapter, None, $m, &None, $tool, $label, json!($json)); )+
        };
    }
    dispatch_ok(
        &adapter,
        "collection_create",
        json!({"collection": "plain"}),
    );
    let text = "Első bekezdés a királyról és a várról.\n\nMásodik bekezdés a folyóról és a hídról.\n\nHarmadik bekezdés a hegyről.";

    case!(&manager, "rag_collection_create",
        "defaults" => {"collection": "kb"},
        "fields and context" => {"collection": "kb2", "embedding_field": "vec", "text_field": "body",
            "text_fields": ["body", "title"], "context_fields": ["title"], "provider": "length",
            "language": "english"},
        "again" => {"collection": "kb"},
        "unknown provider" => {"collection": "kb3", "provider": "nope"},
        "invalid name" => {"collection": "bad name!"},
        "missing collection" => {},
    );
    case!(&None, "rag_collection_create", "no embedding manager" => {"collection": "kb4"});
    case!(&manager, "rag_document_import",
        "chunked" => {"collection": "kb", "content": text, "doc_id": "d1", "title": "Mesék",
            "chunk_size": 60, "overlap": 0, "mode": "text"},
        "metadata" => {"collection": "kb", "content": "Rövid szöveg a tengerről.", "doc_id": "d2",
            "metadata": {"year": 2026}},
        "skip existing" => {"collection": "kb", "content": "Más.", "doc_id": "d1", "if_exists": "skip"},
        "replace existing" => {"collection": "kb", "content": "Rövid szöveg a tengerről, újra.",
            "doc_id": "d2", "if_exists": "replace"},
        "error existing" => {"collection": "kb", "content": "Más.", "doc_id": "d1", "if_exists": "error"},
        "generated doc_id" => {"collection": "kb", "content": "Egy harmadik dokumentum."},
        "language ignored" => {"collection": "kb", "content": "Negyedik.", "doc_id": "d4", "language": "english"},
        "whitespace only" => {"collection": "kb", "content": "   ", "doc_id": "d5"},
        "empty content" => {"collection": "kb", "content": "", "doc_id": "d6"},
        "unknown provider" => {"collection": "kb", "content": "x", "doc_id": "d7", "provider": "nope"},
        "embedding fails" => {"collection": "kb", "content": "fail", "doc_id": "d8"},
        "without RAG config" => {"collection": "plain", "content": "Sima szöveg.", "doc_id": "p1"},
    );
    case!(&None, "rag_document_import", "no embedding manager" => {"collection": "kb", "content": "x"});
    case!(&manager, "embed_document",
        "chunked" => {"collection": "docs", "content": text, "doc_id": "e1", "chunk_size": 60,
            "overlap": 0, "mode": "text", "metadata": {"src": "t"}},
        "explicit provider, no index" => {"collection": "docs", "content": "Rövid.", "doc_id": "e2",
            "provider": "length", "create_vector_index": false},
        "skip existing" => {"collection": "docs", "content": "Más.", "doc_id": "e1", "if_exists": "skip"},
        "generated doc_id" => {"collection": "docs", "content": "Harmadik."},
        "whitespace only" => {"collection": "docs", "content": "   ", "doc_id": "e5"},
        "empty content" => {"collection": "docs", "content": ""},
    );
    case!(&manager, "rag_collection_stats",
        "RAG collection" => {"collection": "kb"},
        "fields and context" => {"collection": "kb2"},
        "plain collection" => {"collection": "plain"},
        "missing collection" => {"collection": "nope"},
    );
    case!(&None, "rag_collection_stats", "no embedding manager" => {"collection": "kb"});
    case!(&manager, "rag_chunks_load",
        "one document" => {"collection": "kb", "doc_ids": ["d1"]},
        "unmerged" => {"collection": "kb", "doc_ids": ["d1", "d2"], "merge_chunks": false},
        "capped" => {"collection": "kb", "doc_ids": ["d1"], "merge_chunks": false, "max_chunks_per_doc": 1},
        "projection" => {"collection": "kb", "doc_ids": ["d1", "d2"], "merge_chunks": false,
            "projection": {"chunk_index": 1, "doc_id": 1}},
        "scored" => {"collection": "kb", "doc_ids": ["d1", "d2"], "query": "folyóról", "merge_chunks": false},
        "scored, merged" => {"collection": "kb", "doc_ids": ["d1"], "query": "bekezdés"},
        "no doc_ids" => {"collection": "kb", "doc_ids": []},
        "unknown doc_id" => {"collection": "kb", "doc_ids": ["zz"]},
        "missing doc_ids" => {"collection": "kb"},
    );
}

/// The ACL tools (`acl_list`, `acl_get`, `acl_set`, `acl_delete`,
/// `acl_cleanup`) and the listener tools (`listener_list`, `listener_get`,
/// `listener_create`, `listener_delete`, `listener_enable`,
/// `listener_disable`), on a sixth database
fn render_acl_listener_group(out: &mut String) {
    let (adapter, _dir) = create_test_adapter();
    for name in ["users", "temp"] {
        dispatch_ok(&adapter, "collection_create", json!({"collection": name}));
    }
    macro_rules! case {
        ($tool:literal, $($label:literal => $json:tt),+ $(,)?) => {
            $( record(out, &adapter, None, $tool, $label, json!($json)); )+
        };
    }

    case!("acl_list", "none" => {});
    case!("acl_get", "none" => {"collection": "users"});
    case!("acl_set",
        "every principal" => {"collection": "users", "rules": [
            {"principal": "apikey:abc", "permissions": "read"},
            {"principal": "interface:localhost", "permissions": "admin"},
            {"principal": "ip:10.0.0.5", "permissions": "write"},
            {"principal": "iprange:10.0.0.0/8", "permissions": "none"},
            {"principal": "anyone:x", "permissions": "read,write"},
        ]},
        "wildcard" => {"collection": "*", "rules": [{"principal": "interface:external", "permissions": "deny"}]},
        "orphan to be" => {"collection": "temp", "rules": [{"principal": "anyone:x", "permissions": "all"}]},
        "missing collection" => {"collection": "nope", "rules": []},
        "bad principal" => {"collection": "users", "rules": [{"principal": "who", "permissions": "read"}]},
        "unknown principal type" => {"collection": "users", "rules": [{"principal": "user:bob", "permissions": "read"}]},
        "bad ip" => {"collection": "users", "rules": [{"principal": "ip:1.2.3", "permissions": "read"}]},
        "missing permissions" => {"collection": "users", "rules": [{"principal": "apikey:abc"}]},
        "missing rules" => {"collection": "users"},
        "empty rules" => {"collection": "users", "rules": []},
    );
    case!("acl_get",
        "set" => {"collection": "users"},
        "wildcard" => {"collection": "*"},
        "missing collection param" => {},
    );
    case!("acl_list", "three" => {});
    // Dropping a collection drops its ACL; an orphan is a rule left behind
    dispatch_ok(&adapter, "collection_drop", json!({"collection": "temp"}));
    case!("acl_list", "after collection drop" => {});
    adapter
        .insert_one(
            mcp_ironbase::acl::SYSTEM_ACL_COLLECTION,
            json!({"collection": "gone", "rules": [
                {"principal": {"type": "anyone"}, "permissions": {"read": true, "write": false, "admin": false}}
            ]}),
        )
        .unwrap();
    case!("acl_cleanup", "one orphan" => {});
    case!("acl_cleanup", "none" => {});
    case!("acl_delete",
        "set" => {"collection": "users"},
        "not set" => {"collection": "users"},
        "builtin" => {"collection": "_system.*"},
        "missing collection param" => {},
    );

    case!("listener_list", "none" => {});
    case!("listener_create",
        "http" => {"id": "web", "bind": "127.0.0.1:9090", "description": "Web"},
        "https" => {"id": "tls", "bind": "0.0.0.0:9443", "tls": true,
            "cert_path": "/etc/c.pem", "key_path": "/etc/k.pem"},
        "update" => {"id": "web", "bind": "127.0.0.1:9091"},
        "tls without cert" => {"id": "bad", "bind": "0.0.0.0:9444", "tls": true},
        "bad bind" => {"id": "bad2", "bind": "nowhere"},
        "missing bind" => {"id": "bad3"},
    );
    case!("listener_get",
        "http" => {"id": "web"},
        "https" => {"id": "tls"},
        "missing" => {"id": "nope"},
        "missing id param" => {},
    );
    case!("listener_disable",
        "existing" => {"id": "web"},
        "missing" => {"id": "nope"},
    );
    case!("listener_get", "disabled" => {"id": "web"});
    case!("listener_enable",
        "existing" => {"id": "web"},
        "missing" => {"id": "nope"},
    );
    case!("listener_list", "two" => {});
    case!("listener_delete",
        "existing" => {"id": "tls"},
        "missing" => {"id": "nope"},
        "default" => {"id": "default"},
        "missing id param" => {},
    );
}

#[test]
fn response_wire_format_matches_golden() {
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

/// Parses a recorded response, strictly, into the tool's shared response type
/// and checks that it serializes back to the same bytes
fn same_bytes<T: ironbase_mcp_types::Tool>(json: &str) -> Result<(), String> {
    let parsed: T::Resp = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let back = serde_json::to_string(&parsed).unwrap();
    assert_eq!(back, json, "{} does not round-trip", T::NAME);
    Ok(())
}

/// Runs `check` on every recorded successful response of the golden file;
/// returns how many it checked (`check` returns `None` for other tools)
fn check_golden(check: impl Fn(&str, &str) -> Option<Result<(), String>>) -> usize {
    let mut checked = 0;
    let mut lines = GOLDEN.lines();
    while let Some(line) = lines.next() {
        let Some(header) = line.strip_prefix("## ") else {
            continue;
        };
        let (tool, label) = header.split_once(" / ").unwrap();
        lines.next(); // in:
        let Some(json) = lines.next().unwrap().strip_prefix("out: ") else {
            continue; // an error
        };
        if let Some(parsed) = check(tool, json) {
            parsed.unwrap_or_else(|e| panic!("{tool} / {label}: {e}"));
            checked += 1;
        }
    }
    checked
}

/// Every recorded response of the search tool group parses, strictly, into the
/// tool's shared response type and serializes back to the same bytes,
/// including a one-chunk `search` document whose own `doc_id` field is a number
/// and the "clash" documents' own fields named like the metadata.
#[test]
fn search_group_golden_parses_as_the_shared_response_types() {
    use ironbase_mcp_types::tools::{
        FulltextAnalyze, FulltextSearch, FuzzySearch, Search, VectorSearch,
    };

    let checked = check_golden(|tool, json| {
        Some(match tool {
            "fulltext_search" => same_bytes::<FulltextSearch>(json),
            "fulltext_analyze" => same_bytes::<FulltextAnalyze>(json),
            "fuzzy_search" => same_bytes::<FuzzySearch>(json),
            "vector_search" => same_bytes::<VectorSearch>(json),
            "search" => same_bytes::<Search>(json),
            _ => return None,
        })
    });
    assert_eq!(checked, 65);
}

/// Every recorded response of the embedding tool group (embedding,
/// auto-embedding and job tools) parses, strictly, into the tool's shared
/// response type and serializes back to the same bytes
#[test]
fn embedding_group_golden_parses_as_the_shared_response_types() {
    use ironbase_mcp_types::tools::{
        AutoEmbedDisable, AutoEmbedEnable, AutoEmbedStatus, EmbedBatch, EmbedCacheClear,
        EmbedCacheStats, EmbedJobCancel, EmbedJobList, EmbedJobStatus, EmbedModelsList, EmbedText,
    };

    let checked = check_golden(|tool, json| {
        // The job ages are normalised to a string placeholder
        let json_owned;
        let json = if tool.starts_with("embed_job_") {
            json_owned = json
                .replace("\"<created_at_secs>\"", "0")
                .replace("\"<updated_at_secs>\"", "0");
            json_owned.as_str()
        } else {
            json
        };
        Some(match tool {
            "embed_text" => same_bytes::<EmbedText>(json),
            "embed_batch" => same_bytes::<EmbedBatch>(json),
            "embed_models_list" => same_bytes::<EmbedModelsList>(json),
            "embed_cache_stats" => same_bytes::<EmbedCacheStats>(json),
            "embed_cache_clear" => same_bytes::<EmbedCacheClear>(json),
            "auto_embed_enable" => same_bytes::<AutoEmbedEnable>(json),
            "auto_embed_disable" => same_bytes::<AutoEmbedDisable>(json),
            "auto_embed_status" => same_bytes::<AutoEmbedStatus>(json),
            "embed_job_list" => same_bytes::<EmbedJobList>(json),
            "embed_job_status" => same_bytes::<EmbedJobStatus>(json),
            "embed_job_cancel" => same_bytes::<EmbedJobCancel>(json),
            _ => return None,
        })
    });
    assert_eq!(checked, 41);
}

/// Every recorded response of the RAG tool group parses, strictly, into the
/// tool's shared response type and serializes back to the same bytes
#[test]
fn rag_group_golden_parses_as_the_shared_response_types() {
    use ironbase_mcp_types::tools::{
        EmbedDocument, RagChunksLoad, RagCollectionCreate, RagCollectionStats, RagDocumentImport,
    };

    let checked = check_golden(|tool, json| {
        Some(match tool {
            "rag_collection_create" => same_bytes::<RagCollectionCreate>(json),
            "rag_document_import" => same_bytes::<RagDocumentImport>(json),
            "embed_document" => same_bytes::<EmbedDocument>(json),
            "rag_collection_stats" => same_bytes::<RagCollectionStats>(json),
            "rag_chunks_load" => same_bytes::<RagChunksLoad>(json),
            _ => return None,
        })
    });
    assert_eq!(checked, 29);
}

/// Every recorded response of the ACL and listener tool group parses,
/// strictly, into the tool's shared response type and serializes back to the
/// same bytes
#[test]
fn acl_listener_group_golden_parses_as_the_shared_response_types() {
    use ironbase_mcp_types::tools::{
        AclCleanup, AclDelete, AclGet, AclList, AclSet, ListenerCreate, ListenerDelete,
        ListenerDisable, ListenerEnable, ListenerGet, ListenerList,
    };

    let checked = check_golden(|tool, json| {
        Some(match tool {
            "acl_list" => same_bytes::<AclList>(json),
            "acl_get" => same_bytes::<AclGet>(json),
            "acl_set" => same_bytes::<AclSet>(json),
            "acl_delete" => same_bytes::<AclDelete>(json),
            "acl_cleanup" => same_bytes::<AclCleanup>(json),
            "listener_list" => same_bytes::<ListenerList>(json),
            "listener_get" => same_bytes::<ListenerGet>(json),
            "listener_create" => same_bytes::<ListenerCreate>(json),
            "listener_delete" => same_bytes::<ListenerDelete>(json),
            "listener_enable" => same_bytes::<ListenerEnable>(json),
            "listener_disable" => same_bytes::<ListenerDisable>(json),
            _ => return None,
        })
    });
    assert_eq!(checked, 27);
}
