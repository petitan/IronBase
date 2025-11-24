#!/usr/bin/env python3
"""
Debug IronBase secondary index behavior
Tests if the index is properly populated and queried
"""
import sys
sys.path.insert(0, '../ironbase-core/bindings/python')

try:
    import ironbase
    import json

    print("=== IronBase Secondary Index Debug ===\n")

    # Open database
    print("Step 1: Opening database...")
    db = ironbase.IronBase("./docjl_storage.mlite")

    # Get collection
    print("Step 2: Getting 'documents' collection...")
    coll = db.collection("documents")

    # Check if documents exist
    print("\nStep 3: Checking existing documents...")
    all_docs = coll.find({})
    print(f"   Total documents in collection: {len(all_docs)}")

    if all_docs:
        doc = all_docs[0]
        print(f"   First document _id: {doc.get('_id')}")
        print(f"   First document id field: {repr(doc.get('id'))}")
        print(f"   First document id type: {type(doc.get('id'))}")

    # List existing indexes
    print("\nStep 4: Listing existing indexes...")
    try:
        indexes = coll.list_indexes()
        print(f"   Found {len(indexes)} indexes: {indexes}")
    except Exception as e:
        print(f"   ❌ Failed to list indexes: {e}")
        indexes = []

    # Create index (idempotent)
    print("\nStep 5: Creating/verifying secondary index on 'id' field...")
    try:
        result = coll.create_index("id", True)  # unique=True
        print(f"   ✅ Index result: {result}")
    except Exception as e:
        print(f"   ⚠️  Index creation error: {e}")

    # List indexes again
    print("\nStep 6: Listing indexes after creation...")
    try:
        indexes = coll.list_indexes()
        print(f"   Found {len(indexes)} indexes: {indexes}")

        # Check if our index exists
        expected_index = "documents_id"
        if expected_index in indexes:
            print(f"   ✅ Index '{expected_index}' exists")
        else:
            print(f"   ❌ Index '{expected_index}' NOT FOUND!")
    except Exception as e:
        print(f"   ❌ Failed to list indexes: {e}")

    # Test query with the indexed field
    print("\nStep 7: Testing indexed query...")
    query = {"id": "mk_manual_v1"}
    print(f"   Query: {json.dumps(query)}")

    try:
        docs = coll.find(query)
        print(f"   ✅ Query executed successfully")
        print(f"   Results: {len(docs)} document(s) found")

        if docs:
            print(f"   First result _id: {docs[0].get('_id')}")
            print(f"   First result id: {docs[0].get('id')}")
        else:
            print(f"   ❌ NO DOCUMENTS FOUND despite index!")

    except Exception as e:
        print(f"   ❌ Query failed: {e}")
        import traceback
        traceback.print_exc()

    # Test full scan as baseline
    print("\nStep 8: Testing full scan (baseline)...")
    try:
        all_docs = coll.find({})
        print(f"   ✅ Full scan found {len(all_docs)} document(s)")

        # Manually search for mk_manual_v1
        matching = [d for d in all_docs if d.get('id') == 'mk_manual_v1']
        print(f"   Manual search found {len(matching)} document(s) with id='mk_manual_v1'")

    except Exception as e:
        print(f"   ❌ Full scan failed: {e}")

    print("\n=== Debug Complete ===")

except Exception as e:
    print(f"\n❌ FATAL ERROR: {e}")
    import traceback
    traceback.print_exc()
    sys.exit(1)
