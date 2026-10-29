#!/usr/bin/env python3
"""Lane comparison for the `search` pipeline: hybrid vs BM25 vs vector, plus
fusion ablations, measured so the comparison is fair (P3: measure, don't assert).

Why not just compare the label set's numbers lane by lane: the June label set
was judged over a pool of the hybrid `search` lane and a BM25 lane only
(build_labels.py `gather_candidates`), and stores the judged-relevant ids alone
(`candidates_judged` is a count). A document only a vector lane returns was
never judged, so it scores as non-relevant: the vector lane is measured
against labels biased away from it, and the other lanes towards themselves.
And the MCP `embed_text` tool embeds as a *document* (document prefix), while
`search` embeds the query with `embed_query` (query prefix): a vector lane
built on `embed_text` is not the vector half of `search`.

What this does instead:

1. Every configuration runs through the SAME retrieval pipeline `search` uses
   (`db_hybrid_search` in a script: `hybrid::retrieve_and_fuse` +
   `build_doc_groups`, query embedded by `embed_query`), differing only in its
   weights / fusion options. The `mirror` configuration passes `search`'s own
   defaults: `search` must return an order-preserving subsequence of its
   documents (the response layer's context budget may leave documents out, see
   docs/SEARCH_CONTEXT_BUDGET_BUG.md), else the run stops (the configurations
   would not measure `search`'s pipeline). The lanes and ablations are compared
   with `mirror`, the engine; `search` vs `mirror` measures the response layer.
2. The judging pool of each query is the union of every configuration's
   top-k documents and the June relevant ids. Every pooled pair is judged
   again by one judge (the June prompt, temperature 0) over the whole document
   text (its chunks in order), not over a lane's passage. Judgments are cached
   in a JSONL file, so a rerun only judges new pairs and the run is
   reproducible.
3. Metrics per query (nDCG@5/@10, Recall@5/@10, doc level, binary); paired
   differences with a bootstrap 95% CI, a paired sign-flip randomization test
   and Holm correction per bucket and metric; wins/ties/losses.
4. Buckets are fixed before measuring, from the June labels: "specific" if a
   query has at most `--specific-max-relevant` relevant documents (default 4:
   18 of the 62 answerable queries of labeled_rdocs_curated_v2.json),
   "browse" otherwise.

Recall is relative to the pool (every relevant document any configuration or
the June pool found). The scripts run with the `script_exec` tool, which only
a loopback caller may use: run this on the server host.

Usage:
  python3 measure_lanes.py --selftest                     # offline checks
  ANTHROPIC_API_KEY=... python3 measure_lanes.py \\
      --mcp http://localhost:8080/mcp --out-dir lane_eval
"""
import argparse
import csv
import hashlib
import itertools
import json
import math
import os
import random
import re
import sys
import time
import urllib.error
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from build_labels import _judge_prompt, _parse_yesno  # noqa: E402  (same judge as June)
from run_eval import mcp_init, mcp_tool, ndcg_at_k, recall_at_k  # noqa: E402

MCP_DEFAULT = "http://localhost:8080/mcp"
LABELS_DEFAULT = os.path.join(os.path.dirname(__file__), "labeled_rdocs_curated_v2.json")
ANTHROPIC_URL = os.environ.get("ANTHROPIC_BASE_URL", "https://api.anthropic.com").rstrip("/") + "/v1/messages"
JUDGE_MODEL_DEFAULT = "claude-haiku-4-5-20251001"  # the June judge
SEED = 20261028
METRICS = ["ndcg@5", "ndcg@10", "recall@5", "recall@10"]
PRIMARY = "ndcg@5"

# `search`'s own pipeline settings (tools/retrieval/mod.rs handle_search +
# HybridSearchParams defaults in tools/params.rs). The script path injects other
# defaults (deduplicate, mmr_lambda), so every option is passed explicitly.
SEARCH_DEFAULTS = {
    "group_by_document": True,
    "rrf_k": 20.0,
    "rerank": True,
    "deduplicate": False,
    "mmr_lambda": 0.7,
    "merge_chunks": True,
}

