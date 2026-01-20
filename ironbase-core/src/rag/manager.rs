//! RAG Manager - High-level API for semantic document search
//!
//! Coordinates FastText embeddings, HNSW index, and document chunking
//! to provide a complete RAG solution.
//!
//! # Example
//!
//! ```rust,ignore
//! // Create RAG collection
//! let mut rag = RagManager::new("iso_handbook", &fasttext)?;
//!
//! // Import document
//! let result = rag.import_document("doc1", "ISO 17025 Handbook", markdown_content)?;
//! println!("Created {} chunks", result.chunks_created);
//!
//! // Search
//! let results = rag.search("measurement uncertainty", 5)?;
//! for result in results {
//!     println!("[{}] {} (score: {:.3})", result.doc_title, result.section, result.score);
//! }
//!
//! // Save to disk
//! rag.save("./rag_data/iso_handbook")?;
//!
//! // Load from disk
//! let rag = RagManager::load("./rag_data/iso_handbook", &fasttext)?;
//! ```

use super::chunker::Chunker;
use super::fasttext::FastTextEngine;
use super::hnsw::HnswIndex;
use super::types::{
    Chunk, ChunkConfig, HnswConfig, ImportResult, RagError, RagLimits, RagResult, SearchResult,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::Path;
use std::time::Instant;

/// Stored document metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentMeta {
    /// Document ID
    pub id: String,
    /// Document title
    pub title: String,
    /// Number of chunks
    pub chunk_count: usize,
    /// Import timestamp (Unix millis)
    pub imported_at: u64,
}

/// Stored chunk with its embedding reference
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredChunk {
    /// Chunk ID (format: "doc_id:chunk_index")
    pub id: String,
    /// Parent document ID
    pub doc_id: String,
    /// Chunk data
    pub chunk: Chunk,
}

/// RAG collection state (for persistence - deserialize)
#[derive(Debug, Deserialize)]
struct RagState {
    /// Collection name
    name: String,
    /// Document metadata
    documents: HashMap<String, DocumentMeta>,
    /// All stored chunks
    chunks: HashMap<String, StoredChunk>,
    /// Chunk configuration
    chunk_config: ChunkConfig,
    /// HNSW configuration
    hnsw_config: HnswConfig,
}

/// RAG collection state reference (for save - no clone needed)
#[derive(Serialize)]
struct RagStateRef<'a> {
    name: &'a str,
    documents: &'a HashMap<String, DocumentMeta>,
    chunks: &'a HashMap<String, StoredChunk>,
    chunk_config: &'a ChunkConfig,
    hnsw_config: &'a HnswConfig,
}

/// High-level RAG manager
pub struct RagManager<'a> {
    /// Collection name
    name: String,
    /// FastText embedding engine (borrowed)
    fasttext: &'a FastTextEngine,
    /// HNSW index for vector search
    hnsw: HnswIndex,
    /// Document metadata
    documents: HashMap<String, DocumentMeta>,
    /// Stored chunks
    chunks: HashMap<String, StoredChunk>,
    /// Chunker for document processing
    chunker: Chunker,
    /// Configuration
    chunk_config: ChunkConfig,
    hnsw_config: HnswConfig,
    /// Memory limits (dynamic, RAM-based)
    limits: RagLimits,
}

impl<'a> RagManager<'a> {
    /// Create a new RAG collection with automatic RAM-based limits
    pub fn new(name: &str, fasttext: &'a FastTextEngine) -> RagResult<Self> {
        Self::with_limits(name, fasttext, RagLimits::from_system_memory())
    }

    /// Create a new RAG collection with custom limits
    pub fn with_limits(
        name: &str,
        fasttext: &'a FastTextEngine,
        limits: RagLimits,
    ) -> RagResult<Self> {
        let mut chunk_config = ChunkConfig::default();
        // Sync limits from RagLimits
        chunk_config.max_document_bytes = limits.max_document_bytes;
        chunk_config.max_chunks_per_document = limits.max_chunks_per_document;

        let hnsw_config = HnswConfig {
            dim: fasttext.dim(),
            ..Default::default()
        };

        Ok(Self {
            name: name.to_string(),
            fasttext,
            hnsw: HnswIndex::with_limits(hnsw_config.clone(), limits.max_hnsw_vectors),
            documents: HashMap::new(),
            chunks: HashMap::new(),
            chunker: Chunker::new(chunk_config.clone()),
            chunk_config,
            hnsw_config,
            limits,
        })
    }

