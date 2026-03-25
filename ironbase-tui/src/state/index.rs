//! Index management state

/// Index statistics for a single index
#[derive(Debug, Clone, Default)]
pub struct IndexStatistics {
    /// Index name
    pub name: String,
    /// Primary field name
    pub field: String,
    /// Total number of keys in the index
    pub num_keys: u64,
    /// Distinct value count
    pub distinct_count: u64,
    /// Whether histogram data is available (for range queries)
    pub has_histogram: bool,
    /// Whether MCV data is available (for equality queries on skewed data)
    pub has_mcv: bool,
}

impl IndexStatistics {
    /// Parse from JSON value returned by MCP
    pub fn from_value(v: &serde_json::Value) -> Self {
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
            num_keys: v.get("num_keys").and_then(|v| v.as_u64()).unwrap_or(0),
            distinct_count: v
                .get("distinct_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
            has_histogram: v
                .get("has_histogram")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            has_mcv: v.get("has_mcv").and_then(|v| v.as_bool()).unwrap_or(false),
        }
    }

    /// Calculate staleness indicator (rough estimate)
    /// Returns a color hint: "green" (good), "yellow" (stale), "red" (very stale)
    pub fn staleness_hint(&self) -> &'static str {
        // If we have both histogram and MCV, the index is well-analyzed
        if self.has_histogram && self.has_mcv {
            "green"
        } else if self.has_histogram || self.has_mcv {
            "yellow"
        } else if self.num_keys > 0 {
            // Has keys but no statistics
            "red"
        } else {
            // Empty index, no statistics needed
            "green"
        }
    }
}

/// Index entry with type information
#[derive(Debug, Clone)]
pub struct IndexEntry {
    pub name: String,
    pub kind: IndexKind,
    /// Extra info (e.g. field, language, dim, metric)
    pub detail: String,
}

/// Index type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexKind {
    BTree,
    Fulltext,
    Vector,
}

impl IndexKind {
    pub fn label(&self) -> &'static str {
        match self {
            IndexKind::BTree => "B+",
            IndexKind::Fulltext => "FT",
            IndexKind::Vector => "VEC",
        }
    }
}

/// Index management state
#[derive(Debug, Clone, Default)]
pub struct IndexState {
    pub collection: String,
    pub indexes: Vec<String>,
    /// Structured index entries with type info
    pub index_entries: Vec<IndexEntry>,
    pub selected_index: usize,
    pub is_creating: bool,
    pub form_field: usize, // 0 = field name, 1 = compound, 2 = unique, 3 = sparse
    pub field_input: String,
    /// Multiple fields for compound index (comma-separated in input, parsed here)
    pub compound_fields: Vec<String>,
    /// If true, create compound index; if false, single field
    pub is_compound: bool,
    pub unique: bool,
    /// Sparse index: only indexes documents where the field exists
    pub sparse: bool,
    pub error: Option<String>,
    pub message: Option<String>,
    /// Detailed statistics for each index
    pub statistics: Vec<IndexStatistics>,
    /// Whether we're currently analyzing (refreshing stats)
    pub is_analyzing: bool,
}

impl IndexState {
    pub fn new(collection: String, indexes: Vec<String>) -> Self {
        Self {
            collection,
            indexes,
            index_entries: Vec::new(),
            selected_index: 0,
            is_creating: false,
            form_field: 0,
            field_input: String::new(),
            compound_fields: Vec::new(),
            is_compound: false,
            unique: false,
            sparse: false,
            error: None,
            message: None,
            statistics: Vec::new(),
            is_analyzing: false,
        }
    }

    pub fn new_with_entries(collection: String, entries: Vec<IndexEntry>) -> Self {
        let indexes = entries.iter().map(|e| e.name.clone()).collect();
        Self {
            collection,
            indexes,
            index_entries: entries,
            selected_index: 0,
            is_creating: false,
            form_field: 0,
            field_input: String::new(),
            compound_fields: Vec::new(),
            is_compound: false,
            unique: false,
            sparse: false,
            error: None,
            message: None,
            statistics: Vec::new(),
            is_analyzing: false,
        }
    }

    /// Update statistics from MCP response
    pub fn update_statistics(&mut self, stats: Vec<serde_json::Value>) {
        self.statistics = stats.iter().map(IndexStatistics::from_value).collect();
        self.is_analyzing = false;
    }

    /// Get statistics for the currently selected index
    pub fn selected_statistics(&self) -> Option<&IndexStatistics> {
        if self.indexes.is_empty() {
            return None;
        }
        let selected_name = &self.indexes[self.selected_index];
        self.statistics.iter().find(|s| &s.name == selected_name)
    }

    /// Check if any index needs refresh (has keys but no statistics)
    pub fn has_stale_indexes(&self) -> bool {
        self.statistics.iter().any(|s| s.staleness_hint() == "red")
    }

    /// Count stale indexes
    pub fn stale_index_count(&self) -> usize {
        self.statistics
            .iter()
            .filter(|s| s.staleness_hint() == "red")
            .count()
    }

    pub fn select_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    pub fn select_down(&mut self) {
        if self.selected_index + 1 < self.indexes.len() {
            self.selected_index += 1;
        }
    }

    pub fn start_create(&mut self) {
        self.is_creating = true;
        self.form_field = 0;
        self.field_input.clear();
        self.compound_fields.clear();
        self.is_compound = false;
        self.unique = false;
        self.sparse = false;
        self.error = None;
    }

    pub fn cancel_create(&mut self) {
        self.is_creating = false;
        self.field_input.clear();
        self.compound_fields.clear();
        self.is_compound = false;
        self.unique = false;
        self.sparse = false;
        self.error = None;
    }

    pub fn next_form_field(&mut self) {
        self.form_field = (self.form_field + 1) % 4; // 0=fields, 1=compound, 2=unique, 3=sparse
    }

    pub fn toggle_unique(&mut self) {
        self.unique = !self.unique;
    }

    pub fn toggle_compound(&mut self) {
        self.is_compound = !self.is_compound;
    }

    pub fn toggle_sparse(&mut self) {
        self.sparse = !self.sparse;
    }

    pub fn insert_char(&mut self, c: char) {
        self.field_input.push(c);
        self.parse_fields();
        self.error = None;
    }

    pub fn backspace(&mut self) {
        self.field_input.pop();
        self.parse_fields();
        self.error = None;
    }

    /// Parse comma-separated field input into compound_fields
    fn parse_fields(&mut self) {
        self.compound_fields = self
            .field_input
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }

    /// Get fields for index creation
    pub fn get_fields(&self) -> Vec<String> {
        if self.is_compound {
            self.compound_fields.clone()
        } else {
            // Single field mode - take first field only
            self.compound_fields.first().cloned().into_iter().collect()
        }
    }
}
