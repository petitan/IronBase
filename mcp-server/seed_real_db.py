#!/usr/bin/env python3
"""
Seed real IronBase database with test documents via direct file manipulation
"""

import json
import sys

# Sample DOCJL documents
DOC1 = {
    "_id": "test_doc_1",
    "title": "Test Document 1",
    "version": "1.0",
    "author": "Claude",
    "created_at": "2025-11-21T21:00:00Z",
    "modified_at": "2025-11-21T21:00:00Z",
    "blocks_count": 3,
    "tags": ["test", "demo"],
    "docjll": [
        {
            "type": "heading",
            "level": 1,
            "label": "sec:1",
            "content": [{"type": "text", "content": "Introduction"}]
        },
        {
            "type": "paragraph",
            "label": "para:1",
            "content": [
                {"type": "text", "content": "This is a "},
                {"type": "bold", "content": "test document"},
                {"type": "text", "content": " created for MCP testing."}
            ]
        },
        {
            "type": "heading",
            "level": 2,
            "label": "sec:2",
            "content": [{"type": "text", "content": "Features"}]
        }
    ]
}

DOC2 = {
    "_id": "test_doc_2",
    "title": "Requirements Specification",
    "version": "2.0",
    "author": "AI Assistant",
    "created_at": "2025-11-21T21:00:00Z",
    "modified_at": "2025-11-21T21:00:00Z",
    "blocks_count": 4,
    "tags": ["requirements", "spec"],
    "docjll": [
        {
            "type": "heading",
            "level": 1,
            "label": "sec:1",
            "content": [{"type": "text", "content": "Functional Requirements"}]
        },
        {
            "type": "paragraph",
            "label": "req:1",
            "content": [
                {"type": "text", "content": "The system shall support "},
                {"type": "italic", "content": "real-time collaboration"},
                {"type": "text", "content": " on documents."}
            ]
        },
        {
            "type": "paragraph",
            "label": "req:2",
            "content": [
                {"type": "text", "content": "Cross-reference example: see requirement "},
                {"type": "ref", "target": "req:1"}
            ]
        },
        {
            "type": "heading",
            "level": 2,
            "label": "sec:2",
            "content": [{"type": "text", "content": "Non-Functional Requirements"}]
        }
    ]
}

def main():
    print("🌱 IronBase Seed Script")
    print("="*60)
    print("\n⚠️  This script requires:")
    print("  1. Running MCP server with RealIronBaseAdapter")
    print("  2. IronBase Python bindings installed")
    print("  3. Direct database access")
    print("\n📝 Sample documents prepared:")
    print(f"  - {DOC1['_id']}: {DOC1['title']}")
    print(f"  - {DOC2['_id']}: {DOC2['title']}")
    print("\n" + "="*60)
    print("\n💡 To use these documents:")
    print("  Option A: Use the Python IronBase library to insert them")
    print("  Option B: Use the create_document MCP command (if implemented)")
    print("  Option C: Use integration tests with in-memory adapter")
    print("\n" + "="*60)

    # Try to use ironbase if available
    try:
        import ironbase
        print("\n✅ IronBase Python bindings found!")
        print("   Attempting to seed database...")

        db = ironbase.IronBase("./docjl_storage.mlite")
        coll = db.collection("documents")

        coll.insert_one(DOC1)
        print(f"   ✅ Inserted: {DOC1['_id']}")

        coll.insert_one(DOC2)
        print(f"   ✅ Inserted: {DOC2['_id']}")

        print("\n🎉 Database seeded successfully!")

    except ImportError:
        print("\n⚠️  IronBase Python bindings not installed")
        print("   Run: pip install ironbase")
        print("\n📄 Documents saved to JSON for manual insertion:")

        with open("seed_doc1.json", "w") as f:
            json.dump(DOC1, f, indent=2)
        print("   - seed_doc1.json")

        with open("seed_doc2.json", "w") as f:
            json.dump(DOC2, f, indent=2)
        print("   - seed_doc2.json")

    except Exception as e:
        print(f"\n❌ Error: {e}")
        sys.exit(1)

if __name__ == "__main__":
    main()
