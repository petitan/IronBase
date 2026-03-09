//! Vector search state

use serde_json::Value;

/// Distance metric for vector search
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DistanceMetric {
    #[default]
    Cosine,
    Euclidean,
    DotProduct,
}

impl DistanceMetric {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cosine => "cosine",
            Self::Euclidean => "euclidean",
            Self::DotProduct => "dot_product",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Self::Cosine => Self::Euclidean,
            Self::Euclidean => Self::DotProduct,
            Self::DotProduct => Self::Cosine,
        }
    }
}

/// Active tab in vector modal
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VectorTab {
    #[default]
    Search,
    CreateIndex,
    ListIndexes,
}

impl VectorTab {
    pub fn next(&self) -> Self {
        match self {
            Self::Search => Self::CreateIndex,
            Self::CreateIndex => Self::ListIndexes,
            Self::ListIndexes => Self::Search,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            Self::Search => Self::ListIndexes,
            Self::CreateIndex => Self::Search,
            Self::ListIndexes => Self::CreateIndex,
        }
    }
}

/// Vector index information
#[derive(Debug, Clone)]
pub struct VectorIndexInfo {
    pub name: String,
    pub field: String,
    pub dimension: usize,
    pub metric: String,
    pub vector_count: usize,
}

impl VectorIndexInfo {
    pub fn from_value(v: &Value) -> Self {
        Self {
            name: v
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            field: v
                .get("field")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            dimension: v.get("dimension").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
            metric: v
                .get("metric")
                .and_then(|v| v.as_str())
                .unwrap_or("cosine")
                .to_string(),
            vector_count: v.get("vector_count").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
        }
    }
}

/// Vector search result
#[derive(Debug, Clone)]
pub struct VectorSearchResult {
    pub document: Value,
    pub distance: f64,
}

/// Vector search state
#[derive(Debug, Clone, Default)]
pub struct VectorSearchState {
    /// Current collection
    pub collection: String,

    /// Active tab
    pub active_tab: VectorTab,

    // === Search tab ===
    /// Field to search on
    pub search_field: String,
    /// Vector input (JSON array string)
    pub vector_input: String,
    /// Maximum results
    pub k: usize,
    /// Optional filter query (JSON string)
    pub filter_input: String,
    /// Search results
    pub results: Vec<VectorSearchResult>,
    /// Selected result index
    pub selected_result: usize,

    // === Create Index tab ===
    /// Field name for new index
    pub new_field: String,
    /// Dimension (0 = auto-detect)
    pub dimension: usize,
    /// Distance metric
    pub metric: DistanceMetric,
    /// Form field focus (0=field, 1=dimension, 2=metric)
    pub create_form_field: usize,

    // === List Indexes tab ===
    /// List of vector indexes
    pub indexes: Vec<VectorIndexInfo>,
    /// Selected index in list
    pub selected_index: usize,

    // === Common ===
    pub error: Option<String>,
    pub message: Option<String>,
    pub is_loading: bool,
}

impl VectorSearchState {
    pub fn new(collection: String) -> Self {
        Self {
            collection,
            active_tab: VectorTab::Search,
            search_field: String::new(),
            vector_input: String::new(),
            k: 10,
            filter_input: String::new(),
            results: Vec::new(),
            selected_result: 0,
            new_field: String::new(),
            dimension: 0,
            metric: DistanceMetric::Cosine,
            create_form_field: 0,
            indexes: Vec::new(),
            selected_index: 0,
            error: None,
            message: None,
            is_loading: false,
        }
    }

    /// Update indexes from MCP response
    pub fn update_indexes(&mut self, indexes: Vec<Value>) {
        self.indexes = indexes.iter().map(VectorIndexInfo::from_value).collect();
        self.is_loading = false;
    }

    /// Parse vector input as JSON array of f64
    pub fn parse_vector(&self) -> Result<Vec<f64>, String> {
        let trimmed = self.vector_input.trim();
        if trimmed.is_empty() {
            return Err("Vector input is empty".to_string());
        }

        // Try to parse as JSON array
        match serde_json::from_str::<Vec<f64>>(trimmed) {
            Ok(v) => {
                if v.is_empty() {
                    Err("Vector is empty".to_string())
                } else {
                    Ok(v)
                }
            }
            Err(e) => Err(format!("Invalid vector JSON: {}", e)),
        }
    }

    /// Parse filter input as JSON value
    pub fn parse_filter(&self) -> Result<Option<Value>, String> {
        let trimmed = self.filter_input.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }

        match serde_json::from_str::<Value>(trimmed) {
            Ok(v) => Ok(Some(v)),
            Err(e) => Err(format!("Invalid filter JSON: {}", e)),
        }
    }

    /// Update search results
    pub fn update_results(&mut self, results: Vec<Value>) {
        self.results = results
            .into_iter()
            .map(|v| VectorSearchResult {
                document: v.get("document").cloned().unwrap_or(Value::Null),
                distance: v.get("distance").and_then(|d| d.as_f64()).unwrap_or(0.0),
            })
            .collect();
        self.selected_result = 0;
        self.is_loading = false;
    }

    // === Navigation ===

    pub fn next_tab(&mut self) {
        self.active_tab = self.active_tab.next();
    }

    pub fn prev_tab(&mut self) {
        self.active_tab = self.active_tab.prev();
    }

    pub fn result_up(&mut self) {
        if self.selected_result > 0 {
            self.selected_result -= 1;
        }
    }

    pub fn result_down(&mut self) {
        if self.selected_result + 1 < self.results.len() {
            self.selected_result += 1;
        }
    }

    pub fn index_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    pub fn index_down(&mut self) {
        if self.selected_index + 1 < self.indexes.len() {
            self.selected_index += 1;
        }
    }

    pub fn next_create_field(&mut self) {
        self.create_form_field = (self.create_form_field + 1) % 3;
    }

    pub fn toggle_metric(&mut self) {
        self.metric = self.metric.next();
    }

    /// Clear search inputs
    pub fn clear_search(&mut self) {
        self.vector_input.clear();
        self.filter_input.clear();
        self.results.clear();
        self.selected_result = 0;
        self.error = None;
        self.message = None;
    }

    /// Clear create form
    pub fn clear_create_form(&mut self) {
        self.new_field.clear();
        self.dimension = 0;
        self.metric = DistanceMetric::Cosine;
        self.create_form_field = 0;
        self.error = None;
        self.message = None;
    }
}
