# Stage A — Implementation Plan: `search` tool (passage unit + compact contract)

**Scope:** Stage A of `HYBRID_RETRIEVAL_REDESIGN.md` only — the calibration-
independent, high-value, low-risk part (~70–80% of the redesign's value per both
external reviews). **No new retrieval primitive.** Reuses the *existing* RRF
fusion and grouped builder; only the *unit* (passage-anchored-to-document) and the
*response contract* (compact, no internals) are new.

**Explicitly OUT of Stage A:** calibration, abstention verdict logic, HyDE,
`max`-fusion change (that is Stage B), removal of `hybrid_search` (deprecation
comes later). The new `search` tool ships **alongside** `hybrid_search`.

---

## 1. Conventions to follow (read before writing — CLAUDE.md "Kód Konzisztencia")

| Concern | Follow the existing pattern in |
|---------|--------------------------------|
| Param struct + parsing | `params.rs` (`#[derive(Deserialize)]` + `ParseParams::parse`, `#[serde(default=...)]` defaults) |
| Tool schema (no top-level oneOf/anyOf) | `definitions/rag.rs` (the `rag_load_all_chunks` entry added this session) |
| Schema aggregation | `definitions/mod.rs::get_all_tools_json` (`tools.extend(<module>::tools())`) |
| Dispatch routing | `tools/mod.rs::dispatch_tool_inner` (name → module `dispatch`) |
| Reuse of fusion/grouping/merge | `tools/fusion.rs` (`merge_adjacent_chunks`, `lift_common_fields`, `FusedResult`), `tools/hybrid.rs` grouped builder |
| Integration test style | `tests/mcp_tests.rs` (`dispatch_ok`, `seed_rag_kb`, the `rag_load_all_chunks` tests) |
| Errors | `McpError::invalid_params` / `::internal` |

**Output block required by CLAUDE.md when writing the new module:**
`Mintának használt: rag.rs (dispatch+handler), params.rs (param struct), definitions/rag.rs (schema). Konvenciók: ParseParams parse, McpError, serde defaults, fusion.rs reuse.`

---

## 2. Module layout

New module `mcp-server/src/tools/retrieval/` (per redesign §16):

| File | Responsibility |
|------|----------------|
| `retrieval/mod.rs` | `dispatch("search", …)`, `SearchParams`, the `search` handler (orchestrates: retrieve → assemble contract) |
| `retrieval/contract.rs` | the compact response types + assembly (strip internals, passages, token budget, `context_block` formatting) |

Touched existing files:
- `tools/hybrid.rs` — **extract** the retrieve+group core into a reusable
  internal fn (see step 1); the old `handle_hybrid_search` keeps its public
  behavior by calling it. (Seeds the F1/F2 cleanup; low-risk, no behavior change.)
- `tools/mod.rs` — `pub mod retrieval;` + dispatch route for `"search"`.
- `tools/definitions/mod.rs` — `tools.extend(retrieval::tools());` + bump
  `test_tool_count` (92→93 this session already → 94).
