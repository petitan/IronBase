//! The CRUD state machine shared by P2 (`pbt_crud_state_machine.rs`) and
//! P3 (`pbt_crash_recovery.rs`): operations, a `BTreeMap` model, how an
//! operation changes the engine and the model, and the invariants checked
//! after it.

use super::*;
use proptest::prelude::*;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// The model: `_id` (as JSON text) → the stored document, `_id` included
pub type Model = BTreeMap<String, Map<String, Value>>;

#[derive(Debug, Clone)]
pub enum Op {
    Insert(Vec<Map<String, Value>>),
    /// Update the document picked by the index (modulo the model size)
    UpdateById(usize, Value),
    UpdateMany(Value, Value),
    DeleteById(usize),
    DeleteMany(Value),
    CreateIndex(&'static str),
    /// A compound index on (a, o.y): `o.y` is never an array, as a compound
    /// index may index at most one array field
    CreateCompoundIndex,
    /// Drop the index picked by the index (modulo the live index count)
    DropIndex(usize),
    // File storage only
    Checkpoint,
    Compact,
    Reopen,
}

/// An update on top-level fields: `$set`, `$unset` or `$inc`, one operator
/// per update so no field is named twice
pub fn arb_update() -> BoxedStrategy<Value> {
    let field = || prop::sample::select(FIELDS.to_vec());
    let increment = prop_oneof![
        3 => (-3i64..4).prop_map(Value::from),
        1 => prop::sample::select(vec![0.5, -1.5, 2.5]).prop_map(|f| json!(f)),
    ];
    prop_oneof![
        3 => prop::collection::btree_map(field(), arb_field_value(), 1..3)
            .prop_map(|m| json!({ "$set": m })),
        1 => prop::collection::btree_set(field(), 1..3).prop_map(|s| {
            let m: Map<String, Value> = s.into_iter().map(|f| (f.to_string(), json!(""))).collect();
            json!({ "$unset": m })
        }),
        2 => prop::collection::btree_map(field(), increment, 1..3)
            .prop_map(|m| json!({ "$inc": m })),
    ]
    .boxed()
}

pub fn arb_crud_op() -> BoxedStrategy<Op> {
    prop_oneof![
        4 => prop::collection::vec(arb_doc(), 1..4).prop_map(Op::Insert),
        3 => (any::<usize>(), arb_update()).prop_map(|(i, u)| Op::UpdateById(i, u)),
        2 => (arb_query(), arb_update()).prop_map(|(q, u)| Op::UpdateMany(q, u)),
        2 => any::<usize>().prop_map(Op::DeleteById),
        1 => arb_query().prop_map(Op::DeleteMany),
        1 => prop::sample::select(PATHS.to_vec()).prop_map(Op::CreateIndex),
        1 => Just(Op::CreateCompoundIndex),
        1 => any::<usize>().prop_map(Op::DropIndex),
    ]
    .boxed()
}

pub fn arb_file_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        12 => arb_crud_op(),
        1 => Just(Op::Checkpoint),
        1 => Just(Op::Compact),
        1 => Just(Op::Reopen),
    ]
}

/// The `_id` filter of the document the index picks; a filter matching
/// nothing on an empty model
pub fn pick_id(model: &Model, i: usize) -> (Value, Option<String>) {
    match model.keys().nth(i.checked_rem(model.len()).unwrap_or(0)) {
        Some(key) => (json!({ "_id": model[key]["_id"] }), Some(key.clone())),
        None => (json!({ "_id": -987654321 }), None),
    }
}

/// `doc` after `update`, and whether it changed (`1` → `1.0` is a change)
pub fn updated(
    doc: &Map<String, Value>,
    update: &Value,
) -> Result<(Map<String, Value>, bool), String> {
    let mut next = doc.clone();
    apply_update(&mut next, update)?;
    let changed = stored_differently(&next, doc);
    Ok((next, changed))
}

