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
//! - Multiple distance metrics: Cosine, Euclidean, DotProduct
//!
//! # Example
//!
//! ```rust,ignore
//! use ironbase_core::vector::{HnswIndex, VectorIndexConfig, DistanceMetric};
//!
//! let config = VectorIndexConfig::new(300)
//!     .with_metric(DistanceMetric::Cosine);
//! let mut hnsw = HnswIndex::new(config);
//!
//! hnsw.insert("doc1", &[0.1, 0.2, 0.3, ...])?;
//! hnsw.insert("doc2", &[0.2, 0.3, 0.4, ...])?;
//!
//! let results = hnsw.search(&query_vector, 10);
//! for (id, score) in results {
//!     println!("{}: {}", id, score);
//! }
//! ```

use super::config::{DistanceMetric, VectorIndexConfig};
use super::simd;
use crate::error::{IronBaseError, Result};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

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
        // Handle NaN: treat NaN values as equal to maintain consistent ordering behavior
        // This is important because NaN != NaN in IEEE 754, which would break BinaryHeap
        if self.distance.is_nan() && other.distance.is_nan() {
            return true;
        }
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
        // Handle NaN: treat NaN as maximum distance (largest Ord -> pops first from BinaryHeap)
        match (self.distance.is_nan(), other.distance.is_nan()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater, // NaN is "furthest" -> largest Ord -> pops first
            (false, true) => Ordering::Less,    // normal < NaN
            (false, false) => other
                .distance
                .partial_cmp(&self.distance)
                .unwrap_or(Ordering::Equal),
        }
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
        // Handle NaN: treat NaN values as equal to maintain consistent ordering behavior
        if self.distance.is_nan() && other.distance.is_nan() {
            return true;
        }
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
        // Handle NaN: treat NaN as maximum distance (push to back of min-heap)
        match (self.distance.is_nan(), other.distance.is_nan()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater, // NaN is "furthest" -> comes last in min-heap
            (false, true) => Ordering::Less,
            (false, false) => self
                .distance
                .partial_cmp(&other.distance)
                .unwrap_or(Ordering::Equal),
        }
    }
}

/// Result of a vector search
#[derive(Debug, Clone)]
pub struct VectorSearchResult {
    /// Document ID
    pub id: String,
    /// Similarity/distance score (meaning depends on metric)
    pub score: f32,
}

/// HNSW index for fast approximate nearest neighbor search
#[derive(Debug, Serialize, Deserialize)]
pub struct HnswIndex {
    /// Configuration
    config: VectorIndexConfig,
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
    /// Whether the index has been modified since last save
    #[serde(skip)]
    dirty: bool,
    /// WAL-replay recovery watermark (task #26): tx_id of the most-recent
    /// committed transaction whose data is durable in the persisted `.hnsw`
    /// cache file. Serialized OUTSIDE the bincode body so backward compat
    /// with v2 (no watermark) is preserved — see `to_bytes` / `from_bytes`.
    #[serde(skip)]
    last_flushed_tx_id: u64,
}

impl HnswIndex {
    /// Create a new HNSW index with the given configuration
    pub fn new(config: VectorIndexConfig) -> Self {
        let level_mult = 1.0 / (config.m as f64).ln();
        Self {
            config,
            nodes: Vec::new(),
            entry_point: None,
            max_level: 0,
            id_to_index: HashMap::new(),
            level_mult,
            dirty: false,
            last_flushed_tx_id: 0,
        }
    }

    /// Create with specified dimension only (uses default config)
    pub fn with_dim(dim: usize) -> Self {
        Self::new(VectorIndexConfig::new(dim))
    }

    /// Create with dimension and custom max vectors limit
    pub fn with_dim_and_limits(dim: usize, max_vectors: usize) -> Self {
        Self::new(VectorIndexConfig::new(dim).with_max_vectors(max_vectors))
    }

    /// Get the number of active (non-orphan) vectors in the index
    pub fn len(&self) -> usize {
        self.id_to_index.len()
    }

    /// Check if the index is empty
    pub fn is_empty(&self) -> bool {
        self.id_to_index.is_empty()
    }

    /// Get the total number of nodes including orphans from lazy removal
    pub fn total_nodes(&self) -> usize {
        self.nodes.len()
    }

