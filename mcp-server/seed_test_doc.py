#!/usr/bin/env python3
"""
Seed the database with a simple test document for testing insert positions
"""
import sys
sys.path.insert(0, 'examples')

# Use IronBase directly to seed the database
import sys
sys.path.insert(0, '../ironbase-core/bindings/python')

try:
    import ironbase

    # Open/create database
    db = ironbase.IronBase("./docjl_storage.mlite")

    # Create collection if it doesn't exist
    try:
        db.create_collection("documents")
    except:
        pass  # Collection might already exist

    # Create test document
    test_doc = {
        "id": "mk_manual_v1",
        "metadata": {
            "title": "Test Manual",
            "version": "1.0.0",
            "created_at": "2025-01-01T00:00:00Z",
            "modified_at": "2025-01-01T00:00:00Z"
        },
        "docjll": [
            {
                "type": "heading",
                "level": 1,
                "content": [{"type": "text", "content": "Section 1"}],
                "label": "sec:1"
            },
            {
                "type": "paragraph",
                "content": [{"type": "text", "content": "This is paragraph 1"}],
                "label": "para:1"
            },
            {
                "type": "heading",
                "level": 1,
                "content": [{"type": "text", "content": "Section 2"}],
                "label": "sec:2"
            },
            {
                "type": "paragraph",
                "content": [{"type": "text", "content": "This is paragraph 2"}],
                "label": "para:2"
            },
            {
                "type": "heading",
                "level": 1,
                "content": [{"type": "text", "content": "Section 3"}],
                "label": "sec:3"
            },
            {
                "type": "paragraph",
                "content": [{"type": "text", "content": "This is paragraph 3"}],
                "label": "para:3"
            }
        ]
    }

    # Insert document
    coll = db.get_collection("documents")
    result = coll.insert_one(test_doc)

    print(f"✅ Document inserted successfully!")
    print(f"   Document ID: {result}")
    print(f"   Semantic ID: {test_doc['id']}")
    print(f"   Blocks: {len(test_doc['docjll'])}")

except Exception as e:
    print(f"❌ Failed to seed database: {e}")
    import traceback
    traceback.print_exc()
    sys.exit(1)