    /// Create with custom configuration
    pub fn with_config(
        name: &str,
        fasttext: &'a FastTextEngine,
        chunk_config: ChunkConfig,
        hnsw_config: HnswConfig,
    ) -> RagResult<Self> {
        Self::with_config_and_limits(
            name,
            fasttext,
            chunk_config,
            hnsw_config,
            RagLimits::from_system_memory(),
        )
    }

    /// Create with custom configuration and limits
    pub fn with_config_and_limits(
        name: &str,
        fasttext: &'a FastTextEngine,
        mut chunk_config: ChunkConfig,
        hnsw_config: HnswConfig,
        limits: RagLimits,
    ) -> RagResult<Self> {
        // Sync limits from RagLimits
        chunk_config.max_document_bytes = limits.max_document_bytes;
        chunk_config.max_chunks_per_document = limits.max_chunks_per_document;

        Ok(Self {
            name: name.to_string(),
            fasttext,
            hnsw: HnswIndex::with_limits(hnsw_config.clone(), limits.max_hnsw_vectors),
            documents: HashMap::new(),
            chunks: HashMap::new(),
            chunker: Chunker::new(chunk_config.clone()),
            chunk_config,
            hnsw_config,
            limits,
        })
    }

    /// Get collection name
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get number of documents
    pub fn document_count(&self) -> usize {
        self.documents.len()
    }

    /// Get number of chunks
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// Get current limits
    pub fn limits(&self) -> &RagLimits {
        &self.limits
    }

    /// Get document by ID
    pub fn get_document(&self, doc_id: &str) -> Option<&DocumentMeta> {
        self.documents.get(doc_id)
    }

    /// List all documents
    pub fn list_documents(&self) -> Vec<&DocumentMeta> {
        self.documents.values().collect()
    }

    /// Import a markdown document
    ///
    /// # Arguments
    ///
    /// * `doc_id` - Unique document identifier
    /// * `title` - Document title
    /// * `content` - Markdown content
    ///
    /// # Returns
    ///
    /// Import result with statistics
    pub fn import_document(
        &mut self,
        doc_id: &str,
        title: &str,
        content: &str,
    ) -> RagResult<ImportResult> {
        let start = Instant::now();

        // Check for duplicate
        if self.documents.contains_key(doc_id) {
            return Err(RagError::ChunkError(format!(
                "Document already exists: {}",
                doc_id
            )));
        }

        // Chunk the document (with OOM protection)
        let raw_chunks = self.chunker.chunk_markdown(content)?;

        // OOM Protection: Check total chunks limit (dynamic, RAM-based)
        // Note: Per-document chunk limit is already enforced by Chunker
        let new_total = self.chunks.len() + raw_chunks.len();
        if new_total > self.limits.max_total_chunks {
            return Err(RagError::ChunkError(format!(
                "Collection would exceed max chunks: {} + {} = {} (max: {}). Remove documents or create a new collection.",
                self.chunks.len(),
                raw_chunks.len(),
                new_total,
                self.limits.max_total_chunks
            )));
        }

        // Extract tables for counting
        let tables_extracted = raw_chunks
            .iter()
            .filter(|c| c.block_type == super::types::BlockType::Table)
            .count();

        // Process chunks: embed and add to HNSW
        let mut chunk_count = 0;
        for (idx, chunk) in raw_chunks.into_iter().enumerate() {
            let chunk_id = format!("{}:{}", doc_id, idx);

            // Generate embedding
            let embedding = self.fasttext.document_embedding(&chunk.text);

            // Skip if no valid embedding (all zeros)
            if embedding.iter().all(|&v| v == 0.0) {
                tracing::debug!(
                    "Skipping chunk {} - no valid embedding (no known words)",
                    chunk_id
                );
                continue;
            }

            // Add to HNSW index
            self.hnsw.insert(&chunk_id, &embedding)?;

            // Store chunk
            self.chunks.insert(
                chunk_id.clone(),
                StoredChunk {
                    id: chunk_id,
                    doc_id: doc_id.to_string(),
                    chunk,
                },
            );

            chunk_count += 1;
        }

        // Store document metadata
        self.documents.insert(
            doc_id.to_string(),
            DocumentMeta {
                id: doc_id.to_string(),
                title: title.to_string(),
                chunk_count,
                imported_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
            },
        );

        let elapsed = start.elapsed();

        Ok(ImportResult {
            doc_id: doc_id.to_string(),
            chunks_created: chunk_count,
            tables_extracted,
            import_time_ms: elapsed.as_millis() as u64,
        })
    }

