//! Embedding generation module for MCP server
//!
//! Supports multiple providers via [embedding] config section:
//! - Ollama (default, local LLM server — BGE-M3, nomic-embed-text, etc.)
//! - vLLM (OpenAI-compatible local server)
//! - OpenAI (cloud API)
//! - FastText (legacy, offline, Hungarian support)
//!
//! Features:
//! - LRU cache for embedding results (10K entries, 1 hour TTL)

pub mod cache;
mod fasttext;
mod http_provider;
mod presets;

pub use cache::{CacheConfig, CacheStats, CachedEmbedding, EmbeddingCache};
pub use fasttext::FastTextProvider;
pub use http_provider::{
    AuthMethod, HttpEmbeddingProvider, HttpProviderConfig, RequestFormat, ResponseFormat,
};

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// Result type for embedding operations
pub type EmbeddingResult<T> = Result<T, EmbeddingError>;

/// Errors that can occur during embedding operations
#[derive(Debug, thiserror::Error)]
pub enum EmbeddingError {
    #[error("Provider not found: {0}")]
    ProviderNotFound(String),

    #[error("Model not found: {0}")]
    ModelNotFound(String),

    #[error("Failed to load model: {0}")]
    ModelLoadError(String),

    #[error("Embedding failed: {0}")]
    EmbeddingFailed(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("HTTP error: {0}")]
    HttpError(String),

    #[error("API error: {0}")]
    ApiError(String),

    #[error("Invalid configuration: {0}")]
    ConfigError(String),
}

/// Trait for embedding providers
pub trait EmbeddingProvider: Send + Sync {
    /// Generate embedding for a single text
    fn embed(&self, text: &str) -> EmbeddingResult<Vec<f32>>;

    /// Generate embeddings for multiple texts (batch)
    fn embed_batch(&self, texts: &[&str]) -> EmbeddingResult<Vec<Vec<f32>>> {
        // Default implementation: call embed for each text
        texts.iter().map(|t| self.embed(t)).collect()
    }

    /// Get the dimension of the embedding vectors
    fn dimension(&self) -> usize;

    /// Get the model name
    fn model_name(&self) -> &str;

    /// Get the provider name
    fn provider_name(&self) -> &str;

    /// Get the display name for UI/logs (defaults to provider_name)
    fn display_name(&self) -> &str {
        self.provider_name()
    }

    /// Embed a query text (may use different prefix than document embedding)
    fn embed_query(&self, text: &str) -> EmbeddingResult<Vec<f32>> {
        self.embed(text)
    }

    /// Preprocessing pipeline version string.
    /// Changes trigger automatic re-embedding on server restart.
    fn preprocessing_version(&self) -> &str {
        "default"
    }
}

/// Information about an available model
#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelInfo {
    pub provider: String,
    pub model: String,
    pub dimension: usize,
    pub description: String,
    pub available: bool,
}

/// Manager for embedding providers with optional caching
pub struct EmbeddingManager {
    providers: HashMap<String, Arc<dyn EmbeddingProvider>>,
    default_provider: String,
    /// Optional embedding cache (shared across all providers)
    cache: Option<Arc<EmbeddingCache>>,
}

