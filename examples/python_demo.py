#!/usr/bin/env python3
"""
IronBase Python Demo - Comprehensive Feature Showcase
Equivalent to the C# Demo in IronBase.NET/Demo/Program.cs
"""

import os
import ironbase

# ============================================================
# Helper Functions
# ============================================================

def print_section(title):
    print()
    print("─" * 58)
    print(f"  {title}")
    print("─" * 58)

def print_users(users, prefix=""):
    print(f"{prefix}Found {len(users)} users:")
    for u in users:
        print(f"{prefix}- {u.get('Name')}, {u.get('Age')}, {u.get('City')}")

def print_accounts(accounts):
    all_accts = accounts.find({})
    for a in all_accts:
        print(f"    {a.get('Owner')} ({a.get('AccountId')}): ${a.get('Balance')}")

# ============================================================
# Main Demo
# ============================================================

def main():
    print("╔══════════════════════════════════════════════════════════╗")
    print("║          IronBase Python Demo - Feature Showcase         ║")
    print("╚══════════════════════════════════════════════════════════╝")
    print()

    # Clean up old database file
    db_path = "/tmp/demo.mlite"
    if os.path.exists(db_path):
        os.remove(db_path)
    if os.path.exists(db_path + ".wal"):
        os.remove(db_path + ".wal")

    # Open database
    db = ironbase.IronBase(db_path)

    print(f"Database path: {db_path}")
    print()

    # ============================================================
    # 1. BASIC CRUD OPERATIONS
    # ============================================================
    print_section("1. BASIC CRUD OPERATIONS")

    users = db.collection("users")

    # Insert One
    print(">>> InsertOne")
    result = users.insert_one({
        "Name": "Alice",
        "Age": 30,
        "City": "New York",
        "Email": "alice@example.com",
        "Tags": ["developer", "team-lead"],
        "Profile": {"Score": 95, "Level": "senior"}
    })
    print(f"    Inserted ID: {result}")

    # Insert Many
    print("\n>>> InsertMany")
    many_result = users.insert_many([
        {"Name": "Bob", "Age": 25, "City": "Los Angeles", "Email": "bob@example.com",
         "Tags": ["developer"], "Profile": {"Score": 82, "Level": "mid"}},
        {"Name": "Carol", "Age": 35, "City": "New York", "Email": "carol@example.com",
         "Tags": ["manager", "team-lead"], "Profile": {"Score": 88, "Level": "senior"}},
        {"Name": "David", "Age": 28, "City": "Chicago", "Email": "david@example.com",
         "Tags": ["developer", "intern"], "Profile": {"Score": 75, "Level": "junior"}},
        {"Name": "Eve", "Age": 32, "City": "New York", "Email": "eve@example.com",
         "Tags": ["developer", "devops"], "Profile": {"Score": 91, "Level": "senior"}},
        {"Name": "Frank", "Age": 45, "City": "Boston", "Email": "frank@example.com",
         "Tags": ["architect"], "Profile": {"Score": 98, "Level": "principal"}},
    ])
    print(f"    Inserted count: {len(many_result)}")

    # Find All
    print("\n>>> Find (all)")
    all_users = users.find({})
    print(f"    Total users: {len(all_users)}")
    for u in all_users:
        print(f"    - {u.get('Name')}, {u.get('Age')}, {u.get('City')}")

    # Find One
    print("\n>>> FindOne")
    alice = users.find_one({"Name": "Alice"})
    print(f"    Found: {alice.get('Name')}, Age: {alice.get('Age')}")

    # Count Documents
    print("\n>>> CountDocuments")
    count = users.count_documents({})
    print(f"    Total documents: {count}")

    # ============================================================
    # 2. QUERY OPERATORS
    # ============================================================
    print_section("2. QUERY OPERATORS")

    # Comparison operators: $gt, $gte, $lt, $lte
    print(">>> Age >= 30 (using $gte)")
    senior_users = users.find({"Age": {"$gte": 30}})
    print_users(senior_users, "    ")

    print("\n>>> Age < 30 (using $lt)")
    young_users = users.find({"Age": {"$lt": 30}})
    print_users(young_users, "    ")

    # $ne - Not Equal
    print("\n>>> City != 'New York' (using $ne)")
    not_ny_users = users.find({"City": {"$ne": "New York"}})
    print_users(not_ny_users, "    ")

    # $in - In array
    print("\n>>> City in ['New York', 'Boston'] (using $in)")
    ny_or_boston = users.find({"City": {"$in": ["New York", "Boston"]}})
    print_users(ny_or_boston, "    ")

    # $exists - Field exists
    print("\n>>> Email exists (using $exists)")
    with_email = users.find({"Email": {"$exists": True}})
    print(f"    Users with email: {len(with_email)}")

    # $regex - Regex match
    print("\n>>> Name starts with 'A' or 'B' (using $regex)")
    ab_names = users.find({"Name": {"$regex": "^[AB]"}})
    print_users(ab_names, "    ")

    # Logical operators: $and, $or
    print("\n>>> (Age >= 30 AND City = 'New York') using $and")
    senior_ny = users.find({"$and": [
        {"Age": {"$gte": 30}},
        {"City": "New York"}
    ]})
    print_users(senior_ny, "    ")

    print("\n>>> (City = 'Chicago' OR City = 'Boston') using $or")
    chicago_or_boston = users.find({"$or": [
        {"City": "Chicago"},
        {"City": "Boston"}
    ]})
    print_users(chicago_or_boston, "    ")

    # Using JSON filter - dot notation
    print("\n>>> Using dot notation filter: Profile.Score > 90")
    high_scorers = users.find({"Profile.Score": {"$gt": 90}})
    print_users(high_scorers, "    ")

    # ============================================================
    # 3. UPDATE OPERATORS
    # ============================================================
    print_section("3. UPDATE OPERATORS")

    # $set - Set field value
    print(">>> $set: Update Alice's age to 31")
    update_result = users.update_one(
        {"Name": "Alice"},
        {"$set": {"Age": 31}}
    )
    print(f"    Matched: {update_result.get('matched_count')}, Modified: {update_result.get('modified_count')}")

    # $inc - Increment
    print("\n>>> $inc: Increment Bob's Profile.Score by 5")
    users.update_one(
        {"Name": "Bob"},
        {"$inc": {"Profile.Score": 5}}
    )
    bob = users.find_one({"Name": "Bob"})
    print(f"    Bob's new score: {bob.get('Profile', {}).get('Score')}")

    # $push - Add to array
    print("\n>>> $push: Add 'speaker' tag to Carol")
    users.update_one(
        {"Name": "Carol"},
        {"$push": {"Tags": "speaker"}}
    )
    carol = users.find_one({"Name": "Carol"})
    print(f"    Carol's tags: {carol.get('Tags')}")

    # $pull - Remove from array
    print("\n>>> $pull: Remove 'intern' tag from David")
    users.update_one(
        {"Name": "David"},
        {"$pull": {"Tags": "intern"}}
    )
    david = users.find_one({"Name": "David"})
    print(f"    David's tags: {david.get('Tags')}")

    # $addToSet - Add unique value to array
    print("\n>>> $addToSet: Add 'mentor' to Eve (unique only)")
    users.update_one(
        {"Name": "Eve"},
        {"$addToSet": {"Tags": "mentor"}}
    )
    # Try adding again - should not duplicate
    users.update_one(
        {"Name": "Eve"},
        {"$addToSet": {"Tags": "mentor"}}
    )
    eve = users.find_one({"Name": "Eve"})
    print(f"    Eve's tags: {eve.get('Tags')}")

    # UpdateMany
    print("\n>>> UpdateMany: Add 'verified' status to all NY users")
    update_many_result = users.update_many(
        {"City": "New York"},
        {"$set": {"Verified": True}}
    )
    print(f"    Matched: {update_many_result.get('matched_count')}, Modified: {update_many_result.get('modified_count')}")

    # ============================================================
    # 4. FIND OPTIONS (Projection, Sort, Limit, Skip)
    # ============================================================
    print_section("4. FIND OPTIONS")

    # Sort
    print(">>> Sort by Age ascending")
    sorted_by_age = users.find({}, sort=[("Age", 1)])
    for u in sorted_by_age:
        print(f"    {u.get('Name')}: {u.get('Age')}")

    print("\n>>> Sort by City asc, Age desc")
    multi_sort = users.find({}, sort=[("City", 1), ("Age", -1)])
    for u in multi_sort:
        print(f"    {u.get('Name')}: {u.get('City')}, {u.get('Age')}")

    # Limit & Skip (Pagination)
    print("\n>>> Pagination: Skip 2, Limit 2")
    page = users.find({}, skip=2, limit=2)
    print(f"    Results: {len(page)}")
    for u in page:
        print(f"    - {u.get('Name')}")

    # Projection
    print("\n>>> Projection: Only Name and City")
    projected = users.find(
        {},
        projection={"Name": 1, "City": 1, "_id": 0},
        limit=3
    )
    print(f"    Returned {len(projected)} documents with limited fields")

    # ============================================================
    # 5. INDEXING
    # ============================================================
    print_section("5. INDEXING")

    # Create single field index
    print(">>> Create index on 'Email' (unique)")
    email_idx = users.create_index("Email", unique=True)
    print(f"    Created: {email_idx}")

    print("\n>>> Create index on 'Age'")
    age_idx = users.create_index("Age")
    print(f"    Created: {age_idx}")

    # Create compound index
    print("\n>>> Create compound index on ['City', 'Age']")
    compound_idx = users.create_compound_index(["City", "Age"])
    print(f"    Created: {compound_idx}")

    # List indexes
    print("\n>>> List all indexes")
    indexes = users.list_indexes()
    for idx in indexes:
        print(f"    - {idx}")

    # Explain query
    print("\n>>> Explain query: Age = 30")
    plan = users.explain({"Age": 30})
    print(f"    Query plan: {plan}")

    # ============================================================
    # 6. AGGREGATION PIPELINE
    # ============================================================
    print_section("6. AGGREGATION PIPELINE")

    # Simple aggregation: Group by city
    print(">>> Group users by City, count and avg age")
    city_stats = users.aggregate([
        {"$group": {
            "_id": "$City",
            "count": {"$sum": 1},
            "avgAge": {"$avg": "$Age"},
            "maxAge": {"$max": "$Age"}
        }},
        {"$sort": {"count": -1}}
    ])
    for stat in city_stats:
        avg_age = stat.get('avgAge')
        avg_str = f"{avg_age:.1f}" if avg_age else "N/A"
        print(f"    {stat.get('_id')}: {stat.get('count')} users, avg age: {avg_str}, max: {stat.get('maxAge')}")

    # Pipeline with $match, $group, $project
    print("\n>>> Pipeline: Match senior (Age >= 30), group by City")
    senior_stats = users.aggregate([
        {"$match": {"Age": {"$gte": 30}}},
        {"$group": {
            "_id": "$City",
            "count": {"$sum": 1},
            "avgScore": {"$avg": "$Profile.Score"}
        }},
        {"$sort": {"avgScore": -1}},
        {"$limit": 3}
    ])
    for stat in senior_stats:
        avg = stat.get('avgScore')
        avg_str = f"{avg:.1f}" if avg else "N/A"
        print(f"    {stat.get('_id')}: {stat.get('count')} senior users, avg score: {avg_str}")

    # $project stage - Reshape documents
    print("\n>>> $project: Rename fields and exclude _id")
    projected_users = users.aggregate([
        {"$project": {
            "_id": 0,
            "fullName": "$Name",
            "years": "$Age",
            "location": "$City"
        }},
        {"$limit": 3}
    ])
    for pu in projected_users:
        print(f"    {pu.get('fullName')}: {pu.get('years')} years old, from {pu.get('location')}")

    # $skip stage - Pagination
    print("\n>>> $skip + $limit: Skip first 2, get next 2 (pagination)")
    skipped = users.aggregate([
        {"$sort": {"Age": 1}},
        {"$skip": 2},
        {"$limit": 2},
        {"$project": {"_id": "$Name", "age": "$Age"}}
    ])
    for s in skipped:
        print(f"    {s.get('_id')}: age {s.get('age')}")

    # $min/$max accumulators
    print("\n>>> $min/$max: Min and max age per city")
    min_max_stats = users.aggregate([
        {"$group": {
            "_id": "$City",
            "minAge": {"$min": "$Age"},
            "maxAge": {"$max": "$Age"},
            "avgAge": {"$avg": "$Age"}
        }},
        {"$sort": {"_id": 1}}
    ])
    for m in min_max_stats:
        avg = m.get('avgAge')
        avg_str = f"{avg:.1f}" if avg else "N/A"
        print(f"    {m.get('_id')}: min={m.get('minAge')}, max={m.get('maxAge')}, avg={avg_str}")

    # $first/$last accumulators
    print("\n>>> $first/$last: First and last user name per city (sorted by age)")
    first_last_stats = users.aggregate([
        {"$sort": {"Age": 1}},
        {"$group": {
            "_id": "$City",
            "youngest": {"$first": "$Name"},
            "oldest": {"$last": "$Name"},
            "count": {"$sum": 1}
        }},
        {"$sort": {"count": -1}}
    ])
    for f in first_last_stats:
        print(f"    {f.get('_id')}: youngest={f.get('youngest')}, oldest={f.get('oldest')} ({f.get('count')} users)")

    # Complex multi-stage pipeline
    print("\n>>> Complex pipeline: $match -> $group -> $sort -> $skip -> $limit")
    complex_pipeline = users.aggregate([
        {"$match": {"Profile.Score": {"$gte": 75}}},
        {"$group": {
            "_id": "$Profile.Level",
            "userCount": {"$sum": 1},
            "avgScore": {"$avg": "$Profile.Score"},
            "minScore": {"$min": "$Profile.Score"},
            "maxScore": {"$max": "$Profile.Score"}
        }},
        {"$sort": {"avgScore": -1}},
        {"$skip": 0},
        {"$limit": 5}
    ])
    for c in complex_pipeline:
        avg = c.get('avgScore')
        avg_str = f"{avg:.1f}" if avg else "N/A"
        print(f"    {c.get('_id')}: {c.get('userCount')} users, avg={avg_str}, min={c.get('minScore')}, max={c.get('maxScore')}")

    # ===== NEW: Expression Operators in $project =====
    print("\n--- Expression Operators (NEW!) ---")

    # $subtract - Calculate score range per level
    print("\n>>> $subtract: Calculate score range (maxScore - minScore)")
    score_range = users.aggregate([
        {"$group": {
            "_id": "$Profile.Level",
            "minScore": {"$min": "$Profile.Score"},
            "maxScore": {"$max": "$Profile.Score"}
        }},
        {"$project": {
            "_id": 1,
            "minScore": 1,
            "maxScore": 1,
            "scoreRange": {"$subtract": ["$maxScore", "$minScore"]}
        }},
        {"$sort": {"scoreRange": -1}}
    ])
    for sr in score_range:
        print(f"    {sr.get('_id')}: range={sr.get('scoreRange')} (min={sr.get('minScore')}, max={sr.get('maxScore')})")

    # $add - Calculate total value (age + score)
    print("\n>>> $add: Calculate age + score = total value")
    total_values = users.aggregate([
        {"$project": {
            "name": "$Name",
            "totalValue": {"$add": ["$Age", "$Profile.Score"]}
        }},
        {"$sort": {"totalValue": -1}},
        {"$limit": 3}
    ])
    for tv in total_values:
        print(f"    {tv.get('name')}: totalValue={tv.get('totalValue')}")

    # $multiply and $divide - Calculate weighted score
    print("\n>>> $multiply & $divide: Calculate weighted score = (score * 10) / age")
    weighted_scores = users.aggregate([
        {"$project": {
            "name": "$Name",
            "age": "$Age",
            "score": "$Profile.Score",
            "weightedScore": {"$divide": [{"$multiply": ["$Profile.Score", 10]}, "$Age"]}
        }},
        {"$sort": {"weightedScore": -1}},
        {"$limit": 3}
    ])
    for ws in weighted_scores:
        print(f"    {ws.get('name')}: weighted={ws.get('weightedScore'):.2f} (age={ws.get('age')}, score={ws.get('score')})")

    # $concat - Build full description string
    print("\n>>> $concat: Build user description string")
    descriptions = users.aggregate([
        {"$project": {
            "description": {"$concat": ["$Name", " from ", "$City", " (Level: ", "$Profile.Level", ")"]}
        }},
        {"$limit": 3}
    ])
    for d in descriptions:
        print(f"    {d.get('description')}")

    # Nested expressions - Complex calculation
    print("\n>>> Nested expressions: (score - 50) * 2 + age")
    nested_calc = users.aggregate([
        {"$project": {
            "name": "$Name",
            "complexValue": {"$add": [{"$multiply": [{"$subtract": ["$Profile.Score", 50]}, 2]}, "$Age"]}
        }},
        {"$sort": {"complexValue": -1}},
        {"$limit": 3}
    ])
    for nc in nested_calc:
        print(f"    {nc.get('name')}: complexValue={nc.get('complexValue')}")

    # ============================================================
    # 7. NESTED DOCUMENTS (Dot Notation)
    # ============================================================
    print_section("7. NESTED DOCUMENTS (Dot Notation)")

    companies = db.collection("companies")

    print(">>> Insert companies with nested documents")
    companies.insert_many([
        {
            "Name": "TechCorp",
            "Location": {
                "Country": "USA",
                "City": "San Francisco",
                "Address": {"Street": "123 Tech Blvd", "Zip": "94105"}
            },
            "Stats": {"Employees": 500, "Revenue": 50000000, "Rating": 4.5}
        },
        {
            "Name": "DataSoft",
            "Location": {
                "Country": "USA",
                "City": "New York",
                "Address": {"Street": "456 Data Ave", "Zip": "10001"}
            },
            "Stats": {"Employees": 200, "Revenue": 20000000, "Rating": 4.2}
        },
        {
            "Name": "CloudNet",
            "Location": {
                "Country": "Germany",
                "City": "Berlin",
                "Address": {"Street": "789 Cloud Str", "Zip": "10115"}
            },
            "Stats": {"Employees": 150, "Revenue": 15000000, "Rating": 4.8}
        },
        {
            "Name": "AILabs",
            "Location": {
                "Country": "USA",
                "City": "Boston",
                "Address": {"Street": "321 AI Road", "Zip": "02101"}
            },
            "Stats": {"Employees": 300, "Revenue": 35000000, "Rating": 4.6}
        }
    ])
    print("    Inserted 4 companies with nested location and stats")

    # Query nested field with dot notation
    print("\n>>> Query: Location.Country = 'USA' (dot notation)")
    us_companies = companies.find({"Location.Country": "USA"})
    for c in us_companies:
        loc = c.get('Location', {})
        print(f"    - {c.get('Name')} in {loc.get('City')}")

    print("\n>>> Query: Location.City = 'New York'")
    ny_companies = companies.find({"Location.City": "New York"})
    for c in ny_companies:
        loc = c.get('Location', {})
        addr = loc.get('Address', {})
        print(f"    - {c.get('Name')}: {addr.get('Street')}")

    # Query deeply nested field
    print("\n>>> Query: Location.Address.Zip starts with '10' (regex on nested)")
    zip10_companies = companies.find({"Location.Address.Zip": {"$regex": "^10"}})
    for c in zip10_companies:
        loc = c.get('Location', {})
        addr = loc.get('Address', {})
        print(f"    - {c.get('Name')}: ZIP {addr.get('Zip')}")

    # Query nested numeric field
    print("\n>>> Query: Stats.Employees >= 200")
    large_companies = companies.find({"Stats.Employees": {"$gte": 200}})
    for c in large_companies:
        stats = c.get('Stats', {})
        print(f"    - {c.get('Name')}: {stats.get('Employees')} employees")

    print("\n>>> Query: Stats.Rating > 4.5")
    high_rated = companies.find({"Stats.Rating": {"$gt": 4.5}})
    for c in high_rated:
        stats = c.get('Stats', {})
        print(f"    - {c.get('Name')}: rating {stats.get('Rating')}")

    # Update nested field with dot notation
    print("\n>>> Update: Set TechCorp's Stats.Rating to 4.9")
    companies.update_one(
        {"Name": "TechCorp"},
        {"$set": {"Stats.Rating": 4.9}}
    )
    tech_corp = companies.find_one({"Name": "TechCorp"})
    print(f"    TechCorp new rating: {tech_corp.get('Stats', {}).get('Rating')}")

    # Update deeply nested field
    print("\n>>> Update: Change DataSoft's Location.Address.Street")
    companies.update_one(
        {"Name": "DataSoft"},
        {"$set": {"Location.Address.Street": "789 New Data Plaza"}}
    )
    data_soft = companies.find_one({"Name": "DataSoft"})
    print(f"    DataSoft new address: {data_soft.get('Location', {}).get('Address', {}).get('Street')}")

    # Increment nested numeric field
    print("\n>>> Update: Increment CloudNet's Stats.Employees by 50")
    companies.update_one(
        {"Name": "CloudNet"},
        {"$inc": {"Stats.Employees": 50}}
    )
    cloud_net = companies.find_one({"Name": "CloudNet"})
    print(f"    CloudNet employees: {cloud_net.get('Stats', {}).get('Employees')}")

    # Aggregation with nested fields
    print("\n>>> Aggregation: Group by Location.Country, sum employees")
    country_stats = companies.aggregate([
        {"$group": {
            "_id": "$Location.Country",
            "totalEmployees": {"$sum": "$Stats.Employees"},
            "avgRating": {"$avg": "$Stats.Rating"},
            "companyCount": {"$sum": 1}
        }},
        {"$sort": {"totalEmployees": -1}}
    ])
    for s in country_stats:
        avg = s.get('avgRating')
        avg_str = f"{avg:.2f}" if avg else "N/A"
        print(f"    {s.get('_id')}: {s.get('companyCount')} companies, {s.get('totalEmployees')} employees, avg rating: {avg_str}")

    # Sort by nested field
    print("\n>>> Sort by Stats.Revenue descending")
    by_revenue = companies.find({}, sort=[("Stats.Revenue", -1)])
    for c in by_revenue:
        stats = c.get('Stats', {})
        print(f"    - {c.get('Name')}: ${stats.get('Revenue'):,}")

    # Create index on nested field
    print("\n>>> Create index on nested field 'Stats.Rating'")
    rating_idx = companies.create_index("Stats.Rating")
    print(f"    Created: {rating_idx}")

    print("\n>>> Create index on deeply nested 'Location.Address.Zip'")
    zip_idx = companies.create_index("Location.Address.Zip")
    print(f"    Created: {zip_idx}")

    # List indexes on companies
    print("\n>>> List indexes on companies collection")
    company_indexes = companies.list_indexes()
    for idx in company_indexes:
        print(f"    - {idx}")

    # ============================================================
    # 8. ADVANCED ARRAY OPERATORS
    # ============================================================
    print_section("8. ADVANCED ARRAY OPERATORS")

    products = db.collection("products")

    print(">>> Insert products with tags array")
    products.insert_many([
        {"Name": "Laptop Pro", "Price": 1299, "Tags": ["electronics", "computer", "portable"], "Ratings": [5, 4, 5, 5, 4]},
        {"Name": "Wireless Mouse", "Price": 49, "Tags": ["electronics", "accessory", "wireless"], "Ratings": [4, 4, 3, 5]},
        {"Name": "USB Hub", "Price": 29, "Tags": ["electronics", "accessory", "usb"], "Ratings": [3, 4, 4]},
        {"Name": "Desk Lamp", "Price": 79, "Tags": ["furniture", "lighting", "office"], "Ratings": [5, 5, 5]},
        {"Name": "Gaming Chair", "Price": 399, "Tags": ["furniture", "gaming", "ergonomic"], "Ratings": [4, 5, 4, 5]},
    ])
    print("    Inserted 5 products")

    # $all - Match documents where array contains ALL specified values
    print("\n>>> $all: Products with tags containing BOTH 'electronics' AND 'accessory'")
    all_match = products.find({"Tags": {"$all": ["electronics", "accessory"]}})
    for p in all_match:
        print(f"    - {p.get('Name')}: {p.get('Tags')}")

    # $size - Match documents where array has specific size
    print("\n>>> $size: Products with exactly 3 tags")
    size3 = products.find({"Tags": {"$size": 3}})
    for p in size3:
        print(f"    - {p.get('Name')}: {len(p.get('Tags', []))} tags")

    # $elemMatch - Match documents where array element matches multiple conditions
    print("\n>>> $elemMatch: Products with a rating that equals 5")
    has5_rating = products.find({"Ratings": {"$elemMatch": {"$eq": 5}}})
    for p in has5_rating:
        print(f"    - {p.get('Name')}: ratings {p.get('Ratings')}")

    # $nin - Not in
    print("\n>>> $nin: Products without 'furniture' or 'gaming' tags")
    not_furniture = products.find({"Tags": {"$nin": ["furniture", "gaming"]}})
    for p in not_furniture:
        print(f"    - {p.get('Name')}")

    # $pop - Remove last element from array
    print("\n>>> $pop: Remove last rating from Laptop Pro")
    products.update_one(
        {"Name": "Laptop Pro"},
        {"$pop": {"Ratings": 1}}
    )
    laptop = products.find_one({"Name": "Laptop Pro"})
    print(f"    Laptop Pro ratings: {laptop.get('Ratings')}")

    # ============================================================
    # 9. ADDITIONAL QUERY PATTERNS
    # ============================================================
    print_section("9. ADDITIONAL QUERY PATTERNS")

    # Combined nested field queries
    print(">>> Complex nested query: USA companies with >200 employees and rating >4.0")
    complex_query = companies.find({
        "$and": [
            {"Location.Country": "USA"},
            {"Stats.Employees": {"$gt": 200}},
            {"Stats.Rating": {"$gt": 4.0}}
        ]
    })
    for c in complex_query:
        stats = c.get('Stats', {})
        print(f"    - {c.get('Name')}: {stats.get('Employees')} employees, rating {stats.get('Rating')}")

    # Range query on nested field
    print("\n>>> Range query: Revenue between 20M and 40M")
    revenue_range = companies.find({
        "$and": [
            {"Stats.Revenue": {"$gte": 20000000}},
            {"Stats.Revenue": {"$lte": 40000000}}
        ]
    })
    for c in revenue_range:
        stats = c.get('Stats', {})
        print(f"    - {c.get('Name')}: ${stats.get('Revenue'):,}")

    # Not operator with nested field
    print("\n>>> $not: Companies NOT in Germany")
    not_germany = companies.find({
        "Location.Country": {"$not": {"$eq": "Germany"}}
    })
    for c in not_germany:
        loc = c.get('Location', {})
        print(f"    - {c.get('Name')} ({loc.get('Country')})")

    # Projection with nested fields
    print("\n>>> Projection: Only Name and nested Stats.Rating")
    projected_companies = companies.find(
        {},
        projection={"Name": 1, "Stats.Rating": 1, "_id": 0},
        limit=3
    )
    print(f"    Returned {len(projected_companies)} documents with projected fields")

    # ============================================================
    # 10. PERSISTENCE DEMO
    # ============================================================
    print_section("10. PERSISTENCE DEMO")

    persist_path = "/tmp/persist_demo.mlite"
    if os.path.exists(persist_path):
        os.remove(persist_path)
    if os.path.exists(persist_path + ".wal"):
        os.remove(persist_path + ".wal")

    print(">>> Session 1: Create database and insert data")
    persist_db = ironbase.IronBase(persist_path)
    persist_users = persist_db.collection("demo_users")

    # Create index on nested field
    persist_users.create_index("Profile.Score")
    print("    Created index on Profile.Score")

    # Insert users with nested profiles
    persist_users.insert_many([
        {"Name": "Alice", "Age": 30, "City": "NYC", "Profile": {"Score": 95, "Level": "senior"}},
        {"Name": "Bob", "Age": 25, "City": "LA", "Profile": {"Score": 82, "Level": "mid"}},
        {"Name": "Carol", "Age": 35, "City": "NYC", "Profile": {"Score": 91, "Level": "senior"}},
    ])
    print("    Inserted 3 users with nested profiles")

    # Verify data
    persist_count1 = persist_users.count_documents({})
    print(f"    Total users: {persist_count1}")

    # Flush to ensure data is written
    persist_db.checkpoint()
    print("    Data flushed to disk")

    # Close and reopen
    del persist_db

    print("\n>>> Session 2: Reopen database and verify data persisted")
    persist_db = ironbase.IronBase(persist_path)
    persist_users = persist_db.collection("demo_users")

    # Check document count
    persist_count2 = persist_users.count_documents({})
    print(f"    Documents found after reopen: {persist_count2}")

    # Verify indexes persisted
    persist_indexes = persist_users.list_indexes()
    print(f"    Indexes: {persist_indexes}")

    # Query using nested field index
    print("\n    Query: Profile.Score >= 90 (using persisted index)")
    persist_high_scorers = persist_users.find({"Profile.Score": {"$gte": 90}})
    for u in persist_high_scorers:
        print(f"      - {u.get('Name')}: score {u.get('Profile', {}).get('Score')}")

    # Verify explain shows index usage
    persist_explain = persist_users.explain({"Profile.Score": {"$gte": 90}})
    print(f"\n    Explain: {persist_explain}")

    # Cleanup persistence demo
    del persist_db
    os.remove(persist_path)
    if os.path.exists(persist_path + ".wal"):
        os.remove(persist_path + ".wal")
    print("\n    Persistence demo cleanup complete")

    # ============================================================
    # 11. TRANSACTIONS
    # ============================================================
    print_section("11. TRANSACTIONS")

    accounts = db.collection("accounts")

    # Setup accounts
    accounts.insert_many([
        {"AccountId": "A001", "Owner": "Alice", "Balance": 1000},
        {"AccountId": "A002", "Owner": "Bob", "Balance": 500}
    ])

    print(">>> Initial balances:")
    print_accounts(accounts)

    # Begin transaction
    print("\n>>> Begin transaction: Transfer 200 from Alice to Bob")
    tx_id = db.begin_transaction()
    print(f"    Transaction ID: {tx_id}")

    try:
        # Deduct from Alice
        accounts.update_one(
            {"AccountId": "A001"},
            {"$inc": {"Balance": -200}}
        )

        # Add to Bob
        accounts.update_one(
            {"AccountId": "A002"},
            {"$inc": {"Balance": 200}}
        )

        # Commit
        db.commit_transaction(tx_id)
        print("    Transaction committed!")
    except Exception as ex:
        db.rollback_transaction(tx_id)
        print(f"    Transaction rolled back: {ex}")

    print("\n>>> Final balances:")
    print_accounts(accounts)

    # ============================================================
    # 12. DATABASE OPERATIONS
    # ============================================================
    print_section("12. DATABASE OPERATIONS")

    # List collections
    print(">>> List collections")
    collections = db.list_collections()
    for col in collections:
        print(f"    - {col}")

    # Get stats
    print("\n>>> Database statistics")
    stats = db.stats()
    print(f"    {stats}")

    # Distinct values
    print("\n>>> Distinct cities")
    cities = users.distinct("City")
    print(f"    Cities: {cities}")

    # ============================================================
    # 13. DELETE OPERATIONS
    # ============================================================
    print_section("13. DELETE OPERATIONS")

    print(f">>> Users before delete: {users.count_documents({})}")

    # Delete one
    print("\n>>> DeleteOne: Remove David")
    delete_result = users.delete_one({"Name": "David"})
    print(f"    Deleted: {delete_result}")

    # Delete many
    print("\n>>> DeleteMany: Remove users with Age < 30")
    delete_many_result = users.delete_many({"Age": {"$lt": 30}})
    print(f"    Deleted: {delete_many_result}")

    print(f"\n>>> Users after delete: {users.count_documents({})}")

    # ============================================================
    # 14. CLEANUP & COMPACTION
    # ============================================================
    print_section("14. CLEANUP & COMPACTION")

    print(">>> Compact database (remove tombstones)")
    compact_result = db.compact()
    print(f"    Compaction result: {compact_result}")

    print("\n>>> Flush to disk")
    db.checkpoint()
    print("    Data flushed successfully!")

    # Drop collection
    print("\n>>> Drop 'accounts' collection")
    db.drop_collection("accounts")
    print(f"    Collections remaining: {db.list_collections()}")

    # ============================================================
    # DONE
    # ============================================================
    print()
    print("╔══════════════════════════════════════════════════════════╗")
    print("║                    Demo Complete!                        ║")
    print("╚══════════════════════════════════════════════════════════╝")

    # Cleanup
    del db
    os.remove(db_path)
    if os.path.exists(db_path + ".wal"):
        os.remove(db_path + ".wal")


if __name__ == "__main__":
    main()