    /// Remove a document and its chunks
    pub fn remove_document(&mut self, doc_id: &str) -> RagResult<bool> {
        if !self.documents.contains_key(doc_id) {
            return Ok(false);
        }

        // Remove chunks
        let chunk_ids: Vec<String> = self
            .chunks
            .values()
            .filter(|c| c.doc_id == doc_id)
            .map(|c| c.id.clone())
            .collect();

        for chunk_id in chunk_ids {
            self.chunks.remove(&chunk_id);
            // Note: HNSW doesn't support efficient removal
            // A rebuild would be needed for full removal
        }

        // Remove document
        self.documents.remove(doc_id);

        Ok(true)
    }

    /// Upsert a single chunk
    ///
    /// If the chunk_id exists, it will be replaced with the new text.
    /// If it doesn't exist, it will be created.
    ///
    /// # Arguments
    ///
    /// * `doc_id` - Document ID this chunk belongs to
    /// * `chunk_id` - Chunk ID (e.g., "7:13")
    /// * `text` - New text content for the chunk
    /// * `section` - Optional section path
    ///
    /// # Returns
    ///
    /// UpsertResult with info about what happened
    pub fn upsert_chunk(
        &mut self,
        doc_id: &str,
        chunk_id: &str,
        text: &str,
        section: Option<Vec<String>>,
    ) -> RagResult<super::types::UpsertResult> {
        // Check if document exists
        if !self.documents.contains_key(doc_id) {
            return Err(RagError::ChunkError(format!(
                "Document '{}' not found. Create document first with import_document.",
                doc_id
            )));
        }

        // Check if this is an update or insert
        let is_update = self.chunks.contains_key(chunk_id);

        // Generate new embedding
        let embedding = self.fasttext.document_embedding(text);

        // Check for valid embedding
        if embedding.iter().all(|&v| v == 0.0) {
            return Err(RagError::ChunkError(
                "Chunk text has no valid embedding - no known words in model".to_string(),
            ));
        }

        // Upsert into HNSW (updates vector if exists, inserts if not)
        self.hnsw.upsert(chunk_id, &embedding)?;

        // Create chunk struct
        let chunk = super::types::Chunk {
            text: text.to_string(),
            char_range: (0, text.len()),
            token_count: text.split_whitespace().count(),
            block_type: super::types::BlockType::Paragraph,
            section_path: section.unwrap_or_default(),
            parent_heading: None,
        };

        // Store chunk
        self.chunks.insert(
            chunk_id.to_string(),
            StoredChunk {
                id: chunk_id.to_string(),
                doc_id: doc_id.to_string(),
                chunk,
            },
        );

        // Update document chunk count if this is a new chunk
        if !is_update {
            if let Some(doc) = self.documents.get_mut(doc_id) {
                doc.chunk_count += 1;
            }
        }

        Ok(super::types::UpsertResult {
            chunk_id: chunk_id.to_string(),
            is_update,
        })
    }

