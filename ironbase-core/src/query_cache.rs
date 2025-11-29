// ironbase-core/src/query_cache.rs
// Query result caching with LRU eviction policy

use crate::document::DocumentId;
use lru::LruCache;
use parking_lot::RwLock;
use serde_json::Value;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;

/// Hash of a query (collection + query JSON)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QueryHash(u64);

impl QueryHash {
    /// Create a hash from collection name and query JSON
    pub fn new(collection: &str, query: &Value) -> Self {
        let mut hasher = DefaultHasher::new();
        collection.hash(&mut hasher);

        // Hash the query JSON in a deterministic way
        // serde_json guarantees stable serialization
        let query_str = serde_json::to_string(query).unwrap_or_default();
        query_str.hash(&mut hasher);

        QueryHash(hasher.finish())
    }
}

/// Query cache with LRU eviction and collection-level invalidation
///
/// Caches query results (DocumentIds) to avoid repeated scans.
/// Thread-safe with RwLock for concurrent access.
///
/// Uses a reverse index (collection → query hashes) to enable
/// selective invalidation: only queries for the modified collection
/// are invalidated, not the entire cache.
pub struct QueryCache {
    cache: RwLock<LruCache<QueryHash, Vec<DocumentId>>>,
    /// Reverse index: collection name → set of query hashes for that collection
    collection_index: RwLock<HashMap<String, HashSet<QueryHash>>>,
    capacity: usize,
}

impl QueryCache {
    /// Create a new query cache with specified capacity
    ///
    /// # Arguments
    /// * `capacity` - Maximum number of cached queries (recommended: 1000)
    pub fn new(capacity: usize) -> Self {
        let non_zero_capacity =
            NonZeroUsize::new(capacity).unwrap_or(NonZeroUsize::new(1000).unwrap());
        QueryCache {
            cache: RwLock::new(LruCache::new(non_zero_capacity)),
            collection_index: RwLock::new(HashMap::new()),
            capacity,
        }
    }

    /// Get cached result for a query (returns None if not cached)
    ///
    /// Uses peek() to avoid updating LRU order on read
    pub fn get(&self, query_hash: &QueryHash) -> Option<Vec<DocumentId>> {
        let cache = self.cache.read();
        cache.peek(query_hash).cloned()
    }

    /// Insert query result into cache
    ///
    /// # Arguments
    /// * `collection` - The collection name this query belongs to
    /// * `query_hash` - The hash of the query
    /// * `doc_ids` - The document IDs returned by the query
    ///
    /// Automatically evicts LRU entry if cache is full and maintains
    /// the reverse index for collection-level invalidation.
    pub fn insert(&self, collection: &str, query_hash: QueryHash, doc_ids: Vec<DocumentId>) {
        let mut cache = self.cache.write();

        // Handle LRU eviction: if at capacity and inserting new key, clean up reverse index
        if cache.len() >= self.capacity && !cache.contains(&query_hash) {
            if let Some((evicted_hash, _)) = cache.peek_lru() {
                let evicted_hash = *evicted_hash;
                // Remove from all collection indexes (we don't track which collection it belonged to)
                // This is O(collections * entries_per_collection) but happens rarely
                drop(cache); // Release cache lock before acquiring collection_index lock
                let mut coll_index = self.collection_index.write();
                for hashes in coll_index.values_mut() {
                    hashes.remove(&evicted_hash);
                }
                drop(coll_index);
                cache = self.cache.write(); // Re-acquire cache lock
            }
        }

        cache.put(query_hash, doc_ids);
        drop(cache);

        // Update reverse index
        let mut coll_index = self.collection_index.write();
        coll_index
            .entry(collection.to_string())
            .or_default()
            .insert(query_hash);
    }

    /// Invalidate all cached queries for a specific collection
    ///
    /// Called on insert/update/delete operations to maintain consistency.
    /// Only invalidates queries belonging to the specified collection,
    /// leaving other collections' cached queries intact.
    pub fn invalidate_collection(&self, collection: &str) {
        // Get query hashes for this collection
        let mut coll_index = self.collection_index.write();
        let hashes_to_remove = coll_index.remove(collection);
        drop(coll_index);

        // Remove from LRU cache
        if let Some(hashes) = hashes_to_remove {
            let mut cache = self.cache.write();
            for hash in hashes {
                cache.pop(&hash);
            }
        }
    }

    /// Get cache statistics
    pub fn stats(&self) -> CacheStats {
        let cache = self.cache.read();
        CacheStats {
            capacity: self.capacity,
            size: cache.len(),
        }
    }
}

