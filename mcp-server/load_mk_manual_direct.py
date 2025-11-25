#!/usr/bin/env python3
"""
Load mk_manual_v1 directly into IronBase database
"""
import sys
import json

# Add Python bindings to path
sys.path.insert(0, '../ironbase-core/bindings/python')

try:
    import ironbase

    # Load the JSON document
    print("📖 Loading mk_manual_final.json...")
    with open('mk_manual_final.json', 'r', encoding='utf-8') as f:
        doc = json.load(f)

    print(f"   ✅ Loaded document with {len(doc['docjll'])} blocks")
    print(f"   Document ID: {doc['id']}")
    print(f"   Title: {doc['metadata']['title']}")

    # Open/create database
    print("\n🗄️  Opening IronBase database...")
    db = ironbase.IronBase("./docjl_storage.mlite")
    print("   ✅ Database opened")

    # Get or create collection
    print("\n📁 Getting 'documents' collection...")
    try:
        coll = db.collection("documents")
        print("   ✅ Collection exists")
    except:
        print("   Creating new collection...")
        db.create_collection("documents")
        coll = db.collection("documents")
        print("   ✅ Collection created")

    # Insert document
    print("\n💾 Inserting document into database...")
    result = coll.insert_one(doc)

    print(f"\n✅ SUCCESS! Document loaded into database")
    print(f"   Database ID (_id): {result}")
    print(f"   Semantic ID (id): {doc['id']}")
    print(f"   Total blocks: {len(doc['docjll'])}")

    # Verify by counting
    count = coll.count_documents({})
    print(f"\n📊 Collection now has {count} document(s)")

except Exception as e:
    print(f"\n❌ ERROR: {e}")
    import traceback
    traceback.print_exc()
    sys.exit(1)
