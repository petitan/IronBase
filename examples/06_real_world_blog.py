#!/usr/bin/env python3
"""
Example 6: Real-World Application - Simple Blog System

Demonstrates MongoLite in a realistic scenario with:
- Multiple collections (users, posts, comments)
- Relationships between documents
- Common blog operations (CRUD, search, stats)

Run: python examples/06_real_world_blog.py
"""
import ironbase
import os
from datetime import datetime, timedelta

DB_FILE = "examples/data/blog_example.db"

def setup():
    """Create examples/data directory if it doesn't exist"""
    os.makedirs("examples/data", exist_ok=True)
    if os.path.exists(DB_FILE):
        os.remove(DB_FILE)

def get_timestamp(days_ago=0):
    """Get ISO timestamp for N days ago"""
    return (datetime.now() - timedelta(days=days_ago)).isoformat()

def main():
    setup()

    print("="*60)
    print("MongoLite Example 6: Real-World Blog System")
    print("="*60 + "\n")

    db = ironbase.MongoLite(DB_FILE)

    # ==================== Setup Collections ====================
    users = db.collection("users")
    posts = db.collection("posts")
    comments = db.collection("comments")

    # ==================== 1. User Management ====================
    print("1. User Management - Creating user accounts\n")

    users.insert_many([
        {
            "username": "alice_dev",
            "email": "alice@example.com",
            "display_name": "Alice Johnson",
            "bio": "Software developer and tech blogger",
            "joined": get_timestamp(90),
            "role": "admin",
            "active": True
        },
        {
            "username": "bob_writer",
            "email": "bob@example.com",
            "display_name": "Bob Smith",
            "bio": "Freelance writer",
            "joined": get_timestamp(60),
            "role": "author",
            "active": True
        },
        {
            "username": "carol_tech",
            "email": "carol@example.com",
            "display_name": "Carol White",
            "bio": "Tech enthusiast",
            "joined": get_timestamp(30),
            "role": "author",
            "active": True
        },
        {
            "username": "dave_reader",
            "email": "dave@example.com",
            "display_name": "Dave Brown",
            "bio": "Just loves reading blogs",
            "joined": get_timestamp(10),
            "role": "reader",
            "active": True
        }
    ])

    total_users = users.count_documents({})
    print(f"✓ Created {total_users} user accounts\n")

    # ==================== 2. Publishing Blog Posts ====================
    print("2. Publishing Blog Posts\n")

    # Alice creates posts
    alice = users.find_one({"username": "alice_dev"})
    alice_id = alice["_id"]

    posts.insert_many([
        {
            "title": "Getting Started with MongoLite",
            "author_id": alice_id,
            "author_name": "alice_dev",
            "content": "MongoLite is an embedded NoSQL database with MongoDB-compatible API...",
            "tags": ["database", "mongodb", "python"],
            "published": get_timestamp(7),
            "status": "published",
            "views": 150,
            "likes": 23
        },
        {
            "title": "Building REST APIs with Python",
            "author_id": alice_id,
            "author_name": "alice_dev",
            "content": "Learn how to build scalable REST APIs using Python and FastAPI...",
            "tags": ["python", "api", "fastapi"],
            "published": get_timestamp(14),
            "status": "published",
            "views": 320,
            "likes": 45
        }
    ])

    # Bob creates posts
    bob = users.find_one({"username": "bob_writer"})
    bob_id = bob["_id"]

    posts.insert_many([
        {
            "title": "The Future of Web Development",
            "author_id": bob_id,
            "author_name": "bob_writer",
            "content": "Web development is evolving rapidly with new frameworks...",
            "tags": ["web", "javascript", "trends"],
            "published": get_timestamp(5),
            "status": "published",
            "views": 89,
            "likes": 12
        },
        {
            "title": "Draft: Understanding Databases",
            "author_id": bob_id,
            "author_name": "bob_writer",
            "content": "Work in progress...",
            "tags": ["database"],
            "published": get_timestamp(1),
            "status": "draft",
            "views": 0,
            "likes": 0
        }
    ])

    # Carol creates a post
    carol = users.find_one({"username": "carol_tech"})
    carol_id = carol["_id"]

    posts.insert_one({
        "title": "10 Python Tips for Beginners",
        "author_id": carol_id,
        "author_name": "carol_tech",
        "content": "Python is a great language for beginners. Here are 10 tips...",
        "tags": ["python", "beginner", "tutorial"],
        "published": get_timestamp(3),
        "status": "published",
        "views": 245,
        "likes": 38
    })

    published_count = posts.count_documents({"status": "published"})
    print(f"✓ Published {published_count} blog posts\n")

    # ==================== 3. Adding Comments ====================
    print("3. Adding Comments\n")

    # Get some posts
    mongo_post = posts.find_one({"title": "Getting Started with MongoLite"})
    python_tips_post = posts.find_one({"title": "10 Python Tips for Beginners"})

    dave = users.find_one({"username": "dave_reader"})
    dave_id = dave["_id"]

    comments.insert_many([
        {
            "post_id": mongo_post["_id"],
            "author_id": dave_id,
            "author_name": "dave_reader",
            "content": "Great tutorial! Very helpful for beginners.",
            "posted": get_timestamp(6),
            "likes": 3
        },
        {
            "post_id": mongo_post["_id"],
            "author_id": bob_id,
            "author_name": "bob_writer",
            "content": "Nice work Alice! Looking forward to more posts.",
            "posted": get_timestamp(5),
            "likes": 1
        },
        {
            "post_id": python_tips_post["_id"],
            "author_id": alice_id,
            "author_name": "alice_dev",
            "content": "Excellent tips! Tip #5 is my favorite.",
            "posted": get_timestamp(2),
            "likes": 2
        },
        {
            "post_id": python_tips_post["_id"],
            "author_id": dave_id,
            "author_name": "dave_reader",
            "content": "This helped me so much, thank you!",
            "posted": get_timestamp(1),
            "likes": 5
        }
    ])

    comment_count = comments.count_documents({})
    print(f"✓ Added {comment_count} comments\n")

    # ==================== 4. Query Operations ====================
    print("4. Common Blog Queries\n")

    # Find all published posts
    print("📄 All Published Posts:")
    published_posts = posts.find({"status": "published"})
    for post in published_posts:
        print(f"  • \"{post['title']}\" by {post['author_name']}")
        print(f"    Views: {post['views']}, Likes: {post['likes']}")

    print()

    # Find posts by tag
    # NOTE: Array element matching not yet implemented
    # This will be enabled when we add support for: {"tags": "python"}
    print("🏷️  Posts tagged with 'python':")
    print("  (Array element matching coming soon...)")
    print()

    # Find popular posts (views > 200)
    print("🔥 Popular Posts (views > 200):")
    popular = posts.find({"views": {"$gt": 200}, "status": "published"})
    for post in popular:
        print(f"  • {post['title']} - {post['views']} views")

    print()

    # ==================== 5. User Activity Stats ====================
    print("5. User Activity Statistics\n")

    all_users = users.find({"role": {"$in": ["author", "admin"]}})
    print("Author Statistics:")
    print("━" * 50)

    for user in all_users:
        user_id = user["_id"]
        post_count = posts.count_documents({"author_id": user_id})
        published_count = posts.count_documents({
            "author_id": user_id,
            "status": "published"
        })
        comment_count = comments.count_documents({"author_id": user_id})

        print(f"\n{user['display_name']} (@{user['username']})")
        print(f"  Posts: {post_count} ({published_count} published)")
        print(f"  Comments: {comment_count}")

    print("\n")

    # ==================== 6. Blog Analytics ====================
    print("6. Blog Analytics Dashboard\n")
    print("━" * 50)

    # Overall stats
    total_posts = posts.count_documents({"status": "published"})
    total_comments = comments.count_documents({})
    total_authors = users.count_documents({"role": {"$in": ["author", "admin"]}})

    print(f"\n📊 Overall Statistics:")
    print(f"  Total Published Posts: {total_posts}")
    print(f"  Total Comments: {total_comments}")
    print(f"  Total Authors: {total_authors}")

    # Calculate total views and likes
    all_posts = posts.find({"status": "published"})
    total_views = sum(p["views"] for p in all_posts)
    all_posts = posts.find({"status": "published"})  # Re-query
    total_likes = sum(p["likes"] for p in all_posts)

    print(f"  Total Views: {total_views}")
    print(f"  Total Likes: {total_likes}")

    # Tag analytics
    print(f"\n🏷️  Tag Usage:")
    all_posts = posts.find({"status": "published"})
    tag_counts = {}
    for post in all_posts:
        for tag in post.get("tags", []):
            tag_counts[tag] = tag_counts.get(tag, 0) + 1

    for tag, count in sorted(tag_counts.items(), key=lambda x: x[1], reverse=True):
        print(f"  • {tag}: {count} post(s)")

    print()

    # ==================== 7. Update Operations ====================
    print("7. Update Operations - Engagement tracking\n")

    # Someone views a post
    print("📖 User views 'Getting Started with MongoLite'")
    posts.update_one(
        {"title": "Getting Started with MongoLite"},
        {"$inc": {"views": 1}}
    )

    # Someone likes a post
    print("👍 User likes 'Getting Started with MongoLite'")
    posts.update_one(
        {"title": "Getting Started with MongoLite"},
        {"$inc": {"likes": 1}}
    )

    # Update post - add new tag
    print("🏷️  Adding 'tutorial' tag to post")
    posts.update_one(
        {"title": "Getting Started with MongoLite"},
        {"$set": {"tags": ["database", "mongodb", "python", "tutorial"]}}
    )

    updated_post = posts.find_one({"title": "Getting Started with MongoLite"})
    print(f"\nUpdated post stats:")
    print(f"  Views: {updated_post['views']}")
    print(f"  Likes: {updated_post['likes']}")
    print(f"  Tags: {', '.join(updated_post['tags'])}")
    print()

    # ==================== 8. Content Moderation ====================
    print("8. Content Moderation - Publishing draft post\n")

    # Bob publishes his draft
    draft_post = posts.find_one({"status": "draft"})
    print(f"Publishing: \"{draft_post['title']}\"")

    posts.update_one(
        {"_id": draft_post["_id"]},
        {"$set": {"status": "published", "published": get_timestamp(0)}}
    )

    published_count = posts.count_documents({"status": "published"})
    print(f"✓ Now {published_count} published posts\n")

    # ==================== 9. Search Functionality ====================
    print("9. Search Functionality\n")

    # Search posts by keyword in title
    search_term = "Python"
    print(f"🔍 Searching for posts with '{search_term}' in title...")

    # Note: This is a simple case-sensitive search
    # Real implementation would use full-text search
    matching_posts = posts.find({"status": "published"})
    results = [p for p in matching_posts if search_term.lower() in p["title"].lower()]

    print(f"Found {len(results)} result(s):")
    for post in results:
        print(f"  • {post['title']}")
        print(f"    by {post['author_name']} - {post['views']} views")

    print()

    # ==================== 10. Cleanup - Delete user ====================
    print("10. User Management - Deactivate inactive user\n")

    # Deactivate user instead of deleting (better practice)
    print("Deactivating dave_reader account...")
    users.update_one(
        {"username": "dave_reader"},
        {"$set": {"active": False}}
    )

    active_users = users.count_documents({"active": True})
    print(f"✓ Active users: {active_users}\n")

    # ==================== Final Summary ====================
    print("="*60)
    print("📈 Final Blog Statistics")
    print("="*60)

    print(f"\nUsers: {users.count_documents({})}")
    print(f"  Active: {users.count_documents({'active': True})}")
    print(f"  Admins: {users.count_documents({'role': 'admin'})}")
    print(f"  Authors: {users.count_documents({'role': 'author'})}")
    print(f"  Readers: {users.count_documents({'role': 'reader'})}")

    print(f"\nPosts: {posts.count_documents({})}")
    print(f"  Published: {posts.count_documents({'status': 'published'})}")
    print(f"  Drafts: {posts.count_documents({'status': 'draft'})}")

    print(f"\nComments: {comments.count_documents({})}")

    print(f"\nMost Active Author:")
    authors = users.find({"role": {"$in": ["author", "admin"]}})
    max_posts = 0
    top_author = None
    for author in authors:
        count = posts.count_documents({"author_id": author["_id"]})
        if count > max_posts:
            max_posts = count
            top_author = author

    if top_author:
        print(f"  {top_author['display_name']} with {max_posts} post(s)")

    print()

    db.close()

    print("="*60)
    print("✅ Blog system demo completed successfully!")
    print("="*60)

if __name__ == "__main__":
    main()
