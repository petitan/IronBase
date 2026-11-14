//! Wire-format safety net for MCP tool responses (read-side tool group).
//!
//! `collection_list`, `find`, `count`, `index_list`, `index_stats`, `explain` and
//! `db_stats` run through the real `dispatch_tool` against a seeded temporary
//! database; the exact serialized JSON of each response (or the exact error) is
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
//! Regenerate (only for an intended wire-format change):
//! `UPDATE_GOLDEN=1 cargo test --test response_wire_golden`

use mcp_ironbase::{dispatch_tool, IronBaseAdapter, ServerInfo};
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
        &None,
        &None,
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

    out
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
