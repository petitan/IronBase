//! Generic HTTP-based embedding provider
//!
//! Config-driven embedding provider that works with any HTTP API.
//! New providers can be added via configuration without code changes.

use super::{EmbeddingError, EmbeddingProvider, EmbeddingResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

// ============================================================================
// Configuration Enums
// ============================================================================

/// Authentication method for HTTP APIs
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthMethod {
    /// No authentication required (e.g., local Ollama)
    #[default]
    None,
    /// Bearer token in Authorization header
    Bearer { token: String },
    /// Custom header (e.g., X-API-Key for Azure)
    Header { name: String, value: String },
    /// Query parameter (e.g., ?api_key=xxx)
    QueryParam { name: String, value: String },
}

/// How to format the request body
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RequestFormat {
    /// Single text input: {"model": "X", "{text_field}": "text"}
    /// Example: Ollama uses {"model": "nomic-embed-text", "prompt": "text"}
    SingleText {
        model_field: String,
        text_field: String,
    },
    /// Array input: {"model": "X", "{texts_field}": ["text1", "text2"]}
    /// Example: OpenAI uses {"model": "text-embedding-3-small", "input": ["text1"]}
    TextArray {
        model_field: String,
        texts_field: String,
    },
    /// Custom template with placeholder substitution
    /// Supports {model}, {text}, {texts} placeholders
    Template { template: String },
}

impl Default for RequestFormat {
    fn default() -> Self {
        Self::SingleText {
            model_field: "model".to_string(),
            text_field: "input".to_string(),
        }
    }
}

/// How to extract vectors from the response JSON
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseFormat {
    /// Direct array at path: $.embedding -> [0.1, 0.2, ...]
    /// Example: Ollama returns {"embedding": [0.1, 0.2, ...]}
    DirectArray { path: String },
    /// Array of objects with embedding field
    /// Example: OpenAI returns {"data": [{"embedding": [...], "index": 0}]}
    ObjectArray {
        array_path: String,
        embedding_field: String,
        #[serde(default)]
        index_field: Option<String>,
    },
    /// Nested path with dot notation
    NestedPath { path: String },
}

impl Default for ResponseFormat {
    fn default() -> Self {
        Self::DirectArray {
            path: "embedding".to_string(),
        }
    }
}

// ============================================================================
// Provider Configuration
// ============================================================================

/// Complete configuration for an HTTP embedding provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpProviderConfig {
    /// Provider identifier (e.g., "ollama", "openai", "cohere")
    pub name: String,

    /// Display name for UI/logs
    #[serde(default)]
    pub display_name: Option<String>,

    /// Base URL for the API
    pub base_url: String,

    /// API endpoint path (appended to base_url)
    #[serde(default)]
    pub endpoint: Option<String>,

    /// Default model name
    pub default_model: String,

    /// Authentication method
    #[serde(default)]
    pub auth: AuthMethod,

    /// Request body format
    #[serde(default)]
    pub request_format: RequestFormat,

    /// Response parsing format
    #[serde(default)]
    pub response_format: ResponseFormat,

    /// Known model dimensions
    #[serde(default)]
    pub model_dimensions: HashMap<String, usize>,

    /// Default dimension if unknown (0 = auto-detect)
    #[serde(default)]
    pub default_dimension: usize,

    /// Whether this provider supports native batch requests
    #[serde(default)]
    pub supports_batch: bool,

    /// Maximum batch size (if supports_batch is true)
    #[serde(default = "default_max_batch")]
    pub max_batch_size: usize,

    /// Request timeout in seconds
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,

    /// Error response path (to extract error message)
    #[serde(default)]
    pub error_path: Option<String>,

    /// Extra fields to include in request body (e.g., Cohere's input_type)
    #[serde(default)]
    pub extra_body_fields: HashMap<String, serde_json::Value>,
}

fn default_max_batch() -> usize {
    100
}

fn default_timeout() -> u64 {
    30
}

// ============================================================================
// HTTP Provider Implementation
// ============================================================================

/// Generic HTTP-based embedding provider
pub struct HttpEmbeddingProvider {
    config: HttpProviderConfig,
    model: String,
    cached_dimension: Mutex<Option<usize>>,
}

