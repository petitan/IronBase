//! Built-in provider presets for common embedding APIs
//!
//! These factory methods create pre-configured HttpProviderConfig
//! for popular embedding services. Users can override individual
//! settings as needed.

use super::http_provider::{AuthMethod, HttpProviderConfig, RequestFormat, ResponseFormat};
use serde_json::json;
use std::collections::HashMap;

impl HttpProviderConfig {
    /// Ollama local embedding API
    ///
    /// No authentication required. Ollama must be running locally.
    ///
    /// Models: nomic-embed-text (768), mxbai-embed-large (1024),
    ///         snowflake-arctic-embed (1024), all-minilm (384)
    pub fn ollama(base_url: Option<&str>, model: Option<&str>) -> Self {
        let mut model_dimensions = HashMap::new();
        model_dimensions.insert("nomic-embed-text".to_string(), 768);
        model_dimensions.insert("mxbai-embed-large".to_string(), 1024);
        model_dimensions.insert("snowflake-arctic-embed".to_string(), 1024);
        model_dimensions.insert("all-minilm".to_string(), 384);
        model_dimensions.insert("bge-m3".to_string(), 1024);

        Self {
            name: "ollama".to_string(),
            display_name: Some("Ollama".to_string()),
            base_url: base_url.unwrap_or("http://localhost:11434").to_string(),
            endpoint: Some("/api/embeddings".to_string()),
            default_model: model.unwrap_or("nomic-embed-text").to_string(),
            auth: AuthMethod::None,
            request_format: RequestFormat::SingleText {
                model_field: "model".to_string(),
                text_field: "prompt".to_string(),
            },
            response_format: ResponseFormat::DirectArray {
                path: "embedding".to_string(),
            },
            model_dimensions,
            default_dimension: 768,
            supports_batch: false,
            max_batch_size: 1,
            timeout_secs: 60,
            error_path: Some("error".to_string()),
            extra_body_fields: HashMap::new(),
        }
    }

    /// OpenAI embedding API
    ///
    /// Requires OPENAI_API_KEY environment variable or explicit key.
    ///
    /// Models: text-embedding-ada-002 (1536), text-embedding-3-small (1536),
    ///         text-embedding-3-large (3072)
    pub fn openai(api_key: &str, model: Option<&str>) -> Self {
        let mut model_dimensions = HashMap::new();
        model_dimensions.insert("text-embedding-ada-002".to_string(), 1536);
        model_dimensions.insert("text-embedding-3-small".to_string(), 1536);
        model_dimensions.insert("text-embedding-3-large".to_string(), 3072);

        Self {
            name: "openai".to_string(),
            display_name: Some("OpenAI".to_string()),
            base_url: "https://api.openai.com".to_string(),
            endpoint: Some("/v1/embeddings".to_string()),
            default_model: model.unwrap_or("text-embedding-3-small").to_string(),
            auth: AuthMethod::Bearer {
                token: api_key.to_string(),
            },
            request_format: RequestFormat::TextArray {
                model_field: "model".to_string(),
                texts_field: "input".to_string(),
            },
            response_format: ResponseFormat::ObjectArray {
                array_path: "data".to_string(),
                embedding_field: "embedding".to_string(),
                index_field: Some("index".to_string()),
            },
            model_dimensions,
            default_dimension: 1536,
            supports_batch: true,
            max_batch_size: 2048,
            timeout_secs: 30,
            error_path: Some("error.message".to_string()),
            extra_body_fields: HashMap::new(),
        }
    }

