#!/usr/bin/env python3
"""
End-to-End Tests for Fuzzy and Full-Text Search
Tests the new fuzzy search and full-text search features
"""

from ironbase import IronBase
import os

# Disable verbose logging for cleaner test output
IronBase.set_log_level("WARN")

def cleanup(path):
    """Clean up test database files"""
    for ext in [".mlite", ".wal", ".lock"]:
        try:
            os.remove(path.replace(".mlite", ext))
        except FileNotFoundError:
            pass

def test_fuzzy_index_creation():
    """Test: Create fuzzy index"""
    print("=" * 70)
    print("TEST: Fuzzy Index Creation")
    print("=" * 70)

    db_path = "test_fuzzy_index.mlite"
    cleanup(db_path)

    db = IronBase(db_path)
    articles = db.collection("articles")

    # Create fuzzy index with default settings
    index_name = articles.create_fuzzy_index("title")
    assert index_name is not None
    assert "title" in index_name
    assert "fuzzy" in index_name
    print(f"✓ Created fuzzy index: {index_name}")

    # Create fuzzy index with custom algorithm
    index_name2 = articles.create_fuzzy_index("content", algorithm="levenshtein", threshold=0.7)
    assert index_name2 is not None
    print(f"✓ Created fuzzy index with Levenshtein: {index_name2}")

    db.close()
    cleanup(db_path)
    print("✓ PASSED\n")

def test_fuzzy_search():
    """Test: Fuzzy search functionality"""
    print("=" * 70)
    print("TEST: Fuzzy Search")
    print("=" * 70)

    db_path = "test_fuzzy_search.mlite"
    cleanup(db_path)

    db = IronBase(db_path)
    articles = db.collection("articles")

    # Create index FIRST (before inserting data)
    articles.create_fuzzy_index("name")

    # Insert test data
    articles.insert_many([
        {"name": "Python", "type": "language"},
        {"name": "Pithon", "type": "typo"},      # Typo of Python
        {"name": "Cython", "type": "language"},  # Similar to Python
        {"name": "Java", "type": "language"},    # Not similar
    ])

    # Search for "Python" - should find Python and Pithon (similar)
    results = articles.fuzzy_search("name", "Python", threshold=0.7)
    assert len(results) >= 1, f"Expected at least 1 result, got {len(results)}"
    print(f"✓ Fuzzy search found {len(results)} results for 'Python'")

    # Check that results have document and score
    for doc, score in results:
        assert score > 0, "Score should be positive"
        assert "name" in doc, "Document should have 'name' field"
        print(f"  - {doc['name']}: score={score:.3f}")

    db.close()
    cleanup(db_path)
    print("✓ PASSED\n")

def test_fuzzy_search_algorithms():
    """Test: Different fuzzy search algorithms"""
    print("=" * 70)
    print("TEST: Fuzzy Search Algorithms")
    print("=" * 70)

    db_path = "test_fuzzy_algorithms.mlite"
    cleanup(db_path)

    db = IronBase(db_path)
    words = db.collection("words")

    # Create index
    words.create_fuzzy_index("word")

    # Insert test data
    words.insert_many([
        {"word": "hello"},
        {"word": "hallo"},   # 1 char different
        {"word": "hullo"},   # 1 char different
        {"word": "world"},   # Very different
    ])

    # Test with Jaro-Winkler (default)
    results_jw = words.fuzzy_search("word", "hello", algorithm="jaro_winkler", threshold=0.8)
    print(f"✓ Jaro-Winkler found {len(results_jw)} results")

    # Test with Levenshtein
    results_lev = words.fuzzy_search("word", "hello", algorithm="levenshtein", threshold=0.7)
    print(f"✓ Levenshtein found {len(results_lev)} results")

    # Test with Damerau-Levenshtein
    results_dl = words.fuzzy_search("word", "hello", algorithm="damerau_levenshtein", threshold=0.7)
    print(f"✓ Damerau-Levenshtein found {len(results_dl)} results")

    db.close()
    cleanup(db_path)
    print("✓ PASSED\n")

