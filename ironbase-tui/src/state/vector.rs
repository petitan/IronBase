//! Vector search state

use crate::i18n::t;
use crate::widgets::{push_digits, Input, InputOutcome, TextInput};
use ironbase_mcp_types::responses::{VectorIndexInfo as IndexInfo, VectorMetric, VectorSearchHit};
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

/// The server reports `dotproduct`; the TUI names it as `index_create` takes it
impl From<VectorMetric> for DistanceMetric {
    fn from(metric: VectorMetric) -> Self {
        match metric {
            VectorMetric::Cosine => Self::Cosine,
            VectorMetric::Euclidean => Self::Euclidean,
            VectorMetric::DotProduct => Self::DotProduct,
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

impl From<&IndexInfo> for VectorIndexInfo {
    fn from(idx: &IndexInfo) -> Self {
        Self {
            name: idx.name.clone(),
            field: idx.field.clone(),
            dimension: idx.dim,
            metric: DistanceMetric::from(idx.metric).as_str().to_string(),
            vector_count: idx.vector_count,
        }
    }
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
    pub vector_input: TextInput,
    /// Maximum results
    pub k: usize,
    /// Optional filter query (JSON string)
    pub filter_input: String,
    /// Search results: each hit's document and `_score` (cosine/L2/IP per the
    /// index metric; higher is more similar for cosine/dot-product)
    pub results: Vec<VectorSearchHit>,
    /// Selected result index
    pub selected_result: usize,

    // === Create Index tab ===
    /// Field name for new index
    pub new_field: TextInput,
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
    /// Edit the query vector (search tab) or the field name (create form);
    /// an edit clears the error
    pub fn handle_text_input(&mut self, input: Input) -> InputOutcome {
        let outcome = match self.active_tab {
            VectorTab::Search => self.vector_input.apply(input),
            VectorTab::CreateIndex if self.create_form_field == 0 => self.new_field.apply(input),
            // The dimension takes the digits of a paste (its keys: the handler)
            VectorTab::CreateIndex if self.create_form_field == 1 => match input {
                Input::Paste(text) => {
                    self.dimension = push_digits(self.dimension, text);
                    InputOutcome::Changed
                }
                Input::Key(..) => InputOutcome::Ignored,
            },
            _ => InputOutcome::Ignored,
        };
        if outcome == InputOutcome::Changed {
            self.error = None;
        }
        outcome
    }

    pub fn new(collection: String) -> Self {
        Self {
            collection,
            active_tab: VectorTab::Search,
            search_field: String::new(),
            vector_input: TextInput::default(),
            k: 10,
            filter_input: String::new(),
            results: Vec::new(),
            selected_result: 0,
            new_field: TextInput::default(),
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
    pub fn update_indexes(&mut self, indexes: Vec<IndexInfo>) {
        self.indexes = indexes.iter().map(VectorIndexInfo::from).collect();
        self.is_loading = false;
    }

    /// Parse vector input as JSON array of f64
    pub fn parse_vector(&self) -> Result<Vec<f64>, String> {
        let trimmed = self.vector_input.value().trim();
        if trimmed.is_empty() {
            return Err(t().vector_input_empty().to_string());
        }

        // Try to parse as JSON array
        match serde_json::from_str::<Vec<f64>>(trimmed) {
            Ok(v) => {
                if v.is_empty() {
                    Err(t().vector_empty().to_string())
                } else {
                    Ok(v)
                }
            }
            Err(e) => Err(t().invalid_vector(&e)),
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
            Err(e) => Err(t().invalid_filter(&e)),
        }
    }

    /// Update search results
    pub fn update_results(&mut self, results: Vec<VectorSearchHit>) {
        self.results = results;
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