impl EmbeddingManager {
    /// Create a new EmbeddingManager with default configuration (no cache)
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
            default_provider: String::new(),
            cache: None,
        }
    }

    /// Create a new EmbeddingManager with caching enabled
    pub fn with_cache(cache_config: CacheConfig) -> Self {
        Self {
            providers: HashMap::new(),
            default_provider: String::new(),
            cache: Some(Arc::new(EmbeddingCache::with_config(cache_config))),
        }
    }

    /// Enable caching on an existing manager
    pub fn enable_cache(&mut self, cache_config: CacheConfig) {
        self.cache = Some(Arc::new(EmbeddingCache::with_config(cache_config)));
    }

    /// Get cache reference (for stats/clear operations)
    pub fn cache(&self) -> Option<&Arc<EmbeddingCache>> {
        self.cache.as_ref()
    }

    /// Initialize with FastText as default provider (with cache)
    pub fn with_fasttext(model_path: &Path) -> EmbeddingResult<Self> {
        let mut manager = Self::with_cache(CacheConfig::default());

        match FastTextProvider::load(model_path) {
            Ok(provider) => {
                let model_name = provider.model_name().to_string();
                manager.register_provider("fasttext", Arc::new(provider));
                manager.default_provider = "fasttext".to_string();
                log::info!(
                    "FastText provider initialized with model: {} (dim={})",
                    model_name,
                    manager.providers["fasttext"].dimension()
                );
            }
            Err(e) => {
                log::warn!("Failed to load FastText model: {}", e);
            }
        }

        Ok(manager)
    }

    /// Register a provider
    pub fn register_provider(&mut self, name: &str, provider: Arc<dyn EmbeddingProvider>) {
        if self.default_provider.is_empty() {
            self.default_provider = name.to_string();
        }
        self.providers.insert(name.to_string(), provider);
    }

    /// Get the default provider
    pub fn default_provider(&self) -> Option<Arc<dyn EmbeddingProvider>> {
        self.providers.get(&self.default_provider).cloned()
    }

    /// Get a provider by name
    pub fn get_provider(&self, name: &str) -> Option<Arc<dyn EmbeddingProvider>> {
        self.providers.get(name).cloned()
    }

    /// Embed text using specified or default provider (with cache)
    pub fn embed(&self, text: &str, provider: Option<&str>) -> EmbeddingResult<Vec<f32>> {
        let provider_name = provider.unwrap_or(&self.default_provider);
        let provider = self
            .providers
            .get(provider_name)
            .ok_or_else(|| EmbeddingError::ProviderNotFound(provider_name.to_string()))?;

        let model_name = provider.model_name();

        // Check cache first
        if let Some(ref cache) = self.cache {
            if let Some(cached) = cache.get(text, provider_name, model_name) {
                return Ok(cached.vector);
            }
        }

        // Generate embedding
        let vector = provider.embed(text)?;

        // Store in cache
        if let Some(ref cache) = self.cache {
            cache.put(text, provider_name, model_name, vector.clone());
        }

        Ok(vector)
    }

    /// Embed a query text using specified or default provider (with cache)
    ///
    /// Uses query-specific prefix (e.g., "query: " for BGE-M3) for better retrieval.
    pub fn embed_query(&self, text: &str, provider: Option<&str>) -> EmbeddingResult<Vec<f32>> {
        let provider_name = provider.unwrap_or(&self.default_provider);
        let provider = self
            .providers
            .get(provider_name)
            .ok_or_else(|| EmbeddingError::ProviderNotFound(provider_name.to_string()))?;

        let model_name = provider.model_name();

        // Cache key includes "query:" prefix to differentiate from document embeddings
        let cache_key = format!("query:{}", text);

        if let Some(ref cache) = self.cache {
            if let Some(cached) = cache.get(&cache_key, provider_name, model_name) {
                return Ok(cached.vector);
            }
        }

        let vector = provider.embed_query(text)?;

        if let Some(ref cache) = self.cache {
            cache.put(&cache_key, provider_name, model_name, vector.clone());
        }

        Ok(vector)
    }

    /// Embed batch of texts using specified or default provider (with cache)
    ///
    /// Cache lookup is done per-text, and only texts not in cache are embedded.
    /// This provides deduplication within batches automatically.
    pub fn embed_batch(
        &self,
        texts: &[&str],
        provider: Option<&str>,
    ) -> EmbeddingResult<Vec<Vec<f32>>> {
        let provider_name = provider.unwrap_or(&self.default_provider);
        let provider = self
            .providers
            .get(provider_name)
            .ok_or_else(|| EmbeddingError::ProviderNotFound(provider_name.to_string()))?;

        let model_name = provider.model_name();

        // Try to get all from cache first
        if let Some(ref cache) = self.cache {
            let mut results: Vec<Vec<f32>> = Vec::with_capacity(texts.len());
            let mut needs_embedding: Vec<(usize, &str)> = Vec::new();

            for (i, text) in texts.iter().enumerate() {
                if let Some(cached) = cache.get(text, provider_name, model_name) {
                    results.push(cached.vector);
                } else {
                    // Placeholder - will be filled later
                    results.push(Vec::new());
                    needs_embedding.push((i, text));
                }
            }

            // If all cached, return early
            if needs_embedding.is_empty() {
                return Ok(results);
            }

            // Embed only uncached texts
            let uncached_texts: Vec<&str> = needs_embedding.iter().map(|(_, t)| *t).collect();
            let new_embeddings = provider.embed_batch(&uncached_texts)?;

            // Update results and cache
            for ((i, text), embedding) in needs_embedding.iter().zip(new_embeddings.into_iter()) {
                cache.put(text, provider_name, model_name, embedding.clone());
                results[*i] = embedding;
            }

            Ok(results)
        } else {
            // No cache, just embed all
            provider.embed_batch(texts)
        }
    }

    /// List available models
    pub fn list_models(&self) -> Vec<ModelInfo> {
        self.providers
            .iter()
            .map(|(name, provider)| ModelInfo {
                provider: name.clone(),
                model: provider.model_name().to_string(),
                dimension: provider.dimension(),
                description: format!("{} embedding provider", provider.display_name()),
                available: true,
            })
            .collect()
    }

    /// Check if any providers are available
    pub fn has_providers(&self) -> bool {
        !self.providers.is_empty()
    }

    /// Get the default provider name
    pub fn default_provider_name(&self) -> &str {
        &self.default_provider
    }

    /// Set the default provider
    pub fn set_default(&mut self, name: &str) -> bool {
        if self.providers.contains_key(name) {
            self.default_provider = name.to_string();
            true
        } else {
            false
        }
    }
}

