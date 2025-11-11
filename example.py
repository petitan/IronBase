#!/usr/bin/env python3
"""
IronBase - Példa használat
MongoDB-szerű API SQLite-szerű beágyazott adatbázishoz
"""

from ironbase import IronBase

def main():
    # 1. Adatbázis megnyitása (vagy létrehozása)
    print("=" * 60)
    print("IronBase Demo - MongoDB-like Embedded Database")
    print("=" * 60)
    
    db = IronBase("myapp.mlite")
    print(f"\n✓ Adatbázis megnyitva: {db}")
    
    # 2. Collection lekérése
    users = db.collection("users")
    print(f"✓ Collection létrehozva: {users}")
    
    # 3. Dokumentumok beszúrása
    print("\n--- INSERT OPERATIONS ---")
    
    # Egy dokumentum beszúrása
    result1 = users.insert_one({
        "name": "Kovács János",
        "email": "janos@example.com",
        "age": 30,
        "city": "Budapest",
        "hobbies": ["futás", "olvasás", "programozás"]
    })
    print(f"Beszúrva: {result1}")
    
    # Több dokumentum beszúrása
    result2 = users.insert_many([
        {
            "name": "Nagy Anna",
            "email": "anna@example.com",
            "age": 25,
            "city": "Szeged",
            "hobbies": ["tánc", "utazás"]
        },
        {
            "name": "Szabó Péter",
            "email": "peter@example.com",
            "age": 35,
            "city": "Debrecen",
            "hobbies": ["zene", "sport"]
        },
        {
            "name": "Kiss Éva",
            "email": "eva@example.com",
            "age": 28,
            "city": "Budapest",
            "hobbies": ["főzés", "kertészkedés"]
        }
    ])
    print(f"Több dokumentum beszúrva: {result2}")
    
    # 4. Dokumentumok számlálása
    print("\n--- COUNT OPERATIONS ---")
    count = users.count_documents()
    print(f"Összes dokumentum: {count}")
    
    # 5. Collection-ök listázása
    print("\n--- LIST COLLECTIONS ---")
    collections = db.list_collections()
    print(f"Collection-ök: {collections}")
    
    # 6. Statisztikák
    print("\n--- DATABASE STATS ---")
    stats = db.stats()
    print(stats)
    
    # 7. További példa collection
    print("\n--- PRODUCTS COLLECTION ---")
    products = db.collection("products")
    
    products.insert_many([
        {
            "name": "Laptop",
            "category": "Electronics",
            "price": 299999,
            "stock": 15,
            "tags": ["computer", "portable", "work"]
        },
        {
            "name": "Telefon",
            "category": "Electronics",
            "price": 199999,
            "stock": 30,
            "tags": ["mobile", "communication"]
        },
        {
            "name": "Könyv - Python",
            "category": "Books",
            "price": 8999,
            "stock": 50,
            "tags": ["education", "programming"]
        }
    ])
    
    print(f"✓ Termékek beszúrva: {products.count_documents()} db")
    
    # 8. Adatbázis bezárása
    print("\n--- CLOSING DATABASE ---")
    db.close()
    print("✓ Adatbázis bezárva")
    
    print("\n" + "=" * 60)
    print("Demo befejezve! Adatbázis fájl: myapp.mlite")
    print("=" * 60)


def demo_queries():
    """
    Jövőbeli query műveletek demo
    (Ezek még nincsenek teljesen implementálva)
    """
    db = IronBase("myapp.mlite")
    users = db.collection("users")
    
    # TODO: Ezek a műveletek még fejlesztés alatt állnak
    
    # Keresés
    # user = users.find_one({"name": "Kovács János"})
    # all_users = users.find({})
    # budapest_users = users.find({"city": "Budapest"})
    # young_users = users.find({"age": {"$lt": 30}})
    
    # Frissítés
    # users.update_one(
    #     {"name": "Kovács János"},
    #     {"$set": {"age": 31}}
    # )
    
    # Törlés
    # users.delete_one({"name": "Szabó Péter"})
    
    # Indexelés
    # users.create_index({"email": 1}, unique=True)
    
    db.close()


if __name__ == "__main__":
    main()
    
    print("\n\nHasználati példák:")
    print("-" * 60)
    print("""
    # Alapvető használat
    from ironbase import IronBase
    
    # Adatbázis megnyitása
    db = IronBase("mydata.mlite")
    
    # Collection
    users = db.collection("users")
    
    # CRUD műveletek
    users.insert_one({"name": "Test", "age": 25})
    users.insert_many([{...}, {...}])
    
    # (Hamarosan: find, update, delete)
    # users.find({"age": {"$gt": 20}})
    # users.update_one({"name": "Test"}, {"$set": {"age": 26}})
    # users.delete_one({"name": "Test"})
    
    # Bezárás
    db.close()
    """)