def test_fulltext_index_creation():
    """Test: Create full-text search index"""
    print("=" * 70)
    print("TEST: Full-Text Index Creation")
    print("=" * 70)

    db_path = "test_fulltext_index.mlite"
    cleanup(db_path)

    db = IronBase(db_path)
    articles = db.collection("articles")

    # Create fulltext index with English
    index_name = articles.create_fulltext_index("content", language="english")
    assert index_name is not None
    assert "content" in index_name
    assert "fts" in index_name  # Full-text search suffix
    print(f"✓ Created fulltext index (English): {index_name}")

    # Create fulltext index with Hungarian
    index_name2 = articles.create_fulltext_index("title", language="hungarian")
    assert index_name2 is not None
    print(f"✓ Created fulltext index (Hungarian): {index_name2}")

    db.close()
    cleanup(db_path)
    print("✓ PASSED\n")

def test_fulltext_search():
    """Test: Full-text search functionality"""
    print("=" * 70)
    print("TEST: Full-Text Search")
    print("=" * 70)

    db_path = "test_fulltext_search.mlite"
    cleanup(db_path)

    db = IronBase(db_path)
    articles = db.collection("articles")

    # Create index FIRST
    articles.create_fulltext_index("content", language="english")

    # Insert test data
    articles.insert_many([
        {"title": "Article 1", "content": "The quick brown fox jumps over the lazy dog"},
        {"title": "Article 2", "content": "Python is a programming language for developers"},
        {"title": "Article 3", "content": "Machine learning uses algorithms and data science"},
        {"title": "Article 4", "content": "Programming in Python is fun and educational"},
    ])

    # Search for "programming"
    results = articles.fulltext_search("content", "programming", limit=10)
    assert len(results) >= 1, f"Expected at least 1 result, got {len(results)}"
    print(f"✓ Fulltext search found {len(results)} results for 'programming'")

    # Check result structure
    for doc, score, tokens in results:
        assert score > 0, "Score should be positive"
        assert isinstance(tokens, list), "Tokens should be a list"
        print(f"  - {doc['title']}: score={score:.3f}, tokens={tokens}")

    db.close()
    cleanup(db_path)
    print("✓ PASSED\n")

def test_fulltext_search_pagination():
    """Test: Full-text search with pagination"""
    print("=" * 70)
    print("TEST: Full-Text Search Pagination")
    print("=" * 70)

    db_path = "test_fulltext_pagination.mlite"
    cleanup(db_path)

    db = IronBase(db_path)
    articles = db.collection("articles")

    # Create index FIRST
    articles.create_fulltext_index("content", language="english")

    # Insert 10 matching documents
    for i in range(10):
        articles.insert_one({
            "title": f"Article {i}",
            "content": f"This is document number {i} about programming and development"
        })

    # Get first page
    page1 = articles.fulltext_search("content", "programming", limit=3)
    assert len(page1) == 3, f"Expected 3 results, got {len(page1)}"
    print(f"✓ Page 1: {len(page1)} results")

    # Get second page
    page2 = articles.fulltext_search("content", "programming", limit=3, skip=3)
    assert len(page2) == 3, f"Expected 3 results, got {len(page2)}"
    print(f"✓ Page 2: {len(page2)} results")

    # Get third page
    page3 = articles.fulltext_search("content", "programming", limit=3, skip=6)
    assert len(page3) == 3, f"Expected 3 results, got {len(page3)}"
    print(f"✓ Page 3: {len(page3)} results")

    db.close()
    cleanup(db_path)
    print("✓ PASSED\n")

def test_fulltext_search_projection():
    """Test: Full-text search with projection"""
    print("=" * 70)
    print("TEST: Full-Text Search Projection")
    print("=" * 70)

    db_path = "test_fulltext_projection.mlite"
    cleanup(db_path)

    db = IronBase(db_path)
    articles = db.collection("articles")

    # Create index FIRST
    articles.create_fulltext_index("content", language="english")

    # Insert test data with large content
    articles.insert_one({
        "title": "Test Article",
        "content": "Programming in Python is very popular among developers worldwide",
        "metadata": {"author": "John", "views": 1000}
    })

    # Search with projection - only return title
    results = articles.fulltext_search(
        "content",
        "programming",
        projection={"title": 1}
    )
    assert len(results) >= 1
    doc, score, tokens = results[0]
    assert "title" in doc
    print(f"✓ Projection returned: {list(doc.keys())}")

    db.close()
    cleanup(db_path)
    print("✓ PASSED\n")

