// src/query_planner.rs
// Query planner and optimizer - index selection

use crate::index::{Histogram, IndexKey, IndexPrefixInfo, MostCommonValues};
use serde_json::Value;

/// Parsed regex prefix information
#[derive(Debug, Clone, PartialEq)]
pub struct RegexPrefixInfo {
    /// The extracted literal prefix
    pub prefix: String,
    /// True if the regex is just the prefix (no trailing wildcards)
    pub exact: bool,
    /// True if the regex has (?i) flag
    pub case_insensitive: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalOperator {
    And,
    Or,
    Nor,
}

impl RegexPrefixInfo {
    /// Check if this regex prefix can be optimized with a standard index
    #[allow(dead_code)]
    pub fn is_optimizable(&self) -> bool {
        !self.prefix.is_empty() && !self.case_insensitive
    }

    /// Check if this regex prefix can be optimized for $in (must be exact and non-CI)
    pub fn is_optimizable_for_in(&self) -> bool {
        !self.prefix.is_empty() && self.exact && !self.case_insensitive
    }
}

/// A candidate query plan with cost estimation for planner selection
#[derive(Debug, Clone)]
pub struct CandidatePlan {
    /// The query plan
    pub plan: QueryPlan,
    /// The field this plan operates on
    pub field: String,
    /// Estimated cost (lower is better) - based on selectivity heuristics
    pub estimated_cost: f64,
    /// Human-readable reason for this plan (for explain/debug)
    pub reason: String,
}

impl CandidatePlan {
    /// Create a new candidate plan with computed cost
    pub fn new(
        plan: QueryPlan,
        field: String,
        estimated_cost: f64,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            plan,
            field,
            estimated_cost,
            reason: reason.into(),
        }
    }

    /// Create a candidate with default cost and auto-generated reason
    /// Used when no statistics are available - uses high cost to deprioritize
    pub fn with_default_cost(plan: QueryPlan, field: String, index_name: &str) -> Self {
        let reason = format!("Index {} on field {} (no stats)", index_name, field);
        // High cost when no statistics available - prefer indexes with known stats
        Self::new(plan, field, 1_000_000.0, reason)
    }

    /// Create a candidate with selectivity-based cost
    /// Lower distinct_count = higher selectivity = lower cost
    /// multikey_ratio (0.0-1.0) indicates how many docs have array values
    pub fn with_selectivity(
        plan: QueryPlan,
        field: String,
        index_name: &str,
        distinct_count: u64,
        total_docs: u64,
        multikey_ratio: f32,
    ) -> Self {
        // Use actual total_docs - no artificial minimum
        // If total_docs is 0, use 1 to avoid division by zero issues
        let total_docs = total_docs.max(1);
        let base_cost = if distinct_count > 0 {
            // Selectivity = 1 / distinct_count
            // Estimated rows = total_docs / distinct_count
            // Cost = estimated rows (lower is better)
            (total_docs as f64) / (distinct_count as f64)
        } else {
            // Unknown selectivity - use total_docs (worst case: scan all)
            total_docs as f64
        };

        // Multikey indexes have more entries per document (one per array element)
        // This means scanning might need to read more index entries
        // Overhead: 1.0 (no multikey) to 1.5 (25% of docs have arrays with avg 3 elements)
        let multikey_overhead = 1.0 + (multikey_ratio as f64 * 0.5);
        let estimated_cost = base_cost * multikey_overhead;

        let reason = format!(
            "Index {} on field {} (distinct: {}, est. rows: {:.0})",
            index_name, field, distinct_count, estimated_cost
        );
        Self::new(plan, field, estimated_cost, reason)
    }

    /// Create a candidate with MCV-aware selectivity estimation
    ///
    /// Uses Most Common Values (MCV) statistics when available for more accurate
    /// selectivity estimates on skewed distributions. Falls back to uniform
    /// distribution if MCV is not available.
    ///
    /// # Algorithm
    ///
    /// 1. If query value is in MCV: selectivity = freq(value) / total_keys
    /// 2. If not in MCV: uniform estimate over remaining values
    pub fn with_selectivity_mcv(
        plan: QueryPlan,
        field: String,
        index_name: &str,
        query_value: &IndexKey,
        distinct_count: u64,
        total_docs: u64,
        multikey_ratio: f32,
        mcv: Option<&MostCommonValues>,
    ) -> Self {
        let total_docs = total_docs.max(1);

        let (selectivity, estimation_method) = if let Some(mcv) = mcv {
            if mcv.is_valid() {
                let sel = mcv.estimate_selectivity(query_value, distinct_count);
                (sel, "mcv")
            } else if distinct_count > 0 {
                (1.0 / distinct_count as f64, "uniform")
            } else {
                (1.0, "full-scan")
            }
        } else if distinct_count > 0 {
            (1.0 / distinct_count as f64, "uniform")
        } else {
            (1.0, "full-scan")
        };

        let base_cost = selectivity * total_docs as f64;
        let multikey_overhead = 1.0 + (multikey_ratio as f64 * 0.5);
        let estimated_cost = base_cost * multikey_overhead;

        let reason = format!(
            "Index {} on field {} (selectivity: {:.4}, method: {}, est. rows: {:.0})",
            index_name, field, selectivity, estimation_method, estimated_cost
        );
        Self::new(plan, field, estimated_cost, reason)
    }

    /// Create a candidate with histogram-based range selectivity
    ///
    /// For range queries ($gt, $gte, $lt, $lte), use histogram to estimate selectivity
    /// instead of uniform 1/distinct_count assumption. This provides much more accurate
    /// estimates for skewed distributions.
    ///
    /// Falls back to uniform estimate (0.33) if histogram is not available.
    pub fn with_range_selectivity(
        plan: QueryPlan,
        field: String,
        index_name: &str,
        histogram: Option<&Histogram>,
        start: Option<&IndexKey>,
        end: Option<&IndexKey>,
        total_docs: u64,
        multikey_ratio: f32,
    ) -> Self {
        // Use actual total_docs - no artificial minimum
        let total_docs = total_docs.max(1);

        // Use histogram if available, otherwise fall back to uniform estimate
        let (selectivity, estimation_method) = if let Some(hist) = histogram {
            let sel = hist.estimate_range_selectivity(start, end);
            (sel, "histogram")
        } else {
            // Fallback: assume range covers 33% of values (uniform distribution)
            (0.33, "uniform")
        };

        // Estimated rows = total_docs * selectivity
        let base_cost = (total_docs as f64) * selectivity;

        // Apply multikey overhead
        let multikey_overhead = 1.0 + (multikey_ratio as f64 * 0.5);
        let estimated_cost = base_cost * multikey_overhead;

        let reason = format!(
            "Index {} on field {} (range, sel: {:.1}%, est. rows: {:.0}, method: {})",
            index_name,
            field,
            selectivity * 100.0,
            estimated_cost,
            estimation_method
        );
        Self::new(plan, field, estimated_cost, reason)
    }
}

/// Query plan - describes how to execute a query
/// NOTE: CollectionScan variant was removed - analyze_query() returns None for full scan,
/// and explain_query() handles None case by generating "CollectionScan" JSON directly.
#[derive(Debug, Clone)]
#[allow(clippy::enum_variant_names)]
pub enum QueryPlan {
    /// Index scan for equality match
    IndexScan {
        index_name: String,
        field: String,
        key: IndexKey,
        /// If true, this is a compound index prefix query - use range scan internally
        is_compound: bool,
    },

    /// Index range scan
    IndexRangeScan {
        index_name: String,
        field: String,
        start: Option<IndexKey>,
        end: Option<IndexKey>,
        inclusive_start: bool,
        inclusive_end: bool,
    },

    /// Sparse index scan for $exists: true queries
    /// Returns all doc_ids in the sparse index (which only contains docs where field exists)
    #[allow(dead_code)]
    SparseIndexScan { index_name: String, field: String },

    /// Regex prefix scan (e.g., /^prefix/)
    RegexPrefixScan {
        index_name: String,
        field: String,
        prefix: String,
        exact: bool,
        /// If true, uses case-insensitive index with lowercased prefix
        case_insensitive: bool,
    },

    /// Multi-regex prefix scan for $in with regex patterns
    /// e.g., { field: { $in: [{"$regex": "^A"}, {"$regex": "^B"}] } }
    MultiRegexPrefixScan {
        index_name: String,
        field: String,
        prefixes: Vec<String>,
    },

    /// Multi-value scan for $in with plain values
    /// e.g., { _id: { $in: ["a", "b", "c"] } }
    /// Performs O(k) index lookups instead of O(n) collection scan
    MultiValueScan {
        index_name: String,
        field: String,
        keys: Vec<IndexKey>,
    },
}

/// Query planner - analyzes queries and selects optimal execution plan
pub struct QueryPlanner;

impl QueryPlanner {
    /// Extract top-level logical operator and its clauses if present.
    ///
    /// Only matches when the query contains a single top-level logical operator.
    pub fn extract_logical_clauses(query_json: &Value) -> Option<(LogicalOperator, Vec<Value>)> {
        let obj = query_json.as_object()?;
        if obj.len() != 1 {
            return None;
        }

        let (op, clauses) = if let Some(Value::Array(clauses)) = obj.get("$and") {
            (LogicalOperator::And, clauses)
        } else if let Some(Value::Array(clauses)) = obj.get("$or") {
            (LogicalOperator::Or, clauses)
        } else if let Some(Value::Array(clauses)) = obj.get("$nor") {
            (LogicalOperator::Nor, clauses)
        } else {
            return None;
        };

        if clauses.is_empty() {
            return None;
        }

        Some((op, clauses.clone()))
    }

    /// Merge $and/$or same-field clauses into implicit form for better index usage.
    ///
    /// Returns `Some(merged_query)` if the clauses can be rewritten as an equivalent
    /// implicit query that the standard planner can optimize. Returns `None` if the
    /// clauses cannot be safely merged.
    ///
    /// **$and merge rules:**
    /// - Same field, range operators ($gte/$gt/$lte/$lt) → single condition object
    /// - Conflicting ranges (e.g., 2× $gte) → tighter bound (max lower, min upper)
    /// - Different fields (1 clause per field) → implicit AND: {field1:cond1, field2:cond2}
    /// - Bare equality {field: val} → {field: {$eq: val}}
    /// - Unsupported ($regex, $fuzzy, $text, nested $and/$or, multi-field clause) → None
    ///
    /// **$or merge rules:**
    /// - Same field, all simple equality → {field: {$in: [val1, val2, ...]}}
    /// - Otherwise → None
    pub fn try_merge_logical_clauses(
        logical_op: LogicalOperator,
        clauses: &[Value],
        index_fields: &[IndexPrefixInfo],
    ) -> Option<Value> {
        match logical_op {
            LogicalOperator::And => Self::try_merge_and_clauses(clauses, index_fields),
            LogicalOperator::Or => Self::try_merge_or_clauses(clauses, index_fields),
            LogicalOperator::Nor => None, // $nor: complex semantics, never merge
        }
    }

    /// Try to merge $and clauses into implicit form.
    fn try_merge_and_clauses(clauses: &[Value], index_fields: &[IndexPrefixInfo]) -> Option<Value> {
        use serde_json::Map;

        // Collect per-field conditions from all clauses
        // Key: field name, Value: list of (operator, value) pairs
        let mut field_conditions: std::collections::HashMap<String, Vec<(String, Value)>> =
            std::collections::HashMap::new();

        for clause in clauses {
            let obj = clause.as_object()?;

            // Multi-field clause → cannot merge
            if obj.len() != 1 {
                return None;
            }

            let (field, value) = obj.iter().next()?;

            // Nested logical operators → cannot merge
            if field.starts_with('$') {
                return None;
            }

            // Check what kind of condition this is
            match value {
                // Bare equality: {field: "value"} or {field: 123}
                Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null => {
                    field_conditions
                        .entry(field.clone())
                        .or_default()
                        .push(("$eq".to_string(), value.clone()));
                }
                // Operator object: {field: {$gte: 10, $lte: 20}}
                Value::Object(cond_map) => {
                    for (op, val) in cond_map {
                        // Unsupported operators → cannot merge
                        match op.as_str() {
                            "$gt" | "$gte" | "$lt" | "$lte" | "$eq" | "$ne" | "$in" | "$nin"
                            | "$exists" | "$type" => {}
                            _ => return None, // $regex, $fuzzy, $text, $elemMatch, etc.
                        }
                        field_conditions
                            .entry(field.clone())
                            .or_default()
                            .push((op.clone(), val.clone()));
                    }
                }
                // Array or other → cannot merge
                _ => return None,
            }
        }

        // Check that at least one field has an index
        let has_indexed_field = field_conditions
            .keys()
            .any(|field| Self::find_index_for_field(field, index_fields).is_some());
        if !has_indexed_field {
            return None;
        }

        // Build merged query: {field1: {$op1: val1, $op2: val2}, field2: ...}
        let mut merged = Map::new();

        for (field, conditions) in &field_conditions {
            // Resolve conflicting range operators for the same field
            let resolved = Self::resolve_range_conditions(conditions)?;
            merged.insert(field.clone(), resolved);
        }

        Some(Value::Object(merged))
    }

