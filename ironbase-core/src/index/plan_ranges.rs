//! Single source of truth for translating a [`QueryPlan`] into the index
//! key-ranges that `count`, `find` and the streaming cursor all scan.
//!
//! Audit 2026-06-06 finding #8: one `QueryPlan` was interpreted by **four**
//! separate executors (the two `count_with_plan` blocks, `collect_doc_ids_from_plan`
//! and `FindCursor::new_index_scan_from_plan`) that had to be hand-kept in
//! lockstep. A numeric Int/Float two-bucket fix once landed in three of them and
//! missed the cursor, shipping a bug where `find_streaming` silently dropped
//! Float-keyed docs. This module derives the ranges **once** so a fix lands once.
//!
//! [`PlanRanges::from_plan`] is pure `IndexKey`/`BPlusTree` math — it borrows a
//! `&BPlusTree` only to read a compound index's arity (`build_prefix_range`) and
//! touches no storage, catalog or lock. That keeps it safe to call while holding
//! any lock order (issue #75).
//!
//! Consumers differ only in *how* they read the ranges:
//! - count-exact: [`BPlusTree::count_exact`] — O(1) memory, no post-filter
//!   (caller must guarantee [`PlanRanges::exact`] and a non-multikey index);
//! - materialized find: [`BPlusTree::scan_ranges`] — caller post-filters;
//! - streaming cursor: a single contiguous range (see
//!   [`PlanRanges::single_contiguous`]).

use super::btree::{BPlusTree, RangeQueryMode, ScanOrder};
use super::key::{numeric_range_buckets, prefix_scan_upper_bound};
use super::IndexKey;
use crate::document::DocumentId;
use crate::error::{IronBaseError, Result};
use crate::query_planner::{QueryPlan, QueryPlanner};
use std::collections::HashMap;

/// One contiguous B+ tree key interval to scan.
#[derive(Debug, Clone)]
pub(crate) struct SubRange {
    pub start: IndexKey,
    pub end: IndexKey,
    pub inclusive_start: bool,
    pub inclusive_end: bool,
}

/// The index key-ranges derived from a [`QueryPlan`], plus whether the raw range
/// answer is exact for the count fast path.
#[derive(Debug, Clone)]
pub(crate) struct PlanRanges {
    /// Index the plan scans (every `QueryPlan` variant names one).
    pub index_name: String,
    /// 1 range (`IndexScan` / single-regex / sparse / single non-numeric range),
    /// 2 (numeric Int+Float buckets), or N (`$in` / multi-regex).
    pub sub_ranges: Vec<SubRange>,
    /// Whether `count` may sum the raw validated range counts WITHOUT a
    /// post-filter. This carries only the per-plan part of that decision —
    /// chiefly the `RegexPrefixScan` `exact && !case_insensitive` rule (count.rs,
    /// audit findings F + I). For a non-numeric range, `from_plan` builds a
    /// TYPE-TIGHT range (an absent bound is filled with the present bound's type
    /// min/max), so the range holds only the operator's type and stays exact —
    /// `{f: {$lt: "z"}}` scans `[String(""), "z")`, not `[Null, "z"]`, keeping the
    /// O(1) count fast path while excluding the Int/Float/Bool keys the operator
    /// can never match (finding #1 + perf follow-up, 2026-06-08). `exact` is false
    /// only for shapes a single range cannot tighten (a bound type with no scalar
    /// min/max sentinel, or a fully-unbounded range). The caller still ANDs `exact`
    /// with its own coverage + non-multikey gates; a non-exact plan routes count
    /// through the index-narrowed post-filter, matching find.
    pub exact: bool,
}

impl PlanRanges {
    /// Whether the ranges form a single contiguous interval — the streaming
    /// cursor's precondition (it cannot span the disjoint numeric Int/Float
    /// buckets or a `$in`/multi-regex union). Derived, never stored, so it can
    /// never desync from `sub_ranges`.
    pub(crate) fn single_contiguous(&self) -> bool {
        self.sub_ranges.len() == 1
    }

    fn single(
        index_name: String,
        start: IndexKey,
        end: IndexKey,
        inclusive_start: bool,
        inclusive_end: bool,
        exact: bool,
    ) -> Self {
        PlanRanges {
            index_name,
            sub_ranges: vec![SubRange {
                start,
                end,
                inclusive_start,
                inclusive_end,
            }],
            exact,
        }
    }

