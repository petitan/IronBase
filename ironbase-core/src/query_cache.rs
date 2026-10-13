// ironbase-core/src/query_cache.rs
// Query result caching with LRU eviction policy

use crate::document::DocumentId;
use crate::limits::{QUERY_CACHE_MAX_BYTES, QUERY_CACHE_MAX_RESULT_IDS};
use lru::LruCache;
use parking_lot::RwLock;
use serde_json::Value;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};

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
    /// Write generation per collection, bumped by every invalidation. A reader
    /// takes it before computing a result and stores the result only if it is
    /// unchanged, so a result computed across a concurrent write is never cached.
    generations: RwLock<HashMap<String, u64>>,
    /// Derived per-index facts (e.g. "index covers every document") keyed by
    /// (collection, fact key), valid only while the collection's write
    /// generation is unchanged — so a fact costing an index walk is computed
    /// once per write, not once per query.
    facts: RwLock<HashMap<(String, String), (u64, bool)>>,
    capacity: usize,
    /// Estimated bytes of the cached id lists, changed only under the `cache`
    /// write lock. Bounded by `QUERY_CACHE_MAX_BYTES` (audit 2026-10-07 M9).
    bytes: AtomicUsize,
}

/// Estimated heap + inline bytes of a cached id list.
fn doc_ids_bytes(doc_ids: &[DocumentId]) -> usize {
    doc_ids
        .iter()
        .map(|id| {
            std::mem::size_of::<DocumentId>()
                + match id {
                    DocumentId::Int(_) => 0,
                    DocumentId::String(s) | DocumentId::ObjectId(s) => s.len(),
                }
        })
        .sum()
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
            generations: RwLock::new(HashMap::new()),
            facts: RwLock::new(HashMap::new()),
            capacity,
            bytes: AtomicUsize::new(0),
        }
    }

    /// A cached per-collection fact, or `compute()` cached under the write
    /// generation read before computing it (not cached if a write happened
    /// meanwhile).
    pub fn fact(&self, collection: &str, key: &str, compute: impl FnOnce() -> bool) -> bool {
        let generation = self.generation(collection);
        let cache_key = (collection.to_string(), key.to_string());
        if let Some(&(g, value)) = self.facts.read().get(&cache_key) {
            if g == generation {
                return value;
            }
        }
        let value = compute();
        let generations = self.generations.read();
        if generations.get(collection).copied().unwrap_or(0) == generation {
            self.facts.write().insert(cache_key, (generation, value));
        }
        value
    }

    /// Current write generation of a collection (see [`Self::insert_if_current`]).
    pub fn generation(&self, collection: &str) -> u64 {
        self.generations
            .read()
            .get(collection)
            .copied()
            .unwrap_or(0)
    }

    /// Insert a result computed after reading `generation`, unless the
    /// collection was written (invalidated) in the meantime.
    pub fn insert_if_current(
        &self,
        collection: &str,
        query_hash: QueryHash,
        doc_ids: Vec<DocumentId>,
        generation: u64,
    ) {
        // Hold the generation lock across the insert so an invalidation cannot
        // slip in between the check and the insert.
        let generations = self.generations.read();
        if generations.get(collection).copied().unwrap_or(0) != generation {
            return;
        }
        self.insert(collection, query_hash, doc_ids);
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
        if doc_ids.len() > QUERY_CACHE_MAX_RESULT_IDS {
            return;
        }
        let size = doc_ids_bytes(&doc_ids);
        if size > QUERY_CACHE_MAX_BYTES {
            return;
        }
        // BUG #3 fix: Acquire both locks upfront to avoid TOCTOU race condition
        // Lock ordering: always cache first, then collection_index (prevents deadlock)
        let mut cache = self.cache.write();
        let mut coll_index = self.collection_index.write();

        // A re-insert replaces the old entry: account for it first
        if let Some(old) = cache.pop(&query_hash) {
            self.bytes.fetch_sub(doc_ids_bytes(&old), Ordering::Relaxed);
        }

        // LRU eviction until both the entry and the byte budget fit; clean
        // up the reverse index for every evicted entry
        while cache.len() >= self.capacity
            || self.bytes.load(Ordering::Relaxed) + size > QUERY_CACHE_MAX_BYTES
        {
            let Some((evicted_hash, evicted)) = cache.pop_lru() else {
                break;
            };
            self.bytes
                .fetch_sub(doc_ids_bytes(&evicted), Ordering::Relaxed);
            // Remove from all collection indexes atomically
            for hashes in coll_index.values_mut() {
                hashes.remove(&evicted_hash);
            }
        }

        cache.put(query_hash, doc_ids);
        self.bytes.fetch_add(size, Ordering::Relaxed);

        // Update reverse index
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
        // Lock ordering: generations, then cache, then collection_index (the
        // same order as insert_if_current → insert)
        let mut generations = self.generations.write();
        *generations.entry(collection.to_string()).or_default() += 1;
        // BUG #3 fix: Use same lock ordering as insert() to prevent deadlock
        // Lock ordering: always cache first, then collection_index
        let mut cache = self.cache.write();
        let mut coll_index = self.collection_index.write();

        // Get and remove query hashes for this collection
        if let Some(hashes) = coll_index.remove(collection) {
            for hash in hashes {
                if let Some(old) = cache.pop(&hash) {
                    self.bytes.fetch_sub(doc_ids_bytes(&old), Ordering::Relaxed);
                }
            }
        }

        // If cache is fully empty, drop internal allocations.
        if cache.is_empty() && coll_index.is_empty() {
            let non_zero_capacity =
                NonZeroUsize::new(self.capacity).unwrap_or(NonZeroUsize::new(1000).unwrap());
            *cache = LruCache::new(non_zero_capacity);
            *coll_index = HashMap::new();
        }
    }

    /// Get cache statistics
    pub fn stats(&self) -> CacheStats {
        let cache = self.cache.read();
        CacheStats {
            capacity: self.capacity,
            size: cache.len(),
            bytes: self.bytes.load(Ordering::Relaxed),
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
    /// Estimated bytes of the cached results (at most `QUERY_CACHE_MAX_BYTES`)
    pub bytes: usize,
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

    #[test]
    fn test_cache_tracks_bytes() {
        let cache = QueryCache::new(100);
        let hash = QueryHash::new("users", &json!({"a": 1}));
        let ids = vec![DocumentId::String("x".repeat(100))];
        let size = doc_ids_bytes(&ids);
        cache.insert("users", hash, ids.clone());
        assert_eq!(cache.stats().bytes, size);
        // A re-insert of the same query replaces, not adds
        cache.insert("users", hash, ids);
        assert_eq!(cache.stats().bytes, size);
        cache.invalidate_collection("users");
        assert_eq!(cache.stats().bytes, 0);
    }

    #[test]
    fn test_cache_evicts_to_byte_budget() {
        let cache = QueryCache::new(100);
        // Each entry is just over a third of the budget: three never fit
        let len = QUERY_CACHE_MAX_BYTES / 3 / 1000;
        for q in 0..3 {
            let ids: Vec<DocumentId> = (0..1000)
                .map(|i| DocumentId::String(format!("{i:0len$}")))
                .collect();
            cache.insert("users", QueryHash::new("users", &json!({ "q": q })), ids);
        }
        let stats = cache.stats();
        assert!(stats.bytes <= QUERY_CACHE_MAX_BYTES);
        assert_eq!(stats.size, 2);
        // The least recently inserted one went
        assert!(cache
            .get(&QueryHash::new("users", &json!({"q": 0})))
            .is_none());
    }
}