    /// Resolve potentially conflicting conditions on a single field into one condition object.
    ///
    /// For conflicting bounds (e.g., 2× $gte), picks the tighter bound.
    fn resolve_range_conditions(conditions: &[(String, Value)]) -> Option<Value> {
        use serde_json::Map;

        // Single condition shortcut
        if conditions.len() == 1 {
            let (op, val) = &conditions[0];
            if op == "$eq" {
                // Bare equality stays as operator form for planner
                let mut m = Map::new();
                m.insert("$eq".to_string(), val.clone());
                return Some(Value::Object(m));
            }
            let mut m = Map::new();
            m.insert(op.clone(), val.clone());
            return Some(Value::Object(m));
        }

        let mut result = Map::new();

        // Track bound values for conflict resolution
        let mut gte_val: Option<&Value> = None;
        let mut gt_val: Option<&Value> = None;
        let mut lte_val: Option<&Value> = None;
        let mut lt_val: Option<&Value> = None;
        let mut eq_vals: Vec<&Value> = Vec::new();

        for (op, val) in conditions {
            match op.as_str() {
                "$gte" => {
                    gte_val = Some(match gte_val {
                        Some(existing) => {
                            // Tighter bound: max of the two
                            if Self::compare_json_for_bound(val, existing)
                                == std::cmp::Ordering::Greater
                            {
                                val
                            } else {
                                existing
                            }
                        }
                        None => val,
                    });
                }
                "$gt" => {
                    gt_val = Some(match gt_val {
                        Some(existing) => {
                            if Self::compare_json_for_bound(val, existing)
                                == std::cmp::Ordering::Greater
                            {
                                val
                            } else {
                                existing
                            }
                        }
                        None => val,
                    });
                }
                "$lte" => {
                    lte_val = Some(match lte_val {
                        Some(existing) => {
                            // Tighter bound: min of the two
                            if Self::compare_json_for_bound(val, existing)
                                == std::cmp::Ordering::Less
                            {
                                val
                            } else {
                                existing
                            }
                        }
                        None => val,
                    });
                }
                "$lt" => {
                    lt_val = Some(match lt_val {
                        Some(existing) => {
                            if Self::compare_json_for_bound(val, existing)
                                == std::cmp::Ordering::Less
                            {
                                val
                            } else {
                                existing
                            }
                        }
                        None => val,
                    });
                }
                "$eq" => {
                    eq_vals.push(val);
                }
                // Pass through other operators as-is ($ne, $in, $nin, $exists, $type)
                other => {
                    result.insert(other.to_string(), val.clone());
                }
            }
        }

        // Handle conflicting equality: $and: [{f:1}, {f:2}] → impossible
        if eq_vals.len() > 1 {
            return None;
        }

        let has_range =
            gte_val.is_some() || gt_val.is_some() || lte_val.is_some() || lt_val.is_some();

        // $eq + range → None (conflicting: e.g. $eq:5 + $gte:10 is impossible or redundant)
        // Don't merge — let the engine handle it via the fallback path.
        if eq_vals.len() == 1 && has_range {
            return None;
        }

        if let Some(eq_v) = eq_vals.first() {
            result.insert("$eq".to_string(), (*eq_v).clone());
        }

        // Emit resolved range operators
        if let Some(v) = gte_val {
            result.insert("$gte".to_string(), v.clone());
        }
        if let Some(v) = gt_val {
            result.insert("$gt".to_string(), v.clone());
        }
        if let Some(v) = lte_val {
            result.insert("$lte".to_string(), v.clone());
        }
        if let Some(v) = lt_val {
            result.insert("$lt".to_string(), v.clone());
        }

        // Detect contradictory range: lower bound > upper bound → None
        // Also detect mixed-type bounds (e.g. Number lower + String upper) as impossible
        let effective_lower = gte_val.or(gt_val);
        let effective_upper = lte_val.or(lt_val);
        if let (Some(lo), Some(hi)) = (effective_lower, effective_upper) {
            // Bounds of different value-type categories can never form an overlapping
            // range (a number can't be both ≥ 5 and ≤ false), so the range is
            // impossible. Detect this generically by type rank rather than only the
            // Number/String pair — `{$gte: 5, $lte: false}` and `{$gte: 5, $lte: null}`
            // previously slipped through to a 0.0-cost index misselection (audit #27
            // finding C). Same-type bounds fall through to the value comparison below.
            let mixed_types = Self::value_type_rank(lo) != Self::value_type_rank(hi);
            if mixed_types || Self::compare_json_for_bound(lo, hi) == std::cmp::Ordering::Greater {
                return None;
            }
        }

        Some(Value::Object(result))
    }

    /// Try to merge $or clauses into implicit $in form.
    fn try_merge_or_clauses(clauses: &[Value], index_fields: &[IndexPrefixInfo]) -> Option<Value> {
        // All clauses must be single-field simple equality on the same field
        let mut target_field: Option<String> = None;
        let mut values: Vec<Value> = Vec::new();

        for clause in clauses {
            let obj = clause.as_object()?;
            if obj.len() != 1 {
                return None; // Multi-field clause
            }
            let (field, value) = obj.iter().next()?;
            if field.starts_with('$') {
                return None; // Nested logical
            }

            // Extract the equality value
            let eq_value = match value {
                // Bare equality: {field: "value"}
                Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null => value.clone(),
                // Explicit $eq: {field: {$eq: value}}
                Value::Object(cond_map) if cond_map.len() == 1 => {
                    if let Some(v) = cond_map.get("$eq") {
                        v.clone()
                    } else {
                        return None; // Non-equality operator
                    }
                }
                _ => return None,
            };

            // Verify all clauses target the same field
            match &target_field {
                Some(existing) => {
                    if existing != field {
                        return None; // Different fields
                    }
                }
                None => {
                    target_field = Some(field.clone());
                }
            }

            values.push(eq_value);
        }

        let field = target_field?;

        // Must have an index on this field
        Self::find_index_for_field(&field, index_fields)?;

        // Build {field: {$in: [val1, val2, ...]}}
        let mut merged = serde_json::Map::new();
        let mut in_obj = serde_json::Map::new();
        in_obj.insert("$in".to_string(), Value::Array(values));
        merged.insert(field, Value::Object(in_obj));

        Some(Value::Object(merged))
    }

    /// Value-type category rank, used to detect mixed-type range bounds (audit #27
    /// finding C). Bounds with different ranks can never overlap into a valid range.
    fn value_type_rank(v: &Value) -> u8 {
        match v {
            Value::Null => 0,
            Value::Bool(_) => 1,
            Value::Number(_) => 2,
            Value::String(_) => 3,
            Value::Array(_) => 4,
            Value::Object(_) => 5,
        }
    }

    /// Compare two JSON values for bound tightening (used in conflicting range resolution).
    ///
    /// Supports Number and String comparisons.
    /// Mixed types (e.g. Number vs String) return Less to signal an impossible range,
    /// so the contradictory range check in resolve_range_conditions() can filter it out.
    fn compare_json_for_bound(a: &Value, b: &Value) -> std::cmp::Ordering {
        match (a, b) {
            (Value::Number(an), Value::Number(bn)) => {
                let af = an.as_f64().unwrap_or(0.0);
                let bf = bn.as_f64().unwrap_or(0.0);
                af.partial_cmp(&bf).unwrap_or(std::cmp::Ordering::Equal)
            }
            (Value::String(a_s), Value::String(b_s)) => a_s.cmp(b_s),
            // Mixed types (e.g. Number vs String) → impossible range
            _ => std::cmp::Ordering::Less,
        }
    }

    /// Find an index for a given field (compound index aware)
    ///
    /// Takes a list of IndexPrefixInfo containing:
    /// - index_name: The index name
    /// - prefix_field: The first field (for compound) or only field (for single)
    /// - is_compound: Whether this is a compound index
    ///
    /// Skips indexes that are currently being built (building == true).
    ///
    /// Returns (index_name, is_compound) if found.
    fn find_index_for_field(
        field: &str,
        index_fields: &[IndexPrefixInfo],
    ) -> Option<(String, bool)> {
        // Prefer a single-field (non-compound) index, falling back to a compound
        // one only when no single index exists. Without this, the result depended
        // on the (HashMap-derived, unordered) iteration order: if a compound index
        // happened to come first, callers that skip compounds (`if is_compound
        // continue`) dropped the whole field to a collection scan even though a
        // usable single index was present — a nondeterministic perf regression
        // (audit #27 finding B).
        index_fields
            .iter()
            .find(|info| info.prefix_field == field && !info.building && !info.is_compound)
            .or_else(|| {
                index_fields
                    .iter()
                    .find(|info| info.prefix_field == field && !info.building)
            })
            .map(|info| (info.index_name.clone(), info.is_compound))
    }

    /// Analyze a query with compound-index-aware field matching (v2)
    ///
    /// This version takes IndexPrefixInfo to correctly handle compound indexes
    /// by using them for prefix field queries with range scans.
    ///
    /// Also handles sparse index optimization for $exists: true queries.
    pub fn analyze_query_with_fields(
        query_json: &Value,
        index_fields: &[IndexPrefixInfo],
    ) -> Option<(String, QueryPlan)> {
        // Collect all candidate plans
        let candidates = Self::collect_candidates(query_json, index_fields);

        // Select best candidate (lowest cost)
        Self::select_best_candidate(candidates).map(|c| (c.field, c.plan))
    }

    /// Collect all candidate query plans for a given query
    ///
    /// Returns a list of all applicable index plans, each with estimated cost.
    /// This enables the planner to choose the best plan based on selectivity.
    pub fn collect_candidates(
        query_json: &Value,
        index_fields: &[IndexPrefixInfo],
    ) -> Vec<CandidatePlan> {
        let mut candidates = Vec::new();

        if let Value::Object(ref _map) = query_json {
            // NOTE: $-prefixed root keys (e.g. $text, $expr) are skipped per-field
            // inside each collect_*_candidates method. We do NOT skip the entire query
            // here, so that indexable fields like "age" in {"age": 25, "$text": "hello"}
            // can still get index plans.

            // Collect candidates from each analyzer
            Self::collect_exists_candidates(query_json, index_fields, &mut candidates);
            Self::collect_regex_candidates(query_json, index_fields, &mut candidates);
            Self::collect_in_regex_candidates(query_json, index_fields, &mut candidates);
            Self::collect_in_candidates(query_json, index_fields, &mut candidates);
            Self::collect_range_candidates(query_json, index_fields, &mut candidates);
            Self::collect_equality_candidates(query_json, index_fields, &mut candidates);
        }

        candidates
    }