/// Create an EmbeddingManager from TOML config
///
/// Supports: "fasttext", "ollama", "vllm", "openai"
/// Falls back to FastText if provider is unknown.
pub fn create_from_config(
    config: &crate::http_server::EmbeddingTomlConfig,
) -> EmbeddingResult<EmbeddingManager> {
    let mut manager = EmbeddingManager::with_cache(CacheConfig::default());

    match config.provider.as_str() {
        "ollama" => {
            let base_url = config
                .base_url
                .as_deref()
                .unwrap_or("http://localhost:11434");
            let model = config.model.as_deref();

            let mut http_config = HttpProviderConfig::ollama_batch(Some(base_url), model);

            // Apply config overrides
            http_config.timeout_secs = config.timeout_secs;
            http_config.max_retries = config.max_retries;
            http_config.retry_base_delay_ms = config.retry_base_delay_ms;
            if let Some(batch_size) = config.batch_size {
                http_config.max_batch_size = batch_size;
            }

            let provider = HttpEmbeddingProvider::new(http_config);
            manager.register_provider("ollama", Arc::new(provider));
            manager.default_provider = "ollama".to_string();

            // Health check (non-blocking)
            check_http_provider_available(base_url, model.unwrap_or("bge-m3"));
        }
        "vllm" => {
            let base_url = config
                .base_url
                .as_deref()
                .unwrap_or("http://localhost:8000");
            let model = config.model.as_deref();

            let mut http_config = HttpProviderConfig::vllm(base_url, model);

            http_config.timeout_secs = config.timeout_secs;
            http_config.max_retries = config.max_retries;
            http_config.retry_base_delay_ms = config.retry_base_delay_ms;
            if let Some(batch_size) = config.batch_size {
                http_config.max_batch_size = batch_size;
            }

            // Apply API key if provided
            if let Some(ref api_key) = config.api_key {
                http_config.auth = http_provider::AuthMethod::Bearer {
                    token: api_key.clone(),
                };
            }

            let provider = HttpEmbeddingProvider::new(http_config);
            manager.register_provider("vllm", Arc::new(provider));
            manager.default_provider = "vllm".to_string();
        }
        "openai" => {
            let api_key = config
                .api_key
                .clone()
                .or_else(|| std::env::var("OPENAI_API_KEY").ok())
                .ok_or_else(|| {
                    EmbeddingError::ConfigError(
                        "OpenAI provider requires api_key in [embedding] or OPENAI_API_KEY env var"
                            .to_string(),
                    )
                })?;
            let model = config.model.as_deref();

            let mut http_config = HttpProviderConfig::openai(&api_key, model);
            // Override preprocessing_version if provided

            http_config.timeout_secs = config.timeout_secs;
            http_config.max_retries = config.max_retries;
            http_config.retry_base_delay_ms = config.retry_base_delay_ms;

            let provider = HttpEmbeddingProvider::new(http_config);
            manager.register_provider("openai", Arc::new(provider));
            manager.default_provider = "openai".to_string();
        }
        "fasttext" => {
            // Use model_path from [embedding] section, or fall back to env var
            let model_path = config
                .model_path
                .clone()
                .or_else(|| std::env::var("IRONBASE_FASTTEXT_MODEL").ok());

            if let Some(path) = model_path {
                if !path.is_empty() {
                    match FastTextProvider::load(Path::new(&path)) {
                        Ok(provider) => {
                            manager.register_provider("fasttext", Arc::new(provider));
                            manager.default_provider = "fasttext".to_string();
                        }
                        Err(e) => {
                            log::warn!("Failed to load FastText model '{}': {}", path, e);
                        }
                    }
                }
            }
        }
        other => {
            return Err(EmbeddingError::ConfigError(format!(
                "Unknown embedding provider '{}'. Supported: fasttext, ollama, vllm, openai",
                other
            )));
        }
    }

    Ok(manager)
}

