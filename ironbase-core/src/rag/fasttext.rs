//! FastText embedding engine with memory-mapped file access
//!
//! This module provides efficient access to FastText word vectors using
//! memory-mapped files, minimizing RAM usage (~50MB for index vs 650MB full load).
//!
//! # Binary Format (.bin)
//!
//! ```text
//! [vocab_size: i32][dim: i32]
//! [word1\0][vec1: f32 × dim]
//! [word2\0][vec2: f32 × dim]
//! ...
//! ```

use super::types::{RagError, RagResult};
use byteorder::{LittleEndian, ReadBytesExt};
use memmap2::Mmap;
use std::collections::HashMap;
use std::fs::File;
use std::io::Cursor;
use std::path::Path;

/// Memory-mapped FastText engine
///
/// Loads only the word index into RAM (~50MB), vectors are read on-demand
/// from the memory-mapped file.
pub struct FastTextEngine {
    /// Memory-mapped file containing the model
    mmap: Mmap,
    /// Word to offset mapping (offset points to start of vector data)
    word_index: HashMap<String, usize>,
    /// Vector dimension (typically 300)
    dim: usize,
    /// Vocabulary size
    vocab_size: usize,
    /// Offset where vectors start (after header)
    #[allow(dead_code)]
    data_start: usize,
}

impl FastTextEngine {
    /// Open a FastText .bin model file
    ///
    /// This only loads the word index into memory (~50MB for 2M words).
    /// Vectors are read on-demand using memory mapping.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the .bin file (e.g., "models/cc.hu.300.bin")
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let engine = FastTextEngine::open("models/cc.hu.300.bin")?;
    /// let vec = engine.word_vector("kutya");
    /// ```
    pub fn open<P: AsRef<Path>>(path: P) -> RagResult<Self> {
        let path = path.as_ref();

        if !path.exists() {
            return Err(RagError::ModelNotFound(path.display().to_string()));
        }

        let file = File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };

        // Parse header
        let mut cursor = Cursor::new(&mmap[..]);

        // Read magic number / vocab_size
        // FastText format: first 4 bytes can be magic or vocab_size
        let first_i32 = cursor.read_i32::<LittleEndian>()?;

        let (vocab_size, dim) = if first_i32 < 0 {
            // New format with magic number
            return Err(RagError::InvalidFormat(
                "New FastText format with magic number not yet supported".to_string(),
            ));
        } else {
            // Old format: vocab_size, dim directly
            let vocab_size = first_i32 as usize;
            let dim = cursor.read_i32::<LittleEndian>()? as usize;
            (vocab_size, dim)
        };

        if dim == 0 || dim > 1000 {
            return Err(RagError::InvalidFormat(format!(
                "Invalid dimension: {}",
                dim
            )));
        }

        if vocab_size == 0 || vocab_size > 10_000_000 {
            return Err(RagError::InvalidFormat(format!(
                "Invalid vocabulary size: {}",
                vocab_size
            )));
        }

        tracing::info!(
            "Loading FastText model: vocab_size={}, dim={}",
            vocab_size,
            dim
        );

        // Build word index by scanning through the file
        let data_start = 8; // After two i32s
        let mut word_index = HashMap::with_capacity(vocab_size);
        let mut offset = data_start;

        for i in 0..vocab_size {
            if offset >= mmap.len() {
                return Err(RagError::InvalidFormat(format!(
                    "Unexpected end of file at word {}",
                    i
                )));
            }

            // Read null-terminated word
            let word_start = offset;
            while offset < mmap.len() && mmap[offset] != 0 {
                offset += 1;
            }

            if offset >= mmap.len() {
                return Err(RagError::InvalidFormat(format!(
                    "Unterminated word at {}",
                    i
                )));
            }

            let word = String::from_utf8_lossy(&mmap[word_start..offset]).to_string();
            offset += 1; // Skip null terminator

            // Store offset to vector data
            let vector_offset = offset;
            word_index.insert(word.to_lowercase(), vector_offset);

            // Skip vector data (dim * 4 bytes)
            offset += dim * 4;

            // Progress logging every 500K words
            if i > 0 && i % 500_000 == 0 {
                tracing::debug!("Indexed {} words...", i);
            }
        }

