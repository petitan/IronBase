#!/usr/bin/env python3
"""
Example 1: Basic CRUD Operations

Demonstrates the fundamental Create, Read, Update, Delete operations
with IronBase.

Run: python examples/01_basic_crud.py
"""
import ironbase
import os

# Database file
DB_FILE = "examples/data/crud_example.db"

def setup():
    """Create examples/data directory if it doesn't exist"""
    os.makedirs("examples/data", exist_ok=True)
    if os.path.exists(DB_FILE):
        os.remove(DB_FILE)

def main():
    setup()

    print("="*60)
    print("IronBase Example 1: Basic CRUD Operations")
    print("="*60 + "\n")

    # Open database
    db = ironbase.IronBase(DB_FILE)
    users = db.collection("users")

    # ==================== CREATE ====================
    print("1. CREATE - Insert documents\n")

    # Insert one document
    result = users.insert_one({
        "name": "Alice Johnson",
        "email": "alice@example.com",
        "age": 28,
        "role": "developer"
    })
    print(f"✓ Inserted one document with ID: {result['inserted_id']}")

    # Insert multiple documents
    result = users.insert_many([
        {"name": "Bob Smith", "email": "bob@example.com", "age": 35, "role": "manager"},
        {"name": "Carol White", "email": "carol@example.com", "age": 24, "role": "designer"},
        {"name": "Dave Brown", "email": "dave@example.com", "age": 42, "role": "developer"},
    ])
    print(f"✓ Inserted {len(result['inserted_ids'])} documents")
    print(f"  IDs: {result['inserted_ids']}\n")

    # ==================== READ ====================
    print("2. READ - Query documents\n")

    # Find all documents
    print("All users:")
    all_users = users.find({})
    for user in all_users:
        print(f"  • {user['name']} ({user['role']}) - Age: {user['age']}")

    # Find one specific document
    print("\nFind one user named 'Alice Johnson':")
    alice = users.find_one({"name": "Alice Johnson"})
    if alice:
        print(f"  Found: {alice['name']}, {alice['email']}")

    # Find with filters
    print("\nDevelopers only:")
    developers = users.find({"role": "developer"})
    for dev in developers:
        print(f"  • {dev['name']}")

    # Find with comparison operators
    print("\nUsers older than 30:")
    older_users = users.find({"age": {"$gt": 30}})
    for user in older_users:
        print(f"  • {user['name']} - {user['age']} years old")
    print()

    # ==================== UPDATE ====================
    print("3. UPDATE - Modify documents\n")

    # Update one document
    result = users.update_one(
        {"name": "Alice Johnson"},
        {"$set": {"age": 29, "department": "Engineering"}}
    )
    print(f"✓ Updated {result['modified_count']} document")

    # Verify the update
    alice_updated = users.find_one({"name": "Alice Johnson"})
    print(f"  Alice's new age: {alice_updated['age']}")
    print(f"  Alice's department: {alice_updated.get('department', 'N/A')}")

    # Update multiple documents
    result = users.update_many(
        {"role": "developer"},
        {"$set": {"team": "Backend"}}
    )
    print(f"\n✓ Updated {result['modified_count']} developers with team assignment")

    # Increment a field
    result = users.update_one(
        {"name": "Bob Smith"},
        {"$inc": {"age": 1}}  # Happy birthday, Bob!
    )
    bob = users.find_one({"name": "Bob Smith"})
    print(f"✓ Bob's new age after increment: {bob['age']}\n")

    # ==================== DELETE ====================
    print("4. DELETE - Remove documents\n")

    print(f"Users before delete: {users.count_documents({})}")

    # Delete one document
    result = users.delete_one({"name": "Carol White"})
    print(f"✓ Deleted {result['deleted_count']} document (Carol)")

    print(f"Users after delete: {users.count_documents({})}")

    # Delete multiple documents
    result = users.delete_many({"age": {"$gt": 40}})
    print(f"✓ Deleted {result['deleted_count']} user(s) over 40")

    print(f"Final user count: {users.count_documents({})}")

    # Show remaining users
    print("\nRemaining users:")
    for user in users.find({}):
        print(f"  • {user['name']} - {user['role']}")

    # Close database
    db.close()

    print("\n" + "="*60)
    print("✅ Example completed successfully!")
    print("="*60)

if __name__ == "__main__":
    main()
