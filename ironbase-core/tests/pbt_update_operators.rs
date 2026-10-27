//! P4 (property-based testing plan): the update operators against an
//! oracle of their MongoDB semantics.
//!
//! A random document gets a random update of up to three clauses (`$set`,
//! `$unset`, `$inc`, `$push`, `$pull`, `$addToSet`, `$pop`) on top-level and
//! nested paths. The engine must reject what the oracle rejects; otherwise
//! the stored document and the reported modified count must equal the
//! oracle's. A second property runs `update_many` over an indexed collection
//! and checks the documents and the index answers afterwards.

mod pbt_support;

use ironbase_core::storage::MemoryStorage;
use ironbase_core::DatabaseCore;
use pbt_support::*;
use proptest::prelude::*;
use serde_json::{json, Map, Value};
use std::cmp::Ordering;

/// Paths an update may name: top-level fields, fields of the object `o`
/// (which may be missing, a scalar or an object), and a new nested path
const UPDATE_PATHS: [&str; 7] = ["a", "b", "c", "o", "o.x", "o.y", "n.m"];

fn conflicts(p: &str, q: &str) -> bool {
    p == q || p.starts_with(&format!("{q}.")) || q.starts_with(&format!("{p}."))
}

fn arb_clause() -> impl Strategy<Value = (&'static str, &'static str, Value)> {
    let path = prop::sample::select(UPDATE_PATHS.to_vec());
    let increment = prop_oneof![
        3 => (-3i64..4).prop_map(Value::from),
        1 => prop::sample::select(vec![i64::MAX, i64::MIN]).prop_map(Value::from),
        2 => prop::sample::select(vec![0.5, -1.5, 0.0, 1e308]).prop_map(|f| json!(f)),
    ];
    let items = || prop::collection::vec(arb_scalar(), 0..3);
    let operand = prop_oneof![
        3 => arb_field_value().prop_map(|v| ("$set", v)),
        1 => Just(("$unset", json!(""))),
        2 => increment.prop_map(|v| ("$inc", v)),
        1 => arb_scalar().prop_map(|v| ("$push", v)),
        1 => items().prop_map(|v| ("$push", json!({ "$each": v }))),
        1 => arb_scalar().prop_map(|v| ("$pull", v)),
        1 => arb_scalar().prop_map(|v| ("$pull", json!({ "$gt": v }))),
        1 => arb_scalar().prop_map(|v| ("$addToSet", v)),
        1 => items().prop_map(|v| ("$addToSet", json!({ "$each": v }))),
        1 => prop::sample::select(vec![-1, 1]).prop_map(|d| ("$pop", json!(d))),
    ];
    (path, operand).prop_map(|(path, (op, v))| (op, path, v))
}

/// An update of 1..=3 clauses on paths that do not overlap (overlapping
/// paths in one update are a MongoDB error the property does not cover)
fn arb_update() -> impl Strategy<Value = Value> {
    prop::collection::vec(arb_clause(), 1..4).prop_map(|clauses| {
        let mut update = Map::new();
        let mut taken: Vec<&str> = Vec::new();
        for (op, path, v) in clauses {
            if taken.iter().any(|t| conflicts(t, path)) {
                continue;
            }
            taken.push(path);
            update
                .entry(op)
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .unwrap()
                .insert(path.to_string(), v);
        }
        Value::Object(update)
    })
}

// ============================================================================
// Oracle
// ============================================================================

/// Why the oracle rejects an update
type Rejected = String;

/// A field's place: its parent object and its name there
type Slot<'a> = (&'a mut Map<String, Value>, &'a str);

/// The parent object of `path`'s last part. `Ok(None)`: the path cannot be
/// reached (a missing or non-object step) and `create` is false. With
/// `create`, missing steps are created and a non-object step is an error
/// (MongoDB: "Cannot create field 'x' in element {o: 5}").
fn parent<'a>(
    doc: &'a mut Map<String, Value>,
    path: &'a str,
    create: bool,
) -> Result<Option<Slot<'a>>, Rejected> {
    let mut parts: Vec<&str> = path.split('.').collect();
    let last = parts.pop().unwrap();
    let mut current = doc;
    for part in parts {
        if !current.contains_key(part) {
            if !create {
                return Ok(None);
            }
            current.insert(part.to_string(), json!({}));
        }
        current = match current.get_mut(part).unwrap() {
            Value::Object(map) => map,
            other if create => return Err(format!("cannot create {last} in {other}")),
            _ => return Ok(None),
        };
    }
    Ok(Some((current, last)))
}

/// MongoDB value equality for array membership: numbers by value
fn same(a: &Value, b: &Value) -> bool {
    equal(a, b)
}

