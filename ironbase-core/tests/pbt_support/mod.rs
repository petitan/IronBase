//! Shared generators and reference oracle for the property-based tests
//! (property-based testing plan, phase 1).
//!
//! The oracle is a deliberately small and slow evaluator of the query and
//! update semantics the engine documents, written independently of the
//! engine's matcher. A property compares what the engine answers on some
//! execution path with what the oracle answers on the same data.
//!
//! Semantics fixed here (the engine's documented, MongoDB-compatible ones):
//! - a missing field equals `null` (`$eq`, `$in`; `$gte`/`$lte: null`);
//! - numbers compare by exact value across integers and floats (`1 == 1.0`);
//! - an array field matches when the array itself or any element matches;
//! - values of different types are not comparable (`$gt` etc. never match);
//! - `$ne`, `$nin` and `$not` are the negations, so they match a missing field.
#![allow(dead_code)]

pub mod crud;

use proptest::prelude::*;
use serde_json::{json, Map, Value};
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};

/// Top-level fields a document may hold; `o` is an object with `x` and `y`
pub const FIELDS: [&str; 3] = ["a", "b", "c"];
/// Paths a query or an index may name
pub const PATHS: [&str; 5] = ["a", "b", "c", "o.x", "o.y"];

// ============================================================================
// Generators
// ============================================================================

/// Floats that stress the number paths: exact integers, short fractions,
/// signed zero, a subnormal, values whose shortest decimal form needs 17
/// digits, and any finite float
pub fn arb_float() -> impl Strategy<Value = f64> {
    prop_oneof![
        3 => prop::sample::select(vec![
            0.5, 1.0, 2.0, -1.5, 0.1, 0.2, 0.3, -0.0, 2.5,
            123456789.12345679, 1e-310, 9007199254740993.0, 1e300,
        ]),
        1 => any::<f64>().prop_filter("finite", |f| f.is_finite()),
    ]
}

/// A scalar from a small pool, so equality queries often hit stored values
pub fn arb_scalar() -> BoxedStrategy<Value> {
    prop_oneof![
        4 => (-3i64..4).prop_map(Value::from),
        1 => prop::sample::select(vec![i64::MIN, i64::MAX, 1 << 53]).prop_map(Value::from),
        3 => arb_float().prop_map(|f| json!(f)),
        3 => prop::sample::select(vec!["", "a", "b", "ab", "é", "ő", "Ő"]).prop_map(Value::from),
        1 => any::<bool>().prop_map(Value::from),
        1 => Just(Value::Null),
    ]
    .boxed()
}

/// A field value: mostly a scalar, sometimes a short array of scalars
pub fn arb_field_value() -> BoxedStrategy<Value> {
    prop_oneof![
        5 => arb_scalar(),
        1 => prop::collection::vec(arb_scalar(), 0..4).prop_map(Value::Array),
    ]
    .boxed()
}

/// A document without `_id`: each field present or missing; `o` an object
/// with `x`/`y`, a scalar, or missing
pub fn arb_doc() -> BoxedStrategy<Map<String, Value>> {
    let o = prop_oneof![
        3 => (
            prop::option::of(arb_field_value()),
            prop::option::of(arb_scalar())
        )
            .prop_map(|(x, y)| {
                let mut o = Map::new();
                if let Some(x) = x {
                    o.insert("x".into(), x);
                }
                if let Some(y) = y {
                    o.insert("y".into(), y);
                }
                Some(Value::Object(o))
            }),
        1 => arb_scalar().prop_map(Some),
        1 => Just(None),
    ];
    (
        prop::collection::vec(prop::option::of(arb_field_value()), FIELDS.len()),
        o,
    )
        .prop_map(|(values, o)| {
            let mut doc = Map::new();
            for (field, value) in FIELDS.iter().zip(values) {
                if let Some(value) = value {
                    doc.insert(field.to_string(), value);
                }
            }
            if let Some(o) = o {
                doc.insert("o".into(), o);
            }
            doc
        })
        .boxed()
}

/// One field condition: `{path: value}` or `{path: {$op: ...}}`
fn arb_condition() -> impl Strategy<Value = Value> {
    let path = prop::sample::select(PATHS.to_vec());
    let operand = prop_oneof![
        3 => arb_scalar(),
        1 => (
            prop::sample::select(vec!["$eq", "$ne", "$gt", "$gte", "$lt", "$lte"]),
            arb_scalar()
        )
            .prop_map(|(op, v)| json!({ op: v })),
        1 => (
            prop::sample::select(vec!["$gt", "$gte"]),
            arb_scalar(),
            prop::sample::select(vec!["$lt", "$lte"]),
            arb_scalar()
        )
            .prop_map(|(lo, l, hi, h)| json!({ lo: l, hi: h })),
        1 => (
            prop::sample::select(vec!["$in", "$nin"]),
            prop::collection::vec(arb_scalar(), 0..4)
        )
            .prop_map(|(op, vs)| json!({ op: vs })),
        1 => any::<bool>().prop_map(|e| json!({ "$exists": e })),
        1 => (
            prop::sample::select(vec!["$eq", "$gt", "$lte", "$in"]),
            arb_scalar()
        )
            .prop_map(|(op, v)| {
                let v = if op == "$in" { json!([v]) } else { v };
                json!({ "$not": { op: v } })
            }),
    ];
    (path, operand).prop_map(|(p, o)| json!({ p: o }))
}

