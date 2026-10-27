//! P7 (property-based testing plan): aggregation pipelines against an
//! oracle.
//!
//! - `$match` returns what `find` returns;
//! - `$group` by a field (missing = null; numbers by value, `1` and `1.0` one
//!   group) with `$sum` (also of 1, a count), `$avg`, `$min`, `$max` over a
//!   numeric field gives the oracle's groups and values;
//! - `$sort` + `$limit` (the top-K path) gives the first K of a full sort;
//! - `$skip` / `$limit` after a sort give the matching slice.
//!
//! In memory, with and without an index on the grouped and sorted fields.

mod pbt_support;

use ironbase_core::storage::MemoryStorage;
use ironbase_core::DatabaseCore;
use pbt_support::*;
use proptest::prelude::*;
use serde_json::{json, Map, Value};

/// Documents: `g` (group key: small scalars, `1` and `1.0`, null, missing),
/// `n` (numeric or missing), `s` (sort key: distinct integer, so sorts are
/// deterministic) and the P1 fields
fn arb_docs() -> impl Strategy<Value = Vec<Map<String, Value>>> {
    let g = prop::option::of(prop_oneof![
        3 => (0i64..3).prop_map(Value::from),
        1 => Just(json!(1.0)),
        1 => prop::sample::select(vec!["x", "y"]).prop_map(Value::from),
        1 => Just(Value::Null),
    ]);
    let n = prop::option::of(prop_oneof![
        3 => (-5i64..6).prop_map(Value::from),
        1 => prop::sample::select(vec![0.5, -1.5, 2.25]).prop_map(|f| json!(f)),
    ]);
    prop::collection::vec((g, n, arb_doc()), 0..40).prop_map(|rows| {
        rows.into_iter()
            .enumerate()
            .map(|(i, (g, n, mut doc))| {
                if let Some(g) = g {
                    doc.insert("g".into(), g);
                }
                if let Some(n) = n {
                    doc.insert("n".into(), n);
                }
                // Distinct, in a scrambled order
                doc.insert("s".into(), json!((i as i64 * 7919) % 1009));
                doc
            })
            .collect()
    })
}