    fn multi(index_name: String, sub_ranges: Vec<SubRange>, exact: bool) -> Self {
        PlanRanges {
            index_name,
            sub_ranges,
            exact,
        }
    }

    /// Derive the index key-ranges a plan scans. Mirrors exactly what
    /// `collect_doc_ids_from_plan` (find) derives today, so the consumers below
    /// produce identical results.
    ///
    /// `index` is borrowed only for a compound `IndexScan`'s `build_prefix_range`
    /// (arity); no storage/catalog/lock is touched.
    pub(crate) fn from_plan(plan: &QueryPlan, index: &BPlusTree) -> Self {
        let index_name = plan.index_name().to_string();
        match plan {
            QueryPlan::IndexScan {
                key, is_compound, ..
            } => {
                let (start, end) = if *is_compound {
                    // Compound prefix: [prefix..Null, prefix..MaxKey].
                    index.build_prefix_range(key.clone())
                } else {
                    // Single-field equality point lookup.
                    (key.clone(), key.clone())
                };
                PlanRanges::single(index_name, start, end, true, true, true)
            }

            QueryPlan::IndexRangeScan {
                start,
                end,
                inclusive_start,
                inclusive_end,
                ..
            } => {
                if let Some(buckets) = numeric_range_buckets(
                    start.as_ref(),
                    end.as_ref(),
                    *inclusive_start,
                    *inclusive_end,
                ) {
                    // Numeric range spans two disjoint key buckets (all Int sort
                    // below all Float) — scan/count both so Float-keyed docs are
                    // not missed (finding D). One bucket may be absent.
                    let mut sub_ranges = Vec::with_capacity(2);
                    if let Some((s, e, is, ie)) = buckets.int_range {
                        sub_ranges.push(SubRange {
                            start: s,
                            end: e,
                            inclusive_start: is,
                            inclusive_end: ie,
                        });
                    }
                    let (fs, fe, fis, fie) = buckets.float_range;
                    sub_ranges.push(SubRange {
                        start: fs,
                        end: fe,
                        inclusive_start: fis,
                        inclusive_end: fie,
                    });
                    PlanRanges::multi(index_name, sub_ranges, true)
                } else {
                    // Non-numeric (string/bool) range. Make the scanned range
                    // TYPE-TIGHT: an absent bound is filled with the PRESENT bound's
                    // type min/max, so the range holds ONLY keys of that type. This
                    // keeps count's O(1) `count_exact` path valid (no cross-bucket
                    // keys to over-count) AND tightens the find/cursor scan — rather
                    // than widening to `[Null, hi]` / `[lo, MaxKey]` and leaning on a
                    // post-filter, which over-counted on the count fast path AND
                    // dropped count from O(1) to O(matches) even on a type-
                    // homogeneous field (finding #1 + perf follow-up, 2026-06-08).
                    //
                    // Bounds here are non-numeric (numeric goes through the bucket
                    // path) — String in practice, occasionally Bool. The planner
                    // already rejects mixed-type two-sided ranges, so a two-sided
                    // range shares one bucket. A FILLED bound is INCLUSIVE of the
                    // type boundary; the PRESENT bound keeps the plan's inclusivity.
                    // IndexKey order: Null < Bool < Int < Float < String < Compound
                    // < MaxKey. `String("")` is the lexicographically smallest string
                    // (so `[String(""), hi)` excludes every Bool/Int/Float/Null key);
                    // a single-field index stores no Compound keys, so `[lo, MaxKey]`
                    // with a String `lo` is already string-only (String is the top
                    // scalar bucket).
                    let (lo, lo_incl, hi, hi_incl, exact) = match (start.as_ref(), end.as_ref()) {
                        // Two-sided, same bucket → already type-tight.
                        (Some(s), Some(e))
                            if QueryPlanner::index_key_type_bucket(s)
                                == QueryPlanner::index_key_type_bucket(e) =>
                        {
                            (s.clone(), *inclusive_start, e.clone(), *inclusive_end, true)
                        }
                        // Open lower → the upper bound's type-minimum.
                        (None, Some(e @ IndexKey::String(_))) => (
                            IndexKey::String(String::new()),
                            true,
                            e.clone(),
                            *inclusive_end,
                            true,
                        ),
                        (None, Some(e @ IndexKey::Bool(_))) => {
                            (IndexKey::Bool(false), true, e.clone(), *inclusive_end, true)
                        }
                        // Open upper → the lower bound's type-maximum (MaxKey for a
                        // String lower — the top scalar bucket; Bool(true) for Bool).
                        (Some(s @ IndexKey::String(_)), None) => {
                            (s.clone(), *inclusive_start, IndexKey::MaxKey, true, true)
                        }
                        (Some(s @ IndexKey::Bool(_)), None) => (
                            s.clone(),
                            *inclusive_start,
                            IndexKey::Bool(true),
                            true,
                            true,
                        ),
                        // Unhandled bound type, both-absent, or (defensively) a
                        // two-sided cross-bucket range → widest range + post-filter.
                        _ => (
                            start.clone().unwrap_or(IndexKey::Null),
                            *inclusive_start,
                            end.clone().unwrap_or(IndexKey::MaxKey),
                            *inclusive_end,
                            false,
                        ),
                    };
                    PlanRanges::single(index_name, lo, hi, lo_incl, hi_incl, exact)
                }
            }

            QueryPlan::RegexPrefixScan {
                prefix,
                exact,
                case_insensitive,
                ..
            } => {
                let start = IndexKey::String(prefix.clone());
                // Half-open [prefix, upper) — see prefix_scan_upper_bound (H).
                let end = prefix_scan_upper_bound(prefix);
                // O(1) count over the raw range is correct only for an exact,
                // case-sensitive prefix: a non-exact regex (`^a.*b`) over-counts
                // the range, and a `(?i)` regex's Unicode case-folding differs
                // from the index's to_lowercase keys (findings F + I). Otherwise
                // the count must post-filter (exact = false).
                let exact = *exact && !*case_insensitive;
                PlanRanges::single(index_name, start, end, true, false, exact)
            }

            QueryPlan::SparseIndexScan { .. } => {
                // Sparse index holds only docs where the field exists → the whole
                // key space answers `$exists: true` exactly.
                PlanRanges::single(
                    index_name,
                    IndexKey::Null,
                    IndexKey::MaxKey,
                    true,
                    true,
                    true,
                )
            }

            QueryPlan::MultiRegexPrefixScan { prefixes, .. } => {
                let sub_ranges = prefixes
                    .iter()
                    .map(|p| SubRange {
                        start: IndexKey::String(p.clone()),
                        end: prefix_scan_upper_bound(p),
                        inclusive_start: true,
                        inclusive_end: false,
                    })
                    .collect();
                PlanRanges::multi(index_name, sub_ranges, true)
            }

            QueryPlan::MultiValueScan { keys, .. } => {
                // O(k) point lookups for `$in`. The planner dedups the keys, so
                // disjoint point ranges sum without double-counting (the multikey
                // gate handles array-valued docs separately).
                let sub_ranges = keys
                    .iter()
                    .map(|k| SubRange {
                        start: k.clone(),
                        end: k.clone(),
                        inclusive_start: true,
                        inclusive_end: true,
                    })
                    .collect();
                PlanRanges::multi(index_name, sub_ranges, true)
            }
        }
    }
}

