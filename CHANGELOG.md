# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed — index-based $group count path: multiplier overflow + stale planner references (mcp-server v1.0.535, core v0.3.341)

Post-review follow-up on this branch (fresh `/code-review` pass over the full diff).

**Index-based `$group` count overflow.** The v1.0.533 fix made the CountOnly fast path saturate
`(count as i64).saturating_mul(multiplier)` to match the streaming accumulator — but the **third**
path with the same `$sum: <constant>` semantics, the index-based `$group` execution
(`group_stage.rs` `try_index_based_execute_with_context` and the legacy `try_index_based_execute`),
still used plain `n * count`: debug panic / silent release wrap on
`[{$group: {_id: "$city", t: {$sum: i64::MAX}}}]` over an indexed field. Both sites now use
`saturating_mul`; regression test `test_index_based_group_count_saturates_multiplier_overflow`.

**Stale references to the removed planner.** `docs/AGGREGATION_OPTIMIZER_PLAN.md` got a status note
(Phase 2 cost model implemented, found dead, removed in this branch — do not re-implement);
the `(but may use CountByField ...)` comment in `aggregation_context_tests.rs` and the
`LOGICAL PLAN TYPES` banner in `optimizer.rs` no longer name deleted machinery. Re-added the
unit-level guard deleted with the planner tests: `test_no_count_only_with_field_id` pins that a
field-`_id` count never takes the CountOnly fast path (`id_kind` guard).

### BREAKING / Fixed — $count empty-input MongoDB compat + dead aggregation-planner code removed (mcp-server v1.0.534, core v0.3.340)