# name -> (options over SEARCH_DEFAULTS, description). `search` must return a
# subsequence of `mirror`.
CONFIGS = {
    "mirror": ({}, "search's pipeline and defaults, without the response budget"),
    "bm25": ({"vector_weight": 0.0, "fulltext_weight": 1.0}, "lexical lane only"),
    "vector": ({"vector_weight": 1.0, "fulltext_weight": 0.0}, "vector lane only"),
    "no_rerank": ({"rerank": False}, "hybrid without the rerank boosts"),
    "keyword": ({"search_mode": "keyword"}, "hybrid, weights 0.2/0.8"),
    "rrf_k60": ({"rrf_k": 60.0}, "hybrid, rrf_k 60"),
}
# Lanes whose other weight is 0: a document only the zero-weight lane returned
# scores 0 and must not fill the top-k
SINGLE_LANE = {"bm25", "vector"}

RHAI = """
let r = db_hybrid_search(params.collection, params.query, params.options);
if type_of(r) == "string" { throw r; }
let out = [];
for g in r {
    if g.best_score > params.min_score { out.push(g.doc_id); }
}
out
"""


# --- retrieval --------------------------------------------------------------
def run_search(mcp, collection, query, k):
    sr = mcp_tool(mcp, "search", {"collection": collection, "query": query, "limit": k})
    return [d["doc_id"] for d in sr.get("documents", []) if d.get("doc_id")]


def run_config(mcp, name, collection, query, k):
    options = dict(SEARCH_DEFAULTS)
    options.update(CONFIGS[name][0])
    options["limit"] = k
    params = {"collection": collection, "query": query, "options": options,
              "min_score": 0.0 if name in SINGLE_LANE else -1.0}
    r = mcp_tool(mcp, "script_exec", {"code": RHAI, "params": params}, timeout=120)
    result = r.get("result")
    if not isinstance(result, list):
        raise RuntimeError(f"script returned {result!r} (logs: {r.get('logs')})")
    return [str(d) for d in result]


def document_text(mcp, collection, doc_id, text_field, max_chars, cache):
    """Title and the document's text (its chunks in order), for the judge"""
    if doc_id in cache:
        return cache[doc_id]
    r = mcp_tool(mcp, "find", {"collection": collection, "filter": {"doc_id": doc_id},
                               "sort": {"chunk_index": 1}, "limit": 200,
                               "projection": {"title": 1, text_field: 1, "chunk_index": 1}})
    docs = r.get("documents", [])
    title = next((d.get("title") for d in docs if d.get("title")), "")
    text = "\n".join(str(d.get(text_field) or "") for d in docs)
    cache[doc_id] = (title, text[:max_chars], len(docs))
    return cache[doc_id]


# --- judging ----------------------------------------------------------------
def judge_key(model, query, doc_id):
    return hashlib.sha256(f"{model}\x00{query}\x00{doc_id}".encode()).hexdigest()


BATCHES_URL = os.environ.get("ANTHROPIC_BASE_URL", "https://api.anthropic.com").rstrip("/") + "/v1/messages/batches"
# The Batches API runs the same requests asynchronously at 50% of standard
# prices (most batches finish within an hour, 24 h maximum). Worth it for a
# large judging pool; `--judge-mode sync` stays the default for small reruns.
BATCH_MAX_REQUESTS = 100_000


