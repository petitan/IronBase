//! API Key management modal - list, create, revoke, delete API keys

use super::render_modal_frame;
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

/// API Key information from the server
#[derive(Debug, Clone)]
pub struct ApiKeyInfo {
    pub id: u64,
    pub name: String,
    pub key_preview: String,
    pub created_at: String,
    pub enabled: bool,
}

impl ApiKeyInfo {
    /// Parse from JSON value
    pub fn from_value(value: &serde_json::Value) -> Option<Self> {
        Some(Self {
            id: value.get("_id").and_then(|v| v.as_u64())?,
            name: value
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            key_preview: value
                .get("key_preview")
                .and_then(|v| v.as_str())
                .unwrap_or("***")
                .to_string(),
            created_at: value
                .get("created_at")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            enabled: value
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        })
    }
}

/// Modal mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiKeyModalMode {
    /// Listing API keys
    List,
    /// Creating a new key (entering name)
    Create,
    /// Confirming revoke/delete
    Confirm,
}

/// Confirm action type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmAction {
    Revoke,
    Delete,
}

/// API Key modal state
#[derive(Debug)]
pub struct ApiKeyState {
    /// Current mode
    pub mode: ApiKeyModalMode,
    /// List of API keys
    pub keys: Vec<ApiKeyInfo>,
    /// Selected index in list
    pub selected: usize,
    /// Input buffer (for create mode - name)
    pub input: String,
    /// Newly created key (full key, only shown once)
    pub new_key: Option<String>,
    /// Error message
    pub error: Option<String>,
    /// Success message
    pub success: Option<String>,
    /// Loading state
    pub loading: bool,
    /// Confirm action
    pub confirm_action: Option<ConfirmAction>,
    /// Admin key (from env or input)
    pub admin_key: Option<String>,
}

impl Default for ApiKeyState {
    fn default() -> Self {
        Self::new()
    }
}

impl ApiKeyState {
    pub fn new() -> Self {
        // Try to get admin key from environment
        let admin_key = std::env::var("IRONBASE_ADMIN_KEY").ok();

        Self {
            mode: ApiKeyModalMode::List,
            keys: Vec::new(),
            selected: 0,
            input: String::new(),
            new_key: None,
            error: None,
            success: None,
            loading: false,
            confirm_action: None,
            admin_key,
        }
    }

    pub fn select_next(&mut self) {
        if !self.keys.is_empty() {
            self.selected = (self.selected + 1) % self.keys.len();
        }
    }

    pub fn select_prev(&mut self) {
        if !self.keys.is_empty() {
            self.selected = self.selected.checked_sub(1).unwrap_or(self.keys.len() - 1);
        }
    }

    pub fn get_selected(&self) -> Option<&ApiKeyInfo> {
        self.keys.get(self.selected)
    }

    pub fn start_create(&mut self) {
        self.mode = ApiKeyModalMode::Create;
        self.input.clear();
        self.error = None;
        self.success = None;
        self.new_key = None;
    }

    pub fn start_revoke(&mut self) {
        if self.get_selected().is_some() {
            self.mode = ApiKeyModalMode::Confirm;
            self.confirm_action = Some(ConfirmAction::Revoke);
            self.error = None;
            self.success = None;
        }
    }

    pub fn start_delete(&mut self) {
        if self.get_selected().is_some() {
            self.mode = ApiKeyModalMode::Confirm;
            self.confirm_action = Some(ConfirmAction::Delete);
            self.error = None;
            self.success = None;
        }
    }

    pub fn cancel(&mut self) {
        self.mode = ApiKeyModalMode::List;
        self.confirm_action = None;
        self.input.clear();
    }

    pub fn set_keys(&mut self, keys: Vec<ApiKeyInfo>) {
        self.keys = keys;
        if self.selected >= self.keys.len() {
            self.selected = self.keys.len().saturating_sub(1);
        }
    }

    pub fn set_error(&mut self, error: String) {
        self.error = Some(error);
        self.success = None;
    }

    pub fn set_success(&mut self, msg: String) {
        self.success = Some(msg);
        self.error = None;
    }

