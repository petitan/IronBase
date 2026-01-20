//! Vector index configuration types
//!
//! This module defines the configuration structures for vector indexes,
//! including HNSW parameters and distance metrics.

use crate::log_warn;
use serde::{Deserialize, Serialize};

/// Maximum supported vector dimension
pub const MAX_VECTOR_DIM: usize = 4096;

/// Default maximum vectors per index
pub const DEFAULT_MAX_VECTORS: usize = 100_000;

/// Warning threshold for vector count
pub const VECTOR_COUNT_WARNING_THRESHOLD: usize = 1_000_000;

/// Distance metric for vector similarity
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DistanceMetric {
    /// Cosine similarity (normalized, range [-1, 1], higher is more similar)
    #[default]
    Cosine,
    /// Euclidean (L2) distance (lower is more similar)
    Euclidean,
    /// Dot product / inner product (higher is more similar)
    DotProduct,
}

impl DistanceMetric {
    /// Returns true if higher scores mean more similar
    pub fn higher_is_better(&self) -> bool {
        match self {
            DistanceMetric::Cosine => true,
            DistanceMetric::Euclidean => false,
            DistanceMetric::DotProduct => true,
        }
    }

    /// Convert a raw distance to a similarity score in [0, 1] range
    pub fn to_similarity(&self, distance: f32) -> f32 {
        match self {
            DistanceMetric::Cosine => (distance + 1.0) / 2.0, // [-1, 1] -> [0, 1]
            DistanceMetric::Euclidean => 1.0 / (1.0 + distance), // [0, inf) -> (0, 1]
            DistanceMetric::DotProduct => distance,           // Keep as-is (assumes normalized)
        }
    }
}

/// Configuration for a vector index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorIndexConfig {
    /// Vector dimension (REQUIRED)
    pub dim: usize,

    /// Field name containing the vector (for auto-indexing on insert/update)
    #[serde(default)]
    pub field: String,

    /// Maximum number of vectors in this index
    #[serde(default = "default_max_vectors")]
    pub max_vectors: usize,

    /// Distance metric for similarity calculation
    #[serde(default)]
    pub metric: DistanceMetric,

    /// HNSW M parameter: max connections per node per layer
    /// Higher M = better recall, more memory, slower insert
    /// Recommended: 12-48, default: 16
    #[serde(default = "default_m")]
    pub m: usize,

    /// HNSW ef_construction: size of dynamic candidate list during construction
    /// Higher = better recall, slower insert
    /// Recommended: 100-500, default: 200
    #[serde(default = "default_ef_construction")]
    pub ef_construction: usize,

    /// HNSW ef_search: size of dynamic candidate list during search
    /// Higher = better recall, slower search
    /// Recommended: 50-200, default: 50
    #[serde(default = "default_ef_search")]
    pub ef_search: usize,
}

fn default_max_vectors() -> usize {
    DEFAULT_MAX_VECTORS
}

fn default_m() -> usize {
    16
}

fn default_ef_construction() -> usize {
    200
}

fn default_ef_search() -> usize {
    50
}

impl Default for VectorIndexConfig {
    fn default() -> Self {
        Self {
            dim: 0, // Must be set explicitly
            field: String::new(),
            max_vectors: DEFAULT_MAX_VECTORS,
            metric: DistanceMetric::default(),
            m: 16,
            ef_construction: 200,
            ef_search: 50,
        }
    }
}

impl VectorIndexConfig {
    /// Create a new config with the given dimension
    pub fn new(dim: usize) -> Self {
        Self {
            dim,
            ..Default::default()
        }
    }

    /// Set the field name for auto-indexing
    pub fn with_field(mut self, field: impl Into<String>) -> Self {
        self.field = field.into();
        self
    }

    /// Set the maximum number of vectors
    pub fn with_max_vectors(mut self, max_vectors: usize) -> Self {
        self.max_vectors = max_vectors;
        self
    }

    /// Set the distance metric
    pub fn with_metric(mut self, metric: DistanceMetric) -> Self {
        self.metric = metric;
        self
    }

    /// Set HNSW M parameter
    pub fn with_m(mut self, m: usize) -> Self {
        self.m = m;
        self
    }

    /// Set HNSW ef_construction parameter
    pub fn with_ef_construction(mut self, ef_construction: usize) -> Self {
        self.ef_construction = ef_construction;
        self
    }

    /// Set HNSW ef_search parameter
    pub fn with_ef_search(mut self, ef_search: usize) -> Self {
        self.ef_search = ef_search;
        self
    }

    /// Validate the configuration
    pub fn validate(&self) -> Result<(), VectorConfigError> {
        if self.dim == 0 {
            return Err(VectorConfigError::DimensionRequired);
        }

        if self.dim > MAX_VECTOR_DIM {
            return Err(VectorConfigError::DimensionTooLarge {
                dim: self.dim,
                max: MAX_VECTOR_DIM,
            });
        }

        if self.m < 2 {
            return Err(VectorConfigError::InvalidM { m: self.m });
        }

        if self.ef_construction < self.m {
            return Err(VectorConfigError::EfConstructionTooSmall {
                ef_construction: self.ef_construction,
                m: self.m,
            });
        }

        if self.max_vectors > VECTOR_COUNT_WARNING_THRESHOLD {
            // Log warning but don't fail
            log_warn!(
                "Vector index configured for {} vectors, which may use significant memory",
                self.max_vectors
            );
        }

        Ok(())
    }

