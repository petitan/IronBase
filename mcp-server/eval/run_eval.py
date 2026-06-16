#!/usr/bin/env python3
"""Retrieval baseline eval for the `search` tool (redesign §12, Stage A/B gate).

Runs the curated label set against a LIVE `search` endpoint over HTTP and reports
nDCG@k / Recall@k (the Stage-A/B quality floor) plus an abstention proxy. This is
the read-only consumer side: it holds no DB lock and does not write the corpus.

Metric definitions are kept BYTE-FOR-BYTE consistent with the unit-tested Rust
harness `mcp-server/tests/retrieval_eval.rs` (binary nDCG, vacuous-recall rule).
`--selftest` reproduces that harness's hand-computed cases so a drift in this
implementation is caught offline, without a server.

NOTE (Stage A): the `search` contract returns `verdict:"unknown"` unconditionally
(contract.rs:178 — Stage D abstention not yet built). So the abstention figure
here is only a `documents == []` proxy; treat it as a Stage-D motivation signal,
not a real abstention metric.

Usage:
  python3 run_eval.py --selftest                      # offline metric check
  python3 run_eval.py --mcp http://localhost:8080/mcp # against a live server
  python3 run_eval.py --labels labeled_rdocs_curated.json --k 5 --verbose
"""
import argparse
import json
import math
import os
import sys
import urllib.request

MCP_DEFAULT = "http://192.168.2.211:8080/mcp"
LABELS_DEFAULT = os.path.join(os.path.dirname(__file__), "labeled_rdocs_curated.json")


