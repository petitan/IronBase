//! RAG (Retrieval Augmented Generation) Integration Tests
//!
//! Tests for the RAG module including:
//! - Markdown chunking
//! - FastText embeddings (mocked)
//! - HNSW vector index
//! - RagManager integration
//! - Memory limits (RagLimits)

use ironbase_core::rag::{BlockType, Chunk, ChunkConfig, Chunker, HnswIndex, RagLimits};

// ============================================================================
// Chunker Tests
// ============================================================================

#[test]
fn test_chunker_simple_markdown() {
    let chunker = Chunker::with_defaults();
    let md = r#"
# Introduction

This is the introduction paragraph with some text.

## Section 1

Section 1 content goes here.

## Section 2

Section 2 content is different.
"#;

    let chunks = chunker.chunk_markdown(md).unwrap();
    assert!(!chunks.is_empty(), "Should produce at least one chunk");

    // Should have section paths
    let section_chunk = chunks.iter().find(|c| c.text.contains("Section 1 content"));
    assert!(section_chunk.is_some(), "Should find Section 1 content");
}

#[test]
fn test_chunker_preserves_tables() {
    let chunker = Chunker::with_defaults();
    let md = r#"
# Table Section

Some text before.

| Header 1 | Header 2 | Header 3 |
|----------|----------|----------|
| Cell 1   | Cell 2   | Cell 3   |
| Cell 4   | Cell 5   | Cell 6   |

Text after table.
"#;

    let chunks = chunker.chunk_markdown(md).unwrap();

    // Find the table chunk
    let table_chunk = chunks.iter().find(|c| c.block_type == BlockType::Table);
    assert!(table_chunk.is_some(), "Should have a table chunk");

    let table = table_chunk.unwrap();
    assert!(
        table.text.contains("Header 1"),
        "Table should contain headers"
    );
    assert!(
        table.text.contains("Cell 6"),
        "Table should contain all cells"
    );
}

#[test]
fn test_chunker_preserves_code_blocks() {
    let chunker = Chunker::with_defaults();
    let md = r#"
# Code Example

Here's some code:

```rust
fn main() {
    println!("Hello, World!");
}
```

And some more text.
"#;

    let chunks = chunker.chunk_markdown(md).unwrap();

    // Find the code chunk
    let code_chunk = chunks.iter().find(|c| c.block_type == BlockType::Code);
    assert!(code_chunk.is_some(), "Should have a code chunk");

    let code = code_chunk.unwrap();
    assert!(code.text.contains("fn main()"), "Code should be preserved");
    assert!(
        code.text.contains("println!"),
        "Code should contain println"
    );
}

#[test]
fn test_chunker_section_hierarchy() {
    let chunker = Chunker::with_defaults();
    let md = r#"
# Chapter 1

## Section 1.1

### Subsection 1.1.1

Deep nested content here.

## Section 1.2

Another section.
"#;

    let chunks = chunker.chunk_markdown(md).unwrap();

    // Find the deeply nested chunk
    let deep_chunk = chunks.iter().find(|c| c.text.contains("Deep nested"));
    assert!(deep_chunk.is_some(), "Should find deep content");

    let path = &deep_chunk.unwrap().section_path;
    assert_eq!(path.len(), 3, "Should have 3-level hierarchy");
    assert!(path[0].contains("Chapter"), "First level is Chapter");
    assert!(path[1].contains("1.1"), "Second level is Section 1.1");
    assert!(path[2].contains("1.1.1"), "Third level is Subsection");
}

#[test]
fn test_chunker_long_text_splitting() {
    // Create a config with small max_tokens to force splitting
    let config = ChunkConfig {
        max_tokens: 50,
        overlap_tokens: 10,
        min_chunk_size: 5,
        ..Default::default()
    };
    let chunker = Chunker::new(config);

    // Generate long text
    let long_text: String = (0..100)
        .map(|i| format!("This is sentence number {}. ", i))
        .collect();
    let md = format!("# Long Document\n\n{}", long_text);

    let chunks = chunker.chunk_markdown(&md).unwrap();

    // Should split into multiple chunks
    assert!(chunks.len() > 1, "Should have multiple chunks");

    // Each chunk should respect the token limit (approximately)
    for chunk in &chunks {
        assert!(
            chunk.token_count <= 75, // Some tolerance
            "Chunk has {} tokens, expected <= 75",
            chunk.token_count
        );
    }
}

