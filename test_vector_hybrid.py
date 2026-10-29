#!/usr/bin/env python3
"""
End-to-End Tests for vector and hybrid search
(create_vector_index, vector_search, hybrid_search)
"""

from ironbase import IronBase

# Disable verbose logging for cleaner test output
IronBase.set_log_level("WARN")

# "lex" matches the words, "sem" only the vector, "both" both
CHUNKS = [
    ("lex", 0, "Maintenance schedule for printers.", [0.0, 1.0, 0.0, 0.0]),
    ("sem", 0, "The automobile needs periodic servicing.", [1.0, 0.0, 0.0, 0.0]),
    ("both", 0, "Car maintenance checklist.", [0.9, 0.1, 0.0, 0.0]),
    ("both", 1, "Tyre pressure table.", [0.0, 0.0, 1.0, 0.0]),
]


def make_kb():
    db = IronBase.open_memory()
    kb = db.collection("kb")
    kb.insert_many([
        {"doc_id": d, "chunk_index": i, "content": c, "title": f"Doc {d}", "embedding": e}
        for d, i, c, e in CHUNKS
    ])
    kb.create_fulltext_index("content", language="english")
    kb.create_vector_index("embedding", 4)
    return db, kb


def test_vector_search():
    """Test: nearest-neighbour search, with a filter"""
    print("=" * 70)
    print("TEST: Vector Search")
    print("=" * 70)
    db, kb = make_kb()

    results = kb.vector_search("embedding", [1.0, 0.0, 0.0, 0.0], limit=2)
    assert [doc["doc_id"] for doc, _ in results] == ["sem", "both"], results
    assert results[0][1] >= results[1][1]
    print(f"✓ Nearest: {[doc['doc_id'] for doc, _ in results]}")

    filtered = kb.vector_search("embedding", [1.0, 0.0, 0.0, 0.0], filter={"doc_id": "lex"})
    assert [doc["doc_id"] for doc, _ in filtered] == ["lex"], filtered
    print("✓ Filter restricts the searched documents")

    try:
        kb.create_vector_index("other", 4, metric="manhattan")
        raise AssertionError("unknown metric accepted")
    except Exception as e:
        assert "manhattan" in str(e)
    print("✓ Unknown metric rejected")
    db.close()
    print("✓ PASSED\n")


def test_hybrid_search_flat():
    """Test: both searches fused, scores attached"""
    print("=" * 70)
    print("TEST: Hybrid Search (flat)")
    print("=" * 70)
    db, kb = make_kb()

    hits = kb.hybrid_search("car maintenance", vector=[1.0, 0.0, 0.0, 0.0], merge_chunks=False)
    ids = [h["doc_id"] for h in hits]
    assert ids[0] == "both", ids
    assert "sem" in ids and "lex" in ids, ids
    top = hits[0]
    assert top["_vector_rank"] > 0 and top["_text_rank"] > 0
    expected = 0.5 / (20 + top["_vector_rank"]) + 0.5 / (20 + top["_text_rank"])
    assert abs(top["_rrf_score"] - expected) < 1e-12
    print(f"✓ Fused order: {ids}")

    bm25 = kb.hybrid_search("maintenance")
    assert sorted(h["doc_id"] for h in bm25) == ["both", "lex"]
    assert all(h["_vector_rank"] == 0 and "_vector_score" not in h for h in bm25)
    print("✓ Without a vector only the fulltext search runs")

    projected = kb.hybrid_search("car", vector=[1.0, 0.0, 0.0, 0.0], projection={"embedding": 0})
    assert all("embedding" not in h for h in projected)
    print("✓ Projection")

    nested = kb.hybrid_search("car", vector=[1.0, 0.0, 0.0, 0.0],
                              projection={"doc_id": 1, "title": 1})
    assert all(set(h) - {"_id", "doc_id", "title"} <= {
        "_rrf_score", "_final_score", "_rerank_boost", "_vector_rank", "_text_rank",
        "_vector_score", "_text_score"} for h in nested), nested
    print("✓ Inclusion projection")

    for bad in ({"mode": "xor"}, {"match_scope": "page"}):
        try:
            kb.hybrid_search("car", **bad)
            raise AssertionError(f"accepted {bad}")
        except ValueError:
            pass
    print("✓ Invalid mode / match_scope rejected")
    db.close()
    print("✓ PASSED\n")


def test_hybrid_search_document_and():
    """Test: mode="and" across a document's chunks"""
    print("=" * 70)
    print("TEST: Hybrid Search (document-level AND)")
    print("=" * 70)
    db, kb = make_kb()

    hits = kb.hybrid_search("maintenance tyre", mode="and", merge_chunks=False)
    assert [h["doc_id"] for h in hits] == ["both", "both"], hits
    print("✓ The words may be in different chunks of a document")

    assert kb.hybrid_search("maintenance tyre", mode="and", match_scope="chunk") == []
    print("✓ match_scope='chunk': no chunk holds both")
    db.close()
    print("✓ PASSED\n")


def test_hybrid_search_grouped():
    """Test: group_by_document"""
    print("=" * 70)
    print("TEST: Hybrid Search (grouped)")
    print("=" * 70)
    db, kb = make_kb()

    groups = kb.hybrid_search(
        "car maintenance", vector=[1.0, 0.0, 0.0, 0.0], group_by_document=True, limit=3,
        projection={"embedding": 0},
        # Unmerged: a merged run is one chunk, and a one-chunk group lifts nothing
        merge_chunks=False,
    )
    order = [g["doc_id"] for g in groups]
    assert order[0] == "both", order
    assert "sem" in order, f"vector-only document dropped: {order}"
    both = groups[0]
    assert both["chunk_count"] == len(both["chunks"])
    assert both["title"] == "Doc both", "common field not lifted to the group"
    assert all("title" not in c for c in both["chunks"])
    assert "_rrf_score" in both["chunks"][0]
    print(f"✓ Groups: {order}")
    db.close()
    print("✓ PASSED\n")


def run_all_tests():
    tests = [
        test_vector_search,
        test_hybrid_search_flat,
        test_hybrid_search_document_and,
        test_hybrid_search_grouped,
    ]

    passed = 0
    failed = 0

    for test in tests:
        try:
            test()
            passed += 1
        except Exception as e:
            failed += 1
            print(f"✗ FAILED: {test.__name__}")
            print(f"  Error: {e}")
            import traceback
            traceback.print_exc()
            print()

    print("=" * 70)
    print(f"RESULTS: {passed} passed, {failed} failed")
    print("=" * 70)

    return failed == 0


if __name__ == "__main__":
    success = run_all_tests()
    exit(0 if success else 1)
