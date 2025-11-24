// ironbase-core/src/query_cache.rs
// Query result caching with LRU eviction policy

use crate::document::DocumentId;
use lru::LruCache;
use parking_lot::RwLock;
use serde_json::Value;
use std::collections::hash_map::DefaultHasher;
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

/// Query cache with LRU eviction
///
/// Caches query results (DocumentIds) to avoid repeated scans.
/// Thread-safe with RwLock for concurrent access.
pub struct QueryCache {
    cache: RwLock<LruCache<QueryHash, Vec<DocumentId>>>,
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
    /// Automatically evicts LRU entry if cache is full
    pub fn insert(&self, query_hash: QueryHash, doc_ids: Vec<DocumentId>) {
        let mut cache = self.cache.write();
        cache.put(query_hash, doc_ids);
    }

    /// Invalidate all cached queries for a collection
    ///
    /// Called on insert/update/delete operations to maintain consistency
    pub fn invalidate_collection(&self, _collection: &str) {
        // Simple approach: clear entire cache
        // OPTIMIZATION: More granular invalidation (track which queries belong to which collection)
        //
        // Current: Nuclear approach - clear entire cache on ANY write
        // Impact: Cache becomes ineffective in write-heavy workloads
        //
        // Granular invalidation design:
        // 1. Add collection tracking to cache entries:
        //    struct CachedEntry { result: Vec<Value>, collections: HashSet<String> }
        // 2. Parse query to extract collection references (easy for simple queries)
        // 3. Only invalidate entries where collections.contains(collection)
        //
        // Complexity: Low (1-2 hours work)
        // Benefit: Significant for multi-collection databases
        // Priority: Low (correctness unaffected, only performance)
        let mut cache = self.cache.write();
        cache.clear();
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
        cache.insert(hash, doc_ids.clone());

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

        cache.insert(hash1, vec![DocumentId::Int(1)]);
        cache.insert(hash2, vec![DocumentId::Int(2)]);
        cache.insert(hash3, vec![DocumentId::Int(3)]); // Should evict hash1 (LRU)

        assert_eq!(cache.get(&hash1), None, "Oldest entry should be evicted");
        assert_eq!(cache.get(&hash2), Some(vec![DocumentId::Int(2)]));
        assert_eq!(cache.get(&hash3), Some(vec![DocumentId::Int(3)]));
    }

    #[test]
    fn test_cache_invalidation() {
        let cache = QueryCache::new(100);
        let query = json!({"age": 25});
        let hash = QueryHash::new("users", &query);

        cache.insert(hash, vec![DocumentId::Int(1)]);
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
        cache.insert(hash, vec![DocumentId::Int(1)]);

        let stats = cache.stats();
        assert_eq!(stats.size, 1);
    }
}