#[test]
fn test_chunker_document_size_limit() {
    let config = ChunkConfig {
        max_document_bytes: 100, // Very small limit for testing
        ..Default::default()
    };
    let chunker = Chunker::new(config);

    // Create document larger than limit
    let large_doc = "x".repeat(200);

    let result = chunker.chunk_markdown(&large_doc);
    assert!(
        result.is_err(),
        "Should reject document exceeding size limit"
    );

    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("too large"),
        "Error should mention size: {}",
        err
    );
}

#[test]
fn test_chunker_chunk_count_limit() {
    let config = ChunkConfig {
        max_tokens: 10,    // Small chunks
        overlap_tokens: 2, // Small overlap (must be < max_tokens)
        min_chunk_size: 1,
        max_chunks_per_document: 5, // Very small limit
        ..Default::default()
    };
    let chunker = Chunker::new(config);

    // Create document that would generate many chunks
    let md = (0..100)
        .map(|i| format!("Paragraph {} with some words here. ", i))
        .collect::<String>();

    let result = chunker.chunk_markdown(&md);
    // Depending on exact splitting, this might succeed or fail
    // The important thing is it doesn't panic or OOM
    match result {
        Ok(chunks) => assert!(chunks.len() <= 5, "Should respect chunk limit"),
        Err(e) => assert!(
            e.to_string().contains("too many chunks"),
            "Error should mention chunk count: {}",
            e
        ),
    }
}

// ============================================================================
// HNSW Index Tests
// ============================================================================

#[test]
fn test_hnsw_empty_index() {
    let hnsw = HnswIndex::with_dim(100);
    assert!(hnsw.is_empty());
    assert_eq!(hnsw.len(), 0);
    assert_eq!(hnsw.dim(), 100);
}

#[test]
fn test_hnsw_insert_and_search() {
    let mut hnsw = HnswIndex::with_dim(3);

    // Insert some vectors
    hnsw.insert("doc1", &[1.0, 0.0, 0.0]).unwrap();
    hnsw.insert("doc2", &[0.0, 1.0, 0.0]).unwrap();
    hnsw.insert("doc3", &[0.0, 0.0, 1.0]).unwrap();

    assert_eq!(hnsw.len(), 3);

    // Search for nearest to [1.0, 0.0, 0.0]
    let results = hnsw.search(&[1.0, 0.0, 0.0], 2);
    assert!(!results.is_empty());
    assert_eq!(results[0].0, "doc1", "Should find doc1 as nearest");
}

#[test]
fn test_hnsw_dimension_mismatch() {
    let mut hnsw = HnswIndex::with_dim(3);

    hnsw.insert("doc1", &[1.0, 0.0, 0.0]).unwrap();

    // Try to insert vector with wrong dimension
    let result = hnsw.insert("doc2", &[1.0, 0.0]); // Only 2 dimensions
    assert!(result.is_err(), "Should reject mismatched dimension");
}

#[test]
fn test_hnsw_duplicate_id() {
    let mut hnsw = HnswIndex::with_dim(3);

    hnsw.insert("doc1", &[1.0, 0.0, 0.0]).unwrap();
    let result = hnsw.insert("doc1", &[0.0, 1.0, 0.0]); // Same ID

    assert!(result.is_err(), "Should reject duplicate ID");
}

#[test]
fn test_hnsw_upsert_insert() {
    let mut hnsw = HnswIndex::with_dim(3);

    // Upsert on empty index = insert
    let was_update = hnsw.upsert("doc1", &[1.0, 0.0, 0.0]).unwrap();
    assert!(!was_update, "First upsert should be an insert");
    assert_eq!(hnsw.len(), 1);
    assert!(hnsw.contains("doc1"));
}

