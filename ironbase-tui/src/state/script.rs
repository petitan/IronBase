//! IronRhai Script Editor State

use crate::widgets::{InputOutcome, TextInput};
use crossterm::event::{KeyCode, KeyModifiers};
use serde_json::Value;

/// Script mode - browse, edit, new, history, inline
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScriptMode {
    #[default]
    Browse,
    Edit,
    New,
    History,
    Inline,
}

/// Pending confirmation action for script modal
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptConfirmAction {
    /// Discard unsaved changes and go back
    DiscardChanges,
    /// Delete the selected script
    DeleteScript,
}

/// Script editor focus area
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScriptFocus {
    #[default]
    List,
    Name,
    Description,
    Tags,
    Editor,
    Params,
    History,
}

// The browse list entries and the history versions: the server's own types
pub use ironbase_mcp_types::responses::{ScriptInfo, ScriptVersion};

/// Script execution result
#[derive(Debug, Clone)]
pub struct ScriptResult {
    pub success: bool,
    pub result: Value,
    pub logs: Vec<String>,
    pub error: Option<String>,
    pub execution_time_ms: u64,
}

/// IronRhai script editor state
#[derive(Debug, Clone)]
pub struct ScriptState {
    // Editor
    pub lines: Vec<String>,
    pub cursor_line: usize,
    pub cursor_col: usize,
    pub scroll_offset: usize,

    // Metadata
    pub name: TextInput,
    pub description: TextInput,
    pub tags: Vec<String>,
    pub selected_tag: usize,
    pub tag_input: TextInput,
    pub tag_input_active: bool,
    pub version: u32,

    // Execution
    pub params_input: TextInput,
    pub result: Option<ScriptResult>,

    // UI state
    pub focus: ScriptFocus,
    pub mode: ScriptMode,
    pub error: Option<String>,
    pub message: Option<String>,
    pub dirty: bool,
    pub loading: bool,
    pub confirm_action: Option<ScriptConfirmAction>,

    // Browse mode
    pub scripts: Vec<ScriptInfo>,
    pub selected_script: usize,

    // History mode
    pub versions: Vec<ScriptVersion>,
    pub selected_version: usize,
}

impl Default for ScriptState {
    fn default() -> Self {
        Self {
            lines: vec!["// IronRhai script".to_string(), "".to_string()],
            cursor_line: 1,
            cursor_col: 0,
            scroll_offset: 0,
            name: TextInput::default(),
            description: TextInput::default(),
            tags: Vec::new(),
            selected_tag: 0,
            tag_input: TextInput::default(),
            tag_input_active: false,
            version: 0,
            params_input: TextInput::new("{}"),
            result: None,
            focus: ScriptFocus::List,
            mode: ScriptMode::Browse,
            error: None,
            message: None,
            dirty: false,
            loading: false,
            confirm_action: None,
            scripts: Vec::new(),
            selected_script: 0,
            versions: Vec::new(),
            selected_version: 0,
        }
    }
}

