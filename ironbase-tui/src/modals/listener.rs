//! Listener management modal

use super::render_modal_frame;
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

/// Listener information
#[derive(Debug, Clone)]
pub struct ListenerInfo {
    pub id: String,
    pub bind: String,
    pub tls: bool,
    pub enabled: bool,
    pub cert_path: Option<String>,
    pub key_path: Option<String>,
}

impl ListenerInfo {
    pub fn from_value(value: &serde_json::Value) -> Option<Self> {
        Some(Self {
            id: value
                .get("_id")
                .or_else(|| value.get("id"))
                .and_then(|v| v.as_str())?
                .to_string(),
            bind: value.get("bind").and_then(|v| v.as_str())?.to_string(),
            tls: value.get("tls").and_then(|v| v.as_bool()).unwrap_or(false),
            enabled: value
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            cert_path: value
                .get("cert_path")
                .and_then(|v| v.as_str())
                .map(String::from),
            key_path: value
                .get("key_path")
                .and_then(|v| v.as_str())
                .map(String::from),
        })
    }
}

/// Modal mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListenerModalMode {
    /// Listing all listeners
    List,
    /// Adding a new listener
    Add,
    /// Confirm action (delete/enable/disable)
    Confirm,
}

/// Confirm action type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListenerAction {
    Delete,
    Enable,
    Disable,
}

/// Add form field focus
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddField {
    Id,
    Bind,
    Tls,
}

/// Listener modal state
#[derive(Debug)]
pub struct ListenerState {
    /// Current mode
    pub mode: ListenerModalMode,
    /// List of listeners
    pub listeners: Vec<ListenerInfo>,
    /// Selected index
    pub selected: usize,
    /// Confirm action
    pub confirm_action: Option<ListenerAction>,
    /// Add form: ID
    pub add_id: String,
    /// Add form: Bind address
    pub add_bind: String,
    /// Add form: TLS enabled
    pub add_tls: bool,
    /// Add form: current field
    pub add_field: AddField,
    /// Error message
    pub error: Option<String>,
    /// Success message
    pub success: Option<String>,
    /// Loading state
    pub loading: bool,
    /// Is localhost connection (can edit)
    pub is_localhost: bool,
}

impl Default for ListenerState {
    fn default() -> Self {
        Self::new()
    }
}

impl ListenerState {
    pub fn new() -> Self {
        Self {
            mode: ListenerModalMode::List,
            listeners: Vec::new(),
            selected: 0,
            confirm_action: None,
            add_id: String::new(),
            add_bind: String::new(),
            add_tls: false,
            add_field: AddField::Id,
            error: None,
            success: None,
            loading: false,
            is_localhost: false,
        }
    }

    pub fn select_next(&mut self) {
        if !self.listeners.is_empty() {
            self.selected = (self.selected + 1) % self.listeners.len();
        }
    }

    pub fn select_prev(&mut self) {
        if !self.listeners.is_empty() {
            self.selected = self
                .selected
                .checked_sub(1)
                .unwrap_or(self.listeners.len() - 1);
        }
    }

    pub fn get_selected(&self) -> Option<&ListenerInfo> {
        self.listeners.get(self.selected)
    }

    pub fn start_add(&mut self) {
        if self.is_localhost {
            self.mode = ListenerModalMode::Add;
            self.add_id.clear();
            self.add_bind = "0.0.0.0:8080".to_string();
            self.add_tls = false;
            self.add_field = AddField::Id;
            self.error = None;
            self.success = None;
        }
    }

    pub fn start_delete(&mut self) {
        if self.get_selected().is_some() && self.is_localhost {
            self.mode = ListenerModalMode::Confirm;
            self.confirm_action = Some(ListenerAction::Delete);
        }
    }

    pub fn start_enable(&mut self) {
        if let Some(listener) = self.get_selected() {
            if !listener.enabled && self.is_localhost {
                self.mode = ListenerModalMode::Confirm;
                self.confirm_action = Some(ListenerAction::Enable);
            }
        }
    }

    pub fn start_disable(&mut self) {
        if let Some(listener) = self.get_selected() {
            if listener.enabled && self.is_localhost {
                self.mode = ListenerModalMode::Confirm;
                self.confirm_action = Some(ListenerAction::Disable);
            }
        }
    }

    pub fn cancel(&mut self) {
        self.mode = ListenerModalMode::List;
        self.confirm_action = None;
    }

    pub fn next_add_field(&mut self) {
        self.add_field = match self.add_field {
            AddField::Id => AddField::Bind,
            AddField::Bind => AddField::Tls,
            AddField::Tls => AddField::Id,
        };
    }