#[test]
fn test_hnsw_upsert_update() {
    let mut hnsw = HnswIndex::with_dim(3);

    // First insert
    hnsw.insert("doc1", &[1.0, 0.0, 0.0]).unwrap();

    // Search before update
    let results_before = hnsw.search(&[0.0, 1.0, 0.0], 1);
    let score_before = results_before[0].1;

    // Upsert with new vector (closer to [0, 1, 0])
    let was_update = hnsw.upsert("doc1", &[0.0, 1.0, 0.0]).unwrap();
    assert!(was_update, "Second upsert should be an update");
    assert_eq!(hnsw.len(), 1, "Should still have only 1 vector");

    // Search after update - should now have higher similarity
    let results_after = hnsw.search(&[0.0, 1.0, 0.0], 1);
    let score_after = results_after[0].1;
    assert!(
        score_after > score_before,
        "Updated vector should have higher similarity to query"
    );
}

#[test]
fn test_hnsw_upsert_dimension_mismatch() {
    let mut hnsw = HnswIndex::with_dim(3);
    hnsw.insert("doc1", &[1.0, 0.0, 0.0]).unwrap();

    // Try to upsert with wrong dimension
    let result = hnsw.upsert("doc1", &[1.0, 0.0]); // 2D instead of 3D
    assert!(result.is_err(), "Should reject mismatched dimension");
}

#[test]
fn test_hnsw_upsert_multiple() {
    let mut hnsw = HnswIndex::with_dim(3);

    // Multiple upserts on same ID
    let r1 = hnsw.upsert("doc1", &[1.0, 0.0, 0.0]).unwrap();
    let r2 = hnsw.upsert("doc1", &[0.0, 1.0, 0.0]).unwrap();
    let r3 = hnsw.upsert("doc1", &[0.0, 0.0, 1.0]).unwrap();

    assert!(!r1, "First should be insert");
    assert!(r2, "Second should be update");
    assert!(r3, "Third should be update");
    assert_eq!(hnsw.len(), 1, "Should still have only 1 vector");

    // Verify final vector
    let results = hnsw.search(&[0.0, 0.0, 1.0], 1);
    assert_eq!(results[0].0, "doc1");
    assert!(
        results[0].1 > 0.99,
        "Should find exact match after final update"
    );
}

#[test]
fn test_hnsw_search_limit() {
    let mut hnsw = HnswIndex::with_dim(3);

    // Insert 10 vectors
    for i in 0..10 {
        let v = vec![i as f32, 0.0, 0.0];
        hnsw.insert(&format!("doc{}", i), &v).unwrap();
    }

    // Search with limit
    let results = hnsw.search(&[5.0, 0.0, 0.0], 3);
    assert_eq!(results.len(), 3, "Should return exactly 3 results");
}

#[test]
fn test_hnsw_rebuild() {
    let mut hnsw = HnswIndex::with_dim(3);

    hnsw.insert("doc1", &[1.0, 0.0, 0.0]).unwrap();
    hnsw.insert("doc2", &[0.0, 1.0, 0.0]).unwrap();

    let original_len = hnsw.len();
    assert_eq!(original_len, 2);

    // Rebuild the index (reorganizes internally)
    hnsw.rebuild().unwrap();

    // Should still have same data after rebuild
    assert_eq!(hnsw.len(), 2);
    assert!(hnsw.contains("doc1"));
    assert!(hnsw.contains("doc2"));
}

#[test]
fn test_hnsw_vector_limit() {
    // Create index with small max_vectors limit
    let mut hnsw = HnswIndex::with_dim_and_limits(3, 5);

    // Insert up to limit
    for i in 0..5 {
        hnsw.insert(&format!("doc{}", i), &[i as f32, 0.0, 0.0])
            .unwrap();
    }

    // Should reject when at limit
    let result = hnsw.insert("doc5", &[5.0, 0.0, 0.0]);
    assert!(result.is_err(), "Should reject when at vector limit");
    assert!(
        result.unwrap_err().to_string().contains("full"),
        "Error should mention index is full"
    );
}

// ============================================================================
// RagLimits Tests
// ============================================================================

#[test]
fn test_rag_limits_default() {
    let limits = RagLimits::default();

    assert_eq!(
        limits.max_document_bytes,
        10 * 1024 * 1024,
        "Default max doc: 10MB"
    );
    assert_eq!(limits.max_chunks_per_document, 10_000);
    assert_eq!(limits.max_total_chunks, 100_000);
    assert_eq!(limits.max_hnsw_vectors, 100_000);
}

