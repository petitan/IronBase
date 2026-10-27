//! P6 (property-based testing plan): the B+ tree against a sorted multiset
//! of (key, document id) pairs.
//!
//! Random insert and delete sequences with repeated keys, mixed key types
//! (null, bool, int, float with `-0.0`, string, compound) and long keys that
//! force node splits; after them every query of the tree agrees with the
//! model: size, point search, count and scan over random ranges (inclusive
//! or not, skip, limit, ascending and descending), and the full scan in key
//! order. A second property checks that `IndexKey`'s order is a total order
//! consistent with its equality.

use ironbase_core::document::DocumentId;
use ironbase_core::index::{BPlusTree, IndexKey, OrderedFloat, RangeQueryMode, ScanOrder};
use proptest::prelude::*;
use std::cmp::Ordering;
use std::collections::BTreeMap;

fn cases(default: u32) -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn arb_scalar_key() -> impl Strategy<Value = IndexKey> {
    prop_oneof![
        1 => Just(IndexKey::Null),
        1 => any::<bool>().prop_map(IndexKey::Bool),
        4 => (-4i64..4).prop_map(IndexKey::Int),
        1 => any::<i64>().prop_map(IndexKey::Int),
        2 => prop::sample::select(vec![-0.0, 0.0, 0.5, -1.5, 1e300, 1e-310])
            .prop_map(|f| IndexKey::Float(OrderedFloat(f))),
        3 => prop::sample::select(vec!["", "a", "ab", "b", "é", "ő"]).prop_map(|s| IndexKey::String(s.into())),
        // Long keys fill pages and force splits early
        1 => (0u8..6, 150usize..400).prop_map(|(c, n)| IndexKey::String(((b'a' + c) as char).to_string().repeat(n))),
    ]
}

fn arb_key() -> impl Strategy<Value = IndexKey> {
    prop_oneof![
        6 => arb_scalar_key(),
        1 => prop::collection::vec(arb_scalar_key(), 2).prop_map(IndexKey::Compound),
    ]
}

#[derive(Debug, Clone)]
enum Op {
    Insert(IndexKey),
    /// Delete the model entry the index picks (modulo its size)
    Delete(usize),
    /// Delete a pair that is not in the tree
    DeleteMissing(IndexKey),
}

fn arb_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => arb_key().prop_map(Op::Insert),
        2 => any::<usize>().prop_map(Op::Delete),
        1 => arb_key().prop_map(Op::DeleteMissing),
    ]
}

/// A range: bounds, inclusiveness, skip, limit, order
#[derive(Debug, Clone)]
struct Range {
    start: IndexKey,
    end: IndexKey,
    start_incl: bool,
    end_incl: bool,
    skip: usize,
    limit: Option<usize>,
    desc: bool,
}

fn arb_range() -> impl Strategy<Value = Range> {
    let bound = prop_oneof![
        8 => arb_key(),
        1 => Just(IndexKey::Null),
        1 => Just(IndexKey::MaxKey),
    ];
    (
        bound.clone(),
        bound,
        any::<bool>(),
        any::<bool>(),
        0usize..5,
        prop::option::of(0usize..20),
        any::<bool>(),
    )
        .prop_map(
            |(start, end, start_incl, end_incl, skip, limit, desc)| Range {
                start,
                end,
                start_incl,
                end_incl,
                skip,
                limit,
                desc,
            },
        )
}

/// The model: each (key, id) pair once, sorted by key then id
type Model = BTreeMap<(IndexKey, i64), ()>;

fn in_range(key: &IndexKey, r: &Range) -> bool {
    let above = match key.cmp(&r.start) {
        Ordering::Greater => true,
        Ordering::Equal => r.start_incl,
        Ordering::Less => false,
    };
    let below = match key.cmp(&r.end) {
        Ordering::Less => true,
        Ordering::Equal => r.end_incl,
        Ordering::Greater => false,
    };
    above && below
}

fn id_of(id: &DocumentId) -> i64 {
    match id {
        DocumentId::Int(n) => *n,
        other => panic!("unexpected id {other:?}"),
    }
}

