#!/usr/bin/env python3
"""
Import PeTitanKalimpalo.Documents.chunks.json into IronBase test database
"""
import json
import ironbase
import time

def import_chunks():
    # Load JSON file
    print("Loading JSON file...")
    with open('PeTitanKalimpalo.Documents.chunks.json', 'r', encoding='utf-8') as f:
        chunks = json.load(f)

    print(f"✓ Loaded {len(chunks)} chunks")

    # Create database
    print("\nCreating test database...")
    db = ironbase.IronBase("test_chunks.mlite")
    collection = db.collection("chunks")

    # Import chunks in batches
    print("\nImporting chunks...")
    batch_size = 100
    start_time = time.time()

    for i in range(0, len(chunks), batch_size):
        batch = chunks[i:i+batch_size]
        # Clean up MongoDB-specific fields
        for doc in batch:
            # Convert $oid to string
            if '_id' in doc and isinstance(doc['_id'], dict) and '$oid' in doc['_id']:
                doc['_id'] = doc['_id']['$oid']
            if 'files_id' in doc and isinstance(doc['files_id'], dict) and '$oid' in doc['files_id']:
                doc['files_id'] = doc['files_id']['$oid']

        collection.insert_many(batch)
        print(f"  Imported {min(i+batch_size, len(chunks))}/{len(chunks)} chunks")

    elapsed = time.time() - start_time

    # Verify
    count = collection.count_documents({})
    print(f"\n✓ Import complete!")
    print(f"  Time: {elapsed:.2f}s")
    print(f"  Documents: {count}")
    print(f"  Speed: {count/elapsed:.0f} docs/sec")

    # Show some stats
    print("\n--- Sample Data ---")
    sample = collection.find_one({})
    if sample:
        print(f"First document keys: {list(sample.keys())}")
        if 'n' in sample:
            print(f"Chunk number: {sample['n']}")
        if 'files_id' in sample:
            print(f"Files ID: {sample['files_id']}")

    # Verify count before close
    verify_count = collection.count_documents({})
    print(f"\n  Verification before close: {verify_count} documents")

    # Close database (calls flush internally)
    db.close()
    print("✓ Database closed: test_chunks.mlite")

    # Verify file exists and has size
    import os
    size = os.path.getsize("test_chunks.mlite")
    print(f"✓ Database file size: {size / 1024 / 1024:.2f} MB")

if __name__ == "__main__":
    import_chunks()
