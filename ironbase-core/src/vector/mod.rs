//! Vector index module for approximate nearest neighbor (ANN) search
//!
//! This module provides vector indexing capabilities using HNSW
//! (Hierarchical Navigable Small World) algorithm for fast similarity search.
//!
//! # Features
//!
//! - HNSW index with configurable M and ef parameters
//! - Support for Cosine, Euclidean, and Dot Product distance metrics
//! - Automatic index persistence as cache files (.hnsw)
//! - Auto-rebuild from documents if cache is missing/stale
//! - Filter support for hybrid search (vector + attribute filter)
//!
//! # Example
//!
//! ```rust,ignore
//! use ironbase_core::vector::{VectorIndexConfig, DistanceMetric};
//!
//! // Create vector index config
//! let config = VectorIndexConfig::new(300)
//!     .with_metric(DistanceMetric::Cosine)
//!     .with_max_vectors(100_000);
//!
//! // Create index on collection
//! collection.create_vector_index("embedding", config)?;
//!
//! // Search
//! let results = collection.vector_search("embedding", &query_vec, 10)?;
//! ```

mod config;
mod hnsw;
pub mod simd;

pub use config::{
    DistanceMetric, VectorConfigError, VectorIndexConfig, VectorIndexMetadata, DEFAULT_MAX_VECTORS,
    MAX_VECTOR_DIM, VECTOR_COUNT_WARNING_THRESHOLD,
};

pub use hnsw::{HnswIndex, VectorSearchResult};

pub use simd::{
    cosine_similarity, cosine_similarity_with_norm, dot_product, euclidean_distance, l2_norm,
    normalize, normalize_in_place, squared_euclidean_distance, squared_l2_norm,
};
