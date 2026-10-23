//! SIMD-optimized vector operations
//!
//! This module provides optimized implementations of common vector operations
//! used in HNSW nearest neighbor search. The code is structured to enable
//! LLVM auto-vectorization on stable Rust.
//!
//! # Optimizations
//!
//! 1. **Independent lanes**: 8 partial sums, so LLVM can keep them in one SIMD
//!    register (a single running sum is a serial chain it may not reorder)
//! 2. **Squared distance**: Avoid sqrt in inner loops where possible
//! 3. **Pre-computed norms**: Cache vector norms for cosine similarity
//! 4. **Aligned access patterns**: Predictable memory access for prefetching

/// Independent partial sums per kernel: LLVM keeps each in a SIMD lane
/// (8 × f32 = one AVX2 register)
const LANES: usize = 8;

/// Add up the partial sums pairwise
#[inline]
fn sum_lanes(acc: &[f32; LANES]) -> f32 {
    ((acc[0] + acc[4]) + (acc[1] + acc[5])) + ((acc[2] + acc[6]) + (acc[3] + acc[7]))
}

/// Compute squared Euclidean distance between two vectors (no sqrt)
///
/// This is faster than full Euclidean distance and preserves ordering
/// for nearest neighbor search (since sqrt is monotonic).
///
/// # Performance
///
/// For 300-dimensional vectors:
/// - ~2x faster than naive implementation with sqrt
/// - Auto-vectorizes to AVX2/SSE on x86_64
#[inline]
pub fn squared_euclidean_distance(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    // LANES independent sums: one running sum is a serial chain of
    // dependent additions that LLVM may not reorder (strict float
    // semantics), so it never vectorized (hybrid review T6)
    let mut acc = [0.0f32; LANES];
    let (a_chunks, a_rest) = a.as_chunks::<LANES>();
    let (b_chunks, b_rest) = b.as_chunks::<LANES>();
    for (x, y) in a_chunks.iter().zip(b_chunks) {
        for i in 0..LANES {
            let d = x[i] - y[i];
            acc[i] += d * d;
        }
    }
    let mut sum = sum_lanes(&acc);

    // Handle remainder
    for (x, y) in a_rest.iter().zip(b_rest) {
        let d = x - y;
        sum += d * d;
    }

    sum
}

/// Compute Euclidean distance between two vectors
///
/// Uses squared distance internally and applies sqrt once at the end.
#[inline]
pub fn euclidean_distance(a: &[f32], b: &[f32]) -> f32 {
    squared_euclidean_distance(a, b).sqrt()
}

/// Compute dot product of two vectors
///
/// # Performance
///
/// Uses 8-wide loop unrolling for SIMD auto-vectorization.
#[inline]
pub fn dot_product(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    let mut acc = [0.0f32; LANES];
    let (a_chunks, a_rest) = a.as_chunks::<LANES>();
    let (b_chunks, b_rest) = b.as_chunks::<LANES>();
    for (x, y) in a_chunks.iter().zip(b_chunks) {
        for i in 0..LANES {
            acc[i] += x[i] * y[i];
        }
    }
    let mut sum = sum_lanes(&acc);

    for (x, y) in a_rest.iter().zip(b_rest) {
        sum += x * y;
    }

    sum
}

/// Compute the L2 norm (magnitude) of a vector
///
/// # Performance
///
/// Uses optimized dot product internally.
#[inline]
pub fn l2_norm(v: &[f32]) -> f32 {
    dot_product(v, v).sqrt()
}

/// Compute squared L2 norm (magnitude squared) of a vector
///
/// Faster than l2_norm when sqrt is not needed.
#[inline]
pub fn squared_l2_norm(v: &[f32]) -> f32 {
    dot_product(v, v)
}

/// Compute cosine similarity between two vectors
///
/// Returns a value in [-1, 1] where 1 means identical direction.
///
/// # Performance
///
/// Computes dot product and norms in a single pass for better cache utilization.
#[inline]
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    let mut dot = [0.0f32; LANES];
    let mut norm_a_sq = [0.0f32; LANES];
    let mut norm_b_sq = [0.0f32; LANES];
    let (a_chunks, a_rest) = a.as_chunks::<LANES>();
    let (b_chunks, b_rest) = b.as_chunks::<LANES>();

    // Single pass: compute dot product and both norms simultaneously
    for (x, y) in a_chunks.iter().zip(b_chunks) {
        for i in 0..LANES {
            dot[i] += x[i] * y[i];
            norm_a_sq[i] += x[i] * x[i];
            norm_b_sq[i] += y[i] * y[i];
        }
    }
    let (mut dot, mut norm_a_sq, mut norm_b_sq) = (
        sum_lanes(&dot),
        sum_lanes(&norm_a_sq),
        sum_lanes(&norm_b_sq),
    );

    // Handle remainder
    for (&ai, &bi) in a_rest.iter().zip(b_rest) {
        dot += ai * bi;
        norm_a_sq += ai * ai;
        norm_b_sq += bi * bi;
    }

    let norm_product = (norm_a_sq * norm_b_sq).sqrt();
    if norm_product > 0.0 {
        dot / norm_product
    } else {
        0.0
    }
}

