//! IronRhai Script Editor State

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

/// Script info for browse list (without code)
#[derive(Debug, Clone)]
pub struct ScriptInfo {
    pub name: String,
    pub description: Option<String>,
    pub version: u32,
    pub tags: Vec<String>,
    pub execution_count: u64,
    pub last_run_at: Option<String>,
}

/// Script version for history
#[derive(Debug, Clone)]
pub struct ScriptVersion {
    pub version: u32,
    pub code: String,
    pub description: Option<String>,
    pub created_at: String,
}

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
    pub name: String,
    pub name_cursor: usize,
    pub description: String,
    pub desc_cursor: usize,
    pub tags: Vec<String>,
    pub selected_tag: usize,
    pub tag_input: String,
    pub tag_input_active: bool,
    pub version: u32,

    // Execution
    pub params_input: String,
    pub params_cursor: usize,
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
            name: String::new(),
            name_cursor: 0,
            description: String::new(),
            desc_cursor: 0,
            tags: Vec::new(),
            selected_tag: 0,
            tag_input: String::new(),
            tag_input_active: false,
            version: 0,
            params_input: "{}".to_string(),
            params_cursor: 2,
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
        self.name_cursor = 0;
        self.description.clear();
        self.desc_cursor = 0;
        self.tags.clear();
        self.tag_input.clear();
        self.tag_input_active = false;
        self.version = 0;
        self.params_input = "{}".to_string();
        self.params_cursor = 2;
        self.result = None;
        self.error = None;
        self.message = None;
        self.dirty = false;
    }

    /// Load script for editing
    pub fn load_script(&mut self, name: String, code: String, desc: Option<String>, tags: Vec<String>, version: u32) {
        self.mode = ScriptMode::Edit;
        self.focus = ScriptFocus::Editor;
        self.lines = code.lines().map(String::from).collect();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.cursor_line = 0;
        self.cursor_col = 0;
        self.scroll_offset = 0;
        self.name = name;
        self.name_cursor = self.name.chars().count();
        self.description = desc.unwrap_or_default();
        self.desc_cursor = self.description.chars().count();
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
        self.name = "[inline]".to_string();
        self.description.clear();
        self.tags.clear();
        self.version = 0;
        self.result = None;
        self.error = None;
        self.dirty = false;
    }

    // === Editor operations ===

    pub fn insert_char(&mut self, c: char) {
        match self.focus {
            ScriptFocus::Name => {
                let byte_pos = self.name.char_indices().nth(self.name_cursor).map(|(i, _)| i).unwrap_or(self.name.len());
                self.name.insert(byte_pos, c);
                self.name_cursor += 1;
                self.dirty = true;
            }
            ScriptFocus::Description => {
                let byte_pos = self.description.char_indices().nth(self.desc_cursor).map(|(i, _)| i).unwrap_or(self.description.len());
                self.description.insert(byte_pos, c);
                self.desc_cursor += 1;
                self.dirty = true;
            }
            ScriptFocus::Tags if self.tag_input_active => {
                self.tag_input.push(c);
            }
            ScriptFocus::Editor => {
                if let Some(line) = self.lines.get_mut(self.cursor_line) {
                    let char_count = line.chars().count();
                    if self.cursor_col <= char_count {
                        let byte_pos = line.char_indices().nth(self.cursor_col).map(|(i, _)| i).unwrap_or(line.len());
                        line.insert(byte_pos, c);
                        self.cursor_col += 1;
                        self.dirty = true;
                    }
                }
            }
            ScriptFocus::Params => {
                let byte_pos = self.params_input.char_indices().nth(self.params_cursor).map(|(i, _)| i).unwrap_or(self.params_input.len());
                self.params_input.insert(byte_pos, c);
                self.params_cursor += 1;
            }
            _ => {}
        }
        self.error = None;
    }

    pub fn backspace(&mut self) {
        match self.focus {
            ScriptFocus::Name if self.name_cursor > 0 => {
                let byte_pos = self.name.char_indices().nth(self.name_cursor - 1).map(|(i, _)| i).unwrap_or(0);
                self.name.remove(byte_pos);
                self.name_cursor -= 1;
                self.dirty = true;
            }
            ScriptFocus::Description if self.desc_cursor > 0 => {
                let byte_pos = self.description.char_indices().nth(self.desc_cursor - 1).map(|(i, _)| i).unwrap_or(0);
                self.description.remove(byte_pos);
                self.desc_cursor -= 1;
                self.dirty = true;
            }
            ScriptFocus::Tags if self.tag_input_active => {
                self.tag_input.pop();
            }
            ScriptFocus::Editor => {
                if self.cursor_col > 0 {
                    if let Some(line) = self.lines.get_mut(self.cursor_line) {
                        let byte_pos = line.char_indices().nth(self.cursor_col - 1).map(|(i, _)| i).unwrap_or(0);
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
            ScriptFocus::Params if self.params_cursor > 0 => {
                let byte_pos = self.params_input.char_indices().nth(self.params_cursor - 1).map(|(i, _)| i).unwrap_or(0);
                self.params_input.remove(byte_pos);
                self.params_cursor -= 1;
            }
            _ => {}
        }
        self.error = None;
    }

    pub fn insert_newline(&mut self) {
        if self.focus == ScriptFocus::Editor {
            if let Some(line) = self.lines.get_mut(self.cursor_line) {
                let byte_pos = line.char_indices().nth(self.cursor_col).map(|(i, _)| i).unwrap_or(line.len());
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
        match self.focus {
            ScriptFocus::Name if self.name_cursor > 0 => self.name_cursor -= 1,
            ScriptFocus::Description if self.desc_cursor > 0 => self.desc_cursor -= 1,
            ScriptFocus::Editor => {
                if self.cursor_col > 0 {
                    self.cursor_col -= 1;
                } else if self.cursor_line > 0 {
                    self.cursor_line -= 1;
                    self.cursor_col = self.lines.get(self.cursor_line).map(|l| l.chars().count()).unwrap_or(0);
                }
            }
            ScriptFocus::Params if self.params_cursor > 0 => self.params_cursor -= 1,
            _ => {}
        }
    }

    pub fn cursor_right(&mut self) {
        match self.focus {
            ScriptFocus::Name if self.name_cursor < self.name.chars().count() => self.name_cursor += 1,
            ScriptFocus::Description if self.desc_cursor < self.description.chars().count() => self.desc_cursor += 1,
            ScriptFocus::Editor => {
                let char_count = self.lines.get(self.cursor_line).map(|l| l.chars().count()).unwrap_or(0);
                if self.cursor_col < char_count {
                    self.cursor_col += 1;
                } else if self.cursor_line + 1 < self.lines.len() {
                    self.cursor_line += 1;
                    self.cursor_col = 0;
                }
            }
            ScriptFocus::Params if self.params_cursor < self.params_input.chars().count() => self.params_cursor += 1,
            _ => {}
        }
    }

    pub fn cursor_up(&mut self) {
        match self.focus {
            ScriptFocus::Editor if self.cursor_line > 0 => {
                self.cursor_line -= 1;
                let char_count = self.lines.get(self.cursor_line).map(|l| l.chars().count()).unwrap_or(0);
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
                let char_count = self.lines.get(self.cursor_line).map(|l| l.chars().count()).unwrap_or(0);
                self.cursor_col = self.cursor_col.min(char_count);
            }
            ScriptFocus::List if self.selected_script + 1 < self.scripts.len() => self.selected_script += 1,
            ScriptFocus::History if self.selected_version + 1 < self.versions.len() => self.selected_version += 1,
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
        match self.focus {
            ScriptFocus::Name => self.name_cursor = 0,
            ScriptFocus::Description => self.desc_cursor = 0,
            ScriptFocus::Editor => self.cursor_col = 0,
            ScriptFocus::Params => self.params_cursor = 0,
            _ => {}
        }
    }

    pub fn cursor_end(&mut self) {
        match self.focus {
            ScriptFocus::Name => self.name_cursor = self.name.chars().count(),
            ScriptFocus::Description => self.desc_cursor = self.description.chars().count(),
            ScriptFocus::Editor => {
                self.cursor_col = self.lines.get(self.cursor_line).map(|l| l.chars().count()).unwrap_or(0);
            }
            ScriptFocus::Params => self.params_cursor = self.params_input.chars().count(),
            _ => {}
        }
    }

    pub fn delete_char(&mut self) {
        if self.focus == ScriptFocus::Editor {
            if let Some(line) = self.lines.get_mut(self.cursor_line) {
                let char_count = line.chars().count();
                if self.cursor_col < char_count {
                    let byte_pos = line.char_indices().nth(self.cursor_col).map(|(i, _)| i).unwrap_or(line.len());
                    let next_byte_pos = line.char_indices().nth(self.cursor_col + 1).map(|(i, _)| i).unwrap_or(line.len());
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
            let tag = self.tag_input.trim().to_string();
            if !self.tags.contains(&tag) {
                self.tags.push(tag);
                self.dirty = true;
            }
            self.tag_input.clear();
            self.tag_input_active = false;
        }
    }

    pub fn remove_selected_tag(&mut self) {
        if !self.tags.is_empty() && self.selected_tag < self.tags.len() {
            self.tags.remove(self.selected_tag);
            if self.selected_tag > 0 {
                self.selected_tag -= 1;
            }
            self.dirty = true;
        }
    }

    pub fn select_prev_tag(&mut self) {
        if self.selected_tag > 0 {
            self.selected_tag -= 1;
        }
    }

    pub fn select_next_tag(&mut self) {
        if self.selected_tag + 1 < self.tags.len() {
            self.selected_tag += 1;
        }
    }

    pub fn toggle_tag_input(&mut self) {
        self.tag_input_active = !self.tag_input_active;
        if self.tag_input_active {
            self.tag_input.clear();
        }
    }

    // === Utility ===

    pub fn get_code(&self) -> String {
        self.lines.join("\n")
    }

    pub fn get_selected_script(&self) -> Option<&ScriptInfo> {
        self.scripts.get(self.selected_script)
    }

    pub fn get_selected_version(&self) -> Option<&ScriptVersion> {
        self.versions.get(self.selected_version)
    }
}
