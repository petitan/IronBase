#!/usr/bin/env python3
"""Semi-automated label builder for the retrieval eval harness (redesign §12,
Stage B/C). See docs/HYBRID_RETRIEVAL_REDESIGN.md.

Pipeline (READ-ONLY against production; relevance judgment via Anthropic API):
  1. Pull real user queries from the `tudastar_log` collection (intent=search),
     dedup + sample.
  2. For each query, build a DE-BIASED judging pool over `rdocs`: union of the
     unified `search` lane (dense+fusion) and a pure BM25 `fulltext_search` lane,
     deduped by doc_id (see gather_candidates — removes single-lane/top-k pool
     bias so coverage misses become labelable).
  3. Judge each candidate's relevance with a STRONGER-than-consumer LLM (default
     Anthropic Haiku, so the judge is not the thing being evaluated, and the local
     ollama serving production `search` embeddings is never loaded by labeling).
  4. Emit labeled.json in the harness format:
     [{query, collection, relevant_doc_ids, answerable, source_rating}]

The output is a STARTING POINT — spot-check it (P3: measure, don't assert). It
feeds both the Stage-C calibration spike and the Stage-B nDCG/Recall gate.

Usage:
  python3 build_labels.py --num-queries 150 --search-k 25 --fulltext-k 25 \
      --mcp http://localhost:8080/mcp --out labeled_rdocs.json
"""
import argparse
import json
import os
import re
import sys
import time
import urllib.request
import urllib.error

ANTHROPIC_URL = "https://api.anthropic.com/v1/messages"

MCP_DEFAULT = "http://192.168.2.211:8080/mcp"
OLLAMA_DEFAULT = "http://localhost:11434/api/generate"
COLLECTION = "rdocs"
LOG_COLLECTION = "tudastar_log"