- `tools/definitions/retrieval.rs` — schema for `search` (new; or co-locate in
  the module's `tools()` mirroring `definitions/rag.rs`).
- `Cargo.toml` (workspace) + `mcp-server/Cargo.toml` — version bump.

---

## 3. The `search` surface (intent only)

```rust
#[derive(Debug, Deserialize)]
pub struct SearchParams {
    pub collection: String,
    pub query: String,
    #[serde(default, deserialize_with = "deserialize_nonempty_filter")] // reuse params.rs helper
    pub filter: Option<Value>,
    #[serde(default = "default_search_limit")]      // max DOCUMENTS (coarse intent)
    pub limit: usize,
    #[serde(default = "default_format")]            // "structured" | "context_block"
    pub format: String,
    #[serde(default)]                               // include diagnostics block
    pub debug: bool,
}
```

No mechanism knobs. `rrf_k`, weights, `match_scope`, `merge_chunks`, etc. are NOT
accepted (they live in the core config). If a caller passes one → reject with
`McpError::invalid_params` pointing to the config (P6: not silently ignored).

---

## 4. The compact response contract (`contract.rs`)

```rust
struct SearchResponse {
    verdict: &'static str,        // Stage A: always "unknown" (no calibration yet — honest, §9 four-state)
    documents: Vec<SearchDoc>,
    trimmed: bool,                // token budget dropped lower passages (P6)
    diagnostics: Option<Value>,   // only when debug=true
}
struct SearchDoc {
    doc_id: String,
    // doc-level fields lifted generically (title/customer/year/…) — NO hardcoded list,
    // reuse fusion::lift_common_fields output
    relevance: f64,               // Stage A: the existing fused/RRF score (NOT calibrated; documented as relative-only)
    passages: Vec<Passage>,
}
struct Passage { text: String }   // merged passage text; NO embedding, NO _scores, NO chunk_index
```

**Assembly rule (universal, no hardcoded field names):** take the existing grouped
output (doc groups with merged chunks + lifted fields), and for each group emit a
`SearchDoc` that keeps `doc_id` + lifted doc-level fields, sets `relevance =
best_score`, and maps each merged chunk to a `Passage { text: <text field> }` —
**dropping** `embedding`, every `_`-prefixed engine field, and chunk-tracking
fields (`chunk_index`, `start_char`, …). This is the single biggest small-model
win (no more 1024-float arrays in context).

**Token budget:** estimate package size (char/word based — a budget, not a
relevance decision, so P3-allowed); if over the configured budget, drop the
lowest-relevance passages/documents and set `trimmed = true`. Budget lives in
`RetrievalConfig`, not the MCP surface.

**`context_block` format:** render `documents` into a citation-marked plain-text
block ready to paste into the qwen prompt (e.g. `[doc_id] title\n<passages>`).

---

## 5. Step-by-step (each step independently compilable + tested)

1. **Extract retrieval core.** In `hybrid.rs`, factor the STEP 1–7 retrieve+group
   logic into `pub(crate) fn retrieve_grouped(adapter, embedding_manager, &cfg) ->
   Result<Vec<DocGroup>>` where `DocGroup` is the internal grouped struct (doc_id,
   best_score, lifted fields, merged passages). `handle_hybrid_search` grouped
   path calls it and shapes the *old* response (unchanged). **Test:** existing
   `test_hybrid_grouped_*` stay green (no behavior change).
2. **`SearchParams` + schema.** Add the param struct (`retrieval/mod.rs`) and the
   `search` tool schema (`definitions/retrieval.rs`), register in
   `definitions/mod.rs`; bump `test_tool_count`. **Test:** `test_no_top_level_oneof_allof_anyof`, `test_tool_count`, `test_all_tools_have_required_fields` green.
3. **Contract assembly (`contract.rs`).** `Vec<DocGroup>` → `SearchResponse`:
   strip internals, build passages, lift fields, relevance, token budget, both
   formats. **Unit tests:** no `embedding`/`_`-prefixed keys in output; passages
   non-empty; `verdict=="unknown"`; `context_block` contains doc_id markers.
4. **`search` handler + dispatch.** Wire `retrieval::dispatch("search")` →
   `retrieve_grouped` (with a `RetrievalConfig` defaulting group-by-document on,
   merge on) → `contract::assemble`. Route in `tools/mod.rs`. Reject mechanism
   params. **Integration tests** (`mcp_tests.rs`, `seed_rag_kb` style): tool
   listed; returns documents+passages; **no embedding, no `_`-metadata**;
   `verdict=="unknown"`; mechanism param → error.
5. **Version bump + CHANGELOG** (`Added`: `search` tool, Stage A).

---

## 6. Stage A acceptance gate (from redesign §12)

- **No retrieval-quality regression:** the document ordering from `search` equals
  `hybrid_search group_by_document=true` for the same query/filter (same fusion
  underneath). Asserted by an equivalence test on the doc-id order.
- **Token economy (metric 5):** mean response byte/token size of `search` is
  materially lower than `hybrid_search` for the same query (driven by dropping
  embeddings + engine metadata). Asserted by a comparison test.
- All existing tests green (`cargo test -p mcp-ironbase-server`), `fmt` + `clippy`
  clean.

---

## 7. Risk / non-goals reminder

- This is additive: `hybrid_search` unchanged and still available. Zero production
  risk to the current chat path until the caller opts into `search`.
- `relevance` in Stage A is the **existing relative fused score**, explicitly
  documented as not-absolute (calibration is Stage C+). `verdict` is honestly
  `unknown`. No abstention claims are made in Stage A.
- The retrieve-core extraction (step 1) must be behavior-preserving for
  `hybrid_search`; its existing tests are the guard.
