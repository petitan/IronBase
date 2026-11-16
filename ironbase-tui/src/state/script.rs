//! IronRhai Script Editor State

use crate::widgets::{Input, InputOutcome, TextArea, TextInput};
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
    pub editor: TextArea,

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
            editor: TextArea::with_cursor("// IronRhai script\n", 1, 0),
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
        self.stop_tag_typing();
    }

    /// Leave the tag being typed (a mode change must not carry it over)
    fn stop_tag_typing(&mut self) {
        self.tag_input.clear();
        self.tag_input_active = false;
    }

    /// Start new script
    pub fn start_new(&mut self) {
        self.mode = ScriptMode::New;
        self.focus = ScriptFocus::Name;
        self.editor = TextArea::with_cursor("// IronRhai script\n", 1, 0);
        self.name.clear();
        self.description.clear();
        self.tags.clear();
        self.selected_tag = 0;
        self.stop_tag_typing();
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
        self.editor = TextArea::new(&code);
        self.name.set_value(name);
        self.description.set_value(desc.unwrap_or_default());
        self.tags = tags;
        self.selected_tag = 0;
        self.stop_tag_typing();
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
        self.editor = TextArea::with_cursor("// Ad-hoc script\n", 1, 0);
        self.name.set_value("[inline]");
        self.description.clear();
        self.tags.clear();
        self.selected_tag = 0;
        self.stop_tag_typing();
        self.version = 0;
        self.result = None;
        self.error = None;
        self.dirty = false;
    }

    // === Editor operations ===

    /// A key for the focused one-line field (name, description, the tag
    /// being typed, params); a change clears the error, and a change of the
    /// name or description makes the script dirty
    pub fn handle_field_input(&mut self, input: Input) -> InputOutcome {
        let outcome = match self.focus {
            ScriptFocus::Name => self.name.apply(input),
            ScriptFocus::Description => self.description.apply(input),
            ScriptFocus::Tags if self.tag_input_active => self.tag_input.apply(input),
            ScriptFocus::Params => self.params_input.apply(input),
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

    /// Edit the focused field or the code; nothing while a confirmation is
    /// shown
    pub fn handle_input(&mut self, input: Input) -> InputOutcome {
        if self.confirm_action.is_some() {
            return InputOutcome::Ignored;
        }
        match self.focus {
            ScriptFocus::Editor => self.handle_editor_input(input),
            _ => self.handle_field_input(input),
        }
    }

    /// A key for the code editor; an edit marks the script changed
    pub fn handle_editor_input(&mut self, input: Input) -> InputOutcome {
        let outcome = self.editor.apply(input);
        if outcome == InputOutcome::Changed {
            self.dirty = true;
            self.error = None;
        }
        outcome
    }

    pub fn next_focus(&mut self) {
        self.focus = match self.focus {
            ScriptFocus::Name => ScriptFocus::Description,
            ScriptFocus::Description => ScriptFocus::Tags,
            ScriptFocus::Tags => ScriptFocus::Editor,
            ScriptFocus::Editor => ScriptFocus::Params,
            ScriptFocus::Params => ScriptFocus::Name,
            other => other,
        };
        self.stop_tag_typing();
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
        self.stop_tag_typing();
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
    use crossterm::event::{KeyCode, KeyModifiers};

    /// Editing the code marks the script changed; a cursor move does not
    #[test]
    fn test_editor_edit_marks_the_script_changed() {
        let mut state = ScriptState::new();
        state.load_script("s".to_string(), "ab\ncd".to_string(), None, vec![], 1);
        assert_eq!(
            state.handle_editor_input(Input::Key(KeyCode::Down, KeyModifiers::NONE)),
            InputOutcome::Moved
        );
        assert!(!state.dirty);
        state.error = Some("hiba".to_string());
        state.handle_editor_input(Input::Key(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(state.editor.text(), "abcd");
        assert!(state.dirty);
        assert_eq!(state.error, None);
    }

    fn key(state: &mut ScriptState, code: KeyCode) -> InputOutcome {
        state.handle_field_input(Input::Key(code, KeyModifiers::NONE))
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
    fn test_mode_change_leaves_the_tag_being_typed() {
        for leave in [ScriptState::reset_to_browse, ScriptState::enter_inline] {
            let mut state = ScriptState::new();
            state.start_new();
            state.focus = ScriptFocus::Tags;
            state.tag_input_active = true;
            type_into(&mut state, "ab");
            leave(&mut state);
            assert!(!state.tag_input_active);
            assert_eq!(state.tag_input.value(), "");
        }
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
        assert_eq!(state.editor.text(), "let a = 1;\nlet b = 2;");
        assert_eq!((state.name.cursor(), state.description.cursor()), (7, 6));
        assert_eq!(state.version, 3);
        assert!(!state.dirty);

        // Empty code still has one line to type into
        state.load_script("empty".to_string(), String::new(), None, vec![], 1);
        assert_eq!(state.editor.line_count(), 1);
        assert!(state.editor.is_empty());

        state.enter_inline();
        assert_eq!(state.mode, ScriptMode::Inline);
        assert_eq!(state.name.value(), "[inline]");
        assert_eq!(state.editor.cursor(), (1, 0));

        state.handle_editor_input(Input::Key(KeyCode::Char('x'), KeyModifiers::NONE));
        assert!(state.dirty);
        state.reset_to_browse();
        assert_eq!(state.mode, ScriptMode::Browse);
        assert_eq!(state.focus, ScriptFocus::List);
        assert!(!state.dirty);
    }
}
