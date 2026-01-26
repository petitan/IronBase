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
    /// Find an index for a given field (compound index aware)
    ///
    /// Takes a list of IndexPrefixInfo containing:
    /// - index_name: The index name
    /// - prefix_field: The first field (for compound) or only field (for single)
    /// - is_compound: Whether this is a compound index
    ///
    /// Returns (index_name, is_compound) if found.
    fn find_index_for_field(
        field: &str,
        index_fields: &[IndexPrefixInfo],
    ) -> Option<(String, bool)> {
        index_fields
            .iter()
            .find(|info| info.prefix_field == field)
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

        if let Value::Object(ref map) = query_json {
            // Skip logical operators at root level - not optimizable yet
            if map.keys().any(|k| k.starts_with('$')) {
                return candidates;
            }

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
                .find(|i| i.prefix_field == field && i.sparse)
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
                .find(|i| i.prefix_field == field && !i.is_compound)
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

                        // Find matching index for this field
                        if let Some(info) = index_fields
                            .iter()
                            .find(|i| i.prefix_field == *field && !i.is_compound)
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
                .find(|i| i.prefix_field == field && !i.is_compound)
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
                let equality_value: Option<&Value> = if let Value::Object(ref val_map) = value {
                    if val_map.len() == 1 {
                        if let Some(eq_val) = val_map.get("$eq") {
                            // Explicit $eq operator: {"field": {"$eq": X}}
                            Some(eq_val)
                        } else if val_map.keys().any(|k| k.starts_with('$')) {
                            // Other operators like $gt, $in, etc. - skip
                            None
                        } else {
                            // Object value without operators - use as-is
                            Some(value)
                        }
                    } else if val_map.keys().any(|k| k.starts_with('$')) {
                        // Multiple operators or mixed - skip
                        None
                    } else {
                        // Plain object value - use as-is
                        Some(value)
                    }
                } else {
                    // Scalar or array value - use as-is
                    Some(value)
                };

                let eq_val = match equality_value {
                    Some(v) => v,
                    None => continue,
                };

                // Find ALL matching indexes for this field (not just first)
                for info in index_fields.iter().filter(|i| i.prefix_field == *field) {
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
                        // Look for a sparse index on this field
                        if let Some(info) = index_fields
                            .iter()
                            .find(|info| info.prefix_field == *field && info.sparse)
                        {
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

                        let start = if has_gte {
                            cond_map.get("$gte").map(IndexKey::from)
                        } else if has_gt {
                            cond_map.get("$gt").map(IndexKey::from)
                        } else {
                            None
                        };

                        let end = if has_lte {
                            cond_map.get("$lte").map(IndexKey::from)
                        } else if has_lt {
                            cond_map.get("$lt").map(IndexKey::from)
                        } else {
                            None
                        };

                        let inclusive_start = has_gte || (!has_gt && !has_gte);
                        let inclusive_end = has_lte || (!has_lt && !has_lte);

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
                        .find(|i| i.prefix_field == *field && i.case_insensitive)
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

            let operator = match logical_op {
                LogicalOperator::And => "$and",
                LogicalOperator::Or => "$or",
                LogicalOperator::Nor => "$nor",
            };

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
}