def test_list_fulltext_indexes():
    """Test: List full-text indexes"""
    print("=" * 70)
    print("TEST: List Full-Text Indexes")
    print("=" * 70)

    db_path = "test_list_fulltext.mlite"
    cleanup(db_path)

    db = IronBase(db_path)
    articles = db.collection("articles")

    # Create multiple fulltext indexes
    articles.create_fulltext_index("title", language="english")
    articles.create_fulltext_index("content", language="hungarian")

    # List indexes
    indexes = articles.list_fulltext_indexes()
    assert len(indexes) >= 2, f"Expected at least 2 indexes, got {len(indexes)}"
    print(f"✓ Found {len(indexes)} fulltext indexes:")
    for idx in indexes:
        print(f"  - {idx['name']}: field={idx['field']}, language={idx['language']}")

    db.close()
    cleanup(db_path)
    print("✓ PASSED\n")

def test_fuzzy_and_fulltext_combined():
    """Test: Using both fuzzy and full-text search together"""
    print("=" * 70)
    print("TEST: Combined Fuzzy and Full-Text Search")
    print("=" * 70)

    db_path = "test_combined_search.mlite"
    cleanup(db_path)

    db = IronBase(db_path)
    articles = db.collection("articles")

    # Create both types of indexes FIRST
    articles.create_fuzzy_index("title")
    articles.create_fulltext_index("content", language="english")

    # Insert test data
    articles.insert_one({
        "title": "Python",
        "content": "Python is a versatile programming language used for web development and data science"
    })

    # Fuzzy search on title
    fuzzy_results = articles.fuzzy_search("title", "Pyton", threshold=0.7)  # Typo
    print(f"✓ Fuzzy search found {len(fuzzy_results)} results for 'Pyton' (typo)")

    # Fulltext search on content
    fulltext_results = articles.fulltext_search("content", "programming")
    print(f"✓ Fulltext search found {len(fulltext_results)} results for 'programming'")

    assert len(fuzzy_results) >= 1, "Fuzzy search should find the Python document"
    assert len(fulltext_results) >= 1, "Fulltext search should find the document"

    db.close()
    cleanup(db_path)
    print("✓ PASSED\n")

def test_hungarian_fulltext():
    """Test: Hungarian language full-text search"""
    print("=" * 70)
    print("TEST: Hungarian Full-Text Search")
    print("=" * 70)

    db_path = "test_hungarian_fulltext.mlite"
    cleanup(db_path)

    db = IronBase(db_path)
    articles = db.collection("articles")

    # Create Hungarian fulltext index
    articles.create_fulltext_index("content", language="hungarian")

    # Insert Hungarian text
    articles.insert_many([
        {"title": "Cikk 1", "content": "A programozás nagyon fontos a modern világban"},
        {"title": "Cikk 2", "content": "Python egy népszerű programozási nyelv"},
        {"title": "Cikk 3", "content": "A király palotája gyönyörű"},
    ])

    # Search for Hungarian word
    results = articles.fulltext_search("content", "programozás")
    print(f"✓ Hungarian search found {len(results)} results for 'programozás'")

    # Hungarian stemmer should handle word variations
    results2 = articles.fulltext_search("content", "király")
    print(f"✓ Hungarian search found {len(results2)} results for 'király'")

    db.close()
    cleanup(db_path)
    print("✓ PASSED\n")

def run_all_tests():
    """Run all fuzzy and fulltext search tests"""
    print("\n" + "=" * 70)
    print("IRONBASE FUZZY & FULL-TEXT SEARCH TEST SUITE")
    print("=" * 70 + "\n")

    tests = [
        test_fuzzy_index_creation,
        test_fuzzy_search,
        test_fuzzy_search_algorithms,
        test_fulltext_index_creation,
        test_fulltext_search,
        test_fulltext_search_pagination,
        test_fulltext_search_projection,
        test_list_fulltext_indexes,
        test_fuzzy_and_fulltext_combined,
        test_hungarian_fulltext,
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
