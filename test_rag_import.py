#!/usr/bin/env python3
"""
End-to-End Tests for chunking and RAG import (chunk_text, Collection.rag_import)
"""

import ironbase
from ironbase import IronBase

# Disable verbose logging for cleaner test output
IronBase.set_log_level("WARN")

DOC = """# Manual

## Engine

Check the engine oil every month. """ + "Oil keeps the engine cool. " * 20 + """

## Tyres

Tyre pressure matters. """ + "Inflate the tyres to the listed pressure. " * 20


def fake_embed(texts):
    """One deterministic 4-dim vector per text"""
    return [[float(len(t) % 7), float("oil" in t), float("tyre" in t.lower()), 1.0] for t in texts]


def test_chunk_text():
    """Test: chunks carry layout, headings and the text to embed"""
    print("=" * 70)
    print("TEST: chunk_text")
    print("=" * 70)
    chunks = ironbase.chunk_text(DOC, chunk_size=300, overlap=30)
    assert len(chunks) >= 3, chunks
    assert [c["index"] for c in chunks] == list(range(len(chunks)))
    assert all(c["total"] == len(chunks) for c in chunks)
    for c in chunks:
        assert DOC.count(c["text"][:20]) >= 1
        assert c["end_char"] > c["start_char"]
    tyres = [c for c in chunks if c["section_path"] and "Tyres" in c["section_path"]]
    assert tyres, "markdown section path missing"
    assert "Tyres" in tyres[-1]["embed_text"], "breadcrumb missing from the embed text"
    plain = ironbase.chunk_text("word " * 500, chunk_size=200, overlap=0, mode="text")
    assert len(plain) > 1 and all(c["section_path"] is None for c in plain)
    print(f"✓ {len(chunks)} markdown chunks, {len(plain)} text chunks")
    print("✓ PASSED\n")


def test_rag_import_and_search():
    """Test: one document per chunk, embeddings from the callable, searchable"""
    print("=" * 70)
    print("TEST: rag_import")
    print("=" * 70)
    db = IronBase.open_memory()
    kb = db.collection("kb")
    calls = []

    def embed(texts):
        calls.append(len(texts))
        return fake_embed(texts)

    out = kb.rag_import(DOC, embed, doc_id="manual", chunk_size=300, overlap=30,
                        title="Car manual", metadata={"year": 2026, "doc_id": "evil"})
    n = out["chunks_created"]
    assert out == {"doc_id": "manual", "chunks_created": n, "chunks_replaced": 0,
                   "skipped": False}, out
    assert calls == [n], "embed must be called once with every chunk"
    docs = kb.find({"doc_id": "manual"})
    assert len(docs) == n
    assert all(d["title"] == "Car manual" and d["year"] == 2026 for d in docs)
    assert all(len(d["embedding"]) == 4 for d in docs)
    print(f"✓ {n} chunks stored with embeddings, metadata cannot override doc_id")

    kb.create_fulltext_index("content", language="english")
    kb.create_vector_index("embedding", 4)
    groups = kb.hybrid_search("tyre pressure", vector=fake_embed(["tyre pressure"])[0],
                              group_by_document=True, projection={"embedding": 0})
    assert groups and groups[0]["doc_id"] == "manual", groups
    print("✓ Imported chunks are searchable")
    db.close()
    print("✓ PASSED\n")


def test_rag_import_if_exists():
    """Test: re-import replaces, skip/error/append policies"""
    print("=" * 70)
    print("TEST: rag_import if_exists")
    print("=" * 70)
    db = IronBase.open_memory()
    kb = db.collection("kb")
    first = kb.rag_import(DOC, doc_id="d", chunk_size=300)
    assert first["chunks_created"] > 0

    again = kb.rag_import("Short replacement text.", doc_id="d")
    assert again["chunks_created"] == 1 and again["chunks_replaced"] == first["chunks_created"]
    assert kb.count_documents({"doc_id": "d"}) == 1
    print("✓ Re-import replaces the chunks")

    called = []
    skipped = kb.rag_import(DOC, lambda t: called.append(1) or fake_embed(t), doc_id="d",
                            if_exists="skip")
    assert skipped["skipped"] and not called, "skip must not embed"
    try:
        kb.rag_import(DOC, doc_id="d", if_exists="error")
        raise AssertionError("existing doc_id accepted with if_exists='error'")
    except ironbase.IronBaseException as e:
        assert "already exists" in str(e)
    appended = kb.rag_import("More.", doc_id="d", if_exists="append")
    assert appended["chunks_created"] == 1 and kb.count_documents({"doc_id": "d"}) == 2
    print("✓ skip / error / append")

    for bad in ({"if_exists": "skp"},):
        try:
            kb.rag_import(DOC, doc_id="d", **bad)
            raise AssertionError(f"accepted {bad}")
        except ValueError:
            pass
    try:
        kb.rag_import(DOC, lambda t: [[1.0]], doc_id="e")
        raise AssertionError("wrong number of vectors accepted")
    except ironbase.IronBaseException:
        pass
    assert kb.count_documents({"doc_id": "e"}) == 0
    print("✓ Invalid if_exists and a wrong vector count rejected")

    # A vector index fixes the dimension: a wrong one is refused BEFORE the
    # stored chunks are replaced
    kb.create_vector_index("embedding", 4)
    kb.rag_import("Indexed text.", fake_embed, doc_id="v")
    try:
        kb.rag_import("New text.", lambda t: [[1.0, 0.0, 0.0] for _ in t], doc_id="v")
        raise AssertionError("3-dim vectors accepted by a 4-dim index")
    except ironbase.IronBaseException as e:
        assert "dimension" in str(e)
    assert [d["content"] for d in kb.find({"doc_id": "v"})] == ["Indexed text."]
    print("✓ Wrong embedding dimension rejected, the old chunks kept")

    for bad_mode in ("markdwon", "md "):
        try:
            kb.rag_import(DOC, doc_id="m", mode=bad_mode)
            raise AssertionError(f"accepted mode {bad_mode!r}")
        except ValueError:
            pass
    print("✓ Invalid chunking mode rejected")

    auto = kb.rag_import("Some text.")
    assert len(auto["doc_id"]) == 36, auto
    print("✓ A new doc_id is generated")
    db.close()
    print("✓ PASSED\n")


def run_all_tests():
    tests = [
        test_chunk_text,
        test_rag_import_and_search,
        test_rag_import_if_exists,
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