    /// Select the best candidate based on estimated cost
    ///
    /// Returns the candidate with the lowest estimated cost, or None if empty.
    pub fn select_best_candidate(candidates: Vec<CandidatePlan>) -> Option<CandidatePlan> {
        candidates.into_iter().min_by(|a, b| {
            a.estimated_cost
                .partial_cmp(&b.estimated_cost)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }

    /// Collect candidates from $exists: true queries (sparse index optimization)
    fn collect_exists_candidates(
        query_json: &Value,
        index_fields: &[IndexPrefixInfo],
        candidates: &mut Vec<CandidatePlan>,
    ) {
        if let Some((field, plan)) = Self::analyze_exists_query(query_json, index_fields) {
            // Sparse index scan is very efficient - low cost
            if let Some(info) = index_fields
                .iter()
                .find(|i| i.prefix_field == field && i.sparse && !i.building)
            {
                // Dynamic cost based on actual index size (num_keys)
                // Small sparse index = very cheap, large sparse index = proportionally more expensive
                let cost = if info.num_keys > 0 {
                    (info.num_keys as f64).max(1.0)
                } else {
                    100.0 // Fallback if num_keys unknown
                };
                candidates.push(CandidatePlan::new(
                    plan,
                    field,
                    cost,
                    format!(
                        "Sparse index {} for $exists:true (keys: {})",
                        info.index_name, info.num_keys
                    ),
                ));
            }
        }
    }

    /// Collect candidates from regex prefix queries
    fn collect_regex_candidates(
        query_json: &Value,
        index_fields: &[IndexPrefixInfo],
        candidates: &mut Vec<CandidatePlan>,
    ) {
        if let Some((field, plan)) = Self::analyze_regex_query(query_json, index_fields) {
            // Find the index info for selectivity
            let info = index_fields.iter().find(|i| {
                i.prefix_field == field &&
                !i.is_compound &&
                !i.building &&
                // Match the index used in the plan
                matches!(&plan, QueryPlan::RegexPrefixScan { index_name, .. } if i.index_name == *index_name)
            });

            if let Some(info) = info {
                candidates.push(CandidatePlan::with_selectivity(
                    plan,
                    field,
                    &info.index_name,
                    info.distinct_count,
                    info.num_keys, // Use actual index key count
                    info.multikey_ratio,
                ));
            } else {
                candidates.push(CandidatePlan::with_default_cost(
                    plan.clone(),
                    field.clone(),
                    match &plan {
                        QueryPlan::RegexPrefixScan { index_name, .. } => index_name,
                        _ => "unknown",
                    },
                ));
            }
        }
    }

    /// Collect candidates from $in with regex patterns
    fn collect_in_regex_candidates(
        query_json: &Value,
        index_fields: &[IndexPrefixInfo],
        candidates: &mut Vec<CandidatePlan>,
    ) {
        if let Some((field, plan)) = Self::analyze_in_with_regex(query_json, index_fields) {
            if let Some(info) = index_fields
                .iter()
                .find(|i| i.prefix_field == field && !i.is_compound && !i.building)
            {
                // Multi-regex scan cost depends on number of prefixes
                let prefix_count = match &plan {
                    QueryPlan::MultiRegexPrefixScan { prefixes, .. } => prefixes.len(),
                    _ => 1,
                };
                // Use actual num_keys for cost estimation
                let total_docs = info.num_keys.max(1);
                let base_cost = if info.distinct_count > 0 {
                    (total_docs as f64) / info.distinct_count as f64
                } else {
                    total_docs as f64
                };
                candidates.push(CandidatePlan::new(
                    plan,
                    field,
                    base_cost * prefix_count as f64,
                    format!(
                        "Multi-regex on {} ({} prefixes)",
                        info.index_name, prefix_count
                    ),
                ));
            }
        }
    }

    /// Collect candidates from $in with plain values (not regex)
    /// e.g., { _id: { $in: ["a", "b", "c"] } }
    ///
    /// This enables O(k) index lookups instead of O(n) collection scan
    /// where k = number of values in $in, n = collection size.
    fn collect_in_candidates(
        query_json: &Value,
        index_fields: &[IndexPrefixInfo],
        candidates: &mut Vec<CandidatePlan>,
    ) {
        if let Value::Object(ref map) = query_json {
            for (field, value) in map {
                // Skip operator fields at root level
                if field.starts_with('$') {
                    continue;
                }

                // Look for { field: { $in: [...] } } pattern
                if let Value::Object(ref cond_map) = value {
                    if let Some(Value::Array(in_values)) = cond_map.get("$in") {
                        // Skip if $in contains operators (handled by collect_in_regex_candidates)
                        let has_operators = in_values.iter().any(|v| {
                            v.as_object()
                                .map(|o| o.keys().any(|k| k.starts_with('$')))
                                .unwrap_or(false)
                        });
                        if has_operators {
                            continue;
                        }

                        // Skip empty $in
                        if in_values.is_empty() {
                            continue;
                        }

                        // Find matching index for this field (skip building indexes)
                        if let Some(info) = index_fields
                            .iter()
                            .find(|i| i.prefix_field == *field && !i.is_compound && !i.building)
                        {
                            // Convert values to IndexKeys
                            let keys: Vec<IndexKey> =
                                in_values.iter().map(IndexKey::from).collect();

                            let plan = QueryPlan::MultiValueScan {
                                index_name: info.index_name.clone(),
                                field: field.clone(),
                                keys: keys.clone(),
                            };

                            // Cost: k index lookups, each O(log n)
                            // Better than collection scan O(n) when k << n
                            let cost = keys.len() as f64 * (info.num_keys as f64).log2().max(1.0);

                            candidates.push(CandidatePlan::new(
                                plan,
                                field.clone(),
                                cost,
                                format!(
                                    "Multi-value scan on {} ({} keys)",
                                    info.index_name,
                                    keys.len()
                                ),
                            ));
                        }
                    }
                }
            }
        }
    }

    /// Collect candidates from range queries ($gt, $gte, $lt, $lte)
    ///
    /// Uses histogram-based selectivity estimation when available for more accurate
    /// row count estimates. Falls back to uniform 33% estimate otherwise.
    fn collect_range_candidates(
        query_json: &Value,
        index_fields: &[IndexPrefixInfo],
        candidates: &mut Vec<CandidatePlan>,
    ) {
        if let Some((field, plan)) = Self::analyze_range_query_v2(query_json, index_fields) {
            if let Some(info) = index_fields
                .iter()
                .find(|i| i.prefix_field == field && !i.is_compound && !i.building)
            {
                // Extract start/end from the plan for histogram-based estimation
                // Clone the keys since we need to move plan later
                let (start_key, end_key) = match &plan {
                    QueryPlan::IndexRangeScan { start, end, .. } => (start.clone(), end.clone()),
                    _ => (None, None),
                };

                // Use histogram-based range selectivity estimation
                candidates.push(CandidatePlan::with_range_selectivity(
                    plan,
                    field,
                    &info.index_name,
                    info.histogram.as_ref(),
                    start_key.as_ref(),
                    end_key.as_ref(),
                    info.num_keys, // Use actual key count for better estimate
                    info.multikey_ratio,
                ));
            }
        }
    }

    /// Collect candidates from equality queries
    ///
    /// Handles both implicit equality `{"field": "value"}` and explicit
    /// `{"field": {"$eq": "value"}}` syntax for MongoDB compatibility.
    fn collect_equality_candidates(
        query_json: &Value,
        index_fields: &[IndexPrefixInfo],
        candidates: &mut Vec<CandidatePlan>,
    ) {
        if let Value::Object(ref map) = query_json {
            for (field, value) in map {
                // Skip operator fields
                if field.starts_with('$') {
                    continue;
                }

                // Determine the actual equality value:
                // - {"field": {"$eq": X}} -> use X
                // - {"field": X} where X is not an operator object -> use X
                //
                // Audit #27 finding A: Object and Array values are NOT
                // indexable as keys — `IndexKey::from(&Value)` collapses both
                // to `IndexKey::Null` (key.rs:119). Picking an index in that
                // case produces an `IndexScan { key: Null }` plan that points
                // at NULL-valued docs, not at docs whose field deep-equals
                // the object/array. With `count_documents`'s `fully_covered`
                // short-circuit (count.rs:215) the planner returns the wrong
                // count silently. Skip the candidate; the `$eq` operator on
                // the collection-scan path performs proper deep equality.
                let is_indexable_value =
                    |v: &Value| -> bool { !matches!(v, Value::Object(_) | Value::Array(_)) };
                let equality_value: Option<&Value> = if let Value::Object(ref val_map) = value {
                    if val_map.len() == 1 {
                        if let Some(eq_val) = val_map.get("$eq") {
                            // Explicit $eq operator: {"field": {"$eq": X}}
                            // — the inner value must itself be indexable.
                            if is_indexable_value(eq_val) {
                                Some(eq_val)
                            } else {
                                None
                            }
                        } else if val_map.keys().any(|k| k.starts_with('$')) {
                            // Other operators like $gt, $in, etc. — handled
                            // by the dedicated collect_*_candidates helpers.
                            None
                        } else {
                            // Plain object value — not indexable as a key.
                            None
                        }
                    } else if val_map.keys().any(|k| k.starts_with('$')) {
                        // Multiple operators or mixed — handled elsewhere.
                        None
                    } else {
                        // Plain object value — not indexable.
                        None
                    }
                } else if matches!(value, Value::Array(_)) {
                    // Array value — not indexable as a single key.
                    None
                } else {
                    // Scalar value (Null/Bool/Number/String) — usable.
                    Some(value)
                };

                let eq_val = match equality_value {
                    Some(v) => v,
                    None => continue,
                };

                // Find ALL matching indexes for this field (not just first, skip building)
                for info in index_fields
                    .iter()
                    .filter(|i| i.prefix_field == *field && !i.building)
                {
                    let key = IndexKey::from(eq_val);
                    let plan = QueryPlan::IndexScan {
                        index_name: info.index_name.clone(),
                        field: field.clone(),
                        key: key.clone(),
                        is_compound: info.is_compound,
                    };

                    // Use MCV-aware selectivity estimation for more accurate cost
                    candidates.push(CandidatePlan::with_selectivity_mcv(
                        plan,
                        field.clone(),
                        &info.index_name,
                        &key,
                        info.distinct_count,
                        info.num_keys,
                        info.multikey_ratio,
                        info.mcv.as_ref(),
                    ));
                }
            }
        }
    }

    /// Analyze query for $exists: true with sparse index optimization
    ///
    /// For sparse indexes, all doc_ids in the index are documents where the field exists.
    /// This enables efficient $exists: true queries without full collection scan.
    fn analyze_exists_query(
        query_json: &Value,
        index_fields: &[IndexPrefixInfo],
    ) -> Option<(String, QueryPlan)> {
        if let Value::Object(ref map) = query_json {
            for (field, conditions) in map {
                if field.starts_with('$') {
                    continue; // Skip logical operators at root level
                }

                if let Value::Object(ref cond_map) = conditions {
                    // Check for $exists: true, but only if it's the sole operator
                    if let Some(Value::Bool(true)) = cond_map.get("$exists") {
                        if cond_map.keys().any(|k| k != "$exists") {
                            continue;
                        }
                        // Look for a sparse index on this field (skip building indexes)
                        if let Some(info) = index_fields.iter().find(|info| {
                            info.prefix_field == *field && info.sparse && !info.building
                        }) {
                            return Some((
                                field.clone(),
                                QueryPlan::SparseIndexScan {
                                    index_name: info.index_name.clone(),
                                    field: field.clone(),
                                },
                            ));
                        }
                    }
                }
            }
        }

        None
    }

    /// Analyze query for range operators with compound-index-aware matching
    fn analyze_range_query_v2(
        query_json: &Value,
        index_fields: &[IndexPrefixInfo],
    ) -> Option<(String, QueryPlan)> {
        if let Value::Object(ref map) = query_json {
            for (field, conditions) in map {
                if field.starts_with('$') {
                    continue; // Skip logical operators at root level
                }

                if let Value::Object(ref cond_map) = conditions {
                    let has_gt = cond_map.contains_key("$gt");
                    let has_gte = cond_map.contains_key("$gte");
                    let has_lt = cond_map.contains_key("$lt");
                    let has_lte = cond_map.contains_key("$lte");

                    if has_gt || has_gte || has_lt || has_lte {
                        // Compound-index-aware field matching (we only need the index name for range queries)
                        let (index_name, is_compound) =
                            Self::find_index_for_field(field, index_fields)?;
                        if is_compound {
                            continue;
                        }

                        // Resolve lower bound: if both $gt and $gte exist,
                        // pick the tighter (higher value) bound.
                        // $gt: 10 + $gte: 5 → use $gt: 10 (exclusive, tighter)
                        // $gt: 3  + $gte: 8 → use $gte: 8 (inclusive, tighter)
                        let (start, inclusive_start) =
                            match (cond_map.get("$gt"), cond_map.get("$gte")) {
                                (Some(gt_val), Some(gte_val)) => {
                                    match Self::compare_json_for_bound(gt_val, gte_val) {
                                        std::cmp::Ordering::Greater | std::cmp::Ordering::Equal => {
                                            // $gt value >= $gte value → $gt is tighter (exclusive)
                                            (Some(IndexKey::from(gt_val)), false)
                                        }
                                        std::cmp::Ordering::Less => {
                                            // $gte value > $gt value → $gte is tighter (inclusive)
                                            (Some(IndexKey::from(gte_val)), true)
                                        }
                                    }
                                }
                                (Some(gt_val), None) => (Some(IndexKey::from(gt_val)), false),
                                (None, Some(gte_val)) => (Some(IndexKey::from(gte_val)), true),
                                (None, None) => (None, true),
                            };

                        // Resolve upper bound: if both $lt and $lte exist,
                        // pick the tighter (lower value) bound.
                        // $lt: 100 + $lte: 200 → use $lt: 100 (exclusive, tighter)
                        // $lt: 50  + $lte: 20  → use $lte: 20 (inclusive, tighter)
                        let (end, inclusive_end) = match (cond_map.get("$lt"), cond_map.get("$lte"))
                        {
                            (Some(lt_val), Some(lte_val)) => {
                                match Self::compare_json_for_bound(lt_val, lte_val) {
                                    std::cmp::Ordering::Less | std::cmp::Ordering::Equal => {
                                        // $lt value <= $lte value → $lt is tighter (exclusive)
                                        (Some(IndexKey::from(lt_val)), false)
                                    }
                                    std::cmp::Ordering::Greater => {
                                        // $lte value < $lt value → $lte is tighter (inclusive)
                                        (Some(IndexKey::from(lte_val)), true)
                                    }
                                }
                            }
                            (Some(lt_val), None) => (Some(IndexKey::from(lt_val)), false),
                            (None, Some(lte_val)) => (Some(IndexKey::from(lte_val)), true),
                            (None, None) => (None, true),
                        };

                        return Some((
                            field.clone(),
                            QueryPlan::IndexRangeScan {
                                index_name,
                                field: field.clone(),
                                start,
                                end,
                                inclusive_start,
                                inclusive_end,
                            },
                        ));
                    }
                }
            }
        }

        None
    }

    /// Analyze query for anchored regex prefix: { field: { $regex: "^prefix" } }
    fn analyze_regex_query(
        query_json: &Value,
        index_fields: &[IndexPrefixInfo],
    ) -> Option<(String, QueryPlan)> {
        if let Value::Object(ref map) = query_json {
            for (field, conditions) in map {
                if field.starts_with('$') {
                    continue;
                }

                let cond_map = match conditions.as_object() {
                    Some(obj) => obj,
                    None => continue,
                };

                let pattern = cond_map.get("$regex")?.as_str()?;
                let options = cond_map.get("$options").and_then(|v| v.as_str());

                // Use unified parse_regex_prefix (handles options validation)
                let info = Self::parse_regex_prefix(pattern, options)?;

                // For case-insensitive regex, try to find a CI index
                if info.case_insensitive {
                    // Look for case-insensitive index by flag (not by name suffix)
                    if let Some(idx_info) = index_fields
                        .iter()
                        .find(|i| i.prefix_field == *field && i.case_insensitive && !i.building)
                    {
                        if !idx_info.is_compound {
                            return Some((
                                field.clone(),
                                QueryPlan::RegexPrefixScan {
                                    index_name: idx_info.index_name.clone(),
                                    field: field.clone(),
                                    prefix: info.prefix.to_lowercase(), // Lowercase for CI index
                                    exact: info.exact,
                                    case_insensitive: true,
                                },
                            ));
                        }
                    }
                    // No CI index found - fallback to collection scan
                    continue;
                }

                let (index_name, is_compound) = Self::find_index_for_field(field, index_fields)?;
                if is_compound {
                    continue;
                }

                return Some((
                    field.clone(),
                    QueryPlan::RegexPrefixScan {
                        index_name,
                        field: field.clone(),
                        prefix: info.prefix,
                        exact: info.exact,
                        case_insensitive: false,
                    },
                ));
            }
        }

        None
    }

    /// Analyze query for $in with regex patterns: { field: { $in: [{"$regex": "^A"}, ...] } }
    ///
    /// Returns MultiRegexPrefixScan if ALL values in $in are optimizable regex prefixes.
    /// Falls back to None if any value is not an anchored regex prefix.
    fn analyze_in_with_regex(
        query_json: &Value,
        index_fields: &[IndexPrefixInfo],
    ) -> Option<(String, QueryPlan)> {
        if let Value::Object(ref map) = query_json {
            for (field, conditions) in map {
                if field.starts_with('$') {
                    continue;
                }

                let cond_map = match conditions.as_object() {
                    Some(obj) => obj,
                    None => continue,
                };

                // Look for $in operator
                let in_array = match cond_map.get("$in") {
                    Some(Value::Array(arr)) if !arr.is_empty() => arr,
                    _ => continue,
                };

                // Check if we have an index on this field
                let (index_name, is_compound) = Self::find_index_for_field(field, index_fields)?;
                if is_compound {
                    continue; // Don't optimize compound indexes for now
                }

                // Try to extract prefix from each value in $in using unified helper
                let mut prefixes = Vec::with_capacity(in_array.len());
                for val in in_array {
                    // Each value must be an object with $regex
                    let val_map = val.as_object()?;
                    let pattern = val_map.get("$regex").and_then(|v| v.as_str())?;
                    let options = val_map.get("$options").and_then(|v| v.as_str());

                    // Use unified parse_regex_prefix (handles options validation)
                    let info = Self::parse_regex_prefix(pattern, options)?;

                    // Must be optimizable for $in (exact and non-CI)
                    if !info.is_optimizable_for_in() {
                        return None;
                    }

                    prefixes.push(info.prefix);
                }

                // All values are optimizable regex prefixes!
                return Some((
                    field.clone(),
                    QueryPlan::MultiRegexPrefixScan {
                        index_name,
                        field: field.clone(),
                        prefixes,
                    },
                ));
            }
        }

        None
    }

    /// Parse a regex pattern with options and extract prefix information.
    ///
    /// This is the unified entry point for regex prefix extraction.
    /// Returns None if the pattern cannot be optimized (no anchor, has $options, etc.)
    ///
    /// # Arguments
    /// * `pattern` - The regex pattern string
    /// * `options` - Optional $options string (if present, optimization is disabled)
    pub fn parse_regex_prefix(pattern: &str, options: Option<&str>) -> Option<RegexPrefixInfo> {
        // Reject if $options is set (cannot optimize)
        if let Some(opts) = options {
            if !opts.is_empty() {
                return None;
            }
        }

        // Delegate to the internal extraction
        Self::extract_regex_prefix_internal(pattern)
    }

    /// Extract literal prefix from a regex like "^prefix" or "(?i)^prefix".
    ///
    /// Returns RegexPrefixInfo with (prefix, exact, case_insensitive).
    /// Stops at the first unescaped regex meta character.
    fn extract_regex_prefix_internal(pattern: &str) -> Option<RegexPrefixInfo> {
        let mut remaining = pattern;
        let mut case_insensitive = false;

        // Check for (?i) prefix (case-insensitive flag)
        if remaining.starts_with("(?i)") {
            case_insensitive = true;
            remaining = &remaining[4..];
        }

        let mut chars = remaining.chars().peekable();
        if chars.next()? != '^' {
            return None;
        }

        // Reject other inline modifiers like (?m), (?s), etc.
        if let (Some('('), Some('?')) = (chars.peek().copied(), chars.clone().nth(1)) {
            return None;
        }

        let mut prefix = String::new();
        let mut exact = true;
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                let escaped = chars.next()?;
                if matches!(
                    escaped,
                    '.' | '*'
                        | '+'
                        | '?'
                        | '('
                        | ')'
                        | '['
                        | ']'
                        | '{'
                        | '}'
                        | '|'
                        | '^'
                        | '$'
                        | '\\'
                ) {
                    prefix.push(escaped);
                    continue;
                }
                return None;
            }

            if ch == '(' && matches!(chars.peek(), Some('?')) {
                return None;
            }

            if matches!(
                ch,
                '.' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '^' | '$'
            ) {
                // Any regex meta means we cannot treat this as a pure prefix match.
                exact = false;
                break;
            }

            prefix.push(ch);
        }

        if prefix.is_empty() {
            None
        } else {
            if chars.peek().is_some() {
                exact = false;
            }
            Some(RegexPrefixInfo {
                prefix,
                exact,
                case_insensitive,
            })
        }
    }

