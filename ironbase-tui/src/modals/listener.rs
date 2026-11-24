//! Listener management modal

use super::render_modal_frame;
use crate::i18n::t;
use crate::theme::Theme;
use crate::widgets::{marked, Input, InputOutcome, TextInput};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

/// A configured listener, as the listener tools report it
pub use ironbase_mcp_types::responses::ListenerInfo;

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
    pub add_id: TextInput,
    /// Add form: Bind address
    pub add_bind: TextInput,
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
    /// Edit the focused field of the add form
    pub fn handle_input(&mut self, input: Input) -> InputOutcome {
        match (self.mode, self.add_field) {
            (ListenerModalMode::Add, AddField::Id) => self.add_id.apply(input),
            (ListenerModalMode::Add, AddField::Bind) => self.add_bind.apply(input),
            _ => InputOutcome::Ignored,
        }
    }

    pub fn new() -> Self {
        Self {
            mode: ListenerModalMode::List,
            listeners: Vec::new(),
            selected: 0,
            confirm_action: None,
            add_id: TextInput::default(),
            add_bind: TextInput::default(),
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
            self.add_bind.set_value("0.0.0.0:8080");
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
    let inner = render_modal_frame(frame, area, t().listeners_title(), theme, 70, 60);

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
        Span::raw(format!(" {}  ", t().key_navigate())),
    ];

    if state.is_localhost {
        help_spans.extend([
            Span::styled("[n]", Style::default().fg(theme.accent)),
            Span::raw(format!(" {}  ", t().key_new())),
            Span::styled("[e]", Style::default().fg(theme.accent)),
            Span::raw(format!(" {}  ", t().key_enable())),
            Span::styled("[x]", Style::default().fg(theme.accent)),
            Span::raw(format!(" {}  ", t().key_disable())),
            Span::styled("[d]", Style::default().fg(theme.accent)),
            Span::raw(format!(" {}  ", t().cmd_delete())),
        ]);
    }

    help_spans.extend([
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}", t().key_close())),
    ]);

    let help = Paragraph::new(Line::from(help_spans)).style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    // Listeners list
    render_listeners_list(frame, chunks[1], state, theme);

    // Warning
    let warning = Paragraph::new(t().restart_needed()).style(Style::default().fg(theme.warning));
    frame.render_widget(warning, chunks[2]);

    // Status/error line
    render_status(frame, chunks[3], state, theme);
}

fn render_listeners_list(frame: &mut Frame, area: Rect, state: &ListenerState, theme: &Theme) {
    let block = Block::default()
        .title(format!(" {} ", t().listeners_title()))
        .title_style(Style::default().fg(theme.fg))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.loading {
        let loading = Paragraph::new(t().loading()).style(Style::default().fg(theme.muted));
        frame.render_widget(loading, inner);
        return;
    }

    if state.listeners.is_empty() {
        let empty = Paragraph::new(t().no_listeners()).style(Style::default().fg(theme.muted));
        frame.render_widget(empty, inner);
        return;
    }

    // Header
    let header_chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(inner);

    let header = Paragraph::new(Line::from(vec![
        Span::styled(format!("{:<14}", "ID"), Style::default().fg(theme.muted)),
        Span::raw("  "),
        Span::styled(
            format!("{:<22}", t().label_bind()),
            Style::default().fg(theme.muted),
        ),
        Span::raw("  "),
        Span::styled(format!("{:<5}", "TLS"), Style::default().fg(theme.muted)),
        Span::raw("  "),
        Span::styled(t().label_status(), Style::default().fg(theme.muted)),
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
                t().status_active()
            } else {
                t().status_disabled()
            };
            let status_color = if listener.enabled {
                theme.success
            } else {
                theme.error
            };
            let tls_str = if listener.tls { t().yes() } else { t().no() };

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

            ListItem::new(marked(is_selected, line)).style(style)
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
    let instructions =
        Paragraph::new(t().enter_listener_details()).style(Style::default().fg(theme.fg));
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
    state.add_id.render(
        frame,
        id_inner,
        Style::default().fg(theme.fg),
        state.add_field == AddField::Id,
    );

    // Bind field
    let bind_style = if state.add_field == AddField::Bind {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.muted)
    };
    let bind_block = Block::default()
        .title(format!(" {} ", t().label_bind_address()))
        .borders(Borders::ALL)
        .border_style(bind_style);
    let bind_inner = bind_block.inner(chunks[2]);
    frame.render_widget(bind_block, chunks[2]);
    state.add_bind.render(
        frame,
        bind_inner,
        Style::default().fg(theme.fg),
        state.add_field == AddField::Bind,
    );

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
        t().checkbox_enabled()
    } else {
        t().checkbox_disabled()
    };
    let tls_hint = if state.add_field == AddField::Tls {
        format!(" {}", t().space_to_toggle())
    } else {
        String::new()
    };
    frame.render_widget(
        Paragraph::new(format!("{}{}", tls_text, tls_hint)),
        tls_inner,
    );

    // Buttons
    let buttons = Paragraph::new(Line::from(vec![
        Span::styled("[Enter]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_create())),
        Span::styled("[Tab]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_next_field())),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}", t().key_cancel())),
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

    let listener_id = state.get_selected().map(|l| l.id.as_str()).unwrap_or("?");
    let text = match state.confirm_action {
        Some(ListenerAction::Delete) => t().confirm_delete_listener(listener_id),
        Some(ListenerAction::Enable) => t().confirm_enable_listener(listener_id),
        Some(ListenerAction::Disable) => t().confirm_disable_listener(listener_id),
        None => String::new(),
    };

    let message = Paragraph::new(text)
        .style(Style::default().fg(theme.warning))
        .wrap(ratatui::widgets::Wrap { trim: true });
    frame.render_widget(message, chunks[0]);

    let buttons = Paragraph::new(Line::from(vec![
        Span::styled("[y]", Style::default().fg(theme.error)),
        Span::raw(format!(" {}  ", t().yes())),
        Span::styled("[n/Esc]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}", t().no())),
    ]));
    frame.render_widget(buttons, chunks[1]);
}

fn render_status(frame: &mut Frame, area: Rect, state: &ListenerState, theme: &Theme) {
    let status = if let Some(ref err) = state.error {
        Paragraph::new(err.as_str()).style(Style::default().fg(theme.error))
    } else if let Some(ref success) = state.success {
        Paragraph::new(success.as_str()).style(Style::default().fg(theme.success))
    } else if !state.is_localhost {
        Paragraph::new(t().listener_read_only()).style(Style::default().fg(theme.warning))
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
