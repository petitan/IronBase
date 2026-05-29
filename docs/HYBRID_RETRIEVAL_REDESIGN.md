# Hybrid Retrieval Redesign — Architecture for Review

**Status:** Design proposal (pre-implementation). Breaking change permitted.
**Target consumer:** a small local LLM (qwen3.5:9b via Ollama/AI-proxy) doing RAG.
**Date:** 2026-05-29. **Revision: v3 — revised after second external review.**

> **v3 changelog (second review).** The second review judged v2 "diagnosis and
> therapy now near the same weight class," with four remaining issues, all
> addressed here: **(§7 hidden contradiction)** "ranking never depends on
> calibration" vs "relevance = max calibrated P" — fixed by defining the ranking
> key explicitly per regime (§8: raw fused score when calibration is off,
> calibrated P only when gated on). **(#5 max too conservative)** `max` is now
> framed as a deliberate conservative *floor*, not the optimum; the combiner is
> eval-selected among max / OR / correlation-corrected / learned (§7).
> **(#6 mixture still secretly preferred)** the calibrator candidates are now
> listed on equal footing with the mixture explicitly *not* preferred; the spike
> may reject all (§5). **(#4 too future-tense)** a concrete, pre-registered spike
> specification with corpora, sample sizes, and pass thresholds is added (§12),
> and §4a now states plainly that the load-bearing value is the prosaic Stage-A
> work, not the calibration track — adopting the board's constraint that C–E stay
> experimental until numerically proven.

> **v2 changelog (what the review changed).** An external review judged the
> diagnosis stronger than the therapy, with the central flaw being that the whole
> stack was built on one unproven primitive (unsupervised relevance calibration),
> a single point of failure. v2 **inverts the dependency**: a robust, proven
> rank/score core works on its own; calibration, HyDE, and abstention become
> *additive, independently-gated enhancement layers* that degrade gracefully.
> Specific fixes: fusion default changed to correlation-robust `max` (§7); the
> 2-component mixture is no longer assumed universal and is gated by a spike on
> real corpora *before* anything builds on it (§5); HyDE demoted to an
> eval-gated option, not a load-bearing default (§7/P5); the "no heuristics"
> dogma refined to "no unvalidated magic constants in relevance decisions;
> documented, validated heuristics are acceptable baselines and fallbacks" (P3).
> See §4a for the dependency posture and staged delivery.

This document is self-contained: a reviewer who has not seen the surrounding
discussion should be able to understand, assess, and grade it. It specifies
*what* and *why*, with enough *how* to evaluate soundness, testability, and risk.

---

## 1. Context & problem

The current `hybrid_search` MCP tool fuses BM25 (lexical) and HNSW vector
(semantic) retrieval via Reciprocal Rank Fusion (RRF), with reranking, MMR
diversity, adjacent-chunk merge, and document grouping. It exposes **23
parameters** (`mcp-server/src/tools/params.rs::HybridSearchParams`).

Four conceptual defects, in order of depth:

1. **Unit confusion.** Chunks are the embedding/index unit, but they leaked out
   as the *retrieval* unit. Document reconstruction (qualification, grouping,
   adjacent-merge, `match_scope`, `max_chunks_per_doc`) is all compensation for
   having shredded documents and needing them back. This defect generates most of
   the **parameter surface** — but *not* most of retrieval's intrinsic difficulty
   (review §1). Lexical/semantic divergence, query ambiguity, domain drift,
   multi-hop, metadata filtering, and ranking instability remain real problems
   that hiding the chunk does **not** solve; this redesign scopes the unit/contract
   and the absolute-relevance gap, and explicitly leaves those open (§15).
2. **No absolute relevance.** RRF fuses *ranks*; it discards score magnitude. It
   structurally cannot answer "is anything actually relevant?" — so it cannot
   abstain. A hallucination-prone small model is then fed marginal results for
   queries the corpus cannot answer.
3. **Fusion-as-blending.** Fixed weights always average the two modalities. For a
   part-number query ("PEF-35") the lexical signal is authoritative and the
   vector signal is noise; blending dilutes the authoritative signal.
4. **Config-as-request-parameter.** Mechanism knobs (`rrf_k`, `mmr_lambda`,
   weights, boosts) are exposed to the caller. The caller is a 9B model that
   cannot tune them and lacks corpus statistics; it either ignores them (fine) or
   fills them wrong (actively harmful).

The consumer being a *small* model turns these from elegance concerns into
correctness requirements: a 9B model cannot stitch chunk shrapnel, cannot tune
knobs, cannot reliably abstain from weak context, and degrades with token bloat
(the current response embeds the full 1024-float `embedding` array per chunk by
default).

---

## 2. Goals / non-goals

**Goals (priority order):**
1. **Correctness of the answer/abstain decision** — never hand marginal evidence
   to the model; emit an explicit "no answer in corpus" verdict.
2. **Sufficient + minimal evidence** — everything needed, nothing extra (token
   economy, "lost in the middle").
3. **Citeability** — every returned passage is anchored to its source document.

**Non-goals:**
- Per-call mechanism tuning by the model.
- Exposing "chunk" as a public concept.
- Returning a fixed K regardless of relevance.
- Returning raw scores / embeddings / engine internals to the model by default.

---

## 3. Locked design principles

| # | Principle | Resolves |
|---|-----------|----------|
| P1 | **Passage-anchored-to-document is the universal retrieval unit.** Chunks are internal. Output is ranked documents, each carrying merged/deduped evidence passages. A short document's passage ≈ the whole document; a long manual returns its relevant section. One mechanism, no per-collection branching. | Defect 1, universality, long-manual case |
| P2 | **Calibrated relevance is an additive enhancement, not the foundation.** *(v2, was: "the core primitive")* A robust rank/score core (§4a) ranks and fuses without calibration. Calibration sits *on top* to add an absolute-relevance signal that powers abstention. If calibration is unavailable/provisional/failing its gate, the system still ranks and returns results; only the abstention verdict degrades to `unknown` (surfaced). No single point of failure. | Defect 2; review §3 |
| P3 | **No unvalidated magic constants in relevance decisions.** *(v2, refined from "no heuristics")* A relevance decision rule (abstention threshold, fusion mixing) must either be parameter-free or fit/validated against the eval (§12) — never a hand-typed number asserted without measurement. A **documented, eval-validated heuristic is acceptable**, and is the fallback when a learned alternative does not beat it. Infrastructure constants (BM25 k1/b, HNSW M/ef, candidate widths) are out of scope — they are standard, stable, and tuned by their own literature. | Defect 4; review §5 |
| P4 | **Intent in, mechanism owned by server.** The model supplies only intent (`collection`, `query`, `filter`, `limit`). All mechanism is server-side, calibrated, and never in the call. | Defect 4 |
| P5 | **HyDE is an eval-gated semantic option, default-on only if it wins.** *(v2, demoted from "default path")* The baseline semantic path is raw-query embedding. HyDE (embedding LLM-generated hypotheticals) is enabled per collection **only if §12 measures a recall/nDCG win over the baseline**. The current deployment reports HyDE "works well" but without numbers; that claim is treated as a hypothesis to be measured, not evidence. | Semantic recall — if measured |
| P6 | **No silent fallback.** Any degraded path (HyDE unavailable, calibration cold-start/provisional, fusion fallback) is surfaced in the response and logs, never silently substituted. | Project invariant (CLAUDE.md) |
| P7 | **Graceful degradation / staged enhancement.** *(v2, new)* Every enhancement layer (calibration, HyDE, abstention) is independently switchable and independently gated by the eval. With all of them off, the system equals the robust baseline core (no regression vs today). Each layer ships only after it passes its gate. | Review §3, §7 |

**Precise scope of P3 (v2, refined after review):**
- *Intent parameter* (e.g. `limit=3 documents`) — caller's wish, **allowed**.
- *Infrastructure constant* (BM25 k1/b, HNSW M/ef, candidate width) — standard,
  stable, literature-tuned, **allowed**; not what P3 targets.
- *Relevance decision rule* (abstention threshold, fusion mixing) — must be
  parameter-free **or** validated against the eval (§12). A **documented,
  eval-validated heuristic threshold is acceptable** and is the fallback when a
  learned/fitted alternative does not beat it on the eval. What remains forbidden
  is a hand-typed relevance constant *asserted without measurement* (the old
  rerank `1.5×`/`1.3×` boosts are the anti-pattern). The review's point stands: a
  validated heuristic can be more stable than a fragile fitted model — so the
  eval, not ideology, decides which ships.

---

## 4. Architecture overview

```
                          ┌─────────────────────────────────────────────┐
   MCP façade  ──────────▶│  search(collection, query, filter?, limit?)  │   (intent only)
   (LLM-facing)           └───────────────────────┬─────────────────────┘
                                                   │
                          ┌────────────────────────▼─────────────────────┐
   L2  Retrieval          │  Orchestrator                                 │
       orchestration      │   1. HyDE expand (P5)                         │
                          │   2. modality retrieval (L0)                  │
                          │   3. calibrate (L1)                           │
                          │   4. fuse (parameter-free)                    │
                          │   5. assemble passages→documents (P1)         │
                          │   6. verdict / abstain (P2)                   │
                          └───┬───────────────┬───────────────┬──────────┘
                              │               │               │
        ┌─────────────────────▼──┐  ┌─────────▼────────┐  ┌───▼───────────────┐
   L1   │  Calibrator (per coll, │  │  L0 Vector index │  │  L0 Fulltext index │
        │  per modality)         │  │  (HNSW + HyDE)   │  │  (BM25 + chunk_doc │
        │  raw score → P(rel)    │  │                  │  │   mapping)         │
        └────────────────────────┘  └──────────────────┘  └────────────────────┘

   L3  Response contract: compact evidence package (no internals, token-budgeted)
   Core: RetrievalConfig struct (all knobs; for Rust callers / eval / power-use)
```

**Layer responsibilities**

- **L0 Index layer (existing, unchanged):** HNSW vector index, BM25 fulltext
  index, `chunk_doc_mapping`. Emits raw per-modality candidate lists with *raw*
  scores (cosine, BM25). No fusion logic here.
- **L1 Calibration layer (new):** an *additive, gated* enhancement (§5; not the
  foundation — see §4a). Pure function of raw scores + a per-(collection,
  modality) fitted model → `P(relevant)`.
- **L2 Orchestration (new; replaces the `hybrid.rs` god-function):** the pipeline
  in §6. Stateless given config; each step independently testable.
- **L3 Response contract (new):** assembles the compact evidence package (§10).
- **MCP façade (new, thin):** maps intent → `RetrievalConfig` defaults, calls L2,
  returns L3. ~4 parameters (§11).
- **Core `RetrievalConfig` (new):** the full knob set lives here, for Rust
  callers, the eval harness, and power use — never on the MCP surface.

---

## 4a. Dependency posture & staged delivery (v2 — the review's main fix)

The review's strongest point: building ranking + fusion + routing + abstention
all on one unproven primitive (`calibrated P`) is a single point of failure —
worse than today, where BM25 and vector each work independently. v2 inverts this.

**Dependency layers, bottom is load-bearing, top is additive:**

```
Layer 3  Abstention verdict        ── needs calibration; absent ⇒ verdict="unknown"
Layer 2  Calibration (P_relevant)  ── ADDITIVE; gated by §12 before it powers anything
Layer 1b HyDE semantic expansion   ── ADDITIVE; gated by §12; baseline = raw-query embed
Layer 1a Robust fusion (ranking)   ── LOAD-BEARING: correlation-robust, parameter-free (§7)
Layer 0  BM25 + HNSW (raw scores)  ── LOAD-BEARING: unchanged, each works alone
─────────────────────────────────────────────────────────────────────────────
Layer R  Passage→document assembly + compact contract (§8, §10)
          ── INDEPENDENT of everything above; the low-risk, diagnosis-strong win
```

**Consequences (each answers a specific review criticism):**

- *No single point of failure (review §3).* If calibration fails its gate or is
  provisional, Layers 0–1a still rank and return results; only Layer 3 abstention
  degrades to a surfaced `unknown`. The system never becomes *worse* than the
  robust core.
- *Primitive proven before it's load-bearing (review §4).* Calibration is built
  and measured as a **spike on the real corpora first** (quotes + the long
  manual). It only graduates to powering abstention after passing §12's
  calibration gate. Nothing depends on it until then.
- *Diagnosis-strong wins ship first (review §1, §8).* Layer R (unit fix + response
  contract: chunk hidden, embeddings stripped, passages, citations, token budget)
  is independent of the unproven parts and delivers most of the practical value
  immediately, at low risk.

**Staged delivery order (each stage independently shippable & gated):**

> **Ordering correction (v3.1).** An earlier draft listed Stage B ("replace RRF
> with `max` fusion") as implementable right after Stage A. That was wrong and
> self-contradicted §7: `max(P_v, P_t)` operates on **calibrated** probabilities,
> which do not exist until Stage C — `max` of raw cosine (bounded) and raw BM25
> (unbounded, corpus-dependent) is meaningless. RRF (rank-based, scale-free) is
> therefore retained as the fusion through A **and until calibration ships**.
> Stage B (max fusion) is a *consumer* of calibration, so it is reordered to
> **after** Stage C, and both depend on the **eval harness** as their gate.

1. **Stage A — Layer R (DONE).** Passage-anchored unit + compact contract over the
   *existing* RRF fusion. No new primitive. Token + clarity win. Gate met:
   no retrieval-quality regression (doc-order equivalence vs `hybrid_search`),
   token economy improved (§12 metric 5).
2. **Eval harness (prerequisite for every gate below).** Labeled query sets +
   metric implementations (nDCG@k, Recall@k, Brier, abstention precision/recall).
   Without it, "no heuristics → must measure" (P3) cannot be enforced and B/C/D/E
   cannot be gated. The metric code and a no-regression gate on a controlled
   labeled set are buildable now; real per-corpus labels are a data step (§12).
3. **Stage C — Layer 2 spike.** Build calibration, measure it standalone on real
   corpora (§12 metric 2). **Hard gate: if no calibrator candidate separates,
   do not proceed to abstention; keep it a non-shipping experiment.**
4. **Stage B — Layer 1a (depends on C).** Replace RRF with the correlation-robust
   parameter-free `max` over *calibrated* probabilities (§7), only once Stage C's
   calibrator passes its gate. Gate: no nDCG/Recall regression vs RRF (eval harness).
5. **Stage D — Layer 3.** Abstention verdict, only if Stage C passed. Gate:
   abstention precision/recall (§12 metric 3) + lower hallucination rate (metric 4).
6. **Stage E — Layer 1b.** HyDE, only if it beats the raw-query baseline (§12).

This sequencing makes the proposal "improve a production system incrementally,"
not "replace it with an unproven research design."

**Where the value actually is (v3, accepting the review board's framing).** The
load-bearing value of this redesign is **not** calibration, HyDE, or abstention —
those are an experimental track that may yet prove a dead end. The value is the
prosaic, calibration-independent **Stage A**: hiding the chunk abstraction, the
document-centric contract, token economy, the smaller MCP surface, diagnostics
separation, and gradual migration. These ship first and stand on their own even
if the entire calibration line is later abandoned.

**Adopted production constraint (the review board's condition, v3.1 corrected):**
> Stage A is shipped (v1.0.504). The eval harness is the next buildable
> prerequisite. **Stages B–E may run only as an experimental branch and must
> never become a production dependency until the validation plan (§12)
> numerically proves their advantage** — and Stage B specifically cannot precede
> Stage C, since `max` fusion consumes calibrated probabilities. A failed
> calibration track costs us nothing already shipped.

---

## 5. Enhancement primitive — relevance calibration (high-risk; gated, additive)

This is the highest-risk component and, per §4a, **additive and gated**: nothing
ships on top of it until it passes the §12 calibration gate on real corpora. It
is specified as a trait so the method can be swapped without touching
orchestration.

**The bimodality assumption is NOT taken for granted (review §4).** Real score
distributions — BM25 especially — are often not cleanly two-component: multiple
relevance levels, document types, and query regimes can blur or multi-modalize
them. Therefore the calibrator is *pluggable* and the 2-component mixture is only
the **first candidate**, not a committed foundation. The Stage-C spike (§4a)
measures separability directly (§12 metric 2) and is empowered to reject the
mixture in favour of an alternative (non-parametric isotonic calibration against
weak labels, or a quantile/empirical-CDF mapping) — or to conclude that reliable
absolute calibration is not achievable for a given corpus, in which case Layer 3
abstention is simply not enabled there (the system stays at the robust core).

```
trait Calibrator {
    /// Map a batch of raw modality scores to P(relevant) ∈ [0,1].
    fn calibrate(&self, modality: Modality, raw_scores: &[f64]) -> Vec<f64>;
    /// Status surfaced to the response (P6): Ready | Provisional(reason).
    fn status(&self, collection: &str, modality: Modality) -> CalibrationStatus;
}
```

**The spike evaluates a set of candidate calibrators on equal footing (v3 —
review §6 noted v2 still secretly preferred the mixture). No method is the
committed answer; the eval (§12 metric 2) picks the winner, or rejects all.**

Candidates, none privileged:

1. **2-component mixture (EM), unsupervised.** P(relevant) = posterior of the
   relevant component. *Strength:* parameter-free, no labels. *Weakness:* assumes
   the score distribution is cleanly bimodal — often false for BM25 (review §4).
   Listed first only for exposition, **not as the preferred method.**
2. **Empirical-CDF / quantile mapping** of scores against an unsupervised
   background estimate. *Strength:* no distributional-shape assumption. *Weakness:*
   needs a background definition.
3. **Isotonic regression against weak labels** (a small LLM-judged sample). Not
   "v2-later" — available to the spike if the unsupervised options fail. *Strength:*
   makes no shape assumption, directly fits P(relevant). *Weakness:* needs a
   labeled sample (the spike produces one anyway, see §12).

The spike's verdict has three outcomes: a calibrator passes the gate (it ships
for that collection), or a *different* candidate wins, or **none passes** — in
which case Layer-3 abstention is simply not enabled for that corpus and the system
stays at the robust core (P2/P7). The mixture is explicitly **not** the foundation
the design leans on; calibration as a *capability* is, and it may be delivered by
whichever candidate survives — or not delivered at all, without breaking the core.

- Whatever the winning calibrator, its relevant/non-relevant **boundary is derived
  from the fit**, never hand-set (§9). Cosine and BM25 are calibrated
  independently (their stability differs — review §4); abstention may run
  cosine-led where BM25 calibration proves unreliable.
- Fit offline/periodically from sampled results, persisted with the index.
  **Cold-start:** `status = Provisional` surfaced (P6); verdict → `unknown` (§9).

**Signal from HyDE (P5) feeds calibration, it does not bypass it.** HyDE produces
`N` hypothetical documents; each is embedded and run as a separate semantic query.
Their results are additional samples of the same posterior — agreement across
generations sharpens the fused posterior naturally (more evidence → tighter
estimate); scatter leaves it diffuse (abstain-leaning). No "agreement multiplier"
constant is introduced.

**Continuous-improvement seam:** the `Calibrator` trait lets a richer learned
calibrator (more labels, usage signal) replace whichever candidate ships, with no
orchestration change.

**Risk & validation:** calibration quality is the central research risk, and it is
*isolated* (P2/P7) — its failure costs only the abstention verdict, not ranking.
§12 metric 2 (reliability diagram / Brier on a held-out labeled set) measures it
directly so a reviewer can grade whether *any* candidate holds before it ships.

---

## 6. Retrieval orchestration pipeline (L2)

Replaces the ~490-line `handle_hybrid_search`. Each step is a pure, independently
testable function.

1. **Semantic embedding (HyDE optional, P5).** Baseline: embed the raw query.
   If HyDE is enabled for this collection (it passed its §12 gate): `query → LLM →
   N hypotheticals → embed each`, cached by query. If HyDE is enabled but the LLM
   is unavailable at request time: fall back to raw-query embedding **and set
   `hyde: unavailable` in the response (P6)**. With HyDE off, this step is just the
   raw-query embedding — no LLM call, no added latency.
2. **Modality retrieval (L0).** Vector search (with HyDE embeddings) and BM25
   search, each returning raw-scored candidate *chunks* with their `doc_id`.
   Internal candidate width is a `RetrievalConfig` value, not an MCP parameter.
3. **Calibrate (L1).** Raw cosine → `P_v`, raw BM25 → `P_t`, per candidate.
4. **Fuse (parameter-free, §7).** Combine `P_v`, `P_t` → `P`. Routing emerges
   here with no classifier.
5. **Assemble passages → documents (P1, §8).** Group chunks by document, merge
   adjacent chunks into passages, dedupe overlap, rank.
6. **Verdict / abstain (§9).** Derive `answered | weak | none` from the
   calibrated posterior.

No `rerank` step with magic boosts (deleted; calibration replaces it; a learned
cross-encoder reranker may slot in at the v2 calibration seam).

---

## 7. Fusion — correlation-robust, parameter-free (v2, revised after review §6)

**The review correctly rejected probabilistic OR** `P = 1 − (1−P_v)(1−P_t)`: it
assumes the two evidences are independent, but BM25 and the embedding run on the
*same text* and are typically correlated — OR then double-counts shared evidence
and is systematically overconfident. That breaks abstention (the thing we need
calibration for in the first place).

**v3 position: `max(P_v, P_t)` is the conservative *floor*, not the claimed
optimum — the combiner is eval-selected (review §5).**

The review correctly notes the swing risk: OR over-counts correlated evidence
(over-confident), but `max` *ignores corroboration* (under-confident). Example —
`P_v=0.58, P_t=0.55` from two independent-ish signals plausibly means true
relevance `>0.9`, yet `max` returns `0.58`. So `max` is not asserted as optimal.

- **Why `max` is the starting default:** it is **correlation-safe** (cannot
  double-count, valid under any dependence) and **parameter-free** (P3, P4). For
  an abstention-driven, hallucination-averse system, a conservative floor is the
  right *bias when the correlation is still unknown* — it under-claims rather than
  over-claims. It is the safe Stage-A/B ranking combiner.
- **The combiner is itself an eval choice (§12), not a committed constant.** The
  candidates — `max` (floor), probabilistic OR (independence-assuming ceiling), a
  correlation-corrected combiner (copula / measured ρ between modalities), and a
  learned combiner (v2 calibrator era) — are measured on the eval; the one that
  best trades off corroboration-gain vs over-confidence ships. `max` is where we
  *start* because it cannot make abstention worse than the single best modality.
- **Routing still emerges** under `max`: for "PEF-35", `P_t` high, `P_v` low →
  `P ≈ P_t`, the authoritative modality dominates, no classifier.

The honest summary: v1 was over-optimistic (OR), the v2 default is deliberately
under-optimistic (`max`); the *correct* point between them is an empirical
question the eval answers — and until it does, under-claiming is the safer error
for this system.

**Important coupling with §4a:** this fusion is the **Layer-1a load-bearing
core** and must produce a usable *ranking on its own*, before any calibration.
When calibration is off/provisional, Layer 1a ranks by the raw-modality fusion
(e.g. RRF retained as the proven ranking baseline in Stage A/B); when calibration
is on and gated, `max(P_v, P_t)` additionally yields the absolute score for
abstention. Ranking never depends on calibration succeeding.

v2-later upgrades (gated by §12, not assumed): a copula / correlation-corrected
combiner, or a learned mixing term with the learned calibrator — same seam.

---

## 8. Passage assembly (universal unit, P1)

- Group fused candidate chunks by `doc_id`.
- Merge adjacent chunks (consecutive index, same doc) into **passages**, removing
  overlap and de-duplicating propagated table headers (reuse the proven
  `fusion::merge_adjacent_chunks` logic, now internal-only).

**Ranking key — explicit in both regimes (v3, fixes review §7).** The v2 text
said "ranking never depends on calibration" yet defined relevance as "max
calibrated P" — a real contradiction. Resolved by defining the ranking key per
regime, with calibration strictly additive:

| Regime | Chunk score used for ranking | Notes |
|--------|------------------------------|-------|
| **Calibration OFF / provisional / failed gate** (default until Stage D passes) | the **raw fused score** from §7 (modality scores combined; or RRF rank as the Stage-A/B baseline) | ranking is fully defined without calibration — this is the load-bearing path |
| **Calibration ON (gated, Stage D+)** | the **calibrated `P`** | same ordering intent, now on an absolute [0,1] scale so the *same* number also drives the abstention verdict (§9) |

- A passage's score = max of its chunks' scores (under whichever regime is active).
- A document's score = max passage score.
- Rank documents by score; within a document, rank passages by score.
- **Invariant:** the *ordering* is produced by the load-bearing core in both
  regimes; calibration, when on, re-expresses that score on an absolute scale (for
  abstention) but is never *required* to produce a ranking. This is what makes
  "ranking never depends on calibration" literally true.
- **Universality:** a short quote yields one passage ≈ the whole document; the
  long manual yields its relevant section(s). No per-collection branch in the
  mechanism. The "whole document" expansion is served by the existing
  `rag_load_all_chunks` tool.

---

## 9. Verdict & abstention (P2)

Verdict states. When calibration is enabled and gated (§4a Stage D), the band
edges are **derived from the calibration fit** (§5), not hand-set:

| Verdict | Condition |
|---------|-----------|
| `answered` | top calibrated `P` above the relevant-component dominance point |
| `weak` | top `P` in the overlap region (relevant/non-relevant ambiguous) |
| `none` | top `P` below the non-relevant dominance point |
| `unknown` | **calibration off / provisional / failed its gate** — results are still ranked and returned by the robust core, but no absolute-relevance verdict is asserted (P2, P7). Surfaced, never silent (P6). |

The verdict is a first-class response field. A `weak`/`none` verdict is the
signal the small model needs to abstain instead of confabulating; `unknown` tells
the caller the system cannot vouch for absolute relevance here (ranking is still
valid). This four-state design is what makes calibration *additive*: its absence
is an honest `unknown`, not a silent guess and not a failure of the whole search.

---

## 10. Response contract (L3)

Compact, model-facing, no internals by default:

```json
{
  "verdict": "answered | weak | none | unknown",
  "documents": [
    {
      "doc_id": "…",
      "title": "…",
      "source": "…",                // citation anchor (filename / path / url)
      "relevance": 0.0,             // calibrated P, [0,1], comparable across queries
      "passages": [ { "text": "…", "relevance": 0.0 } ]
    }
  ],
  "trimmed": false,                 // true if token budget dropped lower passages (P6)
  "diagnostics": {                  // present only when debug flag set
    "hyde": "applied | unavailable",
    "calibration": "ready | provisional:<reason>",
    "fusion": "…", "raw_scores": [ … ]
  }
}
```

- **No `embedding`, no `_rrf_score`/`_rerank_boost`/`chunk_index`** in the default
  payload — those move behind `diagnostics`.
- **Token-budgeted:** the package targets a configured max size (small-model
  context); least-relevant passages are dropped to fit and `trimmed: true` is set.
- Optional `format: "context_block"` returns a pre-rendered, citation-marked text
  block ready to paste into the model prompt.

---

## 11. Control surface

**MCP façade (LLM-facing) — intent only:**

```
search(
  collection,    // required
  query,         // required
  filter?,       // structured metadata filter (year, customer, …) — genuine intent
  limit?,        // coarse: max documents (default from RetrievalConfig)
  format?,       // "structured" (default) | "context_block"
  debug?         // include diagnostics (default false)
)
```

**Core `RetrievalConfig` (Rust / eval / power-use) — all mechanism:**
candidate widths, HyDE generation count, calibration model handles, token budget,
verdict bands source, fusion variant. Set at the collection level or per call
*in code*, never on the MCP surface.

This is the two-tier split: rich mechanism where rich mechanism belongs
(programmatic), narrow intent where the weak consumer lives.

---

## 12. Validation & grading plan

Because P3 forbids heuristics, quality **must be measured**, not asserted. This
section is the anchor for an external grade.

**Datasets**
- A labeled eval set per collection: `(query, relevant_doc_ids, answerable: bool)`.
  The `answerable=false` queries (corpus genuinely lacks the answer) test
  abstention. Seed from real query logs + synthetic negatives.

**Metrics**
1. **Retrieval quality:** nDCG@k, Recall@k vs the current RRF system (must not
   regress on answerable queries).
2. **Calibration quality (the crux):** reliability diagram + Brier score of
   `P(relevant)` against labels. A reviewer grades the primitive directly here.
3. **Abstention quality:** precision/recall of the `none` verdict on
   `answerable=false` queries. This is the headline small-model-safety metric.
4. **End-to-end answer correctness:** run the actual qwen3.5:9b over the returned
   package; measure answer accuracy + hallucination rate vs the current system.
5. **Token economy:** mean response tokens vs current (expected large drop from
   dropping embeddings + tight passages).

**Gates for shipping:** no regression on (1); (3) abstention precision above an
agreed bar; (4) hallucination rate strictly lower than current.

**Concrete spike specification (v3 — review §4: stop being purely future-tense).**
The calibration/abstention research track is not "someday"; it has a defined first
experiment so the design is falsifiable:

- **Corpora:** the two real production corpora — `rdocs` (quotes, ~22k chunks,
  vllm/bge-m3) and the long manual (gépkönyv). These exercise both the short-doc
  and long-doc regimes.
- **Labels:** 150–200 real queries per corpus from logs, each judged
  `(relevant_doc_ids, answerable)` — half answerable, half deliberately
  unanswerable (to test abstention). Judging by qwen + spot human check.
- **Pass thresholds (pre-registered, so the result is not retrofitted):**
  calibration **Brier ≤ 0.20** and a monotone reliability curve (metric 2);
  abstention **precision ≥ 0.90 at recall ≥ 0.50** on the unanswerable half
  (metric 3); end-to-end **hallucination rate strictly below** the current system
  (metric 4).
- **Outcome rule:** if no calibrator candidate clears metric 2 on a corpus,
  Layer-3 abstention is **not shipped** for that corpus — full stop, not "tune
  until it passes." The track can be abandoned without affecting Stages A/B.

This converts the gates from rhetorical safety-nets into a concrete, falsifiable
experiment with numbers attached.

---

## 13. Performance & caching

- **HyDE latency** is the main cost (an LLM generation per query). Mitigations:
  query-keyed cache (existing embed cache extends to HyDE outputs); `N`
  hypotheticals configurable; batch embedding.
- **Calibration** is fit offline/periodically and persisted with the index;
  per-query calibration is O(candidates) arithmetic.
- **Fusion / assembly** reuse the existing zero-copy drain + merge code paths.

---

## 14. Migration (breaking)

The redesign is a breaking replacement of the `hybrid_search` surface.

- New tool `search` ships alongside; `hybrid_search` enters deprecation, mapping
  its surviving intent params (`collection`, `query`, `filter`, `limit`) onto
  `search`. Mechanism params (`rrf_k`, weights, `mmr_lambda`, `match_scope`,
  `group_by_document`, `max_chunks_per_doc`, …) are **rejected with a pointer to
  the config**, not silently ignored (P6).
- The Rhai `db_hybrid_search` and the existing `rag_load_all_chunks` tools adopt
  the new contract.
- One release of overlap, then `hybrid_search` is removed.

---

## 15. Risks & open questions

*(v2: the previous "calibration is a single point of failure" risk is structurally
removed by §4a — calibration is now additive and gated; its failure degrades only
the abstention verdict to `unknown`, never the ranking.)*

| Risk | Severity | Mitigation |
|------|----------|------------|
| Mixture-fit / any calibrator fails to separate relevant vs non-relevant on a corpus (review §4) | High | **Stage-C spike measures this on real corpora before anything builds on it** (§4a); §12 metric (2) is a hard gate; if it fails, Layer-3 abstention is simply not enabled there and the system stays at the robust core — no regression |
| Score distribution not bimodal (multi-level relevance, mixed doc/query types) (review §4) | High | calibrator is pluggable (§5): non-parametric isotonic / empirical-CDF alternatives; the spike is empowered to reject the mixture |
| Modality correlation makes the combiner overconfident (review §6) | Medium | `max` default is correlation-safe (§7); OR/copula only if a gate shows a win |
| BM25 calibration less stable than cosine | Medium | per-modality independent fits + gates; abstention can be cosine-led where BM25 calibration is unreliable |
| HyDE adds latency / fails / unproven (review §7) | Medium | demoted to eval-gated option (P5); raw-query baseline always works; caching; surfaced if unavailable (P6) |
| Cold-start: no calibration data for a new collection | Medium | `Provisional` status surfaced (P6); system runs at robust core until samples accrue |
| Residual retrieval complexity not solved by the unit fix (review §1): query ambiguity, domain drift, multi-hop, ranking instability | Medium | acknowledged as out of scope of this redesign; the unit fix addresses the *parameter surface*, not all of retrieval — these remain open and are tracked separately |

**Open questions for the reviewer / owner:**
- Sampling policy for fitting the mixture (which executed queries feed it; refresh
  cadence).
- Where verdict bands are persisted and how often re-derived.
- Whether `format: context_block` should be the default for the qwen path.

---

## 16. Codebase placement

| Component | Proposed location |
|-----------|-------------------|
| Calibrator trait + mixture impl | `ironbase-core/src/retrieval/calibration.rs` (new) |
| Orchestrator (L2) | `mcp-server/src/tools/retrieval/orchestrator.rs` (new; absorbs hybrid.rs) |
| Fusion (parameter-free) | `mcp-server/src/tools/retrieval/fusion.rs` (refactor of current `fusion.rs`) |
| Passage assembly | reuse `merge_adjacent_chunks`, move under `retrieval/` |
| HyDE | `mcp-server/src/tools/retrieval/hyde.rs` (new) |
| Response contract | `mcp-server/src/tools/retrieval/contract.rs` (new) |
| MCP façade `search` | `mcp-server/src/tools/retrieval/mod.rs` (new) + `definitions/` |
| `RetrievalConfig` | `ironbase-core/src/retrieval/config.rs` (new) |
| Eval harness | `mcp-server/tests/retrieval_eval/` (new) |

---

## Appendix — principle → mechanism traceability

| Principle | Mechanism that enforces it |
|-----------|----------------------------|
| P1 universal passage unit | §8 assembly; chunk never public |
| P2 calibration is additive, not foundational | §4a layers; §9 verdict degrades to `unknown` if calibration absent |
| P3 no unvalidated magic constants | §5 spike-gated calibrator; §7 parameter-free `max`; §9 gated bands; §12 decides |
| P4 intent in / mechanism owned | §11 two-tier surface |
| P5 HyDE eval-gated option | §4a Stage E; §6 step 1; §12 vs raw-query baseline |
| P6 no silent fallback | §5 Provisional, §6 hyde flag, §10 trimmed, §14 rejected params |
| P7 graceful degradation / staged | §4a layers + staged delivery A–E |
