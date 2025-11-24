#!/usr/bin/env python3
"""
Debug: Check if the 'id' index is created properly
"""
import sys
sys.path.insert(0, '../ironbase-core/bindings/python')

try:
    import ironbase

    # Open database
    print("📖 Opening database...")
    db = ironbase.IronBase("./docjl_storage.mlite")

    # Get collection
    print("📁 Getting 'documents' collection...")
    coll = db.collection("documents")

    # Try to create index (should be idempotent)
    print("\n🔧 Creating index on 'id' field...")
    try:
        result = coll.create_index("id", True)  # unique=True
        print(f"   ✅ Index created/verified: {result}")
    except Exception as e:
        print(f"   ⚠️  Index creation error (may already exist): {e}")

    # List all indexes
    print("\n📊 Listing all indexes...")
    try:
        indexes = coll.list_indexes()
        print(f"   Found {len(indexes)} indexes:")
        for idx in indexes:
            print(f"     - {idx}")
    except Exception as e:
        print(f"   ❌ Failed to list indexes: {e}")

    # Test query with index hint
    print("\n🔍 Testing find() with semantic id query...")
    import json
    query = {"id": "mk_manual_v1"}
    print(f"   Query: {json.dumps(query)}")

    try:
        docs = coll.find(query)
        print(f"   ✅ Found {len(docs)} document(s)")
        if docs:
            print(f"      Document _id: {docs[0].get('_id')}")
            print(f"      Document id: {docs[0].get('id')}")
            print(f"      Blocks: {len(docs[0].get('docjll', []))}")
    except Exception as e:
        print(f"   ❌ Query failed: {e}")
        import traceback
        traceback.print_exc()

except Exception as e:
    print(f"\n❌ ERROR: {e}")
    import traceback
    traceback.print_exc()
    sys.exit(1)