    /// Get the number of orphan nodes (removed but still in memory)
    pub fn orphan_count(&self) -> usize {
        self.nodes.len().saturating_sub(self.id_to_index.len())
    }

    /// Check if the index needs rebuilding due to excessive orphan nodes
    ///
    /// Returns true if orphan ratio exceeds 30% and there are at least 100 orphans.
    /// Small absolute counts are ignored to avoid unnecessary rebuilds.
    pub fn needs_rebuild(&self) -> bool {
        let orphans = self.orphan_count();
        if orphans < 100 {
            return false;
        }
        let total = self.nodes.len();
        if total == 0 {
            return false;
        }
        (orphans as f64 / total as f64) > 0.3
    }

    /// Rebuild the index only if orphan ratio exceeds threshold.
    /// Returns Ok(true) if rebuilt, Ok(false) if not needed.
    pub fn rebuild_if_needed(&mut self) -> Result<bool> {
        if self.needs_rebuild() {
            let orphans = self.orphan_count();
            let active = self.id_to_index.len();
            self.rebuild()?;
            crate::log_info!(
                "HNSW index rebuilt: removed {} orphan nodes, {} active vectors remain",
                orphans,
                active
            );
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Get the vector dimension
    pub fn dim(&self) -> usize {
        self.config.dim
    }

    /// Get the distance metric
    pub fn metric(&self) -> DistanceMetric {
        self.config.metric
    }

    /// Check if an ID exists in the index
    pub fn contains(&self, id: &str) -> bool {
        self.id_to_index.contains_key(id)
    }

    /// Check if the index has been modified since last save
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Mark the index as clean (after saving)
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    /// Get the configuration
    pub fn config(&self) -> &VectorIndexConfig {
        &self.config
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
    pub fn insert(&mut self, id: &str, vector: &[f32]) -> Result<()> {
        if vector.len() != self.config.dim {
            return Err(IronBaseError::IndexError(format!(
                "Vector dimension mismatch: expected {}, got {}",
                self.config.dim,
                vector.len()
            )));
        }

        if self.id_to_index.contains_key(id) {
            return Err(IronBaseError::IndexError(format!(
                "ID already exists in vector index: {}",
                id
            )));
        }

        // OOM Protection: Check vector count limit
        // Use self.len() (= id_to_index.len()) to count only active vectors,
        // not self.nodes.len() which includes orphans from lazy removal
        if self.len() >= self.config.max_vectors {
            return Err(IronBaseError::OutOfMemory(format!(
                "Vector index full: {} vectors (max: {}). Remove documents or increase max_vectors.",
                self.len(),
                self.config.max_vectors
            )));
        }

        // OOM Protection: Ensure capacity for new node
        if self.nodes.len() == self.nodes.capacity() {
            let additional = (self.nodes.len() / 4)
                .max(100)
                .min(self.config.max_vectors - self.nodes.len());
            self.nodes.try_reserve(additional).map_err(|_| {
                IronBaseError::OutOfMemory(format!(
                    "Failed to allocate memory for {} additional HNSW nodes. \
                     Consider: 1) Reduce max_vectors, 2) Increase system memory, 3) Use smaller vectors.",
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
        let Some(entry_point) = self.entry_point else {
            self.nodes.push(node);
            self.entry_point = Some(node_index);
            self.max_level = node_level;
            self.id_to_index.insert(id.to_string(), node_index);
            self.dirty = true;
            return Ok(());
        };
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
                                    self.compute_distance(&node.vector, &neighbor_vec)
                                } else {
                                    self.compute_distance(&self.nodes[idx].vector, &neighbor_vec)
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

        self.dirty = true;
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
    /// Vector of results sorted by similarity/distance
    pub fn search(&self, query: &[f32], k: usize) -> Vec<VectorSearchResult> {
        let Some(entry_point) = self.entry_point else {
            return Vec::new();
        };
        if query.len() != self.config.dim {
            return Vec::new();
        }
        let mut current = entry_point;

        // Navigate through layers from top to 1
        for level in (1..=self.max_level).rev() {
            current = self.search_layer_greedy(query, current, level);
        }

        // Search at layer 0 with ef_search candidates
        let candidates = self.search_layer(query, current, self.config.ef_search, 0);

        // Convert to results with similarity scores based on metric
        // Filter out orphan nodes: after lazy remove(), nodes remain in the graph
        // but are no longer in id_to_index. Only return active (non-orphan) results.
        candidates
            .into_iter()
            .filter(|(idx, _)| {
                let node = &self.nodes[*idx];
                self.id_to_index.contains_key(&node.id)
            })
            .take(k)
            .map(|(idx, _distance)| {
                let node = &self.nodes[idx];
                let score = self.compute_similarity(query, &node.vector);
                VectorSearchResult {
                    id: node.id.clone(),
                    score,
                }
            })
            .collect()
    }

    /// Search with a filter function
    ///
    /// Only returns results where the filter returns true for the document ID.
    pub fn search_with_filter<F>(
        &self,
        query: &[f32],
        k: usize,
        filter: F,
    ) -> Vec<VectorSearchResult>
    where
        F: Fn(&str) -> bool,
    {
        let Some(entry_point) = self.entry_point else {
            return Vec::new();
        };
        if query.len() != self.config.dim {
            return Vec::new();
        }
        let mut current = entry_point;

        // Navigate through layers from top to 1
        for level in (1..=self.max_level).rev() {
            current = self.search_layer_greedy(query, current, level);
        }

        // Search at layer 0 with more candidates to compensate for filtering
        let ef = self.config.ef_search * 3; // Expand search space
        let candidates = self.search_layer(query, current, ef, 0);

        // Filter and convert to results
        // First filter out orphan nodes (lazy-deleted), then apply caller's filter.
        candidates
            .into_iter()
            .filter_map(|(idx, _distance)| {
                let node = &self.nodes[idx];
                if !self.id_to_index.contains_key(&node.id) {
                    return None; // orphan node — skip
                }
                if filter(&node.id) {
                    let score = self.compute_similarity(query, &node.vector);
                    Some(VectorSearchResult {
                        id: node.id.clone(),
                        score,
                    })
                } else {
                    None
                }
            })
            .take(k)
            .collect()
    }

    /// Batch insert multiple vectors
    pub fn batch_insert(&mut self, items: &[(&str, &[f32])]) -> Result<usize> {
        let mut inserted = 0;
        for (id, vector) in items {
            self.insert(id, vector)?;
            inserted += 1;
        }
        Ok(inserted)
    }

    /// Upsert a vector - update if exists, insert if not
    ///
    /// For updates, only the vector is changed. The graph structure (edges)
    /// remains the same, which may slightly reduce accuracy but is much faster
    /// than removing and re-inserting.
    ///
    /// # Returns
    ///
    /// true if updated, false if inserted
    pub fn upsert(&mut self, id: &str, vector: &[f32]) -> Result<bool> {
        if vector.len() != self.config.dim {
            return Err(IronBaseError::IndexError(format!(
                "Vector dimension mismatch: expected {}, got {}",
                self.config.dim,
                vector.len()
            )));
        }

        if let Some(&node_idx) = self.id_to_index.get(id) {
            // Update existing node's vector
            self.nodes[node_idx].vector = vector.to_vec();
            self.dirty = true;
            Ok(true) // was update
        } else {
            // Insert new
            self.insert(id, vector)?;
            Ok(false) // was insert
        }
    }

    /// Remove a vector from the index (lazy removal)
    ///
    /// Note: This marks the node as deleted but doesn't fully update
    /// the graph structure. Call rebuild() after many deletions.
    pub fn remove(&mut self, id: &str) -> bool {
        if let Some(&_idx) = self.id_to_index.get(id) {
            // For now, just remove from lookup (lazy removal)
            self.id_to_index.remove(id);
            self.dirty = true;
            true
        } else {
            false
        }
    }

    /// Rebuild the index (useful after many deletions)
    pub fn rebuild(&mut self) -> Result<()> {
        // Collect active vectors using id_to_index as the source of truth.
        // IMPORTANT: iterate id_to_index (HashMap, unique keys) instead of nodes,
        // because nodes may contain duplicate IDs after remove+reinsert cycles
        // (orphan node + new active node with same ID).
        let items: Vec<(String, Vec<f32>)> = self
            .id_to_index
            .iter()
            .map(|(id, &idx)| (id.clone(), self.nodes[idx].vector.clone()))
            .collect();

        // Clear and reinsert
        self.nodes.clear();
        self.entry_point = None;
        self.max_level = 0;
        self.id_to_index.clear();

        for (id, vector) in items {
            self.insert(&id, &vector)?;
        }

        self.dirty = true;
        Ok(())
    }

    /// Get vector by ID
    pub fn get_vector(&self, id: &str) -> Option<&[f32]> {
        self.id_to_index
            .get(id)
            .map(|&idx| self.nodes[idx].vector.as_slice())
    }

    /// HNSW serialization format version
    /// Version 1: Original bincode format (no header)
    /// Version 2: Magic header + version + bincode data
    /// Version 3: Magic header + version + last_flushed_tx_id (u64) + bincode data
    ///            — watermark for WAL-replay-based index recovery (task #26)
    const SERIALIZATION_VERSION: u32 = 3;
    const MAGIC_HEADER: &'static [u8; 4] = b"HNSW";

    /// Serialize the index to bytes (for cache file)
    ///
    /// Format v3: [HNSW magic 4B][version u32 LE][last_flushed_tx_id u64 LE][bincode data...]
    /// Format v2 (legacy): [HNSW magic 4B][version u32 LE][bincode data...]
    ///
    /// v3 adds the WAL-replay watermark outside the bincode body so it can
    /// be read/skipped without parsing the graph. Writer always emits v3;
    /// reader accepts v1, v2, and v3 for backward compat.
    ///
    /// NOTE: This allocates the entire serialized index into a Vec<u8>.
    /// For flush-to-disk, prefer `save_to_writer()` which streams directly
    /// to a file via `bincode::serialize_into`, avoiding the ~2x peak memory.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let data = bincode::serialize(self).map_err(|e| {
            IronBaseError::Serialization(format!("Failed to serialize HNSW index: {}", e))
        })?;

        let mut buf = Vec::with_capacity(16 + data.len());
        buf.extend_from_slice(Self::MAGIC_HEADER);
        buf.extend_from_slice(&Self::SERIALIZATION_VERSION.to_le_bytes());
        buf.extend_from_slice(&self.last_flushed_tx_id.to_le_bytes());
        buf.extend_from_slice(&data);

        Ok(buf)
    }

    /// Streaming serialize: write directly to a writer without intermediate Vec<u8>.
    ///
    /// Same v3 format as `to_bytes()`. Compatible with `from_bytes()` for
    /// deserialization.
    pub fn save_to_writer(&self, writer: &mut impl std::io::Write) -> Result<()> {
        writer.write_all(Self::MAGIC_HEADER).map_err(|e| {
            IronBaseError::Io(std::io::Error::other(format!(
                "Failed to write HNSW header: {}",
                e
            )))
        })?;
        writer
            .write_all(&Self::SERIALIZATION_VERSION.to_le_bytes())
            .map_err(|e| {
                IronBaseError::Io(std::io::Error::other(format!(
                    "Failed to write HNSW version: {}",
                    e
                )))
            })?;
        writer
            .write_all(&self.last_flushed_tx_id.to_le_bytes())
            .map_err(|e| {
                IronBaseError::Io(std::io::Error::other(format!(
                    "Failed to write HNSW watermark: {}",
                    e
                )))
            })?;
        bincode::serialize_into(writer, self).map_err(|e| {
            IronBaseError::Serialization(format!("Failed to serialize HNSW index: {}", e))
        })?;
        Ok(())
    }

    /// Deserialize the index from bytes
    ///
    /// Supports v1 (legacy, no header), v2 (header, no watermark), and v3
    /// (header + 8-byte watermark) formats. Older formats deserialize with
    /// `last_flushed_tx_id = 0`, which triggers full rebuild fallback on
    /// crash recovery.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() >= 8 && &bytes[0..4] == Self::MAGIC_HEADER {
            let version = u32::from_le_bytes(bytes[4..8].try_into().map_err(|_| {
                IronBaseError::Deserialization(serde_json::Error::io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "HNSW header version bytes corrupted",
                )))
            })?);

            if version > Self::SERIALIZATION_VERSION {
                return Err(IronBaseError::Serialization(format!(
                    "HNSW cache file version {} is newer than supported version {}",
                    version,
                    Self::SERIALIZATION_VERSION
                )));
            }

            // Determine bincode start offset + watermark based on version
            let (bincode_start, watermark) = if version >= 3 {
                if bytes.len() < 16 {
                    return Err(IronBaseError::Deserialization(serde_json::Error::io(
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "HNSW v3 header truncated (watermark missing)",
                        ),
                    )));
                }
                let wm = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
                (16usize, wm)
            } else {
                (8usize, 0u64)
            };

            let mut index: Self = bincode::deserialize(&bytes[bincode_start..]).map_err(|e| {
                IronBaseError::Deserialization(serde_json::Error::io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Failed to deserialize HNSW index v{}: {}", version, e),
                )))
            })?;
            index.last_flushed_tx_id = watermark;
            return Ok(index);
        }

