# Aggregation Optimizer Plan

> **STATUS (2026-06-10, historical):** Phase 1 pattern detection lives in
> `ironbase-core/src/aggregation/optimizer.rs` (`analyze_pipeline`: CountOnly fast path +
> $sort+$limit Top-K hint). The Phase 2 cost model described below
> (`AggregationLogicalPlan`, `AggregationPhysicalPlan`, `CostEstimate`, `select_plan`) **was
> implemented, had no live consumer, and was removed** (commit 2b7822f1, core v0.3.340) to
> collapse two sources of truth into one. Index-based per-field counting (the IndexGroup idea)
> is decided by the executor instead: `GroupStage::can_use_index` /
> `try_index_based_execute_with_context`. **Do not re-implement Phase 2 from this document** —
> if a cost model becomes necessary, start from the executor-side decision points.

## Goals
- Preserve result correctness while reducing execution time for common aggregation patterns.
- Introduce a planner that can select efficient physical strategies (count, index scan, full scan).
- Keep fallbacks safe: if optimization is not provably equivalent, run the current pipeline engine.

## Scope (Initial)
- Optimize $group patterns with simple keys and count/sum accumulators.
- Add a minimal cost model using available stats (document count, index presence).
- Extend later to top-k group and richer aggregations.

## Phase 0: Inventory and Baseline
- Document current aggregation execution paths and hot spots.
- Add lightweight telemetry (optional) to measure pipeline time, docs processed, and memory.
- Define a compatibility test suite that compares optimized vs baseline outputs.

## Phase 1: Logical Plan + Rule-Based Rewrite
- Parse pipeline into a logical plan AST (Match, Project, Group, Sort, Limit, Skip).
- Implement rule-based rewrites with equivalence checks:
  - $group with _id: null and only $sum:1 => CountOnly
  - $match + $group _id:null + $sum:1 => CountOnly with filter
  - $group _id:"$field" + only $sum:1 => IndexGroup if single-field B+ index exists
- Add guardrails:
  - Reject rewrite when _id is expression, array, or compound object.
  - Reject when non-count accumulators exist (avg, min, max, push, addToSet, etc).

### Phase 1 Implementation Steps
- Add `AggregationLogicalPlan` enum and builder from pipeline JSON.
- Introduce `GroupShape` detector:
  - `_id` kind: null, field path, expression, compound object.
  - Accumulators: list, only `$sum: 1` is fast-path eligible.
- Implement `rewrite_plan(plan) -> plan` with targeted transformations.
- Add `explain` output to include `logicalPlan` and `rewritesApplied` for debugging.

## Phase 2: Physical Plan Selection (Cost Model)
- Introduce physical operators:
  - FullScanGroup
  - CountOnly
  - IndexGroup (prefix scan)
  - TopKGroup (optional)
- Implement simple cost model:
  - If CountOnly possible => choose it.
  - If IndexGroup possible and estimated cost < FullScanGroup => choose IndexGroup.
  - Otherwise fallback to FullScanGroup.
- Estimated cost inputs (minimal):
  - Collection doc count
  - Index presence for group key

### Phase 2 Implementation Steps
- Add `AggregationPhysicalPlan` and planner:
  - `plan_physical(logical, stats, indexes) -> physical`.
- Add `CostEstimate` struct with coarse weights:
  - `full_scan_cost = doc_count`
  - `index_group_cost = index_entries` (or doc_count if unknown)
  - `count_only_cost = 1`
- Wire planner into `aggregate()` execution path with safe fallback.
- Add tests for plan selection using mocked stats.

## Phase 3: Enriched Statistics (Optional)
- Track per-index cardinality estimates and sample histograms.
- Use stats to estimate selectivity for $match and group key distribution.
- Refine cost model decisions for borderline cases.

### Phase 3 Implementation Steps
- Extend index metadata with `approx_cardinality` (update on insert/delete).
- Add optional background sampling to estimate distribution.
- Expose stats in `explain` and metrics logs.

## Phase 4: Expanded Rewrite Coverage
- Support $group with:
  - $sum on a field (with index support if field is indexed)
  - $min/$max when sortable index exists
  - $sort + $limit on grouped output (TopKGroup)
- Introduce partial aggregation for large datasets.

### Phase 4 Implementation Steps
- Add accumulator-level capabilities and requirements.
- Implement `TopKGroup` with bounded heap.
- Add `$sum` on field with index-based fast path where possible.

## Testing Strategy
- Unit tests for each rewrite rule (inputs and expected logical plans).
- Property tests: compare optimized output vs baseline output for random datasets.
- Performance regression tests for large collections.

### Testing Implementation Steps
- Add golden tests for `explain` output showing applied rewrites.
- Add fuzz tests for pipeline equivalence (small random datasets).

## Rollout Plan
- Feature flag for optimizer enablement.
- Enable CountOnly and IndexGroup first.
- Collect metrics and expand coverage gradually.

### Rollout Implementation Steps
- Add config flag `AGG_OPTIMIZER=on/off` (default off for safety).
- Add metrics counters: `agg_rewrite_applied`, `agg_plan_full_scan`, `agg_plan_index_group`.

## Open Questions
- What statistics are feasible to compute and persist without large overhead?
- Should optimizer decisions be logged for explain/debug?
- What is the policy for safety vs speed (strict vs permissive mode)?

## Module and File Breakdown
- `ironbase-core/src/aggregation/optimizer.rs`
  - Logical plan definitions, rewrite rules, physical plan selection.
- `ironbase-core/src/aggregation/mod.rs`
  - Entry point wiring for optimizer + fallback to existing engine.
- `ironbase-core/src/aggregation/explain.rs` (new or existing explain output)
  - Attach `logicalPlan`, `physicalPlan`, `rewritesApplied`.
- `ironbase-core/src/index/manager.rs`
  - Expose index presence, prefix info, and optional cardinality stats.
- `ironbase-core/src/database/mod.rs` or stats module
  - Collection-level doc count access for cost model.
- `ironbase-core/tests/aggregation_optimizer_tests.rs` (new)
  - Rewrite correctness tests and plan selection tests.
- `ironbase-core/tests/aggregation_equivalence_tests.rs` (optional)
  - Randomized equivalence tests vs baseline.

## Rough Estimates (Engineering Time)
- Phase 1 (logical plan + basic rewrites): 2-3 days
  - AST + rewrite rules + explain output and tests.
- Phase 2 (physical plan + cost model): 2-4 days
  - Planner, integration, plan selection tests.
- Phase 3 (stats + refinement): 3-6 days
  - Cardinality tracking + explain output + sampling.
- Phase 4 (expanded coverage): 4-8 days
  - TopKGroup, additional accumulator paths, more tests.

## Risk Notes
- Rewrites must be provably equivalent; add explicit guards and tests.
- Index-based group requires stable ordering and correct null handling.
- Explain output must not break existing clients; keep additive fields.