    /// Cohere embedding API
    ///
    /// Requires COHERE_API_KEY environment variable or explicit key.
    ///
    /// Models: embed-english-v3.0 (1024), embed-multilingual-v3.0 (1024),
    ///         embed-english-light-v3.0 (384)
    pub fn cohere(api_key: &str, model: Option<&str>) -> Self {
        let mut model_dimensions = HashMap::new();
        model_dimensions.insert("embed-english-v3.0".to_string(), 1024);
        model_dimensions.insert("embed-multilingual-v3.0".to_string(), 1024);
        model_dimensions.insert("embed-english-light-v3.0".to_string(), 384);

        Self {
            name: "cohere".to_string(),
            display_name: Some("Cohere".to_string()),
            base_url: "https://api.cohere.ai".to_string(),
            endpoint: Some("/v1/embed".to_string()),
            default_model: model.unwrap_or("embed-english-v3.0").to_string(),
            auth: AuthMethod::Bearer {
                token: api_key.to_string(),
            },
            request_format: RequestFormat::TextArray {
                model_field: "model".to_string(),
                texts_field: "texts".to_string(),
            },
            response_format: ResponseFormat::DirectArray {
                path: "embeddings".to_string(),
            },
            model_dimensions,
            default_dimension: 1024,
            supports_batch: true,
            max_batch_size: 96,
            timeout_secs: 30,
            error_path: Some("message".to_string()),
            extra_body_fields: {
                let mut extra = HashMap::new();
                // Cohere v2 API requires input_type
                extra.insert("input_type".to_string(), json!("search_document"));
                extra
            },
        }
    }

    /// Mistral AI embedding API
    ///
    /// Requires MISTRAL_API_KEY environment variable or explicit key.
    ///
    /// Models: mistral-embed (1024)
    pub fn mistral(api_key: &str, model: Option<&str>) -> Self {
        let mut model_dimensions = HashMap::new();
        model_dimensions.insert("mistral-embed".to_string(), 1024);

        Self {
            name: "mistral".to_string(),
            display_name: Some("Mistral AI".to_string()),
            base_url: "https://api.mistral.ai".to_string(),
            endpoint: Some("/v1/embeddings".to_string()),
            default_model: model.unwrap_or("mistral-embed").to_string(),
            auth: AuthMethod::Bearer {
                token: api_key.to_string(),
            },
            request_format: RequestFormat::TextArray {
                model_field: "model".to_string(),
                texts_field: "input".to_string(),
            },
            response_format: ResponseFormat::ObjectArray {
                array_path: "data".to_string(),
                embedding_field: "embedding".to_string(),
                index_field: Some("index".to_string()),
            },
            model_dimensions,
            default_dimension: 1024,
            supports_batch: true,
            max_batch_size: 100,
            timeout_secs: 30,
            error_path: Some("message".to_string()),
            extra_body_fields: HashMap::new(),
        }
    }

    /// Azure OpenAI embedding API
    ///
    /// Requires Azure endpoint, API key, and deployment name.
    ///
    /// Models: depends on deployment
    pub fn azure_openai(endpoint: &str, api_key: &str, deployment: &str) -> Self {
        Self {
            name: "azure-openai".to_string(),
            display_name: Some("Azure OpenAI".to_string()),
            base_url: endpoint.to_string(),
            endpoint: Some(format!(
                "/openai/deployments/{}/embeddings?api-version=2024-02-01",
                deployment
            )),
            default_model: deployment.to_string(),
            auth: AuthMethod::Header {
                name: "api-key".to_string(),
                value: api_key.to_string(),
            },
            request_format: RequestFormat::TextArray {
                model_field: "model".to_string(),
                texts_field: "input".to_string(),
            },
            response_format: ResponseFormat::ObjectArray {
                array_path: "data".to_string(),
                embedding_field: "embedding".to_string(),
                index_field: Some("index".to_string()),
            },
            model_dimensions: HashMap::new(),
            default_dimension: 1536,
            supports_batch: true,
            max_batch_size: 16,
            timeout_secs: 30,
            error_path: Some("error.message".to_string()),
            extra_body_fields: HashMap::new(),
        }
    }