        // v1 (legacy): no header, raw bincode
        bincode::deserialize(bytes).map_err(|e| {
            IronBaseError::Deserialization(serde_json::Error::io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Failed to deserialize HNSW index (legacy format): {}", e),
            )))
        })
    }

    /// WAL-replay recovery watermark. Returns the `tx_id` of the most-recent
    /// committed transaction whose data is durable in the persisted file.
    /// Zero for fresh / legacy indexes → full rebuild fallback.
    pub fn last_flushed_tx_id(&self) -> u64 {
        self.last_flushed_tx_id
    }

    /// Advance the watermark (monotonic — never regresses). Called at
    /// flush start with `DatabaseCore::watermark_tx_id()` so the next
    /// flush stamps it into the persisted file.
    pub fn set_flushed_tx_id(&mut self, tx_id: u64) {
        if tx_id > self.last_flushed_tx_id {
            self.last_flushed_tx_id = tx_id;
        }
    }

    // === Private helper methods ===

    /// Compute distance based on configured metric
    #[inline]
    fn compute_distance(&self, a: &[f32], b: &[f32]) -> f32 {
        match self.config.metric {
            DistanceMetric::Euclidean => simd::squared_euclidean_distance(a, b),
            DistanceMetric::Cosine => {
                // Convert cosine similarity to distance (1 - similarity)
                1.0 - simd::cosine_similarity(a, b)
            }
            DistanceMetric::DotProduct => {
                // Negate dot product (higher is better -> lower distance)
                -simd::dot_product(a, b)
            }
        }
    }

    /// Compute similarity score for results (higher = more similar)
    #[inline]
    fn compute_similarity(&self, a: &[f32], b: &[f32]) -> f32 {
        match self.config.metric {
            DistanceMetric::Euclidean => {
                // Convert distance to similarity: 1 / (1 + distance)
                1.0 / (1.0 + simd::euclidean_distance(a, b))
            }
            DistanceMetric::Cosine => simd::cosine_similarity(a, b),
            DistanceMetric::DotProduct => simd::dot_product(a, b),
        }
    }

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
    fn search_layer_greedy(&self, query: &[f32], entry: usize, level: usize) -> usize {
        let mut current = entry;
        let mut current_dist = self.compute_distance(query, &self.nodes[current].vector);

        loop {
            let mut changed = false;
            let neighbors = &self.nodes[current].neighbors;

            if level < neighbors.len() {
                for &neighbor_idx in &neighbors[level] {
                    let dist = self.compute_distance(query, &self.nodes[neighbor_idx].vector);
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

        let entry_dist = self.compute_distance(query, &self.nodes[entry].vector);
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
                        let dist = self.compute_distance(query, &self.nodes[neighbor_idx].vector);

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
}

/// Thread-safe random float generator (0.0 to 1.0)
///
/// Uses a basic LCG with atomic compare-exchange to ensure thread safety.
/// The compare_exchange_weak loop guarantees that concurrent calls don't
/// lose updates (which would result in duplicate random values).
fn rand_float() -> f64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEED: AtomicU64 = AtomicU64::new(12345);

    loop {
        let old_seed = SEED.load(Ordering::Relaxed);
        // LCG multiplier from Knuth's MMIX (PCG family)
        let new_seed = old_seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);

        // Atomic read-modify-write: retry if another thread modified SEED
        if SEED
            .compare_exchange_weak(old_seed, new_seed, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return (new_seed >> 33) as f64 / (1u64 << 31) as f64;
        }
        // Another thread won the race - retry with new seed value
    }
}

// ============================================================================
// LazyLoadable Trait Implementation
// ============================================================================
//
// Note: HNSW's graph structure makes true lazy loading complex - the search
// algorithm needs to traverse multiple layers and neighbors. Unlike B+ tree
// (linear path) or fulltext (independent token entries), HNSW would require
// loading connected nodes which defeats the purpose of lazy loading.
//
// For now, HNSW always loads fully into memory. Future optimization could use
// memory-mapped files for the vector data while keeping the graph in memory.

use crate::index::traits::{IndexTrait, LazyLoadable};

impl IndexTrait for HnswIndex {
    fn name(&self) -> &str {
        // HNSW indexes don't have a name field - return a placeholder
        "hnsw_index"
    }

    fn fields(&self) -> Vec<&str> {
        if self.config.field.is_empty() {
            vec![]
        } else {
            vec![&self.config.field]
        }
    }

    fn entry_count(&self) -> usize {
        self.id_to_index.len()
    }

    fn memory_usage_bytes(&self) -> usize {
        // Estimate: each node has a vector (dim * 4 bytes) + neighbors + metadata
        let dim = self.config.dim;
        let vector_bytes = dim * 4; // f32 = 4 bytes
        let neighbors_bytes = self.config.m * 2 * 8; // ~2*M neighbors per node, usize each
        let metadata_bytes = 64; // id string, layer info, etc.

        let per_node = vector_bytes + neighbors_bytes + metadata_bytes;
        let base_struct = std::mem::size_of::<Self>();

        base_struct + self.nodes.len() * per_node
    }

    fn is_disk_backed(&self) -> bool {
        false // HNSW is currently memory-only (serialized via serde when saved)
    }

    fn flush(&mut self) -> Result<()> {
        // HNSW doesn't have incremental flush - uses serde for full serialization
        Ok(())
    }
}

impl LazyLoadable for HnswIndex {
    fn is_lazy_mode(&self) -> bool {
        false // HNSW doesn't support lazy loading yet
    }

    fn ensure_fully_loaded(&mut self) -> Result<()> {
        // HNSW is always fully loaded
        Ok(())
    }

    fn persisted_size_bytes(&self) -> Option<u64> {
        // HNSW doesn't track persisted size
        None
    }

    fn hot_ratio(&self) -> f32 {
        1.0 // Always fully loaded
    }
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
        assert_eq!(results[0].id, "doc1");
        assert!((results[0].score - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_upsert() {
        let mut hnsw = HnswIndex::with_dim(3);

        // First upsert - insert
        let was_update = hnsw.upsert("doc1", &[1.0, 0.0, 0.0]).unwrap();
        assert!(!was_update);
        assert_eq!(hnsw.len(), 1);

        // Second upsert - update
        let was_update = hnsw.upsert("doc1", &[0.5, 0.5, 0.0]).unwrap();
        assert!(was_update);
        assert_eq!(hnsw.len(), 1);

        // Verify vector was updated
        let vec = hnsw.get_vector("doc1").unwrap();
        assert!((vec[0] - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_distance_metrics() {
        // Test Euclidean
        let config = VectorIndexConfig::new(3).with_metric(DistanceMetric::Euclidean);
        let mut hnsw = HnswIndex::new(config);
        hnsw.insert("doc1", &[0.0, 0.0, 0.0]).unwrap();
        hnsw.insert("doc2", &[3.0, 4.0, 0.0]).unwrap();

        let results = hnsw.search(&[0.0, 0.0, 0.0], 2);
        // doc1 should be closest (distance 0)
        assert_eq!(results[0].id, "doc1");

        // Test DotProduct
        let config = VectorIndexConfig::new(3).with_metric(DistanceMetric::DotProduct);
        let mut hnsw = HnswIndex::new(config);
        hnsw.insert("doc1", &[1.0, 0.0, 0.0]).unwrap();
        hnsw.insert("doc2", &[0.5, 0.5, 0.0]).unwrap();

        let results = hnsw.search(&[1.0, 0.0, 0.0], 2);
        // doc1 should have higher dot product with query
        assert_eq!(results[0].id, "doc1");
    }

    #[test]
    fn test_search_with_filter() {
        let mut hnsw = HnswIndex::with_dim(3);
        hnsw.insert("doc:1:0", &[1.0, 0.0, 0.0]).unwrap();
        hnsw.insert("doc:1:1", &[0.9, 0.1, 0.0]).unwrap();
        hnsw.insert("doc:2:0", &[0.8, 0.2, 0.0]).unwrap();
        hnsw.insert("doc:2:1", &[0.0, 1.0, 0.0]).unwrap();

        // Search only within doc:1
        let results = hnsw.search_with_filter(&[1.0, 0.0, 0.0], 10, |id| id.starts_with("doc:1:"));

        assert_eq!(results.len(), 2);
        for r in &results {
            assert!(r.id.starts_with("doc:1:"));
        }
    }

    #[test]
    fn test_larger_index() {
        let mut hnsw = HnswIndex::with_dim(10);

        // Insert 100 vectors
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
        for r in &results {
            assert!(r.score > 0.0);
        }
    }

    #[test]
    fn test_serialization() {
        let mut hnsw = HnswIndex::with_dim(3);
        hnsw.insert("doc1", &[1.0, 0.0, 0.0]).unwrap();
        hnsw.insert("doc2", &[0.0, 1.0, 0.0]).unwrap();

        // Serialize
        let bytes = hnsw.to_bytes().unwrap();

        // Deserialize
        let hnsw2 = HnswIndex::from_bytes(&bytes).unwrap();

        assert_eq!(hnsw2.len(), 2);
        assert!(hnsw2.contains("doc1"));
        assert!(hnsw2.contains("doc2"));
    }

    #[test]
    fn test_dirty_flag() {
        let mut hnsw = HnswIndex::with_dim(3);
        assert!(!hnsw.is_dirty());

        hnsw.insert("doc1", &[1.0, 0.0, 0.0]).unwrap();
        assert!(hnsw.is_dirty());

        hnsw.mark_clean();
        assert!(!hnsw.is_dirty());

        hnsw.upsert("doc1", &[0.5, 0.5, 0.0]).unwrap();
        assert!(hnsw.is_dirty());
    }

    #[test]
    fn test_search_candidate_nan_handling() {
        use std::collections::BinaryHeap;

        // Test NaN equality
        let nan1 = SearchCandidate {
            index: 0,
            distance: f32::NAN,
        };
        let nan2 = SearchCandidate {
            index: 1,
            distance: f32::NAN,
        };
        let normal = SearchCandidate {
            index: 2,
            distance: 1.0,
        };

        // NaN == NaN should be true for consistent heap behavior
        assert!(nan1 == nan2);
        // NaN != normal
        assert!(nan1 != normal);

        // Test heap ordering: NaN should be treated as corrupted data to remove
        // SearchCandidate uses reverse ordering, so BinaryHeap pops smallest distance first
        // NaN gets largest Ord to pop first (remove bad data immediately)
        let mut heap = BinaryHeap::new();
        heap.push(SearchCandidate {
            index: 0,
            distance: 1.0,
        });
        heap.push(SearchCandidate {
            index: 1,
            distance: f32::NAN,
        });
        heap.push(SearchCandidate {
            index: 2,
            distance: 2.0,
        });

        // NaN should pop first (has largest Ord, treated as corrupted data to remove)
        let first = heap.pop().unwrap();
        assert!(
            first.distance.is_nan(),
            "NaN should be popped first to remove corrupted data"
        );

        // With reverse ordering: smaller distance = larger Ord = pops next
        // So order is: 1.0, then 2.0
        let second = heap.pop().unwrap();
        assert_eq!(second.distance, 1.0);

        let third = heap.pop().unwrap();
        assert_eq!(third.distance, 2.0);
    }

    #[test]
    fn test_nearest_candidate_nan_handling() {
        // Test NaN equality
        let nan1 = NearestCandidate {
            index: 0,
            distance: f32::NAN,
        };
        let nan2 = NearestCandidate {
            index: 1,
            distance: f32::NAN,
        };
        let normal = NearestCandidate {
            index: 2,
            distance: 1.0,
        };

        // NaN == NaN should be true for consistent heap behavior
        assert!(nan1 == nan2);
        // NaN != normal
        assert!(nan1 != normal);

        // Test ordering: NaN should be treated as maximum distance
        // In min-heap (NearestCandidate), NaN comes last
        let mut candidates = [
            NearestCandidate {
                index: 0,
                distance: 1.0,
            },
            NearestCandidate {
                index: 1,
                distance: f32::NAN,
            },
            NearestCandidate {
                index: 2,
                distance: 0.5,
            },
        ];

        // Sort ascending (closest first)
        candidates.sort();

        // 0.5 should be first, then 1.0, then NaN
        assert_eq!(candidates[0].distance, 0.5);
        assert_eq!(candidates[1].distance, 1.0);
        assert!(
            candidates[2].distance.is_nan(),
            "NaN should be last in sorted order"
        );
    }
}