    pub fn has_admin_key(&self) -> bool {
        self.admin_key.is_some()
    }
}

/// Render the API key management modal
pub fn render(frame: &mut Frame, area: Rect, state: &ApiKeyState, theme: &Theme) {
    let inner = render_modal_frame(frame, area, "API Keys", theme, 70, 60);

    match state.mode {
        ApiKeyModalMode::List => render_list_mode(frame, inner, state, theme),
        ApiKeyModalMode::Create => render_create_mode(frame, inner, state, theme),
        ApiKeyModalMode::Confirm => render_confirm_mode(frame, inner, state, theme),
    }
}

fn render_list_mode(frame: &mut Frame, area: Rect, state: &ApiKeyState, theme: &Theme) {
    let chunks = Layout::vertical([
        Constraint::Length(2), // Help text
        Constraint::Min(8),    // Keys list
        Constraint::Length(2), // New key display (if any)
        Constraint::Length(1), // Status/error
    ])
    .split(area);

    // Help text
    let help_spans = if state.has_admin_key() {
        vec![
            Span::styled("[j/k]", Style::default().fg(theme.accent)),
            Span::raw(" Nav  "),
            Span::styled("[n]", Style::default().fg(theme.accent)),
            Span::raw(" New  "),
            Span::styled("[r]", Style::default().fg(theme.accent)),
            Span::raw(" Revoke  "),
            Span::styled("[d]", Style::default().fg(theme.accent)),
            Span::raw(" Delete  "),
            Span::styled("[c]", Style::default().fg(theme.accent)),
            Span::raw(" Copy  "),
            Span::styled("[Esc]", Style::default().fg(theme.accent)),
            Span::raw(" Close"),
        ]
    } else {
        vec![
            Span::styled("[j/k]", Style::default().fg(theme.accent)),
            Span::raw(" Nav  "),
            Span::styled("[Esc]", Style::default().fg(theme.accent)),
            Span::raw(" Close  "),
            Span::styled("(Set IRONBASE_ADMIN_KEY for admin ops)", Style::default().fg(theme.warning)),
        ]
    };

    let help = Paragraph::new(Line::from(help_spans)).style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    // Keys list
    render_keys_list(frame, chunks[1], state, theme);

    // New key display (if just created)
    if let Some(ref key) = state.new_key {
        let new_key_text = Paragraph::new(Line::from(vec![
            Span::styled("New key: ", Style::default().fg(theme.success)),
            Span::styled(key, Style::default().fg(theme.accent).bold()),
            Span::styled(" (copy now, won't be shown again!)", Style::default().fg(theme.warning)),
        ]));
        frame.render_widget(new_key_text, chunks[2]);
    }

    // Status/error line
    render_status(frame, chunks[3], state, theme);
}