def _anthropic_request(url, api_key, body=None, method="GET", timeout=180):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(
        url, data=data, method=method,
        headers={"x-api-key": api_key, "anthropic-version": "2023-06-01",
                 "content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return r.read().decode()


def anthropic_judge_batch(api_key, model, prompts, poll_seconds=30, max_hours=24,
                          log=sys.stderr):
    """Judge `prompts` ({custom_id: prompt}) through the Batches API.

    Returns {custom_id: verdict}; a custom_id missing from the result file or
    whose request errored maps to None (no verdict), like the sync path.
    """
    if len(prompts) > BATCH_MAX_REQUESTS:
        raise ValueError(f"{len(prompts)} requests exceeds the {BATCH_MAX_REQUESTS} batch limit")
    requests_body = [
        {"custom_id": cid,
         "params": {"model": model, "max_tokens": 8, "temperature": 0,
                    "messages": [{"role": "user", "content": prompt}]}}
        for cid, prompt in prompts.items()
    ]
    batch = json.loads(_anthropic_request(BATCHES_URL, api_key,
                                          {"requests": requests_body}, "POST"))
    batch_id = batch["id"]
    print(f"batch {batch_id}: {len(requests_body)} requests submitted (50% pricing)",
          file=log, flush=True)

    deadline = time.time() + max_hours * 3600
    while True:
        status = json.loads(_anthropic_request(f"{BATCHES_URL}/{batch_id}", api_key))
        if status.get("processing_status") == "ended":
            break
        if time.time() > deadline:
            raise TimeoutError(f"batch {batch_id} still {status.get('processing_status')} "
                               f"after {max_hours} h")
        counts = status.get("request_counts", {})
        print(f"  {status.get('processing_status')}: "
              f"{counts.get('succeeded', 0)} succeeded, "
              f"{counts.get('processing', 0)} processing", file=log, flush=True)
        time.sleep(poll_seconds)

    verdicts = {}
    body = _anthropic_request(f"{BATCHES_URL}/{batch_id}/results", api_key, timeout=600)
    for line in body.splitlines():
        if not line.strip():
            continue
        row = json.loads(line)
        outcome = row.get("result", {})
        if outcome.get("type") != "succeeded":
            verdicts[row["custom_id"]] = None
            continue
        text = "".join(b.get("text", "") for b in outcome["message"].get("content", [])
                       if b.get("type") == "text")
        verdicts[row["custom_id"]] = _parse_yesno(text)
    for cid in prompts:
        verdicts.setdefault(cid, None)
    return verdicts


def anthropic_judge(api_key, model, prompt, retries=4):
    body = {"model": model, "max_tokens": 8, "temperature": 0,
            "messages": [{"role": "user", "content": prompt}]}
    for attempt in range(retries):
        req = urllib.request.Request(
            ANTHROPIC_URL, data=json.dumps(body).encode(),
            headers={"x-api-key": api_key, "anthropic-version": "2023-06-01",
                     "content-type": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=60) as r:
                resp = json.loads(r.read().decode())
            text = "".join(b.get("text", "") for b in resp.get("content", [])
                           if b.get("type") == "text")
            return _parse_yesno(text)
        except urllib.error.HTTPError as e:
            if e.code in (429, 500, 502, 503, 529) and attempt + 1 < retries:
                time.sleep(2 ** attempt * 5)
                continue
            print(f"    [judge HTTP {e.code}: {e.read().decode()[:160]}]", file=sys.stderr)
            return None
        except (urllib.error.URLError, TimeoutError, OSError, ValueError) as e:
            if attempt + 1 < retries:
                time.sleep(2 ** attempt * 5)
                continue
            print(f"    [judge error: {e}]", file=sys.stderr)
            return None
    return None


def load_judgments(path):
    out = {}
    if os.path.exists(path):
        with open(path, encoding="utf-8") as f:
            for line in f:
                if line.strip():
                    j = json.loads(line)
                    out[j["key"]] = j["relevant"]
    return out


def is_subsequence(short, long):
    """True if `short` is `long` with some items left out, order kept"""
    it = iter(long)
    return all(any(x == y for y in it) for x in short)


# --- statistics -------------------------------------------------------------
def mean(xs):
    return sum(xs) / len(xs) if xs else float("nan")


def bootstrap_ci(diffs, b=10000, seed=SEED, alpha=0.05):
    """Percentile CI of the mean paired difference (queries resampled)"""
    rng = random.Random(seed)
    n = len(diffs)
    means = sorted(mean([diffs[rng.randrange(n)] for _ in range(n)]) for _ in range(b))
    lo = means[int(math.floor(alpha / 2 * b))]
    hi = means[min(b - 1, int(math.ceil((1 - alpha / 2) * b)) - 1)]
    return lo, hi


def sign_flip_p(diffs, mc=100000, seed=SEED):
    """Two-sided paired randomization test of mean difference 0: exact over all
    sign assignments up to 16 non-zero differences, Monte Carlo above"""
    d = [x for x in diffs if x != 0]
    if not d:
        return 1.0
    obs = abs(sum(d))
    eps = 1e-12
    if len(d) <= 16:
        total = hits = 0
        for signs in itertools.product((1, -1), repeat=len(d)):
            total += 1
            hits += abs(sum(s * x for s, x in zip(signs, d))) >= obs - eps
        return hits / total
    rng = random.Random(seed)
    hits = sum(abs(sum(x if rng.random() < 0.5 else -x for x in d)) >= obs - eps
               for _ in range(mc))
    return (hits + 1) / (mc + 1)


def holm(pvalues):
    """Holm-Bonferroni adjusted p-values, in the input order"""
    m = len(pvalues)
    order = sorted(range(m), key=lambda i: pvalues[i])
    adjusted = [0.0] * m
    running = 0.0
    for rank, i in enumerate(order):
        running = max(running, min(1.0, (m - rank) * pvalues[i]))
        adjusted[i] = running
    return adjusted


def metrics_for(retrieved, relevant):
    return {"ndcg@5": ndcg_at_k(retrieved, relevant, 5),
            "ndcg@10": ndcg_at_k(retrieved, relevant, 10),
            "recall@5": recall_at_k(retrieved, relevant, 5),
            "recall@10": recall_at_k(retrieved, relevant, 10)}


def compare(per_query, base, other, metric):
    diffs = [q["metrics"][other][metric] - q["metrics"][base][metric] for q in per_query]
    lo, hi = bootstrap_ci(diffs)
    return {"base": base, "other": other, "metric": metric, "n": len(diffs),
            "delta": mean(diffs), "ci_lo": lo, "ci_hi": hi, "p": sign_flip_p(diffs),
            "wins": sum(d > 0 for d in diffs), "ties": sum(d == 0 for d in diffs),
            "losses": sum(d < 0 for d in diffs)}


# --- selftest ----------------------------------------------------------------
def selftest():
    ok = True

    def check(name, cond):
        nonlocal ok
        ok = ok and cond
        print(f"  [{'OK' if cond else 'FAIL'}] {name}")

    # Exact sign-flip: d = [1, 1, 1] → only all-+ and all-- reach |3|: 2/8
    check("sign-flip exact", abs(sign_flip_p([1, 1, 1]) - 0.25) < 1e-12)
    check("sign-flip zeros only", sign_flip_p([0, 0]) == 1.0)
    # Symmetric differences: p = 1
    check("sign-flip symmetric", sign_flip_p([1, -1]) == 1.0)
    # Monte Carlo agrees with exact on 17 equal differences: 2 / 2^17
    p = sign_flip_p([1] * 17, mc=20000)
    check("sign-flip Monte Carlo floor", p <= 3 / 20001)
    # Bootstrap of a constant difference is that constant
    lo, hi = bootstrap_ci([0.2] * 10, b=500)
    check("bootstrap constant", abs(lo - 0.2) < 1e-12 and abs(hi - 0.2) < 1e-12)
    lo, hi = bootstrap_ci([0.0, 1.0] * 20, b=2000)
    check("bootstrap covers the mean", lo < 0.5 < hi)
    # Holm: [0.01, 0.04, 0.03] → sorted 0.01*3, max(0.03*2), max(0.04*1)
    adj = holm([0.01, 0.04, 0.03])
    check("holm", all(abs(a - b) < 1e-12 for a, b in zip(adj, [0.03, 0.06, 0.06])))
    m = metrics_for(["a", "x", "b"], {"a", "b"})
    check("metrics", abs(m["ndcg@5"] - 0.919_721_4) < 1e-6 and m["recall@5"] == 1.0)
    check("judge key stable", judge_key("m", "q", "d") == judge_key("m", "q", "d"))
    check("subsequence", is_subsequence(["a", "c"], ["a", "b", "c"])
          and is_subsequence([], ["a"]) and not is_subsequence(["c", "a"], ["a", "b", "c"])
          and not is_subsequence(["d"], ["a"]))
    print("SELFTEST:", "PASS" if ok else "FAIL")
    return 0 if ok else 1


# --- run --------------------------------------------------------------------
def bucket_of(lq, specific_max):
    return "specific" if len(lq["relevant_doc_ids"]) <= specific_max else "browse"


def fmt(x):
    return f"{x:.4f}"


def run(args):
    with open(args.labels, encoding="utf-8") as f:
        labels = [lq for lq in json.load(f)
                  if lq.get("answerable", True) and lq.get("relevant_doc_ids")]
    os.makedirs(args.out_dir, exist_ok=True)
    api_key = os.environ.get("ANTHROPIC_API_KEY")
    if not api_key and not args.no_judge:
        print("ERROR: ANTHROPIC_API_KEY is not set (or pass --no-judge to reuse the "
              "cached judgments only).", file=sys.stderr)
        return 2
    mcp_init(args.mcp)
    configs = list(CONFIGS)
    k = args.k

    # 1. Retrieval, and the mirror check
    runs = []
    mismatches = []
    for i, lq in enumerate(labels, 1):
        coll = lq.get("collection", "rdocs")
        got = {"search": run_search(args.mcp, coll, lq["query"], k)}
        for name in configs:
            got[name] = run_config(args.mcp, name, coll, lq["query"], k)
        if not is_subsequence(got["search"], got["mirror"]):
            mismatches.append((lq["query"], got["search"], got["mirror"]))
        runs.append(got)
        print(f"[{i}/{len(labels)}] retrieved :: {lq['query']}", file=sys.stderr)
    if mismatches:
        print(f"\nSTOP: `search` is not a subsequence of the mirror configuration on "
              f"{len(mismatches)}/{len(labels)} queries; the configurations would not "
              f"measure `search`'s pipeline. First: {mismatches[0]}", file=sys.stderr)
        return 3

    # 2. Pool and judgments
    judgments_path = os.path.join(args.out_dir, "judgments.jsonl")
    judged = load_judgments(judgments_path)
    texts = {}
    new = unknown = 0
    pending = []  # batch mode: (key, query, doc_id, chunks, prompt)
    with open(judgments_path, "a", encoding="utf-8") as jf:

        def record(key, query, doc_id, chunks, verdict):
            nonlocal new, unknown
            judged[key] = verdict
            new += 1
            unknown += verdict is None
            jf.write(json.dumps({"key": key, "model": args.judge_model,
                                 "query": query, "doc_id": doc_id,
                                 "chunks": chunks, "relevant": verdict},
                                ensure_ascii=False) + "\n")
            jf.flush()

        for lq, got in zip(labels, runs):
            coll = lq.get("collection", "rdocs")
            pool = set(lq["relevant_doc_ids"])
            for name in ["search"] + configs:
                pool.update(got[name][:k])
            for doc_id in sorted(pool):
                key = judge_key(args.judge_model, lq["query"], doc_id)
                if key in judged:
                    continue
                if args.no_judge:
                    judged[key] = None
                    unknown += 1
                    continue
                title, text, chunks = document_text(args.mcp, coll, doc_id, args.text_field,
                                                    args.doc_chars, texts)
                prompt = _judge_prompt(lq["query"], title, text) if chunks else None
                if args.judge_mode == "batch":
                    pending.append((key, lq["query"], doc_id, chunks, prompt))
                    continue
                # a document that no longer exists needs no judge call
                verdict = anthropic_judge(api_key, args.judge_model, prompt) if chunks else False
                record(key, lq["query"], doc_id, chunks, verdict)

        if pending:
            askable = {key: prompt for key, _, _, chunks, prompt in pending if chunks}
            verdicts = (anthropic_judge_batch(api_key, args.judge_model, askable)
                        if askable else {})
            for key, query, doc_id, chunks, _ in pending:
                record(key, query, doc_id, chunks,
                       verdicts.get(key) if chunks else False)
    print(f"judged {new} new pairs ({unknown} without a verdict)", file=sys.stderr)

    # 3. Per-query metrics: re-judged pool labels, and the June labels for contrast
    per_query = []
    june_agree = june_total = 0
    for lq, got in zip(labels, runs):
        pool = set(lq["relevant_doc_ids"])
        for name in ["search"] + configs:
            pool.update(got[name][:k])
        verdict = {d: judged.get(judge_key(args.judge_model, lq["query"], d)) for d in pool}
        relevant = {d for d, v in verdict.items() if v}
        for d in lq["relevant_doc_ids"]:
            june_total += 1
            june_agree += bool(verdict.get(d))
        row = {"query": lq["query"], "bucket": bucket_of(lq, args.specific_max_relevant),
               "relevant": len(relevant), "june_relevant": len(lq["relevant_doc_ids"]),
               "unjudged": sum(v is None for v in verdict.values()),
               "metrics": {}, "june_metrics": {}, "top": {}}
        for name in ["search"] + configs:
            row["metrics"][name] = metrics_for(got[name], relevant)
            row["june_metrics"][name] = metrics_for(got[name], set(lq["relevant_doc_ids"]))
            row["top"][name] = got[name][:k]
        per_query.append(row)

    # 4. Report
    comparisons = [("mirror", "bm25"), ("mirror", "vector"), ("mirror", "no_rerank"),
                   ("mirror", "keyword"), ("mirror", "rrf_k60"), ("mirror", "search")]
    shortened = sum(len(got["search"]) < len(got["mirror"]) for got in runs)
    lines = ["# Lane comparison (`search` pipeline)", "",
             f"- labels: `{os.path.basename(args.labels)}`, {len(labels)} answerable queries; "
             f"buckets fixed from the June labels (specific: <= {args.specific_max_relevant} "
             f"relevant)",
             f"- configurations: " + "; ".join(f"`{n}`: {d}" for n, (_, d) in CONFIGS.items()),
             f"- `search` returned a subsequence of `mirror` on every query; fewer documents "
             f"on {shortened}/{len(labels)} (`search` vs `mirror` is the response layer's "
             f"effect, its Recall@10 is over the documents it returned)",
             f"- judge: `{args.judge_model}`, temperature 0, June prompt, whole document "
             f"(<= {args.doc_chars} chars); pool = every configuration's top-{k} + June "
             f"relevant ids",
             f"- June relevant ids judged relevant again: {june_agree}/{june_total} "
             f"({june_agree / june_total:.1%})" if june_total else "",
             f"- pairs without a verdict (excluded from relevant): "
             f"{sum(q['unjudged'] for q in per_query)}",
             "- Recall is relative to the pool; CI: paired bootstrap 95% (10,000); p: paired "
             "sign-flip randomization, Holm-adjusted within bucket and metric", ""]
    csv_rows = []
    _nores = [q for q in per_query if q["relevant"] == 0]
    if _nores:
        lines += [f"## unanswerable-in-pool (n={len(_nores)})", "",
                  "A bíró a pool egyetlen dokumentumát sem ítélte relevánsnak: minden "
                  "konfiguráció nDCG-je definíció szerint 0, a rangsor nem mérhető. "
                  "Ezek abstention-esetek (a `verdict` ma mindig `unknown`), ezért a "
                  "fenti bucketekből ki vannak véve.", "",
                  "| kérdés | bucket | a search visszaadott dokumentumai |",
                  "|---|---|---|"]
        for q in _nores:
            lines.append(f"| {q['query']} | {q['bucket']} | {len(q['top']['search'])} |")
        lines.append("")
    # A query whose pool holds no relevant document scores nDCG 0 for every
    # configuration: the retriever cannot win, so averaging it in understates
    # the quality floor (5 of the 18 specific queries on the June set). Those
    # queries are an abstention problem, not a ranking one, and are reported
    # separately under "unanswerable-in-pool".
    for bucket in ["specific", "browse", "all"]:
        qs = [q for q in per_query
              if (bucket == "all" or q["bucket"] == bucket) and q["relevant"] > 0]
        if not qs:
            continue
        lines += [f"## {bucket} (n={len(qs)})", "",
                  "| configuration | " + " | ".join(METRICS) + " | (June labels) " + PRIMARY + " |",
                  "|---|" + "---|" * (len(METRICS) + 1)]
        for name in ["search"] + configs:
            vals = [mean([q["metrics"][name][m] for q in qs]) for m in METRICS]
            june = mean([q["june_metrics"][name][PRIMARY] for q in qs])
            lines.append(f"| {name} | " + " | ".join(fmt(v) for v in vals) + f" | {fmt(june)} |")
        lines += ["", "| comparison | metric | Δ (other − mirror) | 95% CI | p | p (Holm) | W/T/L |",
                  "|---|---|---|---|---|---|---|"]
        for metric in METRICS:
            rows = [compare(qs, a, b, metric) for a, b in comparisons]
            for r, p_adj in zip(rows, holm([r["p"] for r in rows])):
                r["p_holm"] = p_adj
                r["bucket"] = bucket
                csv_rows.append(r)
                lines.append(f"| {r['other']} vs mirror | {metric} | {r['delta']:+.4f} | "
                             f"[{r['ci_lo']:+.4f}, {r['ci_hi']:+.4f}] | {r['p']:.4f} | "
                             f"{r['p_holm']:.4f} | {r['wins']}/{r['ties']}/{r['losses']} |")
        lines.append("")

    code_re = re.compile(r"\b(?=\w*\d)[\w-]{3,}\b")
    lines += ["## specific: per query, mirror vs bm25 (" + PRIMARY + ")", "",
              "| query | code-like token | mirror | bm25 | Δ |", "|---|---|---|---|---|"]
    for q in sorted((q for q in per_query if q["bucket"] == "specific"),
                    key=lambda q: q["metrics"]["bm25"][PRIMARY] - q["metrics"]["mirror"][PRIMARY]):
        s, b = q["metrics"]["mirror"][PRIMARY], q["metrics"]["bm25"][PRIMARY]
        lines.append(f"| {q['query']} | {'yes' if code_re.search(q['query']) else 'no'} | "
                     f"{fmt(s)} | {fmt(b)} | {b - s:+.4f} |")

    with open(os.path.join(args.out_dir, "report.md"), "w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")
    with open(os.path.join(args.out_dir, "per_query.json"), "w", encoding="utf-8") as f:
        json.dump(per_query, f, ensure_ascii=False, indent=1)
    with open(os.path.join(args.out_dir, "comparisons.csv"), "w", newline="",
              encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=["bucket", "metric", "base", "other", "n", "delta",
                                          "ci_lo", "ci_hi", "p", "p_holm", "wins", "ties",
                                          "losses"])
        w.writeheader()
        w.writerows(csv_rows)
    print("\n".join(lines))
    print(f"\nwritten: {args.out_dir}/report.md, per_query.json, comparisons.csv, "
          f"judgments.jsonl", file=sys.stderr)
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--mcp", default=MCP_DEFAULT)
    ap.add_argument("--labels", default=LABELS_DEFAULT)
    ap.add_argument("--out-dir", default="lane_eval")
    ap.add_argument("--k", type=int, default=10, help="documents per configuration and pool depth")
    ap.add_argument("--specific-max-relevant", type=int, default=4)
    ap.add_argument("--judge-model", default=JUDGE_MODEL_DEFAULT)
    ap.add_argument("--text-field", default="content_text")
    ap.add_argument("--doc-chars", type=int, default=6000)
    ap.add_argument("--judge-mode", choices=["sync", "batch"], default="sync",
                    help="batch: Batches API, 50%% cheaper, up to 24 h latency")
    ap.add_argument("--no-judge", action="store_true",
                    help="use cached judgments only; unjudged pairs count as unknown")
    ap.add_argument("--selftest", action="store_true", help="offline checks, no server")
    args = ap.parse_args()
    if args.selftest:
        sys.exit(selftest())
    sys.exit(run(args))


if __name__ == "__main__":
    main()