impl BPlusTree {
    /// Exact count of catalog-validated documents across all of a plan's
    /// sub-ranges. O(1) memory; no post-filter.
    ///
    /// The caller MUST guarantee [`PlanRanges::exact`] and a non-multikey index
    /// (otherwise summing raw range counts diverges from the query result — use
    /// [`Self::scan_ranges`] + an index-narrowed post-filter instead).
    ///
    /// Subsumes the former `count_numeric_buckets` (the numeric case is just a
    /// two-sub-range `PlanRanges`).
    pub(crate) fn count_exact(
        &self,
        ranges: &PlanRanges,
        catalog: &HashMap<DocumentId, u64>,
    ) -> usize {
        ranges
            .sub_ranges
            .iter()
            .map(|r| {
                self.count_range_validated(
                    &r.start,
                    &r.end,
                    r.inclusive_start,
                    r.inclusive_end,
                    catalog,
                )
            })
            .sum()
    }

    /// Union of document ids across all of a plan's sub-ranges, in scan order.
    ///
    /// A single range is returned verbatim so the caller's own index-sort order
    /// and `HashSet::retain` dedup are preserved (find relies on this). Multiple
    /// ranges (numeric buckets, `$in`, multi-regex) are concatenated then sorted
    /// and de-duplicated — matching the former `scan_numeric_buckets`/`$in` loops —
    /// because a doc may appear in more than one sub-range.
    ///
    /// OOM-safe at the UNION level: each sub-range's docs are `try_reserve`d
    /// before being appended, so accumulating across many sub-ranges
    /// (`$in`/multi-regex) returns an error instead of aborting. A single
    /// sub-range whose own `range_query` materializes more docs than fit in RAM
    /// still aborts inside `range_query` (the unbounded inner `Vec`) — the same
    /// exposure as the single-range path and the former `scan_numeric_buckets`,
    /// not made worse here. This avoids the separate O(range) Count-mode pass the
    /// unified path would otherwise add — a doubled descent (Count then Scan) is
    /// wasted work on the hot numeric-range find/`$match` path.
    ///
    /// Subsumes the former `scan_numeric_buckets`.
    pub(crate) fn scan_ranges(&self, ranges: &PlanRanges) -> Result<Vec<DocumentId>> {
        let scan_mode = || RangeQueryMode::Scan {
            skip: 0,
            limit: None,
            order: ScanOrder::Asc,
        };

        if ranges.sub_ranges.len() == 1 {
            let r = &ranges.sub_ranges[0];
            return Ok(self
                .range_query(
                    &r.start,
                    &r.end,
                    r.inclusive_start,
                    r.inclusive_end,
                    scan_mode(),
                )
                .unwrap_docs());
        }

        let mut ids: Vec<DocumentId> = Vec::new();
        for r in &ranges.sub_ranges {
            let chunk = self
                .range_query(
                    &r.start,
                    &r.end,
                    r.inclusive_start,
                    r.inclusive_end,
                    scan_mode(),
                )
                .unwrap_docs();
            ids.try_reserve(chunk.len()).map_err(|e| {
                IronBaseError::OutOfMemory(format!(
                    "Cannot allocate {} index entries in scan_ranges ({e})",
                    chunk.len()
                ))
            })?;
            ids.extend(chunk);
        }
        // Dedup across overlapping/duplicate-doc sub-ranges (a multikey doc may
        // land in both the Int and Float bucket; find dedups again, harmlessly).
        ids.sort_unstable();
        ids.dedup();
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::key::OrderedFloat;

    fn tree() -> BPlusTree {
        BPlusTree::new("idx".to_string(), "f".to_string(), false, false)
    }

    fn range_scan(start: Option<IndexKey>, end: Option<IndexKey>, is: bool, ie: bool) -> QueryPlan {
        QueryPlan::IndexRangeScan {
            index_name: "idx".to_string(),
            field: "f".to_string(),
            start,
            end,
            inclusive_start: is,
            inclusive_end: ie,
        }
    }

    #[test]
    fn index_scan_single_point_exact_contiguous() {
        let plan = QueryPlan::IndexScan {
            index_name: "idx".to_string(),
            field: "f".to_string(),
            key: IndexKey::Int(7),
            is_compound: false,
        };
        let r = PlanRanges::from_plan(&plan, &tree());
        assert_eq!(r.index_name, "idx");
        assert_eq!(r.sub_ranges.len(), 1);
        assert!(r.exact);
        assert!(r.single_contiguous());
        let sr = &r.sub_ranges[0];
        assert_eq!(sr.start, IndexKey::Int(7));
        assert_eq!(sr.end, IndexKey::Int(7));
        assert!(sr.inclusive_start && sr.inclusive_end);
    }

    #[test]
    fn compound_index_scan_uses_prefix_range() {
        let mut compound =
            BPlusTree::new_compound("c".to_string(), vec!["a".into(), "b".into()], false, false);
        compound
            .insert(
                IndexKey::Compound(vec![IndexKey::Int(1), IndexKey::Int(2)]),
                DocumentId::Int(1),
            )
            .unwrap();
        let plan = QueryPlan::IndexScan {
            index_name: "c".to_string(),
            field: "a".to_string(),
            key: IndexKey::Int(1),
            is_compound: true,
        };
        let r = PlanRanges::from_plan(&plan, &compound);
        assert_eq!(r.sub_ranges.len(), 1);
        assert!(r.single_contiguous() && r.exact);
        // build_prefix_range wraps the key in a Compound with Null/MaxKey tail.
        let sr = &r.sub_ranges[0];
        assert!(matches!(sr.start, IndexKey::Compound(_)));
        assert!(matches!(sr.end, IndexKey::Compound(_)));
    }

    #[test]
    fn numeric_range_splits_into_two_buckets() {
        // {f: {$gte: 18, $lt: 65}} → Int bucket + Float bucket.
        let plan = range_scan(
            Some(IndexKey::Int(18)),
            Some(IndexKey::Int(65)),
            true,
            false,
        );
        let r = PlanRanges::from_plan(&plan, &tree());
        assert_eq!(r.sub_ranges.len(), 2, "numeric range = Int + Float buckets");
        assert!(!r.single_contiguous(), "two buckets are not streamable");
        assert!(r.exact);
        // First sub-range is the Int bucket, second the Float bucket.
        assert!(matches!(r.sub_ranges[0].start, IndexKey::Int(_)));
        assert!(matches!(r.sub_ranges[1].start, IndexKey::Float(_)));
    }

    #[test]
    fn float_only_numeric_range_buckets() {
        // Float bounds that CONTAIN integers (2..=9) → Int bucket present → two
        // sub-ranges (Int then Float). This pins the finding-D regression guard.
        let with_ints = range_scan(
            Some(IndexKey::Float(OrderedFloat(1.5))),
            Some(IndexKey::Float(OrderedFloat(9.5))),
            true,
            true,
        );
        let r = PlanRanges::from_plan(&with_ints, &tree());
        assert_eq!(r.sub_ranges.len(), 2, "Int bucket (2..=9) + Float bucket");
        assert!(matches!(r.sub_ranges[0].start, IndexKey::Int(_)));
        assert!(matches!(r.sub_ranges[1].start, IndexKey::Float(_)));
        assert!(!r.single_contiguous());
        assert!(r.exact);

        // Float bounds with NO integer between them (ceil 1.2 = 2 > floor 1.8 = 1)
        // → Int bucket absent → a single Float sub-range, which IS streamable.
        let no_ints = range_scan(
            Some(IndexKey::Float(OrderedFloat(1.2))),
            Some(IndexKey::Float(OrderedFloat(1.8))),
            true,
            true,
        );
        let r = PlanRanges::from_plan(&no_ints, &tree());
        assert_eq!(
            r.sub_ranges.len(),
            1,
            "no integer in (1.2, 1.8) → Float bucket only"
        );
        assert!(r.single_contiguous());
        assert!(matches!(r.sub_ranges[0].start, IndexKey::Float(_)));
        assert!(r.exact);
    }

    #[test]
    fn string_range_is_single_contiguous() {
        let plan = range_scan(
            Some(IndexKey::String("a".into())),
            Some(IndexKey::String("z".into())),
            true,
            true,
        );
        let r = PlanRanges::from_plan(&plan, &tree());
        assert_eq!(r.sub_ranges.len(), 1);
        assert!(r.single_contiguous() && r.exact);
    }

    #[test]
    fn unbounded_string_upper_defaults_to_maxkey() {
        // {f: {$gte: "abc"}} → [String("abc"), MaxKey]; String is the top scalar
        // bucket on a single-field index, so this is exact (O(1) count).
        let plan = range_scan(Some(IndexKey::String("abc".into())), None, true, true);
        let r = PlanRanges::from_plan(&plan, &tree());
        assert_eq!(r.sub_ranges.len(), 1);
        assert!(r.single_contiguous());
        assert_eq!(r.sub_ranges[0].start, IndexKey::String("abc".into()));
        assert_eq!(r.sub_ranges[0].end, IndexKey::MaxKey);
        assert!(r.exact, "open-upper string range is type-tight → exact");
    }

    #[test]
    fn unbounded_string_lower_is_type_tight_and_exact() {
        // {f: {$lt: "z"}} → open lower filled with String("") (the lex-min string),
        // NOT Null — so the range is string-only and EXACT (keeps the O(1) count
        // fast path; the perf regression of routing it to the narrow path is gone).
        let plan = range_scan(None, Some(IndexKey::String("z".into())), true, false);
        let r = PlanRanges::from_plan(&plan, &tree());
        assert_eq!(r.sub_ranges.len(), 1);
        assert!(r.single_contiguous());
        let sr = &r.sub_ranges[0];
        assert_eq!(
            sr.start,
            IndexKey::String(String::new()),
            "lower = String(\"\"), not Null"
        );
        assert!(sr.inclusive_start, "filled lower bound is inclusive");
        assert_eq!(sr.end, IndexKey::String("z".into()));
        assert!(!sr.inclusive_end, "$lt → exclusive upper preserved");
        assert!(r.exact, "type-tight string range is exact");
    }

    #[test]
    fn unbounded_bool_bounds_are_type_tight() {
        // {f: {$gte: true}} → [Bool(true), Bool(true)] (open upper filled with the
        // Bool max, not MaxKey), exact and bool-only.
        let gte = range_scan(Some(IndexKey::Bool(true)), None, true, true);
        let r = PlanRanges::from_plan(&gte, &tree());
        assert_eq!(r.sub_ranges[0].start, IndexKey::Bool(true));
        assert_eq!(r.sub_ranges[0].end, IndexKey::Bool(true));
        assert!(r.exact, "bool open-upper is type-tight → exact");

        // {f: {$lt: true}} → [Bool(false), Bool(true)) (open lower filled with the
        // Bool min), exact.
        let lt = range_scan(None, Some(IndexKey::Bool(true)), true, false);
        let r = PlanRanges::from_plan(&lt, &tree());
        assert_eq!(r.sub_ranges[0].start, IndexKey::Bool(false));
        assert!(r.sub_ranges[0].inclusive_start);
        assert_eq!(r.sub_ranges[0].end, IndexKey::Bool(true));
        assert!(!r.sub_ranges[0].inclusive_end);
        assert!(r.exact, "bool open-lower is type-tight → exact");
    }

    #[test]
    fn regex_exact_cs_is_exact() {
        let plan = QueryPlan::RegexPrefixScan {
            index_name: "idx".to_string(),
            field: "f".to_string(),
            prefix: "Al".to_string(),
            exact: true,
            case_insensitive: false,
        };
        let r = PlanRanges::from_plan(&plan, &tree());
        assert_eq!(r.sub_ranges.len(), 1);
        assert!(r.single_contiguous());
        assert!(r.exact, "exact && !ci ⇒ exact");
        let sr = &r.sub_ranges[0];
        assert_eq!(sr.start, IndexKey::String("Al".into()));
        assert!(
            sr.inclusive_start && !sr.inclusive_end,
            "half-open [prefix, upper)"
        );
    }

    #[test]
    fn regex_nonexact_or_ci_is_not_exact() {
        for (exact, ci) in [(false, false), (true, true), (false, true)] {
            let plan = QueryPlan::RegexPrefixScan {
                index_name: "idx".to_string(),
                field: "f".to_string(),
                prefix: "al".to_string(),
                exact,
                case_insensitive: ci,
            };
            let r = PlanRanges::from_plan(&plan, &tree());
            assert!(!r.exact, "non-exact OR ci ⇒ post-filter (exact=false)");
            assert!(r.single_contiguous(), "regex is still a single range");
        }
    }

    #[test]
    fn sparse_scan_covers_whole_keyspace() {
        let plan = QueryPlan::SparseIndexScan {
            index_name: "idx".to_string(),
            field: "f".to_string(),
        };
        let r = PlanRanges::from_plan(&plan, &tree());
        assert_eq!(r.sub_ranges.len(), 1);
        assert!(r.single_contiguous() && r.exact);
        assert_eq!(r.sub_ranges[0].start, IndexKey::Null);
        assert_eq!(r.sub_ranges[0].end, IndexKey::MaxKey);
    }

    #[test]
    fn multi_value_scan_is_multi_range_not_contiguous() {
        let plan = QueryPlan::MultiValueScan {
            index_name: "idx".to_string(),
            field: "f".to_string(),
            keys: vec![IndexKey::Int(1), IndexKey::Int(2), IndexKey::Int(3)],
        };
        let r = PlanRanges::from_plan(&plan, &tree());
        assert_eq!(r.sub_ranges.len(), 3);
        assert!(!r.single_contiguous(), "$in union is not streamable");
        assert!(r.exact);
        for sr in &r.sub_ranges {
            assert_eq!(sr.start, sr.end, "each $in key is a point lookup");
        }
    }

    #[test]
    fn single_key_in_is_contiguous() {
        // A one-element $in collapses to a single point range — streamable.
        let plan = QueryPlan::MultiValueScan {
            index_name: "idx".to_string(),
            field: "f".to_string(),
            keys: vec![IndexKey::Int(42)],
        };
        let r = PlanRanges::from_plan(&plan, &tree());
        assert_eq!(r.sub_ranges.len(), 1);
        assert!(r.single_contiguous());
    }

    #[test]
    fn multi_regex_is_multi_range() {
        let plan = QueryPlan::MultiRegexPrefixScan {
            index_name: "idx".to_string(),
            field: "f".to_string(),
            prefixes: vec!["A".into(), "B".into()],
        };
        let r = PlanRanges::from_plan(&plan, &tree());
        assert_eq!(r.sub_ranges.len(), 2);
        assert!(!r.single_contiguous());
        for sr in &r.sub_ranges {
            assert!(sr.inclusive_start && !sr.inclusive_end);
        }
    }

    // ---- consumer smoke tests (count_exact / scan_ranges wire-up) ----

    fn catalog_of(ids: &[DocumentId]) -> HashMap<DocumentId, u64> {
        ids.iter().map(|d| (d.clone(), 0u64)).collect()
    }

    #[test]
    fn scan_and_count_single_range() {
        let mut t = tree();
        for i in 1..=5 {
            t.insert(IndexKey::Int(i), DocumentId::Int(i as i64))
                .unwrap();
        }
        let cat = catalog_of(&(1..=5).map(DocumentId::Int).collect::<Vec<_>>());
        // {f: {$gte: 2, $lte: 4}} (numeric → two buckets, Int bucket carries all).
        let plan = range_scan(Some(IndexKey::Int(2)), Some(IndexKey::Int(4)), true, true);
        let r = PlanRanges::from_plan(&plan, &t);
        let docs = t.scan_ranges(&r).unwrap();
        assert_eq!(docs.len(), 3, "ids 2,3,4");
        assert_eq!(t.count_exact(&r, &cat), 3);
    }

    #[test]
    fn scan_ranges_dedups_multikey_across_buckets() {
        // One doc indexed under both an Int and a Float key inside the range must
        // appear once after scan_ranges' multi-range dedup.
        let mut t = tree();
        t.insert(IndexKey::Int(3), DocumentId::Int(1)).unwrap();
        t.insert(IndexKey::Float(OrderedFloat(3.5)), DocumentId::Int(1))
            .unwrap();
        let plan = range_scan(Some(IndexKey::Int(2)), Some(IndexKey::Int(4)), true, true);
        let r = PlanRanges::from_plan(&plan, &t);
        assert_eq!(r.sub_ranges.len(), 2);
        let docs = t.scan_ranges(&r).unwrap();
        assert_eq!(docs, vec![DocumentId::Int(1)], "deduped across buckets");
    }

    #[test]
    fn count_exact_sums_in_keys() {
        let mut t = tree();
        for i in 1..=4 {
            t.insert(IndexKey::Int(i), DocumentId::Int(i as i64))
                .unwrap();
        }
        let cat = catalog_of(&(1..=4).map(DocumentId::Int).collect::<Vec<_>>());
        let plan = QueryPlan::MultiValueScan {
            index_name: "idx".to_string(),
            field: "f".to_string(),
            keys: vec![IndexKey::Int(1), IndexKey::Int(3)],
        };
        let r = PlanRanges::from_plan(&plan, &t);
        assert_eq!(t.count_exact(&r, &cat), 2);
        assert_eq!(t.scan_ranges(&r).unwrap().len(), 2);
    }
}
