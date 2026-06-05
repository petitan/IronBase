# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