    /// Search for relevant chunks
    ///
    /// # Arguments
    ///
    /// * `query` - Search query text
    /// * `top_k` - Number of results to return
    ///
    /// # Returns
    ///
    /// Vector of search results sorted by relevance
    pub fn search(&self, query: &str, top_k: usize) -> RagResult<Vec<SearchResult>> {
        // Generate query embedding
        let query_embedding = self.fasttext.document_embedding(query);

        // Check for valid embedding
        if query_embedding.iter().all(|&v| v == 0.0) {
            tracing::warn!("Query has no valid embedding - no known words: {}", query);
            return Ok(Vec::new());
        }

        // Search HNSW
        let hnsw_results = self.hnsw.search(&query_embedding, top_k);

        // Convert to SearchResults
        let mut results = Vec::with_capacity(hnsw_results.len());
        for (chunk_id, score) in hnsw_results {
            if let Some(stored_chunk) = self.chunks.get(&chunk_id) {
                let doc = self.documents.get(&stored_chunk.doc_id);
                let doc_title = doc.map(|d| d.title.as_str()).unwrap_or("Unknown");

                results.push(SearchResult {
                    doc_id: stored_chunk.doc_id.clone(),
                    doc_title: doc_title.to_string(),
                    chunk_id: chunk_id.clone(),
                    section: stored_chunk.chunk.section_path.join(" > "),
                    text: stored_chunk.chunk.text.clone(),
                    score,
                    block_type: stored_chunk.chunk.block_type,
                });
            }
        }

        Ok(results)
    }

    /// Search with minimum score threshold
    pub fn search_with_threshold(
        &self,
        query: &str,
        top_k: usize,
        min_score: f32,
    ) -> RagResult<Vec<SearchResult>> {
        let results = self.search(query, top_k)?;
        Ok(results
            .into_iter()
            .filter(|r| r.score >= min_score)
            .collect())
    }

    /// Get a specific chunk by ID
    pub fn get_chunk(&self, chunk_id: &str) -> Option<&StoredChunk> {
        self.chunks.get(chunk_id)
    }

    /// Get all chunks for a document
    pub fn get_document_chunks(&self, doc_id: &str) -> Vec<&StoredChunk> {
        self.chunks
            .values()
            .filter(|c| c.doc_id == doc_id)
            .collect()
    }

    /// Save the RAG collection to disk
    ///
    /// Creates two files:
    /// - `{path}.state.json` - Document and chunk metadata
    /// - `{path}.hnsw.bin` - HNSW index binary
    pub fn save<P: AsRef<Path>>(&self, path: P) -> RagResult<()> {
        let path = path.as_ref();

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Save state (documents, chunks, config) - using references to avoid cloning
        let state = RagStateRef {
            name: &self.name,
            documents: &self.documents,
            chunks: &self.chunks,
            chunk_config: &self.chunk_config,
            hnsw_config: &self.hnsw_config,
        };

        let state_path = format!("{}.state.json", path.display());
        let file = File::create(&state_path)?;
        let writer = BufWriter::new(file);
        serde_json::to_writer_pretty(writer, &state)
            .map_err(|e| RagError::Io(std::io::Error::other(e.to_string())))?;

        // Save HNSW index
        let hnsw_path = format!("{}.hnsw.bin", path.display());
        let file = File::create(&hnsw_path)?;
        let writer = BufWriter::new(file);
        bincode::serialize_into(writer, &self.hnsw)
            .map_err(|e| RagError::Io(std::io::Error::other(e.to_string())))?;

        tracing::info!(
            "RAG collection '{}' saved: {} documents, {} chunks",
            self.name,
            self.documents.len(),
            self.chunks.len()
        );

        Ok(())
    }

    /// Load a RAG collection from disk with automatic RAM-based limits
    pub fn load<P: AsRef<Path>>(path: P, fasttext: &'a FastTextEngine) -> RagResult<Self> {
        Self::load_with_limits(path, fasttext, RagLimits::from_system_memory())
    }

