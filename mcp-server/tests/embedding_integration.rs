//! Integration test for embedding system with real FastText model

use mcp_ironbase::embedding::{EmbeddingManager, EmbeddingProvider, FastTextProvider};
use std::path::Path;

#[test]
#[ignore] // Run with: cargo test --test embedding_integration -- --ignored --nocapture
fn test_fasttext_model_loading() {
    let model_path = std::env::var("IRONBASE_FASTTEXT_MODEL")
        .unwrap_or_else(|_| "/home/petitan/MongoLite/models/cc.hu.300.ironbase.bin".to_string());

    if !Path::new(&model_path).exists() {
        println!("Skipping test: model not found at {}", model_path);
        return;
    }

    println!("Loading FastText model from {}...", model_path);
    let provider = FastTextProvider::load(Path::new(&model_path)).expect("Failed to load model");

    println!("Model: {}", provider.model_name());
    println!("Dimension: {}", provider.dimension());
    assert!(provider.dimension() > 0);

    // Test single embedding
    let text = "Ez egy teszt mondat magyarul.";
    println!("\nEmbedding: \"{}\"", text);
    let vec = provider.embed(text).expect("Embedding failed");

    println!("Vector length: {}", vec.len());
    assert_eq!(vec.len(), provider.dimension());

    println!("First 5 values: {:?}", &vec[..5.min(vec.len())]);

    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    println!("Vector norm: {:.4}", norm);
    assert!((norm - 1.0).abs() < 0.01, "Vector should be normalized");

    // Test batch embedding
    let texts = vec!["kutya", "macska", "ház"];
    println!("\nBatch embedding: {:?}", texts);
    let refs: Vec<&str> = texts.iter().map(|s| &**s).collect();
    let vecs = provider.embed_batch(&refs).expect("Batch failed");
    assert_eq!(vecs.len(), 3);
    println!("Got {} vectors of dim {}", vecs.len(), vecs[0].len());

    println!("\n✅ FastText model test passed!");
}

#[test]
#[ignore]
fn test_embedding_manager_auto_detect() {
    let model_path = "/home/petitan/MongoLite/models/cc.hu.300.ironbase.bin";
    if !Path::new(model_path).exists() {
        println!("Skipping test: model not found");
        return;
    }

    std::env::set_var("IRONBASE_FASTTEXT_MODEL", model_path);

    println!("Testing EmbeddingManager::auto_detect()...");
    let manager = EmbeddingManager::auto_detect();

    println!("Has providers: {}", manager.has_providers());
    assert!(manager.has_providers());

    println!("Default provider: {}", manager.default_provider_name());
    assert_eq!(manager.default_provider_name(), "fasttext");

    let models = manager.list_models();
    println!("\nAvailable models:");
    for m in &models {
        println!("  - {} / {} (dim={}) - {}", m.provider, m.model, m.dimension, m.description);
    }
    assert!(!models.is_empty());

    // Test embed via manager
    let vec = manager.embed("teszt", None).expect("Manager embed failed");
    println!("\nEmbed via manager: {} dimensions", vec.len());
    assert_eq!(vec.len(), 300);

    println!("\n✅ EmbeddingManager test passed!");
}

#[test]
#[ignore]
fn test_display_name_in_list_models() {
    let model_path = "/home/petitan/MongoLite/models/cc.hu.300.ironbase.bin";
    if !Path::new(model_path).exists() {
        println!("Skipping test: model not found");
        return;
    }

    std::env::set_var("IRONBASE_FASTTEXT_MODEL", model_path);
    let manager = EmbeddingManager::auto_detect();

    let models = manager.list_models();
    for m in &models {
        // Check that description uses display_name (should say "fasttext embedding provider")
        println!("Provider: {}, Description: {}", m.provider, m.description);
        assert!(
            m.description.contains("embedding provider"),
            "Description should use display_name format"
        );
    }

    println!("\n✅ display_name test passed!");
}