    /// Legacy wrapper - converts RegexPrefixInfo to tuple for backward compat
    #[allow(dead_code)]
    fn extract_regex_prefix(pattern: &str) -> Option<(String, bool, bool)> {
        Self::extract_regex_prefix_internal(pattern)
            .map(|info| (info.prefix, info.exact, info.case_insensitive))
    }

    /// Create a query plan description for explain output (compound index aware)
    ///
    /// Uses the new compound-index-aware query analysis for accurate explain output.
    /// Returns the chosen plan along with all evaluated candidates.
    pub fn explain_query_with_fields(
        query_json: &Value,
        index_fields: &[IndexPrefixInfo],
    ) -> Value {
        use serde_json::json;

        if let Some((logical_op, clauses)) = Self::extract_logical_clauses(query_json) {
            let operator = match logical_op {
                LogicalOperator::And => "$and",
                LogicalOperator::Or => "$or",
                LogicalOperator::Nor => "$nor",
            };

            // Try clause merge optimization
            if let Some(merged_query) =
                Self::try_merge_logical_clauses(logical_op, &clauses, index_fields)
            {
                let candidates = Self::collect_candidates(&merged_query, index_fields);
                if let Some(best) = Self::select_best_candidate(candidates.clone()) {
                    let chosen_plan = Self::plan_to_json(&best.plan, &best.field);
                    let candidates_json: Vec<Value> =
                        candidates.iter().map(Self::candidate_to_json).collect();

                    return json!({
                        "chosenPlan": chosen_plan,
                        "selectionReason": best.reason,
                        "estimatedRows": best.estimated_cost,
                        "optimization": "clause_merge",
                        "originalOperator": operator,
                        "mergedQuery": merged_query,
                        "candidates": candidates_json,
                        "candidateCount": candidates_json.len(),
                    });
                }
            }

            // Fallback: per-clause explain
            let mut total_candidates = 0usize;
            let clauses_json: Vec<Value> = clauses
                .iter()
                .map(|clause| {
                    let candidates = Self::collect_candidates(clause, index_fields);
                    total_candidates += candidates.len();
                    let chosen = Self::select_best_candidate(candidates.clone());
                    let chosen_plan = chosen
                        .as_ref()
                        .map(|c| Self::plan_to_json(&c.plan, &c.field));
                    json!({
                        "clause": clause,
                        "chosenPlan": chosen_plan,
                        "selectionReason": chosen.as_ref().map(|c| c.reason.clone()),
                        "candidates": candidates.iter().map(Self::candidate_to_json).collect::<Vec<_>>(),
                    })
                })
                .collect();

            return json!({
                "chosenPlan": {
                    "queryPlan": "LogicalIndexScan",
                    "operator": operator,
                    "clauseCount": clauses.len(),
                },
                "selectionReason": "Logical operator planning",
                "clauses": clauses_json,
                "candidateCount": total_candidates,
            });
        }

        // Collect all candidates for explain output
        let candidates = Self::collect_candidates(query_json, index_fields);
        let candidates_json: Vec<Value> = candidates.iter().map(Self::candidate_to_json).collect();

        // Select best candidate
        if let Some(best) = Self::select_best_candidate(candidates) {
            let chosen_plan = Self::plan_to_json(&best.plan, &best.field);

            json!({
                "chosenPlan": chosen_plan,
                "selectionReason": best.reason,
                "estimatedRows": best.estimated_cost,
                "candidates": candidates_json,
                "candidateCount": candidates_json.len(),
            })
        } else {
            // No index available
            let available: Vec<&str> = index_fields.iter().map(|i| i.index_name.as_str()).collect();
            json!({
                "chosenPlan": {
                    "queryPlan": "CollectionScan",
                    "indexUsed": null,
                    "stage": "FULL_SCAN",
                    "estimatedCost": "O(n)",
                },
                "selectionReason": "No suitable index found for query",
                "candidates": [],
                "candidateCount": 0,
                "availableIndexes": available,
            })
        }
    }

    /// Convert a CandidatePlan to JSON for explain output
    fn candidate_to_json(candidate: &CandidatePlan) -> Value {
        use serde_json::json;

        let plan_json = Self::plan_to_json(&candidate.plan, &candidate.field);
        json!({
            "plan": plan_json,
            "field": candidate.field,
            "estimatedRows": candidate.estimated_cost,
            "reason": candidate.reason,
        })
    }

