//! Response structs: the server's JSON (taken from
//! `mcp-server/tests/golden/response_wire.txt`) parses into the tool's `Resp` and
//! serializes back to the same bytes; a missing required field is an error.

use ironbase_mcp_types::responses::*;
use ironbase_mcp_types::tools::{self, Tool};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

/// `text` parses as `T` and serializes back to exactly `text`
fn same_bytes<T: Serialize + DeserializeOwned>(text: &str) -> T {
    let parsed: T = serde_json::from_str(text).unwrap_or_else(|e| panic!("{e}: {text}"));
    assert_eq!(serde_json::to_string(&parsed).unwrap(), text);
    parsed
}

/// `text` with the first occurrence of `"field":<value>` removed (and its comma)
/// does not parse as `T`
fn requires<T: DeserializeOwned + std::fmt::Debug>(text: &str, field: &str) {
    let mut value: Value = serde_json::from_str(text).unwrap();
    fn remove(v: &mut Value, field: &str) -> bool {
        match v {
            Value::Object(map) => {
                map.shift_remove(field).is_some() || map.values_mut().any(|c| remove(c, field))
            }
            Value::Array(items) => items.iter_mut().any(|c| remove(c, field)),
            _ => false,
        }
    }
    assert!(remove(&mut value, field), "no `{field}` in {text}");
    let result = serde_json::from_value::<T>(value);
    assert!(result.is_err(), "missing `{field}` parsed: {result:?}");
}

const VECTOR: &str = r#"{"name":"users_vec_emb","field":"emb","dim":3,"metric":"dotproduct","max_vectors":100000,"vector_count":3,"cache_file":"users_vec_emb.hnsw","created_at":"2026-11-14T10:00:00Z","hnsw_params":{"m":16,"ef_construction":200,"ef_search":50}}"#;
const FULLTEXT: &str = r#"{"name":"users_bio_fts","field":"bio","language":"hungarian","min_word_length":2,"accent_folding":true,"num_documents":3,"num_tokens":6}"#;
const FUZZY: &str = r#"{"name":"users_name_fuzzy","field":"name","algorithm":"jarowinkler","threshold":0.8,"num_entries":4}"#;

