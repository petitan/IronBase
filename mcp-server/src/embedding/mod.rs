//! Embedding generation module for MCP server
//!
//! Supports multiple providers:
//! - FastText (default, offline, Hungarian support)
//! - Any HTTP API via config-driven HttpEmbeddingProvider
//!   - Ollama (local LLM server)
//!   - OpenAI (cloud API)
//!   - Cohere (cloud API)
//!   - Mistral, Azure OpenAI, HuggingFace, etc.

mod fasttext;
mod http_provider;
mod presets;

pub use fasttext::FastTextProvider;
pub use http_provider::{AuthMethod, HttpEmbeddingProvider, HttpProviderConfig, RequestFormat, ResponseFormat};

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

/// Manager for embedding providers
pub struct EmbeddingManager {
    providers: HashMap<String, Arc<dyn EmbeddingProvider>>,
    default_provider: String,
}

impl EmbeddingManager {
    /// Create a new EmbeddingManager with default configuration
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
            default_provider: String::new(),
        }
    }

    /// Initialize with FastText as default provider
    pub fn with_fasttext(model_path: &Path) -> EmbeddingResult<Self> {
        let mut manager = Self::new();

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

    /// Initialize with auto-detection of available providers
    ///
    /// Checks environment variables for API keys and local services.
    /// FastText is used as default if IRONBASE_FASTTEXT_MODEL is set.
    pub fn auto_detect() -> Self {
        let mut manager = Self::new();

        // 1. Try FastText first (highest priority as default)
        if let Ok(model_path) = std::env::var("IRONBASE_FASTTEXT_MODEL") {
            if let Ok(provider) = FastTextProvider::load(Path::new(&model_path)) {
                log::info!("FastText provider auto-detected: {}", model_path);
                manager.register_provider("fasttext", Arc::new(provider));
            }
        }

        // 2. Try Ollama (local, no key needed)
        if Self::check_ollama_available("http://localhost:11434") {
            let config = HttpProviderConfig::ollama(None, None);
            manager.register_provider("ollama", Arc::new(HttpEmbeddingProvider::new(config)));
            log::info!("Ollama provider auto-detected");
        }

        // 3. OpenAI (if key present)
        if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
            let config = HttpProviderConfig::openai(&api_key, None);
            manager.register_provider("openai", Arc::new(HttpEmbeddingProvider::new(config)));
            log::info!("OpenAI provider auto-detected from OPENAI_API_KEY");
        }

        // 4. Cohere (if key present)
        if let Ok(api_key) = std::env::var("COHERE_API_KEY") {
            let config = HttpProviderConfig::cohere(&api_key, None);
            manager.register_provider("cohere", Arc::new(HttpEmbeddingProvider::new(config)));
            log::info!("Cohere provider auto-detected from COHERE_API_KEY");
        }

        // 5. Mistral (if key present)
        if let Ok(api_key) = std::env::var("MISTRAL_API_KEY") {
            let config = HttpProviderConfig::mistral(&api_key, None);
            manager.register_provider("mistral", Arc::new(HttpEmbeddingProvider::new(config)));
            log::info!("Mistral provider auto-detected from MISTRAL_API_KEY");
        }

        // 6. Azure OpenAI (if all required vars present)
        if let (Ok(endpoint), Ok(api_key), Ok(deployment)) = (
            std::env::var("AZURE_OPENAI_ENDPOINT"),
            std::env::var("AZURE_OPENAI_API_KEY"),
            std::env::var("AZURE_OPENAI_DEPLOYMENT"),
        ) {
            let config = HttpProviderConfig::azure_openai(&endpoint, &api_key, &deployment);
            manager.register_provider("azure-openai", Arc::new(HttpEmbeddingProvider::new(config)));
            log::info!("Azure OpenAI provider auto-detected");
        }

        // 7. Voyage AI (if key present)
        if let Ok(api_key) = std::env::var("VOYAGE_API_KEY") {
            let config = HttpProviderConfig::voyage(&api_key, None);
            manager.register_provider("voyage", Arc::new(HttpEmbeddingProvider::new(config)));
            log::info!("Voyage AI provider auto-detected from VOYAGE_API_KEY");
        }

        manager
    }

    /// Check if Ollama is available at the given URL
    fn check_ollama_available(base_url: &str) -> bool {
        ureq::get(base_url)
            .timeout(std::time::Duration::from_secs(1))
            .call()
            .is_ok()
    }

    /// Register a provider
    pub fn register_provider(&mut self, name: &str, provider: Arc<dyn EmbeddingProvider>) {
        if self.default_provider.is_empty() {
            self.default_provider = name.to_string();
        }
        self.providers.insert(name.to_string(), provider);
    }

    /// Add Ollama provider (convenience method)
    pub fn add_ollama(&mut self, base_url: Option<&str>, model: Option<&str>) {
        let config = HttpProviderConfig::ollama(base_url, model);
        self.register_provider("ollama", Arc::new(HttpEmbeddingProvider::new(config)));
    }

    /// Add OpenAI provider (convenience method)
    pub fn add_openai(&mut self, api_key: &str, model: Option<&str>) {
        let config = HttpProviderConfig::openai(api_key, model);
        self.register_provider("openai", Arc::new(HttpEmbeddingProvider::new(config)));
    }

    /// Add Cohere provider (convenience method)
    pub fn add_cohere(&mut self, api_key: &str, model: Option<&str>) {
        let config = HttpProviderConfig::cohere(api_key, model);
        self.register_provider("cohere", Arc::new(HttpEmbeddingProvider::new(config)));
    }

    /// Add a custom HTTP provider from config
    pub fn add_http_provider(&mut self, config: HttpProviderConfig) {
        let name = config.name.clone();
        self.register_provider(&name, Arc::new(HttpEmbeddingProvider::new(config)));
    }

    /// Get the default provider
    pub fn default_provider(&self) -> Option<Arc<dyn EmbeddingProvider>> {
        self.providers.get(&self.default_provider).cloned()
    }

    /// Get a provider by name
    pub fn get_provider(&self, name: &str) -> Option<Arc<dyn EmbeddingProvider>> {
        self.providers.get(name).cloned()
    }

    /// Embed text using specified or default provider
    pub fn embed(&self, text: &str, provider: Option<&str>) -> EmbeddingResult<Vec<f32>> {
        let provider_name = provider.unwrap_or(&self.default_provider);
        let provider = self
            .providers
            .get(provider_name)
            .ok_or_else(|| EmbeddingError::ProviderNotFound(provider_name.to_string()))?;

        provider.embed(text)
    }

    /// Embed batch of texts using specified or default provider
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

        provider.embed_batch(texts)
    }

    /// List available models
    pub fn list_models(&self) -> Vec<ModelInfo> {
        self.providers
            .iter()
            .map(|(name, provider)| ModelInfo {
                provider: name.clone(),
                model: provider.model_name().to_string(),
                dimension: provider.dimension(),
                description: format!("{} provider", provider.provider_name()),
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
    fn test_add_http_provider() {
        let mut manager = EmbeddingManager::new();
        let config = HttpProviderConfig::ollama(Some("http://test:11434"), None);
        manager.add_http_provider(config);

        assert!(manager.get_provider("ollama").is_some());
    }
}