    /// Estimate memory usage in bytes for the given vector count
    pub fn estimate_memory_bytes(&self, vector_count: usize) -> usize {
        // Per vector: dim * 4 bytes (f32)
        // Per node: ~M * 2 * 8 bytes for neighbor lists (average)
        // Plus overhead for HashMap, etc.
        let vector_bytes = vector_count * self.dim * 4;
        let graph_bytes = vector_count * self.m * 2 * 8;
        let overhead = vector_count * 64; // ID strings, etc.

        vector_bytes + graph_bytes + overhead
    }
}

/// Errors that can occur during vector index configuration
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VectorConfigError {
    /// Dimension must be specified
    DimensionRequired,

    /// Dimension exceeds maximum
    DimensionTooLarge { dim: usize, max: usize },

    /// M parameter is invalid
    InvalidM { m: usize },

    /// ef_construction must be >= M
    EfConstructionTooSmall { ef_construction: usize, m: usize },
}

impl std::fmt::Display for VectorConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VectorConfigError::DimensionRequired => {
                write!(f, "Vector dimension must be specified (dim > 0)")
            }
            VectorConfigError::DimensionTooLarge { dim, max } => {
                write!(
                    f,
                    "Vector dimension {} exceeds maximum supported dimension {}",
                    dim, max
                )
            }
            VectorConfigError::InvalidM { m } => {
                write!(f, "HNSW M parameter must be at least 2, got {}", m)
            }
            VectorConfigError::EfConstructionTooSmall { ef_construction, m } => {
                write!(
                    f,
                    "ef_construction ({}) must be >= M ({})",
                    ef_construction, m
                )
            }
        }
    }
}

impl std::error::Error for VectorConfigError {}

/// Metadata for a stored vector index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorIndexMetadata {
    /// Index name (e.g., "embedding_vec")
    pub name: String,

    /// Field name in documents (e.g., "embedding")
    pub field: String,

    /// Configuration
    pub config: VectorIndexConfig,

    /// Cache file path relative to database
    pub cache_file: String,

    /// Number of vectors currently indexed
    pub vector_count: usize,

    /// Creation timestamp
    pub created_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config = VectorIndexConfig::new(300);
        assert_eq!(config.dim, 300);
        assert_eq!(config.max_vectors, DEFAULT_MAX_VECTORS);
        assert_eq!(config.metric, DistanceMetric::Cosine);
        assert_eq!(config.m, 16);
        assert_eq!(config.ef_construction, 200);
        assert_eq!(config.ef_search, 50);
    }

    #[test]
    fn test_config_builder() {
        let config = VectorIndexConfig::new(100)
            .with_max_vectors(50_000)
            .with_metric(DistanceMetric::Euclidean)
            .with_m(32);

        assert_eq!(config.dim, 100);
        assert_eq!(config.max_vectors, 50_000);
        assert_eq!(config.metric, DistanceMetric::Euclidean);
        assert_eq!(config.m, 32);
    }

    #[test]
    fn test_config_validation() {
        // Missing dimension
        let config = VectorIndexConfig::default();
        assert!(matches!(
            config.validate(),
            Err(VectorConfigError::DimensionRequired)
        ));

        // Dimension too large
        let config = VectorIndexConfig::new(5000);
        assert!(matches!(
            config.validate(),
            Err(VectorConfigError::DimensionTooLarge { .. })
        ));

        // Invalid M
        let mut config = VectorIndexConfig::new(100);
        config.m = 1;
        assert!(matches!(
            config.validate(),
            Err(VectorConfigError::InvalidM { .. })
        ));

        // ef_construction too small
        let mut config = VectorIndexConfig::new(100);
        config.m = 32;
        config.ef_construction = 16;
        assert!(matches!(
            config.validate(),
            Err(VectorConfigError::EfConstructionTooSmall { .. })
        ));

        // Valid config
        let config = VectorIndexConfig::new(300);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_distance_metric_similarity() {
        // Cosine: -1 -> 0, 0 -> 0.5, 1 -> 1
        assert!((DistanceMetric::Cosine.to_similarity(-1.0) - 0.0).abs() < 0.001);
        assert!((DistanceMetric::Cosine.to_similarity(0.0) - 0.5).abs() < 0.001);
        assert!((DistanceMetric::Cosine.to_similarity(1.0) - 1.0).abs() < 0.001);

        // Euclidean: 0 -> 1, 1 -> 0.5, large -> ~0
        assert!((DistanceMetric::Euclidean.to_similarity(0.0) - 1.0).abs() < 0.001);
        assert!((DistanceMetric::Euclidean.to_similarity(1.0) - 0.5).abs() < 0.001);
        assert!(DistanceMetric::Euclidean.to_similarity(100.0) < 0.01);
    }

    #[test]
    fn test_memory_estimation() {
        let config = VectorIndexConfig::new(300);

        // 100K vectors at 300 dim
        let mem = config.estimate_memory_bytes(100_000);

        // Should be roughly: 100K * 300 * 4 = 120MB vectors
        //                  + 100K * 16 * 2 * 8 = 25.6MB graph
        //                  + 100K * 64 = 6.4MB overhead
        //                  ≈ 152 MB
        assert!(mem > 100_000_000); // > 100 MB
        assert!(mem < 200_000_000); // < 200 MB
    }

    #[test]
    fn test_serde() {
        let config = VectorIndexConfig::new(100)
            .with_metric(DistanceMetric::DotProduct)
            .with_m(24);

        let json = serde_json::to_string(&config).unwrap();
        let parsed: VectorIndexConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.dim, 100);
        assert_eq!(parsed.metric, DistanceMetric::DotProduct);
        assert_eq!(parsed.m, 24);
    }
}