#[test]
fn collection_list_find_count() {
    same_bytes::<<tools::CollectionList as Tool>::Resp>(r#"{"collections":["empty","users"]}"#);
    let find = same_bytes::<<tools::Find as Tool>::Resp>(
        r#"{"documents":[{"_id":1,"name":"Álmos","nick":null,"age":25.5,"tags":["a"],"address":{"zip":"9021"}}],"count":1}"#,
    );
    assert_eq!(find.total, None);
    let find = same_bytes::<FindResponse>(r#"{"documents":[],"count":0,"total":0}"#);
    assert_eq!(find.total, Some(0));
    same_bytes::<<tools::Count as Tool>::Resp>(r#"{"count":4}"#);

    requires::<FindResponse>(r#"{"documents":[],"count":0}"#, "count");
    requires::<FindResponse>(r#"{"documents":[],"count":0}"#, "documents");
    requires::<CountResponse>(r#"{"count":4}"#, "count");
}

#[test]
fn index_list_shapes() {
    let all = format!(
        r#"{{"btree_indexes":["users_age","users_id"],"fulltext_indexes":[{FULLTEXT}],"fuzzy_indexes":[{FUZZY}],"vector_indexes":[{VECTOR}],"summary":{{"btree_count":2,"fulltext_count":1,"fuzzy_count":1,"vector_count":1,"total":5}}}}"#
    );
    match same_bytes::<<tools::IndexList as Tool>::Resp>(&all) {
        IndexListResponse::All(all) => {
            assert_eq!(all.fuzzy_indexes[0].name, "users_name_fuzzy");
            assert_eq!(all.vector_indexes[0].metric, VectorMetric::DotProduct);
            assert_eq!(all.vector_indexes[0].dim, 3);
        }
        other => panic!("{other:?}"),
    }
    requires::<IndexListResponse>(&all, "fuzzy_indexes");

    let typed = [
        (
            r#"{"indexes":["users_age"],"count":1}"#.to_string(),
            "btree",
        ),
        (
            format!(r#"{{"indexes":[{FULLTEXT}],"count":1}}"#),
            "fulltext",
        ),
        (format!(r#"{{"indexes":[{FUZZY}],"count":1}}"#), "fuzzy"),
        (format!(r#"{{"indexes":[{VECTOR}],"count":1}}"#), "vector"),
    ];
    for (text, kind) in typed {
        let got = match same_bytes::<IndexListResponse>(&text) {
            IndexListResponse::BTree(_) => "btree",
            IndexListResponse::Fulltext(_) => "fulltext",
            IndexListResponse::Fuzzy(_) => "fuzzy",
            IndexListResponse::Vector(_) => "vector",
            IndexListResponse::All(_) => "all",
        };
        assert_eq!(got, kind, "{text}");
    }
    // An empty typed list has no item to tell its subtype by
    assert!(matches!(
        same_bytes::<IndexListResponse>(r#"{"indexes":[],"count":0}"#),
        IndexListResponse::BTree(_)
    ));
    // `IndexListAll::new` computes the summary the server sends
    let built = IndexListAll::new(vec!["a".into()], vec![], vec![], vec![]);
    assert_eq!(
        serde_json::to_string(&built).unwrap(),
        r#"{"btree_indexes":["a"],"fulltext_indexes":[],"fuzzy_indexes":[],"vector_indexes":[],"summary":{"btree_count":1,"fulltext_count":0,"fuzzy_count":0,"vector_count":0,"total":1}}"#
    );
}

#[test]
fn index_stats_is_tagged_by_type() {
    let btree = r#"{"type":"btree","indexes":[{"name":"users_age","field":"age","num_keys":4,"distinct_count":4,"has_histogram":false,"has_mcv":false}],"count":1}"#;
    assert!(matches!(
        same_bytes::<<tools::IndexStats as Tool>::Resp>(btree),
        IndexStatsResponse::BTree(_)
    ));
    requires::<IndexStatsResponse>(btree, "has_mcv");
    requires::<IndexStatsResponse>(btree, "type");
    // An empty list keeps its subtype
    assert!(matches!(
        same_bytes::<IndexStatsResponse>(r#"{"type":"vector","indexes":[],"count":0}"#),
        IndexStatsResponse::Vector(_)
    ));
    same_bytes::<IndexStatsResponse>(&format!(
        r#"{{"type":"fulltext","indexes":[{FULLTEXT}],"count":1}}"#
    ));
    same_bytes::<IndexStatsResponse>(&format!(
        r#"{{"type":"fuzzy","indexes":[{FUZZY}],"count":1}}"#
    ));
    same_bytes::<IndexStatsResponse>(&format!(
        r#"{{"type":"vector","indexes":[{VECTOR}],"count":1}}"#
    ));
    assert!(serde_json::from_str::<IndexStatsResponse>(
        r#"{"type":"hash","indexes":[],"count":0}"#
    )
    .is_err());
}

#[test]
fn explain_plan_passes_through_and_summarises() {
    let indexed = r#"{"plan":{"chosenPlan":{"queryPlan":"IndexScan","indexUsed":"users_email","field":"email","stage":"FETCH_WITH_INDEX","indexType":"equality","searchKey":"String(\"bea@example.com\")","estimatedCost":"O(log n)"},"selectionReason":"Index users_email on field email","estimatedRows":1.0,"candidates":[],"candidateCount":1}}"#;
    let plan = same_bytes::<<tools::Explain as Tool>::Resp>(indexed);
    let summary = plan.summary().unwrap();
    assert_eq!(summary.chosen_plan.query_plan, "IndexScan");
    assert_eq!(
        summary.chosen_plan.index_used.as_deref(),
        Some("users_email")
    );
    assert_eq!(
        summary.chosen_plan.stage.as_deref(),
        Some("FETCH_WITH_INDEX")
    );
    assert_eq!(
        summary.chosen_plan.estimated_cost.as_deref(),
        Some("O(log n)")
    );
    assert_eq!(summary.estimated_rows, Some(1.0));

    let scan = r#"{"plan":{"chosenPlan":{"queryPlan":"CollectionScan","indexUsed":null,"stage":"FULL_SCAN","estimatedCost":"O(n)"},"selectionReason":"No suitable index found for query","candidates":[],"candidateCount":0,"availableIndexes":[]}}"#;
    let summary = same_bytes::<ExplainResponse>(scan).summary().unwrap();
    assert_eq!(summary.chosen_plan.index_used, None);
    assert_eq!(summary.estimated_rows, None);

    let logical = r#"{"plan":{"chosenPlan":{"queryPlan":"LogicalIndexScan","operator":"$or","clauseCount":2},"selectionReason":"Logical operator planning","clauses":[],"candidateCount":1}}"#;
    let summary = same_bytes::<ExplainResponse>(logical).summary().unwrap();
    assert_eq!(summary.chosen_plan.query_plan, "LogicalIndexScan");
    assert_eq!(summary.chosen_plan.stage, None);

    let no_plan: ExplainResponse = serde_json::from_str(r#"{"plan":{}}"#).unwrap();
    assert!(no_plan.summary().is_err());
    requires::<ExplainResponse>(scan, "plan");
}

#[test]
fn db_stats_http_and_stdio() {
    let body = r#"{"database":{"path":"/data/x.mlite","file_size_bytes":71483,"file_size":"69.81 KB"},"collections":[{"name":"users","document_count":4,"btree_index_count":2,"btree_indexes":["users_age","users_id"],"vector_index_count":1,"vector_indexes":[{"name":"users_vec_emb","field":"emb","dim":3,"metric":"dotproduct","vector_count":3}],"total_vectors":3}],"summary":{"collection_count":1,"total_documents":4,"total_btree_indexes":2,"total_vector_indexes":1,"total_vectors":3}"#;
    let wastage = r#""storage_wastage":{"file_size_bytes":71483,"file_size_mb":0,"estimated_live_bytes":0,"estimated_live_mb":0,"bloat_ratio":"infinity (never compacted)","total_writes":4,"total_live":4,"dead_writes":0}}"#;
    let stdio = format!(r#"{body},"server":{{"version":"1.0.0","mode":"stdio"}},{wastage}"#);
    let http = format!(
        r#"{body},"server":{{"version":"1.0.0","protocol":"http","host":"127.0.0.1","port":8080,"require_api_key":true}},{wastage}"#
    );
    let parsed = same_bytes::<<tools::DbStats as Tool>::Resp>(&stdio);
    assert!(matches!(parsed.server, DbStatsServer::Stdio { .. }));
    let parsed = same_bytes::<DbStatsResponse>(&http);
    assert!(matches!(
        parsed.server,
        DbStatsServer::Http { port: 8080, .. }
    ));
    assert_eq!(parsed.stats.summary.total_btree_indexes, 2);

    requires::<DbStatsResponse>(&http, "total_btree_indexes");
    requires::<DbStatsResponse>(&http, "storage_wastage");
    requires::<DbStatsResponse>(&http, "database");
    // The `ironbase://stats` resource is the statistics alone
    same_bytes::<DatabaseStats>(&format!("{body}}}"));
}

#[test]
fn no_params_is_an_empty_object() {
    use ironbase_mcp_types::params::NoParams;
    assert_eq!(serde_json::to_string(&NoParams {}).unwrap(), "{}");
    let _: NoParams = serde_json::from_str("{}").unwrap();
    assert_eq!(<tools::CollectionList as Tool>::NAME, "collection_list");
    assert_eq!(<tools::DbStats as Tool>::NAME, "db_stats");
}

#[test]
fn document_writes() {
    let one = same_bytes::<<tools::InsertOne as Tool>::Resp>(r#"{"inserted_id":"10"}"#);
    assert_eq!(one.inserted_id, "10");
    assert_eq!(one.auto_embedded, None);
    same_bytes::<InsertOneResponse>(r#"{"inserted_id":"e-1","auto_embedded":true}"#);
    requires::<InsertOneResponse>(r#"{"inserted_id":"10"}"#, "inserted_id");
    // `_id`s are strings on the wire, an integer one too
    assert!(serde_json::from_str::<InsertOneResponse>(r#"{"inserted_id":10}"#).is_err());

    same_bytes::<<tools::InsertMany as Tool>::Resp>(
        r#"{"inserted_ids":["20","s-21","14"],"inserted_count":3}"#,
    );
    same_bytes::<InsertManyResponse>(
        r#"{"inserted_ids":["20","21","22"],"inserted_count":3,"auto_embedded_count":2}"#,
    );
    requires::<InsertManyResponse>(
        r#"{"inserted_ids":[],"inserted_count":0}"#,
        "inserted_count",
    );

    let update =
        same_bytes::<<tools::UpdateOne as Tool>::Resp>(r#"{"matched_count":1,"modified_count":0}"#);
    assert_eq!((update.matched_count, update.modified_count), (1, 0));
    assert_eq!(update.upserted_id, None);
    same_bytes::<UpdateOneResponse>(
        r#"{"matched_count":0,"modified_count":0,"upserted_id":"30","auto_embedded":true}"#,
    );
    same_bytes::<UpdateOneResponse>(
        r#"{"matched_count":1,"modified_count":1,"auto_embedded_count":1}"#,
    );
    same_bytes::<UpdateOneResponse>(
        r#"{"matched_count":1,"modified_count":1,"auto_embed_error":"Embedding generation failed: x"}"#,
    );
    requires::<UpdateOneResponse>(
        r#"{"matched_count":1,"modified_count":0}"#,
        "modified_count",
    );

    same_bytes::<<tools::UpdateMany as Tool>::Resp>(r#"{"matched_count":3,"modified_count":0}"#);
    same_bytes::<UpdateManyResponse>(
        r#"{"matched_count":1,"modified_count":1,"auto_embed_error":"Embedding generation failed: x"}"#,
    );
    requires::<UpdateManyResponse>(r#"{"matched_count":3,"modified_count":0}"#, "matched_count");

    same_bytes::<<tools::DeleteOne as Tool>::Resp>(r#"{"deleted_count":0}"#);
    same_bytes::<<tools::DeleteMany as Tool>::Resp>(r#"{"deleted_count":2}"#);
    requires::<DeleteResponse>(r#"{"deleted_count":2}"#, "deleted_count");
}

#[test]
fn collection_management() {
    same_bytes::<<tools::CollectionCreate as Tool>::Resp>(r#"{"success":true,"collection":"c1"}"#);
    let drop = same_bytes::<<tools::CollectionDrop as Tool>::Resp>(
        r#"{"success":true,"dropped":"drop_me","acl_deleted":false}"#,
    );
    assert_eq!(drop.dropped, "drop_me");
    requires::<CollectionDropResponse>(
        r#"{"success":true,"dropped":"drop_me","acl_deleted":false}"#,
        "acl_deleted",
    );
}

#[test]
fn index_create_shapes() {
    let cases = [
        (
            r#"{"index_name":"w_name","field":"name","unique":false,"sparse":false}"#,
            "btree",
        ),
        (
            r#"{"index_name":"w_name_age","fields":["name","age"],"unique":false,"sparse":false}"#,
            "compound",
        ),
        (
            r#"{"index_name":"u_bio_fts","field":"bio","language":"hungarian","min_word_length":3,"accent_folding":false}"#,
            "fulltext",
        ),
        (
            r#"{"index_name":"w_city_fuzzy","field":"city","algorithm":"jaro_winkler","threshold":1.0}"#,
            "fuzzy",
        ),
        (
            r#"{"index_name":"w_vec_emb2","field":"emb2","dim":4,"metric":"dot_product","max_vectors":500,"hnsw_params":{"m":8,"ef_construction":100,"ef_search":20}}"#,
            "vector",
        ),
    ];
    for (text, kind) in cases {
        let parsed = same_bytes::<<tools::IndexCreate as Tool>::Resp>(text);
        let got = match &parsed {
            IndexCreateResponse::BTree(_) => "btree",
            IndexCreateResponse::Compound(_) => "compound",
            IndexCreateResponse::Fulltext(_) => "fulltext",
            IndexCreateResponse::Fuzzy(_) => "fuzzy",
            IndexCreateResponse::Vector(_) => "vector",
        };
        assert_eq!(got, kind, "{text}");
        assert!(text.contains(&format!(r#""index_name":"{}""#, parsed.index_name())));
    }
    // A B+ tree response without `unique` is no shape at all
    requires::<IndexCreateResponse>(cases[0].0, "unique");
    requires::<IndexCreateResponse>(cases[4].0, "ef_search");

    same_bytes::<<tools::IndexDrop as Tool>::Resp>(r#"{"success":true,"dropped":"w_name"}"#);
    requires::<IndexDropResponse>(r#"{"success":true,"dropped":"w_name"}"#, "dropped");
    same_bytes::<<tools::IndexStatsRefresh as Tool>::Resp>(
        r#"{"success":true,"message":"Index statistics refreshed for collection 'w'"}"#,
    );
}

// === Search tool group (golden "fulltext_search" … "search" cases) ===

#[test]
fn fulltext_search_hits_are_flat_documents_with_typed_metadata() {
    let text = r#"{"results":[{"_id":1,"content":"Rust is a safe systems programming language","year":2024,"_score":2.859232119809774,"_matched_tokens":["rust","safe"],"_highlights":[{"field":"content","snippets":["<mark>Rust</mark> is a sa..."]}]},{"_id":4,"content":"An embedded database written in Rust with fast search","_score":0.9381845028807818,"_matched_tokens":["rust"],"_highlights":[{"field":"content","snippets":["...tten in <mark>Rust</mark> with fa..."]}]}],"count":2,"match_scope":"chunk"}"#;
    let r = same_bytes::<<tools::FulltextSearch as Tool>::Resp>(text);
    assert_eq!(r.match_scope, MatchScope::Chunk);
    assert_eq!(r.qualified_doc_ids, None);
    let hit = &r.results[0];
    assert_eq!(hit.score, 2.859232119809774);
    assert_eq!(hit.matched_tokens, ["rust", "safe"]);
    assert_eq!(hit.highlights.as_ref().unwrap()[0].field, "content");
    // The engine metadata is not part of the document
    assert_eq!(
        hit.document.keys().collect::<Vec<_>>(),
        ["_id", "content", "year"]
    );

    let scoped = same_bytes::<FulltextSearchResponse>(
        r#"{"results":[{"_id":4,"_score":2.652041386490226,"_matched_tokens":["rust","search"]}],"count":1,"match_scope":"document","qualified_doc_ids":1}"#,
    );
    assert_eq!(scoped.match_scope, MatchScope::Document);
    assert_eq!(scoped.qualified_doc_ids, Some(1));
    assert_eq!(scoped.results[0].highlights, None);

    for field in ["_score", "_matched_tokens", "match_scope", "count"] {
        requires::<FulltextSearchResponse>(text, field);
    }
}

#[test]
fn fulltext_analyze() {
    let text = r#"{"input":"A királyok és a királynők","language":"Hungarian","accent_folding":true,"min_word_length":2,"tokens":[{"original":"A","normalized":"a","stemmed":"a","kept":false,"filtered_reason":"Too short (min: 2 chars)"},{"original":"királyok","normalized":"kiralyok","stemmed":"kiraly","kept":true},{"original":"és","normalized":"es","stemmed":"es","kept":false,"filtered_reason":"Stop word"},{"original":"a","normalized":"a","stemmed":"a","kept":false,"filtered_reason":"Too short (min: 2 chars)"},{"original":"királynők","normalized":"kiralynok","stemmed":"kiralyno","kept":true}],"stop_words_removed":["és"],"final_tokens":["kiraly","kiralyno"],"inherited_from_index":false}"#;
    let r = same_bytes::<<tools::FulltextAnalyze as Tool>::Resp>(text);
    assert_eq!(r.final_tokens, ["kiraly", "kiralyno"]);
    assert_eq!(r.tokens[1].filtered_reason, None);
    assert!(!r.inherited_from_index);
    requires::<FulltextAnalyzeResponse>(text, "inherited_from_index");
    requires::<FulltextAnalyzeResponse>(text, "kept");
}

#[test]
fn fuzzy_search_hits() {
    let text = r#"{"results":[{"_id":5,"heading":"Gardening","year":2022,"_score":0.9777777777777777,"_matched_value":"Gardening","_highlight":"<mark>Gardening</mark>"}],"count":1}"#;
    let r = same_bytes::<<tools::FuzzySearch as Tool>::Resp>(text);
    assert_eq!(r.results[0].matched_value, "Gardening");
    assert_eq!(
        r.results[0].highlight.as_deref(),
        Some("<mark>Gardening</mark>")
    );
    let plain = same_bytes::<FuzzySearchResponse>(
        r#"{"results":[{"_id":3,"heading":"Python Guide","year":2023,"_score":0.9833333333333333,"_matched_value":"Python Guide"}],"count":1}"#,
    );
    assert_eq!(plain.results[0].highlight, None);
    requires::<FuzzySearchResponse>(text, "_matched_value");
    requires::<FuzzySearchResponse>(text, "_score");
}

#[test]
fn vector_search_score_position_follows_the_projection() {
    // No projection: `_score` after the document's fields
    let last = r#"{"results":[{"_id":3,"embedding":[63.0,1.0],"title":"Python Guide","_score":0.9999997615814209}],"count":1}"#;
    let r = same_bytes::<<tools::VectorSearch as Tool>::Resp>(last);
    assert_eq!(r.results[0].score_position, ScorePosition::Last);
    assert_eq!(r.results[0].score, 0.9999997615814209);
    assert!(!r.results[0].document.contains_key("_score"));
    // With a projection: `_score` first
    let first = r#"{"results":[{"_score":0.9999931454658508,"_id":5,"title":"Gardening"},{"_score":1.0,"title":"Rust"}],"count":2}"#;
    let r = same_bytes::<VectorSearchResponse>(first);
    assert_eq!(r.results[0].score_position, ScorePosition::First);
    assert_eq!(r.results[1].document.len(), 1);

    requires::<VectorSearchResponse>(last, "_score");
    let wrong: Result<VectorSearchResponse, _> =
        serde_json::from_str(r#"{"results":[{"_id":1,"_score":"high"}],"count":1}"#);
    assert!(wrong.is_err());
}

#[test]
fn search_documents_and_context_block() {
    let structured = r#"{"verdict":"unknown","documents":[{"doc_id":"d3","title":"Embedded Databases","year":2025,"relevance":0.04840067340067341,"passages":[{"text":"An embedded database written in Rust with fast search"}]}],"count":1,"trimmed":false}"#;
    let r = same_bytes::<<tools::Search as Tool>::Resp>(structured);
    assert_eq!(r.verdict, SearchVerdict::Unknown);
    let SearchBody::Documents(docs) = &r.body else {
        panic!("{r:?}")
    };
    assert_eq!(docs[0].doc_id, "d3");
    assert_eq!(docs[0].fields.keys().collect::<Vec<_>>(), ["title", "year"]);
    assert_eq!(docs[0].passages.len(), 1);

    let context = r#"{"verdict":"unknown","context":"[d2] Python Guide\nPython is a dynamic language\n\n","count":1,"trimmed":false,"degraded":"semantic search unavailable (no embedding provider configured) — BM25-only"}"#;
    let r = same_bytes::<SearchResponse>(context);
    assert!(matches!(r.body, SearchBody::Context(ref c) if c.starts_with("[d2]")));
    assert!(r.degraded.is_some());

    let budget = r#"{"verdict":"unknown","documents":[],"count":0,"trimmed":true,"dropped_empty_documents":1,"dropped_due_to_budget":2,"diagnostics":{"stage":"A","fusion":"rrf (existing)","calibration":"off","hyde":"off","total_chunks":61,"qualified_doc_ids":null}}"#;
    let r = same_bytes::<SearchResponse>(budget);
    assert_eq!(r.dropped_due_to_budget, Some(2));
    assert_eq!(r.dropped_empty_documents, Some(1));
    assert_eq!(r.diagnostics.as_ref().unwrap().qualified_doc_ids, None);

    for field in [
        "verdict",
        "count",
        "trimmed",
        "relevance",
        "passages",
        "doc_id",
    ] {
        requires::<SearchResponse>(structured, field);
    }
    requires::<SearchResponse>(structured, "documents");
    requires::<SearchResponse>(budget, "total_chunks");
    // A one-chunk document's own non-string `doc_id` reaches the wire as is
    // (golden "search / numeric doc_id field"); it must not fail the whole list
    let numeric: SearchResponse = serde_json::from_str(
        r#"{"verdict":"unknown","documents":[{"doc_id":7,"relevance":0.04285714285714286,"passages":[{"text":"a rust chunk"}]}],"count":1,"trimmed":false}"#,
    )
    .unwrap();
    match numeric.body {
        SearchBody::Documents(docs) => assert_eq!(docs[0].doc_id, serde_json::json!(7)),
        other => panic!("{other:?}"),
    }
}