Follow-up to the aggregate-planner review (findings #3–#5).

**#3 — BREAKING: `$count` over empty input now returns `[]` (MongoDB semantics).**
*Migration:* clients that indexed the result unconditionally (`results[0].n` — including saved Rhai
scripts using `db_aggregate`) must handle an empty result set when the `$match` filters everything
out; previously they received `[{<field>: 0}]`. `$count` is sugar for
`{$group:{_id:null,n:{$sum:1}}},{$project:{_id:0}}`, and a `_id: null` $group emits nothing for zero
input rows — IronBase's streaming `$group` path already returned `[]`, but the `$count` stage returned
`[{<field>: 0}]`, an internal inconsistency. Fixed across **all four** materialization sites so they
agree: `CountStage::execute` (Vec path), both streaming `$count` branches in `pipeline.rs`
(`execute_streaming_with_limits`, `execute_with_context`), and the count-only fast path in
`collection_core/aggregate.rs` (the `include_id && count == 0` guard became `count == 0`, since the
`$count` form now also returns `[]`). New tests cover each path
(`test_count_stage_empty_input`, `test_count_stage_empty_fastpath_returns_no_doc`,
`test_count_stage_empty_streaming_returns_no_doc`, `test_count_after_empty_group_returns_no_doc`); the
`prop_aggregate_match_count_matches_count_documents` property test (which already tolerated empty
results) still passes.

**#4/#5/#6 — removed dead aggregation-planner code (`aggregation/optimizer.rs`).** The entire unused
"Phase 2" cost model (`LogicalPlan`, `PhysicalPlan`, `select_plan`, `CollectionStats`, `CostEstimate`)
had no live consumer — only its own unit tests. Likewise the `CountByField` fast-path detection
(`FastPath::CountByField`, `GroupShape::is_count_by_field`, `is_sort_limit_pattern`,
`AccumulatorKind::is_index_minmax`) computed a value that `aggregate.rs` immediately discarded; the
real index-based per-field count is decided independently by `GroupStage::can_use_index` /
`try_index_based_execute_with_context`, so removing the discarded detection collapses the two sources
of truth (#6) to one. Net effect: ~200 lines deleted, the discarded `FastPath::CountByField` match arm
in `aggregate.rs` removed, no behavior change (full suite + clippy + fmt green). The live optimizer
surface (`analyze_pipeline`, `GroupShape`/`is_count_only`, `FastPath::CountOnly`, the $sort+$limit
Top-K hint) is unchanged.

### Fixed — aggregate CountOnly fast path: spurious empty-input document + multiplier overflow (mcp-server v1.0.533, core v0.3.339)

Two fast-path/slow-path divergences in the aggregation planner's CountOnly optimization
(`collection_core/aggregate.rs`), found reviewing `aggregation/optimizer.rs` and its consumers.

1. **Empty input emitted a spurious group document.** For `[{$group: {_id: null, n: {$sum: 1}}}]`
   the planner takes a `count_documents()` fast path and unconditionally built
   `[{_id: null, n: 0}]`. But MongoDB — and IronBase's own streaming `$group` path
   (`group_stage::execute_streaming_with_context`, which iterates an empty `groups` map and
   returns `[]`) — produce **no** document for a `_id: null` group over zero input rows. So on an
   empty collection, or when a leading `$match` filters everything out, the fast path returned
   `[{_id: null, n: 0}]` where the non-optimized path returned `[]`. Fixed by skipping the output
   document when `include_id && count == 0` (the `$group` form); the `$count` stage keeps
   `include_id == false` and still emits `{field: 0}`, consistent with the streaming `$count`
   branch in `pipeline.rs`.

2. **Multiplier could overflow.** `result_count = (count as i64) * multiplier` used plain `*`,
   which panics in debug builds and silently wraps in release on overflow — e.g.
   `[{$group: {_id: null, t: {$sum: 9223372036854775807}}}]` over a multi-doc collection. The
   streaming accumulator path saturates (`saturating_add`/`saturating_mul` in
   `stages/accumulator.rs`); the fast path now uses `saturating_mul` to match.

New regression tests in `aggregation_accumulator_tests.rs`: `test_count_fastpath_empty_input_returns_no_doc`,
`test_count_fastpath_empty_input_with_trailing_project`, `test_count_fastpath_saturates_multiplier_overflow`.

### Fixed — concurrent upsert on the same filter created duplicate documents (audit P2-4) (mcp-server v1.0.532, core v0.3.338)

`update_one_with_options` implements upsert as `update_one()` (the match — which releases all
locks) **then** `insert_one()`, with no lock spanning the two. Two concurrent upserts on the same
filter (no matching doc, no application-level unique index) could therefore **both** observe
`matched == 0` and each insert → duplicate documents. Empirically reproduced: 8 threads upserting one
filter produced 2–8 documents instead of 1.

The audit's "just hold the collection write lock" turned out to be unsafe: every collection has a
unique `_id` index, so `collection_has_unique_index` is always true and `insert_one` **always** takes
the per-collection write lock — reusing it to wrap the upsert would reentrant-deadlock. The fix adds a
**dedicated per-collection upsert lock** (`collection_upsert_locks`, mirroring `collection_write_locks`)
held across the whole match→insert window. It is distinct from the write lock `insert_one` takes, so
there is no reentrancy; lock order is always upsert_lock → collection_write_lock (no ABBA). Applied to
both `update_one_with_options` variants (generic Safe path and the MemoryStorage path). With an
application unique index the constraint already prevented the duplicate; the lock now also covers the
no-unique-index case.

New `upsert_toctou_test`: concurrent same-filter guards on both the in-memory and Safe (file-backed)
paths assert exactly one document results (these **fail on the pre-fix code**, which is how the bug's
reachability was confirmed before fixing), plus a sequential insert-then-update sanity check.

### Fixed — delete was not atomic: reversed lock order + concurrent double-decrement (audit P2-3) (mcp-server v1.0.531, core v0.3.337)

Every delete write path de-indexed the document **before** acquiring the storage write lock
(`remove_from_indexes()` then `storage.write()`), and read the target lock-free. This reversed the
storage→indexes lock order the rest of the engine follows (#75) and, more seriously, opened a window
where two concurrent deletes of the same document could **both** pass the lock-free live read and each
write a tombstone + `adjust_live_count(-1)` → the live count silently drifted below the true value.
`update_one_prepare` already did this correctly (storage lock held across the index mutation); the
delete paths had diverged because the logic was copy-pasted across **seven** sites.

All delete paths now funnel through a single chokepoint, `CollectionCore::tombstone_doc_atomic`, which
acquires the storage write lock, **re-reads** the document under it (skipping a row already tombstoned by
a concurrent writer, and re-matching the query when given), writes the tombstone, then de-indexes — all
in one critical section, lock order storage→indexes. Callers fixed:

- **delete_one:** `delete_one_prepare` (Safe), `delete_one_raw` (in-memory / Unsafe),
  `delete_one_persist_batch` (Batch). As a side effect `delete_one_raw` now also resolves a string `_id`
  to its stored integer form (`resolve_stored_id`), matching what `delete_one_prepare` already did.
- **delete_many:** `delete_many_raw_with_docs` (in-memory / Unsafe) and the Safe bulk
  `delete_many_persist`. `DeleteManyPrepared` dropped its precomputed `tombstone_writes` + `index_removals`
  lists — the persist re-reads each doc under the lock instead, so there is nothing stale to write and the
  prepare/persist contract shrinks to `deleted` + `wal_entries`.

New `delete_one_atomicity_test` (13 tests) includes concurrent same-`_id` / same-filter guards for both
delete_one and delete_many (in-memory raw path and Safe prepare/persist path) that assert each doc is
deleted exactly once and the live count drops by exactly the matched count — these fail on the pre-fix
code. The `update_one_persist_batch` update path shares the index-before-storage ordering and is out of
this delete-focused fix's scope.
### Fixed — a torn WAL tail (crash mid-append) bricked database startup (mcp-server v1.0.530, core v0.3.336)

`WALEntryIterator::read_next` treated a short read on the entry **header** as a
clean end-of-log (`Ok(None)`) but used a bare `?` on the **data** and **checksum**
reads. A crash while appending a WAL record leaves a torn trailing entry (header
written, data/checksum partial) — the bare `?` surfaced that as
`Err(UnexpectedEof)`, which `recover()` propagated, so **`recover_from_wal`
aborted and the database failed to reopen** after an ordinary crash, even though
the torn entry was never committed.

A short read on the data/checksum is now handled like a short header: the partial
trailing entry (and anything after it) is discarded as a clean end-of-log, which
is the standard WAL recovery semantics — the records before it were fully written
and fsync'd, the torn one never committed. A genuine checksum **mismatch** on a
complete-length entry still returns `WALCorruption` (loud failure, unchanged), so
real corruption is not silently swallowed. New `test_iterator_discards_torn_trailing_entry`
covers truncation mid-data and mid-checksum across the prior complete entries.
### Fixed — multi-instance file lock could be bypassed → two live writers / corruption (mcp-server v1.0.529, core v0.3.335)

`StorageEngine::open` guards single-writer access with an `fs2` exclusive advisory
lock on a sibling `<db>.lock` file. The previous PID-based "stale lock" recovery
made that guarantee bypassable (audit 2026-06-08 P0-1, empirically reproduced):
on a failed `try_lock_exclusive` it read the PID written in the lock file and, if
that PID looked dead, **unlinked and re-created** the lock file before re-locking.
But `try_lock_exclusive` fails only when a **live** process holds the flock, while
the PID could lag (a holder mid-acquire had not rewritten it) or read as dead
across users (`kill(pid, 0)` → `EPERM` for another uid). Because the flock is
per-inode, concurrent recoverers each unlinked the live holder's inode and locked
a **fresh** one — so two openers could both succeed on the same `.mlite` → two
live writers → corruption.

The stale-detection (and `is_process_alive`, with its `kill`/`tasklist` probes) is
removed entirely: the OS already releases the advisory lock when the holding
process dies (crash / SIGKILL) or closes the fd, so a leftover `.lock` file is
harmless and the next `try_lock_exclusive` simply succeeds. A genuine OS/FS lock
fault (`ENOLCK`/`EIO`) is now surfaced instead of masked as `DatabaseLocked`. The
lock path is derived by **appending** `.lock` (`format!("{path}.lock")`) rather
than `with_extension`, so distinct databases like `foo` and `foo.mlite` no longer
collide on one `foo.mlite.lock`. Regression tests cover the bypass, crash-recovery
reopen, and the path collision. (Local-filesystem guarantee; advisory locks are
host-local on NFS, as documented.)

### Refactored — unify the four QueryPlan executors behind one range chokepoint (mcp-server v1.0.528, core v0.3.334)

Audit finding #8: a single `QueryPlan` was interpreted by **four** hand-synced executors — the two
`count_with_plan` blocks (`collection_core/count.rs`), `collect_doc_ids_from_plan`
(`collection_core/mod.rs`), and `FindCursor::new_index_scan_from_plan` (`collection_core/cursor.rs`).
Every plan→index-range fix had to be mirrored 2–4×, and the numeric Int/Float two-bucket fix once
missed the cursor, shipping a bug where `find_streaming` (and aggregation `$match`) silently dropped
Float-keyed docs.

The plan→index key-range derivation now lives **once** in `ironbase-core/src/index/plan_ranges.rs`
(`PlanRanges::from_plan` + the `BPlusTree::count_exact` / `scan_ranges` consumers), read by three thin
modes: count-exact (O(1), no post-filter), materialized find, and the streaming cursor (single
contiguous range only). The four call sites collapse to adapters; `count_numeric_buckets` /
`scan_numeric_buckets` / `max_string_key` are subsumed and removed. Net ~−600 lines of duplicated
derivation. Behavior-preserving, with a new 3-way `count == find == cursor == scan` parity oracle
(file-backed streaming) that guards the cursor path the suite never exercised before. The
unbounded-end non-numeric range now uses `IndexKey::MaxKey` consistently across all three executors
(was `max_string_key()` in count/find vs `MaxKey` in the cursor).

### Fixed — two pre-existing count/find divergences surfaced by the #8 review (mcp-server v1.0.528, core v0.3.334)

- **Single-sided non-numeric range over-counted on `count`.** A typed range with a defaulted open
  end that crosses key-type buckets (e.g. `{f: {$lt: "z"}}` → `[Null, "z"]`) was treated as exact,
  so `count_documents` summed the Int/Float/Bool keys the operator can never match (`count = 6` vs
  `find = 1` on mixed-type data). `PlanRanges::from_plan` now marks such a range non-exact via a
  type-homogeneity check (`QueryPlanner::index_key_type_bucket`), routing `count` through the
  index-narrowed post-filter to match `find`.
- **Numeric range `find(...).sort({field})` returned DocumentId order.** A numeric Int/Float
  two-bucket scan yields DocumentId-sorted ids, but `collect_doc_ids_from_plan` reported
  `uses_index_sort = true`, skipping the in-memory sort. Index-sort is now claimed only for a single
  contiguous (index-ordered) scan, so multi-bucket numeric ranges (and multi-regex unions) re-sort
  correctly.

### Fixed — query-planner correctness: 11 count/find divergence & incomplete-result bugs (mcp-server v1.0.527, core v0.3.333)

A multi-agent review of the query planner (`ironbase-core/src/query_planner.rs` and its
consumers `count.rs` / `collection_core/mod.rs` / `cursor.rs` / `index/btree.rs` / `index/key.rs`)
found and fixed 11 verified bugs. The umbrella cause for most: `count_documents`'s
fully-covered fast path trusts the index plan **without re-verifying** against the query, while
`find` always post-filters — so the two silently diverged.

- **`$in` with an object/array element** collapsed to `IndexKey::Null` (the audit #27-A
  `is_indexable_value` guard existed only for equality). `collect_in_candidates` now skips the
  candidate so the collection scan does proper deep equality.
- **`$in` with duplicate values** double-counted on the fast path → planner now dedups keys.
- **`$and` of two same-field membership ops** (`{$in}∧{$in}`, `{$ne}∧{$ne}`) silently dropped
  the first via a serde-map overwrite → `resolve_range_conditions` bails out on operator collision.
- **`query_fully_covered_by_plan`** only checked the field *name*; an operator the plan does not
  encode (e.g. `{$gte,$ne}`, `{$in,$gte}`, `{$gte,$type}`) was dropped → it now requires the plan
  to cover **every** operator on the field, else routes through the post-filter path.
- **Case-insensitive index** was eligible for case-sensitive equality/range/regex selection (its
  lowercased keys miss uppercase values) → CI indexes are now excluded from CS index selection
  (usable only in the `(?i)` regex branch).
- **Regex prefix upper bound** was `prefix + U+10FFFF` *inclusive*, dropping values of the form
  `prefix + U+10FFFF + …` → now a true half-open `[prefix, successor)` bound.
- **Non-exact / case-insensitive regex count** used the raw range count (no pattern re-check) →
  now routed through the index-narrowed post-filter (also makes `explain` honest and uses the
  index for `^a.*` counts instead of a full scan).
- **Numeric range over a field mixing integers and floats**: `IndexKey` orders all `Int` below
  all `Float`, so `[Int(18), Int(65)]` never reached `Float` keys — `find`/`count` silently
  dropped fractional/`x.0`-float docs. Range scans now cover **both** the Int and Float buckets
  (`numeric_range_buckets`), in find, count, and the streaming cursor.
- **Mixed-type range** (`{$gte: 5, $lte: "z"}`) over-counted the fast path → `analyze_range_query_v2`
  now rejects type-mismatched bounds (mirrors the merge-path guard).
- **Multikey (array) index** counted index *entries*, not distinct documents, on the fast path →
  multikey plans now use the index-narrowed path, which dedups doc_ids up front.

Internal-only: all new helpers are `pub(crate)`; the public crate API is unchanged. Known
limitation (documented, not fixed): a case-insensitive index keys with `str::to_lowercase` while
`(?i)` regex matches with Unicode case folding — they diverge for a few non-ASCII scalars
(Greek final sigma, `İ`, `ß`); no impact on ASCII / Hungarian-accented data.

### Fixed — `find` `_id` fast path now applies `projection` (was silently ignored) (mcp-server v1.0.526, core v0.3.332)

A code-review of the core fast-path system found that `find_with_options`'s `_id` and `_id $in`
fast paths (taken when `sort` is absent) returned the **full document**, never applying
`options.projection` — the projection was only *validated* at the function top, then dropped. The
slow path (taken when a sort is present) applies it. So the same `{"_id": 5}` query with a
projection returned different shapes depending on whether a sort was present, and a caller
projecting out a large/sensitive field would still receive it on the fast path.

- **Fix:** both `_id` fast-path branches now apply the existing `find_options::apply_projection`
  (a no-op when no projection is set) before returning — identical to the slow path. The common
  projection-less `_id` lookup stays O(1) with zero overhead. (`collection_core/mod.rs`)
- **Test:** new `test_id_fast_path_applies_projection` asserts the fast path projects and that the
  fast-path and slow-path (`+sort`) results are byte-identical — the path that was previously
  uncovered by any test.
- **Doc cleanup:** removed a stale `search_and_with_ctx` comment that still referenced the private
  top-k heap deleted in the v1.0.525 delegation. (`fulltext.rs`)

### Changed — fulltext `top_k_scored` now delegates to the shared generic top-k instead of a private heap (mcp-server v1.0.525, core v0.3.331)

P1-4 code-review follow-up. The previous commit added a fulltext-private `BinaryHeap<(Reverse<OrderedScore>, DocumentId)>` for `top_k_scored`, but the codebase already has a generic comparator-based bounded top-k (`collection_core::topk::topk_select_with_skip`) that the sibling `collection_core/search.rs` already uses for score-based `(doc_id, f64)` selection. This commit removes the duplicate, finishing the consolidation the P1-4 change set out to do.

- **Delegate to the generic helper.** `FulltextIndex::top_k_scored` now calls `topk_select_with_skip(scored.filter(|s| s >= min), skip, limit, cmp)` with a `(score desc, doc_id asc)` comparator. `mod topk` is now `pub(crate)` so `fulltext.rs` can reach it. One bounded-top-k implementation fewer to keep in sync. (`fulltext.rs`, `collection_core/mod.rs`)
- **Dead code removed.** The private `OrderedScore` wrapper and the now-unused `BinaryHeap` import are gone (the comparator closure subsumes them).
- **NaN handling restored to the OR-path's original semantics.** The pre-filter `filter(|(_, s)| *s >= min)` drops NaN scores (`NaN >= min` is false), matching the OR paths' behaviour before P1-4 (the interim private heap had kept NaN and sorted it last). Unreachable in practice — BM25 `term_score` cannot produce NaN — but it removes a latent semantic drift the review flagged.

### Changed — fulltext search top-k unified into one bounded helper; the OR path no longer full-sorts every match (mcp-server v1.0.524, core v0.3.330)

Scalability audit item **P1-4** (100GB+ roadmap). The audit's billing was largely already
addressed: the smallest-first AND-merge it called for already existed in `search_and_with_ctx`
(rarest-token-first intersection, scoring only the intersection candidates, plus a Phase-4 top-k
heap), and the "candidate cap on `doc_scores`" is not applicable — the BM25 score of a doc is the
sum of its per-token contributions, so every matching doc must be scored (the `doc_scores` /
`matched` maps are O(N) by nature, as `calculate_candidate_limit`'s own comment notes). The real
remaining gap was the **OR path** (`search` / `search_with_ctx`, which the `fulltext_search` tool
uses), which still materialized every scored match into a `Vec` and `sort_unstable_by` + truncated.

- **One bounded top-k helper.** New `FulltextIndex::top_k_scored` selects the best `skip + limit`
  scored doc_ids via a lazy `BinaryHeap` (no eager `with_capacity(skip+limit)`, mirroring the
  `find` top-k fix) in O(N log k) time / O(k) memory, ranked score-descending then doc_id-ascending
  (NaN scores last). All four search paths — `search`, `search_with_ctx`,
  `search_for_doc_ids_with_ctx`, and the AND `search_and_with_ctx` Phase-4 — now use it, so the
  full-result `Vec` + O(N log N) sort is gone from the OR paths. (`fulltext.rs`)
- **Unified tie ordering.** The AND path's old `(OrderedScore, DocumentId)`-reverse heap ordered
  ties doc_id-*descending*, diverging from the OR path's doc_id-*ascending*. All paths now share
  `top_k_scored`'s doc_id-ascending tie-break — one deterministic ordering across fulltext search.
- **Dead code removed.** `compare_search_results` is gone; the heap key reproduces its ordering, so
  there is one top-k implementation instead of three. The `doc_scores` / `matched` maps remain
  O(N) — that is fundamental to BM25, not debt.

### Fixed — `find` sort+limit uses an O(k) top-k heap, no longer materializes every matched doc before truncating (mcp-server v1.0.523, core v0.3.329)

Scalability audit item **P1-5** (100GB+ roadmap). When a `find` has a sort that no single-field
index can satisfy (a multi-field sort, or a single-field sort with no index on that field),
`QueryExecutionContext` defers the limit (`fetch_limit = None`) and the slow path loaded **every**
matched document into a `Vec` — `try_reserve(doc_count)` and all — before `apply_sort` sorted the
whole thing and pagination truncated it to the limit. On a 50M-row unindexed `sort + limit 10`
that is an O(n) allocation and full sort to return 10 rows. Root cause: there was no top-k path in
`find` (only aggregation had one).

- **Stream the scan into a bounded heap.** Both find paths — `find_with_options` (the collection
  slow path) and `find_with_hint_ext` (the index-hint path) — now detect "in-memory sort required
  AND positive limit" and push documents straight into an O(k) `TopKHeap` (k = skip + limit)
  instead of the Vec. The heap keeps only the k smallest-by-sort docs seen so far, so memory is
  O(k) regardless of how many match. The `try_reserve(doc_count)` O(n) pre-allocation is skipped
  on this path. The heap allocates lazily (not `with_capacity(skip+limit)`), so deep pagination
  over few matches stays O(retained) and an extreme `skip` can't trigger a capacity-overflow panic.
  When an index already sorted+paginated for us, the cheap pre-paginated Vec path is unchanged.
  (`collection_core/mod.rs`, `collection_core/index_ops.rs`)
- **Reused, not reinvented.** The existing `topk_documents_streaming` heap logic
  (`query_executor.rs`) was extracted into a reusable `TopKHeap { new, push, into_sorted }` so the
  fallible document-loading loop (which returns `Result`) can drive it directly; the streaming
  function now wraps the same struct. (`query_executor.rs`)
- **Comparator unified to prevent a sort regression.** The heap ranked mixed-type fields with an
  `Ordering::Equal` fallback while `apply_sort` used a stable `type_priority` fallback — so on a
  mixed-type sort field the heap could evict the *wrong* documents and the top-k set would differ
  from the full sort. `compare_docs_by_sort` now uses the identical `type_priority` fallback
  (shared via `value_utils::type_priority`); top-k results are now byte-for-byte equal to the
  full-sort prefix on any field, mixed types included. (`query_executor.rs`, `value_utils.rs`,
  `find_options.rs`)
- **Implicit RAM-derived cap for an uncapped full sort.** A required in-memory sort with **no
  limit and no explicit `max_response_bytes`** would still materialize everything. Such a query
  now applies an implicit RAM-derived response cap (the `with_safe_defaults` basis,
  `calculate_safe_response_limit`), so the existing per-doc size guard turns a would-be OOM into a
  typed, actionable error instead. On the top-k path the response-size guard applies to the
  bounded ≤limit result rather than the whole scan. (`collection_core/mod.rs`,
  `collection_core/index_ops.rs`)
- **Stable top-k on tied keys.** The heap carries a scan-sequence tiebreaker, so on tied sort
  keys it retains and orders the same documents as the old stable full sort + truncate (first-k by
  scan order) — byte-for-byte. Without it the `BinaryHeap` could evict an arbitrary tied document
  and return a different, run-to-run nondeterministic page. (`query_executor.rs`)
- **Sort-direction validation is path-independent.** `validate_sort_directions` now runs at every
  find entry point, so an invalid direction (e.g. `2`) is rejected whether or not a limit is
  present. Previously only the full-sort path called `apply_sort`'s validator, so the top-k and
  index-sorted paths silently accepted (and mis-sorted) invalid directions. (`find_options.rs`,
  `collection_core/mod.rs`, `collection_core/index_ops.rs`)

### Fixed — vector index ceiling is RAM-derived, no longer a fixed 100K cap that could silently lose data (mcp-server v1.0.522, core v0.3.328)

Scalability audit item **P0-2** (100GB+ roadmap). The hard `DEFAULT_MAX_VECTORS = 100_000`
ceiling is a latent data-loss flaw at scale: once a vector index left at the default cap grows
past 100K vectors, the HNSW insert returns `OutOfMemory`, every auto-index caller logs-and-drops
it while the document is already persisted, so those documents silently never appear in vector
search. (A 2026-06-05 audit of the production `docs.mlite` found **no current loss** — its main
`docs` index had been given an explicit `262144` cap and is fully indexed at 191207/262144 — but
any index left at the default 100K sentinel hits this the moment it crosses 100K.) Root cause:
the fixed cap, fixed here by making the **default** ceiling RAM-derived (an explicit non-default
`max_vectors` like `docs`'s `262144` is still honoured verbatim).

- **RAM-derived ceiling.** `DEFAULT_MAX_VECTORS` now doubles as an "auto" sentinel: an index
  whose `max_vectors` is the default resolves at runtime to a memory-budgeted limit
  (`calculate_max_vectors` / `max_vectors_for_budget`, 50% of available RAM ÷ per-vector cost),
  mirroring `index::traits::calculate_lazy_threshold()`. The limit is **never persisted** — it
  lives in a `#[serde(skip)] HnswIndex::effective_max_vectors: Option<usize>` resolved lazily on
  first insert (`effective_ceiling()`), so existing on-disk indexes (and DBs moved between
  machines) re-derive the ceiling for the current box with **no format change**. The value only
  ever *raises* the legacy 100K floor, never lowers it, and is additionally floored at the
  index's own active population so a large index reopened on a memory-constrained box never
  rejects or `rebuild()`-truncates vectors it already holds. (`vector/config.rs`, `vector/hnsw.rs`)
- **Rejection stays swallowed (logged), deliberately NOT propagated.** A code review showed that
  propagating the `OutOfMemory` out of the auto-index callers corrupts atomicity: the durable
  (`insert_one_persist`) and batch (`persist_buffered_operations`) paths write storage and commit
  the WAL **before** the index step, so a propagated error leaves a persisted "ghost" document
  the caller is told failed — or aborts an entire batch transaction, discarding unrelated ops.
  The three auto-index sites therefore keep `log_error`/`warn`-and-continue, uniform with the
  fulltext swallow and the lazy HNSW rebuild path. With the RAM-derived ceiling a genuine hit is
  reachable only at true RAM exhaustion; there the document is still stored and findable, and the
  drop is logged (a visible degradation at a real resource limit, not a hidden dummy fallback).
- **Hardening:** the per-node `try_reserve` growth increment no longer clamps to the ceiling
  (orphan-heavy indexes could clamp it to 0, leaving the subsequent `push` to reallocate
  unguarded and abort the process); `effective_max_vectors` is an `Option` (not a `0` sentinel)
  so an explicit `max_vectors == 0` is not mistaken for "unresolved"; and the per-vector cost is
  a single shared `per_vector_bytes()` reused by `estimate_memory_bytes`.
- Regression-guarded by `test_vector_overflow_is_swallowed_not_propagated` (atomicity),
  `default_config_resolves_ram_derived_ceiling_lazily` + `explicit_small_cap_rejects_overflow`
  (limit resolution), and `max_vectors_for_budget` / `resolve_max_vectors` arithmetic tests;
  HNSW recall gates and the full ironbase-core suite stay green.

### Performance — compaction snapshot no longer deep-clones the catalog under the write lock (mcp-server v1.0.521, core v0.3.327)

Scalability audit item **P0-1** (100GB+ roadmap). `StorageEngine.collections` is now held
behind an `Arc<HashMap<…>>` so the catalog can be snapshotted copy-on-write.

- **`compact_prepare` (Phase A) took an O(N) deep clone of the entire collection catalog
  under the global `storage.write()` lock.** On a large database the `document_catalog`
  (`HashMap<DocumentId,u64>`) + `document_order` of every collection is tens of millions of
  entries (multi-GB); cloning it while holding the write lock stalled **all** writes for
  seconds at the start of every compaction. Now Phase A takes an O(1) `Arc::clone` snapshot and
  releases the lock immediately. The frozen snapshot drives the lock-free Phase-B scan and the
  Phase-C catch-up diff exactly as before. (`storage/compaction.rs`, `database/maintenance.rs`)
- **Copy-on-write semantics.** All catalog mutations go through a new private
  `StorageEngine::collections_mut()` (`Arc::make_mut`). While a compaction snapshot is alive,
  the *first* concurrent write deep-clones the catalog once; every other write is O(1). An
  idle or read-only compaction pays **zero** clones. Worst case is therefore one deferred clone
  borne by a single writer, versus the previous unconditional clone under the lock. Reads are
  unchanged (via `Deref`); the public API (`collections_ref`, `get_collection_meta_mut`, …) is
  untouched. (`storage/mod.rs`, `storage/metadata.rs`)
- Regression-guarded by `compact_prepare_snapshot_is_cow_not_deep_clone` (asserts `Arc::ptr_eq`
  before the first mutation and divergence after) plus the full storage + compaction suites.

### Fixed — checkpoint/compact durability regressions from PR #89 (mcp-server v1.0.520, core v0.3.326)

Three correctness regressions introduced by the scalability batch-1 split (PR #89, v1.0.519),
fixed before deploy. All in the storage durability path (`ironbase-core`).

- **Data loss (HIGH) — concurrent write lost across `checkpoint_wal_only`.** The two-phase
  checkpoint serializes metadata under `storage.read()` (Phase A), releases it, then clears the
  WAL under `storage.write()` (Phase B). When Phase A saw clean metadata it returned `None`; the
  P1-3 split then made the Phase-B `None` arm clear the WAL unconditionally. An insert committing
  in the gap between the two phases dirties the in-memory catalog and writes its only durable
  record to the WAL — clearing it without flushing stranded that committed document on a crash
  before the next checkpoint. Fix: the `None` arm now re-checks `is_metadata_dirty()` under the
  Phase-B write lock and runs a full `checkpoint()` (flush-then-clear) when dirty, restoring the
  pre-PR-89 `_ => checkpoint()` safety. (`database/maintenance.rs`)
- **`checkpoint_wal_clear_only` did not reset `metadata_snapshot_pending`.** Clearing the WAL
  discards the metadata snapshot it held, so the next `ensure_metadata_snapshot()` must write a
  fresh one; leaving the flag set made it early-return, yielding an empty WAL with no recovery
  base. Now reset after `wal.clear()`, matching every sibling WAL-clearer. (`storage/mod.rs`)
- **`last_compact_size` reached the Header only at `close()`.** The Drop/crash path persisted the
  tx_id watermark but not the compaction-size baseline, so a process that compacted then dropped
  (or crashed) without an explicit `close()` lost it → `bloat_ratio = +inf` → the auto-compact
  rewrote the whole file on the next start (P1-7). Now persisted into the Header at compact time
  (under the storage write lock, in both the blocking and non-blocking compact paths), which marks
  metadata dirty so Drop's `flush()` and the next checkpoint both write it. (`database/maintenance.rs`)

### Fixed — `search` context budget collapsed broad queries (mcp-server v1.0.518)

The `search` tool's response contract (`retrieval/contract.rs`) spent its 12 000-char
context budget **depth-first**: it emitted every passage of the first document before the
next, so one large document (e.g. 10 × ~1000-char passages) consumed ~70 % of the budget and
all remaining matched documents were dropped with empty passages. A broad query (`fékerőmérő`:
602 matching documents, 194 from the shared engine) returned only **4**, and `limit` was
effectively ignored. The fix allocates the budget **breadth-first** (Pass 1: one passage per
document up to `limit`; Pass 2: fill the remainder with further passages), so `count` reflects
matched documents up to `limit`, each with ≥1 representative passage. Budget-dropped documents
are surfaced via a new `dropped_due_to_budget` field (P6). Pure response-shaping change — the
shared `retrieve_and_fuse`/`build_doc_groups` engine and `db_hybrid_search` are unchanged.

### BREAKING — MCP tool naming canonicalization + index consolidation (mcp-server v1.0.509)

Single canonical naming convention (`<resource>_<verb>`, noun-first, full words) plus
consolidation of the per-subtype index tools. **No aliases** — old names are gone.
Tool count: 93 → 87.

**Renamed (14):**

| Old | New |
|-----|-----|
| `count_documents` | `count` |
| `begin_transaction` | `transaction_begin` |
| `commit_transaction` | `transaction_commit` |
| `rollback_transaction` | `transaction_rollback` |
| `insert_one_tx` | `transaction_insert_one` |
| `update_one_tx` | `transaction_update_one` |
| `delete_one_tx` | `transaction_delete_one` |
| `admin_list_all_collections` | `admin_collection_list` |
| `admin_create_system_collection` | `admin_collection_create_system` |
| `admin_set_collection_flags` | `admin_collection_set_flags` |
| `admin_drop_protected` | `admin_collection_drop_protected` |
| `rag_load_all_chunks` | `rag_chunks_load` |
| `embed_list_models` | `embed_models_list` |
| `listener_add` | `listener_create` |

**Index consolidation — 6 tools removed**, folded into generic tools via a `type`
parameter (`btree` default | `fulltext` | `fuzzy` | `vector`):

- Removed: `index_create_fulltext`, `index_create_fuzzy`, `index_create_vector`,
  `index_list_fulltext`, `index_list_vector`, `index_drop_vector`.
- `index_create` gains `type` + per-type fields (server-side per-type validation;
  no top-level `oneOf`, per Anthropic schema constraint).
- `index_list` gains an optional `type` filter, and **now includes fuzzy indexes**
  when listing all subtypes (previously a silent omission).
- `index_drop` already handled all subtypes; vector drops are routed to the dedicated
  cleanup (removes the on-disk HNSW cache file + `vector_indexes` metadata that the
  generic path left stale).
- `index_stats` gains `type` (fulltext → num_documents/num_tokens, fuzzy → num_entries,
  vector → vector_count).
- `vector_search` is unchanged.

New core/adapter support: `CollectionCore::list_fuzzy_indexes()` (ironbase-core) +
`IronBaseAdapter::list_fuzzy_indexes()` (mcp-server). Rhai `db_*` scripting functions
are a separate namespace and are unchanged.

### Changed — internal cleanup from code review (mcp-server v1.0.507; no API change)

Altitude/cleanup follow-ups to the v1.0.505–506 work; no user-visible behavior change.

- **ACL reload is policy-table-driven**: the post-call live `AclConfig` reload no
  longer keys off a hardcoded `matches!(name, "acl_set"|...)` name-list in
  `engine.rs` (the scattered-list anti-pattern the `tool_policy()` table removed).
  `ToolPolicy` gains a `mutates_acl` axis; `engine.rs` reloads when
  `tool_policy(name).mutates_acl`. A future ACL-mutating tool gets the reload for
  free from its policy entry. Parity test: `test_tool_policy_mutates_acl_axis`.
- **Documented the deliberate admin-key enforcement split**: `admin_key` is gated
  in `dispatch_tool` (not `engine.rs`) because the stdio transport bypasses
  `execute_tool`; the `ToolPolicy::admin_key` doc now states this so it is not
  "consolidated" into `check_acl` (which would drop the gate on stdio).
- **Rhai `db_hybrid_search` params**: replaced the ~15 hand-copied option inserts
  with `map_to_json(&opts)` bulk-passthrough + injection of the Rhai-specific
  defaults; the Rhai surface now tracks new `HybridSearchParams` fields
  automatically instead of drifting.
- **Grouped hybrid search skips the wasted adjacent-chunk merge**: in
  `group_by_document` mode `build_doc_groups` re-fetches chunks via its own Phase-2
  search, so `retrieve_and_fuse` no longer runs `merge_adjacent_chunks` there
  (doc ordering is unaffected — merge preserves each doc's max score).
- **De-duplicated integration-test plumbing**: `create_test_adapter`/`dispatch_ok`/
  `dispatch_err` moved to `tests/common/mod.rs` (shared by `mcp_tests.rs` and
  `retrieval_eval.rs`).

### Fixed (mcp-server v1.0.506) — multi-field non-RAG document qualification

- **Multi-field document-scope queries returned 0 results on non-RAG collections.**
  The v1.0.505 non-RAG fix (`qualify_documents` keying the filter on `_id` instead
  of a `doc_id` the documents don't carry) covered only the single-field path. The
  multi-field union in `apply_document_qualification` (and `qualified_doc_count`)
  still hardcoded `doc_id`, so a non-RAG collection with 2+ fulltext-indexed fields
  + a multi-word `match_scope="document"` query qualified an empty set → 0 hits.
  The union is now key-agnostic (honors `_id` or `doc_id`) and preserves native id
  Values. Regression test: `hybrid::pipeline_integration_tests::multi_field_non_rag_document_scope_qualifies`.

### Removed / BREAKING (mcp-server v1.0.505) — deprecated tools + dead code cleanup

- **`hybrid_search` MCP tool removed** (superseded by `search`; tool count 94 → 92).
  This supersedes the v1.0.504 note below that `hybrid_search` is "still available".
  The shared RRF pipeline (`hybrid::retrieve_and_fuse` + `build_doc_groups`) is
  retained — `search` and the Rhai `db_hybrid_search` use it. The tool's #68/#71/#72
  regression tests are ported in-crate (`hybrid::pipeline_integration_tests`).
- **`vector_search_filter` MCP tool removed.** `adapter.vector_search_with_filter`
  is retained (used by the `search`/hybrid pipeline and Rhai `db_vector_search_filter`).
- **Rhai `db_hybrid_search` consolidated**: was a ~380-line parallel reimplementation
  of the fusion pipeline (and its own provider resolution); rewritten as thin glue
  delegating to the shared `retrieve_and_fuse` + `build_doc_groups`. One fusion path now.
- **Deprecated Rust items removed**: `commit_transaction_with_indexes`,
  `adapter.find_with_hint`, and the legacy `GroupStage::execute` /
  `Accumulator::update` / `update_with_limits` cluster (tests migrated to
  `execute_streaming`).

### Fixed (mcp-server v1.0.505)

- **ACL changes now take effect immediately**: `acl_set`/`acl_delete`/`acl_cleanup`
  reload the live in-memory `AclConfig` in `execute_tool` (previously the persisted
  rule only applied after a server restart — verified on production).
- **`db_stats`/`db_compact`/`db_checkpoint` fail-open closed**: these were marked
  Admin but had no system-collection scope and no localhost gate, so the ACL check
  was skipped entirely (`db_stats` leaked the full collection catalog to an Internal
  caller). They are now loopback-only, consistent with `db_open`.
- **`fulltext_search match_scope="document"` returned 0 on non-RAG collections**:
  `qualify_documents` now intersects posting lists on the native `_id` for
  collections without a parent `doc_id` field.

### Changed (mcp-server v1.0.505)

- **ACL authorization is table-driven**: a single declarative `tool_policy()` table
  is the source of truth for all four axes (permission, system collection, localhost,
  admin key); `get_required_permission` / `get_system_collection_for_tool` /
  `requires_localhost` are thin projections and the admin-key gate is centralized in
  `dispatch_tool`.

### Tooling — retrieval evaluation harness (redesign §12; no runtime change)

- **`mcp-server/tests/retrieval_eval.rs`** — the measurement foundation that gates
  every staged retrieval change (the "no heuristics → must measure" enforcement,
  P3). Provides correct, **hand-computed-unit-tested** metric implementations
  (nDCG@k, Recall@k, abstention precision/recall), a `LabeledQuery` data format,
  and a quality gate on a labeled set. (In v1.0.505 the `hybrid_search` baseline was
  dropped with that tool; the gate became a self-contained absolute nDCG/Recall floor.)
  Real per-corpus labels (rdocs + the long manual) are a separate data step; the
  synthetic seed proves the harness and guards regressions.
- **Design-doc ordering correction (`docs/HYBRID_RETRIEVAL_REDESIGN.md` v3.1)**:
  Stage B ("`max` fusion") was wrongly listed as implementable right after Stage A.
  `max(P_v, P_t)` consumes **calibrated** probabilities (Stage C); on raw cosine +
  raw BM25 it is meaningless. RRF (rank-based, scale-free) stays as the fusion
  through Stage A and until calibration ships. Stage B is reordered to depend on
  Stage C, with the eval harness as the shared gate.

### Added (mcp-server v1.0.504) — `search` tool: intent-shaped hybrid retrieval (redesign Stage A)

First stage of the hybrid-retrieval redesign (`docs/HYBRID_RETRIEVAL_REDESIGN.md`,
`docs/STAGE_A_IMPLEMENTATION_PLAN.md`). At the time, additive and low-risk —
`hybrid_search` was unchanged and still available. **(Superseded in v1.0.505:
`hybrid_search` is removed; `search` is the sole retrieval tool.)** The `search`
tool is an intent-shaped façade built over the *existing* RRF fusion — no new
retrieval primitive.

- **New MCP tool `search(collection, query, filter?, limit?, format?, debug?)`** —
  an intent-only surface (~4 effective params vs `hybrid_search`'s 23). Retrieval
  mechanism (weights, `rrf_k`, `match_scope`, merge, caps, …) is server-owned;
  passing such a param is **rejected with a pointer to the config**, never silently
  ignored (P6).
- **Document-anchored compact contract** (small-model optimized): ranked source
  documents, each with overlap-merged **passages** carrying *only* text — **no
  `embedding` arrays, no `_`-prefixed engine metadata, no chunk-tracking fields**.
  The headline token-economy win for a small local consumer (qwen3.5:9b); a
  regression test asserts the `search` payload is smaller than the equivalent
  `hybrid_search` response.
- **`format: "context_block"`** returns a citation-marked text block ready to paste
  into a prompt; `"structured"` (default) returns documents+passages.
- **Token budget**: trimmed to a configured character budget (least-relevant
  passages dropped) with a surfaced `trimmed` flag.
- **Honest verdict**: Stage A makes no absolute-relevance/abstention claims →
  `verdict` is always `"unknown"` (four-state contract, redesign §9; calibration is
  the gated Stage C–E track).
- **Graceful degradation (P7)**: with no embedding provider, `search` falls back to
  BM25-only and **surfaces it** (`degraded` field), never silently.
- Internals: the shared retrieve+fuse pipeline (STEP 1–6) is extracted into
  `hybrid::retrieve_and_fuse` and the two-phase grouped builder into
  `hybrid::build_doc_groups` (both `pub(crate)`, behavior-preserving). `search`
  calls these **directly** — no JSON round-trip into `hybrid_search` and no
  re-parse of its response shape; `contract::assemble` consumes `DocGroup`s.
  New `tools/retrieval/` module (`mod.rs` façade + `contract.rs`), schema in
  `definitions/retrieval.rs`.
- Code-review hardening (folded in): hard token budget that always keeps ≥1
  passage and surfaces over-budget via `trimmed`; multi-chunk doc-field assembly
  excludes the resolved text field (no body duplication for non-`content`
  fields); mechanism-param rejection targets a **known** key set (benign/protocol
  fields tolerated like every other tool) rather than deny-all-unknown;
  non-object arguments get a clear error; a surfaced `dropped_empty_documents`
  count when a qualified doc carries no body text under the resolved field
  (no silent shrink, P6).
- Tests: 6 new integration tests (compact contract / no-internals, mechanism-param
  rejection, benign-unknown-key tolerance, `context_block`, doc-order equivalence
  vs `hybrid_search` grouped, payload-size win). 307 lib + 44 integration + 5
  schema green.

### Added (mcp-server v1.0.503) — `rag_load_all_chunks` tool (#73)

- **New MCP tool `rag_load_all_chunks`** loads every chunk for a given list of `doc_ids` from a RAG collection. Two modes:
  - **Pure load** (`query` omitted): results sorted by `(doc_id, chunk_index)` ASC, no scoring fields on hits. Intended use: UI doc-detail-view, where the client just wants the whole document with overlap-merge applied.
  - **Scored load** (`query` provided): chunks scored via fulltext OR-search filtered to the requested `doc_ids`, sorted by `_score` DESC, every hit carries `_score`. Intended use: RAG context-expansion after `hybrid_search` — the top-K doc_ids come from hybrid, and this tool returns ALL the relevant chunks per doc with `_text_score` so the client can pick its own context budget.
- **Adjacent-chunk merge by default** (`merge_chunks=true`): same algorithm and code path as `hybrid_search` (`fusion::merge_adjacent_chunks`) — overlap removal, table-header dedup (#63), `chunk_merged`/`chunks_in_merge` metadata.
- **`max_chunks_per_doc` parameter** — cap per source document, applied AFTER merge so a merged run counts as one chunk.
- **Empty `doc_ids` returns empty results, not an error** (issue #73 AC4); missing doc_ids in the collection are silently skipped (AC5).
- **Rhai equivalent `db_rag_load_all_chunks(collection, doc_ids)` / `db_rag_load_all_chunks(collection, doc_ids, options)`** — feature parity with the MCP tool, delegating to `tools::rag::dispatch` so the merge/cap/sorting logic stays single-source.

### Hardening (mcp-server v1.0.503) — `hybrid_search` flat-mode `max_chunks_per_doc` (#72)

- **Flat mode now honors `max_chunks_per_doc`** — caps how many chunks from the same `doc_id` survive into the global top-K. The cap is applied AFTER reranking and BEFORE `merge_chunks` / MMR, so an adjacent-merge run counts as a single result (issue #72 AC3): merging an N-chunk overlap-cluster does not silently bypass the cap. Combined with the grouped-mode cap introduced in v1.0.502, one parameter name now means the same thing in both modes — clients no longer need to over-fetch with `limit=500, merge_chunks=False` to ensure cross-doc coverage; `limit=50, max_chunks_per_doc=5` guarantees at least 10 distinct docs in the top-K.

### Hardening (mcp-server v1.0.502) — `hybrid_search group_by_document=true` chunk-shape (#71)

- **Grouped-mode chunks now carry the same chunk-level engine score-fields as flat mode** for chunks that survived the Phase 1 RRF+rerank pipeline: `_rrf_score`, `_final_score`, `_rerank_boost`, `_vector_rank`, `_text_rank`, `_vector_score`, `_text_score`. A Phase 2-only chunk (pulled in by the doc-extension fulltext OR search but never fused in Phase 1) carries only `_text_score`; the absence of `_final_score` etc. is itself the signal that the chunk was not globally ranked. Single source of truth: `apply_score_fields` is now shared by `enrich_result` (flat) and the grouped Phase 2 builder. Before v1.0.502 the grouped builder injected only `_text_score`, so any client that aggregated per-chunk `_final_score` to re-rank documents grouped-mode-side got a degenerated rank that ignored the rerank-boost (empirical Q22/Q23 regression in PeTitanWeb).
- **`max_chunks_per_doc` parameter (grouped mode only in this release)** — caps the length of each group's `chunks[]` array. Applied **after** `lift_common_fields` (so doc-level field promotion sees the full Phase 2 chunk set even when `max_chunks_per_doc=1`) and **before** `total_chunks` accounting (so the response field reflects what the client actually receives). Default `null` = no cap (backward-compatible). The matching flat-mode cap arrives in a follow-up PR (#72); both share one parameter name for a unified client API.

### RAG Pipeline (mcp-server v1.0.494 – v1.0.501)

Comprehensive engineering overview: [`docs/RAG_PIPELINE.md`](docs/RAG_PIPELINE.md).

#### Breaking (v1.0.501)
- **`fulltext_search` response shape is now flat (#68)**, consistent with `hybrid_search`. Document fields move to the top level; engine metadata uses `_`-prefixed keys to avoid colliding with user fields.
  - Before: `{"document": {"_id":1,"title":"T",...}, "score": 6.2, "matched_tokens": [...], "highlights": ...}`
  - After: `{"_id":1,"title":"T",..., "_score": 6.2, "_matched_tokens": [...], "_highlights": ...}`
  - Migration: `hit.document.X → hit.X`, `hit.score → hit._score`, `hit.matched_tokens → hit._matched_tokens`, `hit.highlights → hit._highlights`. The `hybrid_search` parser works for both tools now.
- **`fuzzy_search` response shape is now flat (review follow-up)**, same shape as `fulltext_search`. Hit keys: `<doc fields>`, `_score`, `_matched_value`, `_highlight?`. Brings the entire fulltext-style tool suite to one parser.
- **Rhai `db_fulltext_search`, `db_fuzzy_search`, `db_vector_search` and `db_vector_search_filter` mirror the flat shape** (review follow-up). Same MCP↔Rhai consistency pattern as #66.
- **`vector_search` MCP response confirmed flat across all readers**: MCP `vector_search` already emitted the flat shape (`<doc fields>, _score`); the TUI vector state/renderer was silently reading the pre-flat keys (`document`/`distance` → always null/0.0) — fixed (`VectorSearchResult.distance` renamed to `score`, reads `_score` from the flat hit).
- **`tuti-tui` updated** to consume the flat `fulltext_search` shape (`ironbase-tui/src/modals/fulltext.rs`): reads `_score` and the flat result; the doc-preview helper now also skips `_`-prefixed engine metadata.

#### Hardening (review follow-up, v1.0.501)
- **`GROUP_RESERVED_KEYS` widened** to cover chunk-level engine and structural fields (`chunk_index`, `chunk_total`, `start_char`, `end_char`, `table_header`, `chunk_merged`, `chunks_in_merge`, `_text_score`, `_vector_score`, `_rrf_score`, `_final_score`, `_rerank_boost`, `_vector_rank`, `_text_rank`) **plus the default chunk payload (`content`, `embedding`)** — duplicate ingest can leave every chunk with identical content/embedding; promoting either to the group root would bloat the response (embedding) or mislabel chunk text as doc-level (content). Previously only the four group-root keys were reserved; a coincidentally identical chunk-level field (e.g. all chunks tied on `_text_score`, or every chunk carrying `chunk_merged=true` after a merge run) could lift to the group root, falsely promoting chunk semantics to document semantics. Now those keys stay in `chunks[i]` regardless of value identity. Regression test added.
- **Rhai `db_hybrid_search` now honors `group_by_document` and `match_scope`** — feature parity with the MCP `hybrid_search` tool: STEP 1.5 qualification via the shared `fusion::apply_document_qualification`, Phase 1+2 grouped builder with `lift_common_fields`, and per-chunk `_text_score` annotation. Phase 2 also emits a `tracing::warn!` when a chunk lacks `doc_id` (matching MCP) instead of silently dropping. The Rhai impl returns a `Vec<Dynamic>` of group objects (`{doc_id, best_score, chunk_count, <lifted doc fields>, chunks: [...]}`) when `group_by_document: true`. CLAUDE.md's existing documentation of these options now matches behavior.
- **Stale HTML/Markdown docs updated** (`docs/en/ironbase-guide.html`, `docs/hu/ironbase-guide.html`): the fuzzy/fulltext_search response-shape examples (which were always inaccurate via a `doc` envelope key) now show the flat v1.0.501+ shape. `docs/hu/README.md`'s Python-binding tuple shape is unchanged — that's the PyO3 surface, not MCP.
- **`hybrid_search group_by_document=true` lifts doc-level fields to the group root (#69)**. Keys whose values are identical across ALL chunks of a group (e.g. `title`, `customer`, `year`, `date`, `doc_type`) are moved to the group top level and removed from each `chunks[i]`. Chunk-specific keys (`content`, `_id`, `chunk_index`, `embedding`, `_text_score`, ...) naturally vary across chunks and stay in `chunks[i]`. Generic, no hardcoded field list.
  - **Single-chunk groups are NOT lifted** (review hardening): with one chunk, "all match" is vacuously true on every key; lifting would empty `chunks[0]`. Gated → single-chunk groups keep `chunks[0]` self-contained.
  - **Engine-reserved group-root keys** (`doc_id`, `best_score`, `chunk_count`, `chunks`) are never lifted (review hardening), so a user field that happens to match one of those names cannot overwrite engine metadata at the group root.
  - Migration: `group.chunks[0].title → group.title` for multi-chunk groups (single-chunk groups still expose doc-level fields via `chunks[0].title`).
  - Flat (non-grouped) mode is unchanged.

#### Stale documentation updates (v1.0.501)
- `mcp-server/src/prompts/search.rs` — the `fulltext-search` and fuzzy-search prompt "Response Format" examples updated to the flat shape so LLMs/clients are taught the current contract.

#### Added
- **Contextual chunk embedding** (v1.0.494): `build_embed_text(body, section_path)` prepends a section breadcrumb and flattens markdown tables on the embedded text only; the stored chunk text stays an untouched slice of the source. `strip_markdown_tables` is wired into the embedding path. `db_rag_import` field set now matches `rag_document_import` (`section_path`, `heading_level`).
- **Fenced-code-aware heading detection** (v1.0.494): `# comment` inside ` ``` ` blocks no longer pollutes `section_path`.
- **Idempotent chunk import (#67, v1.0.495)**: new `if_exists` parameter on `rag_document_import` / `embed_document` / Rhai `db_rag_import` — `replace` (default) | `skip` | `error` | `append`. Shared `insert_chunks_idempotent` helper (`helpers.rs`) with safe ordering — captures existing chunk ids via `_id` projection, inserts new, deletes old → never loses good data on partial failure. `should_skip_before_embedding` pre-check skips the expensive embedding call when `skip`/`error` would short-circuit (v1.0.498).
- **Deterministic merge-field resolution (#64, v1.0.496)**: `pick_text_field` resolves the hybrid-search merge field deterministically (content-preferred, lexicographic fallback) — fixes a HashMap-order bug that occasionally caused `merge_adjacent_chunks` to concatenate `title` instead of `content`.
- **Configurable fulltext language (#65, v1.0.497)**: `language` parameter on `rag_document_import` + Rhai `db_rag_import` (default `"none"`, Snowball stemmers for hungarian/english/german). `fulltext_analyze` gains optional `collection`+`field` to inherit the real index `FtsOptions` via the new `adapter.get_fulltext_index_options` — the debugger now reflects exactly how that index tokenizes (`inherited_from_index` response flag).
- **Markdown table header propagation (#63, v1.0.499)**: table-continuation chunks get the `<header>\n<separator>` block prepended (stored in a new `table_header` chunk field), so every retrieved table chunk is self-interpretable. Detection runs on the raw (overlap-free) slice (production default `overlap=100` works). `merge_adjacent_chunks` strips the duplicated header via the stored field — exact, CRLF-safe. `current_table_header` resets on heading change.
- **Multi-field fulltext indexing (#66, v1.0.500)**: new `text_fields` parameter on `rag_collection_create` + `rag_document_import` + Rhai `db_rag_create`/`db_rag_import` — creates an FTS index on every listed field (primary `text_field` always included, deduplicated). `RagConfig` gains `text_fields: Vec<String>` (`#[serde(default)]` → backward-compatible). `hybrid_search` (MCP and Rhai `db_hybrid_search`) defaults to the configured multi-field set when the caller omits `text_fields`. `rag_document_import` auto-creates a `RagConfig` when none exists, so the default applies to import-only workflows too. `rag_collection_stats` reports `text_fields`.

#### Hardened (code review follow-ups)
- `embed_document` now filters `RESERVED_METADATA_KEYS` like `rag_document_import` — user-supplied `metadata.doc_id` can no longer override the chunk's `doc_id` and silently defeat #67 idempotency (v1.0.498). `RESERVED_METADATA_KEYS` consolidated into a single shared definition (`helpers.rs`); `table_header` added.
- `rag_document_import` `language_ignored` warning only fires when the requested language *differs* from the stored config (no false positive on repeated imports with identical arguments) (v1.0.500).
- Multi-field default search intersects with the actually-indexed fields (`resolve_search_text_fields`) — a configured-but-unindexed field (failed creation / later `index_drop`) no longer hard-errors `fulltext_search_multi`; it is silently excluded from the default set. Explicit `text_fields` from the caller is honored as-is (v1.0.500).
- Rhai layer brought in line with MCP: `get_rag_config` carries `text_fields`, `db_hybrid_search` and `db_rag_stats` are consistent with the MCP equivalents (v1.0.500).
- `auto_embed_enable` warns when applied to a chunk-imported collection (verbatim-embed boundary made visible, not silently degraded) (v1.0.494).

### Fixed
- **MCP System Collections Hidden Flag**: `ensure_system_collections()` now fixes flags on existing collections
  - Legacy databases where `_system.scripts` was created before flag system now get `hidden: true`
  - Auto-migrates on MCP server startup - no manual intervention needed

### Added
- **Cross-Process File Locking**: Prevents database corruption from multiple processes
  - Uses `fs2` crate for OS-level exclusive file locks
  - Non-blocking `try_lock_exclusive()` - returns error immediately if locked (deadlock-free)
  - Automatic lock release on process exit or crash
  - New `DatabaseLocked` error type for clear error messages
- **Explicit `close()` Method**: Release file lock without waiting for GC/Drop
  - `DatabaseCore::close()` - flush and release lock for immediate reopening
  - Critical for Python/C# where GC timing is unpredictable
  - Fixes Windows CI test failures with file persistence tests
- **Safe Hot Backup**: Snapshot isolation for consistent backups from running databases
  - Shared lock only during metadata read (~1ms) - DB continues operating
  - Append-only storage guarantees data immutability during backup
  - `concurrent_writes` flag indicates if DB was modified during backup
  - New data automatically included in next incremental backup
- Full GitHub publication readiness (CONTRIBUTING, CODE_OF_CONDUCT, SECURITY)
- CI/CD workflows for all platforms
- **Compound Index Prefix Queries**: Queries on first field of compound index now use index
  - `{"country": "HU"}` uses `(country, city)` compound index via range scan
  - New `IndexKey::MaxKey` sentinel for upper bounds
  - `build_prefix_range()` method for compound prefix bounds
  - `IndexPrefixInfo` struct for QueryPlanner compound awareness
- **Explicit Collection Existence Checks**: `get_collection()` now returns error if collection doesn't exist
  - Prevents typos from creating empty collections (e.g., `find("uusers")`)
  - Insert operations still create collections implicitly (MongoDB-compatible)
  - New `CollectionNotFound` error type propagates through MCP layer

### Changed
- **Performance Optimization**: `get_collection()` now uses READ locks only
  - Hot path reduced from 4 locks (incl. WRITE) to 2 READ locks
  - New `with_shared_indexes_readonly()` method for existing collections
  - Significantly reduces lock contention under high concurrency
  - Storage WRITE lock no longer needed for read operations

### Fixed
- **MCP `collection_create` tool**: Now actually creates the collection
  - Previously returned success without performing any operation
  - Root cause: tool handler was a no-op, relying on implicit creation that never happened

### Removed
- **FileStorage**: Removed deprecated wrapper around StorageEngine
  - Was redundant layer with no added functionality
  - Use `StorageEngine` directly instead

## [1.0.5] - 2025-12-06

### Added
- **Fuzzy Text Search**: New `$fuzzy` query operator with multiple algorithms
  - Jaro-Winkler (default, fastest)
  - Levenshtein (most accurate)
  - Damerau-Levenshtein (best for typos/OCR)
  - Configurable similarity threshold (0.0-1.0)
- **Fuzzy Indexes**: `create_fuzzy_index()` for optimized fuzzy searches
  - Accelerates `$fuzzy` queries with pre-computed similarity
  - Supports all three algorithms
  - Configurable threshold per index
- **Pagination Enhancement**: `include_total` parameter for FindOptions
  - Returns total document count alongside results
  - Enables "Showing 1-10 of 100 results" UI patterns
- **MCP Tools**: New fuzzy search tools
  - `fuzzy_search` - Execute fuzzy text queries
  - `index_create_fuzzy` - Create fuzzy text indexes
- **MCP Server Improvements**
  - Configurable max body size with human-readable format
  - `-Update` switch for easy service updates (Windows)
  - Improved Windows install experience
  - CRLF to LF normalization in config parsing

### Fixed
- Windows configuration file line ending issues

## [0.3.0] - 2025-11-26

### Added
- **C# Bindings**: Complete .NET bindings with MongoDB-like API
  - NuGet package support
  - Cursor/Streaming API for large datasets
  - Collection-level transaction support
  - Schema validation
  - Logging API
- **Nested Documents**: Full dot notation support
  - Query nested fields: `{"address.city": "NYC"}`
  - Update nested fields with all operators
  - Index nested fields for fast lookups
- **Array Operations**: MongoDB-style array element matching
  - `$size` operator for array length queries
  - Array element queries with dot notation
- **MCP Server**: Model Context Protocol integration
  - DOCJL document editing support
  - 15+ prompts for AI-assisted operations
  - Resources and tools endpoints
- **Testing Infrastructure**
  - 5-hour fuzzing corpus (~367M iterations)
  - Chaos testing framework
  - 140+ C# tests
  - 31 nested document tests
- **In-Memory Mode**: `DatabaseCore::open_memory()` for testing

### Fixed
- Integer overflow in `$sum` aggregation
- `$inc` now creates non-existent fields
- Dot notation for nested field indexes on persistence reload
- UTF-8 character preservation in MCP bridge

### Changed
- Modularized `collection_core` module structure
- Improved documentation with dot notation examples

## [0.2.0] - 2025-11-10

### Added
- **Query Operators**: Complete MongoDB-compatible query system
  - Comparison: `$eq`, `$ne`, `$gt`, `$gte`, `$lt`, `$lte`, `$in`, `$nin`
  - Logical: `$and`, `$or`, `$not`, `$nor`
  - Element: `$exists`, `$type`
  - Array: `$all`, `$elemMatch`
  - Regex: `$regex`
- **Update Operators**
  - `$set`, `$unset`, `$inc`
  - `$push`, `$pull`, `$addToSet`, `$pop`
- **Aggregation Pipeline**
  - Stages: `$match`, `$group`, `$project`, `$sort`, `$limit`, `$skip`
  - Accumulators: `$sum`, `$avg`, `$min`, `$max`, `$first`, `$last`
- **B+ Tree Indexing**
  - Unique and non-unique indexes
  - Compound indexes
  - Query optimizer with explain()
- **FindOptions**: Projection, sort, limit, skip
- **Transactions**: ACD (Atomicity, Consistency, Durability) with WAL
- **Durability Modes**: Safe, Batch, Unsafe
- **JSON Schema Validation**
- **Query Cache**: LRU cache for repeated queries
- **Python Bindings**: Complete PyO3-based Python API

### Changed
- Strategy pattern for query operators (83% complexity reduction)
- Improved storage engine with compaction support

## [0.1.0] - 2025-11-01

### Added
- Initial release
- Core CRUD operations: `insert_one`, `insert_many`, `find`, `find_one`, `update_one`, `update_many`, `delete_one`, `delete_many`
- Append-only storage engine
- Document catalog with offset tracking
- Basic query matching
- WAL (Write-Ahead Log) for crash recovery
- MIT License

[Unreleased]: https://github.com/petitan/IronBase/compare/v1.0.55...HEAD
[1.0.5]: https://github.com/petitan/IronBase/compare/v0.3.0...v1.0.5
[0.3.0]: https://github.com/petitan/IronBase/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/petitan/IronBase/releases/tag/v0.2.0
[0.1.0]: https://github.com/petitan/IronBase/releases/tag/v0.1.0
