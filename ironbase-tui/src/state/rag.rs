//! RAG & Embedding state

use serde_json::Value;

/// Active tab in RAG modal
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RagTab {
    #[default]
    Search,
    Import,
    Collections,
    Models,
    Jobs,
}

impl RagTab {
    pub fn next(&self) -> Self {
        match self {
            Self::Search => Self::Import,
            Self::Import => Self::Collections,
            Self::Collections => Self::Models,
            Self::Models => Self::Jobs,
            Self::Jobs => Self::Search,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            Self::Search => Self::Jobs,
            Self::Import => Self::Search,
            Self::Collections => Self::Import,
            Self::Models => Self::Collections,
            Self::Jobs => Self::Models,
        }
    }
}

/// RAG & Embedding modal state
#[derive(Debug, Clone, Default)]
pub struct RagState {
    /// Current collection
    pub collection: String,
    /// Active tab
    pub active_tab: RagTab,

    // === Search tab ===
    pub search_query: String,
    pub search_limit: usize,
    pub search_vector_weight: f64,
    pub search_fulltext_weight: f64,
    pub search_rrf_k: f64,
    pub search_results: Vec<Value>,
    pub selected_result: usize,
    /// Focus: 0=query, 1=options (limit/weights/rrf_k cycle)
    pub search_focus: usize,
    /// Which option is focused (0=limit, 1=vec_w, 2=ft_w, 3=rrf_k)
    pub search_option_idx: usize,

    // === Import tab ===
    pub import_content: String,
    pub import_title: String,
    pub import_chunk_size: usize,
    pub import_overlap: usize,
    pub import_mode: String,
    /// Focus field (0=content, 1=title, 2=chunk_size, 3=overlap, 4=mode)
    pub import_form_field: usize,
    pub import_result: Option<Value>,

    // === Collections tab ===
    pub rag_collections: Vec<Value>,
    pub selected_collection: usize,
    pub creating: bool,
    pub create_collection_name: String,
    pub create_embedding_field: String,
    pub create_text_field: String,
    pub create_provider: String,
    pub create_language: String,
    /// Focus field in create form (0=name, 1=emb_field, 2=text_field, 3=provider, 4=language)
    pub create_form_field: usize,

    // === Models tab ===
    pub models: Vec<Value>,
    pub selected_model: usize,
    pub auto_embed_config: Option<Value>,
    pub cache_stats: Option<Value>,
    pub embed_test_input: String,
    pub embed_test_result: Option<Value>,
    /// Sub-mode: 0=main view, 1=test input
    pub models_mode: usize,

    // === Jobs tab ===
    pub jobs: Vec<Value>,
    pub selected_job: usize,

    // === Common ===
    pub is_loading: bool,
    pub error: Option<String>,
    pub message: Option<String>,
}

impl RagState {
    pub fn new(collection: String) -> Self {
        Self {
            collection,
            active_tab: RagTab::Search,
            search_limit: 10,
            search_vector_weight: 0.5,
            search_fulltext_weight: 0.5,
            search_rrf_k: 20.0,
            import_chunk_size: 1000,
            import_overlap: 100,
            import_mode: "auto".to_string(),
            create_embedding_field: "embedding".to_string(),
            create_text_field: "content".to_string(),
            create_provider: "fasttext".to_string(),
            create_language: "hungarian".to_string(),
            ..Default::default()
        }
    }

    // === Tab navigation ===

    pub fn next_tab(&mut self) {
        self.active_tab = self.active_tab.next();
        self.error = None;
        self.message = None;
    }

    pub fn prev_tab(&mut self) {
        self.active_tab = self.active_tab.prev();
        self.error = None;
        self.message = None;
    }

    // === Search tab navigation ===

    pub fn search_result_up(&mut self) {
        if self.selected_result > 0 {
            self.selected_result -= 1;
        }
    }

    pub fn search_result_down(&mut self) {
        if self.selected_result + 1 < self.search_results.len() {
            self.selected_result += 1;
        }
    }

    // === Collections tab navigation ===

    pub fn collection_up(&mut self) {
        if self.selected_collection > 0 {
            self.selected_collection -= 1;
        }
    }

    pub fn collection_down(&mut self) {
        if self.selected_collection + 1 < self.rag_collections.len() {
            self.selected_collection += 1;
        }
    }

    // === Models tab navigation ===

    pub fn model_up(&mut self) {
        if self.selected_model > 0 {
            self.selected_model -= 1;
        }
    }

    pub fn model_down(&mut self) {
        if self.selected_model + 1 < self.models.len() {
            self.selected_model += 1;
        }
    }

    // === Jobs tab navigation ===

    pub fn job_up(&mut self) {
        if self.selected_job > 0 {
            self.selected_job -= 1;
        }
    }

    pub fn job_down(&mut self) {
        if self.selected_job + 1 < self.jobs.len() {
            self.selected_job += 1;
        }
    }

    /// Get selected job ID (if any)
    pub fn selected_job_id(&self) -> Option<String> {
        self.jobs.get(self.selected_job).and_then(|j| {
            j.get("job_id")
                .or_else(|| j.get("id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
    }

    // === Import tab ===

    pub fn next_import_field(&mut self) {
        self.import_form_field = (self.import_form_field + 1) % 5;
    }

    pub fn toggle_import_mode(&mut self) {
        self.import_mode = match self.import_mode.as_str() {
            "auto" => "markdown".to_string(),
            "markdown" => "text".to_string(),
            "text" => "auto".to_string(),
            _ => "auto".to_string(),
        };
    }

    // === Collections tab ===

    pub fn next_create_field(&mut self) {
        self.create_form_field = (self.create_form_field + 1) % 5;
    }

    pub fn toggle_create_language(&mut self) {
        self.create_language = match self.create_language.as_str() {
            "hungarian" => "english".to_string(),
            "english" => "german".to_string(),
            "german" => "none".to_string(),
            "none" => "hungarian".to_string(),
            _ => "hungarian".to_string(),
        };
    }

    pub fn toggle_create_provider(&mut self) {
        self.create_provider = match self.create_provider.as_str() {
            "fasttext" => "openai".to_string(),
            "openai" => "ollama".to_string(),
            "ollama" => "fasttext".to_string(),
            _ => "fasttext".to_string(),
        };
    }
}