    pub fn toggle_tls(&mut self) {
        self.add_tls = !self.add_tls;
    }

    pub fn set_listeners(&mut self, listeners: Vec<ListenerInfo>) {
        self.listeners = listeners;
        if self.selected >= self.listeners.len() {
            self.selected = self.listeners.len().saturating_sub(1);
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
}

/// Render the Listener management modal
pub fn render(frame: &mut Frame, area: Rect, state: &ListenerState, theme: &Theme) {
    let inner = render_modal_frame(frame, area, "Listeners", theme, 70, 60);

    match state.mode {
        ListenerModalMode::List => render_list_mode(frame, inner, state, theme),
        ListenerModalMode::Add => render_add_mode(frame, inner, state, theme),
        ListenerModalMode::Confirm => render_confirm_mode(frame, inner, state, theme),
    }
}

fn render_list_mode(frame: &mut Frame, area: Rect, state: &ListenerState, theme: &Theme) {
    let chunks = Layout::vertical([
        Constraint::Length(2), // Help text
        Constraint::Min(8),    // Listeners list
        Constraint::Length(2), // Warning
        Constraint::Length(1), // Status/error
    ])
    .split(area);

    // Help text
    let mut help_spans = vec![
        Span::styled("[j/k]", Style::default().fg(theme.accent)),
        Span::raw(" Nav  "),
    ];

    if state.is_localhost {
        help_spans.extend([
            Span::styled("[n]", Style::default().fg(theme.accent)),
            Span::raw(" New  "),
            Span::styled("[e]", Style::default().fg(theme.accent)),
            Span::raw(" Enable  "),
            Span::styled("[x]", Style::default().fg(theme.accent)),
            Span::raw(" Disable  "),
            Span::styled("[d]", Style::default().fg(theme.accent)),
            Span::raw(" Delete  "),
        ]);
    }

    help_spans.extend([
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(" Close"),
    ]);

    let help = Paragraph::new(Line::from(help_spans)).style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    // Listeners list
    render_listeners_list(frame, chunks[1], state, theme);

    // Warning
    let warning = Paragraph::new("Changes require server restart to take effect")
        .style(Style::default().fg(theme.warning));
    frame.render_widget(warning, chunks[2]);

    // Status/error line
    render_status(frame, chunks[3], state, theme);
}

fn render_listeners_list(frame: &mut Frame, area: Rect, state: &ListenerState, theme: &Theme) {
    let block = Block::default()
        .title(" Listeners ")
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

    if state.listeners.is_empty() {
        let empty =
            Paragraph::new("No listeners configured").style(Style::default().fg(theme.muted));
        frame.render_widget(empty, inner);
        return;
    }

    // Header
    let header_chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(inner);

    let header = Paragraph::new(Line::from(vec![
        Span::styled(format!("{:<14}", "ID"), Style::default().fg(theme.muted)),
        Span::raw("  "),
        Span::styled(format!("{:<22}", "Bind"), Style::default().fg(theme.muted)),
        Span::raw("  "),
        Span::styled(format!("{:<5}", "TLS"), Style::default().fg(theme.muted)),
        Span::raw("  "),
        Span::styled("Status", Style::default().fg(theme.muted)),
    ]));
    frame.render_widget(header, header_chunks[0]);

    // List items
    let items: Vec<ListItem> = state
        .listeners
        .iter()
        .enumerate()
        .map(|(i, listener)| {
            let is_selected = i == state.selected;
            let status = if listener.enabled {
                "Active"
            } else {
                "Disabled"
            };
            let status_color = if listener.enabled {
                theme.success
            } else {
                theme.error
            };
            let tls_str = if listener.tls { "Yes" } else { "No" };

            let line = Line::from(vec![
                Span::raw(format!("{:<14}", truncate(&listener.id, 14))),
                Span::raw("  "),
                Span::raw(format!("{:<22}", truncate(&listener.bind, 22))),
                Span::raw("  "),
                Span::raw(format!("{:<5}", tls_str)),
                Span::raw("  "),
                Span::styled(status, Style::default().fg(status_color)),
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

fn render_add_mode(frame: &mut Frame, area: Rect, state: &ListenerState, theme: &Theme) {
    let chunks = Layout::vertical([
        Constraint::Length(2), // Instructions
        Constraint::Length(3), // ID field
        Constraint::Length(3), // Bind field
        Constraint::Length(3), // TLS toggle
        Constraint::Length(2), // Buttons
        Constraint::Min(1),    // Spacer
        Constraint::Length(1), // Status
    ])
    .split(area);

    // Instructions
    let instructions = Paragraph::new("Enter listener details (Tab to switch fields):")
        .style(Style::default().fg(theme.fg));
    frame.render_widget(instructions, chunks[0]);

    // ID field
    let id_style = if state.add_field == AddField::Id {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.muted)
    };
    let id_block = Block::default()
        .title(" ID ")
        .borders(Borders::ALL)
        .border_style(id_style);
    let id_inner = id_block.inner(chunks[1]);
    frame.render_widget(id_block, chunks[1]);
    let id_text = if state.add_field == AddField::Id {
        format!("{}_", state.add_id)
    } else {
        state.add_id.clone()
    };
    frame.render_widget(Paragraph::new(id_text), id_inner);

    // Bind field
    let bind_style = if state.add_field == AddField::Bind {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.muted)
    };
    let bind_block = Block::default()
        .title(" Bind Address ")
        .borders(Borders::ALL)
        .border_style(bind_style);
    let bind_inner = bind_block.inner(chunks[2]);
    frame.render_widget(bind_block, chunks[2]);
    let bind_text = if state.add_field == AddField::Bind {
        format!("{}_", state.add_bind)
    } else {
        state.add_bind.clone()
    };
    frame.render_widget(Paragraph::new(bind_text), bind_inner);

    // TLS toggle
    let tls_style = if state.add_field == AddField::Tls {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.muted)
    };
    let tls_block = Block::default()
        .title(" TLS ")
        .borders(Borders::ALL)
        .border_style(tls_style);
    let tls_inner = tls_block.inner(chunks[3]);
    frame.render_widget(tls_block, chunks[3]);
    let tls_text = if state.add_tls {
        "[x] Enabled"
    } else {
        "[ ] Disabled"
    };
    let tls_hint = if state.add_field == AddField::Tls {
        " (Space to toggle)"
    } else {
        ""
    };
    frame.render_widget(
        Paragraph::new(format!("{}{}", tls_text, tls_hint)),
        tls_inner,
    );

    // Buttons
    let buttons = Paragraph::new(Line::from(vec![
        Span::styled("[Enter]", Style::default().fg(theme.accent)),
        Span::raw(" Create  "),
        Span::styled("[Tab]", Style::default().fg(theme.accent)),
        Span::raw(" Next field  "),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(" Cancel"),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(buttons, chunks[4]);

    // Status
    render_status(frame, chunks[6], state, theme);
}

fn render_confirm_mode(frame: &mut Frame, area: Rect, state: &ListenerState, theme: &Theme) {
    let chunks = Layout::vertical([
        Constraint::Length(3), // Message
        Constraint::Length(2), // Buttons
        Constraint::Min(1),    // Spacer
    ])
    .split(area);

    let action = match state.confirm_action {
        Some(ListenerAction::Delete) => "DELETE",
        Some(ListenerAction::Enable) => "ENABLE",
        Some(ListenerAction::Disable) => "DISABLE",
        None => "???",
    };

    let listener_id = state.get_selected().map(|l| l.id.as_str()).unwrap_or("?");

    let message = Paragraph::new(format!(
        "Are you sure you want to {} the listener '{}'?",
        action, listener_id
    ))
    .style(Style::default().fg(theme.warning))
    .wrap(ratatui::widgets::Wrap { trim: true });
    frame.render_widget(message, chunks[0]);

    let buttons = Paragraph::new(Line::from(vec![
        Span::styled("[y]", Style::default().fg(theme.error)),
        Span::raw(" Yes  "),
        Span::styled("[n/Esc]", Style::default().fg(theme.accent)),
        Span::raw(" No"),
    ]));
    frame.render_widget(buttons, chunks[1]);
}

fn render_status(frame: &mut Frame, area: Rect, state: &ListenerState, theme: &Theme) {
    let status = if let Some(ref err) = state.error {
        Paragraph::new(err.as_str()).style(Style::default().fg(theme.error))
    } else if let Some(ref success) = state.success {
        Paragraph::new(success.as_str()).style(Style::default().fg(theme.success))
    } else if !state.is_localhost {
        Paragraph::new("Read-only: Listener management requires localhost connection")
            .style(Style::default().fg(theme.warning))
    } else {
        Paragraph::new("").style(Style::default().fg(theme.muted))
    };
    frame.render_widget(status, area);
}

fn truncate(s: &str, max_len: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_len {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_len.saturating_sub(3)).collect();
        format!("{}...", truncated)
    }
}
