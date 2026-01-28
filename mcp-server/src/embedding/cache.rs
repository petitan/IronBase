//! Embedding cache implementation
//!
//! Provides LRU caching for embedding vectors to avoid redundant API calls
//! and improve performance for repeated queries.

use lru::LruCache;
use parking_lot::RwLock;
use sha2::{Digest, Sha256};
use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

/// Cached embedding entry
#[derive(Debug, Clone)]
pub struct CachedEmbedding {
    /// The embedding vector
    pub vector: Vec<f32>,
    /// Provider that generated this embedding
    pub provider: String,
    /// Model used
    pub model: String,
    /// When this entry was created
    pub created_at: Instant,
    /// Number of times this entry was hit
    pub hit_count: u64,
}

impl CachedEmbedding {
    /// Create a new cached embedding
    pub fn new(vector: Vec<f32>, provider: String, model: String) -> Self {
        Self {
            vector,
            provider,
            model,
            created_at: Instant::now(),
            hit_count: 0,
        }
    }

    /// Check if this entry has expired
    pub fn is_expired(&self, ttl: Duration) -> bool {
        self.created_at.elapsed() > ttl
    }

    /// Age of this entry in seconds
    pub fn age_secs(&self) -> u64 {
        self.created_at.elapsed().as_secs()
    }
}

/// Cache configuration
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Maximum number of entries in the cache
    pub max_entries: usize,
    /// Time-to-live for cache entries (None = no expiration)
    pub ttl: Option<Duration>,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_entries: 10_000,
            ttl: Some(Duration::from_secs(3600)), // 1 hour default
        }
    }
}

impl CacheConfig {
    /// Create config with custom max entries
    pub fn with_max_entries(mut self, max: usize) -> Self {
        self.max_entries = max;
        self
    }

    /// Create config with custom TTL
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = Some(ttl);
        self
    }

    /// Create config without TTL (entries never expire)
    pub fn without_ttl(mut self) -> Self {
        self.ttl = None;
        self
    }
}

/// Cache statistics
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct CacheStats {
    /// Number of cache hits
    pub hits: u64,
    /// Number of cache misses
    pub misses: u64,
    /// Current number of entries in cache
    pub entries: usize,
    /// Maximum capacity
    pub max_entries: usize,
    /// Total bytes used (approximate)
    pub bytes_used: usize,
    /// Hit rate percentage
    pub hit_rate: f64,
}

/// Thread-safe embedding cache
pub struct EmbeddingCache {
    cache: RwLock<LruCache<String, CachedEmbedding>>,
    config: CacheConfig,
    stats: RwLock<CacheStatsInternal>,
}

#[derive(Debug, Default)]
struct CacheStatsInternal {
    hits: u64,
    misses: u64,
}

impl EmbeddingCache {
    /// Create a new embedding cache with default configuration
    pub fn new() -> Self {
        Self::with_config(CacheConfig::default())
    }

    /// Create a new embedding cache with custom configuration
    pub fn with_config(config: CacheConfig) -> Self {
        let capacity = NonZeroUsize::new(config.max_entries.max(1))
            .expect("max(1) ensures non-zero capacity");
        Self {
            cache: RwLock::new(LruCache::new(capacity)),
            config,
            stats: RwLock::new(CacheStatsInternal::default()),
        }
    }

    /// Generate cache key from text, provider, and model
    ///
    /// Uses length-prefixed encoding to prevent collision attacks:
    /// - "ab" + "cd" != "a" + "bcd" because lengths differ
    pub fn cache_key(text: &str, provider: &str, model: &str) -> String {
        let mut hasher = Sha256::new();

        // Length-prefixed encoding: [len:8 bytes][data]
        // This prevents collision between ("ab", "cd") and ("a", "bcd")
        hasher.update((text.len() as u64).to_le_bytes());
        hasher.update(text.as_bytes());

        hasher.update((provider.len() as u64).to_le_bytes());
        hasher.update(provider.as_bytes());

        hasher.update((model.len() as u64).to_le_bytes());
        hasher.update(model.as_bytes());

        hex::encode(hasher.finalize())
    }

    /// Get an embedding from cache
    pub fn get(&self, text: &str, provider: &str, model: &str) -> Option<CachedEmbedding> {
        let key = Self::cache_key(text, provider, model);

        // Try to get from cache
        let mut cache = self.cache.write();
        let entry = cache.get_mut(&key)?;

        // Check TTL
        if let Some(ttl) = self.config.ttl {
            if entry.is_expired(ttl) {
                cache.pop(&key);
                let mut stats = self.stats.write();
                stats.misses += 1;
                return None;
            }
        }

        // Update hit count
        entry.hit_count += 1;
        let result = entry.clone();

        // Update stats
        let mut stats = self.stats.write();
        stats.hits += 1;

        Some(result)
    }