/// A query: a field condition, two conditions in one object (implicit
/// `$and`), or `$and`/`$or`/`$nor` over sub-queries
pub fn arb_query() -> BoxedStrategy<Value> {
    let leaf = prop_oneof![
        4 => arb_condition(),
        1 => (arb_condition(), arb_condition()).prop_map(|(a, b)| {
            let mut merged = a.as_object().unwrap().clone();
            for (k, v) in b.as_object().unwrap() {
                merged.entry(k.clone()).or_insert_with(|| v.clone());
            }
            Value::Object(merged)
        }),
        1 => Just(json!({})),
    ];
    leaf.prop_recursive(2, 8, 3, |inner| {
        (
            prop::sample::select(vec!["$and", "$or", "$nor"]),
            prop::collection::vec(inner, 1..4),
        )
            .prop_map(|(op, qs)| json!({ op: qs }))
    })
    .boxed()
}

/// The cases a property runs: `PROPTEST_CASES` when set (the nightly run
/// raises it), else `default` (sized for the PR CI)
pub fn cases(default: u32) -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

// ============================================================================
// Oracle: values
// ============================================================================

/// The value at a dot path (`o.x`); None when any step is missing
pub fn get_path<'a>(doc: &'a Map<String, Value>, path: &str) -> Option<&'a Value> {
    let mut parts = path.split('.');
    let mut value = doc.get(parts.next()?)?;
    for part in parts {
        value = value.as_object()?.get(part)?;
    }
    Some(value)
}

/// Exact order of an integer against a finite float
fn cmp_int_float(i: i128, f: f64) -> Ordering {
    if f >= 1e38 {
        return Ordering::Less;
    }
    if f <= -1e38 {
        return Ordering::Greater;
    }
    let floor = f.floor();
    match i.cmp(&(floor as i128)) {
        Ordering::Equal if f > floor => Ordering::Less,
        o => o,
    }
}

fn as_int(n: &serde_json::Number) -> Option<i128> {
    n.as_i64()
        .map(i128::from)
        .or_else(|| n.as_u64().map(i128::from))
}

/// Order of two values of the same comparable type; None across types
pub fn compare(a: &Value, b: &Value) -> Option<Ordering> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => match (as_int(x), as_int(y)) {
            (Some(i), Some(j)) => Some(i.cmp(&j)),
            (Some(i), None) => Some(cmp_int_float(i, y.as_f64()?)),
            (None, Some(j)) => Some(cmp_int_float(j, x.as_f64()?).reverse()),
            (None, None) => x.as_f64()?.partial_cmp(&y.as_f64()?),
        },
        (Value::String(x), Value::String(y)) => Some(x.cmp(y)),
        (Value::Bool(x), Value::Bool(y)) => Some(x.cmp(y)),
        (Value::Null, Value::Null) => Some(Ordering::Equal),
        _ => None,
    }
}

/// Value equality: numbers by value, arrays element-wise, objects key-wise
pub fn equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(_), Value::Number(_)) => compare(a, b) == Some(Ordering::Equal),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| equal(p, q))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| equal(v, w)))
        }
        _ => a == b,
    }
}

// ============================================================================
// Oracle: queries
// ============================================================================

fn eq_matches(field: Option<&Value>, q: &Value) -> bool {
    match field {
        None => q.is_null(),
        Some(v) => equal(v, q) || matches!(v, Value::Array(a) if a.iter().any(|e| equal(e, q))),
    }
}

fn cmp_matches(field: Option<&Value>, q: &Value, pred: fn(Ordering) -> bool) -> bool {
    match field {
        None => q.is_null() && pred(Ordering::Equal),
        Some(v) => {
            compare(v, q).is_some_and(pred)
                || matches!(v, Value::Array(a) if a.iter().any(|e| compare(e, q).is_some_and(pred)))
        }
    }
}

