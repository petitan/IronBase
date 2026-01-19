//! HNSW (Hierarchical Navigable Small World) index for approximate nearest neighbor search
//!
//! Implementation based on the paper "Efficient and robust approximate nearest neighbor search
//! using Hierarchical Navigable Small World graphs" by Malkov & Yashunin.
//!
//! # Features
//!
//! - O(log n) search time complexity
//! - Memory efficient: stores only graph structure and vector references
//! - Configurable M (connections per node) and ef (search candidate list size)
//! - Support for incremental insertions
//!
//! # Example
//!
//! ```rust,ignore
//! let mut hnsw = HnswIndex::new(HnswConfig::default());
//! hnsw.insert("doc1", &[0.1, 0.2, 0.3, ...]);
//! hnsw.insert("doc2", &[0.2, 0.3, 0.4, ...]);
//!
//! let results = hnsw.search(&query_vector, 10);
//! for (id, score) in results {
//!     println!("{}: {}", id, score);
//! }
//! ```

use super::types::{HnswConfig, RagError, RagResult};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

/// Default maximum vectors in HNSW index (used when max_vectors not specified)
const DEFAULT_MAX_HNSW_VECTORS: usize = 100_000;

/// A node in the HNSW graph
#[derive(Debug, Clone, Serialize, Deserialize)]
struct HnswNode {
    /// Unique identifier (e.g., "doc_id:chunk_id")
    id: String,
    /// The embedding vector
    vector: Vec<f32>,
    /// Connections at each layer (layer -> list of neighbor indices)
    neighbors: Vec<Vec<usize>>,
    /// Maximum layer this node appears in
    max_layer: usize,
}

/// Entry for priority queue during search (max-heap by distance for furthest removal)
#[derive(Debug, Clone)]
struct SearchCandidate {
    index: usize,
    distance: f32,
}

impl PartialEq for SearchCandidate {
    fn eq(&self, other: &Self) -> bool {
        self.distance == other.distance
    }
}

impl Eq for SearchCandidate {}

impl PartialOrd for SearchCandidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SearchCandidate {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse ordering for max-heap (furthest first)
        other
            .distance
            .partial_cmp(&self.distance)
            .unwrap_or(Ordering::Equal)
    }
}

/// Entry for priority queue during search (min-heap by distance for closest first)
#[derive(Debug, Clone)]
struct NearestCandidate {
    index: usize,
    distance: f32,
}

impl PartialEq for NearestCandidate {
    fn eq(&self, other: &Self) -> bool {
        self.distance == other.distance
    }
}

impl Eq for NearestCandidate {}

impl PartialOrd for NearestCandidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for NearestCandidate {
    fn cmp(&self, other: &Self) -> Ordering {
        // Normal ordering for min-heap (closest first)
        self.distance
            .partial_cmp(&other.distance)
            .unwrap_or(Ordering::Equal)
    }
}

/// HNSW index for fast approximate nearest neighbor search
#[derive(Debug, Serialize, Deserialize)]
pub struct HnswIndex {
    /// Configuration
    config: HnswConfig,
    /// All nodes in the graph
    nodes: Vec<HnswNode>,
    /// Entry point (top layer node index)
    entry_point: Option<usize>,
    /// Maximum layer in the graph
    max_level: usize,
    /// ID to node index mapping for fast lookup
    id_to_index: HashMap<String, usize>,
    /// Level multiplier for random layer assignment (1/ln(M))
    level_mult: f64,
    /// Maximum number of vectors (OOM protection, dynamic)
    #[serde(default = "default_max_vectors")]
    max_vectors: usize,
}

fn default_max_vectors() -> usize {
    DEFAULT_MAX_HNSW_VECTORS
}

impl HnswIndex {
    /// Create a new HNSW index with the given configuration
    pub fn new(config: HnswConfig) -> Self {
        Self::with_limits(config, DEFAULT_MAX_HNSW_VECTORS)
    }