def _post(url, payload, timeout):
    data = json.dumps(payload).encode()
    req = urllib.request.Request(url, data=data, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.loads(r.read().decode())


def mcp_init(mcp):
    _post(mcp, {"jsonrpc": "2.0", "id": 0, "method": "initialize",
                "params": {"protocolVersion": "2024-11-05", "capabilities": {},
                           "clientInfo": {"name": "label-builder", "version": "1"}}}, 15)


def mcp_tool(mcp, name, args, timeout=60):
    resp = _post(mcp, {"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                       "params": {"name": name, "arguments": args}}, timeout)
    res = resp.get("result", {})
    if res.get("isError"):
        raise RuntimeError(res.get("content", [{}])[0].get("text", "tool error"))
    return json.loads(res["content"][0]["text"])


def pull_queries(mcp, n):
    """Distinct real search queries from tudastar_log, with their last rating."""
    r = mcp_tool(mcp, "find", {"collection": LOG_COLLECTION, "filter": {"intent": "search"},
                               "limit": 2000, "projection": {"query": 1, "rating": 1, "_id": 0}})
    seen = {}
    for d in r.get("documents", []):
        q = (d.get("query") or "").strip()
        if len(q) >= 3 and q.lower() not in seen:
            seen[q.lower()] = {"query": q, "rating": d.get("rating")}
    out = list(seen.values())
    return out[:n] if n else out


_THINK_RE = re.compile(r"<think>.*?</think>", re.S | re.I)


def _parse_yesno(raw):
    """Robust YES/NO parse: strip reasoning-model <think> blocks, then decide.
    Returns True/False, or None if the model gave no clear verdict."""
    text = _THINK_RE.sub("", raw or "").strip().upper()
    if not text:
        return None
    if text.startswith("YES"):
        return True
    if text.startswith("NO"):
        return False
    has_yes, has_no = "YES" in text, "NO" in text
    if has_yes and not has_no:
        return True
    if has_no and not has_yes:
        return False
    return None  # ambiguous → unknown (surfaced, not guessed)


def _judge_prompt(query, title, excerpt):
    # STRICT bar: with a wide two-lane candidate pool over a redundant corpus
    # (many near-duplicate quote templates), a permissive "does it help?" judge
    # marks ~90% relevant — which both caps Recall@k (too many relevant to fit in
    # top-k) and saturates nDCG (any ordering scores high). Require a DIRECT/PRIMARY
    # answer so relevant sets stay small and the metric discriminates.
    return (
        "You judge document relevance for a search system, using a STRICT bar. The "
        "document may be in Hungarian. Reply with EXACTLY one word: YES or NO.\n\n"
        f"Query: {query}\n"
        f"Document title: {title}\n"
        f"Document excerpt: {excerpt}\n\n"
        "Answer YES only if this document is a DIRECT, PRIMARY answer to the query: "
        "its main subject is exactly what the query asks about, or it contains the "
        "specific item/figure requested. Answer NO if it only mentions the query "
        "terms in passing, is merely about the same general category, or is a "
        "generic/near-duplicate template that does not specifically match. When "
        "unsure, answer NO. YES or NO:"
    )


def anthropic_judge(api_key, model, query, title, excerpt, timeout=30):
    """Relevance judge via the Anthropic API (default backend). Does NOT touch the
    local ollama — that serves production `search` embeddings and must not be
    loaded by labeling (a prior local-judge run tripped its circuit breaker)."""
    body = {
        "model": model,
        "max_tokens": 8,
        "messages": [{"role": "user", "content": _judge_prompt(query, title, excerpt)}],
    }
    req = urllib.request.Request(
        ANTHROPIC_URL, data=json.dumps(body).encode(),
        headers={"x-api-key": api_key, "anthropic-version": "2023-06-01",
                 "content-type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            resp = json.loads(r.read().decode())
        text = "".join(b.get("text", "") for b in resp.get("content", [])
                       if b.get("type") == "text")
        return _parse_yesno(text)
    except (urllib.error.URLError, TimeoutError, OSError, ValueError) as e:
        detail = ""
        if isinstance(e, urllib.error.HTTPError):
            try:
                detail = " " + e.read().decode()[:160]
            except Exception:
                pass
        print(f"    [anthropic judge error: {e}{detail}]", file=sys.stderr)
        return None


def gather_candidates(mcp, query, search_k, ft_k, excerpt_chars):
    """Union the judging pool across two complementary retrieval lanes so the judge
    sees candidates that the unified `search` alone would bury — removing the
    single-lane / top-k POOL BIAS that makes coverage misses structurally
    unlabelable (a relevant doc the system never surfaces can never be marked
    relevant, so recall against such labels is upward-biased):
      • `search`        — dense+fusion lane (intent-only, server-owned weights)
      • `fulltext_search` on content_text — pure BM25 keyword lane (surfaces exact
        codes like PEF-35 that the fusion lane drowns among near-dup templates)
    Dedup by doc_id; first (higher-ranked) occurrence wins the excerpt. Returns
    [{doc_id, title, excerpt}], at most search_k + ft_k unique parent docs."""
    cand = {}  # doc_id -> candidate; dict preserves first-seen (≈ rank) order
    # Lane 1: unified search — passages excerpt.
    try:
        sr = mcp_tool(mcp, "search", {"collection": COLLECTION, "query": query, "limit": search_k})
        for d in sr.get("documents", []):
            did = d.get("doc_id")
            if not did or did in cand:
                continue
            excerpt = " ".join(p.get("text", "") for p in d.get("passages", []))[:excerpt_chars]
            cand[did] = {"doc_id": did, "title": d.get("title") or "", "excerpt": excerpt}
    except Exception as e:
        print(f"    [search lane error: {e}]", file=sys.stderr)
    # Lane 2: pure BM25 keyword — content_text excerpt. Flat hit shape (v1.0.501+),
    # OR mode for broad pool recall (we want every token-bearing doc judged).
    try:
        fr = mcp_tool(mcp, "fulltext_search",
                      {"collection": COLLECTION, "query": query, "field": "content_text",
                       "limit": ft_k, "mode": "or"})
        for d in fr.get("results", []):
            did = d.get("doc_id")
            if not did or did in cand:
                continue
            cand[did] = {"doc_id": did, "title": d.get("title") or "",
                         "excerpt": str(d.get("content_text") or "")[:excerpt_chars]}
    except Exception as e:
        print(f"    [fulltext lane error: {e}]", file=sys.stderr)
    return list(cand.values())


def ollama_judge(ollama, model, query, title, excerpt, timeout=90):
    """Local-ollama judge (opt-in via --judge-backend ollama). WARNING: the local
    ollama also serves production `search` embeddings; heavy use can trip its
    circuit breaker. Prefer the anthropic backend."""
    prompt = _judge_prompt(query, title, excerpt)
    try:
        resp = _post(ollama, {"model": model, "prompt": prompt, "stream": False,
                              # think:false disables qwen3 reasoning if present;
                              # ignored by non-thinking models (gemma3 etc.).
                              "think": False,
                              "options": {"temperature": 0, "num_predict": 24}}, timeout)
        return _parse_yesno(resp.get("response"))
    except (urllib.error.URLError, TimeoutError, OSError, ValueError) as e:
        print(f"    [judge error: {e}]", file=sys.stderr)
        return None  # unknown — excluded, surfaced (no silent guess)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--num-queries", type=int, default=150)
    ap.add_argument("--search-k", type=int, default=25,
                    help="unified-search lane depth for the judging pool")
    ap.add_argument("--fulltext-k", type=int, default=25,
                    help="BM25 keyword lane depth for the judging pool (de-bias)")
    ap.add_argument("--judge-backend", choices=["anthropic", "ollama"], default="anthropic")
    ap.add_argument("--judge-model", default=None,
                    help="default: claude-haiku-4-5-20251001 (anthropic) / gemma3:12b (ollama)")
    ap.add_argument("--mcp", default=MCP_DEFAULT)
    ap.add_argument("--ollama", default=OLLAMA_DEFAULT)
    ap.add_argument("--out", default="labeled_rdocs.json")
    ap.add_argument("--excerpt-chars", type=int, default=600)
    ap.add_argument("--throttle", type=float, default=0.0,
                    help="seconds to sleep between judge calls")
    args = ap.parse_args()

    # Resolve judge backend + callable.
    if args.judge_backend == "anthropic":
        api_key = os.environ.get("ANTHROPIC_API_KEY")
        if not api_key:
            print("ERROR: ANTHROPIC_API_KEY env var not set (required for --judge-backend "
                  "anthropic).", file=sys.stderr)
            sys.exit(2)
        model = args.judge_model or "claude-haiku-4-5-20251001"
        judge = lambda q, t, e: anthropic_judge(api_key, model, q, t, e)
    else:
        model = args.judge_model or "gemma3:12b"
        judge = lambda q, t, e: ollama_judge(args.ollama, model, q, t, e)

    mcp_init(args.mcp)
    queries = pull_queries(args.mcp, args.num_queries)
    print(f"queries: {len(queries)} | judge: {args.judge_backend}/{model} | "
          f"pool: search-k={args.search_k}+fulltext-k={args.fulltext_k}", file=sys.stderr)

    labeled = []
    for i, q in enumerate(queries, 1):
        query = q["query"]
        candidates = gather_candidates(args.mcp, query, args.search_k, args.fulltext_k,
                                       args.excerpt_chars)
        if not candidates:
            print(f"[{i}/{len(queries)}] no candidates for {query!r}", file=sys.stderr)
            continue
        relevant, judged, unknown = [], 0, 0
        for c in candidates:
            verdict = judge(query, c["title"], c["excerpt"])
            if args.throttle:
                time.sleep(args.throttle)
            if verdict is None:
                unknown += 1
                continue
            judged += 1
            if verdict:
                relevant.append(c["doc_id"])
        labeled.append({
            "query": query,
            "collection": COLLECTION,
            "relevant_doc_ids": relevant,
            "answerable": len(relevant) > 0,
            "source_rating": q.get("rating"),
            "candidates_judged": judged,
            "candidates_unknown": unknown,
        })
        print(f"[{i}/{len(queries)}] {query!r} → {len(relevant)}/{judged} relevant"
              + (f" ({unknown} unknown)" if unknown else ""), file=sys.stderr)

    with open(args.out, "w") as f:
        json.dump(labeled, f, ensure_ascii=False, indent=2)
    ans = sum(1 for x in labeled if x["answerable"])
    print(f"\nWROTE {args.out}: {len(labeled)} labeled queries, {ans} answerable, "
          f"{len(labeled) - ans} unanswerable (abstention targets).", file=sys.stderr)


if __name__ == "__main__":
    main()