/// Compute cosine similarity with pre-computed norm for vector `a`
///
/// Use this when searching against a query vector multiple times -
/// pre-compute the query norm once and reuse it.
///
/// # Arguments
///
/// * `a` - Query vector
/// * `a_norm` - Pre-computed L2 norm of `a`
/// * `b` - Target vector
#[inline]
pub fn cosine_similarity_with_norm(a: &[f32], a_norm: f32, b: &[f32]) -> f32 {
    if a_norm == 0.0 {
        return 0.0;
    }

    let dot = dot_product(a, b);
    let b_norm = l2_norm(b);

    if b_norm > 0.0 {
        dot / (a_norm * b_norm)
    } else {
        0.0
    }
}

/// Normalize a vector in-place to unit length
///
/// After normalization, the vector will have L2 norm = 1.0.
/// Does nothing if the vector has zero norm.
#[inline]
pub fn normalize_in_place(v: &mut [f32]) {
    let norm = l2_norm(v);
    if norm > 0.0 {
        let inv_norm = 1.0 / norm;
        for x in v.iter_mut() {
            *x *= inv_norm;
        }
    }
}

/// Create a normalized copy of a vector
#[inline]
pub fn normalize(v: &[f32]) -> Vec<f32> {
    let mut result = v.to_vec();
    normalize_in_place(&mut result);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f32 = 1e-5;

    fn approx_eq(a: f32, b: f32) -> bool {
        (a - b).abs() < EPSILON
    }

    #[test]
    fn test_squared_euclidean_distance() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![4.0, 5.0, 6.0];
        // (4-1)^2 + (5-2)^2 + (6-3)^2 = 9 + 9 + 9 = 27
        assert!(approx_eq(squared_euclidean_distance(&a, &b), 27.0));
    }

    #[test]
    fn test_euclidean_distance() {
        let a = vec![0.0, 0.0, 0.0];
        let b = vec![3.0, 4.0, 0.0];
        assert!(approx_eq(euclidean_distance(&a, &b), 5.0));
    }

    #[test]
    fn test_dot_product() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![4.0, 5.0, 6.0];
        // 1*4 + 2*5 + 3*6 = 4 + 10 + 18 = 32
        assert!(approx_eq(dot_product(&a, &b), 32.0));
    }

    #[test]
    fn test_l2_norm() {
        let v = vec![3.0, 4.0];
        assert!(approx_eq(l2_norm(&v), 5.0));
    }

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.0, 2.0, 3.0];
        assert!(approx_eq(cosine_similarity(&a, &b), 1.0));
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        assert!(approx_eq(cosine_similarity(&a, &b), 0.0));
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![-1.0, -2.0, -3.0];
        assert!(approx_eq(cosine_similarity(&a, &b), -1.0));
    }

    #[test]
    fn test_normalize() {
        let v = vec![3.0, 4.0];
        let n = normalize(&v);
        assert!(approx_eq(l2_norm(&n), 1.0));
        assert!(approx_eq(n[0], 0.6));
        assert!(approx_eq(n[1], 0.8));
    }

    #[test]
    fn test_large_vector_300_dim() {
        // Test with 300-dimensional vectors
        let a: Vec<f32> = (0..300).map(|i| i as f32 * 0.01).collect();
        let b: Vec<f32> = (0..300).map(|i| (i + 1) as f32 * 0.01).collect();

        // Should not panic and produce reasonable results
        let dist = euclidean_distance(&a, &b);
        assert!(dist > 0.0);
        assert!(dist < 10.0);

        let sim = cosine_similarity(&a, &b);
        assert!(sim > 0.9); // Very similar vectors
    }

    #[test]
    fn test_cosine_with_precomputed_norm() {
        let a = vec![1.0, 2.0, 3.0];
        let a_norm = l2_norm(&a);
        let b = vec![4.0, 5.0, 6.0];

        let sim1 = cosine_similarity(&a, &b);
        let sim2 = cosine_similarity_with_norm(&a, a_norm, &b);

        assert!(approx_eq(sim1, sim2));
    }

    #[test]
    fn test_remainder_handling() {
        // Test vectors that don't divide evenly by 8
        for len in [1, 3, 7, 9, 15, 17, 300, 301] {
            let a: Vec<f32> = (0..len).map(|i| i as f32).collect();
            let b: Vec<f32> = (0..len).map(|i| (i + 1) as f32).collect();

            // Should not panic
            let _ = squared_euclidean_distance(&a, &b);
            let _ = dot_product(&a, &b);
            let _ = cosine_similarity(&a, &b);
        }
    }
}