fn check(tree: &BPlusTree, model: &Model, ranges: &[Range]) -> Result<(), TestCaseError> {
    prop_assert_eq!(tree.size() as usize, model.len(), "size");

    // Full ascending scan: the model's pairs, keys in order
    let all = tree.range_query_pairs(
        &IndexKey::Null,
        &IndexKey::MaxKey,
        true,
        true,
        0,
        None,
        ScanOrder::Asc,
    );
    let mut got: Vec<(IndexKey, i64)> = all.iter().map(|(k, id)| (k.clone(), id_of(id))).collect();
    prop_assert!(
        got.windows(2).all(|w| w[0].0 <= w[1].0),
        "full scan out of key order"
    );
    got.sort();
    let want: Vec<(IndexKey, i64)> = model.keys().cloned().collect();
    prop_assert_eq!(&got, &want, "full scan");

    // Point search: an id with that key, or none
    for (key, _) in model.keys().take(10) {
        let found = tree.search(key).map(|id| id_of(&id));
        prop_assert!(
            found.is_some_and(|id| model.contains_key(&(key.clone(), id))),
            "search {:?} gave {:?}",
            key,
            found
        );
    }

    for r in ranges {
        let mut expected: Vec<(IndexKey, i64)> = model
            .keys()
            .filter(|(k, _)| in_range(k, r))
            .cloned()
            .collect();
        let count = tree
            .range_query(
                &r.start,
                &r.end,
                r.start_incl,
                r.end_incl,
                RangeQueryMode::Count,
            )
            .unwrap_count();
        prop_assert_eq!(count, expected.len(), "count {:?}", r);

        if r.desc {
            expected.reverse();
        }
        let order = if r.desc {
            ScanOrder::Desc
        } else {
            ScanOrder::Asc
        };
        let docs = tree
            .range_query(
                &r.start,
                &r.end,
                r.start_incl,
                r.end_incl,
                RangeQueryMode::Scan {
                    skip: r.skip,
                    limit: r.limit,
                    order,
                },
            )
            .unwrap_docs();
        let window: Vec<&(IndexKey, i64)> = expected
            .iter()
            .skip(r.skip)
            .take(r.limit.unwrap_or(usize::MAX))
            .collect();
        prop_assert_eq!(docs.len(), window.len(), "scan size {:?}", r);
        // Ties between equal keys may come in any id order: compare the
        // key sequence, and that each id holds the key at its position
        let key_of: BTreeMap<i64, &IndexKey> = model.keys().map(|(k, id)| (*id, k)).collect();
        for (doc, (key, _)) in docs.iter().zip(&window) {
            let id = id_of(doc);
            prop_assert_eq!(
                key_of.get(&id).copied(),
                Some(key),
                "scan {:?}: id {} at key {:?}",
                r,
                id,
                key
            );
        }
        let mut ids: Vec<i64> = docs.iter().map(id_of).collect();
        ids.sort();
        ids.dedup();
        prop_assert_eq!(ids.len(), docs.len(), "scan {:?} repeats an id", r);
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(cases(48)))]

    #[test]
    fn btree_agrees_with_a_sorted_multiset(
        ops in prop::collection::vec(arb_op(), 1..400),
        ranges in prop::collection::vec(arb_range(), 1..8),
    ) {
        let mut tree = BPlusTree::new("i".into(), "k".into(), false, false);
        let mut model = Model::new();
        for (n, op) in ops.iter().enumerate() {
            match op {
                Op::Insert(key) => {
                    tree.insert(key.clone(), DocumentId::Int(n as i64)).unwrap();
                    model.insert((key.clone(), n as i64), ());
                }
                Op::Delete(i) => {
                    if let Some((key, id)) = model.keys().nth(i.checked_rem(model.len()).unwrap_or(0)).cloned() {
                        tree.delete(&key, &DocumentId::Int(id)).unwrap();
                        model.remove(&(key, id));
                    }
                }
                Op::DeleteMissing(key) => {
                    let _ = tree.delete(key, &DocumentId::Int(-1));
                }
            }
        }
        check(&tree, &model, &ranges)?;
    }

    /// A unique tree accepts a key once; a rejected insert changes nothing
    #[test]
    fn unique_btree_rejects_exactly_the_repeated_keys(keys in prop::collection::vec(arb_key(), 1..200)) {
        let mut tree = BPlusTree::new("u".into(), "k".into(), true, false);
        let mut model = Model::new();
        for (n, key) in keys.iter().enumerate() {
            let taken = model.keys().any(|(k, _)| k == key);
            let result = tree.insert(key.clone(), DocumentId::Int(n as i64));
            prop_assert_eq!(result.is_err(), taken, "insert {:?}", key);
            if !taken {
                model.insert((key.clone(), n as i64), ());
            }
        }
        check(&tree, &model, &[])?;
    }

    /// `IndexKey`'s order is total and agrees with its equality
    #[test]
    fn index_key_order_is_total(a in arb_key(), b in arb_key(), c in arb_key()) {
        prop_assert_eq!(a.cmp(&b), b.cmp(&a).reverse(), "antisymmetry {:?} {:?}", a, b);
        prop_assert_eq!(a.cmp(&b) == Ordering::Equal, a == b, "Ord vs Eq {:?} {:?}", a, b);
        if a <= b && b <= c {
            prop_assert!(a <= c, "transitivity {:?} {:?} {:?}", a, b, c);
        }
    }
}