    /// HuggingFace Inference API
    ///
    /// Requires HUGGINGFACE_API_KEY environment variable or explicit key.
    /// Model must be specified explicitly.
    pub fn huggingface(api_key: &str, model: &str) -> Self {
        Self {
            name: "huggingface".to_string(),
            display_name: Some("HuggingFace".to_string()),
            base_url: format!(
                "https://api-inference.huggingface.co/pipeline/feature-extraction/{}",
                model
            ),
            endpoint: None,
            default_model: model.to_string(),
            auth: AuthMethod::Bearer {
                token: api_key.to_string(),
            },
            request_format: RequestFormat::Template {
                template: r#"{"inputs": "{text}"}"#.to_string(),
            },
            response_format: ResponseFormat::DirectArray {
                path: "".to_string(), // Root is the array
            },
            model_dimensions: HashMap::new(),
            default_dimension: 0, // Auto-detect
            supports_batch: false,
            max_batch_size: 1,
            timeout_secs: 60,
            error_path: Some("error".to_string()),
            extra_body_fields: HashMap::new(),
        }
    }

    /// Voyage AI embedding API
    ///
    /// Requires VOYAGE_API_KEY environment variable or explicit key.
    ///
    /// Models: voyage-large-2 (1024), voyage-code-2 (1536)
    pub fn voyage(api_key: &str, model: Option<&str>) -> Self {
        let mut model_dimensions = HashMap::new();
        model_dimensions.insert("voyage-large-2".to_string(), 1024);
        model_dimensions.insert("voyage-code-2".to_string(), 1536);
        model_dimensions.insert("voyage-2".to_string(), 1024);

        Self {
            name: "voyage".to_string(),
            display_name: Some("Voyage AI".to_string()),
            base_url: "https://api.voyageai.com".to_string(),
            endpoint: Some("/v1/embeddings".to_string()),
            default_model: model.unwrap_or("voyage-2").to_string(),
            auth: AuthMethod::Bearer {
                token: api_key.to_string(),
            },
            request_format: RequestFormat::TextArray {
                model_field: "model".to_string(),
                texts_field: "input".to_string(),
            },
            response_format: ResponseFormat::ObjectArray {
                array_path: "data".to_string(),
                embedding_field: "embedding".to_string(),
                index_field: Some("index".to_string()),
            },
            model_dimensions,
            default_dimension: 1024,
            supports_batch: true,
            max_batch_size: 128,
            timeout_secs: 30,
            error_path: Some("detail".to_string()),
            extra_body_fields: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ollama_preset() {
        let config = HttpProviderConfig::ollama(None, None);
        assert_eq!(config.name, "ollama");
        assert_eq!(config.base_url, "http://localhost:11434");
        assert_eq!(config.default_model, "nomic-embed-text");
        assert!(matches!(config.auth, AuthMethod::None));
        assert!(!config.supports_batch);
    }

    #[test]
    fn test_ollama_custom_url() {
        let config = HttpProviderConfig::ollama(Some("http://192.168.1.100:11434"), None);
        assert_eq!(config.base_url, "http://192.168.1.100:11434");
    }

    #[test]
    fn test_openai_preset() {
        let config = HttpProviderConfig::openai("sk-test", None);
        assert_eq!(config.name, "openai");
        assert!(config.supports_batch);
        assert_eq!(config.max_batch_size, 2048);
        assert!(matches!(config.auth, AuthMethod::Bearer { .. }));
    }

    #[test]
    fn test_cohere_preset() {
        let config = HttpProviderConfig::cohere("test-key", Some("embed-multilingual-v3.0"));
        assert_eq!(config.default_model, "embed-multilingual-v3.0");
        assert_eq!(config.default_dimension, 1024);
    }

    #[test]
    fn test_azure_openai_preset() {
        let config = HttpProviderConfig::azure_openai(
            "https://myresource.openai.azure.com",
            "my-api-key",
            "my-deployment",
        );
        assert_eq!(config.name, "azure-openai");
        assert!(matches!(config.auth, AuthMethod::Header { .. }));
    }

    #[test]
    fn test_model_dimensions_lookup() {
        let config = HttpProviderConfig::openai("sk-test", None);
        assert_eq!(
            config.model_dimensions.get("text-embedding-3-large"),
            Some(&3072)
        );
    }
}
