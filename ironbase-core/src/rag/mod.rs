//! RAG (Retrieval Augmented Generation) module for IronBase
//!
//! Provides semantic search capabilities using:
//! - FastText embeddings (memory-mapped for efficiency)
//! - HNSW index for fast approximate nearest neighbor search
//! - Smart chunking with table preservation
//!
//! # Example
//!
//! ```rust,ignore
//! use ironbase_core::rag::{FastTextEngine, HnswIndex, Chunker};
//!
//! // Load FastText model (memory-mapped)
//! let fasttext = FastTextEngine::open("models/cc.hu.300.bin")?;
//!
//! // Create embedding
//! let embedding = fasttext.document_embedding("A mérési bizonytalanság számítása...");
//!
//! // Build HNSW index
//! let mut hnsw = HnswIndex::with_dim(300);
//! hnsw.insert("doc1:chunk1", &embedding)?;
//!
//! // Search in HNSW index
//! let results = hnsw.search(&query_embedding, 10);
//! ```

mod chunker;
mod fasttext;
mod hnsw;
mod manager;
mod types;

pub use chunker::{estimate_tokens, extract_tables, Chunker};
pub use fasttext::FastTextEngine;
pub use hnsw::HnswIndex;
pub use manager::{DocumentMeta, RagManager, RagStats, StoredChunk};
pub use types::*;
