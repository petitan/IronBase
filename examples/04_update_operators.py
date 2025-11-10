#!/usr/bin/env python3
"""
Example 4: Update Operators

Demonstrates all update operators in MongoLite:
- $set - Set field values
- $inc - Increment/decrement numeric fields
- $unset - Remove fields from documents

Run: python examples/04_update_operators.py
"""
import ironbase
import os

DB_FILE = "examples/data/update_ops_example.db"

def setup():
    """Create examples/data directory if it doesn't exist"""
    os.makedirs("examples/data", exist_ok=True)
    if os.path.exists(DB_FILE):
        os.remove(DB_FILE)

def print_user(name, collection):
    """Helper function to print user details"""
    user = collection.find_one({"name": name})
    if user:
        print(f"  {user['name']}:")
        for key, value in user.items():
            if key not in ['_id', 'name']:
                print(f"    {key}: {value}")
    else:
        print(f"  {name}: Not found")

def main():
    setup()

    print("="*60)
    print("MongoLite Example 4: Update Operators")
    print("="*60 + "\n")

    db = ironbase.MongoLite(DB_FILE)
    users = db.collection("users")

    # Insert initial test data
    print("Inserting initial user data...\n")
    users.insert_many([
        {
            "name": "Alice",
            "email": "alice@example.com",
            "age": 28,
            "score": 100,
            "level": 5,
            "temp_field": "will be removed"
        },
        {
            "name": "Bob",
            "email": "bob@example.com",
            "age": 35,
            "score": 250,
            "level": 8
        },
        {
            "name": "Carol",
            "email": "carol@example.com",
            "age": 30,
            "score": 150,
            "level": 6,
            "premium": False
        }
    ])

    # ==================== $set - Update/Add Fields ====================
    print("1. $set - Update existing fields or add new ones\n")

    print("Before update:")
    print_user("Alice", users)
    print()

    # Update existing field and add new field
    result = users.update_one(
        {"name": "Alice"},
        {
            "$set": {
                "age": 29,  # Update existing
                "city": "New York",  # Add new field
                "premium": True  # Add new field
            }
        }
    )

    print(f"✓ Modified {result['modified_count']} document\n")
    print("After update:")
    print_user("Alice", users)
    print()

    # ==================== $set with update_many ====================
    print("2. $set with update_many - Update multiple documents\n")

    print("Before: Users with level < 7")
    for user in users.find({"level": {"$lt": 7}}):
        print(f"  • {user['name']}: level {user['level']}, premium: {user.get('premium', 'N/A')}")
    print()

    # Set premium status for all users with level < 7
    result = users.update_many(
        {"level": {"$lt": 7}},
        {"$set": {"premium": True, "promotion": "Spring2024"}}
    )

    print(f"✓ Modified {result['modified_count']} document(s)\n")
    print("After:")
    for user in users.find({"level": {"$lt": 7}}):
        print(f"  • {user['name']}: premium={user['premium']}, promotion={user['promotion']}")
    print()

    # ==================== $inc - Increment/Decrement ====================
    print("3. $inc - Increment or decrement numeric fields\n")

    print("Bob's current stats:")
    print_user("Bob", users)
    print()

    # Increment score by 50, decrement level by 1
    result = users.update_one(
        {"name": "Bob"},
        {
            "$inc": {
                "score": 50,  # Add 50 to score
                "level": -1,  # Subtract 1 from level
                "login_count": 1  # Will create field if doesn't exist
            }
        }
    )

    print(f"✓ Modified {result['modified_count']} document\n")
    print("Bob's updated stats:")
    print_user("Bob", users)
    print()

    # ==================== $inc with update_many ====================
    print("4. $inc with update_many - Bonus points for everyone!\n")

    print("Scores before bonus:")
    for user in users.find({}):
        print(f"  • {user['name']}: {user['score']} points")
    print()

    # Give 25 bonus points to all users
    result = users.update_many(
        {},
        {"$inc": {"score": 25}}
    )

    print(f"✓ Modified {result['modified_count']} document(s)\n")
    print("Scores after bonus:")
    for user in users.find({}):
        print(f"  • {user['name']}: {user['score']} points (+25)")
    print()

    # ==================== $unset - Remove Fields ====================
    print("5. $unset - Remove fields from documents\n")

    print("Alice before cleanup:")
    print_user("Alice", users)
    print()

    # Remove temporary field
    result = users.update_one(
        {"name": "Alice"},
        {"$unset": {"temp_field": ""}}  # Value doesn't matter for $unset
    )

    print(f"✓ Modified {result['modified_count']} document\n")
    print("Alice after cleanup (temp_field removed):")
    print_user("Alice", users)
    print()

    # ==================== $unset with update_many ====================
    print("6. $unset with update_many - Remove promotion field from all\n")

    print("Before:")
    for user in users.find({"promotion": {"$exists": True}}):
        print(f"  • {user['name']}: promotion = {user['promotion']}")
    print()

    # Remove promotion field from everyone
    result = users.update_many(
        {},
        {"$unset": {"promotion": ""}}
    )

    print(f"✓ Modified {result['modified_count']} document(s)")
    print("Promotion field removed from all users\n")

    # ==================== Combining Multiple Operators ====================
    print("7. Combining operators - Complex update\n")

    print("Carol before complex update:")
    print_user("Carol", users)
    print()

    # Set some fields, increment others, remove others
    result = users.update_one(
        {"name": "Carol"},
        {
            "$set": {
                "status": "active",
                "last_login": "2024-01-15"
            },
            "$inc": {
                "score": 100,
                "level": 2
            },
            "$unset": {
                "premium": ""  # Remove the premium field
            }
        }
    )

    print(f"✓ Modified {result['modified_count']} document\n")
    print("Carol after complex update:")
    print_user("Carol", users)
    print()

    # ==================== Practical Example: Game Score System ====================
    print("8. Practical example - Game session simulation\n")

    # Reset Alice's data for demo
    users.update_one(
        {"name": "Alice"},
        {
            "$set": {
                "score": 0,
                "level": 1,
                "lives": 3,
                "game_status": "playing"
            }
        }
    )

    print("Game session for Alice:")
    print("━" * 40)

    # Round 1
    print("\n[Round 1] Alice completes a level!")
    users.update_one(
        {"name": "Alice"},
        {
            "$inc": {"score": 500, "level": 1},
            "$set": {"last_checkpoint": "Level 2"}
        }
    )
    alice = users.find_one({"name": "Alice"})
    print(f"  Score: {alice['score']}, Level: {alice['level']}, Lives: {alice['lives']}")

    # Round 2
    print("\n[Round 2] Alice takes damage!")
    users.update_one(
        {"name": "Alice"},
        {
            "$inc": {"lives": -1, "score": 100}
        }
    )
    alice = users.find_one({"name": "Alice"})
    print(f"  Score: {alice['score']}, Level: {alice['level']}, Lives: {alice['lives']}")

    # Round 3
    print("\n[Round 3] Alice finds a bonus!")
    users.update_one(
        {"name": "Alice"},
        {
            "$inc": {"score": 1000, "lives": 1},
            "$set": {"has_power_up": True}
        }
    )
    alice = users.find_one({"name": "Alice"})
    print(f"  Score: {alice['score']}, Level: {alice['level']}, Lives: {alice['lives']}")
    print(f"  Power-up: {alice['has_power_up']}")

    # Game over
    print("\n[Game Over] Saving final stats...")
    users.update_one(
        {"name": "Alice"},
        {
            "$set": {"game_status": "completed"},
            "$unset": {"has_power_up": "", "last_checkpoint": ""}
        }
    )

    print("\nFinal stats:")
    print_user("Alice", users)
    print()

    db.close()

    print("="*60)
    print("✅ Example completed successfully!")
    print("="*60)

if __name__ == "__main__":
    main()
