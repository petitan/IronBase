//! Index management state

/// Index management state
#[derive(Debug, Clone, Default)]
pub struct IndexState {
    pub collection: String,
    pub indexes: Vec<String>,
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
}

impl IndexState {
    pub fn new(collection: String, indexes: Vec<String>) -> Self {
        Self {
            collection,
            indexes,
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
        }
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
