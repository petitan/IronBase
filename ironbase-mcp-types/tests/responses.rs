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
