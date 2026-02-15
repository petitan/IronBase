//! FastText embedding provider using memory-mapped files
//!
//! Loads FastText models in IronBase binary format without loading
//! the entire model into RAM. Uses memmap for efficient lazy loading.
//!
//! Supports two formats:
//!
//! **v1 (legacy):**
//! ```text
//! [vocab_size: i32][dim: i32]
//! [word1\0][vec1: f32 × dim]
//! ...
//! ```
//!
//! **v2 (with subword support):**
//! ```text
//! Header (32 bytes):
//!   magic: b"IBv2", dim: u32, vocab_size: u32, bucket_count: u32,
//!   minn: u32, maxn: u32, bucket_offset: u64
//! Word Section: [word\0][f32 × dim] × vocab_size
//! Bucket Section: [f32 × dim] × bucket_count
//! ```

use super::{EmbeddingError, EmbeddingProvider, EmbeddingResult};
use ironbase_core::fulltext::{tokenize as fulltext_tokenize, FtsLanguage, FtsOptions};
use memmap2::Mmap;
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

/// Maximum vocabulary size (OOM protection for corrupted files)
const MAX_VOCAB_SIZE: usize = 10_000_000;

/// Maximum vector dimension (OOM protection)
const MAX_DIMENSION: usize = 4096;

/// Maximum bucket count (OOM protection)
const MAX_BUCKET_COUNT: usize = 10_000_000;

/// v2 magic bytes
const V2_MAGIC: &[u8; 4] = b"IBv2";

/// v2 header size in bytes
const V2_HEADER_SIZE: usize = 32;

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
    // v2 subword support fields
    /// Number of subword buckets (0 for v1 format)
    bucket_count: usize,
    /// Byte offset to bucket section in mmap
    bucket_offset: usize,
    /// Minimum n-gram length (v2 only)
    minn: usize,
    /// Maximum n-gram length (v2 only)
    maxn: usize,
}

impl FastTextProvider {
    /// Load a FastText model from IronBase binary format (v1 or v2)
    pub fn load(path: &Path) -> EmbeddingResult<Self> {
        let file = File::open(path).map_err(|e| {
            EmbeddingError::ModelLoadError(format!("Failed to open model file: {}", e))
        })?;

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

        // Auto-detect format: check for v2 magic
        let is_v2 = mmap.len() >= V2_HEADER_SIZE && &mmap[0..4] == V2_MAGIC;

        let model_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        if is_v2 {
            Self::load_v2(mmap, model_name)
        } else {
            Self::load_v1(mmap, model_name)
        }
    }

    /// Load v1 format (legacy: vocab_size + dim + word/vector pairs)
    fn load_v1(mmap: Mmap, model_name: String) -> EmbeddingResult<Self> {
        let vocab_size = i32::from_le_bytes([mmap[0], mmap[1], mmap[2], mmap[3]]) as usize;
        let dim = i32::from_le_bytes([mmap[4], mmap[5], mmap[6], mmap[7]]) as usize;

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
            "Loading FastText v1 model: vocab_size={}, dim={}",
            vocab_size,
            dim
        );

        let word_index = Self::build_word_index(&mmap, 8, vocab_size, dim)?;
        let actual_vocab_size = word_index.len();
        log::info!("FastText v1 model loaded: {} words indexed", actual_vocab_size);