fn apply_clause(
    doc: &mut Map<String, Value>,
    op: &str,
    path: &str,
    v: &Value,
) -> Result<(), Rejected> {
    match op {
        "$set" => {
            let (map, key) = parent(doc, path, true)?.unwrap();
            map.insert(key.to_string(), v.clone());
        }
        "$unset" => {
            if let Some((map, key)) = parent(doc, path, false)? {
                map.shift_remove(key);
            }
        }
        "$inc" => {
            let (map, key) = parent(doc, path, true)?.unwrap();
            let next = match map.get(key) {
                None => v.clone(),
                Some(current) if current.is_number() => match (current.as_i64(), v.as_i64()) {
                    (Some(c), Some(i)) => {
                        json!(c
                            .checked_add(i)
                            .ok_or_else(|| format!("$inc overflow {c} + {i}"))?)
                    }
                    _ => {
                        let sum = current.as_f64().unwrap() + v.as_f64().unwrap();
                        if !sum.is_finite() {
                            return Err(format!("$inc to {sum} (not storable in JSON)"));
                        }
                        json!(sum)
                    }
                },
                Some(other) => return Err(format!("$inc on non-number {other}")),
            };
            map.insert(key.to_string(), next);
        }
        "$push" | "$addToSet" => {
            let items: Vec<Value> = match v.get("$each") {
                Some(Value::Array(each)) => each.clone(),
                _ => vec![v.clone()],
            };
            let (map, key) = parent(doc, path, true)?.unwrap();
            let array = match map.entry(key.to_string()).or_insert_with(|| json!([])) {
                Value::Array(array) => array,
                other => return Err(format!("{op} on non-array {other}")),
            };
            for item in items {
                if op == "$push" || !array.iter().any(|e| same(e, &item)) {
                    array.push(item);
                }
            }
        }
        "$pull" | "$pop" => {
            let Some((map, key)) = parent(doc, path, false)? else {
                return Ok(());
            };
            let array = match map.get_mut(key) {
                None => return Ok(()),
                Some(Value::Array(array)) => array,
                Some(other) => return Err(format!("{op} on non-array {other}")),
            };
            if op == "$pop" {
                if !array.is_empty() {
                    if v.as_i64() == Some(-1) {
                        array.remove(0);
                    } else {
                        array.pop();
                    }
                }
            } else {
                array.retain(|e| match v.get("$gt") {
                    Some(bound) => compare(e, bound) != Some(Ordering::Greater),
                    None => !same(e, v),
                });
            }
        }
        other => panic!("oracle: unsupported update operator {other}"),
    }
    Ok(())
}

/// The updated document and whether it changed (byte-wise: `1` → `1.0` is a
/// change), or why MongoDB rejects the update
fn oracle_update(
    doc: &Map<String, Value>,
    update: &Value,
) -> Result<(Map<String, Value>, bool), Rejected> {
    let mut next = doc.clone();
    for (op, clauses) in update.as_object().unwrap() {
        for (path, v) in clauses.as_object().unwrap() {
            apply_clause(&mut next, op, path, v)?;
        }
    }
    let changed = stored_differently(&next, doc);
    Ok((next, changed))
}

// ============================================================================
// Properties
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(cases(256)))]

    /// One document, one update by `_id`: rejected exactly when the oracle
    /// rejects it, else the stored document and modified count are the
    /// oracle's
    #[test]
    fn update_one_matches_the_oracle(doc in arb_doc(), update in arb_update()) {
        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
        let stored = db.insert_docs("u", std::slice::from_ref(&doc)).remove(0);
        let filter = json!({ "_id": stored["_id"] });
        let expected = oracle_update(&stored, &update);
        let result = db.update_one("u", &filter, &update);
        let after = db.find_one("u", &filter).unwrap();
        match (expected, result) {
            (Err(why), Ok(r)) => prop_assert!(false, "accepted ({r:?}), oracle rejects: {why}; stored {after}"),
            (Ok(_), Err(e)) => prop_assert!(false, "rejected ({e}), oracle accepts"),
            (Err(_), Err(_)) => prop_assert_eq!(&after, &Value::Object(stored), "a rejected update left a trace"),
            (Ok((next, changed)), Ok((matched, modified))) => {
                prop_assert_eq!(matched, 1);
                prop_assert_eq!(&after, &Value::Object(next));
                prop_assert_eq!(modified, changed as u64, "modified count");
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(cases(64)))]

    /// `update_many` over an indexed collection: the documents equal the
    /// oracle's, and queries through the indexes agree with it afterwards
    #[test]
    fn update_many_keeps_documents_and_indexes_consistent(
        docs in prop::collection::vec(arb_doc(), 1..20),
        filter in arb_query(),
        update in arb_update(),
        probes in prop::collection::vec(arb_query(), 1..4),
    ) {
        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
        let stored = db.insert_docs("u", &docs);
        for path in PATHS {
            db.create_index("u", path);
        }
        let mut expected = Vec::new();
        let mut rejected = false;
        for doc in &stored {
            if matches(doc, &filter) {
                match oracle_update(doc, &update) {
                    Ok((next, _)) => expected.push(next),
                    Err(_) => { rejected = true; expected.push(doc.clone()) }
                }
            } else {
                expected.push(doc.clone());
            }
        }
        let result = db.update_many("u", &filter, &update);
        prop_assert_eq!(result.is_err(), rejected, "update_many {:?}, filter {}", result, filter);
        if rejected {
            // A rejected document stops the whole update_many: nothing written
            expected = stored.clone();
        }
        let all = db.find("u", &json!({})).unwrap();
        prop_assert_eq!(ids(&all), ids(&expected.iter().cloned().map(Value::Object).collect::<Vec<_>>()));
        for doc in &expected {
            let found = db.find_one("u", &json!({ "_id": doc["_id"] })).unwrap();
            prop_assert_eq!(&found, &Value::Object(doc.clone()));
        }
        for probe in &probes {
            let want = ids(&expected.iter().filter(|d| matches(d, probe)).cloned().map(Value::Object).collect::<Vec<_>>());
            let got = db.find("u", probe).unwrap();
            prop_assert_eq!(got.len(), want.len(), "probe {} (duplicates?)", probe);
            prop_assert_eq!(ids(&got), want, "probe {}", probe);
        }
    }
}
