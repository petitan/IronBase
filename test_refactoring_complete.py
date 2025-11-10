#!/usr/bin/env python3
"""Comprehensive test for refactored MongoLite functionality"""
import ironbase
import os

DB_FILE = 'test_refactoring.db'

def cleanup():
    if os.path.exists(DB_FILE):
        os.remove(DB_FILE)

def test_scenario(description, test_func):
    print(f"\n{'='*60}")
    print(f"TEST: {description}")
    print('='*60)
    try:
        test_func()
        print("✅ PASSED")
        return True
    except AssertionError as e:
        print(f"❌ FAILED: {e}")
        return False
    except Exception as e:
        print(f"❌ ERROR: {e}")
        return False

def test_basic_persistence():
    """Test that data persists across database reopens"""
    cleanup()

    # Create and populate
    db1 = ironbase.MongoLite(DB_FILE)
    users = db1.collection('users')
    users.insert_many([
        {'name': 'Alice', 'age': 28},
        {'name': 'Bob', 'age': 32},
    ])
    count1 = users.count_documents({})
    db1.close()

    # Reopen and verify
    db2 = ironbase.MongoLite(DB_FILE)
    users2 = db2.collection('users')
    count2 = users2.count_documents({})
    db2.close()

    assert count1 == 2, f"Expected 2 before close, got {count1}"
    assert count2 == 2, f"Expected 2 after reopen, got {count2}"
    print(f"  ✓ Data persisted correctly: {count2} documents")

def test_multi_collection_persistence():
    """Test multiple collections persist independently"""
    cleanup()

    # Create multiple collections
    db1 = ironbase.MongoLite(DB_FILE)
    users = db1.collection('users')
    posts = db1.collection('posts')
    comments = db1.collection('comments')

    users.insert_many([{'name': 'Alice'}, {'name': 'Bob'}])
    posts.insert_many([{'title': 'Post 1'}, {'title': 'Post 2'}, {'title': 'Post 3'}])
    comments.insert_many([{'text': 'Comment 1'}])

    u1 = users.count_documents({})
    p1 = posts.count_documents({})
    c1 = comments.count_documents({})
    db1.close()

    # Reopen and verify all collections
    db2 = ironbase.MongoLite(DB_FILE)
    users2 = db2.collection('users')
    posts2 = db2.collection('posts')
    comments2 = db2.collection('comments')

    u2 = users2.count_documents({})
    p2 = posts2.count_documents({})
    c2 = comments2.count_documents({})
    db2.close()

    assert u1 == u2 == 2, f"Users: expected 2, got {u1} before, {u2} after"
    assert p1 == p2 == 3, f"Posts: expected 3, got {p1} before, {p2} after"
    assert c1 == c2 == 1, f"Comments: expected 1, got {c1} before, {c2} after"
    print(f"  ✓ Users: {u2}, Posts: {p2}, Comments: {c2}")

def test_last_id_persistence():
    """Test that last_id persists and continues correctly"""
    cleanup()

    # Insert some documents
    db1 = ironbase.MongoLite(DB_FILE)
    users = db1.collection('users')
    users.insert_many([{'name': 'Alice'}, {'name': 'Bob'}])
    docs1 = list(users.find({}))
    ids1 = sorted([doc['_id'] for doc in docs1])
    db1.close()

    # Reopen and insert more
    db2 = ironbase.MongoLite(DB_FILE)
    users2 = db2.collection('users')
    users2.insert_one({'name': 'Carol'})
    docs2 = list(users2.find({}))
    ids2 = sorted([doc['_id'] for doc in docs2])
    db2.close()

    assert ids1 == [1, 2], f"Expected [1, 2], got {ids1}"
    assert ids2 == [1, 2, 3], f"Expected [1, 2, 3], got {ids2}"
    print(f"  ✓ IDs before: {ids1}, after: {ids2}")