    /// Put an embedding in cache
    pub fn put(&self, text: &str, provider: &str, model: &str, vector: Vec<f32>) {
        let key = Self::cache_key(text, provider, model);
        let entry = CachedEmbedding::new(vector, provider.to_string(), model.to_string());

        let mut cache = self.cache.write();
        cache.put(key, entry);
    }

    /// Record a cache miss (for stats tracking)
    pub fn record_miss(&self) {
        let mut stats = self.stats.write();
        stats.misses += 1;
    }

    /// Get cache statistics
    pub fn stats(&self) -> CacheStats {
        let cache = self.cache.read();
        let stats = self.stats.read();

        // Calculate approximate bytes used
        let bytes_used: usize = cache
            .iter()
            .map(|(k, v)| {
                k.len()
                    + v.vector.len() * std::mem::size_of::<f32>()
                    + v.provider.len()
                    + v.model.len()
            })
            .sum();

        let total_requests = stats.hits + stats.misses;
        let hit_rate = if total_requests > 0 {
            (stats.hits as f64 / total_requests as f64) * 100.0
        } else {
            0.0
        };

        CacheStats {
            hits: stats.hits,
            misses: stats.misses,
            entries: cache.len(),
            max_entries: self.config.max_entries,
            bytes_used,
            hit_rate,
        }
    }

    /// Clear all entries from cache
    pub fn clear(&self) {
        let mut cache = self.cache.write();
        cache.clear();
    }

    /// Get the number of entries in cache
    pub fn len(&self) -> usize {
        self.cache.read().len()
    }

    /// Check if cache is empty
    pub fn is_empty(&self) -> bool {
        self.cache.read().is_empty()
    }
}

impl Default for EmbeddingCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_key() {
        let key1 = EmbeddingCache::cache_key("hello", "fasttext", "cc.hu.300");
        let key2 = EmbeddingCache::cache_key("hello", "fasttext", "cc.hu.300");
        let key3 = EmbeddingCache::cache_key("world", "fasttext", "cc.hu.300");

        assert_eq!(key1, key2);
        assert_ne!(key1, key3);
        assert_eq!(key1.len(), 64); // SHA256 hex = 64 chars
    }

    #[test]
    fn test_cache_key_no_collision() {
        // These would collide without length-prefixed encoding
        let key1 = EmbeddingCache::cache_key("ab", "cd", "ef");
        let key2 = EmbeddingCache::cache_key("a", "bcd", "ef");
        let key3 = EmbeddingCache::cache_key("ab", "c", "def");
        let key4 = EmbeddingCache::cache_key("abcdef", "", "");

        // All should be different
        assert_ne!(key1, key2);
        assert_ne!(key1, key3);
        assert_ne!(key1, key4);
        assert_ne!(key2, key3);
        assert_ne!(key2, key4);
        assert_ne!(key3, key4);
    }

    #[test]
    fn test_cache_put_get() {
        let cache = EmbeddingCache::new();

        cache.put("hello", "fasttext", "model", vec![1.0, 2.0, 3.0]);

        let result = cache.get("hello", "fasttext", "model");
        assert!(result.is_some());

        let entry = result.unwrap();
        assert_eq!(entry.vector, vec![1.0, 2.0, 3.0]);
        assert_eq!(entry.hit_count, 1);
    }

    #[test]
    fn test_cache_miss() {
        let cache = EmbeddingCache::new();

        let result = cache.get("nonexistent", "provider", "model");
        assert!(result.is_none());
    }

    #[test]
    fn test_cache_stats() {
        let cache = EmbeddingCache::new();

        cache.put("hello", "fasttext", "model", vec![1.0, 2.0, 3.0]);
        let _ = cache.get("hello", "fasttext", "model"); // hit
        let _ = cache.get("world", "fasttext", "model"); // miss
        cache.record_miss(); // explicit miss

        let stats = cache.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1); // only the explicit miss is recorded
        assert_eq!(stats.entries, 1);
    }

    #[test]
    fn test_cache_clear() {
        let cache = EmbeddingCache::new();

        cache.put("hello", "fasttext", "model", vec![1.0, 2.0, 3.0]);
        assert_eq!(cache.len(), 1);

        cache.clear();
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn test_cache_lru_eviction() {
        let config = CacheConfig::default().with_max_entries(2);
        let cache = EmbeddingCache::with_config(config);

        cache.put("a", "p", "m", vec![1.0]);
        cache.put("b", "p", "m", vec![2.0]);
        cache.put("c", "p", "m", vec![3.0]); // should evict "a"

        assert_eq!(cache.len(), 2);
        assert!(cache.get("a", "p", "m").is_none());
        assert!(cache.get("b", "p", "m").is_some());
        assert!(cache.get("c", "p", "m").is_some());
    }
}