/// Non-blocking health check for HTTP-based embedding providers.
/// Logs warnings if provider is unavailable but does NOT block startup.
fn check_http_provider_available(base_url: &str, model: &str) {
    match ureq::get(base_url)
        .timeout(std::time::Duration::from_secs(2))
        .call()
    {
        Ok(_) => {
            log::info!("Embedding provider at {} is reachable", base_url);
            // Try a probe embedding to verify model is available
            let probe_url = format!("{}/api/embed", base_url.trim_end_matches('/'));
            match ureq::post(&probe_url)
                .set("Content-Type", "application/json")
                .timeout(std::time::Duration::from_secs(10))
                .send_json(serde_json::json!({
                    "model": model,
                    "input": "test"
                })) {
                Ok(_) => {
                    log::info!("Embedding model '{}' is available", model);
                }
                Err(e) => {
                    log::warn!(
                        "Embedding model '{}' may not be available: {}. Run 'ollama pull {}' if needed.",
                        model, e, model
                    );
                }
            }
        }
        Err(e) => {
            log::warn!(
                "Embedding provider at {} is not reachable: {}. Embeddings will fail until provider is available.",
                base_url, e
            );
        }
    }
}

impl Default for EmbeddingManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockProvider {
        dim: usize,
    }

    impl EmbeddingProvider for MockProvider {
        fn embed(&self, _text: &str) -> EmbeddingResult<Vec<f32>> {
            Ok(vec![0.1; self.dim])
        }

        fn dimension(&self) -> usize {
            self.dim
        }

        fn model_name(&self) -> &str {
            "mock-model"
        }

        fn provider_name(&self) -> &str {
            "mock"
        }
    }

    #[test]
    fn test_manager_register_provider() {
        let mut manager = EmbeddingManager::new();
        manager.register_provider("mock", Arc::new(MockProvider { dim: 100 }));

        assert!(manager.has_providers());
        assert_eq!(manager.default_provider_name(), "mock");
    }

    #[test]
    fn test_manager_embed() {
        let mut manager = EmbeddingManager::new();
        manager.register_provider("mock", Arc::new(MockProvider { dim: 100 }));

        let result = manager.embed("test", None).unwrap();
        assert_eq!(result.len(), 100);
    }

    #[test]
    fn test_manager_list_models() {
        let mut manager = EmbeddingManager::new();
        manager.register_provider("mock1", Arc::new(MockProvider { dim: 100 }));
        manager.register_provider("mock2", Arc::new(MockProvider { dim: 200 }));

        let models = manager.list_models();
        assert_eq!(models.len(), 2);
    }

    #[test]
    fn test_set_default() {
        let mut manager = EmbeddingManager::new();
        manager.register_provider("mock1", Arc::new(MockProvider { dim: 100 }));
        manager.register_provider("mock2", Arc::new(MockProvider { dim: 200 }));

        assert!(manager.set_default("mock2"));
        assert_eq!(manager.default_provider_name(), "mock2");

        assert!(!manager.set_default("nonexistent"));
    }

    #[test]
    fn test_register_http_provider() {
        let mut manager = EmbeddingManager::new();
        let config = HttpProviderConfig::ollama(Some("http://test:11434"), None);
        let provider = Arc::new(HttpEmbeddingProvider::new(config));
        manager.register_provider("ollama", provider);

        assert!(manager.get_provider("ollama").is_some());
    }
}