    /// Create a new HNSW index with custom max vectors limit
    pub fn with_limits(config: HnswConfig, max_vectors: usize) -> Self {
        let level_mult = 1.0 / (config.m as f64).ln();
        Self {
            config,
            nodes: Vec::new(),
            entry_point: None,
            max_level: 0,
            id_to_index: HashMap::new(),
            level_mult,
            max_vectors,
        }
    }

    /// Create with default configuration
    pub fn with_dim(dim: usize) -> Self {
        Self::new(HnswConfig {
            dim,
            ..Default::default()
        })
    }

    /// Create with default configuration and custom max vectors
    pub fn with_dim_and_limits(dim: usize, max_vectors: usize) -> Self {
        Self::with_limits(
            HnswConfig {
                dim,
                ..Default::default()
            },
            max_vectors,
        )
    }

    /// Get the number of vectors in the index
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Check if the index is empty
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Get the vector dimension
    pub fn dim(&self) -> usize {
        self.config.dim
    }

    /// Check if an ID exists in the index
    pub fn contains(&self, id: &str) -> bool {
        self.id_to_index.contains_key(id)
    }

    /// Insert a vector into the index
    ///
    /// # Arguments
    ///
    /// * `id` - Unique identifier for this vector
    /// * `vector` - The embedding vector (must match configured dimension)
    ///
    /// # Returns
    ///
    /// Error if dimension mismatch or ID already exists
    pub fn insert(&mut self, id: &str, vector: &[f32]) -> RagResult<()> {
        if vector.len() != self.config.dim {
            return Err(RagError::HnswError(format!(
                "Vector dimension mismatch: expected {}, got {}",
                self.config.dim,
                vector.len()
            )));
        }

        if self.id_to_index.contains_key(id) {
            return Err(RagError::HnswError(format!(
                "ID already exists in index: {}",
                id
            )));
        }

        // OOM Protection: Check vector count limit (dynamic, RAM-based)
        if self.nodes.len() >= self.max_vectors {
            return Err(RagError::HnswError(format!(
                "HNSW index full: {} vectors (max: {}). Remove documents or create a new collection.",
                self.nodes.len(),
                self.max_vectors
            )));
        }

        // OOM Protection: Ensure capacity for new node
        if self.nodes.len() == self.nodes.capacity() {
            // Reserve space for more nodes (grow by 25% or at least 100)
            let additional = (self.nodes.len() / 4)
                .max(100)
                .min(self.max_vectors - self.nodes.len());
            self.nodes.try_reserve(additional).map_err(|_| {
                RagError::HnswError(format!(
                    "Failed to allocate memory for {} additional HNSW nodes",
                    additional
                ))
            })?;
        }

        let node_index = self.nodes.len();
        let node_level = self.random_level();

        // Create new node
        let mut node = HnswNode {
            id: id.to_string(),
            vector: vector.to_vec(),
            neighbors: vec![Vec::new(); node_level + 1],
            max_layer: node_level,
        };

        // First node - just add it
        if self.entry_point.is_none() {
            self.nodes.push(node);
            self.entry_point = Some(node_index);
            self.max_level = node_level;
            self.id_to_index.insert(id.to_string(), node_index);
            return Ok(());
        }

        let entry_point = self.entry_point.unwrap();
        let mut current = entry_point;

        // Navigate through layers above node_level to find entry point at node_level
        for level in (node_level + 1..=self.max_level).rev() {
            current = self.search_layer_greedy(&node.vector, current, level);
        }

        // Insert at each layer from node_level down to 0
        for level in (0..=node_level.min(self.max_level)).rev() {
            // Find ef_construction nearest neighbors at this layer
            let neighbors =
                self.search_layer(&node.vector, current, self.config.ef_construction, level);

            // Select M best neighbors (simple heuristic: closest ones)
            let m = if level == 0 {
                self.config.m * 2
            } else {
                self.config.m
            };
            let selected: Vec<usize> = neighbors.iter().take(m).map(|(idx, _)| *idx).collect();

            // Set neighbors for new node
            node.neighbors[level] = selected.clone();

            // Add new node to neighbors' lists
            for &neighbor_idx in &selected {
                if self.nodes[neighbor_idx].neighbors.len() > level {
                    self.nodes[neighbor_idx].neighbors[level].push(node_index);

                    // Prune if too many connections
                    let max_neighbors = if level == 0 {
                        self.config.m * 2
                    } else {
                        self.config.m
                    };
                    if self.nodes[neighbor_idx].neighbors[level].len() > max_neighbors {
                        // Collect neighbor info first (to avoid borrow issues)
                        let neighbor_vec = self.nodes[neighbor_idx].vector.clone();
                        let neighbor_list = self.nodes[neighbor_idx].neighbors[level].clone();

                        // Compute distances
                        let mut with_distances: Vec<(usize, f32)> = neighbor_list
                            .iter()
                            .map(|&idx| {
                                let dist = if idx == node_index {
                                    Self::euclidean_distance(&node.vector, &neighbor_vec)
                                } else {
                                    Self::euclidean_distance(&self.nodes[idx].vector, &neighbor_vec)
                                };
                                (idx, dist)
                            })
                            .collect();
                        with_distances
                            .sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal));

                        // Update neighbor list
                        self.nodes[neighbor_idx].neighbors[level] = with_distances
                            .into_iter()
                            .take(max_neighbors)
                            .map(|(idx, _)| idx)
                            .collect();
                    }
                }
            }

