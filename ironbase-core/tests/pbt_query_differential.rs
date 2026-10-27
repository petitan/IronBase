//! P1 (property-based testing plan): every query path answers what the
//! reference oracle answers.
//!
//! Random documents and random queries over integers, floats (including
//! values that do not survive a naive print → parse), strings with Hungarian
//! accents, booleans, nulls, arrays, a nested object and missing fields.
//! Each query is asked through `find`, `count_documents`, `find_one`,
//! `find_streaming` and `aggregate([$match])`, on a collection without
//! indexes, one indexed before the inserts and one indexed after them; every
//! answer must equal the oracle's.

mod pbt_support;

use ironbase_core::storage::MemoryStorage;
use ironbase_core::{DatabaseCore, StorageEngine};
use pbt_support::*;
use proptest::prelude::*;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

/// Index layouts: the paths given single-field indexes, and an optional
/// compound index on (a, o.y): `o.y` is never an array, as a compound index
/// may index at most one array field (MongoDB rule, rejected on insert)
fn arb_indexes() -> impl Strategy<Value = (Vec<&'static str>, bool)> {
    (
        prop::sample::subsequence(PATHS.to_vec(), 1..=PATHS.len()),
        any::<bool>(),
    )
}

fn create_indexes(db: &impl Db, paths: &[&str], compound: bool) {
    for path in paths {
        db.create_index("p", path);
    }
    if compound {
        db.create_compound_index("p", &["a", "o.y"]);
    }
}

/// Every query path of `db` against the oracle; `label` names the layout
fn check_paths(
    label: &str,
    db: &impl Db,
    stored: &[Map<String, Value>],
    query: &Value,
) -> Result<(), TestCaseError> {
    let expected: BTreeSet<String> = stored
        .iter()
        .filter(|d| matches(d, query))
        .map(|d| d["_id"].to_string())
        .collect();
    let why = |path: &str| format!("{label}, {path}, query {query}");

    let found = db
        .find("p", query)
        .map_err(|e| TestCaseError::fail(format!("{}: {e}", why("find"))))?;
    prop_assert_eq!(&ids(&found), &expected, "{}", why("find"));
    prop_assert_eq!(found.len(), expected.len(), "{} (duplicates?)", why("find"));

    let count = db.count("p", query);
    prop_assert_eq!(count as usize, expected.len(), "{}", why("count_documents"));

    match db.find_one("p", query) {
        Some(doc) => prop_assert!(
            expected.contains(&doc["_id"].to_string()),
            "{}",
            why("find_one")
        ),
        None => prop_assert!(expected.is_empty(), "{}", why("find_one")),
    }

    let streamed = db.find_streaming("p", query);
    prop_assert_eq!(
        streamed.len(),
        expected.len(),
        "{} (duplicates?)",
        why("find_streaming")
    );
    prop_assert_eq!(&ids(&streamed), &expected, "{}", why("find_streaming"));

    let matched = db.aggregate("p", &json!([{ "$match": query }]));
    prop_assert_eq!(&ids(&matched), &expected, "{}", why("aggregate $match"));
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(cases(96)))]

    /// In memory: no index, indexes built before the inserts, indexes built
    /// over existing documents
    #[test]
    fn query_paths_agree_with_the_oracle(
        docs in prop::collection::vec(arb_doc(), 0..40),
        queries in prop::collection::vec(arb_query(), 1..8),
        (paths, compound) in arb_indexes(),
    ) {
        let plain = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
        let stored = plain.insert_docs("p", &docs);

        let before = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
        before.insert_docs("p", &[]);
        create_indexes(&before, &paths, compound);
        before.insert_docs("p", &docs);

        let after = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
        after.insert_docs("p", &docs);
        create_indexes(&after, &paths, compound);

        let layout = format!("indexes {paths:?} compound {compound}");
        for query in &queries {
            check_paths("no index", &plain, &stored, query)?;
            check_paths(&format!("indexed before inserts, {layout}"), &before, &stored, query)?;
            check_paths(&format!("indexed after inserts, {layout}"), &after, &stored, query)?;
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(cases(16)))]

    /// On file storage, before and after reopening: the documents and the
    /// persisted indexes come back from disk (floats included)
    #[test]
    fn query_paths_agree_with_the_oracle_on_disk(
        docs in prop::collection::vec(arb_doc(), 0..30),
        queries in prop::collection::vec(arb_query(), 1..6),
        (paths, compound) in arb_indexes(),
    ) {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("p.mlite");
        let stored = {
            let db = DatabaseCore::<StorageEngine>::open(&path).unwrap();
            let stored = db.insert_docs("p", &docs);
            create_indexes(&db, &paths, compound);
            for query in &queries {
                check_paths("file", &db, &stored, query)?;
            }
            db.close().unwrap();
            stored
        };
        let db = DatabaseCore::<StorageEngine>::open(&path).unwrap();
        for query in &queries {
            check_paths("file, reopened", &db, &stored, query)?;
        }
        db.close().unwrap();
    }
}
