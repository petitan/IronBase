#!/usr/bin/env python3
"""
Clean DOCJL import using ONLY IronBase API.
NO direct file operations, NO shortcuts, NO hacks.
"""
import sys
import os
import json

# Add IronBase to path
sys.path.insert(0, '/home/petitan/MongoLite')
import ironbase

def main():
    # Source file - NESTED format with 675 blocks
    source_file = 'mk_manual_normalized.json'
    db_file = 'docjl_storage.mlite'

    # Remove old DB if exists
    if os.path.exists(db_file):
        print(f"🗑️  Removing old database: {db_file}")
        os.remove(db_file)

    # Load source document
    print(f"📖 Loading source: {source_file}")
    with open(source_file, 'r', encoding='utf-8') as f:
        doc = json.load(f)

    blocks_count = len(doc.get('blocks', []))
    title = doc.get('metadata', {}).get('title', 'Unknown')
    print(f"✅ Loaded document: {title}")
    print(f"   Blocks: {blocks_count}")

    # Create database using ONLY API
    print(f"\n🔨 Creating database: {db_file}")
    db = ironbase.IronBase(db_file)

    # Create collection using ONLY API
    print("📦 Creating collection: documents")
    coll = db.collection('documents')

    # Insert using ONLY API
    print("💾 Inserting document...")
    doc_id = coll.insert_one(doc)
    print(f"✅ Inserted with _id: {doc_id}")

    # Verify using ONLY API
    print("\n🔍 Verifying...")
    count = coll.count_documents({})
    print(f"   Count: {count}")

    docs = list(coll.find({}))
    print(f"   Retrieved: {len(docs)} documents")

    if len(docs) > 0:
        retrieved_doc = docs[0]
        retrieved_blocks = len(retrieved_doc.get('blocks', []))
        retrieved_title = retrieved_doc.get('metadata', {}).get('title', 'Unknown')
        print(f"   Title: {retrieved_title}")
        print(f"   Blocks: {retrieved_blocks}")

        if retrieved_blocks == blocks_count:
            print("\n✅✅✅ SUCCESS! Database created correctly.")
            return 0
        else:
            print(f"\n❌ ERROR: Block count mismatch! Expected {blocks_count}, got {retrieved_blocks}")
            return 1
    else:
        print("\n❌ ERROR: No documents retrieved!")
        return 1

if __name__ == '__main__':
    sys.exit(main())