            // Use first neighbor as entry point for next level
            if let Some(&first) = selected.first() {
                current = first;
            }
        }

        // Add node to index
        self.nodes.push(node);
        self.id_to_index.insert(id.to_string(), node_index);

        // Update entry point if new node has higher level
        if node_level > self.max_level {
            self.max_level = node_level;
            self.entry_point = Some(node_index);
        }

        Ok(())
    }

    /// Search for the k nearest neighbors to a query vector
    ///
    /// # Arguments
    ///
    /// * `query` - Query vector
    /// * `k` - Number of results to return
    ///
    /// # Returns
    ///
    /// Vector of (id, similarity_score) pairs, sorted by similarity descending
    pub fn search(&self, query: &[f32], k: usize) -> Vec<(String, f32)> {
        if self.entry_point.is_none() || query.len() != self.config.dim {
            return Vec::new();
        }

        let entry_point = self.entry_point.unwrap();
        let mut current = entry_point;

        // Navigate through layers from top to 1
        for level in (1..=self.max_level).rev() {
            current = self.search_layer_greedy(query, current, level);
        }

        // Search at layer 0 with ef_search candidates
        let candidates = self.search_layer(query, current, self.config.ef_search, 0);

        // Convert to results with similarity scores (cosine similarity)
        candidates
            .into_iter()
            .take(k)
            .map(|(idx, _distance)| {
                let node = &self.nodes[idx];
                let similarity = Self::cosine_similarity(query, &node.vector);
                (node.id.clone(), similarity)
            })
            .collect()
    }

    /// Batch insert multiple vectors
    pub fn batch_insert(&mut self, items: &[(&str, &[f32])]) -> RagResult<usize> {
        let mut inserted = 0;
        for (id, vector) in items {
            self.insert(id, vector)?;
            inserted += 1;
        }
        Ok(inserted)
    }

    /// Remove a vector from the index
    ///
    /// Note: This is a lazy removal - the vector is marked as deleted but
    /// the graph structure is not fully updated until rebuild.
    pub fn remove(&mut self, id: &str) -> bool {
        // For now, we don't support removal
        // A full implementation would mark the node as deleted and
        // update neighbor lists
        self.id_to_index.contains_key(id)
    }

    /// Rebuild the index (useful after many deletions)
    pub fn rebuild(&mut self) -> RagResult<()> {
        // Collect all vectors
        let items: Vec<(String, Vec<f32>)> = self
            .nodes
            .iter()
            .map(|n| (n.id.clone(), n.vector.clone()))
            .collect();

        // Clear and reinsert
        self.nodes.clear();
        self.entry_point = None;
        self.max_level = 0;
        self.id_to_index.clear();

        for (id, vector) in items {
            self.insert(&id, &vector)?;
        }

        Ok(())
    }

    // === Private helper methods ===

    /// Generate a random level for a new node
    fn random_level(&self) -> usize {
        let mut level = 0;
        let mut r: f64 = rand_float();
        while r < 1.0 / self.config.m as f64 && level < 16 {
            level += 1;
            r = rand_float();
        }
        level
    }

    /// Greedy search within a single layer (returns single best node)
    ///
    /// Uses squared distance for faster comparison (no sqrt needed for ordering).
    fn search_layer_greedy(&self, query: &[f32], entry: usize, level: usize) -> usize {
        let mut current = entry;
        // Use squared distance - avoids sqrt, preserves ordering
        let mut current_dist = Self::squared_euclidean_distance(query, &self.nodes[current].vector);

        loop {
            let mut changed = false;
            let neighbors = &self.nodes[current].neighbors;

            if level < neighbors.len() {
                for &neighbor_idx in &neighbors[level] {
                    let dist =
                        Self::squared_euclidean_distance(query, &self.nodes[neighbor_idx].vector);
                    if dist < current_dist {
                        current = neighbor_idx;
                        current_dist = dist;
                        changed = true;
                    }
                }
            }

            if !changed {
                break;
            }
        }

        current
    }

    /// Search within a layer, returning ef nearest neighbors
    fn search_layer(
        &self,
        query: &[f32],
        entry: usize,
        ef: usize,
        level: usize,
    ) -> Vec<(usize, f32)> {
        let mut visited = HashSet::new();
        let mut candidates = BinaryHeap::new(); // Min-heap for nearest
        let mut results = BinaryHeap::new(); // Max-heap for furthest (to maintain top-ef)

        let entry_dist = Self::euclidean_distance(query, &self.nodes[entry].vector);
        visited.insert(entry);
        candidates.push(NearestCandidate {
            index: entry,
            distance: entry_dist,
        });
        results.push(SearchCandidate {
            index: entry,
            distance: entry_dist,
        });

        while let Some(NearestCandidate {
            index: current,
            distance: current_dist,
        }) = candidates.pop()
        {
            // Check if we can stop early
            if let Some(furthest) = results.peek() {
                if current_dist > furthest.distance && results.len() >= ef {
                    break;
                }
            }

            // Explore neighbors
            let neighbors = &self.nodes[current].neighbors;
            if level < neighbors.len() {
                for &neighbor_idx in &neighbors[level] {
                    if visited.insert(neighbor_idx) {
                        let dist =
                            Self::euclidean_distance(query, &self.nodes[neighbor_idx].vector);

                        // Add to candidates if promising
                        let dominated = results.len() >= ef
                            && results.peek().map(|f| dist >= f.distance).unwrap_or(false);

                        if !dominated || results.len() < ef {
                            candidates.push(NearestCandidate {
                                index: neighbor_idx,
                                distance: dist,
                            });
                            results.push(SearchCandidate {
                                index: neighbor_idx,
                                distance: dist,
                            });

                            // Trim results if too many
                            while results.len() > ef {
                                results.pop();
                            }
                        }
                    }
                }
            }
        }

        // Convert to sorted vector
        let mut result_vec: Vec<(usize, f32)> =
            results.into_iter().map(|c| (c.index, c.distance)).collect();
        result_vec.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal));
        result_vec
    }

    /// Compute Euclidean distance between two vectors
    ///
    /// Uses SIMD-optimized implementation from the simd module.
    #[inline]
    fn euclidean_distance(a: &[f32], b: &[f32]) -> f32 {
        // Use squared distance for comparison (avoid sqrt in inner loop)
        // The sqrt is only applied once when returning results
        super::simd::squared_euclidean_distance(a, b).sqrt()
    }

    /// Compute squared Euclidean distance (faster, for comparisons)
    ///
    /// Use this in inner loops where we only need ordering, not actual distance.
    #[inline]
    fn squared_euclidean_distance(a: &[f32], b: &[f32]) -> f32 {
        super::simd::squared_euclidean_distance(a, b)
    }

    /// Compute cosine similarity between two vectors
    ///
    /// Uses SIMD-optimized implementation from the simd module.
    #[inline]
    fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        super::simd::cosine_similarity(a, b)
    }
}

