#!/usr/bin/env python3
"""
Import PeTitanKalimpalo.Documents.chunks.json into IronBase
This is a MongoDB GridFS chunks collection export
"""

from ironbase import IronBase
import json
import time
from datetime import datetime

def convert_mongodb_export(doc):
    """Convert MongoDB JSON export format to standard JSON"""
    result = {}

    for key, value in doc.items():
        if isinstance(value, dict):
            # Handle MongoDB extended JSON format
            if "$oid" in value:
                # ObjectId → keep as string
                result[key] = value["$oid"]
            elif "$binary" in value:
                # Binary data → keep base64 string
                result[key] = value["$binary"]["base64"]
            elif "$date" in value:
                # Date → keep as ISO string
                result[key] = value["$date"]
            elif "$numberInt" in value:
                result[key] = int(value["$numberInt"])
            elif "$numberLong" in value:
                result[key] = int(value["$numberLong"])
            else:
                # Nested object
                result[key] = convert_mongodb_export(value)
        elif isinstance(value, list):
            result[key] = [convert_mongodb_export(item) if isinstance(item, dict) else item for item in value]
        else:
            result[key] = value

    return result

def import_chunks():
    print("🚀 Starting import of PeTitanKalimpalo.Documents.chunks.json")
    print("=" * 60)

    # Open database
    db = IronBase("chunks_database.mlite")
    coll = db.collection("chunks")

    print(f"✅ Database opened: chunks_database.mlite")
    print(f"✅ Collection: chunks")
    print()

    # Load JSON file
    json_path = "/home/petitan/Ironbase/PeTitanKalimpalo.Documents.chunks.json"
    print(f"📂 Loading JSON file: {json_path}")

    start_time = time.time()

    with open(json_path, 'r', encoding='utf-8') as f:
        data = json.load(f)

    load_time = time.time() - start_time
    print(f"✅ JSON loaded in {load_time:.2f}s")
    print(f"📊 Total documents: {len(data)}")
    print()

    # Convert and insert in batches
    batch_size = 100
    total = len(data)
    inserted = 0

    print(f"💾 Inserting documents in batches of {batch_size}...")
    print()

    insert_start = time.time()

    for i in range(0, total, batch_size):
        batch = data[i:i+batch_size]

        # Convert MongoDB extended JSON format
        converted_batch = [convert_mongodb_export(doc) for doc in batch]

        # Insert batch
        coll.insert_many(converted_batch)

        inserted += len(batch)
        progress = (inserted / total) * 100

        print(f"  Progress: {inserted}/{total} ({progress:.1f}%) - "
              f"Batch {i//batch_size + 1}/{(total + batch_size - 1)//batch_size}")

    insert_time = time.time() - insert_start
    total_time = time.time() - start_time

    print()
    print("=" * 60)
    print("✅ IMPORT COMPLETE!")
    print(f"📊 Documents inserted: {inserted}")
    print(f"⏱️  Insert time: {insert_time:.2f}s")
    print(f"⏱️  Total time: {total_time:.2f}s")
    print(f"🚀 Throughput: {inserted/insert_time:.0f} docs/sec")
    print()

    # Verify
    count = coll.count_documents()
    print(f"🔍 Verification: {count} documents in collection")

    # Show sample document
    sample = coll.find_one({})
    if sample:
        print()
        print("📄 Sample document:")
        print(json.dumps(sample, indent=2, default=str)[:500] + "...")

    db.close()
    print()
    print("✅ Database closed")

if __name__ == "__main__":
    try:
        import_chunks()
    except Exception as e:
        print(f"❌ ERROR: {e}")
        import traceback
        traceback.print_exc()
        exit(1)