        Ok(Self {
            mmap,
            word_index,
            dim,
            vocab_size: actual_vocab_size,
            model_name,
            bucket_count: 0,
            bucket_offset: 0,
            minn: 0,
            maxn: 0,
        })
    }

    /// Load v2 format (32-byte header + word section + bucket section)
    fn load_v2(mmap: Mmap, model_name: String) -> EmbeddingResult<Self> {
        if mmap.len() < V2_HEADER_SIZE {
            return Err(EmbeddingError::ModelLoadError(
                "v2 file too small for header".to_string(),
            ));
        }

        // Parse 32-byte header: magic(4) + dim(4) + vocab_size(4) + bucket_count(4)
        //                        + minn(4) + maxn(4) + bucket_offset(8)
        let dim = u32::from_le_bytes([mmap[4], mmap[5], mmap[6], mmap[7]]) as usize;
        let vocab_size =
            u32::from_le_bytes([mmap[8], mmap[9], mmap[10], mmap[11]]) as usize;
        let bucket_count =
            u32::from_le_bytes([mmap[12], mmap[13], mmap[14], mmap[15]]) as usize;
        let minn = u32::from_le_bytes([mmap[16], mmap[17], mmap[18], mmap[19]]) as usize;
        let maxn = u32::from_le_bytes([mmap[20], mmap[21], mmap[22], mmap[23]]) as usize;
        let bucket_offset = u64::from_le_bytes([
            mmap[24], mmap[25], mmap[26], mmap[27], mmap[28], mmap[29], mmap[30], mmap[31],
        ]) as usize;

        // Validate header values
        if dim == 0 || dim > MAX_DIMENSION {
            return Err(EmbeddingError::ModelLoadError(format!(
                "v2: invalid dimension {} (must be 1-{})",
                dim, MAX_DIMENSION
            )));
        }
        if vocab_size > MAX_VOCAB_SIZE {
            return Err(EmbeddingError::ModelLoadError(format!(
                "v2: vocab_size {} exceeds maximum {}",
                vocab_size, MAX_VOCAB_SIZE
            )));
        }
        if bucket_count > MAX_BUCKET_COUNT {
            return Err(EmbeddingError::ModelLoadError(format!(
                "v2: bucket_count {} exceeds maximum {}",
                bucket_count, MAX_BUCKET_COUNT
            )));
        }
        if minn > maxn || maxn > 100 {
            return Err(EmbeddingError::ModelLoadError(format!(
                "v2: invalid n-gram range minn={} maxn={}",
                minn, maxn
            )));
        }
        if bucket_offset >= mmap.len() {
            return Err(EmbeddingError::ModelLoadError(format!(
                "v2: bucket_offset {} beyond file size {}",
                bucket_offset,
                mmap.len()
            )));
        }

        // Validate bucket section fits in file
        let expected_bucket_end = bucket_offset + bucket_count * dim * 4;
        if expected_bucket_end > mmap.len() {
            return Err(EmbeddingError::ModelLoadError(format!(
                "v2: bucket section end {} beyond file size {}",
                expected_bucket_end,
                mmap.len()
            )));
        }

        log::info!(
            "Loading FastText v2 model: vocab_size={}, dim={}, buckets={}, minn={}, maxn={}",
            vocab_size,
            dim,
            bucket_count,
            minn,
            maxn
        );

        let word_index = Self::build_word_index(&mmap, V2_HEADER_SIZE, vocab_size, dim)?;
        let actual_vocab_size = word_index.len();

        log::info!(
            "FastText v2 model loaded: {} words indexed, {} subword buckets",
            actual_vocab_size,
            bucket_count
        );

        Ok(Self {
            mmap,
            word_index,
            dim,
            vocab_size: actual_vocab_size,
            model_name,
            bucket_count,
            bucket_offset,
            minn,
            maxn,
        })
    }

    /// Build word index from word section starting at `start_offset`
    fn build_word_index(
        mmap: &Mmap,
        start_offset: usize,
        vocab_size: usize,
        dim: usize,
    ) -> EmbeddingResult<HashMap<String, usize>> {
        let mut word_index = HashMap::new();
        word_index.try_reserve(vocab_size).map_err(|_| {
            EmbeddingError::ModelLoadError(format!(
                "failed to allocate word index for {} words",
                vocab_size
            ))
        })?;

        let mut offset = start_offset;

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

        Ok(word_index)
    }

    /// Get vector for a single known word from the word section
    fn word_vector(&self, word: &str) -> Option<Vec<f32>> {
        let offset = *self.word_index.get(word)?;
        self.read_vector_at(offset)
    }

    /// Read a vector (dim × f32) from the mmap at the given byte offset
    fn read_vector_at(&self, offset: usize) -> Option<Vec<f32>> {
        let end = offset + self.dim * 4;
        if end > self.mmap.len() {
            return None;
        }

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

    // ---- v2 subword support ----

    /// FNV-1a hash (FastText convention, operates on raw bytes)
    fn fnv_hash(data: &[u8]) -> u32 {
        let mut h: u32 = 2_166_136_261;
        for &b in data {
            h ^= b as u32;
            h = h.wrapping_mul(16_777_619);
        }
        h
    }

    /// Extract character n-grams from padded word "<word>"
    fn char_ngrams(word: &str, minn: usize, maxn: usize) -> Vec<String> {
        let padded = format!("<{}>", word);
        let chars: Vec<char> = padded.chars().collect();
        let mut ngrams = Vec::new();

        for n in minn..=maxn {
            if n > chars.len() {
                break;
            }
            for i in 0..=chars.len() - n {
                let ngram: String = chars[i..i + n].iter().collect();
                ngrams.push(ngram);
            }
        }

        ngrams
    }

    /// Read a single bucket vector from the bucket section
    fn bucket_vector(&self, bucket_id: usize) -> Option<Vec<f32>> {
        if bucket_id >= self.bucket_count {
            return None;
        }
        let offset = self.bucket_offset + bucket_id * self.dim * 4;
        self.read_vector_at(offset)
    }

    /// Compute OOV word vector from subword n-gram buckets
    fn compute_subword_vector(&self, word: &str) -> Option<Vec<f32>> {
        if self.bucket_count == 0 {
            return None;
        }

        let ngrams = Self::char_ngrams(word, self.minn, self.maxn);
        if ngrams.is_empty() {
            return None;
        }

        let mut result = vec![0.0f32; self.dim];
        let mut count = 0usize;

        for ngram in &ngrams {
            let bucket_id = Self::fnv_hash(ngram.as_bytes()) as usize % self.bucket_count;
            if let Some(bvec) = self.bucket_vector(bucket_id) {
                for (i, v) in bvec.iter().enumerate() {
                    result[i] += v;
                }
                count += 1;
            }
        }

        if count == 0 {
            return None;
        }

        // Average
        let scale = 1.0 / count as f32;
        for x in result.iter_mut() {
            *x *= scale;
        }

        Some(result)
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

    /// Check if this model has subword (v2) support
    #[allow(dead_code)]
    pub fn has_subword_support(&self) -> bool {
        self.bucket_count > 0
    }
}

impl EmbeddingProvider for FastTextProvider {
    fn embed(&self, text: &str) -> EmbeddingResult<Vec<f32>> {
        let opts = FtsOptions::with_settings(FtsLanguage::Hungarian, 2, false);
        let tokens = fulltext_tokenize(text, &opts);

        if tokens.is_empty() {
            return Ok(vec![0.0; self.dim]);
        }

        let mut result = vec![0.0f32; self.dim];
        let mut count = 0usize;

        for token in &tokens {
            if let Some(vec) = self.word_vector(token) {
                // Known word — use stored vector
                for (i, v) in vec.iter().enumerate() {
                    result[i] += v;
                }
                count += 1;
            } else if self.bucket_count > 0 {
                // OOV — compute from subword n-grams (v2 only)
                if let Some(vec) = self.compute_subword_vector(token) {
                    for (i, v) in vec.iter().enumerate() {
                        result[i] += v;
                    }
                    count += 1;
                }
            }
        }

        if count > 0 {
            let scale = 1.0 / count as f32;
            for x in result.iter_mut() {
                *x *= scale;
            }
            Self::normalize(&mut result);
        }

        Ok(result)
    }

    fn embed_batch(&self, texts: &[&str]) -> EmbeddingResult<Vec<Vec<f32>>> {
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
        use ironbase_core::fulltext::{tokenize as fulltext_tokenize, FtsLanguage, FtsOptions};
        let opts = FtsOptions::with_settings(FtsLanguage::Hungarian, 2, false);

        // Hungarian stop words ("van", "egy", "és") should be filtered out
        let tokens = fulltext_tokenize("A király és egy vitéz van itt.", &opts);
        assert!(!tokens.contains(&"van".to_string()), "stop word 'van' should be filtered");
        assert!(!tokens.contains(&"egy".to_string()), "stop word 'egy' should be filtered");
        assert!(!tokens.contains(&"és".to_string()), "stop word 'és' should be filtered");

        // Single char 'a' should be filtered out (min_word_length=2 + stop word)
        assert!(!tokens.contains(&"a".to_string()));

        // Content words should survive (possibly stemmed)
        assert!(!tokens.is_empty(), "content words should remain after filtering");

        // Stemming test: "királyok" → "király" (plural suffix removed)
        let tokens2 = fulltext_tokenize("királyok vitézek", &opts);
        assert!(tokens2.iter().any(|t| t.starts_with("király")), "stemmer should stem 'királyok'");
    }

    #[test]
    fn test_normalize() {
        let mut vec = vec![3.0, 4.0];
        FastTextProvider::normalize(&mut vec);
        let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_fnv_hash() {
        // Known FNV-1a values for verification
        let h1 = FastTextProvider::fnv_hash(b"");
        assert_eq!(h1, 2_166_136_261); // FNV offset basis

        let h2 = FastTextProvider::fnv_hash(b"a");
        assert_eq!(h2, 0xe40c292c);

        // Different inputs should produce different hashes
        let h3 = FastTextProvider::fnv_hash(b"<feker");
        let h4 = FastTextProvider::fnv_hash(b"ekere");
        assert_ne!(h3, h4);
    }

    #[test]
    fn test_char_ngrams() {
        // "ab" with minn=3, maxn=3 → padded "<ab>" (4 chars)
        // 3-grams: "<ab", "ab>"
        let ngrams = FastTextProvider::char_ngrams("ab", 3, 3);
        assert_eq!(ngrams, vec!["<ab", "ab>"]);

        // "abc" with minn=3, maxn=4 → padded "<abc>" (5 chars)
        // 3-grams: "<ab", "abc", "bc>"
        // 4-grams: "<abc", "abc>"
        let ngrams = FastTextProvider::char_ngrams("abc", 3, 4);
        assert_eq!(ngrams, vec!["<ab", "abc", "bc>", "<abc", "abc>"]);

        // Unicode: "áé" with minn=3, maxn=3 → padded "<áé>" (4 chars)
        let ngrams = FastTextProvider::char_ngrams("áé", 3, 3);
        assert_eq!(ngrams, vec!["<áé", "áé>"]);

        // Empty word with minn=5 → padded "<>" (2 chars), no 5-grams possible
        let ngrams = FastTextProvider::char_ngrams("", 5, 5);
        assert!(ngrams.is_empty());
    }

    #[test]
    fn test_v1_format_detection() {
        // Create a minimal v1 file in memory: vocab_size=1, dim=2, word "hi\0", vec [1.0, 2.0]
        let mut data: Vec<u8> = Vec::new();
        data.extend_from_slice(&1i32.to_le_bytes()); // vocab_size
        data.extend_from_slice(&2i32.to_le_bytes()); // dim
        data.extend_from_slice(b"hi\0"); // word
        data.extend_from_slice(&1.0f32.to_le_bytes()); // vec[0]
        data.extend_from_slice(&2.0f32.to_le_bytes()); // vec[1]

        // First 4 bytes = [1, 0, 0, 0] — not "IBv2", so should be v1
        assert_ne!(&data[0..4], V2_MAGIC);

        // Write to tempfile and load
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_v1.bin");
        std::fs::write(&path, &data).unwrap();

        let provider = FastTextProvider::load(&path).unwrap();
        assert_eq!(provider.dim, 2);
        assert_eq!(provider.vocab_size, 1);
        assert_eq!(provider.bucket_count, 0);
        assert!(!provider.has_subword_support());

        // Check the vector
        let vec = provider.word_vector("hi").unwrap();
        assert_eq!(vec, vec![1.0, 2.0]);
    }

    #[test]
    fn test_v2_header_parsing() {
        // Create a minimal v2 file: 1 word "hi", dim=2, 1 bucket
        let dim: u32 = 2;
        let vocab_size: u32 = 1;
        let bucket_count: u32 = 1;
        let minn: u32 = 3;
        let maxn: u32 = 5;

        // Word section: "hi\0" + [1.0, 2.0]
        let word_section_size = 3 + dim as usize * 4; // "hi\0" = 3 bytes + 2 floats
        let bucket_offset: u64 = (V2_HEADER_SIZE + word_section_size) as u64;

        // Build header
        let mut data: Vec<u8> = Vec::new();
        data.extend_from_slice(V2_MAGIC);
        data.extend_from_slice(&dim.to_le_bytes());
        data.extend_from_slice(&vocab_size.to_le_bytes());
        data.extend_from_slice(&bucket_count.to_le_bytes());
        data.extend_from_slice(&minn.to_le_bytes());
        data.extend_from_slice(&maxn.to_le_bytes());
        data.extend_from_slice(&bucket_offset.to_le_bytes());
        assert_eq!(data.len(), V2_HEADER_SIZE);

        // Word section
        data.extend_from_slice(b"hi\0");
        data.extend_from_slice(&1.0f32.to_le_bytes());
        data.extend_from_slice(&2.0f32.to_le_bytes());

        // Bucket section: 1 bucket with [3.0, 4.0]
        data.extend_from_slice(&3.0f32.to_le_bytes());
        data.extend_from_slice(&4.0f32.to_le_bytes());

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_v2.bin");
        std::fs::write(&path, &data).unwrap();

        let provider = FastTextProvider::load(&path).unwrap();
        assert_eq!(provider.dim, 2);
        assert_eq!(provider.vocab_size, 1);
        assert_eq!(provider.bucket_count, 1);
        assert_eq!(provider.minn, 3);
        assert_eq!(provider.maxn, 5);
        assert_eq!(provider.bucket_offset, bucket_offset as usize);
        assert!(provider.has_subword_support());

        // Known word
        let vec = provider.word_vector("hi").unwrap();
        assert_eq!(vec, vec![1.0, 2.0]);

        // Bucket vector
        let bvec = provider.bucket_vector(0).unwrap();
        assert_eq!(bvec, vec![3.0, 4.0]);
    }

    #[test]
    fn test_v2_oov_subword_vector() {
        // Create v2 file with 2 buckets, dim=2, no known words
        let dim: u32 = 2;
        let vocab_size: u32 = 0;
        let bucket_count: u32 = 2;
        let minn: u32 = 3;
        let maxn: u32 = 3;
        let bucket_offset: u64 = V2_HEADER_SIZE as u64; // no words, buckets start right away

        let mut data: Vec<u8> = Vec::new();
        data.extend_from_slice(V2_MAGIC);
        data.extend_from_slice(&dim.to_le_bytes());
        data.extend_from_slice(&vocab_size.to_le_bytes());
        data.extend_from_slice(&bucket_count.to_le_bytes());
        data.extend_from_slice(&minn.to_le_bytes());
        data.extend_from_slice(&maxn.to_le_bytes());
        data.extend_from_slice(&bucket_offset.to_le_bytes());

        // Bucket 0: [1.0, 0.0], Bucket 1: [0.0, 1.0]
        data.extend_from_slice(&1.0f32.to_le_bytes());
        data.extend_from_slice(&0.0f32.to_le_bytes());
        data.extend_from_slice(&0.0f32.to_le_bytes());
        data.extend_from_slice(&1.0f32.to_le_bytes());

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_v2_oov.bin");
        std::fs::write(&path, &data).unwrap();

        let provider = FastTextProvider::load(&path).unwrap();

        // OOV word should produce a non-zero vector from subword buckets
        let vec = provider.compute_subword_vector("teszt").unwrap();
        assert_eq!(vec.len(), 2);
        // Should be non-zero (average of bucket vectors hit by n-gram hashes)
        let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(norm > 0.0, "OOV vector should be non-zero");
    }
}
