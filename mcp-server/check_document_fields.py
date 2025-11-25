#!/usr/bin/env python3
"""
Check what fields are actually in the stored document
"""
import sys
sys.path.insert(0, '../ironbase-core/bindings/python')

try:
    import ironbase
    import json

    db = ironbase.IronBase("./docjl_storage.mlite")
    coll = db.collection("documents")

    # Get ALL documents
    all_docs = coll.find({})
    print(f"Found {len(all_docs)} documents")

    if all_docs:
        doc = all_docs[0]
        print(f"\nDocument _id: {doc.get('_id')}")
        print(f"Document has {len(doc)} top-level fields")
        print("\nTop-level fields:")
        for key in sorted(doc.keys()):
            value = doc[key]
            if isinstance(value, list):
                print(f"  {key}: <list with {len(value)} items>")
            elif isinstance(value, dict):
                print(f"  {key}: <dict with {len(value)} keys>")
            else:
                print(f"  {key}: {repr(value)[:100]}")

        # Check specifically for 'id' field
        if 'id' in doc:
            print(f"\n✅ 'id' field EXISTS: {doc['id']}")
        else:
            print("\n❌ 'id' field MISSING!")

except Exception as e:
    print(f"❌ ERROR: {e}")
    import traceback
    traceback.print_exc()