def test_create_drop_collection():
    """Test create and drop collection with persistence"""
    cleanup()

    # Create collections (by accessing them)
    db1 = ironbase.MongoLite(DB_FILE)
    db1.collection('temp1')
    db1.collection('temp2')
    db1.collection('temp3')
    collections1 = sorted(db1.list_collections())
    db1.close()

    # Reopen and verify
    db2 = ironbase.MongoLite(DB_FILE)
    collections2 = sorted(db2.list_collections())

    # Drop one collection
    db2.drop_collection('temp2')
    collections3 = sorted(db2.list_collections())
    db2.close()

    # Reopen and verify drop persisted
    db3 = ironbase.MongoLite(DB_FILE)
    collections4 = sorted(db3.list_collections())
    db3.close()

    assert collections1 == ['temp1', 'temp2', 'temp3'], f"Expected 3 collections, got {collections1}"
    assert collections2 == ['temp1', 'temp2', 'temp3'], f"Collections not persisted"
    assert collections3 == ['temp1', 'temp3'], f"Drop didn't work"
    assert collections4 == ['temp1', 'temp3'], f"Drop didn't persist"
    print(f"  ✓ Create/drop operations persist correctly")

def test_metadata_update_persistence():
    """Test that metadata updates (like document_count) persist"""
    cleanup()

    # Insert documents and verify count
    db1 = ironbase.MongoLite(DB_FILE)
    users = db1.collection('users')
    users.insert_many([{'name': f'User{i}'} for i in range(5)])
    count1 = users.count_documents({})
    db1.close()

    # Reopen and verify count still correct
    db2 = ironbase.MongoLite(DB_FILE)
    users2 = db2.collection('users')
    count2 = users2.count_documents({})

    # Insert more
    users2.insert_many([{'name': f'User{i}'} for i in range(5, 8)])
    count3 = users2.count_documents({})
    db2.close()

    # Reopen final time
    db3 = ironbase.MongoLite(DB_FILE)
    users3 = db3.collection('users')
    count4 = users3.count_documents({})
    db3.close()

    assert count1 == 5, f"Expected 5, got {count1}"
    assert count2 == 5, f"Count not persisted, expected 5 got {count2}"
    assert count3 == 8, f"Expected 8, got {count3}"
    assert count4 == 8, f"Final count not persisted, expected 8 got {count4}"
    print(f"  ✓ Metadata persists: {count1} -> {count2} -> {count3} -> {count4}")

def test_collection_isolation():
    """Test that collections remain isolated after reopen"""
    cleanup()

    db1 = ironbase.MongoLite(DB_FILE)
    users = db1.collection('users')
    posts = db1.collection('posts')

    users.insert_many([{'type': 'user', 'name': 'Alice'}])
    posts.insert_many([{'type': 'post', 'title': 'Post 1'}])
    db1.close()

    # Reopen and verify isolation
    db2 = ironbase.MongoLite(DB_FILE)
    users2 = db2.collection('users')
    posts2 = db2.collection('posts')

    user_docs = list(users2.find({}))
    post_docs = list(posts2.find({}))

    db2.close()

    assert len(user_docs) == 1, f"Expected 1 user, got {len(user_docs)}"
    assert len(post_docs) == 1, f"Expected 1 post, got {len(post_docs)}"
    assert user_docs[0]['type'] == 'user', "Got wrong document type in users"
    assert post_docs[0]['type'] == 'post', "Got wrong document type in posts"
    print(f"  ✓ Collections remain isolated after reopen")

# Run all tests
print("\n" + "="*60)
print("COMPREHENSIVE REFACTORING TEST SUITE")
print("="*60)

tests = [
    ("Basic data persistence", test_basic_persistence),
    ("Multi-collection persistence", test_multi_collection_persistence),
    ("Last ID continuation", test_last_id_persistence),
    ("Create/drop collection persistence", test_create_drop_collection),
    ("Metadata update persistence", test_metadata_update_persistence),
    ("Collection isolation after reopen", test_collection_isolation),
]

results = []
for desc, func in tests:
    results.append(test_scenario(desc, func))

# Summary
print("\n" + "="*60)
print("TEST SUMMARY")
print("="*60)
passed = sum(results)
total = len(results)
print(f"\nPassed: {passed}/{total}")

if passed == total:
    print("\n✅ ALL TESTS PASSED! Refactoring is complete and correct.")
else:
    print(f"\n❌ {total - passed} test(s) failed")

cleanup()
