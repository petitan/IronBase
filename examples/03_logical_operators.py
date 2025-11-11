#!/usr/bin/env python3
"""
Example 3: Logical Operators

Demonstrates all logical operators in IronBase:
- $and (logical AND - all conditions must match)
- $or (logical OR - at least one condition must match)
- $not (logical NOT - negates a condition)
- $nor (logical NOR - none of the conditions must match)

Run: python examples/03_logical_operators.py
"""
import ironbase
import os

DB_FILE = "examples/data/logical_ops_example.db"

def setup():
    """Create examples/data directory if it doesn't exist"""
    os.makedirs("examples/data", exist_ok=True)
    if os.path.exists(DB_FILE):
        os.remove(DB_FILE)

def main():
    setup()

    print("="*60)
    print("IronBase Example 3: Logical Operators")
    print("="*60 + "\n")

    db = ironbase.IronBase(DB_FILE)
    employees = db.collection("employees")

    # Insert test data
    print("Inserting employee data...\n")
    employees.insert_many([
        {"name": "Alice", "department": "Engineering", "age": 28, "salary": 85000, "active": True},
        {"name": "Bob", "department": "Engineering", "age": 35, "salary": 95000, "active": True},
        {"name": "Carol", "department": "Marketing", "age": 30, "salary": 70000, "active": True},
        {"name": "Dave", "department": "Sales", "age": 42, "salary": 65000, "active": False},
        {"name": "Eve", "department": "Engineering", "age": 25, "salary": 75000, "active": True},
        {"name": "Frank", "department": "Marketing", "age": 38, "salary": 80000, "active": True},
        {"name": "Grace", "department": "Sales", "age": 29, "salary": 68000, "active": True},
        {"name": "Henry", "department": "Engineering", "age": 45, "salary": 110000, "active": False},
    ])

    # ==================== $and (Logical AND) ====================
    print("1. $and (Logical AND) - All conditions must match\n")
    print("Find active Engineering employees with salary > $80,000\n")

    results = employees.find({
        "$and": [
            {"department": "Engineering"},
            {"active": True},
            {"salary": {"$gt": 80000}}
        ]
    })

    print(f"Found {len(results)} employee(s):")
    for emp in results:
        print(f"  • {emp['name']}")
        print(f"    Salary: ${emp['salary']:,}")
        print(f"    Age: {emp['age']}")
    print()

    # Note: Implicit AND (same as above)
    print("Note: Multiple conditions at the same level are implicitly AND:\n")
    results2 = employees.find({
        "department": "Engineering",
        "active": True,
        "salary": {"$gt": 80000}
    })
    print(f"Implicit AND also found {len(results2)} employee(s) (same result)\n")

    # ==================== $or (Logical OR) ====================
    print("2. $or (Logical OR) - At least one condition must match\n")
    print("Find employees in Sales OR Marketing departments\n")

    results = employees.find({
        "$or": [
            {"department": "Sales"},
            {"department": "Marketing"}
        ]
    })

    print(f"Found {len(results)} employee(s):")
    for emp in results:
        print(f"  • {emp['name']} - {emp['department']}")
    print()

    # More complex OR
    print("3. Complex $or - Find young (<30) OR high earners (>$90k)\n")

    results = employees.find({
        "$or": [
            {"age": {"$lt": 30}},
            {"salary": {"$gt": 90000}}
        ]
    })

    print(f"Found {len(results)} employee(s):")
    for emp in results:
        reason = []
        if emp['age'] < 30:
            reason.append(f"age {emp['age']}")
        if emp['salary'] > 90000:
            reason.append(f"salary ${emp['salary']:,}")
        print(f"  • {emp['name']} ({', '.join(reason)})")
    print()

    # ==================== $not (Logical NOT) ====================
    print("4. $not (Logical NOT) - Negates a condition\n")
    print("Find employees NOT older than 35\n")

    results = employees.find({
        "age": {"$not": {"$gt": 35}}
    })

    print(f"Found {len(results)} employee(s) aged 35 or younger:")
    for emp in results:
        print(f"  • {emp['name']} - Age: {emp['age']}")
    print()

    # ==================== $nor (Logical NOR) ====================
    print("5. $nor (Logical NOR) - None of the conditions must match\n")
    print("Find employees who are NEITHER in Sales NOR inactive\n")

    results = employees.find({
        "$nor": [
            {"department": "Sales"},
            {"active": False}
        ]
    })

    print(f"Found {len(results)} employee(s):")
    for emp in results:
        print(f"  • {emp['name']} - {emp['department']} (active: {emp['active']})")
    print()

    # ==================== Combining AND + OR ====================
    print("6. Combining $and + $or - Complex nested logic\n")
    print("Find: (Engineering OR Marketing) AND active AND salary > $70k\n")

    results = employees.find({
        "$and": [
            {
                "$or": [
                    {"department": "Engineering"},
                    {"department": "Marketing"}
                ]
            },
            {"active": True},
            {"salary": {"$gt": 70000}}
        ]
    })

    print(f"Found {len(results)} employee(s):")
    for emp in results:
        print(f"  • {emp['name']}")
        print(f"    Department: {emp['department']}")
        print(f"    Salary: ${emp['salary']:,}")
    print()

    # ==================== Complex Real-World Query ====================
    print("7. Real-world query - Find candidates for promotion\n")
    print("Criteria: Active employees who are either:")
    print("  a) In Engineering with salary < $100k and age >= 30, OR")
    print("  b) In any department with salary < $75k and age >= 35\n")

    results = employees.find({
        "$and": [
            {"active": True},
            {
                "$or": [
                    {
                        "$and": [
                            {"department": "Engineering"},
                            {"salary": {"$lt": 100000}},
                            {"age": {"$gte": 30}}
                        ]
                    },
                    {
                        "$and": [
                            {"salary": {"$lt": 75000}},
                            {"age": {"$gte": 35}}
                        ]
                    }
                ]
            }
        ]
    })

    print(f"🎯 {len(results)} candidate(s) for promotion:")
    for emp in results:
        print(f"\n  • {emp['name']}")
        print(f"    Department: {emp['department']}")
        print(f"    Age: {emp['age']}")
        print(f"    Current Salary: ${emp['salary']:,}")

        # Determine reason
        if emp['department'] == 'Engineering' and emp['salary'] < 100000 and emp['age'] >= 30:
            print(f"    Reason: Senior engineer, room for salary growth")
        elif emp['salary'] < 75000 and emp['age'] >= 35:
            print(f"    Reason: Experienced, below market rate")
    print()

    # ==================== NOT with OR ====================
    print("8. $not with $or - Find employees NOT in common scenarios\n")
    print("Find: NOT (young age < 30 OR low salary < $70k)\n")
    print("In other words: age >= 30 AND salary >= $70k\n")

    # Using nested NOT
    results = employees.find({
        "$and": [
            {"age": {"$not": {"$lt": 30}}},
            {"salary": {"$not": {"$lt": 70000}}}
        ]
    })

    print(f"Found {len(results)} experienced, well-paid employee(s):")
    for emp in results:
        print(f"  • {emp['name']} - Age: {emp['age']}, Salary: ${emp['salary']:,}")
    print()

    db.close()

    print("="*60)
    print("✅ Example completed successfully!")
    print("="*60)

if __name__ == "__main__":
    main()