/// Apply one CRUD operation to the engine and the model; `indexes` holds
/// the (path, name) of the live indexes
pub fn apply(
    db: &impl Db,
    model: &mut Model,
    indexes: &mut Vec<(&'static str, String)>,
    op: &Op,
) -> Result<(), TestCaseError> {
    match op {
        Op::Insert(docs) => {
            for doc in db.insert_docs("s", docs) {
                model.insert(doc["_id"].to_string(), doc);
            }
        }
        Op::UpdateById(i, update) => {
            let (filter, key) = pick_id(model, *i);
            let updated = key.as_ref().map(|key| updated(&model[key], update));
            match (db.update_first("s", &filter, update), updated) {
                (Ok((matched, modified)), None) => {
                    prop_assert_eq!((matched, modified), (0, 0), "update_one of no document");
                }
                (Ok((matched, modified)), Some(Ok((next, changed)))) => {
                    prop_assert_eq!(matched, 1, "update_one matched");
                    prop_assert_eq!(modified, changed as u64, "update_one modified, {}", update);
                    model.insert(key.unwrap(), next);
                }
                (Err(_), Some(Err(_))) => {}
                (result, expected) => {
                    prop_assert!(false, "update_one {:?}, model {:?}", result, expected)
                }
            }
        }
        Op::UpdateMany(filter, update) => {
            let hits: Vec<String> = model
                .iter()
                .filter(|(_, d)| matches(d, filter))
                .map(|(k, _)| k.clone())
                .collect();
            let updates: Result<Vec<_>, String> = hits
                .iter()
                .map(|key| updated(&model[key], update).map(|u| (key.clone(), u)))
                .collect();
            match (db.update_all("s", filter, update), updates) {
                (Ok((matched, modified)), Ok(updates)) => {
                    prop_assert_eq!(
                        matched as usize,
                        hits.len(),
                        "update_many matched, filter {}",
                        filter
                    );
                    let changed = updates.iter().filter(|(_, (_, changed))| *changed).count();
                    prop_assert_eq!(
                        modified as usize,
                        changed,
                        "update_many modified, {}",
                        update
                    );
                    for (key, (next, _)) in updates {
                        model.insert(key, next);
                    }
                }
                // A rejected update_many writes nothing
                (Err(_), Err(_)) => {}
                (result, expected) => {
                    prop_assert!(false, "update_many {:?}, model {:?}", result, expected)
                }
            }
        }
        Op::DeleteById(i) => {
            let (filter, key) = pick_id(model, *i);
            prop_assert_eq!(
                db.delete_first("s", &filter),
                key.is_some() as u64,
                "delete_one"
            );
            if let Some(key) = key {
                model.remove(&key);
            }
        }
        Op::DeleteMany(filter) => {
            let before = model.len();
            model.retain(|_, d| !matches(d, filter));
            let deleted = db.delete_all("s", filter);
            prop_assert_eq!(
                deleted as usize,
                before - model.len(),
                "delete_many, filter {}",
                filter
            );
        }
        Op::CreateCompoundIndex => {
            if !indexes.iter().any(|(p, _)| *p == "a,o.y") {
                indexes.push(("a,o.y", db.create_compound_index("s", &["a", "o.y"])));
            }
        }
        Op::CreateIndex(path) => {
            if !indexes.iter().any(|(p, _)| p == path) {
                indexes.push((path, db.create_index("s", path)));
            }
        }
        Op::DropIndex(i) => {
            if !indexes.is_empty() {
                let (_, name) = indexes.remove(i % indexes.len());
                db.drop_index("s", &name);
            }
        }
        Op::Checkpoint | Op::Compact | Op::Reopen => unreachable!("handled by the file harness"),
    }
    Ok(())
}

/// The invariants that hold after every operation
pub fn check(
    db: &impl Db,
    model: &Model,
    probes: &[Value],
    after: &Op,
) -> Result<(), TestCaseError> {
    let why = format!("after {after:?}");
    let all = db
        .find("s", &json!({}))
        .map_err(|e| TestCaseError::fail(format!("{why}: {e}")))?;
    prop_assert_eq!(
        all.len(),
        model.len(),
        "find({{}}) size (duplicates?), {}",
        why
    );
    let found: BTreeMap<String, Value> =
        all.into_iter().map(|d| (d["_id"].to_string(), d)).collect();
    let expected: BTreeMap<String, Value> = model
        .iter()
        .map(|(k, d)| (k.clone(), Value::Object(d.clone())))
        .collect();
    prop_assert_eq!(&found, &expected, "find({{}}), {}", why);

    prop_assert_eq!(
        db.count("s", &json!({})) as usize,
        model.len(),
        "count, {}",
        why
    );
    let (live, order_exact) = db.storage_invariants("s");
    prop_assert_eq!(live as usize, model.len(), "live_document_count, {}", why);
    prop_assert!(
        order_exact,
        "document_order is not the catalog's ids, {}",
        why
    );

    for probe in probes {
        let want: std::collections::BTreeSet<String> = model
            .iter()
            .filter(|(_, d)| matches(d, probe))
            .map(|(k, _)| k.clone())
            .collect();
        let got = db
            .find("s", probe)
            .map_err(|e| TestCaseError::fail(format!("{why}: {e}")))?;
        prop_assert_eq!(got.len(), want.len(), "probe {} size, {}", probe, why);
        prop_assert_eq!(&ids(&got), &want, "probe {}, {}", probe, why);
        prop_assert_eq!(
            db.count("s", probe) as usize,
            want.len(),
            "probe {} count, {}",
            probe,
            why
        );
    }
    Ok(())
}
