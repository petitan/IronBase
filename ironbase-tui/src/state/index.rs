//! Index management state

/// Index management state
#[derive(Debug, Clone, Default)]
pub struct IndexState {
    pub collection: String,
    pub indexes: Vec<String>,
    pub selected_index: usize,
    pub is_creating: bool,
    pub form_field: usize, // 0 = field name, 1 = unique checkbox
    pub field_input: String,
    pub unique: bool,
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
            unique: false,
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
        self.unique = false;
        self.error = None;
    }

    pub fn cancel_create(&mut self) {
        self.is_creating = false;
        self.field_input.clear();
        self.unique = false;
        self.error = None;
    }

    pub fn next_form_field(&mut self) {
        self.form_field = (self.form_field + 1) % 2;
    }

    pub fn toggle_unique(&mut self) {
        self.unique = !self.unique;
    }

    pub fn insert_char(&mut self, c: char) {
        self.field_input.push(c);
        self.error = None;
    }

    pub fn backspace(&mut self) {
        self.field_input.pop();
        self.error = None;
    }
}