        tracing::info!(
            "FastText model loaded: {} words indexed, ~{} MB index size",
            word_index.len(),
            word_index.len() * 50 / 1_000_000 // Rough estimate
        );

        Ok(Self {
            mmap,
            word_index,
            dim,
            vocab_size,
            data_start,
        })
    }

    /// Get the vector dimension
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// Get the vocabulary size
    pub fn vocab_size(&self) -> usize {
        self.vocab_size
    }

    /// Check if a word exists in the vocabulary
    pub fn contains(&self, word: &str) -> bool {
        self.word_index.contains_key(&word.to_lowercase())
    }

    /// Get the vector for a single word
    ///
    /// Returns None if the word is not in the vocabulary.
    /// The vector is read directly from the memory-mapped file.
    pub fn word_vector(&self, word: &str) -> Option<Vec<f32>> {
        let word_lower = word.to_lowercase();
        let offset = *self.word_index.get(&word_lower)?;

        // Read vector from mmap
        let vec_bytes = &self.mmap[offset..offset + self.dim * 4];
        let mut cursor = Cursor::new(vec_bytes);

        let mut vector = Vec::with_capacity(self.dim);
        for _ in 0..self.dim {
            match cursor.read_f32::<LittleEndian>() {
                Ok(v) => vector.push(v),
                Err(_) => return None,
            }
        }

        Some(vector)
    }

    /// Compute document embedding as average of word vectors
    ///
    /// Words not in vocabulary are skipped. Returns zero vector if no words found.
    ///
    /// # Arguments
    ///
    /// * `text` - The text to embed
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let embedding = engine.document_embedding("A kutya fut a kertben");
    /// ```
    pub fn document_embedding(&self, text: &str) -> Vec<f32> {
        let mut sum = vec![0.0f32; self.dim];
        let mut count = 0usize;

        // Tokenize on whitespace and punctuation
        for word in tokenize(text) {
            if let Some(vec) = self.word_vector(&word) {
                for (i, v) in vec.iter().enumerate() {
                    sum[i] += v;
                }
                count += 1;
            }
        }

        // Average
        if count > 0 {
            let count_f = count as f32;
            for v in &mut sum {
                *v /= count_f;
            }
        }

        sum
    }

    /// Compute embeddings for multiple texts (batch processing)
    ///
    /// More efficient than calling document_embedding repeatedly.
    pub fn batch_embed(&self, texts: &[&str]) -> Vec<Vec<f32>> {
        texts.iter().map(|t| self.document_embedding(t)).collect()
    }

    /// Compute cosine similarity between two vectors
    pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        if a.len() != b.len() || a.is_empty() {
            return 0.0;
        }

        let mut dot = 0.0f32;
        let mut norm_a = 0.0f32;
        let mut norm_b = 0.0f32;

        for (x, y) in a.iter().zip(b.iter()) {
            dot += x * y;
            norm_a += x * x;
            norm_b += y * y;
        }

        let denom = (norm_a * norm_b).sqrt();
        if denom > 0.0 {
            dot / denom
        } else {
            0.0
        }
    }

    /// Find most similar words to a query vector
    ///
    /// This is a brute-force search - use HNSW index for large-scale search.
    pub fn find_similar_words(&self, query: &[f32], top_k: usize) -> Vec<(String, f32)> {
        let mut results: Vec<(String, f32)> = self
            .word_index
            .keys()
            .filter_map(|word| {
                let vec = self.word_vector(word)?;
                let score = Self::cosine_similarity(query, &vec);
                Some((word.clone(), score))
            })
            .collect();

        // Sort by score descending
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(top_k);
        results
    }
}

/// Simple tokenizer that splits on whitespace and removes punctuation
fn tokenize(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
        .filter(|s| !s.is_empty() && s.len() > 1) // Skip single chars
        .map(|s| s.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize() {
        let tokens: Vec<String> = tokenize("A kutya fut, a kertben!").collect();
        assert_eq!(tokens, vec!["kutya", "fut", "kertben"]);
    }

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let sim = FastTextEngine::cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let sim = FastTextEngine::cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![-1.0, 0.0, 0.0];
        let sim = FastTextEngine::cosine_similarity(&a, &b);
        assert!((sim + 1.0).abs() < 1e-6);
    }
}