fn operator_matches(field: Option<&Value>, op: &str, operand: &Value) -> bool {
    match op {
        "$eq" => eq_matches(field, operand),
        "$ne" => !eq_matches(field, operand),
        "$gt" => cmp_matches(field, operand, |o| o == Ordering::Greater),
        "$gte" => cmp_matches(field, operand, |o| o != Ordering::Less),
        "$lt" => cmp_matches(field, operand, |o| o == Ordering::Less),
        "$lte" => cmp_matches(field, operand, |o| o != Ordering::Greater),
        "$in" => operand
            .as_array()
            .unwrap()
            .iter()
            .any(|q| eq_matches(field, q)),
        "$nin" => !operand
            .as_array()
            .unwrap()
            .iter()
            .any(|q| eq_matches(field, q)),
        "$exists" => field.is_some() == operand.as_bool().unwrap(),
        "$not" => !condition_matches(field, operand),
        other => panic!("oracle: unsupported operator {other}"),
    }
}

/// A field condition: an operator object (all operators must hold) or a
/// plain value (equality)
fn condition_matches(field: Option<&Value>, condition: &Value) -> bool {
    match condition {
        Value::Object(ops) if ops.keys().all(|k| k.starts_with('$')) && !ops.is_empty() => ops
            .iter()
            .all(|(op, operand)| operator_matches(field, op, operand)),
        plain => eq_matches(field, plain),
    }
}

/// Whether `doc` matches `query`
pub fn matches(doc: &Map<String, Value>, query: &Value) -> bool {
    query
        .as_object()
        .unwrap()
        .iter()
        .all(|(key, value)| match key.as_str() {
            "$and" => value.as_array().unwrap().iter().all(|q| matches(doc, q)),
            "$or" => value.as_array().unwrap().iter().any(|q| matches(doc, q)),
            "$nor" => !value.as_array().unwrap().iter().any(|q| matches(doc, q)),
            path => condition_matches(get_path(doc, path), value),
        })
}

// ============================================================================
// Oracle: updates (the operators the state machine generates)
// ============================================================================

/// Apply `$set` / `$unset` / `$inc` on top-level fields as the engine does:
/// `$inc` adds integers as integers, otherwise as floats, and starts a
/// missing field at 0. `Err` when the engine rejects the update: `$inc` on a
/// non-numeric field, an int64 overflow, or a float that is not finite (JSON
/// has no infinity)
pub fn apply_update(doc: &mut Map<String, Value>, update: &Value) -> Result<(), String> {
    for (op, fields) in update.as_object().unwrap() {
        for (field, value) in fields.as_object().unwrap() {
            match op.as_str() {
                "$set" => {
                    doc.insert(field.clone(), value.clone());
                }
                "$unset" => {
                    doc.remove(field);
                }
                "$inc" => {
                    let current = doc.get(field).cloned().unwrap_or(json!(0));
                    let next = match (current.as_i64(), value.as_i64()) {
                        (Some(c), Some(i)) => json!(c
                            .checked_add(i)
                            .ok_or_else(|| format!("$inc {c} + {i} overflows"))?),
                        _ => match (current.as_f64(), value.as_f64()) {
                            (Some(c), Some(i)) if (c + i).is_finite() => json!(c + i),
                            (Some(c), Some(i)) => return Err(format!("$inc {c} + {i} overflows")),
                            _ => return Err(format!("$inc on non-number {current}")),
                        },
                    };
                    doc.insert(field.clone(), next);
                }
                other => panic!("oracle: unsupported update operator {other}"),
            }
        }
    }
    Ok(())
}

// ============================================================================
// Helpers
// ============================================================================

/// The `_id`s of documents, as their JSON text (ids may be of any type)
pub fn ids(docs: &[Value]) -> BTreeSet<String> {
    docs.iter().map(|d| d["_id"].to_string()).collect()
}

/// Whether two documents are stored differently (`1` vs `1.0` and `0.0` vs
/// `-0.0` differ): what an update's modified count reports
pub fn stored_differently(a: &Map<String, Value>, b: &Map<String, Value>) -> bool {
    serde_json::to_string(a).unwrap() != serde_json::to_string(b).unwrap()
}

