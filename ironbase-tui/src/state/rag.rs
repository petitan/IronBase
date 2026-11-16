//! RAG & Embedding state

use crate::widgets::{push_digits, Input, InputOutcome, TextArea, TextInput};
use ironbase_mcp_types::responses::{
    AutoEmbedStatusResponse, EmbedCacheStatsResponse, EmbedTextResponse, JobInfo, ModelInfo,
    RagCollectionStatsResponse, RagDocumentImportResponse, SearchBody, SearchDocument,
    SearchResponse,
};

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
    // (`search` is intent-only: weights, RRF K and the like are server-owned)
    pub search_query: TextInput,
    pub search_limit: usize,
    /// Documents of the last `search`
    pub search_results: Vec<SearchDocument>,
    /// The last `search` shortened or left out passages, or dropped documents
    pub search_trimmed: bool,
    /// Documents that matched but did not fit the server's context budget
    pub search_dropped_due_to_budget: usize,
    /// Documents that matched but had no body text
    pub search_dropped_empty: usize,
    pub selected_result: usize,
    /// Focus: 0=query, 1=limit
    pub search_focus: usize,

    // === Import tab ===
    pub import_content: TextArea,
    pub import_title: TextInput,
    pub import_chunk_size: usize,
    pub import_overlap: usize,
    pub import_mode: String,
    /// Focus field (0=content, 1=title, 2=chunk_size, 3=overlap, 4=mode)
    pub import_form_field: usize,
    pub import_result: Option<RagDocumentImportResponse>,

    // === Collections tab ===
    pub rag_collections: Vec<RagCollectionStatsResponse>,
    pub selected_collection: usize,
    pub creating: bool,
    pub create_collection_name: TextInput,
    pub create_embedding_field: TextInput,
    pub create_text_field: TextInput,
    pub create_provider: String,
    pub create_language: String,
    /// Focus field in create form (0=name, 1=emb_field, 2=text_field, 3=provider, 4=language)
    pub create_form_field: usize,

    // === Models tab ===
    /// The server's embedding providers, sorted by name
    pub models: Vec<ModelInfo>,
    pub selected_model: usize,
    pub auto_embed_config: Option<AutoEmbedStatusResponse>,
    pub cache_stats: Option<EmbedCacheStatsResponse>,
    pub embed_test_input: TextInput,
    pub embed_test_result: Option<EmbedTextResponse>,
    /// Sub-mode: 0=main view, 1=test input
    pub models_mode: usize,

    // === Jobs tab ===
    /// The server's jobs, newest first
    pub jobs: Vec<JobInfo>,
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
            import_chunk_size: 1000,
            import_overlap: 100,
            import_mode: "auto".to_string(),
            create_embedding_field: TextInput::new("embedding"),
            create_text_field: TextInput::new("content"),
            create_provider: "ollama".to_string(),
            create_language: "hungarian".to_string(),
            ..Default::default()
        }
    }

    /// The single-line input that has the focus, if any (the import
    /// content is a multi-line text, see `handle_text_input`)
    fn focused_input(&mut self) -> Option<&mut TextInput> {
        match self.active_tab {
            RagTab::Search if self.search_focus == 0 => Some(&mut self.search_query),
            RagTab::Import if self.import_form_field == 1 => Some(&mut self.import_title),
            RagTab::Collections if self.creating => match self.create_form_field {
                0 => Some(&mut self.create_collection_name),
                1 => Some(&mut self.create_embedding_field),
                2 => Some(&mut self.create_text_field),
                _ => None,
            },
            RagTab::Models if self.models_mode == 1 => Some(&mut self.embed_test_input),
            _ => None,
        }
    }

    /// Edit the focused input, the import content included; an edit clears
    /// the error
    pub fn handle_text_input(&mut self, input: Input) -> InputOutcome {
        let outcome = if self.active_tab == RagTab::Import && self.import_form_field == 0 {
            self.import_content.apply(input)
        } else if let (RagTab::Import, 2 | 3, Input::Paste(text)) =
            (self.active_tab, self.import_form_field, input)
        {
            // Chunk size and overlap take the digits of a paste (their keys:
            // the handler)
            let field = if self.import_form_field == 2 {
                &mut self.import_chunk_size
            } else {
                &mut self.import_overlap
            };
            *field = push_digits(*field, text);
            InputOutcome::Changed
        } else {
            match self.focused_input() {
                Some(field) => field.apply(input),
                None => InputOutcome::Ignored,
            }
        };
        if outcome == InputOutcome::Changed {
            self.error = None;
        }
        outcome
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
        self.jobs.get(self.selected_job).map(|j| j.id.clone())
    }

    /// Take a job list (the server lists jobs in hash-map order): newest
    /// first, the selection staying on the same job while it is listed
    pub fn set_jobs(&mut self, mut jobs: Vec<JobInfo>) {
        let selected = self.selected_job_id();
        jobs.sort_by(|a, b| {
            job_number(&b.id)
                .cmp(&job_number(&a.id))
                .then(b.id.cmp(&a.id))
        });
        self.selected_job = selected
            .and_then(|id| jobs.iter().position(|j| j.id == id))
            .unwrap_or(0);
        self.jobs = jobs;
    }

    /// Take the provider list (hash-map order on the server): sorted by
    /// name, the selection staying on the same provider while it is listed
    pub fn set_models(&mut self, mut models: Vec<ModelInfo>) {
        let selected = self.selected_provider();
        models.sort_by(|a, b| a.provider.cmp(&b.provider));
        self.selected_model = selected
            .and_then(|p| models.iter().position(|m| m.provider == p))
            .unwrap_or(0);
        self.models = models;
    }

    /// The provider selected on the Models tab (`None`: the server's default)
    pub fn selected_provider(&self) -> Option<String> {
        self.models
            .get(self.selected_model)
            .map(|m| m.provider.clone())
    }

    /// Take a `search` response: its documents and what the server left out
    pub fn set_search_response(&mut self, response: SearchResponse) -> Result<(), String> {
        let SearchBody::Documents(documents) = response.body else {
            return Err("search: expected documents, got a context block".to_string());
        };
        self.search_results = documents;
        self.search_trimmed = response.trimmed;
        self.search_dropped_due_to_budget = response.dropped_due_to_budget.unwrap_or(0);
        self.search_dropped_empty = response.dropped_empty_documents.unwrap_or(0);
        self.selected_result = 0;
        Ok(())
    }

    /// Drop the last `search`'s documents and what it left out (a failed
    /// search: they do not belong to the query shown)
    pub fn clear_search_response(&mut self) {
        self.search_results.clear();
        self.search_trimmed = false;
        self.search_dropped_due_to_budget = 0;
        self.search_dropped_empty = 0;
        self.selected_result = 0;
    }

    /// The last `search` did not return everything that matched
    pub fn search_incomplete(&self) -> bool {
        self.search_trimmed
            || self.search_dropped_due_to_budget > 0
            || self.search_dropped_empty > 0
    }

    /// "Found N results", with what the server left out
    pub fn search_summary(&self) -> String {
        let found = format!("Found {} results", self.search_results.len());
        if !self.search_incomplete() {
            return found;
        }
        let mut left_out = Vec::new();
        if self.search_dropped_due_to_budget > 0 {
            left_out.push(format!(
                "{} more over the context budget",
                self.search_dropped_due_to_budget
            ));
        }
        if self.search_dropped_empty > 0 {
            left_out.push(format!("{} without body text", self.search_dropped_empty));
        }
        if left_out.is_empty() {
            left_out.push("passages shortened".to_string());
        }
        format!("{} (trimmed: {})", found, left_out.join(", "))
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
            "ollama" => "openai".to_string(),
            "openai" => "ollama".to_string(),
            _ => "ollama".to_string(),
        };
    }
}

/// The sequence number of a `job_<n>` id (jobs are numbered in creation order)
fn job_number(id: &str) -> Option<u64> {
    id.strip_prefix("job_")?.parse().ok()
}