    /// Convert a QueryPlan to JSON for explain output
    fn plan_to_json(plan: &QueryPlan, field: &str) -> Value {
        use serde_json::json;

        match plan {
            QueryPlan::IndexScan {
                ref index_name,
                ref key,
                is_compound,
                ..
            } => {
                json!({
                    "queryPlan": if *is_compound { "CompoundIndexScan" } else { "IndexScan" },
                    "indexUsed": index_name,
                    "field": field,
                    "stage": "FETCH_WITH_INDEX",
                    "indexType": if *is_compound { "compound_prefix" } else { "equality" },
                    "searchKey": format!("{:?}", key),
                    "estimatedCost": "O(log n)",
                })
            }
            QueryPlan::IndexRangeScan {
                ref index_name,
                ref start,
                ref end,
                inclusive_start,
                inclusive_end,
                ..
            } => {
                json!({
                    "queryPlan": "IndexRangeScan",
                    "indexUsed": index_name,
                    "field": field,
                    "stage": "FETCH_WITH_INDEX",
                    "indexType": "range",
                    "range": {
                        "start": format!("{:?}", start),
                        "end": format!("{:?}", end),
                        "inclusiveStart": inclusive_start,
                        "inclusiveEnd": inclusive_end,
                    },
                    "estimatedCost": "O(log n + k)",
                })
            }
            QueryPlan::SparseIndexScan { ref index_name, .. } => {
                json!({
                    "queryPlan": "SparseIndexScan",
                    "indexUsed": index_name,
                    "field": field,
                    "stage": "FETCH_WITH_SPARSE_INDEX",
                    "indexType": "sparse_exists",
                    "description": "Returns all doc_ids from sparse index (field exists)",
                    "estimatedCost": "O(k)",
                })
            }
            QueryPlan::RegexPrefixScan {
                ref index_name,
                ref prefix,
                exact,
                case_insensitive,
                ..
            } => {
                json!({
                    "queryPlan": if *case_insensitive { "CIRegexPrefixScan" } else { "RegexPrefixScan" },
                    "indexUsed": index_name,
                    "field": field,
                    "stage": "FETCH_WITH_INDEX",
                    "indexType": if *case_insensitive { "ci_regex_prefix" } else { "regex_prefix" },
                    "prefix": prefix,
                    "exact": exact,
                    "caseInsensitive": case_insensitive,
                    "estimatedCost": "O(log n + k)",
                })
            }
            QueryPlan::MultiRegexPrefixScan {
                ref index_name,
                ref prefixes,
                ..
            } => {
                json!({
                    "queryPlan": "MultiRegexPrefixScan",
                    "indexUsed": index_name,
                    "field": field,
                    "stage": "FETCH_WITH_INDEX",
                    "indexType": "multi_regex_prefix",
                    "prefixes": prefixes,
                    "prefixCount": prefixes.len(),
                    "estimatedCost": "O(k * (log n + m))",
                })
            }
            QueryPlan::MultiValueScan {
                ref index_name,
                ref keys,
                field: ref plan_field,
            } => {
                let _ = field; // silence unused warning for function param
                json!({
                    "queryPlan": "MultiValueScan",
                    "indexUsed": index_name,
                    "field": plan_field,
                    "stage": "FETCH_WITH_INDEX",
                    "indexType": "multi_value_in",
                    "keyCount": keys.len(),
                    "description": "$in query optimized with index lookups",
                    "estimatedCost": "O(k * log n)",
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_equality_query_analysis() {
        let query = json!({"age": 25});
        let index_fields = vec![
            IndexPrefixInfo {
                index_name: "users_age".to_string(),
                prefix_field: "age".to_string(),
                is_compound: false,
                num_fields: 1,
                sparse: false,
                distinct_count: 0,
                building: false,
                num_keys: 0,
                case_insensitive: false,
                null_count: 0,
                multikey_ratio: 0.0,
                histogram: None,
                mcv: None,
            },
            IndexPrefixInfo {
                index_name: "users_id".to_string(),
                prefix_field: "id".to_string(),
                is_compound: false,
                num_fields: 1,
                sparse: false,
                distinct_count: 0,
                building: false,
                num_keys: 0,
                case_insensitive: false,
                null_count: 0,
                multikey_ratio: 0.0,
                histogram: None,
                mcv: None,
            },
        ];

        let result = QueryPlanner::analyze_query_with_fields(&query, &index_fields);
        assert!(result.is_some());

        let (field, plan) = result.unwrap();
        assert_eq!(field, "age");

        match plan {
            QueryPlan::IndexScan {
                index_name,
                field,
                key,
                ..
            } => {
                assert_eq!(index_name, "users_age");
                assert_eq!(field, "age");
                assert_eq!(key, IndexKey::Int(25));
            }
            _ => panic!("Expected IndexScan"),
        }
    }

    #[test]
    fn test_range_query_analysis() {
        let query = json!({"age": {"$gte": 18, "$lt": 65}});
        let index_fields = vec![IndexPrefixInfo {
            index_name: "users_age".to_string(),
            prefix_field: "age".to_string(),
            is_compound: false,
            num_fields: 1,
            sparse: false,
            distinct_count: 0,
            building: false,
            num_keys: 0,
            case_insensitive: false,
            null_count: 0,
            multikey_ratio: 0.0,
            histogram: None,
            mcv: None,
        }];

        let result = QueryPlanner::analyze_query_with_fields(&query, &index_fields);
        assert!(result.is_some());

        let (field, plan) = result.unwrap();
        assert_eq!(field, "age");

        match plan {
            QueryPlan::IndexRangeScan {
                index_name,
                start,
                end,
                inclusive_start,
                inclusive_end,
                ..
            } => {
                assert_eq!(index_name, "users_age");
                assert_eq!(start, Some(IndexKey::Int(18)));
                assert_eq!(end, Some(IndexKey::Int(65)));
                assert!(inclusive_start);
                assert!(!inclusive_end);
            }
            _ => panic!("Expected IndexRangeScan"),
        }
    }

    #[test]
    fn test_regex_prefix_query_analysis() {
        let query = json!({"email": {"$regex": "^alice"}} );
        let index_fields = vec![IndexPrefixInfo {
            index_name: "users_email".to_string(),
            prefix_field: "email".to_string(),
            is_compound: false,
            num_fields: 1,
            sparse: false,
            distinct_count: 0,
            building: false,
            num_keys: 0,
            case_insensitive: false,
            null_count: 0,
            multikey_ratio: 0.0,
            histogram: None,
            mcv: None,
        }];

        let result = QueryPlanner::analyze_query_with_fields(&query, &index_fields);
        assert!(result.is_some());

        let (field, plan) = result.unwrap();
        assert_eq!(field, "email");
        match plan {
            QueryPlan::RegexPrefixScan {
                index_name,
                prefix,
                exact,
                ..
            } => {
                assert_eq!(index_name, "users_email");
                assert_eq!(prefix, "alice");
                assert!(exact);
            }
            _ => panic!("Expected RegexPrefixScan"),
        }
    }

    #[test]
    fn test_regex_prefix_with_options_not_optimized() {
        let query = json!({"email": {"$regex": "^alice", "$options": "i"}} );
        let index_fields = vec![IndexPrefixInfo {
            index_name: "users_email".to_string(),
            prefix_field: "email".to_string(),
            is_compound: false,
            num_fields: 1,
            sparse: false,
            distinct_count: 0,
            building: false,
            num_keys: 0,
            case_insensitive: false,
            null_count: 0,
            multikey_ratio: 0.0,
            histogram: None,
            mcv: None,
        }];

        let result = QueryPlanner::analyze_query_with_fields(&query, &index_fields);
        assert!(result.is_none());
    }

    #[test]
    fn test_regex_without_anchor_not_optimized() {
        let query = json!({"email": {"$regex": "alice"}} );
        let index_fields = vec![IndexPrefixInfo {
            index_name: "users_email".to_string(),
            prefix_field: "email".to_string(),
            is_compound: false,
            num_fields: 1,
            sparse: false,
            distinct_count: 0,
            building: false,
            num_keys: 0,
            case_insensitive: false,
            null_count: 0,
            multikey_ratio: 0.0,
            histogram: None,
            mcv: None,
        }];

        let result = QueryPlanner::analyze_query_with_fields(&query, &index_fields);
        assert!(result.is_none());
    }

    #[test]
    fn test_no_index_available() {
        let query = json!({"name": "Alice"});
        let index_fields = vec![IndexPrefixInfo {
            index_name: "users_age".to_string(),
            prefix_field: "age".to_string(),
            is_compound: false,
            num_fields: 1,
            sparse: false,
            distinct_count: 0,
            building: false,
            num_keys: 0,
            case_insensitive: false,
            null_count: 0,
            multikey_ratio: 0.0,
            histogram: None,
            mcv: None,
        }];

        let result = QueryPlanner::analyze_query_with_fields(&query, &index_fields);
        assert!(result.is_none());
    }

    #[test]
    fn test_complex_query_no_optimization() {
        let query = json!({"$and": [{"age": 25}, {"name": "Alice"}]});
        let index_fields = vec![IndexPrefixInfo {
            index_name: "users_age".to_string(),
            prefix_field: "age".to_string(),
            is_compound: false,
            num_fields: 1,
            sparse: false,
            distinct_count: 0,
            building: false,
            num_keys: 0,
            case_insensitive: false,
            null_count: 0,
            multikey_ratio: 0.0,
            histogram: None,
            mcv: None,
        }];

        // Complex queries not yet supported
        let result = QueryPlanner::analyze_query_with_fields(&query, &index_fields);
        assert!(result.is_none());
    }

    #[test]
    fn test_multi_regex_prefix_in_query() {
        // Test: { field: { $in: [{"$regex": "^A"}, {"$regex": "^B"}] } }
        let query = json!({
            "name": {
                "$in": [
                    {"$regex": "^Alice"},
                    {"$regex": "^Bob"},
                    {"$regex": "^Charlie"}
                ]
            }
        });
        let index_fields = vec![IndexPrefixInfo {
            index_name: "users_name".to_string(),
            prefix_field: "name".to_string(),
            is_compound: false,
            num_fields: 1,
            sparse: false,
            distinct_count: 0,
            building: false,
            num_keys: 0,
            case_insensitive: false,
            null_count: 0,
            multikey_ratio: 0.0,
            histogram: None,
            mcv: None,
        }];

        let result = QueryPlanner::analyze_query_with_fields(&query, &index_fields);
        assert!(result.is_some());

        let (field, plan) = result.unwrap();
        assert_eq!(field, "name");
        match plan {
            QueryPlan::MultiRegexPrefixScan {
                index_name,
                prefixes,
                ..
            } => {
                assert_eq!(index_name, "users_name");
                assert_eq!(prefixes, vec!["Alice", "Bob", "Charlie"]);
            }
            _ => panic!("Expected MultiRegexPrefixScan"),
        }
    }

    #[test]
    fn test_multi_regex_prefix_mixed_values_not_optimized() {
        // If $in contains non-regex values, don't optimize
        let query = json!({
            "name": {
                "$in": [
                    {"$regex": "^Alice"},
                    "Bob"  // Plain string, not regex
                ]
            }
        });
        let index_fields = vec![IndexPrefixInfo {
            index_name: "users_name".to_string(),
            prefix_field: "name".to_string(),
            is_compound: false,
            num_fields: 1,
            sparse: false,
            distinct_count: 0,
            building: false,
            num_keys: 0,
            case_insensitive: false,
            null_count: 0,
            multikey_ratio: 0.0,
            histogram: None,
            mcv: None,
        }];

        let result = QueryPlanner::analyze_query_with_fields(&query, &index_fields);
        assert!(result.is_none());
    }

    #[test]
    fn test_multi_regex_prefix_non_anchored_not_optimized() {
        // If any regex is not anchored, don't optimize
        let query = json!({
            "name": {
                "$in": [
                    {"$regex": "^Alice"},
                    {"$regex": "Bob"}  // Not anchored
                ]
            }
        });
        let index_fields = vec![IndexPrefixInfo {
            index_name: "users_name".to_string(),
            prefix_field: "name".to_string(),
            is_compound: false,
            num_fields: 1,
            sparse: false,
            distinct_count: 0,
            building: false,
            num_keys: 0,
            case_insensitive: false,
            null_count: 0,
            multikey_ratio: 0.0,
            histogram: None,
            mcv: None,
        }];

        let result = QueryPlanner::analyze_query_with_fields(&query, &index_fields);
        assert!(result.is_none());
    }

    #[test]
    fn test_multi_regex_prefix_with_options_not_optimized() {
        // If any regex has $options, don't optimize
        let query = json!({
            "name": {
                "$in": [
                    {"$regex": "^Alice"},
                    {"$regex": "^Bob", "$options": "i"}
                ]
            }
        });
        let index_fields = vec![IndexPrefixInfo {
            index_name: "users_name".to_string(),
            prefix_field: "name".to_string(),
            is_compound: false,
            num_fields: 1,
            sparse: false,
            distinct_count: 0,
            building: false,
            num_keys: 0,
            case_insensitive: false,
            null_count: 0,
            multikey_ratio: 0.0,
            histogram: None,
            mcv: None,
        }];

        let result = QueryPlanner::analyze_query_with_fields(&query, &index_fields);
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_regex_prefix_with_case_insensitive() {
        // Case-insensitive regex prefix with exact match
        let result = QueryPlanner::extract_regex_prefix("(?i)^Hello$");
        assert!(result.is_some());
        let (prefix, exact, ci) = result.unwrap();
        assert_eq!(prefix, "Hello");
        assert!(!exact); // Trailing $ means not a pure prefix match
        assert!(ci);
    }

    #[test]
    fn test_extract_regex_prefix_with_case_insensitive_non_exact() {
        // Case-insensitive regex prefix without $ (non-exact)
        let result = QueryPlanner::extract_regex_prefix("(?i)^Hello");
        assert!(result.is_some());
        let (prefix, exact, ci) = result.unwrap();
        assert_eq!(prefix, "Hello");
        assert!(exact); // No trailing regex metachar = exact prefix
        assert!(ci);
    }

    #[test]
    fn test_extract_regex_prefix_case_sensitive() {
        // Regular case-sensitive prefix (exact because nothing follows)
        let result = QueryPlanner::extract_regex_prefix("^Hello");
        assert!(result.is_some());
        let (prefix, exact, ci) = result.unwrap();
        assert_eq!(prefix, "Hello");
        assert!(exact); // No trailing regex metachar = exact prefix
        assert!(!ci);
    }

    #[test]
    fn test_extract_regex_prefix_with_trailing_regex() {
        // Regex with trailing pattern (non-exact)
        let result = QueryPlanner::extract_regex_prefix("^Hello.*");
        assert!(result.is_some());
        let (prefix, exact, ci) = result.unwrap();
        assert_eq!(prefix, "Hello");
        assert!(!exact); // .* follows = non-exact
        assert!(!ci);
    }

    #[test]
    fn test_ci_regex_with_ci_index() {
        // Case-insensitive regex should use CI index when available
        let query = json!({"email": {"$regex": "(?i)^john"}} );
        let index_fields = vec![
            IndexPrefixInfo {
                index_name: "users_email".to_string(),
                prefix_field: "email".to_string(),
                is_compound: false,
                num_fields: 1,
                sparse: false,
                distinct_count: 0,
                building: false,
                num_keys: 0,
                case_insensitive: false,
                null_count: 0,
                multikey_ratio: 0.0,
                histogram: None,
                mcv: None,
            },
            IndexPrefixInfo {
                index_name: "users_email_ci".to_string(),
                prefix_field: "email".to_string(),
                is_compound: false,
                num_fields: 1,
                sparse: true, // CI indexes are sparse
                distinct_count: 0,
                building: false,
                num_keys: 0,
                case_insensitive: true, // This is the CI index!
                null_count: 0,
                multikey_ratio: 0.0,
                histogram: None,
                mcv: None,
            },
        ];

        let result = QueryPlanner::analyze_query_with_fields(&query, &index_fields);
        assert!(result.is_some());

        let (field, plan) = result.unwrap();
        assert_eq!(field, "email");
        match plan {
            QueryPlan::RegexPrefixScan {
                index_name,
                prefix,
                case_insensitive,
                ..
            } => {
                assert_eq!(index_name, "users_email_ci");
                assert_eq!(prefix, "john"); // Lowercased
                assert!(case_insensitive);
            }
            _ => panic!("Expected RegexPrefixScan"),
        }
    }

    #[test]
    fn test_ci_regex_without_ci_index_fallback() {
        // Case-insensitive regex without CI index should fallback to collection scan
        let query = json!({"email": {"$regex": "(?i)^john"}} );
        let index_fields = vec![IndexPrefixInfo {
            index_name: "users_email".to_string(),
            prefix_field: "email".to_string(),
            is_compound: false,
            num_fields: 1,
            sparse: false,
            distinct_count: 0,
            building: false,
            num_keys: 0,
            case_insensitive: false, // Not a CI index
            null_count: 0,
            multikey_ratio: 0.0,
            histogram: None,
            mcv: None,
        }];

        // No CI index available - should return None (collection scan)
        let result = QueryPlanner::analyze_query_with_fields(&query, &index_fields);
        assert!(result.is_none());
    }

    #[test]
    fn test_multi_regex_with_ci_not_optimized() {
        // $in with case-insensitive regex should not be optimized (for simplicity)
        let query = json!({
            "name": {
                "$in": [
                    {"$regex": "(?i)^Alice"},
                    {"$regex": "^Bob"}
                ]
            }
        });
        let index_fields = vec![IndexPrefixInfo {
            index_name: "users_name".to_string(),
            prefix_field: "name".to_string(),
            is_compound: false,
            num_fields: 1,
            sparse: false,
            distinct_count: 0,
            building: false,
            num_keys: 0,
            case_insensitive: false,
            null_count: 0,
            multikey_ratio: 0.0,
            histogram: None,
            mcv: None,
        }];

        let result = QueryPlanner::analyze_query_with_fields(&query, &index_fields);
        assert!(result.is_none());
    }

    #[test]
    fn test_collect_candidates_multiple_indexes() {
        // Test that multiple indexes on same field are collected as candidates
        let query = json!({"status": "active"});
        let index_fields = vec![
            IndexPrefixInfo {
                index_name: "orders_status".to_string(),
                prefix_field: "status".to_string(),
                is_compound: false,
                num_fields: 1,
                sparse: false,
                distinct_count: 5, // Low distinct = high selectivity
                building: false,
                num_keys: 0,
                case_insensitive: false,
                null_count: 0,
                multikey_ratio: 0.0,
                histogram: None,
                mcv: None,
            },
            IndexPrefixInfo {
                index_name: "orders_status_date".to_string(),
                prefix_field: "status".to_string(),
                is_compound: true,
                num_fields: 2,
                sparse: false,
                distinct_count: 100, // Higher distinct
                building: false,
                num_keys: 0,
                case_insensitive: false,
                null_count: 0,
                multikey_ratio: 0.0,
                histogram: None,
                mcv: None,
            },
        ];

        let candidates = QueryPlanner::collect_candidates(&query, &index_fields);
        assert_eq!(candidates.len(), 2);

        // Verify both indexes are in candidates
        let names: Vec<_> = candidates
            .iter()
            .map(|c| match &c.plan {
                QueryPlan::IndexScan { index_name, .. } => index_name.as_str(),
                _ => "",
            })
            .collect();
        assert!(names.contains(&"orders_status"));
        assert!(names.contains(&"orders_status_date"));
    }

    #[test]
    fn test_select_best_candidate_by_selectivity() {
        // Test selectivity-based selection
        // Higher distinct_count = fewer rows per value = BETTER for equality queries
        // Cost = total_docs / distinct_count (estimated matching rows)
        let query = json!({"category": "electronics"});
        let index_fields = vec![
            IndexPrefixInfo {
                index_name: "products_category".to_string(),
                prefix_field: "category".to_string(),
                is_compound: false,
                num_fields: 1,
                sparse: false,
                distinct_count: 10, // Low distinct = ~100 rows per value = higher cost
                building: false,
                num_keys: 0,
                case_insensitive: false,
                null_count: 0,
                multikey_ratio: 0.0,
                histogram: None,
                mcv: None,
            },
            IndexPrefixInfo {
                index_name: "products_category_brand".to_string(),
                prefix_field: "category".to_string(),
                is_compound: true,
                num_fields: 2,
                sparse: false,
                distinct_count: 1000, // High distinct = ~1 row per value = lower cost
                building: false,
                num_keys: 0,
                case_insensitive: false,
                null_count: 0,
                multikey_ratio: 0.0,
                histogram: None,
                mcv: None,
            },
        ];

        let candidates = QueryPlanner::collect_candidates(&query, &index_fields);
        let best = QueryPlanner::select_best_candidate(candidates);

        assert!(best.is_some());
        let best = best.unwrap();
        assert_eq!(best.field, "category");

        // The compound index with HIGHER distinct_count should win (lower estimated rows)
        match best.plan {
            QueryPlan::IndexScan { index_name, .. } => {
                assert_eq!(index_name, "products_category_brand");
            }
            _ => panic!("Expected IndexScan"),
        }
    }

    #[test]
    fn test_candidate_plan_cost_calculation() {
        let plan = QueryPlan::IndexScan {
            index_name: "test_idx".to_string(),
            field: "field".to_string(),
            key: IndexKey::Int(1),
            is_compound: false,
        };

        // Test with_selectivity cost calculation (no multikey)
        let candidate = CandidatePlan::with_selectivity(
            plan.clone(),
            "field".to_string(),
            "test_idx",
            10,   // distinct_count
            1000, // total_docs
            0.0,  // multikey_ratio (no multikey)
        );

        // Cost should be total_docs / distinct_count = 1000 / 10 = 100 (no overhead)
        assert!((candidate.estimated_cost - 100.0).abs() < 0.001);
        assert!(candidate.reason.contains("distinct: 10"));
        assert!(candidate.reason.contains("est. rows: 100"));

        // Test with multikey overhead (25% of docs have arrays)
        let candidate_multikey = CandidatePlan::with_selectivity(
            plan,
            "field".to_string(),
            "test_idx",
            10,   // distinct_count
            1000, // total_docs
            0.25, // multikey_ratio (25% multikey)
        );

        // Cost should be 100 * 1.125 = 112.5 (1.0 + 0.25 * 0.5 overhead)
        assert!((candidate_multikey.estimated_cost - 112.5).abs() < 0.001);
    }

    #[test]
    fn test_candidate_plan_default_cost() {
        let plan = QueryPlan::IndexScan {
            index_name: "test_idx".to_string(),
            field: "field".to_string(),
            key: IndexKey::Int(1),
            is_compound: false,
        };

        let candidate = CandidatePlan::with_default_cost(plan, "field".to_string(), "test_idx");

        // High cost (1_000_000) when no statistics available - prefer indexes with known stats
        assert_eq!(candidate.estimated_cost, 1_000_000.0);
        assert!(candidate.reason.contains("test_idx"));
        assert!(candidate.reason.contains("no stats"));
    }

    #[test]
    fn test_explain_with_candidates() {
        // Test the new explain format with candidates
        let query = json!({"status": "active"});
        let index_fields = vec![
            IndexPrefixInfo {
                index_name: "orders_status".to_string(),
                prefix_field: "status".to_string(),
                is_compound: false,
                num_fields: 1,
                sparse: false,
                distinct_count: 5,
                building: false,
                num_keys: 0,
                case_insensitive: false,
                null_count: 0,
                multikey_ratio: 0.0,
                histogram: None,
                mcv: None,
            },
            IndexPrefixInfo {
                index_name: "orders_status_date".to_string(),
                prefix_field: "status".to_string(),
                is_compound: true,
                num_fields: 2,
                sparse: false,
                distinct_count: 100,
                building: false,
                num_keys: 0,
                case_insensitive: false,
                null_count: 0,
                multikey_ratio: 0.0,
                histogram: None,
                mcv: None,
            },
        ];

        let explain = QueryPlanner::explain_query_with_fields(&query, &index_fields);

        // Check new explain format
        assert!(explain.get("chosenPlan").is_some());
        assert!(explain.get("selectionReason").is_some());
        assert!(explain.get("estimatedRows").is_some());
        assert!(explain.get("candidates").is_some());
        assert!(explain.get("candidateCount").is_some());

        // Should have 2 candidates
        assert_eq!(explain["candidateCount"], 2);

        // Candidates should be an array
        let candidates = explain["candidates"].as_array().unwrap();
        assert_eq!(candidates.len(), 2);

        // Each candidate should have required fields
        for candidate in candidates {
            assert!(candidate.get("plan").is_some());
            assert!(candidate.get("field").is_some());
            assert!(candidate.get("estimatedRows").is_some());
            assert!(candidate.get("reason").is_some());
        }
    }

    #[test]
    fn test_explain_collection_scan_no_candidates() {
        // Test explain when no index is available
        let query = json!({"unknownField": "value"});
        let index_fields = vec![IndexPrefixInfo {
            index_name: "orders_status".to_string(),
            prefix_field: "status".to_string(),
            is_compound: false,
            num_fields: 1,
            sparse: false,
            distinct_count: 5,
            building: false,
            num_keys: 0,
            case_insensitive: false,
            null_count: 0,
            multikey_ratio: 0.0,
            histogram: None,
            mcv: None,
        }];

        let explain = QueryPlanner::explain_query_with_fields(&query, &index_fields);

        // Check collection scan explain
        assert!(explain.get("chosenPlan").is_some());
        assert_eq!(explain["chosenPlan"]["queryPlan"], "CollectionScan");
        assert_eq!(explain["candidateCount"], 0);
        assert!(explain.get("availableIndexes").is_some());
    }

    #[test]
    fn test_parse_regex_prefix_basic() {
        // Basic prefix extraction
        let info = QueryPlanner::parse_regex_prefix("^Hello", None).unwrap();
        assert_eq!(info.prefix, "Hello");
        assert!(info.exact);
        assert!(!info.case_insensitive);
    }

    #[test]
    fn test_parse_regex_prefix_with_options_rejected() {
        // $options should be rejected
        let result = QueryPlanner::parse_regex_prefix("^Hello", Some("i"));
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_regex_prefix_empty_options_ok() {
        // Empty $options should be allowed
        let info = QueryPlanner::parse_regex_prefix("^Hello", Some("")).unwrap();
        assert_eq!(info.prefix, "Hello");
    }

    #[test]
    fn test_parse_regex_prefix_case_insensitive() {
        // (?i) flag
        let info = QueryPlanner::parse_regex_prefix("(?i)^World", None).unwrap();
        assert_eq!(info.prefix, "World");
        assert!(info.exact);
        assert!(info.case_insensitive);
    }

    #[test]
    fn test_regex_prefix_info_is_optimizable() {
        // Regular prefix - optimizable
        let info = RegexPrefixInfo {
            prefix: "test".to_string(),
            exact: true,
            case_insensitive: false,
        };
        assert!(info.is_optimizable());
        assert!(info.is_optimizable_for_in());

        // CI prefix - not optimizable for standard index
        let ci_info = RegexPrefixInfo {
            prefix: "test".to_string(),
            exact: true,
            case_insensitive: true,
        };
        assert!(!ci_info.is_optimizable());
        assert!(!ci_info.is_optimizable_for_in());

        // Non-exact prefix - not optimizable for $in
        let non_exact = RegexPrefixInfo {
            prefix: "test".to_string(),
            exact: false,
            case_insensitive: false,
        };
        assert!(non_exact.is_optimizable());
        assert!(!non_exact.is_optimizable_for_in());
    }

    // --- Clause merge tests ---

    fn make_index(field: &str, name: &str) -> IndexPrefixInfo {
        IndexPrefixInfo {
            index_name: name.to_string(),
            prefix_field: field.to_string(),
            is_compound: false,
            num_fields: 1,
            sparse: false,
            distinct_count: 0,
            building: false,
            num_keys: 0,
            case_insensitive: false,
            null_count: 0,
            multikey_ratio: 0.0,
            histogram: None,
            mcv: None,
        }
    }

    #[test]
    fn audit27_find_index_prefers_single_over_compound() {
        // Finding B: with both a compound and a single index on the same field,
        // the choice must be deterministic (prefer the single one) regardless of
        // the unordered list order — otherwise a compound-first ordering dropped
        // the field to a collection scan at the `if is_compound { continue }` sites.
        let mk_compound = || {
            let mut c = make_index("year", "idx_year_type");
            c.is_compound = true;
            c.num_fields = 2;
            c
        };
        let idx = vec![mk_compound(), make_index("year", "idx_year")];
        let (name, is_compound) =
            QueryPlanner::find_index_for_field("year", &idx).expect("index found");
        assert_eq!(name, "idx_year");
        assert!(!is_compound, "must prefer the single-field index");
        // Reverse order yields the same deterministic result.
        let idx_rev = vec![make_index("year", "idx_year"), mk_compound()];
        assert_eq!(
            QueryPlanner::find_index_for_field("year", &idx_rev)
                .unwrap()
                .0,
            "idx_year"
        );
        // With only a compound index, it is still returned (is_compound = true).
        let only = vec![mk_compound()];
        assert!(
            QueryPlanner::find_index_for_field("year", &only).unwrap().1,
            "only-compound index is returned"
        );
    }

    #[test]
    fn audit27_mixed_type_range_bounds_are_impossible() {
        // Finding C: bounds of different value-type categories form an impossible
        // range; resolve_range_conditions must return None (not a 0-cost index pick).
        let mixed = [
            (json!(5), json!(false)),  // Number / Bool
            (json!(5), json!(null)),   // Number / Null
            (json!("a"), json!(5)),    // String / Number (the original case)
            (json!(true), json!("z")), // Bool / String
        ];
        for (lo, hi) in mixed {
            let conds = vec![
                ("$gte".to_string(), lo.clone()),
                ("$lte".to_string(), hi.clone()),
            ];
            assert!(
                QueryPlanner::resolve_range_conditions(&conds).is_none(),
                "mixed-type bounds {lo}/{hi} must be impossible"
            );
        }
        // A valid same-type range must survive (not over-filtered).
        let ok = vec![
            ("$gte".to_string(), json!(5)),
            ("$lte".to_string(), json!(10)),
        ];
        assert!(QueryPlanner::resolve_range_conditions(&ok).is_some());
    }

    #[test]
    fn test_merge_and_same_field_range() {
        // $and: [{year: {$gte: 2024}}, {year: {$lte: 2025}}] → {year: {$gte: 2024, $lte: 2025}}
        let clauses = vec![
            json!({"year": {"$gte": 2024}}),
            json!({"year": {"$lte": 2025}}),
        ];
        let indexes = vec![make_index("year", "idx_year")];

        let result =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::And, &clauses, &indexes);
        assert!(result.is_some());
        let merged = result.unwrap();
        let year_obj = merged.get("year").unwrap().as_object().unwrap();
        assert_eq!(year_obj.get("$gte").unwrap(), &json!(2024));
        assert_eq!(year_obj.get("$lte").unwrap(), &json!(2025));
    }

    #[test]
    fn test_merge_and_conflicting_gte_takes_tighter() {
        // $and: [{year: {$gte: 2020}}, {year: {$gte: 2024}}] → {year: {$gte: 2024}}
        let clauses = vec![
            json!({"year": {"$gte": 2020}}),
            json!({"year": {"$gte": 2024}}),
        ];
        let indexes = vec![make_index("year", "idx_year")];

        let result =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::And, &clauses, &indexes);
        assert!(result.is_some());
        let merged = result.unwrap();
        let year_obj = merged.get("year").unwrap().as_object().unwrap();
        assert_eq!(year_obj.get("$gte").unwrap(), &json!(2024));
        assert!(year_obj.get("$lte").is_none());
    }

    #[test]
    fn test_merge_and_conflicting_lte_takes_tighter() {
        // $and: [{year: {$lte: 2030}}, {year: {$lte: 2025}}] → {year: {$lte: 2025}}
        let clauses = vec![
            json!({"year": {"$lte": 2030}}),
            json!({"year": {"$lte": 2025}}),
        ];
        let indexes = vec![make_index("year", "idx_year")];

        let result =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::And, &clauses, &indexes);
        assert!(result.is_some());
        let merged = result.unwrap();
        let year_obj = merged.get("year").unwrap().as_object().unwrap();
        assert_eq!(year_obj.get("$lte").unwrap(), &json!(2025));
    }

    #[test]
    fn test_merge_and_different_fields() {
        // $and: [{year: {$gte: 2024}}, {name: "Alice"}] → {year: {$gte: 2024}, name: {$eq: "Alice"}}
        let clauses = vec![json!({"year": {"$gte": 2024}}), json!({"name": "Alice"})];
        let indexes = vec![
            make_index("year", "idx_year"),
            make_index("name", "idx_name"),
        ];

        let result =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::And, &clauses, &indexes);
        assert!(result.is_some());
        let merged = result.unwrap();
        assert!(merged.get("year").is_some());
        assert!(merged.get("name").is_some());
    }

    #[test]
    fn test_merge_and_bare_equality() {
        // $and: [{year: 2024}, {name: "Alice"}] → {year: {$eq: 2024}, name: {$eq: "Alice"}}
        let clauses = vec![json!({"year": 2024}), json!({"name": "Alice"})];
        let indexes = vec![make_index("year", "idx_year")];

        let result =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::And, &clauses, &indexes);
        assert!(result.is_some());
        let merged = result.unwrap();
        let year_obj = merged.get("year").unwrap().as_object().unwrap();
        assert_eq!(year_obj.get("$eq").unwrap(), &json!(2024));
    }

    #[test]
    fn test_merge_or_same_field_equality_to_in() {
        // $or: [{year: 2024}, {year: 2025}] → {year: {$in: [2024, 2025]}}
        let clauses = vec![json!({"year": 2024}), json!({"year": 2025})];
        let indexes = vec![make_index("year", "idx_year")];

        let result =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::Or, &clauses, &indexes);
        assert!(result.is_some());
        let merged = result.unwrap();
        let year_obj = merged.get("year").unwrap().as_object().unwrap();
        let in_arr = year_obj.get("$in").unwrap().as_array().unwrap();
        assert_eq!(in_arr.len(), 2);
        assert!(in_arr.contains(&json!(2024)));
        assert!(in_arr.contains(&json!(2025)));
    }

    #[test]
    fn test_merge_or_explicit_eq_to_in() {
        // $or: [{year: {$eq: 2024}}, {year: {$eq: 2025}}] → {year: {$in: [2024, 2025]}}
        let clauses = vec![
            json!({"year": {"$eq": 2024}}),
            json!({"year": {"$eq": 2025}}),
        ];
        let indexes = vec![make_index("year", "idx_year")];

        let result =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::Or, &clauses, &indexes);
        assert!(result.is_some());
        let merged = result.unwrap();
        let year_obj = merged.get("year").unwrap().as_object().unwrap();
        let in_arr = year_obj.get("$in").unwrap().as_array().unwrap();
        assert_eq!(in_arr.len(), 2);
    }

