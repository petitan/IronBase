#!/usr/bin/env python3
"""Curate the auto-labeled eval set: drop non-retrieval queries + dedup near-
duplicates. One-time DATA hygiene (not a runtime mechanism) — fully logged and
human-overridable. Conservative: when unsure, KEEP.

  python3 curate_labels.py --in labeled_rdocs.json --out labeled_rdocs_curated.json
"""
import argparse
import json
import re
import sys
import unicodedata

# Filler tokens removed before computing a dedup signature.
FILLER = {"egy", "mennyibe", "kerul", "kerek", "adj", "az", "a", "nehany",
          "konkret", "kerem", "generalj", "keszits", "sorolj", "fel"}
# Note: 'ar' (price) is intentionally KEPT in the signature — folding ár-variants
# to 'ar' lets price queries dedup with each other, but keeping the token avoids
# merging a price query ("fékpad árak") with a spec query ("mekkora egy fékpad").
# 'mekkora' also kept for the same reason (it marks a distinct dimension intent).


def drop_reason(q):
    """Return why a query is NOT a retrieval query (None = keep). Conservative."""
    ql = q.lower().strip()
    if len(q) > 55 or q.count(".") >= 3:
        return "junk-text"          # pasted document text
    # No trailing \b: it would fail mid-word (e.g. "készíts listát" → "list"+"á").
    if re.search(r"(hány|hányféle|hány féle|listáz|listál|listát|listája|árlist|"
                 r"készíts list|sorolj fel|utolsó \d|összes|eladások)", ql):
        return "aggregate"          # count / list intent
    if "előfordul" in ql:
        return "existence"          # term-existence check, not retrieval
    if re.search(r"^(mutasd|nyisd|részletezd|add )", ql) or "tartalmát" in ql \
            or ql in ("ára?", "ára", "ár?", "igen", "2026 van"):
        return "followup"           # context-dependent / non-query
    if re.search(r"(ajánlottunk|mit vásárolt|dokumentumai|ajánatai|ajánlatai keres)", ql):
        return "analytical-lookup"  # entity-scoped analytics
    if len(ql.split()) <= 1 and not re.search(r"pef|sice|adas|bosch|m1n1|bm3010|pdc|pel", ql):
        return "thin"               # single non-domain token
    return None


def signature(q):
    """Order-independent normalized token set for near-duplicate clustering."""
    s = unicodedata.normalize("NFKD", q.lower())
    s = "".join(c for c in s if not unicodedata.combining(c))   # strip accents
    s = re.sub(r"[^a-z0-9 -]", " ", s)
    toks = []
    for t in s.split():
        if re.match(r"^ar(a|ak|at|ait|et)?$", t):
            t = "ar"                                            # ár/ára/árak/árat → ar
        toks.append(t)
    toks = [t for t in toks if t not in FILLER]
    return " ".join(sorted(set(toks)))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--in", dest="inp", default="labeled_rdocs.json")
    ap.add_argument("--out", default="labeled_rdocs_curated.json")
    args = ap.parse_args()

    data = json.load(open(args.inp))

    # 1) Drop non-retrieval queries.
    kept, dropped = [], []
    for x in data:
        r = drop_reason(x["query"])
        (dropped if r else kept).append((x, r))

    # 2) Dedup near-duplicates among the kept set. Keep the representative with the
    #    MOST relevant docs (best-labeled); tie → shortest query.
    clusters = {}
    for x, _ in kept:
        clusters.setdefault(signature(x["query"]), []).append(x)
    deduped, merged_away = [], []
    for sig, members in clusters.items():
        members.sort(key=lambda m: (-len(m["relevant_doc_ids"]), len(m["query"])))
        deduped.append(members[0])
        merged_away.extend(members[1:])

    deduped.sort(key=lambda m: (not m["answerable"], m["query"].lower()))
    json.dump(deduped, open(args.out, "w"), ensure_ascii=False, indent=2)

    # Report
    print(f"in: {len(data)}  →  kept-after-drop: {len(kept)}  →  after-dedup: {len(deduped)}",
          file=sys.stderr)
    ans = sum(1 for x in deduped if x["answerable"])
    print(f"curated set: {len(deduped)} ({ans} answerable / {len(deduped)-ans} abstention)",
          file=sys.stderr)

    from collections import Counter
    dc = Counter(r for _, r in dropped)
    print(f"\nDROPPED ({len(dropped)}): {dict(dc)}", file=sys.stderr)
    for x, r in dropped:
        print(f"  [{r}] {x['query'][:60]!r}", file=sys.stderr)
    print(f"\nMERGED AWAY as near-dups ({len(merged_away)}):", file=sys.stderr)
    for m in merged_away:
        print(f"  {m['query'][:55]!r}", file=sys.stderr)
    print(f"\nWROTE {args.out}", file=sys.stderr)


if __name__ == "__main__":
    main()