# --- HTTP plumbing (same shape as build_labels.py) -------------------------
def _post(url, payload, timeout):
    data = json.dumps(payload).encode()
    req = urllib.request.Request(url, data=data, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.loads(r.read().decode())


def mcp_init(mcp):
    _post(mcp, {"jsonrpc": "2.0", "id": 0, "method": "initialize",
                "params": {"protocolVersion": "2024-11-05", "capabilities": {},
                           "clientInfo": {"name": "eval-runner", "version": "1"}}}, 15)


def mcp_tool(mcp, name, args, timeout=60):
    resp = _post(mcp, {"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                       "params": {"name": name, "arguments": args}}, timeout)
    res = resp.get("result", {})
    if res.get("isError"):
        raise RuntimeError(res.get("content", [{}])[0].get("text", "tool error"))
    return json.loads(res["content"][0]["text"])


# --- metrics (consistent with retrieval_eval.rs) ---------------------------
def recall_at_k(retrieved, relevant, k):
    """|relevant ∩ retrieved[:k]| / |relevant|. Empty relevant → 1.0 (vacuous)."""
    if not relevant:
        return 1.0
    top = set(retrieved[:k])
    hit = sum(1 for r in relevant if r in top)
    return hit / len(relevant)


def ndcg_at_k(retrieved, relevant, k):
    """Binary-relevance nDCG@k. DCG = Σ rel_i / log2(i+2); IDCG = ideal ordering."""
    if not relevant:
        return 0.0
    dcg = 0.0
    for i, doc in enumerate(retrieved[:k]):
        if doc in relevant:
            dcg += 1.0 / math.log2(i + 2)
    ideal = min(len(relevant), k)
    idcg = sum(1.0 / math.log2(i + 2) for i in range(ideal))
    return dcg / idcg if idcg > 0 else 0.0


def abstention_metrics(predicted_abstain, actually_unanswerable):
    """Returns (precision, recall) for the abstain decision."""
    tp = sum(1 for p, a in zip(predicted_abstain, actually_unanswerable) if p and a)
    fp = sum(1 for p, a in zip(predicted_abstain, actually_unanswerable) if p and not a)
    fn = sum(1 for p, a in zip(predicted_abstain, actually_unanswerable) if not p and a)
    precision = tp / (tp + fp) if (tp + fp) else 1.0
    recall = tp / (tp + fn) if (tp + fn) else 1.0
    return precision, recall


# --- offline self-test: reproduce the Rust harness hand-computed cases ------
def selftest():
    ok = True

    def check(name, got, want, eps=1e-6):
        nonlocal ok
        good = abs(got - want) < eps
        ok = ok and good
        print(f"  [{'OK' if good else 'FAIL'}] {name}: got={got:.6f} want={want:.6f}")

    # recall (test_recall_at_k_basic): retrieved a,x,b → relevant {a,b}
    check("recall@2", recall_at_k(["a", "x", "b", "c"], {"a", "b"}, 2), 0.5)
    check("recall@3", recall_at_k(["a", "x", "b", "c"], {"a", "b"}, 3), 1.0)
    check("recall@1", recall_at_k(["a", "x", "b", "c"], {"a", "b"}, 1), 0.5)
    # recall vacuous empty relevant
    check("recall@5 empty-relevant", recall_at_k(["a", "b"], set(), 5), 1.0)
    # ndcg perfect / interleaved / none (test_ndcg_*)
    check("ndcg perfect", ndcg_at_k(["a", "b", "c"], {"a", "b", "c"}, 3), 1.0)
    check("ndcg interleaved", ndcg_at_k(["a", "x", "b"], {"a", "b"}, 3), 0.919_721_4)
    check("ndcg no-relevant", ndcg_at_k(["x", "y", "z"], {"a"}, 3), 0.0)
    # abstention (test_abstention_metrics_hand_computed): p=0.5, r=0.5
    p, r = abstention_metrics([True, True, False, False], [True, False, True, False])
    check("abstention precision", p, 0.5)
    check("abstention recall", r, 0.5)

    print("SELFTEST:", "PASS" if ok else "FAIL")
    return 0 if ok else 1


# --- live eval -------------------------------------------------------------
def run(mcp, labels, k, limit, verbose):
    with open(labels, encoding="utf-8") as f:
        label_set = json.load(f)

    mcp_init(mcp)
    print(f"labels={len(label_set)} | mcp={mcp} | k={k} | search limit={limit}\n",
          file=sys.stderr)

    ndcg_sum = recall_sum = 0.0
    pred_abstain, actually_unanswerable = [], []
    n_ok = 0
    for i, lq in enumerate(label_set, 1):
        query = lq["query"]
        collection = lq.get("collection", "rdocs")
        relevant = set(lq.get("relevant_doc_ids", []))
        answerable = bool(lq.get("answerable", True))
        try:
            sr = mcp_tool(mcp, "search", {"collection": collection, "query": query, "limit": limit})
        except Exception as e:
            print(f"[{i}/{len(label_set)}] search FAILED {query!r}: {e}", file=sys.stderr)
            continue
        docs = sr.get("documents", [])
        retrieved = [d.get("doc_id") for d in docs if d.get("doc_id")]

        nd = ndcg_at_k(retrieved, relevant, k)
        rc = recall_at_k(retrieved, relevant, k)
        ndcg_sum += nd
        recall_sum += rc
        pred_abstain.append(len(retrieved) == 0)
        actually_unanswerable.append(not answerable)
        n_ok += 1
        if verbose:
            print(f"[{i}] ndcg@{k}={nd:.3f} recall@{k}={rc:.3f} "
                  f"ret={len(retrieved)} rel={len(relevant)} :: {query}")

    if n_ok == 0:
        print("No queries evaluated (server unreachable?).", file=sys.stderr)
        return 1

    ab_p, ab_r = abstention_metrics(pred_abstain, actually_unanswerable)
    print("\n=== SEARCH BASELINE (rdocs curated label set) ===")
    print(f"queries evaluated: {n_ok}/{len(label_set)}")
    print(f"mean nDCG@{k}   = {ndcg_sum / n_ok:.4f}")
    print(f"mean Recall@{k} = {recall_sum / n_ok:.4f}")
    n_unans = sum(actually_unanswerable)
    print(f"abstention PROXY (documents==[]):  precision={ab_p:.3f} recall={ab_r:.3f}  "
          f"(unanswerable labels: {n_unans}/{n_ok})")
    print("NOTE: verdict is always 'unknown' in Stage A — abstention is a proxy, "
          "motivates Stage D.")
    return 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--mcp", default=MCP_DEFAULT)
    ap.add_argument("--labels", default=LABELS_DEFAULT)
    ap.add_argument("--k", type=int, default=5, help="metric cutoff (nDCG@k / Recall@k)")
    ap.add_argument("--limit", type=int, default=10, help="search result limit")
    ap.add_argument("--verbose", action="store_true")
    ap.add_argument("--selftest", action="store_true", help="offline metric check, no server")
    args = ap.parse_args()
    if args.selftest:
        sys.exit(selftest())
    sys.exit(run(args.mcp, args.labels, args.k, args.limit, args.verbose))


if __name__ == "__main__":
    main()