    /// Load a RAG collection from disk with custom limits
    pub fn load_with_limits<P: AsRef<Path>>(
        path: P,
        fasttext: &'a FastTextEngine,
        limits: RagLimits,
    ) -> RagResult<Self> {
        let path = path.as_ref();

        // Load state
        let state_path = format!("{}.state.json", path.display());
        let file = File::open(&state_path)?;
        let reader = BufReader::new(file);
        let state: RagState = serde_json::from_reader(reader)
            .map_err(|e| RagError::Io(std::io::Error::other(e.to_string())))?;

        // Load HNSW index
        let hnsw_path = format!("{}.hnsw.bin", path.display());
        let file = File::open(&hnsw_path)?;
        let reader = BufReader::new(file);
        let hnsw: HnswIndex = bincode::deserialize_from(reader)
            .map_err(|e| RagError::Io(std::io::Error::other(e.to_string())))?;

        // Verify dimension matches
        if hnsw.dim() != fasttext.dim() {
            return Err(RagError::HnswError(format!(
                "HNSW dimension mismatch: index has {}, FastText has {}",
                hnsw.dim(),
                fasttext.dim()
            )));
        }

        tracing::info!(
            "RAG collection '{}' loaded: {} documents, {} chunks",
            state.name,
            state.documents.len(),
            state.chunks.len()
        );

        // Sync chunk config limits from RagLimits
        let mut chunk_config = state.chunk_config;
        chunk_config.max_document_bytes = limits.max_document_bytes;
        chunk_config.max_chunks_per_document = limits.max_chunks_per_document;

        Ok(Self {
            name: state.name,
            fasttext,
            hnsw,
            documents: state.documents,
            chunks: state.chunks,
            chunker: Chunker::new(chunk_config.clone()),
            chunk_config,
            hnsw_config: state.hnsw_config,
            limits,
        })
    }

    /// Check if a save file exists
    pub fn exists<P: AsRef<Path>>(path: P) -> bool {
        let path = path.as_ref();
        let state_path = format!("{}.state.json", path.display());
        let hnsw_path = format!("{}.hnsw.bin", path.display());
        Path::new(&state_path).exists() && Path::new(&hnsw_path).exists()
    }

    /// Rebuild the HNSW index (useful after many document removals)
    pub fn rebuild_index(&mut self) -> RagResult<()> {
        // Create new HNSW index
        let mut new_hnsw = HnswIndex::new(self.hnsw_config.clone());

        // Re-embed and insert all chunks
        for (chunk_id, stored_chunk) in &self.chunks {
            let embedding = self.fasttext.document_embedding(&stored_chunk.chunk.text);
            if !embedding.iter().all(|&v| v == 0.0) {
                new_hnsw.insert(chunk_id, &embedding)?;
            }
        }

        self.hnsw = new_hnsw;
        Ok(())
    }

    /// Get statistics about the collection
    pub fn stats(&self) -> RagStats {
        let total_tokens: usize = self.chunks.values().map(|c| c.chunk.token_count).sum();
        let avg_chunk_tokens = if self.chunks.is_empty() {
            0.0
        } else {
            total_tokens as f64 / self.chunks.len() as f64
        };

        RagStats {
            name: self.name.clone(),
            document_count: self.documents.len(),
            chunk_count: self.chunks.len(),
            hnsw_vectors: self.hnsw.len(),
            total_tokens,
            avg_chunk_tokens,
            vector_dim: self.hnsw.dim(),
        }
    }
}

/// RAG collection statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagStats {
    /// Collection name
    pub name: String,
    /// Number of documents
    pub document_count: usize,
    /// Number of chunks
    pub chunk_count: usize,
    /// Number of vectors in HNSW
    pub hnsw_vectors: usize,
    /// Total tokens across all chunks
    pub total_tokens: usize,
    /// Average tokens per chunk
    pub avg_chunk_tokens: f64,
    /// Vector dimension
    pub vector_dim: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: These tests require a FastText model to run
    // In real usage, we'd mock the FastText engine

    #[test]
    fn test_rag_stats_default() {
        let stats = RagStats {
            name: "test".to_string(),
            document_count: 0,
            chunk_count: 0,
            hnsw_vectors: 0,
            total_tokens: 0,
            avg_chunk_tokens: 0.0,
            vector_dim: 300,
        };

        assert_eq!(stats.document_count, 0);
        assert_eq!(stats.vector_dim, 300);
    }
}