    // --- Edge cases that should NOT merge ---

    #[test]
    fn test_merge_and_conflicting_equality_returns_none() {
        // $and: [{year: 2024}, {year: 2025}] → None (conflicting equalities)
        let clauses = vec![json!({"year": 2024}), json!({"year": 2025})];
        let indexes = vec![make_index("year", "idx_year")];

        let result =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::And, &clauses, &indexes);
        assert!(result.is_none());
    }

    #[test]
    fn test_merge_or_range_returns_none() {
        // $or: [{year: {$gte: 2024}}, {year: {$lte: 2020}}] → None (range $or not mergeable)
        let clauses = vec![
            json!({"year": {"$gte": 2024}}),
            json!({"year": {"$lte": 2020}}),
        ];
        let indexes = vec![make_index("year", "idx_year")];

        let result =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::Or, &clauses, &indexes);
        assert!(result.is_none());
    }

    #[test]
    fn test_merge_or_different_fields_returns_none() {
        // $or: [{year: 2024}, {name: "Alice"}] → None (different fields)
        let clauses = vec![json!({"year": 2024}), json!({"name": "Alice"})];
        let indexes = vec![
            make_index("year", "idx_year"),
            make_index("name", "idx_name"),
        ];

        let result =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::Or, &clauses, &indexes);
        assert!(result.is_none());
    }

    #[test]
    fn test_merge_nested_logical_returns_none() {
        // $and: [{$or: [{year: 2024}]}, {year: 2025}] → None (nested logical)
        let clauses = vec![json!({"$or": [{"year": 2024}]}), json!({"year": 2025})];
        let indexes = vec![make_index("year", "idx_year")];

        let result =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::And, &clauses, &indexes);
        assert!(result.is_none());
    }

    #[test]
    fn test_merge_multi_field_clause_returns_none() {
        // $and: [{year: 2024, name: "A"}, {city: "B"}] → None (multi-field clause)
        let clauses = vec![json!({"year": 2024, "name": "A"}), json!({"city": "B"})];
        let indexes = vec![make_index("year", "idx_year")];

        let result =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::And, &clauses, &indexes);
        assert!(result.is_none());
    }

    #[test]
    fn test_merge_no_index_returns_none() {
        // No index on the field → None
        let clauses = vec![
            json!({"year": {"$gte": 2024}}),
            json!({"year": {"$lte": 2025}}),
        ];
        let indexes: Vec<IndexPrefixInfo> = vec![]; // No indexes

        let result =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::And, &clauses, &indexes);
        assert!(result.is_none());
    }

    #[test]
    fn test_merge_regex_returns_none() {
        // $and with $regex → None (unsupported operator)
        let clauses = vec![
            json!({"name": {"$regex": "^A"}}),
            json!({"name": {"$regex": ".*B$"}}),
        ];
        let indexes = vec![make_index("name", "idx_name")];

        let result =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::And, &clauses, &indexes);
        assert!(result.is_none());
    }

    #[test]
    fn test_merge_nor_returns_none() {
        // $nor never merges
        let clauses = vec![json!({"year": 2024}), json!({"year": 2025})];
        let indexes = vec![make_index("year", "idx_year")];

        let result =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::Nor, &clauses, &indexes);
        assert!(result.is_none());
    }

    #[test]
    fn test_merge_and_produces_valid_plan() {
        // End-to-end: merged query should produce an IndexRangeScan plan
        let clauses = vec![
            json!({"year": {"$gte": 2024}}),
            json!({"year": {"$lte": 2025}}),
        ];
        let indexes = vec![make_index("year", "idx_year")];

        let merged =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::And, &clauses, &indexes)
                .unwrap();

        let (_, plan) = QueryPlanner::analyze_query_with_fields(&merged, &indexes).unwrap();
        match plan {
            QueryPlan::IndexRangeScan {
                index_name,
                start,
                end,
                inclusive_start,
                inclusive_end,
                ..
            } => {
                assert_eq!(index_name, "idx_year");
                assert!(start.is_some());
                assert!(end.is_some());
                assert!(inclusive_start);
                assert!(inclusive_end);
            }
            other => panic!("Expected IndexRangeScan, got {:?}", other),
        }
    }

    #[test]
    fn test_merge_or_produces_valid_plan() {
        // End-to-end: merged $or query should produce a MultiValueScan plan
        let clauses = vec![json!({"year": 2024}), json!({"year": 2025})];
        let indexes = vec![make_index("year", "idx_year")];

        let merged =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::Or, &clauses, &indexes)
                .unwrap();

        let (_, plan) = QueryPlanner::analyze_query_with_fields(&merged, &indexes).unwrap();
        match plan {
            QueryPlan::MultiValueScan {
                index_name, keys, ..
            } => {
                assert_eq!(index_name, "idx_year");
                assert_eq!(keys.len(), 2);
            }
            other => panic!("Expected MultiValueScan, got {:?}", other),
        }
    }

    #[test]
    fn test_merge_explain_shows_optimization() {
        let query = json!({"$and": [{"year": {"$gte": 2024}}, {"year": {"$lte": 2025}}]});
        let indexes = vec![make_index("year", "idx_year")];

        let explain = QueryPlanner::explain_query_with_fields(&query, &indexes);
        assert_eq!(explain.get("optimization").unwrap(), "clause_merge");
        assert_eq!(explain.get("originalOperator").unwrap(), "$and");
        assert!(explain.get("mergedQuery").is_some());
    }

    // --- Additional edge case tests (code review follow-up) ---

    #[test]
    fn test_merge_and_eq_plus_range_returns_none() {
        // $eq combined with range → None (conflicting or redundant)
        let clauses = vec![json!({"year": {"$eq": 5}}), json!({"year": {"$gte": 10}})];
        let indexes = vec![make_index("year", "idx_year")];
        let result =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::And, &clauses, &indexes);
        assert!(result.is_none(), "$eq + $gte should not merge");
    }

    #[test]
    fn test_merge_and_contradictory_range_returns_none() {
        // $gte: 100, $lte: 10 → impossible range → None
        let clauses = vec![
            json!({"year": {"$gte": 100}}),
            json!({"year": {"$lte": 10}}),
        ];
        let indexes = vec![make_index("year", "idx_year")];
        let result =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::And, &clauses, &indexes);
        assert!(result.is_none(), "contradictory range should not merge");
    }

    #[test]
    fn test_merge_and_gt_lt_exclusive_bounds() {
        // $gt/$lt (exclusive) should merge correctly
        let clauses = vec![
            json!({"year": {"$gt": 2024}}),
            json!({"year": {"$lt": 2026}}),
        ];
        let indexes = vec![make_index("year", "idx_year")];
        let result =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::And, &clauses, &indexes);
        assert!(result.is_some());
        let merged = result.unwrap();
        let year_obj = merged.get("year").unwrap().as_object().unwrap();
        assert_eq!(year_obj.get("$gt").unwrap(), &json!(2024));
        assert_eq!(year_obj.get("$lt").unwrap(), &json!(2026));
    }

    #[test]
    fn test_merge_and_conflicting_gt_takes_tighter() {
        // 2× $gt: max of the two
        let clauses = vec![
            json!({"year": {"$gt": 2020}}),
            json!({"year": {"$gt": 2024}}),
        ];
        let indexes = vec![make_index("year", "idx_year")];
        let result =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::And, &clauses, &indexes);
        assert!(result.is_some());
        let merged = result.unwrap();
        let year_obj = merged.get("year").unwrap().as_object().unwrap();
        assert_eq!(year_obj.get("$gt").unwrap(), &json!(2024));
    }

    #[test]
    fn test_merge_and_string_range() {
        // String range: {$gte: "Alice", $lte: "Bob"}
        let clauses = vec![
            json!({"name": {"$gte": "Alice"}}),
            json!({"name": {"$lte": "Bob"}}),
        ];
        let indexes = vec![make_index("name", "idx_name")];
        let result =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::And, &clauses, &indexes);
        assert!(result.is_some());
        let merged = result.unwrap();
        let name_obj = merged.get("name").unwrap().as_object().unwrap();
        assert_eq!(name_obj.get("$gte").unwrap(), &json!("Alice"));
        assert_eq!(name_obj.get("$lte").unwrap(), &json!("Bob"));
    }

    #[test]
    fn test_merge_and_string_contradictory_range_returns_none() {
        // "Zebra" > "Alice" → contradictory → None
        let clauses = vec![
            json!({"name": {"$gte": "Zebra"}}),
            json!({"name": {"$lte": "Alice"}}),
        ];
        let indexes = vec![make_index("name", "idx_name")];
        let result =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::And, &clauses, &indexes);
        assert!(
            result.is_none(),
            "contradictory string range should not merge"
        );
    }

    #[test]
    fn test_merge_or_boolean_equality() {
        // $or: [{active: true}, {active: false}] → {active: {$in: [true, false]}}
        let clauses = vec![json!({"active": true}), json!({"active": false})];
        let indexes = vec![make_index("active", "idx_active")];
        let result =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::Or, &clauses, &indexes);
        assert!(result.is_some());
        let merged = result.unwrap();
        let active_obj = merged.get("active").unwrap().as_object().unwrap();
        let in_arr = active_obj.get("$in").unwrap().as_array().unwrap();
        assert_eq!(in_arr.len(), 2);
    }

    #[test]
    fn test_merge_or_null_equality() {
        // $or: [{field: null}, {field: 123}] → {field: {$in: [null, 123]}}
        let clauses = vec![json!({"field": null}), json!({"field": 123})];
        let indexes = vec![make_index("field", "idx_field")];
        let result =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::Or, &clauses, &indexes);
        assert!(result.is_some());
        let merged = result.unwrap();
        let field_obj = merged.get("field").unwrap().as_object().unwrap();
        let in_arr = field_obj.get("$in").unwrap().as_array().unwrap();
        assert_eq!(in_arr.len(), 2);
    }

    #[test]
    fn test_merge_and_single_clause() {
        // $and: [{year: {$gte: 2024}}] → {year: {$gte: 2024}} (trivial merge)
        let clauses = vec![json!({"year": {"$gte": 2024}})];
        let indexes = vec![make_index("year", "idx_year")];
        let result =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::And, &clauses, &indexes);
        assert!(result.is_some());
        let merged = result.unwrap();
        let year_obj = merged.get("year").unwrap().as_object().unwrap();
        assert_eq!(year_obj.get("$gte").unwrap(), &json!(2024));
    }

    #[test]
    fn test_merge_or_single_clause() {
        // $or: [{year: 2024}] → {year: {$in: [2024]}} (trivial merge)
        let clauses = vec![json!({"year": 2024})];
        let indexes = vec![make_index("year", "idx_year")];
        let result =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::Or, &clauses, &indexes);
        assert!(result.is_some());
        let merged = result.unwrap();
        let year_obj = merged.get("year").unwrap().as_object().unwrap();
        assert!(year_obj.get("$in").is_some());
    }

    #[test]
    fn test_merge_and_ne_passthrough() {
        // $and with $ne: should pass through as-is
        let clauses = vec![
            json!({"year": {"$ne": 2024}}),
            json!({"year": {"$gte": 2020}}),
        ];
        let indexes = vec![make_index("year", "idx_year")];
        let result =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::And, &clauses, &indexes);
        assert!(result.is_some());
        let merged = result.unwrap();
        let year_obj = merged.get("year").unwrap().as_object().unwrap();
        assert_eq!(year_obj.get("$ne").unwrap(), &json!(2024));
        assert_eq!(year_obj.get("$gte").unwrap(), &json!(2020));
    }

    #[test]
    fn test_merge_and_exists_passthrough() {
        // $and with $exists: should pass through
        let clauses = vec![
            json!({"year": {"$exists": true}}),
            json!({"year": {"$gte": 2020}}),
        ];
        let indexes = vec![make_index("year", "idx_year")];
        let result =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::And, &clauses, &indexes);
        assert!(result.is_some());
        let merged = result.unwrap();
        let year_obj = merged.get("year").unwrap().as_object().unwrap();
        assert_eq!(year_obj.get("$exists").unwrap(), &json!(true));
        assert_eq!(year_obj.get("$gte").unwrap(), &json!(2020));
    }

    #[test]
    fn test_merge_and_mixed_gt_gte_keeps_both() {
        // $gt and $gte on same field → both kept (engine picks the tighter)
        let clauses = vec![
            json!({"year": {"$gt": 2020}}),
            json!({"year": {"$gte": 2025}}),
        ];
        let indexes = vec![make_index("year", "idx_year")];
        let result =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::And, &clauses, &indexes);
        assert!(result.is_some());
        let merged = result.unwrap();
        let year_obj = merged.get("year").unwrap().as_object().unwrap();
        assert_eq!(year_obj.get("$gt").unwrap(), &json!(2020));
        assert_eq!(year_obj.get("$gte").unwrap(), &json!(2025));
    }

    #[test]
    fn test_merge_or_with_operator_returns_none() {
        // $or with non-equality operator → None
        let clauses = vec![
            json!({"year": {"$gte": 2024}}),
            json!({"year": {"$lte": 2020}}),
        ];
        let indexes = vec![make_index("year", "idx_year")];
        let result =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::Or, &clauses, &indexes);
        assert!(result.is_none());
    }

    #[test]
    fn test_merge_or_mixed_bare_and_explicit_eq() {
        // $or: [{year: 2024}, {year: {$eq: 2025}}] → {year: {$in: [2024, 2025]}}
        let clauses = vec![json!({"year": 2024}), json!({"year": {"$eq": 2025}})];
        let indexes = vec![make_index("year", "idx_year")];
        let result =
            QueryPlanner::try_merge_logical_clauses(LogicalOperator::Or, &clauses, &indexes);
        assert!(result.is_some());
        let merged = result.unwrap();
        let year_obj = merged.get("year").unwrap().as_object().unwrap();
        let in_arr = year_obj.get("$in").unwrap().as_array().unwrap();
        assert_eq!(in_arr.len(), 2);
        assert!(in_arr.contains(&json!(2024)));
        assert!(in_arr.contains(&json!(2025)));
    }

    /// Audit #27 finding A — Object/Array equality MUST NOT pick an index.
    ///
    /// `IndexKey::from(&Value)` collapses every Object and Array value to
    /// `IndexKey::Null` (key.rs:119, `_ => IndexKey::Null`). If the planner
    /// treats `{field: {sub: "x"}}` as an equality candidate, it builds an
    /// `IndexScan { key: Null, ... }` plan that points at NULL-valued docs,
    /// not at docs whose `field` deep-equals the object. Combined with
    /// `count_documents`'s `fully_covered=true` short-circuit
    /// (count.rs:215), the planner returns the wrong count silently.
    ///
    /// Correct behaviour: skip the index for Object/Array equality
    /// candidates, fall through to collection scan + `$eq` operator
    /// (which performs proper deep equality).
    #[test]
    fn audit27_object_equality_does_not_pick_index() {
        let query = json!({"address": {"city": "NYC"}});
        let index_fields = vec![IndexPrefixInfo {
            index_name: "by_address".to_string(),
            prefix_field: "address".to_string(),
            is_compound: false,
            num_fields: 1,
            sparse: false,
            distinct_count: 100,
            building: false,
            num_keys: 100,
            case_insensitive: false,
            null_count: 0,
            multikey_ratio: 0.0,
            histogram: None,
            mcv: None,
        }];
        let plan = QueryPlanner::analyze_query_with_fields(&query, &index_fields);
        // The planner MUST NOT pick an IndexScan for an object-valued
        // equality — IndexKey::Null would mismatch every real address.
        assert!(
            !matches!(plan, Some((_, QueryPlan::IndexScan { .. }))),
            "object-valued equality must not pick an IndexScan; got {:?}",
            plan
        );
    }

    /// Audit #27 finding A — same issue but for Array equality:
    /// `{tags: ["x", "y"]}` with an index on `tags` must not produce an
    /// IndexScan (the array → Null collapse is identical).
    #[test]
    fn audit27_array_equality_does_not_pick_index() {
        let query = json!({"tags": ["x", "y"]});
        let index_fields = vec![IndexPrefixInfo {
            index_name: "by_tags".to_string(),
            prefix_field: "tags".to_string(),
            is_compound: false,
            num_fields: 1,
            sparse: false,
            distinct_count: 50,
            building: false,
            num_keys: 50,
            case_insensitive: false,
            null_count: 0,
            multikey_ratio: 0.0,
            histogram: None,
            mcv: None,
        }];
        let plan = QueryPlanner::analyze_query_with_fields(&query, &index_fields);
        assert!(
            !matches!(plan, Some((_, QueryPlan::IndexScan { .. }))),
            "array-valued equality must not pick an IndexScan; got {:?}",
            plan
        );
    }

    /// Audit #27 finding A — explicit `$eq` with object value must also
    /// skip the index (same root cause: the inner value is still an Object
    /// that collapses to IndexKey::Null).
    #[test]
    fn audit27_explicit_eq_object_does_not_pick_index() {
        let query = json!({"address": {"$eq": {"city": "NYC"}}});
        let index_fields = vec![IndexPrefixInfo {
            index_name: "by_address".to_string(),
            prefix_field: "address".to_string(),
            is_compound: false,
            num_fields: 1,
            sparse: false,
            distinct_count: 100,
            building: false,
            num_keys: 100,
            case_insensitive: false,
            null_count: 0,
            multikey_ratio: 0.0,
            histogram: None,
            mcv: None,
        }];
        let plan = QueryPlanner::analyze_query_with_fields(&query, &index_fields);
        assert!(
            !matches!(plan, Some((_, QueryPlan::IndexScan { .. }))),
            "explicit $eq with object value must not pick an IndexScan; got {:?}",
            plan
        );
    }
}