fn num(v: &Value) -> Option<f64> {
    v.as_f64()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

/// The oracle's groups: key → (count, numeric `n` values)
fn oracle_groups(docs: &[Map<String, Value>]) -> Vec<(Value, usize, Vec<f64>)> {
    let mut groups: Vec<(Value, usize, Vec<f64>)> = Vec::new();
    for doc in docs {
        let key = doc.get("g").cloned().unwrap_or(Value::Null);
        let group = match groups.iter_mut().position(|(k, _, _)| equal(k, &key)) {
            Some(i) => &mut groups[i],
            None => {
                groups.push((key, 0, Vec::new()));
                groups.last_mut().unwrap()
            }
        };
        group.1 += 1;
        if let Some(n) = doc.get("n").and_then(num) {
            group.2.push(n);
        }
    }
    groups
}

fn check_group(
    db: &impl Db,
    stored: &[Map<String, Value>],
    why: &str,
) -> Result<(), TestCaseError> {
    let out = db.aggregate(
        "a",
        &json!([{ "$group": {
            "_id": "$g",
            "count": { "$sum": 1 },
            "total": { "$sum": "$n" },
            "mean": { "$avg": "$n" },
            "low": { "$min": "$n" },
            "high": { "$max": "$n" },
        }}]),
    );
    let expected = oracle_groups(stored);
    prop_assert_eq!(out.len(), expected.len(), "{}: groups {:?}", why, out);
    for (key, count, values) in &expected {
        let row = out.iter().find(|r| equal(&r["_id"], key));
        prop_assert!(row.is_some(), "{}: no group {} in {:?}", why, key, out);
        let row = row.unwrap();
        prop_assert_eq!(
            row["count"].as_u64(),
            Some(*count as u64),
            "{}: count of {}",
            why,
            key
        );
        let total: f64 = values.iter().sum();
        prop_assert!(
            row["total"].as_f64().is_some_and(|t| close(t, total)),
            "{}: $sum of {}: {} vs {}",
            why,
            key,
            row["total"],
            total
        );
        let (mean, low, high) = if values.is_empty() {
            (None, None, None)
        } else {
            (
                Some(total / values.len() as f64),
                values.iter().cloned().reduce(f64::min),
                values.iter().cloned().reduce(f64::max),
            )
        };
        for (name, want) in [("mean", mean), ("low", low), ("high", high)] {
            let got = row[name].as_f64();
            let ok = match want {
                None => row[name].is_null(),
                Some(w) => got.is_some_and(|g| close(g, w)),
            };
            prop_assert!(
                ok,
                "{}: {} of {}: {} vs {:?}",
                why,
                name,
                key,
                row[name],
                want
            );
        }
    }
    Ok(())
}

/// `$group` by `a`, which may be an array, an object-free scalar of any
/// type, or missing: one group per distinct value (arrays as whole values)
fn check_group_by_any_value(
    db: &impl Db,
    stored: &[Map<String, Value>],
    why: &str,
) -> Result<(), TestCaseError> {
    let out = db.aggregate(
        "a",
        &json!([{ "$group": { "_id": "$a", "count": { "$sum": 1 } } }]),
    );
    let mut groups: Vec<(Value, u64)> = Vec::new();
    for doc in stored {
        let key = doc.get("a").cloned().unwrap_or(Value::Null);
        match groups.iter_mut().find(|(k, _)| equal(k, &key)) {
            Some(group) => group.1 += 1,
            None => groups.push((key, 1)),
        }
    }
    prop_assert_eq!(out.len(), groups.len(), "{}: groups by $a {:?}", why, out);
    for (key, count) in &groups {
        let row = out.iter().find(|r| equal(&r["_id"], key));
        prop_assert!(
            row.is_some_and(|r| r["count"].as_u64() == Some(*count)),
            "{}: group {} ({} docs) in {:?}",
            why,
            key,
            count,
            out
        );
    }
    Ok(())
}

fn check_sort_window(
    db: &impl Db,
    stored: &[Map<String, Value>],
    desc: bool,
    skip: usize,
    limit: usize,
    why: &str,
) -> Result<(), TestCaseError> {
    let dir = if desc { -1 } else { 1 };
    let mut sorted: Vec<i64> = stored.iter().map(|d| d["s"].as_i64().unwrap()).collect();
    sorted.sort();
    if desc {
        sorted.reverse();
    }
    let s = |docs: Vec<Value>| {
        docs.iter()
            .map(|d| d["s"].as_i64().unwrap())
            .collect::<Vec<_>>()
    };

    // $sort + $limit: the top-K path
    let top = s(db.aggregate(
        "a",
        &json!([{ "$sort": { "s": dir } }, { "$limit": limit }]),
    ));
    let want: Vec<i64> = sorted.iter().take(limit).cloned().collect();
    prop_assert_eq!(&top, &want, "{}: $sort {} + $limit {}", why, dir, limit);

    // $sort, $skip, $limit
    let window = s(db.aggregate(
        "a",
        &json!([{ "$sort": { "s": dir } }, { "$skip": skip }, { "$limit": limit }]),
    ));
    let want: Vec<i64> = sorted.iter().skip(skip).take(limit).cloned().collect();
    prop_assert_eq!(
        &window,
        &want,
        "{}: $sort {} + $skip {} + $limit {}",
        why,
        dir,
        skip,
        limit
    );
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(cases(64)))]

    #[test]
    fn aggregation_agrees_with_the_oracle(
        docs in arb_docs(),
        queries in prop::collection::vec(arb_query(), 1..4),
        desc in any::<bool>(),
        skip in 0usize..10,
        limit in 1usize..12,
    ) {
        let plain = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
        let stored = plain.insert_docs("a", &docs);
        let indexed = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
        indexed.insert_docs("a", &[]);
        for path in ["g", "n", "s", "a"] {
            indexed.create_index("a", path);
        }
        indexed.insert_docs("a", &docs);

        for (db, label) in [(&plain, "no index"), (&indexed, "indexed")] {
            for query in &queries {
                let matched = db.aggregate("a", &json!([{ "$match": query }]));
                let found = db.find("a", query).unwrap();
                prop_assert_eq!(ids(&matched), ids(&found), "{}: $match vs find, {}", label, query);
                prop_assert_eq!(matched.len(), found.len(), "{}: $match size, {}", label, query);
            }
            check_group(db, &stored, label)?;
            check_group_by_any_value(db, &stored, label)?;
            for query in &queries {
                let expected = stored.iter().filter(|d| matches(d, query)).count();
                let counted = db.aggregate("a", &json!([{ "$match": query }, { "$count": "n" }]));
                let got = counted.first().and_then(|r| r["n"].as_u64()).unwrap_or(0);
                prop_assert_eq!(got as usize, expected, "{}: $count, {}", label, query);
            }
            // $unwind: one row per array element, a scalar one row, a
            // missing/null field or an empty array none (MongoDB)
            let unwound = db.aggregate("a", &json!([{ "$unwind": "$a" }]));
            let expected: usize = stored
                .iter()
                .map(|d| match d.get("a") {
                    None | Some(Value::Null) => 0,
                    Some(Value::Array(items)) => items.len(),
                    Some(_) => 1,
                })
                .sum();
            prop_assert_eq!(unwound.len(), expected, "{}: $unwind rows", label);
            check_sort_window(db, &stored, desc, skip, limit, label)?;
        }
    }
}