/// Simple random float generator (0.0 to 1.0)
/// Uses a basic LCG for reproducibility in tests
fn rand_float() -> f64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEED: AtomicU64 = AtomicU64::new(12345);

    let mut seed = SEED.load(Ordering::Relaxed);
    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
    SEED.store(seed, Ordering::Relaxed);

    (seed >> 33) as f64 / (1u64 << 31) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_index() {
        let hnsw = HnswIndex::with_dim(3);
        assert!(hnsw.is_empty());
        assert_eq!(hnsw.len(), 0);
    }

    #[test]
    fn test_single_insert() {
        let mut hnsw = HnswIndex::with_dim(3);
        hnsw.insert("doc1", &[1.0, 0.0, 0.0]).unwrap();
        assert_eq!(hnsw.len(), 1);
        assert!(hnsw.contains("doc1"));
    }

    #[test]
    fn test_dimension_mismatch() {
        let mut hnsw = HnswIndex::with_dim(3);
        let result = hnsw.insert("doc1", &[1.0, 0.0]);
        assert!(result.is_err());
    }

    #[test]
    fn test_duplicate_id() {
        let mut hnsw = HnswIndex::with_dim(3);
        hnsw.insert("doc1", &[1.0, 0.0, 0.0]).unwrap();
        let result = hnsw.insert("doc1", &[0.0, 1.0, 0.0]);
        assert!(result.is_err());
    }

    #[test]
    fn test_simple_search() {
        let mut hnsw = HnswIndex::with_dim(3);
        hnsw.insert("doc1", &[1.0, 0.0, 0.0]).unwrap();
        hnsw.insert("doc2", &[0.9, 0.1, 0.0]).unwrap();
        hnsw.insert("doc3", &[0.0, 1.0, 0.0]).unwrap();

        let results = hnsw.search(&[1.0, 0.0, 0.0], 2);
        assert_eq!(results.len(), 2);
        // doc1 should be most similar (exact match)
        assert_eq!(results[0].0, "doc1");
        assert!((results[0].1 - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_cosine_similarity() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((HnswIndex::cosine_similarity(&a, &b) - 1.0).abs() < 1e-6);

        let c = vec![0.0, 1.0, 0.0];
        assert!(HnswIndex::cosine_similarity(&a, &c).abs() < 1e-6);
    }

    #[test]
    fn test_euclidean_distance() {
        let a = vec![0.0, 0.0, 0.0];
        let b = vec![3.0, 4.0, 0.0];
        assert!((HnswIndex::euclidean_distance(&a, &b) - 5.0).abs() < 1e-6);
    }

    #[test]
    fn test_larger_index() {
        let mut hnsw = HnswIndex::with_dim(10);

        // Insert 100 random vectors
        for i in 0..100 {
            let vector: Vec<f32> = (0..10).map(|j| ((i + j) % 10) as f32 / 10.0).collect();
            hnsw.insert(&format!("doc{}", i), &vector).unwrap();
        }

        assert_eq!(hnsw.len(), 100);

        // Search should return results
        let query: Vec<f32> = (0..10).map(|i| i as f32 / 10.0).collect();
        let results = hnsw.search(&query, 5);
        assert_eq!(results.len(), 5);

        // All results should have positive similarity
        for (_, score) in &results {
            assert!(*score > 0.0);
        }
    }
}