impl Default for QueryCache {
    fn default() -> Self {
        Self::new(1000)
    }
}

/// Cache statistics
#[derive(Debug, Clone)]
pub struct CacheStats {
    pub capacity: usize,
    pub size: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_query_hash_deterministic() {
        let query = json!({"age": {"$gte": 25}});
        let hash1 = QueryHash::new("users", &query);
        let hash2 = QueryHash::new("users", &query);

        assert_eq!(hash1, hash2, "Same query should produce same hash");
    }

    #[test]
    fn test_query_hash_different_collections() {
        let query = json!({"age": 25});
        let hash1 = QueryHash::new("users", &query);
        let hash2 = QueryHash::new("posts", &query);

        assert_ne!(
            hash1, hash2,
            "Different collections should produce different hashes"
        );
    }

    #[test]
    fn test_query_hash_different_queries() {
        let query1 = json!({"age": 25});
        let query2 = json!({"age": 30});
        let hash1 = QueryHash::new("users", &query1);
        let hash2 = QueryHash::new("users", &query2);

        assert_ne!(
            hash1, hash2,
            "Different queries should produce different hashes"
        );
    }

    #[test]
    fn test_cache_insert_and_get() {
        let cache = QueryCache::new(100);
        let query = json!({"age": 25});
        let hash = QueryHash::new("users", &query);

        let doc_ids = vec![DocumentId::Int(1), DocumentId::Int(2)];
        cache.insert("users", hash, doc_ids.clone());

        let result = cache.get(&hash);
        assert_eq!(result, Some(doc_ids));
    }

    #[test]
    fn test_cache_lru_eviction() {
        let cache = QueryCache::new(2); // Small capacity for testing

        let query1 = json!({"age": 25});
        let query2 = json!({"age": 30});
        let query3 = json!({"age": 35});

        let hash1 = QueryHash::new("users", &query1);
        let hash2 = QueryHash::new("users", &query2);
        let hash3 = QueryHash::new("users", &query3);

        cache.insert("users", hash1, vec![DocumentId::Int(1)]);
        cache.insert("users", hash2, vec![DocumentId::Int(2)]);
        cache.insert("users", hash3, vec![DocumentId::Int(3)]); // Should evict hash1 (LRU)

        assert_eq!(cache.get(&hash1), None, "Oldest entry should be evicted");
        assert_eq!(cache.get(&hash2), Some(vec![DocumentId::Int(2)]));
        assert_eq!(cache.get(&hash3), Some(vec![DocumentId::Int(3)]));
    }

    #[test]
    fn test_cache_invalidation() {
        let cache = QueryCache::new(100);
        let query = json!({"age": 25});
        let hash = QueryHash::new("users", &query);

        cache.insert("users", hash, vec![DocumentId::Int(1)]);
        assert!(cache.get(&hash).is_some());

        cache.invalidate_collection("users");
        assert!(
            cache.get(&hash).is_none(),
            "Cache should be cleared after invalidation"
        );
    }

    #[test]
    fn test_cache_stats() {
        let cache = QueryCache::new(100);
        let stats = cache.stats();

        assert_eq!(stats.capacity, 100);
        assert_eq!(stats.size, 0);

        let query = json!({"age": 25});
        let hash = QueryHash::new("users", &query);
        cache.insert("users", hash, vec![DocumentId::Int(1)]);

        let stats = cache.stats();
        assert_eq!(stats.size, 1);
    }

    #[test]
    fn test_selective_invalidation() {
        let cache = QueryCache::new(100);

        // Insert queries for two different collections
        let query1 = json!({"age": 25});
        let query2 = json!({"name": "Alice"});

        let hash_users = QueryHash::new("users", &query1);
        let hash_posts = QueryHash::new("posts", &query2);

        cache.insert("users", hash_users, vec![DocumentId::Int(1)]);
        cache.insert("posts", hash_posts, vec![DocumentId::Int(2)]);

        // Verify both are cached
        assert!(cache.get(&hash_users).is_some());
        assert!(cache.get(&hash_posts).is_some());

        // Invalidate only users collection
        cache.invalidate_collection("users");

        // users query should be gone, posts query should remain
        assert!(
            cache.get(&hash_users).is_none(),
            "Users cache should be invalidated"
        );
        assert!(
            cache.get(&hash_posts).is_some(),
            "Posts cache should remain"
        );
    }
}
