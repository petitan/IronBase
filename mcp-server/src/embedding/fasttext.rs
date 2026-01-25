//! FastText embedding provider using memory-mapped files
//!
//! Loads FastText models in IronBase binary format without loading
//! the entire model into RAM. Uses memmap for efficient lazy loading.
//!
//! IronBase binary format:
//! ```text
//! [vocab_size: i32][dim: i32]
//! [word1\0][vec1: f32 × dim]
//! [word2\0][vec2: f32 × dim]
//! ...
//! ```

use super::{EmbeddingError, EmbeddingProvider, EmbeddingResult};
use memmap2::Mmap;
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

/// Maximum vocabulary size (OOM protection for corrupted files)
const MAX_VOCAB_SIZE: usize = 10_000_000;

/// Maximum vector dimension (OOM protection)
const MAX_DIMENSION: usize = 4096;

/// Memory-mapped FastText provider
///
/// Only loads the word index into RAM (~50MB for 2M words).
/// Vectors are loaded on-demand from the memory-mapped file.
#[allow(dead_code)]
pub struct FastTextProvider {
    /// Memory-mapped file
    mmap: Mmap,
    /// Word to offset mapping (offset points to the vector data)
    word_index: HashMap<String, usize>,
    /// Vector dimension
    dim: usize,
    /// Vocabulary size
    vocab_size: usize,
    /// Model name (derived from filename)
    model_name: String,
}

impl FastTextProvider {
    /// Load a FastText model from IronBase binary format
    pub fn load(path: &Path) -> EmbeddingResult<Self> {
        let file = File::open(path).map_err(|e| {
            EmbeddingError::ModelLoadError(format!("Failed to open model file: {}", e))
        })?;

        // Memory-map the file
        let mmap = unsafe {
            Mmap::map(&file).map_err(|e| {
                EmbeddingError::ModelLoadError(format!("Failed to mmap model file: {}", e))
            })?
        };

        if mmap.len() < 8 {
            return Err(EmbeddingError::ModelLoadError(
                "Model file too small".to_string(),
            ));
        }

        // Read header: vocab_size (i32) + dim (i32)
        let vocab_size = i32::from_le_bytes([mmap[0], mmap[1], mmap[2], mmap[3]]) as usize;
        let dim = i32::from_le_bytes([mmap[4], mmap[5], mmap[6], mmap[7]]) as usize;

        // OOM protection: validate header values
        if vocab_size > MAX_VOCAB_SIZE {
            return Err(EmbeddingError::ModelLoadError(format!(
                "vocab_size {} exceeds maximum {} (corrupted file?)",
                vocab_size, MAX_VOCAB_SIZE
            )));
        }
        if dim == 0 || dim > MAX_DIMENSION {
            return Err(EmbeddingError::ModelLoadError(format!(
                "invalid dimension {} (must be 1-{})",
                dim, MAX_DIMENSION
            )));
        }

        log::info!(
            "Loading FastText model: vocab_size={}, dim={}",
            vocab_size,
            dim
        );

        // Build word index (OOM protection: try_reserve)
        let mut word_index = HashMap::new();
        word_index.try_reserve(vocab_size).map_err(|_| {
            EmbeddingError::ModelLoadError(format!(
                "failed to allocate word index for {} words",
                vocab_size
            ))
        })?;
        let mut offset = 8usize; // Start after header

        for i in 0..vocab_size {
            if offset >= mmap.len() {
                log::warn!("Unexpected end of file at word {}", i);
                break;
            }

            // Read null-terminated word
            let word_start = offset;
            while offset < mmap.len() && mmap[offset] != 0 {
                offset += 1;
            }

            if offset >= mmap.len() {
                break;
            }

            let word = String::from_utf8_lossy(&mmap[word_start..offset]).to_string();
            offset += 1; // Skip null terminator

            // The vector starts here
            let vec_offset = offset;
            word_index.insert(word, vec_offset);

            // Skip the vector data (dim * 4 bytes for f32)
            offset += dim * 4;

            if i > 0 && i % 500_000 == 0 {
                log::info!("  Indexed {} words...", i);
            }
        }

        let actual_vocab_size = word_index.len();
        log::info!("FastText model loaded: {} words indexed", actual_vocab_size);

        // Extract model name from path
        let model_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        Ok(Self {
            mmap,
            word_index,
            dim,
            vocab_size: actual_vocab_size,
            model_name,
        })
    }

    /// Get vector for a single word
    fn word_vector(&self, word: &str) -> Option<Vec<f32>> {
        let offset = *self.word_index.get(word)?;
        let end = offset + self.dim * 4;

        if end > self.mmap.len() {
            return None;
        }

        // Read f32 values from mmap
        let mut vec = Vec::with_capacity(self.dim);
        for i in 0..self.dim {
            let start = offset + i * 4;
            let bytes = [
                self.mmap[start],
                self.mmap[start + 1],
                self.mmap[start + 2],
                self.mmap[start + 3],
            ];
            vec.push(f32::from_le_bytes(bytes));
        }

        Some(vec)
    }

    /// Simple tokenizer: lowercase + split on whitespace and punctuation
    fn tokenize(text: &str) -> Vec<String> {
        text.to_lowercase()
            .split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
            .filter(|s| !s.is_empty() && s.len() > 1)
            .map(|s| s.to_string())
            .collect()
    }

    /// Normalize a vector to unit length
    fn normalize(vec: &mut [f32]) {
        let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in vec.iter_mut() {
                *x /= norm;
            }
        }
    }
}

impl EmbeddingProvider for FastTextProvider {
    fn embed(&self, text: &str) -> EmbeddingResult<Vec<f32>> {
        let tokens = Self::tokenize(text);

        if tokens.is_empty() {
            // Return zero vector for empty input
            return Ok(vec![0.0; self.dim]);
        }

        // Average word vectors
        let mut result = vec![0.0f32; self.dim];
        let mut count = 0usize;

        for token in &tokens {
            if let Some(vec) = self.word_vector(token) {
                for (i, v) in vec.iter().enumerate() {
                    result[i] += v;
                }
                count += 1;
            }
        }

        if count > 0 {
            let scale = 1.0 / count as f32;
            for x in result.iter_mut() {
                *x *= scale;
            }
            // Normalize to unit length
            Self::normalize(&mut result);
        }

        Ok(result)
    }

    fn embed_batch(&self, texts: &[&str]) -> EmbeddingResult<Vec<Vec<f32>>> {
        // Process in parallel for large batches
        if texts.len() > 10 {
            use rayon::prelude::*;
            texts.par_iter().map(|text| self.embed(text)).collect()
        } else {
            texts.iter().map(|text| self.embed(text)).collect()
        }
    }

    fn dimension(&self) -> usize {
        self.dim
    }

    fn model_name(&self) -> &str {
        &self.model_name
    }

    fn provider_name(&self) -> &str {
        "fasttext"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize() {
        let tokens = FastTextProvider::tokenize("Hello, World! This is a test.");
        assert!(tokens.contains(&"hello".to_string()));
        assert!(tokens.contains(&"world".to_string()));
        assert!(tokens.contains(&"this".to_string()));
        assert!(tokens.contains(&"test".to_string()));
        // Single char 'a' should be filtered out
        assert!(!tokens.contains(&"a".to_string()));
    }

    #[test]
    fn test_normalize() {
        let mut vec = vec![3.0, 4.0];
        FastTextProvider::normalize(&mut vec);
        let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
    }
}