impl ScriptState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset to browse mode
    pub fn reset_to_browse(&mut self) {
        self.mode = ScriptMode::Browse;
        self.focus = ScriptFocus::List;
        self.error = None;
        self.message = None;
        self.dirty = false;
        self.confirm_action = None;
    }

    /// Start new script
    pub fn start_new(&mut self) {
        self.mode = ScriptMode::New;
        self.focus = ScriptFocus::Name;
        self.lines = vec!["// IronRhai script".to_string(), "".to_string()];
        self.cursor_line = 1;
        self.cursor_col = 0;
        self.scroll_offset = 0;
        self.name.clear();
        self.description.clear();
        self.tags.clear();
        self.tag_input.clear();
        self.tag_input_active = false;
        self.version = 0;
        self.params_input.set_value("{}");
        self.result = None;
        self.error = None;
        self.message = None;
        self.dirty = false;
    }

    /// Load script for editing
    pub fn load_script(
        &mut self,
        name: String,
        code: String,
        desc: Option<String>,
        tags: Vec<String>,
        version: u32,
    ) {
        self.mode = ScriptMode::Edit;
        self.focus = ScriptFocus::Editor;
        self.lines = code.lines().map(String::from).collect();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.cursor_line = 0;
        self.cursor_col = 0;
        self.scroll_offset = 0;
        self.name.set_value(name);
        self.description.set_value(desc.unwrap_or_default());
        self.tags = tags;
        self.selected_tag = 0;
        self.tag_input.clear();
        self.tag_input_active = false;
        self.version = version;
        self.result = None;
        self.error = None;
        self.message = None;
        self.dirty = false;
    }

    /// Switch to history mode
    pub fn enter_history(&mut self) {
        self.mode = ScriptMode::History;
        self.focus = ScriptFocus::History;
        self.selected_version = 0;
    }

    /// Switch to inline mode (ad-hoc execution)
    pub fn enter_inline(&mut self) {
        self.mode = ScriptMode::Inline;
        self.focus = ScriptFocus::Editor;
        self.lines = vec!["// Ad-hoc script".to_string(), "".to_string()];
        self.cursor_line = 1;
        self.cursor_col = 0;
        self.scroll_offset = 0;
        self.name.set_value("[inline]");
        self.description.clear();
        self.tags.clear();
        self.version = 0;
        self.result = None;
        self.error = None;
        self.dirty = false;
    }

    // === Editor operations ===

    /// A key for the focused one-line field (name, description, the tag
    /// being typed, params); a change clears the error, and a change of the
    /// name or description makes the script dirty
    pub fn handle_field_input(&mut self, code: KeyCode, modifiers: KeyModifiers) -> InputOutcome {
        let outcome = match self.focus {
            ScriptFocus::Name => self.name.handle_key(code, modifiers),
            ScriptFocus::Description => self.description.handle_key(code, modifiers),
            ScriptFocus::Tags if self.tag_input_active => {
                self.tag_input.handle_key(code, modifiers)
            }
            ScriptFocus::Params => self.params_input.handle_key(code, modifiers),
            _ => InputOutcome::Ignored,
        };
        if outcome == InputOutcome::Changed {
            if matches!(self.focus, ScriptFocus::Name | ScriptFocus::Description) {
                self.dirty = true;
            }
            self.error = None;
        }
        outcome
    }

    /// Type `c` into the code editor
    pub fn insert_char(&mut self, c: char) {
        if self.focus == ScriptFocus::Editor {
            if let Some(line) = self.lines.get_mut(self.cursor_line) {
                let char_count = line.chars().count();
                if self.cursor_col <= char_count {
                    let byte_pos = line
                        .char_indices()
                        .nth(self.cursor_col)
                        .map(|(i, _)| i)
                        .unwrap_or(line.len());
                    line.insert(byte_pos, c);
                    self.cursor_col += 1;
                    self.dirty = true;
                }
            }
        }
        self.error = None;
    }

    /// Backspace in the code editor (joins lines at a line start)
    pub fn backspace(&mut self) {
        if self.focus == ScriptFocus::Editor {
            if self.cursor_col > 0 {
                if let Some(line) = self.lines.get_mut(self.cursor_line) {
                    let byte_pos = line
                        .char_indices()
                        .nth(self.cursor_col - 1)
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    line.remove(byte_pos);
                    self.cursor_col -= 1;
                    self.dirty = true;
                }
            } else if self.cursor_line > 0 {
                let current_line = self.lines.remove(self.cursor_line);
                self.cursor_line -= 1;
                if let Some(prev_line) = self.lines.get_mut(self.cursor_line) {
                    self.cursor_col = prev_line.chars().count();
                    prev_line.push_str(&current_line);
                }
                self.dirty = true;
            }
        }
        self.error = None;
    }

    pub fn insert_newline(&mut self) {
        if self.focus == ScriptFocus::Editor {
            if let Some(line) = self.lines.get_mut(self.cursor_line) {
                let byte_pos = line
                    .char_indices()
                    .nth(self.cursor_col)
                    .map(|(i, _)| i)
                    .unwrap_or(line.len());
                let rest = line.split_off(byte_pos);
                self.cursor_line += 1;
                self.lines.insert(self.cursor_line, rest);
                self.cursor_col = 0;
                self.dirty = true;
            }
        }
        self.error = None;
    }

    pub fn insert_tab(&mut self) {
        if self.focus == ScriptFocus::Editor {
            self.insert_char(' ');
            self.insert_char(' ');
        }
    }

    // === Cursor navigation ===

    pub fn cursor_left(&mut self) {
        if self.focus == ScriptFocus::Editor {
            if self.cursor_col > 0 {
                self.cursor_col -= 1;
            } else if self.cursor_line > 0 {
                self.cursor_line -= 1;
                self.cursor_col = self
                    .lines
                    .get(self.cursor_line)
                    .map(|l| l.chars().count())
                    .unwrap_or(0);
            }
        }
    }

    pub fn cursor_right(&mut self) {
        if self.focus == ScriptFocus::Editor {
            let char_count = self
                .lines
                .get(self.cursor_line)
                .map(|l| l.chars().count())
                .unwrap_or(0);
            if self.cursor_col < char_count {
                self.cursor_col += 1;
            } else if self.cursor_line + 1 < self.lines.len() {
                self.cursor_line += 1;
                self.cursor_col = 0;
            }
        }
    }

    pub fn cursor_up(&mut self) {
        match self.focus {
            ScriptFocus::Editor if self.cursor_line > 0 => {
                self.cursor_line -= 1;
                let char_count = self
                    .lines
                    .get(self.cursor_line)
                    .map(|l| l.chars().count())
                    .unwrap_or(0);
                self.cursor_col = self.cursor_col.min(char_count);
            }
            ScriptFocus::List if self.selected_script > 0 => self.selected_script -= 1,
            ScriptFocus::History if self.selected_version > 0 => self.selected_version -= 1,
            _ => {}
        }
    }

    pub fn cursor_down(&mut self) {
        match self.focus {
            ScriptFocus::Editor if self.cursor_line + 1 < self.lines.len() => {
                self.cursor_line += 1;
                let char_count = self
                    .lines
                    .get(self.cursor_line)
                    .map(|l| l.chars().count())
                    .unwrap_or(0);
                self.cursor_col = self.cursor_col.min(char_count);
            }
            ScriptFocus::List if self.selected_script + 1 < self.scripts.len() => {
                self.selected_script += 1
            }
            ScriptFocus::History if self.selected_version + 1 < self.versions.len() => {
                self.selected_version += 1
            }
            _ => {}
        }
    }

    // === Focus navigation ===

    pub fn next_focus(&mut self) {
        self.focus = match self.focus {
            ScriptFocus::Name => ScriptFocus::Description,
            ScriptFocus::Description => ScriptFocus::Tags,
            ScriptFocus::Tags => ScriptFocus::Editor,
            ScriptFocus::Editor => ScriptFocus::Params,
            ScriptFocus::Params => ScriptFocus::Name,
            other => other,
        };
        self.tag_input_active = false;
    }

    pub fn prev_focus(&mut self) {
        self.focus = match self.focus {
            ScriptFocus::Name => ScriptFocus::Params,
            ScriptFocus::Description => ScriptFocus::Name,
            ScriptFocus::Tags => ScriptFocus::Description,
            ScriptFocus::Editor => ScriptFocus::Tags,
            ScriptFocus::Params => ScriptFocus::Editor,
            other => other,
        };
        self.tag_input_active = false;
    }

    pub fn cursor_home(&mut self) {
        if self.focus == ScriptFocus::Editor {
            self.cursor_col = 0;
        }
    }

    pub fn cursor_end(&mut self) {
        if self.focus == ScriptFocus::Editor {
            self.cursor_col = self
                .lines
                .get(self.cursor_line)
                .map(|l| l.chars().count())
                .unwrap_or(0);
        }
    }

    pub fn delete_char(&mut self) {
        if self.focus == ScriptFocus::Editor {
            if let Some(line) = self.lines.get_mut(self.cursor_line) {
                let char_count = line.chars().count();
                if self.cursor_col < char_count {
                    let byte_pos = line
                        .char_indices()
                        .nth(self.cursor_col)
                        .map(|(i, _)| i)
                        .unwrap_or(line.len());
                    let next_byte_pos = line
                        .char_indices()
                        .nth(self.cursor_col + 1)
                        .map(|(i, _)| i)
                        .unwrap_or(line.len());
                    line.replace_range(byte_pos..next_byte_pos, "");
                    self.dirty = true;
                } else if self.cursor_line + 1 < self.lines.len() {
                    // Merge next line
                    let next_line = self.lines.remove(self.cursor_line + 1);
                    if let Some(current_line) = self.lines.get_mut(self.cursor_line) {
                        current_line.push_str(&next_line);
                    }
                    self.dirty = true;
                }
            }
        }
        self.error = None;
    }

    // === Tag operations ===

    pub fn add_tag(&mut self) {
        if !self.tag_input.is_empty() {
            let tag = self.tag_input.value().trim().to_string();
            if !self.tags.contains(&tag) {
                self.tags.push(tag);
                self.dirty = true;
            }
            self.tag_input.clear();
            self.tag_input_active = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(lines: &[&str], line: usize, col: usize) -> ScriptState {
        ScriptState {
            lines: lines.iter().map(|l| l.to_string()).collect(),
            cursor_line: line,
            cursor_col: col,
            focus: ScriptFocus::Editor,
            mode: ScriptMode::Edit,
            ..ScriptState::default()
        }
    }

    #[test]
    fn test_editor_insert_is_by_character_not_byte() {
        let mut state = editor(&["árvíz"], 0, 2);
        state.insert_char('ő');
        assert_eq!(state.lines, vec!["árővíz"]);
        assert_eq!(state.cursor_col, 3);
        assert!(state.dirty);

        // At the end of the line
        state.cursor_end();
        assert_eq!(state.cursor_col, 6);
        state.insert_char('🙂');
        assert_eq!(state.lines, vec!["árővíz🙂"]);
        assert_eq!(state.cursor_col, 7);
    }

    #[test]
    fn test_editor_backspace_and_delete_with_multibyte() {
        let mut state = editor(&["tűzőgép"], 0, 2);
        state.backspace();
        assert_eq!(state.lines, vec!["tzőgép"]);
        assert_eq!(state.cursor_col, 1);

        state.delete_char();
        assert_eq!(state.lines, vec!["tőgép"]);
        assert_eq!(state.cursor_col, 1);
        state.delete_char();
        assert_eq!(state.lines, vec!["tgép"]);
    }

    #[test]
    fn test_editor_backspace_at_line_start_joins_lines() {
        let mut state = editor(&["első", "második"], 1, 0);
        state.backspace();
        assert_eq!(state.lines, vec!["elsőmásodik"]);
        assert_eq!((state.cursor_line, state.cursor_col), (0, 4));
        assert!(state.dirty);

        // First line, first column: nothing to join, not dirty
        let mut state = editor(&["a"], 0, 0);
        state.backspace();
        assert_eq!(state.lines, vec!["a"]);
        assert!(!state.dirty);
    }

    #[test]
    fn test_editor_delete_at_line_end_joins_next_line() {
        let mut state = editor(&["ab", "cd"], 0, 2);
        state.delete_char();
        assert_eq!(state.lines, vec!["abcd"]);
        assert_eq!((state.cursor_line, state.cursor_col), (0, 2));

        // Last line end: nothing happens
        let mut state = editor(&["ab"], 0, 2);
        state.delete_char();
        assert_eq!(state.lines, vec!["ab"]);
        assert!(!state.dirty);
    }

    #[test]
    fn test_editor_newline_splits_at_character_position() {
        let mut state = editor(&["őzláb"], 0, 2);
        state.insert_newline();
        assert_eq!(state.lines, vec!["őz", "láb"]);
        assert_eq!((state.cursor_line, state.cursor_col), (1, 0));

        state.insert_tab();
        assert_eq!(state.lines, vec!["őz", "  láb"]);
        assert_eq!(state.cursor_col, 2);
    }

    #[test]
    fn test_editor_cursor_movement_wraps_and_clamps() {
        let mut state = editor(&["hosszú sor", "űr", ""], 0, 10);
        // Right at line end: start of the next line
        state.cursor_right();
        assert_eq!((state.cursor_line, state.cursor_col), (1, 0));
        // Left at line start: end of the previous line (in characters)
        state.cursor_left();
        assert_eq!((state.cursor_line, state.cursor_col), (0, 10));
        // Down clamps the column to the shorter line
        state.cursor_down();
        assert_eq!((state.cursor_line, state.cursor_col), (1, 2));
        state.cursor_down();
        assert_eq!((state.cursor_line, state.cursor_col), (2, 0));
        // Last line: stays
        state.cursor_down();
        state.cursor_right();
        assert_eq!((state.cursor_line, state.cursor_col), (2, 0));
        // Up keeps the (clamped) column
        state.cursor_up();
        state.cursor_up();
        assert_eq!((state.cursor_line, state.cursor_col), (0, 0));
        state.cursor_up();
        state.cursor_left();
        assert_eq!((state.cursor_line, state.cursor_col), (0, 0));

        state.cursor_end();
        assert_eq!(state.cursor_col, 10);
        state.cursor_home();
        assert_eq!(state.cursor_col, 0);
    }

    fn key(state: &mut ScriptState, code: KeyCode) -> InputOutcome {
        state.handle_field_input(code, KeyModifiers::NONE)
    }

    fn type_into(state: &mut ScriptState, text: &str) {
        for c in text.chars() {
            key(state, KeyCode::Char(c));
        }
    }

    #[test]
    fn test_name_and_description_editing_with_multibyte() {
        let mut state = ScriptState::new();
        state.start_new();
        assert_eq!(state.focus, ScriptFocus::Name);
        type_into(&mut state, "név");
        assert_eq!((state.name.value(), state.name.cursor()), ("név", 3));
        key(&mut state, KeyCode::Left);
        key(&mut state, KeyCode::Backspace);
        assert_eq!((state.name.value(), state.name.cursor()), ("nv", 1));
        key(&mut state, KeyCode::Home);
        assert_eq!(key(&mut state, KeyCode::Backspace), InputOutcome::Moved);
        assert_eq!(state.name.value(), "nv");
        assert!(state.dirty);

        state.next_focus();
        assert_eq!(state.focus, ScriptFocus::Description);
        type_into(&mut state, "ék");
        key(&mut state, KeyCode::Home);
        type_into(&mut state, "ű");
        assert_eq!(
            (state.description.value(), state.description.cursor()),
            ("űék", 1)
        );
    }

    /// Params start as "{}" with the cursor after it; editing them does not
    /// mark the script dirty
    #[test]
    fn test_params_editing() {
        let mut state = ScriptState::new();
        state.start_new();
        state.focus = ScriptFocus::Params;
        assert_eq!(
            (state.params_input.value(), state.params_input.cursor()),
            ("{}", 2)
        );
        key(&mut state, KeyCode::Left);
        type_into(&mut state, "é");
        assert_eq!(state.params_input.value(), "{é}");
        key(&mut state, KeyCode::Backspace);
        assert_eq!(
            (state.params_input.value(), state.params_input.cursor()),
            ("{}", 1)
        );
        assert!(!state.dirty);
    }

    #[test]
    fn test_tags_and_focus_cycle() {
        let mut state = ScriptState::new();
        state.start_new();
        state.focus = ScriptFocus::Tags;

        // Typing only reaches the tag input while it is active
        assert_eq!(key(&mut state, KeyCode::Char('x')), InputOutcome::Ignored);
        assert_eq!(state.tag_input.value(), "");
        state.tag_input_active = true;
        type_into(&mut state, " etl ");
        key(&mut state, KeyCode::Backspace);
        assert_eq!(state.tag_input.value(), " etl");
        state.add_tag();
        assert_eq!(state.tags, vec!["etl"]);
        assert!(!state.tag_input_active);
        assert!(state.dirty);

        // A duplicate is dropped, the input is cleared all the same
        state.tag_input.set_value("etl");
        state.add_tag();
        assert_eq!(state.tags, vec!["etl"]);
        assert_eq!(state.tag_input.value(), "");

        // Name -> Description -> Tags -> Editor -> Params -> Name
        state.focus = ScriptFocus::Name;
        let mut seen = Vec::new();
        for _ in 0..5 {
            state.next_focus();
            seen.push(state.focus);
        }
        assert_eq!(
            seen,
            vec![
                ScriptFocus::Description,
                ScriptFocus::Tags,
                ScriptFocus::Editor,
                ScriptFocus::Params,
                ScriptFocus::Name
            ]
        );
        state.prev_focus();
        assert_eq!(state.focus, ScriptFocus::Params);
        // Browse/History focus is not part of the cycle
        state.focus = ScriptFocus::List;
        state.next_focus();
        assert_eq!(state.focus, ScriptFocus::List);
    }

    #[test]
    fn test_mode_transitions_reset_the_editor() {
        let mut state = ScriptState::new();
        state.load_script(
            "cleanup".to_string(),
            "let a = 1;\nlet b = 2;".to_string(),
            Some("leírás".to_string()),
            vec!["etl".to_string()],
            3,
        );
        assert_eq!(state.mode, ScriptMode::Edit);
        assert_eq!(state.focus, ScriptFocus::Editor);
        assert_eq!(state.lines, vec!["let a = 1;", "let b = 2;"]);
        assert_eq!((state.name.cursor(), state.description.cursor()), (7, 6));
        assert_eq!(state.version, 3);
        assert!(!state.dirty);

        // Empty code still has one line to type into
        state.load_script("empty".to_string(), String::new(), None, vec![], 1);
        assert_eq!(state.lines, vec![""]);

        state.enter_inline();
        assert_eq!(state.mode, ScriptMode::Inline);
        assert_eq!(state.name.value(), "[inline]");
        assert_eq!((state.cursor_line, state.cursor_col), (1, 0));

        state.insert_char('x');
        assert!(state.dirty);
        state.reset_to_browse();
        assert_eq!(state.mode, ScriptMode::Browse);
        assert_eq!(state.focus, ScriptFocus::List);
        assert!(!state.dirty);
    }
}