/// A document as the insert APIs take it
pub fn fields(doc: &Map<String, Value>) -> HashMap<String, Value> {
    doc.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

// ============================================================================
// The engine behind one interface, for both storage backends
// ============================================================================

/// The operations the properties drive, on collection `c` of a database
pub trait Db {
    fn insert_docs(&self, c: &str, docs: &[Map<String, Value>]) -> Vec<Map<String, Value>>;
    fn update_first(&self, c: &str, filter: &Value, update: &Value) -> Result<(u64, u64), String>;
    fn update_all(&self, c: &str, filter: &Value, update: &Value) -> Result<(u64, u64), String>;
    fn delete_first(&self, c: &str, filter: &Value) -> u64;
    fn delete_all(&self, c: &str, filter: &Value) -> u64;
    fn create_index(&self, c: &str, path: &str) -> String;
    fn create_compound_index(&self, c: &str, paths: &[&str]) -> String;
    fn drop_index(&self, c: &str, name: &str);
    fn find(&self, c: &str, query: &Value) -> Result<Vec<Value>, String>;
    fn count(&self, c: &str, query: &Value) -> u64;
    fn find_one(&self, c: &str, query: &Value) -> Option<Value>;
    fn find_streaming(&self, c: &str, query: &Value) -> Vec<Value>;
    fn aggregate(&self, c: &str, pipeline: &Value) -> Vec<Value>;
    /// (live document count, whether `document_order` is exactly the
    /// catalog's ids, each once)
    fn storage_invariants(&self, c: &str) -> (u64, bool);
}

macro_rules! impl_db {
    ($storage:ty) => {
        impl Db for ironbase_core::DatabaseCore<$storage> {
            fn insert_docs(&self, c: &str, docs: &[Map<String, Value>]) -> Vec<Map<String, Value>> {
                self.collection(c).unwrap();
                if docs.is_empty() {
                    return Vec::new();
                }
                let ids = ironbase_core::DatabaseCore::<$storage>::insert_many(
                    self,
                    c,
                    docs.iter().map(fields).collect(),
                )
                .unwrap();
                docs.iter()
                    .zip(ids)
                    .map(|(doc, id)| {
                        let mut doc = doc.clone();
                        doc.insert("_id".into(), serde_json::to_value(id).unwrap());
                        doc
                    })
                    .collect()
            }
            fn update_first(
                &self,
                c: &str,
                filter: &Value,
                update: &Value,
            ) -> Result<(u64, u64), String> {
                ironbase_core::DatabaseCore::<$storage>::update_one(self, c, filter, update)
                    .map_err(|e| e.to_string())
            }
            fn update_all(
                &self,
                c: &str,
                filter: &Value,
                update: &Value,
            ) -> Result<(u64, u64), String> {
                ironbase_core::DatabaseCore::<$storage>::update_many(self, c, filter, update)
                    .map_err(|e| e.to_string())
            }
            fn delete_first(&self, c: &str, filter: &Value) -> u64 {
                ironbase_core::DatabaseCore::<$storage>::delete_one(self, c, filter).unwrap()
            }
            fn delete_all(&self, c: &str, filter: &Value) -> u64 {
                ironbase_core::DatabaseCore::<$storage>::delete_many(self, c, filter).unwrap()
            }
            fn create_index(&self, c: &str, path: &str) -> String {
                self.collection(c)
                    .unwrap()
                    .create_index(path.to_string(), false, false)
                    .unwrap()
            }
            fn create_compound_index(&self, c: &str, paths: &[&str]) -> String {
                self.collection(c)
                    .unwrap()
                    .create_compound_index(
                        paths.iter().map(|p| p.to_string()).collect(),
                        false,
                        false,
                    )
                    .unwrap()
            }
            fn drop_index(&self, c: &str, name: &str) {
                self.get_collection(c).unwrap().drop_index(name).unwrap()
            }
            fn find(&self, c: &str, query: &Value) -> Result<Vec<Value>, String> {
                self.get_collection(c)
                    .unwrap()
                    .find(query)
                    .map_err(|e| e.to_string())
            }
            fn count(&self, c: &str, query: &Value) -> u64 {
                self.get_collection(c)
                    .unwrap()
                    .count_documents(query)
                    .unwrap()
            }
            fn find_one(&self, c: &str, query: &Value) -> Option<Value> {
                self.get_collection(c).unwrap().find_one(query).unwrap()
            }
            fn find_streaming(&self, c: &str, query: &Value) -> Vec<Value> {
                let coll = self.get_collection(c).unwrap();
                let mut cursor = coll.find_streaming(query).unwrap();
                let mut docs = Vec::new();
                while let Some(doc) = cursor.next().unwrap() {
                    docs.push(doc);
                }
                docs
            }
            fn aggregate(&self, c: &str, pipeline: &Value) -> Vec<Value> {
                self.get_collection(c).unwrap().aggregate(pipeline).unwrap()
            }
            fn storage_invariants(&self, c: &str) -> (u64, bool) {
                #[allow(unused_imports)]
                use ironbase_core::storage::Storage;
                let coll = self.get_collection(c).unwrap();
                let storage = coll.storage.read();
                let meta = storage.get_collection_meta(c).unwrap();
                let order: std::collections::HashSet<_> = meta.document_order.iter().collect();
                let exact = order.len() == meta.document_order.len()
                    && order.len() == meta.document_catalog.len()
                    && meta.document_catalog.keys().all(|id| order.contains(id));
                (meta.live_document_count, exact)
            }
        }
    };
}

impl_db!(ironbase_core::storage::MemoryStorage);
impl_db!(ironbase_core::StorageEngine);
