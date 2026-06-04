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

/// Orphan-rebuild thresholds (see `HnswIndex::needs_rebuild`).
///
/// `remove()` is lazy — removed nodes linger in the graph as orphans until a
/// `rebuild()`. Two triggers reclaim them:
/// - RATIO: an orphan-heavy index (small/medium, where local poisoning is most
///   likely) is rebuilt once orphans exceed `ORPHAN_RATIO_THRESHOLD` of total.
/// - ABSOLUTE: a large index under steady replace churn keeps the ratio low
///   forever, so orphans would accumulate unbounded between full compacts (#81).
///   An absolute cap bounds that growth regardless of ratio.
const MIN_ORPHANS_FOR_REBUILD: usize = 100;
const ORPHAN_RATIO_THRESHOLD: f64 = 0.3;
const MAX_ABSOLUTE_ORPHANS: usize = 10_000;

/// Backstop bound on nodes visited per `search_layer` call (#80).
///
/// When the active vector count is below `ef` (or a neighbourhood is
/// orphan-dense), `results` can't fill to `ef` and the distance-based early-stop
/// never fires, so the layer search would otherwise walk the whole reachable
/// graph. The budget = `ef * VISIT_BUDGET_FACTOR` (floored at `MIN_VISIT_BUDGET`)
/// is set far above what a healthy query touches — healthy queries early-stop by
/// distance long before — so it only caps pathological orphan-dense scans.
const VISIT_BUDGET_FACTOR: usize = 64;
const MIN_VISIT_BUDGET: usize = 1024;

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
        // MAX-heap by distance: BinaryHeap pops the GREATEST, and we want the
        // furthest element to pop/peek first (to trim the ef-capped result set).
        // So a larger distance must compare Greater (normal ordering).
        // NaN is treated as the maximum distance (furthest) → pops first (df5cee21).
        match (self.distance.is_nan(), other.distance.is_nan()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater, // NaN is "furthest" -> pops first
            (false, true) => Ordering::Less,
            (false, false) => self
                .distance
                .partial_cmp(&other.distance)
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
        // MIN-heap by distance via Rust's max-heap BinaryHeap: the frontier must
        // pop the NEAREST first (best-first navigation), so a SMALLER distance
        // must compare Greater (reversed ordering). NaN = maximum distance
        // (furthest) → must pop LAST → compares Less.
        match (self.distance.is_nan(), other.distance.is_nan()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Less, // NaN is "furthest" -> pops last
            (false, true) => Ordering::Greater,
            (false, false) => other
                .distance
                .partial_cmp(&self.distance)
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
    /// Per-index PRNG state for HNSW level assignment. Per-index (not a global
    /// static) so builds are deterministic and free of cross-index/thread
    /// contention. Transient (`#[serde(skip)]`): a loaded index restarts the
    /// sequence, which is fine since levels are only assigned on new inserts.
    #[serde(skip)]
    rng_state: u64,
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
            rng_state: 0x2545_F491_4F6C_DD1D,
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

    /// Check if the index needs rebuilding due to excessive orphan nodes.
    ///
    /// Two triggers (either fires), both ignoring tiny absolute counts:
    /// - RATIO: orphans exceed `ORPHAN_RATIO_THRESHOLD` (30%) of total nodes —
    ///   catches orphan-heavy small/medium indexes where search recall/cost is
    ///   most affected.
    /// - ABSOLUTE: orphans exceed `MAX_ABSOLUTE_ORPHANS` regardless of ratio —
    ///   catches a large index under steady replace churn, whose ratio stays
    ///   under 30% but whose orphans would otherwise grow unbounded between full
    ///   compacts (#81).
    pub fn needs_rebuild(&self) -> bool {
        let orphans = self.orphan_count();
        if orphans < MIN_ORPHANS_FOR_REBUILD {
            return false;
        }
        let total = self.nodes.len();
        if total == 0 {
            return false;
        }
        (orphans as f64 / total as f64) > ORPHAN_RATIO_THRESHOLD || orphans >= MAX_ABSOLUTE_ORPHANS
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
    /// Error only on dimension mismatch or capacity exhaustion.
    /// If `id` already exists, the prior entry is removed first (idempotent)
    /// so WAL-replay based recovery can re-apply ops without error.
    pub fn insert(&mut self, id: &str, vector: &[f32]) -> Result<()> {
        if vector.len() != self.config.dim {
            return Err(IronBaseError::IndexError(format!(
                "Vector dimension mismatch: expected {}, got {}",
                self.config.dim,
                vector.len()
            )));
        }

        // Idempotency guard: if the id is already registered, drop the old
        // node first (lazy-delete from id_to_index). Without this, WAL-replay
        // based recovery after crash — which may re-apply an op that is
        // already present in the loaded .hnsw cache — would error instead
        // of producing the correct final state. Mirrors the fulltext guard
        // at `FulltextIndex::insert_impl` (PR 5).
        if self.id_to_index.contains_key(id) {
            self.remove(id);
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

        // PUSH-NODE-FIRST: register the node (with empty neighbor lists) before any
        // linking, so every distance/neighbor computation below uses ordinary
        // `self.nodes[idx]` indexing — no not-yet-pushed special case. The node has
        // no in-edges yet, so it cannot pollute other nodes' searches this step.
        self.nodes.push(HnswNode {
            id: id.to_string(),
            vector: vector.to_vec(),
            neighbors: vec![Vec::new(); node_level + 1],
            max_layer: node_level,
        });
        self.id_to_index.insert(id.to_string(), node_index);

        // First node — it is the entry point.
        let Some(entry_point) = self.entry_point else {
            self.entry_point = Some(node_index);
            self.max_level = node_level;
            self.dirty = true;
            return Ok(());
        };

        // Own a copy of the query vector: the link loop mutates `self.nodes`, so we
        // cannot hold a borrow of `self.nodes[node_index].vector` across it.
        let query = self.nodes[node_index].vector.clone();
        let m_max0 = self.config.m * 2;
        let m_max = self.config.m;

        // Phase 1 (Algorithm 1): greedy ef=1 descent through layers above node_level.
        let mut current = entry_point;
        for level in (node_level + 1..=self.max_level).rev() {
            current = self.search_layer_greedy(&query, current, level);
        }

        // Phase 2: connect from min(node_level, max_level) down to 0.
        for level in (0..=node_level.min(self.max_level)).rev() {
            let candidates = self.search_layer(&query, current, self.config.ef_construction, level);
            let m = if level == 0 { m_max0 } else { m_max };
            let selected = self.select_neighbors_heuristic(&candidates, m);

            // Set the new node's outgoing edges at this level.
            self.nodes[node_index].neighbors[level] = selected.clone();

            // Reciprocal edges + symmetric heuristic prune (base = the neighbor),
            // so the new node is retained unless the neighbor genuinely has M
            // strictly-better diverse connections — prevents born-as-sink nodes.
            for &nbr in &selected {
                if self.nodes[nbr].neighbors.len() <= level {
                    continue; // defensive: nbr should always have a slot at `level`
                }
                self.nodes[nbr].neighbors[level].push(node_index);
                let cap = if level == 0 { m_max0 } else { m_max };
                if self.nodes[nbr].neighbors[level].len() > cap {
                    let base = self.nodes[nbr].vector.clone();
                    let mut cand: Vec<(usize, f32)> = self.nodes[nbr].neighbors[level]
                        .iter()
                        .map(|&x| (x, self.compute_distance(&base, &self.nodes[x].vector)))
                        .collect();
                    cand.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal));
                    let mut pruned = self.select_neighbors_heuristic(&cand, cap);
                    // Guarantee the reciprocal edge survives so the new node is
                    // never born a sink — critical for the degenerate all-ties
                    // case (many identical vectors), where the just-added node
                    // would otherwise be dropped from every neighbor's list.
                    if !pruned.contains(&node_index) {
                        if let Some(last) = pruned.last_mut() {
                            *last = node_index;
                        }
                    }
                    self.nodes[nbr].neighbors[level] = pruned;
                }
            }

            // Hand off the NEAREST candidate (not a heuristic bridge) to the next
            // lower level, so its search starts close to the query.
            if let Some(&(nearest, _)) = candidates.first() {
                current = nearest;
            }
        }

        // Promote to entry point if this node is taller than the current graph.
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

        // Search at layer 0. search_layer already excludes orphan (lazy-removed)
        // nodes; use ef = max(ef_search, k) so a request for more than ef_search
        // results can still be satisfied.
        let ef = self.config.ef_search.max(k);
        let candidates = self.search_layer(query, current, ef, 0);

        candidates
            .into_iter()
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

        // Search at layer 0 with extra candidates to compensate for the caller's
        // filter dropping results. Orphan (lazy-removed) nodes are already
        // excluded by search_layer, so this budget only covers the caller filter.
        let ef = (self.config.ef_search * 3).max(k); // Expand search space
        let candidates = self.search_layer(query, current, ef, 0);

        candidates
            .into_iter()
            .filter_map(|(idx, _distance)| {
                let node = &self.nodes[idx];
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
        if let Some(&idx) = self.id_to_index.get(id) {
            // Lazy removal: drop from the lookup; the node and its edges stay in
            // the graph as an orphan (search_layer traverses through but never
            // returns orphans). Memory is reclaimed later by rebuild().
            self.id_to_index.remove(id);
            self.dirty = true;

            // If we just removed the graph entry point, re-anchor it to a live
            // node. Otherwise upper-layer descent (search_layer_greedy) would
            // start every search from a dead node, which can settle in an orphan
            // local minimum and hurt recall (#77). Pick the highest-level active
            // node (most neighbor layers) as the new entry; None if none remain.
            if self.entry_point == Some(idx) {
                self.entry_point = self
                    .id_to_index
                    .values()
                    .copied()
                    .max_by_key(|&i| self.nodes[i].neighbors.len());
            }
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

    /// Generate a random level for a new node (geometric, P(level≥k)=(1/M)^k).
    fn random_level(&mut self) -> usize {
        let mut level = 0;
        while self.next_rand() < 1.0 / self.config.m as f64 && level < 16 {
            level += 1;
        }
        level
    }

    /// Per-index LCG step in [0, 1). Deterministic given insert order; `insert`
    /// holds `&mut self`, so there is no concurrent access to one index's state.
    fn next_rand(&mut self) -> f64 {
        self.rng_state = self
            .rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        // top 53 bits → f64 mantissa range [0, 1)
        ((self.rng_state >> 11) as f64) / ((1u64 << 53) as f64)
    }

    /// HNSW SELECT-NEIGHBORS-HEURISTIC (Malkov & Yashunin, Algorithm 4).
    ///
    /// A candidate is kept only when it is closer to the base than to every
    /// already-selected neighbor, which preserves long-range "bridge" edges and
    /// keeps the graph globally navigable (closest-only selection builds
    /// disconnected local pockets and collapses recall as the index grows). The
    /// `keepPrunedConnections` rule is emulated by backfilling up to `m` from the
    /// discarded candidates, so every node still receives `m` edges.
    ///
    /// `candidates` MUST be nearest-first by distance to the base (the order
    /// `search_layer` returns; each tuple is `(node_index, distance_to_base)`).
    /// Every index must exist in `self.nodes` — callers push the new node first.
    fn select_neighbors_heuristic(&self, candidates: &[(usize, f32)], m: usize) -> Vec<usize> {
        let mut selected: Vec<usize> = Vec::with_capacity(m);
        let mut deferred: Vec<usize> = Vec::new();

        for &(cand_idx, dist_to_base) in candidates {
            if selected.len() >= m {
                break;
            }
            let cand_vec = &self.nodes[cand_idx].vector;
            // Keep the candidate only if it is closer to the base than to every
            // already-selected neighbor (diversity / long-range bridge retention).
            let is_diverse = selected.iter().all(|&s_idx| {
                dist_to_base < self.compute_distance(cand_vec, &self.nodes[s_idx].vector)
            });
            if is_diverse {
                selected.push(cand_idx);
            } else {
                deferred.push(cand_idx);
            }
        }

        // keepPrunedConnections: guarantee `m` edges by backfilling with the
        // nearest discarded candidates (deferred preserves nearest-first order).
        for cand_idx in deferred {
            if selected.len() >= m {
                break;
            }
            selected.push(cand_idx);
        }

        selected
    }

    /// Greedy ef=1 hill-climb within a single upper layer (returns the closest node).
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

    /// Search within a layer, returning the `ef` nearest *active* neighbors.
    ///
    /// Faithful HNSW Algorithm 2, with two ef-capped sets so deletion is handled
    /// correctly:
    /// - `w_nav` (the ef nearest **navigable** nodes, orphans included) drives the
    ///   frontier expansion and the early-stop, so traversal follows the true
    ///   graph distances and can pass *through* lazily-removed nodes.
    /// - `w_active` (the ef nearest **active** nodes) is what we return, so orphans
    ///   can never crowd a live node out of the results (e.g. a vector replaced
    ///   many times leaves many same-distance orphans around the live one).
    ///
    /// With zero orphans (always true during build/rebuild) the two sets coincide
    /// → canonical HNSW → connected, navigable graph by construction.
    fn search_layer(
        &self,
        query: &[f32],
        entry: usize,
        ef: usize,
        level: usize,
    ) -> Vec<(usize, f32)> {
        let mut visited = HashSet::new();
        let mut candidates = BinaryHeap::new(); // min-heap (nearest first): frontier
        let mut w_nav = BinaryHeap::new(); // max-heap: ef nearest navigable (drives stop)
        let mut w_active = BinaryHeap::new(); // max-heap: ef nearest active (results)

        let is_active = |idx: usize| self.id_to_index.contains_key(&self.nodes[idx].id);

        // #80 backstop: bound work so an orphan-dense neighborhood can't degrade
        // into a full reachable-graph walk on every query.
        let max_visits = ef.saturating_mul(VISIT_BUDGET_FACTOR).max(MIN_VISIT_BUDGET);

        let consider = |idx: usize,
                        dist: f32,
                        w_nav: &mut BinaryHeap<SearchCandidate>,
                        w_active: &mut BinaryHeap<SearchCandidate>| {
            w_nav.push(SearchCandidate {
                index: idx,
                distance: dist,
            });
            while w_nav.len() > ef {
                w_nav.pop();
            }
            if is_active(idx) {
                w_active.push(SearchCandidate {
                    index: idx,
                    distance: dist,
                });
                while w_active.len() > ef {
                    w_active.pop();
                }
            }
        };

        let entry_dist = self.compute_distance(query, &self.nodes[entry].vector);
        visited.insert(entry);
        candidates.push(NearestCandidate {
            index: entry,
            distance: entry_dist,
        });
        consider(entry, entry_dist, &mut w_nav, &mut w_active);

        while let Some(NearestCandidate {
            index: current,
            distance: current_dist,
        }) = candidates.pop()
        {
            if visited.len() >= max_visits {
                break;
            }
            // Canonical stop: once the navigable set is full, stop when the nearest
            // unexplored frontier node is farther than the furthest node in it.
            if w_nav.len() >= ef {
                if let Some(furthest) = w_nav.peek() {
                    if current_dist > furthest.distance {
                        break;
                    }
                }
            }

            let neighbors = &self.nodes[current].neighbors;
            if level < neighbors.len() {
                for &neighbor_idx in &neighbors[level] {
                    if visited.insert(neighbor_idx) {
                        let dist = self.compute_distance(query, &self.nodes[neighbor_idx].vector);
                        // Expand if the navigable set isn't full, or this node is no
                        // farther than its current furthest. `<=` (not `<`) admits
                        // distance ties so that, in a dense same-distance cluster
                        // (e.g. many identical-vector orphans around one live node),
                        // the live node is still explored and collected into
                        // `w_active` instead of being crowded out by the ties.
                        let promising = w_nav.len() < ef
                            || w_nav.peek().map(|f| dist <= f.distance).unwrap_or(true);
                        if promising {
                            candidates.push(NearestCandidate {
                                index: neighbor_idx,
                                distance: dist,
                            });
                            consider(neighbor_idx, dist, &mut w_nav, &mut w_active);
                        }
                    }
                }
            }
        }

        // Results = the nearest active nodes.
        let mut result_vec: Vec<(usize, f32)> = w_active
            .into_iter()
            .map(|c| (c.index, c.distance))
            .collect();
        result_vec.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal));
        result_vec
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

    /// Small, fast HNSW config for threshold tests (recall quality irrelevant).
    fn fast_cfg(dim: usize) -> VectorIndexConfig {
        let mut cfg = VectorIndexConfig::new(dim);
        cfg.m = 4;
        cfg.ef_construction = 10;
        cfg
    }

    fn euclidean_cfg() -> VectorIndexConfig {
        let mut cfg = VectorIndexConfig::new(2);
        cfg.metric = DistanceMetric::Euclidean;
        cfg
    }

    #[test]
    fn select_neighbors_heuristic_prefers_bridges_then_backfills() {
        let base = [0.0f32, 0.0];

        // Scenario 1 — diversity: A and B are a near-duplicate cluster, C is a
        // far "bridge" in another direction. With m=2 the heuristic must keep
        // {A, C} (a long-range edge) rather than closest-only {A, B}.
        let mut idx = HnswIndex::new(euclidean_cfg());
        idx.insert("A", &[1.0, 0.0]).unwrap();
        idx.insert("B", &[1.05, 0.0]).unwrap();
        idx.insert("C", &[0.0, 1.4]).unwrap();
        let (ia, ib, ic) = (
            idx.id_to_index["A"],
            idx.id_to_index["B"],
            idx.id_to_index["C"],
        );
        let mut cands: Vec<(usize, f32)> = [ia, ib, ic]
            .iter()
            .map(|&i| (i, idx.compute_distance(&base, &idx.nodes[i].vector)))
            .collect();
        cands.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        assert_eq!(
            idx.select_neighbors_heuristic(&cands, 2),
            vec![ia, ic],
            "heuristic must keep the bridge C over near-duplicate B"
        );

        // Scenario 2 — backfill: a tight 1-D cluster. Diversity keeps only A, so
        // keepPrunedConnections must backfill the nearest discarded (B before C).
        let mut idx2 = HnswIndex::new(euclidean_cfg());
        idx2.insert("A", &[1.0, 0.0]).unwrap();
        idx2.insert("B", &[1.01, 0.0]).unwrap();
        idx2.insert("C", &[1.02, 0.0]).unwrap();
        let (ja, jb, jc) = (
            idx2.id_to_index["A"],
            idx2.id_to_index["B"],
            idx2.id_to_index["C"],
        );
        let mut cands2: Vec<(usize, f32)> = [ja, jb, jc]
            .iter()
            .map(|&i| (i, idx2.compute_distance(&base, &idx2.nodes[i].vector)))
            .collect();
        cands2.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        assert_eq!(
            idx2.select_neighbors_heuristic(&cands2, 2),
            vec![ja, jb],
            "backfill must add the nearest discarded candidate"
        );
    }

    // ===================================================================
    // HNSW recall validation harness. Structural diagnostics need private
    // fields, so the harness lives in the unit-test module. The recall gates
    // (recall@10 vs brute force, self-recall@1, structural reachability) are
    // ENFORCED in the default suite; only the verbose on-demand report is
    // #[ignore]d. Builds are deterministic (per-index RNG), so gates don't flake.
    // ===================================================================

    /// Production-grade recall gate (clustered N=2000, ef_search=50).
    const RECALL_TARGET: f64 = 0.95;

    /// Production-like HNSW config for recall tests.
    fn recall_cfg(dim: usize) -> VectorIndexConfig {
        let mut cfg = VectorIndexConfig::new(dim);
        cfg.m = 16;
        cfg.ef_construction = 200;
        cfg.ef_search = 50;
        cfg.metric = DistanceMetric::Cosine;
        cfg
    }

    /// Deterministic clustered vectors (like real RAG embeddings: many
    /// near-duplicates per topic) — the regime where the build defect shows.
    /// Vectors are inserted in order with no removals, so node index `i` ==
    /// vector `i`, and the cluster of vector `i` is `i % clusters`.
    fn gen_clustered(
        n: usize,
        dim: usize,
        clusters: usize,
        noise: f32,
        seed0: u64,
    ) -> Vec<Vec<f32>> {
        let mut seed = seed0;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((seed >> 33) as f64 / (1u64 << 31) as f64 - 1.0) as f32
        };
        let centers: Vec<Vec<f32>> = (0..clusters)
            .map(|_| (0..dim).map(|_| next()).collect())
            .collect();
        (0..n)
            .map(|i| {
                centers[i % clusters]
                    .iter()
                    .map(|&x| x + noise * next())
                    .collect()
            })
            .collect()
    }

    fn gen_uniform(n: usize, dim: usize, seed0: u64) -> Vec<Vec<f32>> {
        let mut seed = seed0;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((seed >> 33) as f64 / (1u64 << 31) as f64 - 1.0) as f32
        };
        (0..n).map(|_| (0..dim).map(|_| next()).collect()).collect()
    }

    fn build_index(cfg: VectorIndexConfig, vectors: &[Vec<f32>]) -> HnswIndex {
        let mut idx = HnswIndex::new(cfg);
        for (i, v) in vectors.iter().enumerate() {
            idx.insert(&format!("v{i}"), v).unwrap();
        }
        idx
    }

    /// Mean recall@k of HNSW search vs brute-force exact k-NN over a query sample.
    /// This is the principled correctness measure (approaches exact), not a tuned
    /// self-recall threshold on pathological data.
    fn recall_at_k_vs_bruteforce(
        idx: &HnswIndex,
        vectors: &[Vec<f32>],
        queries: &[usize],
        k: usize,
    ) -> f64 {
        let metric_cfg = idx.config().clone();
        let dummy = HnswIndex::new(metric_cfg); // for compute_distance with same metric
        let mut total = 0.0f64;
        for &qi in queries {
            let q = &vectors[qi];
            // exact top-k by linear scan
            let mut all: Vec<(usize, f32)> = vectors
                .iter()
                .enumerate()
                .map(|(j, v)| (j, dummy.compute_distance(q, v)))
                .collect();
            all.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            let exact: std::collections::HashSet<usize> =
                all.iter().take(k).map(|(j, _)| *j).collect();
            // HNSW top-k (map result ids "v{j}" back to indices)
            let hnsw: std::collections::HashSet<usize> = idx
                .search(q, k)
                .iter()
                .filter_map(|r| r.id.strip_prefix('v').and_then(|s| s.parse::<usize>().ok()))
                .collect();
            let hit = exact.intersection(&hnsw).count();
            total += hit as f64 / k as f64;
        }
        total / queries.len() as f64
    }

    #[test]
    fn recall_at10_vs_bruteforce_uniform() {
        let vectors = gen_uniform(1500, 48, 0xABCD_1234);
        let idx = build_index(recall_cfg(48), &vectors);
        let queries: Vec<usize> = (0..150).map(|i| i * 10).collect();
        let r = recall_at_k_vs_bruteforce(&idx, &vectors, &queries, 10);
        assert!(r >= 0.95, "uniform recall@10 {r:.3} < 0.95");
    }

    #[test]
    fn recall_at10_vs_bruteforce_clustered() {
        // Moderate clustering (within-cluster cosine high but not degenerate) —
        // representative of real RAG embeddings, not the pathological 0.05 case.
        let vectors = gen_clustered(1500, 48, 40, 0.3, 0xBEEF_5678);
        let idx = build_index(recall_cfg(48), &vectors);
        let queries: Vec<usize> = (0..150).map(|i| i * 10).collect();
        let r = recall_at_k_vs_bruteforce(&idx, &vectors, &queries, 10);
        assert!(r >= 0.95, "clustered recall@10 {r:.3} < 0.95");
    }

    #[test]
    fn structural_every_active_node_reachable() {
        // Targets the historical sink defect: after a build, every active node
        // must be reachable from the entry point (zero stranded nodes).
        let vectors = gen_clustered(1500, 48, 40, 0.3, 0xC0FFEE);
        let idx = build_index(recall_cfg(48), &vectors);
        let (reach_all, _reach_l0, zero_in, _avg, _max) = structural_report(&idx);
        assert_eq!(
            reach_all,
            idx.nodes.len(),
            "all active nodes must be reachable"
        );
        assert_eq!(
            zero_in, 0,
            "no node may have zero layer-0 in-degree (sinks)"
        );
    }

    /// Fraction of vectors whose own embedding self-retrieves at rank 1.
    fn self_recall_at1(idx: &HnswIndex, vectors: &[Vec<f32>]) -> f64 {
        let hits = vectors
            .iter()
            .enumerate()
            .filter(|(i, v)| {
                idx.search(v, 1)
                    .first()
                    .map(|r| r.id == format!("v{i}"))
                    .unwrap_or(false)
            })
            .count();
        hits as f64 / vectors.len() as f64
    }

    /// (reachable_all_layer, reachable_layer0, zero_in_degree, avg_in_degree, max_level)
    fn structural_report(idx: &HnswIndex) -> (usize, usize, usize, f64, usize) {
        let n = idx.nodes.len();
        let entry = idx.entry_point.expect("non-empty index");

        let mut seen = std::collections::HashSet::new();
        seen.insert(entry);
        let mut stack = vec![entry];
        while let Some(node) = stack.pop() {
            for layer in &idx.nodes[node].neighbors {
                for &nb in layer {
                    if seen.insert(nb) {
                        stack.push(nb);
                    }
                }
            }
        }

        let mut seen0 = std::collections::HashSet::new();
        seen0.insert(entry);
        let mut stack0 = vec![entry];
        while let Some(node) = stack0.pop() {
            if let Some(l0) = idx.nodes[node].neighbors.first() {
                for &nb in l0 {
                    if seen0.insert(nb) {
                        stack0.push(nb);
                    }
                }
            }
        }

        let mut indeg = vec![0usize; n];
        for node in 0..n {
            if let Some(l0) = idx.nodes[node].neighbors.first() {
                for &nb in l0 {
                    indeg[nb] += 1;
                }
            }
        }
        let zero_in = indeg.iter().filter(|&&d| d == 0).count();
        let avg_in = indeg.iter().sum::<usize>() as f64 / n as f64;
        (seen.len(), seen0.len(), zero_in, avg_in, idx.max_level)
    }

    /// Fraction of self-queries whose upper-layer greedy descent lands in a
    /// DIFFERENT cluster than the target (diagnoses hierarchy navigation).
    fn descent_miss_rate(idx: &HnswIndex, vectors: &[Vec<f32>], clusters: usize) -> f64 {
        let entry = match idx.entry_point {
            Some(e) => e,
            None => return 1.0,
        };
        let mut miss = 0usize;
        for (i, v) in vectors.iter().enumerate() {
            let mut current = entry;
            for level in (1..=idx.max_level).rev() {
                current = idx.search_layer_greedy(v, current, level);
            }
            if current % clusters != i % clusters {
                miss += 1;
            }
        }
        miss as f64 / vectors.len() as f64
    }

    /// On-demand harness report (recall + structural diagnostics). `#[ignore]`d
    /// (slow); run with `cargo test ... -- --ignored hnsw_recall_baseline`.
    #[test]
    #[ignore = "harness measurement; run on demand with --ignored"]
    fn hnsw_recall_baseline_diagnostic() {
        const N: usize = 2000;
        const DIM: usize = 48;
        const CLUSTERS: usize = 40;
        let vectors = gen_clustered(N, DIM, CLUSTERS, 0.3, 0x9E37_79B9_7F4A_7C15);
        let idx = build_index(recall_cfg(DIM), &vectors);
        let recall = self_recall_at1(&idx, &vectors);
        let (reach_all, reach_l0, zero_in, avg_in, max_level) = structural_report(&idx);
        let dmiss = descent_miss_rate(&idx, &vectors, CLUSTERS);
        eprintln!(
            "HNSW HARNESS N={N} recall@1={recall:.3} (target {RECALL_TARGET}) \
             reachable_all={reach_all}/{N} reachable_l0={reach_l0}/{N} \
             zero_in={zero_in} avg_in={avg_in:.1} max_level={max_level} descent_miss={dmiss:.3}"
        );
        assert_eq!(idx.len(), N);
    }

    /// Production recall gate: self-retrieval@1 must meet RECALL_TARGET, and a
    /// serialization round-trip must preserve it.
    #[test]
    fn hnsw_recall_meets_target() {
        const N: usize = 1500;
        const DIM: usize = 48;
        let vectors = gen_clustered(N, DIM, 40, 0.3, 0x1234_5678_9ABC_DEF0);
        let idx = build_index(recall_cfg(DIM), &vectors);

        let r = self_recall_at1(&idx, &vectors);
        assert!(
            r >= RECALL_TARGET,
            "self-recall@1 {r:.3} < target {RECALL_TARGET}"
        );

        // Round-trip must preserve recall (full adjacency is serialized).
        let idx2 = HnswIndex::from_bytes(&idx.to_bytes().unwrap()).unwrap();
        let r2 = self_recall_at1(&idx2, &vectors);
        assert!(
            (r - r2).abs() < 1e-9,
            "recall changed after to_bytes/from_bytes: {r:.3} -> {r2:.3}"
        );
    }

    #[test]
    fn needs_rebuild_respects_min_and_ratio() {
        let mut idx = HnswIndex::new(fast_cfg(2));
        for i in 0..200 {
            idx.insert(&format!("k{i}"), &[i as f32, 0.0]).unwrap();
        }
        // 50 orphans (< MIN_ORPHANS_FOR_REBUILD=100) → no rebuild even at 25%.
        for i in 0..50 {
            idx.remove(&format!("k{i}"));
        }
        assert!(!idx.needs_rebuild());
        // 150 orphans / 200 total = 75% (> 30%) and ≥ 100 → rebuild.
        for i in 50..150 {
            idx.remove(&format!("k{i}"));
        }
        assert!(idx.needs_rebuild());
    }

    #[test]
    fn needs_rebuild_absolute_cap_fires_below_ratio() {
        // #81: a large index under churn keeps the ratio < 30% but accumulates
        // orphans past MAX_ABSOLUTE_ORPHANS — the absolute cap must trigger.
        let total = 34_000;
        let orphans = MAX_ABSOLUTE_ORPHANS; // 10_000
        let mut idx = HnswIndex::new(fast_cfg(2));
        for i in 0..total {
            idx.insert(&format!("k{i}"), &[i as f32, (i % 7) as f32])
                .unwrap();
        }
        for i in 0..orphans {
            idx.remove(&format!("k{i}"));
        }
        let ratio = idx.orphan_count() as f64 / idx.total_nodes() as f64;
        assert!(
            ratio < ORPHAN_RATIO_THRESHOLD,
            "precondition: ratio must be below the ratio trigger, got {ratio}"
        );
        assert!(idx.orphan_count() >= MAX_ABSOLUTE_ORPHANS);
        assert!(
            idx.needs_rebuild(),
            "absolute orphan cap should trigger rebuild even below the ratio threshold"
        );
    }

    #[test]
    fn search_returns_all_active_with_orphans_under_budget() {
        // #80: few active vectors + orphan churn (under the visit budget) must
        // still return every active node — the backstop bounds work without
        // truncating valid results. Uses a realistic config (good connectivity);
        // the point under test is the budget, not graph quality.
        let mut idx = HnswIndex::with_dim(12);
        for i in 0..12 {
            let mut v = vec![0.0f32; 12];
            v[i] = 1.0; // orthogonal one-hot directions, distinct under cosine
            idx.insert(&format!("a{i}"), &v).unwrap();
        }
        // ~600 orphans (well under MIN_VISIT_BUDGET=1024) via insert+remove.
        for i in 0..600 {
            let id = format!("tmp{i}");
            let mut v = vec![0.0f32; 12];
            v[i % 12] = 0.5;
            v[(i + 1) % 12] = 0.5;
            idx.insert(&id, &v).unwrap();
            idx.remove(&id);
        }
        for i in 0..12 {
            let mut v = vec![0.0f32; 12];
            v[i] = 1.0;
            let res = idx.search(&v, 1);
            assert_eq!(
                res.first().map(|r| r.id.as_str()),
                Some(format!("a{i}").as_str()),
                "active anchor a{i} must self-retrieve despite orphan churn"
            );
        }
    }

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
    fn test_duplicate_id_is_idempotent() {
        // Post-0260ccda: insert() with an existing id removes the prior entry
        // and inserts the new vector. Required so WAL-replay based recovery
        // can re-apply ops that are already present in the loaded .hnsw cache.
        let mut hnsw = HnswIndex::with_dim(3);
        hnsw.insert("doc1", &[1.0, 0.0, 0.0]).unwrap();
        hnsw.insert("doc1", &[0.0, 1.0, 0.0]).unwrap();
        assert_eq!(hnsw.len(), 1);
        // Active id_to_index has exactly one entry for "doc1" — the new vector.
        assert!(hnsw.contains("doc1"));
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

        // SearchCandidate is a MAX-heap (the ef-capped result set): the FURTHEST
        // pops first, and NaN is treated as the maximum distance → pops first.
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

        // Pop order (furthest first): NaN, then 2.0, then 1.0.
        let first = heap.pop().unwrap();
        assert!(
            first.distance.is_nan(),
            "NaN (max distance) should pop first"
        );
        let second = heap.pop().unwrap();
        assert_eq!(second.distance, 2.0);
        let third = heap.pop().unwrap();
        assert_eq!(third.distance, 1.0);
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

        // NearestCandidate is a MIN-heap frontier: the NEAREST pops first, and
        // NaN is treated as the maximum distance (furthest) → pops LAST.
        let mut heap = std::collections::BinaryHeap::new();
        heap.push(NearestCandidate {
            index: 0,
            distance: 1.0,
        });
        heap.push(NearestCandidate {
            index: 1,
            distance: f32::NAN,
        });
        heap.push(NearestCandidate {
            index: 2,
            distance: 0.5,
        });

        // Pop order (nearest first): 0.5, then 1.0, then NaN last.
        assert_eq!(heap.pop().unwrap().distance, 0.5);
        assert_eq!(heap.pop().unwrap().distance, 1.0);
        assert!(
            heap.pop().unwrap().distance.is_nan(),
            "NaN (max distance) should pop last"
        );
    }
}