fn render_keys_list(frame: &mut Frame, area: Rect, state: &ApiKeyState, theme: &Theme) {
    let block = Block::default()
        .title(" API Keys ")
        .title_style(Style::default().fg(theme.fg))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.loading {
        let loading = Paragraph::new("Loading...").style(Style::default().fg(theme.muted));
        frame.render_widget(loading, inner);
        return;
    }

    if state.keys.is_empty() {
        let empty = Paragraph::new("No API keys").style(Style::default().fg(theme.muted));
        frame.render_widget(empty, inner);
        return;
    }

    // Header
    let header_chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(inner);

    let header = Paragraph::new(Line::from(vec![
        Span::styled(format!("{:>4}", "ID"), Style::default().fg(theme.muted)),
        Span::raw("  "),
        Span::styled(format!("{:<16}", "Name"), Style::default().fg(theme.muted)),
        Span::raw("  "),
        Span::styled(format!("{:<16}", "Key"), Style::default().fg(theme.muted)),
        Span::raw("  "),
        Span::styled(format!("{:<6}", "Status"), Style::default().fg(theme.muted)),
    ]));
    frame.render_widget(header, header_chunks[0]);

    // List items
    let items: Vec<ListItem> = state
        .keys
        .iter()
        .enumerate()
        .map(|(i, key)| {
            let is_selected = i == state.selected;
            let status = if key.enabled { "Active" } else { "Revoked" };
            let status_color = if key.enabled {
                theme.success
            } else {
                theme.error
            };

            let line = Line::from(vec![
                Span::raw(format!("{:>4}", key.id)),
                Span::raw("  "),
                Span::raw(format!("{:<16}", truncate(&key.name, 16))),
                Span::raw("  "),
                Span::styled(
                    format!("{:<16}", key.key_preview),
                    Style::default().fg(theme.muted),
                ),
                Span::raw("  "),
                Span::styled(format!("{:<6}", status), Style::default().fg(status_color)),
            ]);

            let style = if is_selected {
                Style::default().fg(theme.bg).bg(theme.accent)
            } else {
                Style::default().fg(theme.fg)
            };

            ListItem::new(line).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, header_chunks[1]);
}

fn render_create_mode(frame: &mut Frame, area: Rect, state: &ApiKeyState, theme: &Theme) {
    let chunks = Layout::vertical([
        Constraint::Length(2), // Instructions
        Constraint::Length(3), // Input field
        Constraint::Length(2), // Buttons
        Constraint::Min(1),    // Spacer
        Constraint::Length(1), // Status
    ])
    .split(area);

    // Instructions
    let instructions = Paragraph::new("Enter a name for the new API key:")
        .style(Style::default().fg(theme.fg));
    frame.render_widget(instructions, chunks[0]);

    // Input field
    let input_block = Block::default()
        .title(" Name ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent));

    let input_inner = input_block.inner(chunks[1]);
    frame.render_widget(input_block, chunks[1]);

    let input_text = Paragraph::new(format!("{}_", state.input))
        .style(Style::default().fg(theme.fg));
    frame.render_widget(input_text, input_inner);

    // Buttons
    let buttons = Paragraph::new(Line::from(vec![
        Span::styled("[Enter]", Style::default().fg(theme.accent)),
        Span::raw(" Create  "),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(" Cancel"),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(buttons, chunks[2]);

    // Status
    render_status(frame, chunks[4], state, theme);
}

fn render_confirm_mode(frame: &mut Frame, area: Rect, state: &ApiKeyState, theme: &Theme) {
    let chunks = Layout::vertical([
        Constraint::Length(3), // Message
        Constraint::Length(2), // Buttons
        Constraint::Min(1),    // Spacer
    ])
    .split(area);

    // Message
    let action = match state.confirm_action {
        Some(ConfirmAction::Revoke) => "REVOKE",
        Some(ConfirmAction::Delete) => "DELETE",
        None => "???",
    };

    let key_name = state
        .get_selected()
        .map(|k| k.name.as_str())
        .unwrap_or("?");

    let message = Paragraph::new(format!(
        "Are you sure you want to {} the API key '{}'?",
        action, key_name
    ))
    .style(Style::default().fg(theme.warning))
    .wrap(ratatui::widgets::Wrap { trim: true });
    frame.render_widget(message, chunks[0]);

    // Buttons
    let buttons = Paragraph::new(Line::from(vec![
        Span::styled("[y]", Style::default().fg(theme.error)),
        Span::raw(" Yes  "),
        Span::styled("[n/Esc]", Style::default().fg(theme.accent)),
        Span::raw(" No"),
    ]));
    frame.render_widget(buttons, chunks[1]);
}

fn render_status(frame: &mut Frame, area: Rect, state: &ApiKeyState, theme: &Theme) {
    let status = if let Some(ref err) = state.error {
        Paragraph::new(err.as_str()).style(Style::default().fg(theme.error))
    } else if let Some(ref success) = state.success {
        Paragraph::new(success.as_str()).style(Style::default().fg(theme.success))
    } else {
        Paragraph::new("").style(Style::default().fg(theme.muted))
    };
    frame.render_widget(status, area);
}

/// BUG FIX: Use char count instead of byte length for UTF-8 safety
fn truncate(s: &str, max_len: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_len {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_len.saturating_sub(3)).collect();
        format!("{}...", truncated)
    }
}