#[test]
fn test_rag_limits_low_memory() {
    let limits = RagLimits::low_memory();

    assert_eq!(limits.max_document_bytes, 5 * 1024 * 1024, "Low mem: 5MB");
    assert_eq!(limits.max_chunks_per_document, 2_500);
    assert_eq!(limits.max_total_chunks, 25_000);
    assert_eq!(limits.max_hnsw_vectors, 25_000);
}

#[test]
fn test_rag_limits_with_budget() {
    // Test different memory budgets
    let limits_64 = RagLimits::with_memory_budget(64);
    let limits_512 = RagLimits::with_memory_budget(512);
    let limits_1024 = RagLimits::with_memory_budget(1024);

    // Higher budget = higher limits
    assert!(
        limits_512.max_document_bytes > limits_64.max_document_bytes,
        "512MB budget should have higher doc limit than 64MB"
    );
    assert!(
        limits_1024.max_total_chunks > limits_512.max_total_chunks,
        "1GB budget should have higher chunk limit than 512MB"
    );
}

#[test]
fn test_rag_limits_unlimited() {
    let limits = RagLimits::unlimited();

    assert_eq!(limits.max_document_bytes, usize::MAX);
    assert_eq!(limits.max_chunks_per_document, usize::MAX);
    assert_eq!(limits.max_total_chunks, usize::MAX);
    assert_eq!(limits.max_hnsw_vectors, usize::MAX);
}

#[test]
fn test_rag_limits_from_system_memory() {
    // This should not panic and return reasonable values
    let limits = RagLimits::from_system_memory();

    // Basic sanity checks
    assert!(
        limits.max_document_bytes >= 5 * 1024 * 1024,
        "At least 5MB doc"
    );
    assert!(
        limits.max_chunks_per_document >= 2_500,
        "At least 2.5K chunks"
    );
    assert!(limits.max_total_chunks >= 25_000, "At least 25K total");
}

// ============================================================================
// SIMD Vector Operations Tests
// ============================================================================

mod simd_tests {
    use ironbase_core::rag::simd::*;

    const EPSILON: f32 = 1e-5;

    fn approx_eq(a: f32, b: f32) -> bool {
        (a - b).abs() < EPSILON
    }

    #[test]
    fn test_squared_euclidean_basic() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![4.0, 5.0, 6.0];
        // (4-1)^2 + (5-2)^2 + (6-3)^2 = 9 + 9 + 9 = 27
        assert!(approx_eq(squared_euclidean_distance(&a, &b), 27.0));
    }

    #[test]
    fn test_cosine_similarity_identical() {
        let v = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        assert!(approx_eq(cosine_similarity(&v, &v), 1.0));
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        assert!(approx_eq(cosine_similarity(&a, &b), 0.0));
    }

    #[test]
    fn test_fasttext_dimension_vectors() {
        // Test with typical FastText dimension (300)
        let a: Vec<f32> = (0..300).map(|i| (i as f32) * 0.001).collect();
        let b: Vec<f32> = (0..300).map(|i| ((i + 1) as f32) * 0.001).collect();

        // Should complete without panic
        let dist = euclidean_distance(&a, &b);
        let sim = cosine_similarity(&a, &b);

        assert!(dist > 0.0);
        assert!(sim > 0.0 && sim <= 1.0);
    }

    #[test]
    fn test_vector_lengths() {
        // Test various vector lengths (including those that don't divide by 8)
        for len in [1, 3, 7, 8, 9, 15, 16, 17, 100, 300, 301] {
            let a: Vec<f32> = (0..len).map(|i| i as f32).collect();
            let b: Vec<f32> = (0..len).map(|i| (i + 1) as f32).collect();

            // Should not panic
            let _ = squared_euclidean_distance(&a, &b);
            let _ = cosine_similarity(&a, &b);
        }
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Create a test chunk with minimal data
fn make_test_chunk(text: &str, block_type: BlockType) -> Chunk {
    Chunk {
        text: text.to_string(),
        char_range: (0, text.len()),
        token_count: text.split_whitespace().count(),
        block_type,
        section_path: vec![],
        parent_heading: None,
    }
}

#[test]
fn test_helper_make_test_chunk() {
    let chunk = make_test_chunk("Hello world test", BlockType::Paragraph);
    assert_eq!(chunk.text, "Hello world test");
    assert_eq!(chunk.token_count, 3);
    assert_eq!(chunk.block_type, BlockType::Paragraph);
}
