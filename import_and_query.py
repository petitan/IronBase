#!/usr/bin/env python3
"""
Import and query chunks in single session (no close between)
"""
import json
import time
import ironbase

def main():
    # Load JSON
    print("Loading JSON file...")
    with open('PeTitanKalimpalo.Documents.chunks.json', 'r', encoding='utf-8') as f:
        chunks = json.load(f)
    print(f"✓ Loaded {len(chunks)} chunks\n")

    # Create database
    print("Creating/opening database...")
    db = ironbase.IronBase("chunks_live.mlite")
    collection = db.collection("chunks")

    # Import
    print("Importing chunks...")
    batch_size = 100
    for i in range(0, len(chunks), batch_size):
        batch = chunks[i:i+batch_size]
        for doc in batch:
            if '_id' in doc and isinstance(doc['_id'], dict) and '$oid' in doc['_id']:
                doc['_id'] = doc['_id']['$oid']
            if 'files_id' in doc and isinstance(doc['files_id'], dict) and '$oid' in doc['files_id']:
                doc['files_id'] = doc['files_id']['$oid']
        collection.insert_many(batch)
        print(f"  Imported {min(i+batch_size, len(chunks))}/{len(chunks)}")

    print(f"\n✓ Import complete!\n")

    # Query immediately (same session)
    print("=" * 60)
    print("QUERYING (Same Session)")
    print("=" * 60)

    total = collection.count_documents({})
    print(f"\n📊 Total documents: {total}")

    # Get unique files_ids
    files_ids = collection.distinct("files_id")
    print(f"   Unique files: {len(files_ids)}")

    # Show files
    print(f"\n📁 Files in database:")
    for file_id in files_ids[:10]:
        chunk_count = collection.count_documents({"files_id": file_id})
        print(f"   - {file_id}: {chunk_count} chunks")

    # Find first chunk
    if files_ids:
        print(f"\n🔍 First chunk of file {files_ids[0]}:")
        first = collection.find_one({"files_id": files_ids[0], "n": 0})
        if first:
            print(f"   Chunk #: {first.get('n')}")
            print(f"   Data length: {len(first.get('data', ''))} chars")
            print(f"   Keys: {list(first.keys())}")

    # Aggregation
    print(f"\n📈 Aggregation - chunks per file:")
    pipeline = [
        {"$group": {"_id": "$files_id", "count": {"$sum": 1}}},
        {"$sort": {"count": -1}}
    ]
    results = collection.aggregate(pipeline)
    for result in results[:10]:
        print(f"   {result['_id']}: {result['count']} chunks")

    # Close
    print(f"\n" + "=" * 60)
    print("Closing database...")
    db.close()
    print("✓ Database closed: chunks_live.mlite")
    print("=" * 60)

if __name__ == "__main__":
    main()