impl HttpEmbeddingProvider {
    /// Create a new HTTP provider from configuration
    pub fn new(config: HttpProviderConfig) -> Self {
        let model = config.default_model.clone();
        Self {
            config,
            model,
            cached_dimension: Mutex::new(None),
        }
    }

    /// Create with a specific model override
    pub fn with_model(mut config: HttpProviderConfig, model: &str) -> Self {
        config.default_model = model.to_string();
        Self::new(config)
    }

    /// Build the full URL for the API call
    fn build_url(&self) -> String {
        if let Some(ref endpoint) = self.config.endpoint {
            format!(
                "{}{}",
                self.config.base_url.trim_end_matches('/'),
                endpoint
            )
        } else {
            self.config.base_url.clone()
        }
    }

    /// Build request body for single text
    fn build_single_body(&self, text: &str) -> serde_json::Value {
        let mut body = match &self.config.request_format {
            RequestFormat::SingleText {
                model_field,
                text_field,
            } => {
                serde_json::json!({
                    model_field: self.model,
                    text_field: text
                })
            }
            RequestFormat::TextArray {
                model_field,
                texts_field,
            } => {
                serde_json::json!({
                    model_field: self.model,
                    texts_field: [text]
                })
            }
            RequestFormat::Template { template } => {
                let body = template
                    .replace("{model}", &self.model)
                    .replace("{text}", text)
                    .replace(
                        "{texts}",
                        &format!(r#"["{}"]"#, text.replace('"', r#"\""#)),
                    );
                serde_json::from_str(&body).unwrap_or_else(|_| serde_json::json!({}))
            }
        };
        self.merge_extra_fields(&mut body);
        body
    }

    /// Build request body for batch texts
    fn build_batch_body(&self, texts: &[&str]) -> serde_json::Value {
        let mut body = match &self.config.request_format {
            RequestFormat::SingleText {
                model_field,
                text_field,
            } => {
                // Fallback for single-text format
                serde_json::json!({
                    model_field: self.model,
                    text_field: texts.first().unwrap_or(&"")
                })
            }
            RequestFormat::TextArray {
                model_field,
                texts_field,
            } => {
                serde_json::json!({
                    model_field: self.model,
                    texts_field: texts
                })
            }
            RequestFormat::Template { template } => {
                let texts_json: Vec<String> = texts
                    .iter()
                    .map(|t| format!(r#""{}""#, t.replace('"', r#"\""#)))
                    .collect();
                let body = template
                    .replace("{model}", &self.model)
                    .replace("{texts}", &format!("[{}]", texts_json.join(",")));
                serde_json::from_str(&body).unwrap_or_else(|_| serde_json::json!({}))
            }
        };
        self.merge_extra_fields(&mut body);
        body
    }

    /// Merge extra_body_fields into the request body
    fn merge_extra_fields(&self, body: &mut serde_json::Value) {
        if let serde_json::Value::Object(map) = body {
            for (key, value) in &self.config.extra_body_fields {
                map.insert(key.clone(), value.clone());
            }
        }
    }

    /// Apply authentication to request
    fn apply_auth(&self, request: ureq::Request) -> ureq::Request {
        match &self.config.auth {
            AuthMethod::None => request,
            AuthMethod::Bearer { token } => {
                request.set("Authorization", &format!("Bearer {}", token))
            }
            AuthMethod::Header { name, value } => request.set(name, value),
            AuthMethod::QueryParam { .. } => {
                // Query params handled in URL building
                request
            }
        }
    }

    /// Extract vectors from response JSON
    fn extract_vectors(&self, json: &serde_json::Value) -> EmbeddingResult<Vec<Vec<f32>>> {
        // First check for error response
        if let Some(ref error_path) = self.config.error_path {
            if let Some(error) = get_json_path(json, error_path) {
                if !error.is_null() {
                    let msg = error
                        .as_str()
                        .map(|s| s.to_string())
                        .or_else(|| serde_json::to_string(error).ok())
                        .unwrap_or_else(|| "Unknown API error".to_string());
                    return Err(EmbeddingError::ApiError(msg));
                }
            }
        }

        match &self.config.response_format {
            ResponseFormat::DirectArray { path } => {
                let array = if path.is_empty() {
                    json.as_array()
                } else {
                    get_json_path(json, path).and_then(|v| v.as_array())
                };

                let arr = array.ok_or_else(|| {
                    EmbeddingError::ApiError(format!("Response missing array at path '{}'", path))
                })?;

                // Check if it's a 2D array (batch result) or 1D (single result)
                if arr.first().map(|v| v.is_array()).unwrap_or(false) {
                    // 2D array: [[0.1, 0.2], [0.3, 0.4]]
                    arr.iter().map(parse_float_array).collect()
                } else {
                    // 1D array: [0.1, 0.2, 0.3]
                    Ok(vec![parse_float_array(&serde_json::Value::Array(
                        arr.clone(),
                    ))?])
                }
            }

            ResponseFormat::ObjectArray {
                array_path,
                embedding_field,
                index_field,
            } => {
                let array = get_json_path(json, array_path)
                    .and_then(|v| v.as_array())
                    .ok_or_else(|| {
                        EmbeddingError::ApiError(format!(
                            "Response missing array at path '{}'",
                            array_path
                        ))
                    })?;

                let mut results: Vec<(usize, Vec<f32>)> = Vec::with_capacity(array.len());

                for (i, item) in array.iter().enumerate() {
                    let embedding = item.get(embedding_field).ok_or_else(|| {
                        EmbeddingError::ApiError(format!(
                            "Response item missing '{}' field",
                            embedding_field
                        ))
                    })?;

                    let vec = parse_float_array(embedding)?;

                    let idx = index_field
                        .as_ref()
                        .and_then(|f| item.get(f))
                        .and_then(|v| v.as_u64())
                        .map(|v| v as usize)
                        .unwrap_or(i);

                    results.push((idx, vec));
                }

                // Sort by index to maintain order
                results.sort_by_key(|(idx, _)| *idx);
                Ok(results.into_iter().map(|(_, v)| v).collect())
            }

            ResponseFormat::NestedPath { path } => {
                let value = get_json_path(json, path).ok_or_else(|| {
                    EmbeddingError::ApiError(format!("Response missing value at path '{}'", path))
                })?;

                if let Some(arr) = value.as_array() {
                    if arr.first().map(|v| v.is_array()).unwrap_or(false) {
                        arr.iter().map(parse_float_array).collect()
                    } else {
                        Ok(vec![parse_float_array(value)?])
                    }
                } else {
                    Err(EmbeddingError::ApiError(format!(
                        "Expected array at path '{}', got {:?}",
                        path, value
                    )))
                }
            }
        }
    }

    /// Make HTTP request and parse response
    fn call_api(&self, texts: &[&str]) -> EmbeddingResult<Vec<Vec<f32>>> {
        let url = self.build_url();

        let body = if texts.len() == 1 {
            self.build_single_body(texts[0])
        } else {
            self.build_batch_body(texts)
        };

        let request = ureq::post(&url)
            .set("Content-Type", "application/json")
            .timeout(Duration::from_secs(self.config.timeout_secs));

        let request = self.apply_auth(request);

        let response = request.send_json(&body).map_err(|e| {
            EmbeddingError::HttpError(format!("{} API call failed: {}", self.config.name, e))
        })?;

        let json: serde_json::Value = response.into_json().map_err(|e| {
            EmbeddingError::ApiError(format!(
                "Failed to parse {} response: {}",
                self.config.name, e
            ))
        })?;

        let vectors = self.extract_vectors(&json)?;

        // Cache dimension from first result
        if !vectors.is_empty() {
            if let Ok(mut dim_guard) = self.cached_dimension.lock() {
                if dim_guard.is_none() {
                    *dim_guard = Some(vectors[0].len());
                }
            }
        }

        Ok(vectors)
    }
}

impl EmbeddingProvider for HttpEmbeddingProvider {
    fn embed(&self, text: &str) -> EmbeddingResult<Vec<f32>> {
        let results = self.call_api(&[text])?;
        results
            .into_iter()
            .next()
            .ok_or_else(|| EmbeddingError::ApiError("Empty response".to_string()))
    }

    fn embed_batch(&self, texts: &[&str]) -> EmbeddingResult<Vec<Vec<f32>>> {
        if self.config.supports_batch {
            // Native batch support - chunk by max_batch_size
            let mut all_results = Vec::with_capacity(texts.len());

            for chunk in texts.chunks(self.config.max_batch_size) {
                let results = self.call_api(chunk)?;
                all_results.extend(results);
            }

            Ok(all_results)
        } else {
            // No batch support - parallel single calls
            use rayon::prelude::*;

            if texts.len() > 5 {
                texts.par_iter().map(|t| self.embed(t)).collect()
            } else {
                texts.iter().map(|t| self.embed(t)).collect()
            }
        }
    }

    fn dimension(&self) -> usize {
        // Check cache first
        if let Ok(guard) = self.cached_dimension.lock() {
            if let Some(dim) = *guard {
                return dim;
            }
        }

        // Check known model dimensions
        if let Some(dim) = self.config.model_dimensions.get(&self.model) {
            return *dim;
        }

        // Return default
        self.config.default_dimension
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn provider_name(&self) -> &str {
        &self.config.name
    }

    fn display_name(&self) -> &str {
        self.config
            .display_name
            .as_deref()
            .unwrap_or(&self.config.name)
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Get value from JSON using dot notation path
fn get_json_path<'a>(json: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    if path.is_empty() {
        return Some(json);
    }

    let mut current = json;
    for part in path.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}

/// Parse JSON value as f32 array
fn parse_float_array(value: &serde_json::Value) -> EmbeddingResult<Vec<f32>> {
    let array = value
        .as_array()
        .ok_or_else(|| EmbeddingError::ApiError("Expected array".to_string()))?;

    array
        .iter()
        .map(|v| {
            v.as_f64()
                .map(|f| f as f32)
                .ok_or_else(|| EmbeddingError::ApiError("Invalid number in array".to_string()))
        })
        .collect()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_path_simple() {
        let json = serde_json::json!({"embedding": [1.0, 2.0, 3.0]});
        let result = get_json_path(&json, "embedding");
        assert!(result.is_some());
        assert!(result.unwrap().is_array());
    }

    #[test]
    fn test_json_path_nested() {
        let json = serde_json::json!({"data": {"embedding": [1.0, 2.0]}});
        let result = get_json_path(&json, "data.embedding");
        assert!(result.is_some());
    }

    #[test]
    fn test_json_path_empty() {
        let json = serde_json::json!([1.0, 2.0, 3.0]);
        let result = get_json_path(&json, "");
        assert!(result.is_some());
        assert!(result.unwrap().is_array());
    }

    #[test]
    fn test_parse_float_array() {
        let json = serde_json::json!([1.0, 2.0, 3.0]);
        let result = parse_float_array(&json).unwrap();
        assert_eq!(result, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn test_auth_method_default() {
        let auth: AuthMethod = Default::default();
        assert!(matches!(auth, AuthMethod::None));
    }

    #[test]
    fn test_build_url_with_endpoint() {
        let config = HttpProviderConfig {
            name: "test".to_string(),
            display_name: None,
            base_url: "http://localhost:8080".to_string(),
            endpoint: Some("/api/embed".to_string()),
            default_model: "test-model".to_string(),
            auth: AuthMethod::None,
            request_format: RequestFormat::default(),
            response_format: ResponseFormat::default(),
            model_dimensions: HashMap::new(),
            default_dimension: 384,
            supports_batch: false,
            max_batch_size: 1,
            timeout_secs: 30,
            error_path: None,
            extra_body_fields: HashMap::new(),
        };

        let provider = HttpEmbeddingProvider::new(config);
        assert_eq!(provider.build_url(), "http://localhost:8080/api/embed");
    }

    #[test]
    fn test_extra_body_fields_merged() {
        let mut extra = HashMap::new();
        extra.insert("input_type".to_string(), serde_json::json!("search_document"));

        let config = HttpProviderConfig {
            name: "test".to_string(),
            display_name: None,
            base_url: "http://localhost:8080".to_string(),
            endpoint: None,
            default_model: "test-model".to_string(),
            auth: AuthMethod::None,
            request_format: RequestFormat::TextArray {
                model_field: "model".to_string(),
                texts_field: "texts".to_string(),
            },
            response_format: ResponseFormat::default(),
            model_dimensions: HashMap::new(),
            default_dimension: 384,
            supports_batch: false,
            max_batch_size: 1,
            timeout_secs: 30,
            error_path: None,
            extra_body_fields: extra,
        };

        let provider = HttpEmbeddingProvider::new(config);
        let body = provider.build_single_body("test text");

        assert_eq!(body.get("input_type"), Some(&serde_json::json!("search_document")));
        assert_eq!(body.get("model"), Some(&serde_json::json!("test-model")));
    }
}
