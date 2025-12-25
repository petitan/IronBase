//! ACL (Access Control List) management modal

use super::render_modal_frame;
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

/// ACL rule information
#[derive(Debug, Clone)]
pub struct AclRuleInfo {
    pub principal: String,
    pub permissions: String,
}

impl AclRuleInfo {
    pub fn from_value(value: &serde_json::Value) -> Option<Self> {
        let principal = if let Some(p) = value.get("principal") {
            if let Some(ptype) = p.get("type").and_then(|v| v.as_str()) {
                let pvalue = p.get("value").and_then(|v| {
                    v.as_str().map(|s| s.to_string()).or_else(|| Some(v.to_string()))
                }).unwrap_or_default();
                format!("{}:{}", ptype, pvalue)
            } else {
                p.to_string()
            }
        } else {
            return None;
        };

        let permissions = if let Some(perms) = value.get("permissions") {
            let mut perm_list = Vec::new();
            if perms.get("read").and_then(|v| v.as_bool()).unwrap_or(false) {
                perm_list.push("read");
            }
            if perms.get("write").and_then(|v| v.as_bool()).unwrap_or(false) {
                perm_list.push("write");
            }
            if perms.get("admin").and_then(|v| v.as_bool()).unwrap_or(false) {
                perm_list.push("admin");
            }
            perm_list.join(",")
        } else {
            String::new()
        };

        Some(Self { principal, permissions })
    }
}

/// Collection ACL information
#[derive(Debug, Clone)]
pub struct CollectionAclInfo {
    pub collection: String,
    pub rules: Vec<AclRuleInfo>,
    pub is_builtin: bool,
}

impl CollectionAclInfo {
    pub fn from_value(value: &serde_json::Value) -> Option<Self> {
        let collection = value.get("collection").and_then(|v| v.as_str())?.to_string();

        let rules: Vec<AclRuleInfo> = value
            .get("rules")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(AclRuleInfo::from_value).collect())
            .unwrap_or_default();

        // Consider builtin if collection starts with _system or is "*"
        let is_builtin = collection.starts_with("_system") || collection == "*";

        Some(Self { collection, rules, is_builtin })
    }
}

/// Principal type for ACL rules
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrincipalType {
    Interface,
    ApiKey,
    Ip,
    IpRange,
}

impl PrincipalType {
    pub fn all() -> &'static [PrincipalType] {
        &[
            PrincipalType::Interface,
            PrincipalType::ApiKey,
            PrincipalType::Ip,
            PrincipalType::IpRange,
        ]
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            PrincipalType::Interface => "interface",
            PrincipalType::ApiKey => "apikey",
            PrincipalType::Ip => "ip",
            PrincipalType::IpRange => "iprange",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            PrincipalType::Interface => PrincipalType::ApiKey,
            PrincipalType::ApiKey => PrincipalType::Ip,
            PrincipalType::Ip => PrincipalType::IpRange,
            PrincipalType::IpRange => PrincipalType::Interface,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            PrincipalType::Interface => PrincipalType::IpRange,
            PrincipalType::ApiKey => PrincipalType::Interface,
            PrincipalType::Ip => PrincipalType::ApiKey,
            PrincipalType::IpRange => PrincipalType::Ip,
        }
    }

    pub fn default_value(&self) -> &'static str {
        match self {
            PrincipalType::Interface => "internal",
            PrincipalType::ApiKey => "my_key",
            PrincipalType::Ip => "192.168.1.100",
            PrincipalType::IpRange => "192.168.1.0/24",
        }
    }
}

/// Editable rule in the editor
#[derive(Debug, Clone)]
pub struct EditRule {
    pub principal_type: PrincipalType,
    pub principal_value: String,
    pub read: bool,
    pub write: bool,
    pub admin: bool,
}

impl Default for EditRule {
    fn default() -> Self {
        Self {
            principal_type: PrincipalType::Interface,
            principal_value: "internal".to_string(),
            read: true,
            write: false,
            admin: false,
        }
    }
}

impl EditRule {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "principal": format!("{}:{}", self.principal_type.as_str(), self.principal_value),
            "permissions": self.permissions_string()
        })
    }

    pub fn permissions_string(&self) -> String {
        let mut perms = Vec::new();
        if self.read { perms.push("read"); }
        if self.write { perms.push("write"); }
        if self.admin { perms.push("admin"); }
        if perms.is_empty() { "none".to_string() } else { perms.join(",") }
    }

    pub fn display_principal(&self) -> String {
        format!("{}:{}", self.principal_type.as_str(), self.principal_value)
    }
}

/// Edit mode field focus
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditField {
    PrincipalType,
    PrincipalValue,
    PermRead,
    PermWrite,
    PermAdmin,
}

impl EditField {
    pub fn next(&self) -> Self {
        match self {
            EditField::PrincipalType => EditField::PrincipalValue,
            EditField::PrincipalValue => EditField::PermRead,
            EditField::PermRead => EditField::PermWrite,
            EditField::PermWrite => EditField::PermAdmin,
            EditField::PermAdmin => EditField::PrincipalType,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            EditField::PrincipalType => EditField::PermAdmin,
            EditField::PrincipalValue => EditField::PrincipalType,
            EditField::PermRead => EditField::PrincipalValue,
            EditField::PermWrite => EditField::PermRead,
            EditField::PermAdmin => EditField::PermWrite,
        }
    }
}

/// Modal mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AclModalMode {
    /// Listing all ACL rules by collection
    List,
    /// Viewing rules for one collection
    ViewCollection,
    /// Confirm delete
    ConfirmDelete,
    /// Editing ACL for a collection (full editor)
    Edit,
    /// Confirm save
    ConfirmSave,
}

/// ACL modal state
#[derive(Debug)]
pub struct AclState {
    /// Current mode
    pub mode: AclModalMode,
    /// List of collection ACLs
    pub acls: Vec<CollectionAclInfo>,
    /// Selected collection index
    pub selected: usize,
    /// Selected rule index (in ViewCollection mode)
    pub selected_rule: usize,
    /// Error message
    pub error: Option<String>,
    /// Success message
    pub success: Option<String>,
    /// Loading state
    pub loading: bool,
    /// Is localhost connection (can edit)
    pub is_localhost: bool,

    // === Edit mode fields ===
    /// Collection being edited
    pub edit_collection: String,
    /// Rules being edited
    pub edit_rules: Vec<EditRule>,
    /// Currently selected rule in editor (for deletion)
    pub edit_selected_rule: usize,
    /// Current rule being added/edited
    pub current_rule: EditRule,
    /// Current field focus in add form
    pub edit_field: EditField,
    /// Is adding a new rule (vs editing existing)
    pub is_adding_rule: bool,
}

impl Default for AclState {
    fn default() -> Self {
        Self::new()
    }
}

impl AclState {
    pub fn new() -> Self {
        Self {
            mode: AclModalMode::List,
            acls: Vec::new(),
            selected: 0,
            selected_rule: 0,
            error: None,
            success: None,
            loading: false,
            is_localhost: false,
            // Edit mode
            edit_collection: String::new(),
            edit_rules: Vec::new(),
            edit_selected_rule: 0,
            current_rule: EditRule::default(),
            edit_field: EditField::PrincipalType,
            is_adding_rule: false,
        }
    }

    pub fn select_next(&mut self) {
        match self.mode {
            AclModalMode::List => {
                if !self.acls.is_empty() {
                    self.selected = (self.selected + 1) % self.acls.len();
                }
            }
            AclModalMode::ViewCollection => {
                if let Some(acl) = self.acls.get(self.selected) {
                    if !acl.rules.is_empty() {
                        self.selected_rule = (self.selected_rule + 1) % acl.rules.len();
                    }
                }
            }
            AclModalMode::Edit if !self.is_adding_rule => {
                if !self.edit_rules.is_empty() {
                    self.edit_selected_rule = (self.edit_selected_rule + 1) % self.edit_rules.len();
                }
            }
            _ => {}
        }
    }

    pub fn select_prev(&mut self) {
        match self.mode {
            AclModalMode::List => {
                if !self.acls.is_empty() {
                    self.selected = self.selected.checked_sub(1).unwrap_or(self.acls.len() - 1);
                }
            }
            AclModalMode::ViewCollection => {
                if let Some(acl) = self.acls.get(self.selected) {
                    if !acl.rules.is_empty() {
                        self.selected_rule = self.selected_rule.checked_sub(1).unwrap_or(acl.rules.len() - 1);
                    }
                }
            }
            AclModalMode::Edit if !self.is_adding_rule => {
                if !self.edit_rules.is_empty() {
                    self.edit_selected_rule = self.edit_selected_rule.checked_sub(1).unwrap_or(self.edit_rules.len() - 1);
                }
            }
            _ => {}
        }
    }

    pub fn get_selected(&self) -> Option<&CollectionAclInfo> {
        self.acls.get(self.selected)
    }

    pub fn enter_collection(&mut self) {
        if self.get_selected().is_some() {
            self.mode = AclModalMode::ViewCollection;
            self.selected_rule = 0;
        }
    }

    pub fn back_to_list(&mut self) {
        self.mode = AclModalMode::List;
        self.error = None;
        self.success = None;
    }

    pub fn start_delete(&mut self) {
        if let Some(acl) = self.get_selected() {
            if !acl.is_builtin && self.is_localhost {
                self.mode = AclModalMode::ConfirmDelete;
            }
        }
    }

    pub fn cancel_delete(&mut self) {
        self.mode = AclModalMode::List;
    }

    /// Start editing ACL for a collection (from context menu)
    pub fn start_edit(&mut self, collection: String) {
        self.edit_collection = collection;
        self.edit_rules.clear();
        self.edit_selected_rule = 0;
        self.current_rule = EditRule::default();
        self.edit_field = EditField::PrincipalType;
        self.is_adding_rule = false;
        self.error = None;
        self.success = None;
        self.mode = AclModalMode::Edit;
    }

    /// Start editing existing ACL (from list)
    pub fn start_edit_existing(&mut self) {
        // Clone data before mutating
        let (collection, rules, is_builtin) = match self.get_selected() {
            Some(acl) => (acl.collection.clone(), acl.rules.clone(), acl.is_builtin),
            None => return,
        };

        if is_builtin {
            return;
        }

        self.edit_collection = collection;
        // Convert existing rules to EditRule
        self.edit_rules = rules.iter().map(|r| {
            let (ptype, pvalue) = parse_principal(&r.principal);
            let (read, write, admin) = parse_permissions(&r.permissions);
            EditRule {
                principal_type: ptype,
                principal_value: pvalue,
                read,
                write,
                admin,
            }
        }).collect();
        self.edit_selected_rule = 0;
        self.current_rule = EditRule::default();
        self.edit_field = EditField::PrincipalType;
        self.is_adding_rule = false;
        self.error = None;
        self.success = None;
        self.mode = AclModalMode::Edit;
    }

    /// Start adding a new rule
    pub fn start_add_rule(&mut self) {
        self.current_rule = EditRule::default();
        self.edit_field = EditField::PrincipalType;
        self.is_adding_rule = true;
    }

    /// Cancel adding rule
    pub fn cancel_add_rule(&mut self) {
        self.is_adding_rule = false;
    }

    /// Confirm add rule to list
    pub fn confirm_add_rule(&mut self) {
        if self.current_rule.principal_value.is_empty() {
            self.error = Some("Principal value cannot be empty".to_string());
            return;
        }
        if !self.current_rule.read && !self.current_rule.write && !self.current_rule.admin {
            self.error = Some("Select at least one permission".to_string());
            return;
        }
        self.edit_rules.push(self.current_rule.clone());
        self.current_rule = EditRule::default();
        self.is_adding_rule = false;
        self.error = None;
    }

    /// Delete selected rule from edit list
    pub fn delete_selected_rule(&mut self) {
        if !self.edit_rules.is_empty() {
            self.edit_rules.remove(self.edit_selected_rule);
            if self.edit_selected_rule >= self.edit_rules.len() && !self.edit_rules.is_empty() {
                self.edit_selected_rule = self.edit_rules.len() - 1;
            }
        }
    }

    /// Navigate to next field in add form
    pub fn next_field(&mut self) {
        self.edit_field = self.edit_field.next();
    }

    /// Navigate to prev field in add form
    pub fn prev_field(&mut self) {
        self.edit_field = self.edit_field.prev();
    }

    /// Toggle current permission checkbox
    pub fn toggle_permission(&mut self) {
        match self.edit_field {
            EditField::PermRead => self.current_rule.read = !self.current_rule.read,
            EditField::PermWrite => self.current_rule.write = !self.current_rule.write,
            EditField::PermAdmin => self.current_rule.admin = !self.current_rule.admin,
            _ => {}
        }
    }

    /// Cycle principal type
    pub fn cycle_principal_type(&mut self, forward: bool) {
        if self.edit_field == EditField::PrincipalType {
            self.current_rule.principal_type = if forward {
                self.current_rule.principal_type.next()
            } else {
                self.current_rule.principal_type.prev()
            };
            // Update default value for new type
            self.current_rule.principal_value = self.current_rule.principal_type.default_value().to_string();
        }
    }

    /// Get rules as JSON array for saving
    pub fn get_rules_json(&self) -> Vec<serde_json::Value> {
        self.edit_rules.iter().map(|r| r.to_json()).collect()
    }

    /// Cancel edit mode
    pub fn cancel_edit(&mut self) {
        self.mode = AclModalMode::List;
        self.edit_rules.clear();
        self.edit_collection.clear();
        self.is_adding_rule = false;
    }

    /// Start save confirmation
    pub fn start_save(&mut self) {
        if self.edit_rules.is_empty() {
            self.error = Some("Add at least one rule".to_string());
            return;
        }
        self.mode = AclModalMode::ConfirmSave;
    }

    /// Cancel save
    pub fn cancel_save(&mut self) {
        self.mode = AclModalMode::Edit;
    }

    pub fn set_acls(&mut self, acls: Vec<CollectionAclInfo>) {
        self.acls = acls;
        if self.selected >= self.acls.len() {
            self.selected = self.acls.len().saturating_sub(1);
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

/// Parse principal string "type:value" into components
fn parse_principal(s: &str) -> (PrincipalType, String) {
    if let Some((ptype, pvalue)) = s.split_once(':') {
        let principal_type = match ptype {
            "interface" => PrincipalType::Interface,
            "apikey" => PrincipalType::ApiKey,
            "ip" => PrincipalType::Ip,
            "iprange" => PrincipalType::IpRange,
            _ => PrincipalType::Interface,
        };
        (principal_type, pvalue.to_string())
    } else {
        (PrincipalType::Interface, s.to_string())
    }
}

/// Parse permissions string "read,write,admin" into bools
fn parse_permissions(s: &str) -> (bool, bool, bool) {
    let parts: Vec<&str> = s.split(',').collect();
    (
        parts.contains(&"read"),
        parts.contains(&"write"),
        parts.contains(&"admin"),
    )
}

/// Render the ACL management modal
pub fn render(frame: &mut Frame, area: Rect, state: &AclState, theme: &Theme) {
    let title = match state.mode {
        AclModalMode::Edit | AclModalMode::ConfirmSave => {
            format!("ACL Editor: {}", state.edit_collection)
        }
        _ => "ACL Rules".to_string(),
    };

    let (width, height) = match state.mode {
        AclModalMode::Edit | AclModalMode::ConfirmSave => (80, 80),
        _ => (75, 70),
    };

    let inner = render_modal_frame(frame, area, &title, theme, width, height);

    match state.mode {
        AclModalMode::List => render_list_mode(frame, inner, state, theme),
        AclModalMode::ViewCollection => render_collection_mode(frame, inner, state, theme),
        AclModalMode::ConfirmDelete => render_confirm_delete_mode(frame, inner, state, theme),
        AclModalMode::Edit => render_edit_mode(frame, inner, state, theme),
        AclModalMode::ConfirmSave => render_confirm_save_mode(frame, inner, state, theme),
    }
}

fn render_list_mode(frame: &mut Frame, area: Rect, state: &AclState, theme: &Theme) {
    let chunks = Layout::vertical([
        Constraint::Length(2), // Help text
        Constraint::Min(8),    // ACL list
        Constraint::Length(1), // Status/error
    ])
    .split(area);

    // Help text
    let mut help_spans = vec![
        Span::styled("[j/k]", Style::default().fg(theme.accent)),
        Span::raw(" Nav  "),
        Span::styled("[Enter]", Style::default().fg(theme.accent)),
        Span::raw(" View  "),
    ];

    if state.is_localhost {
        help_spans.extend([
            Span::styled("[e]", Style::default().fg(theme.accent)),
            Span::raw(" Edit  "),
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

    // ACL list
    render_acl_list(frame, chunks[1], state, theme);

    // Status/error line
    render_status(frame, chunks[2], state, theme);
}

fn render_acl_list(frame: &mut Frame, area: Rect, state: &AclState, theme: &Theme) {
    let block = Block::default()
        .title(" Collections ")
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

    if state.acls.is_empty() {
        let empty = Paragraph::new("No ACL rules. Press [a] on a collection to add ACL.")
            .style(Style::default().fg(theme.muted));
        frame.render_widget(empty, inner);
        return;
    }

    // Header
    let header_chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(inner);

    let header = Paragraph::new(Line::from(vec![
        Span::styled(format!("{:<24}", "Collection"), Style::default().fg(theme.muted)),
        Span::raw("  "),
        Span::styled(format!("{:<12}", "Rules"), Style::default().fg(theme.muted)),
        Span::raw("  "),
        Span::styled("Type", Style::default().fg(theme.muted)),
    ]));
    frame.render_widget(header, header_chunks[0]);

    // List items
    let items: Vec<ListItem> = state
        .acls
        .iter()
        .enumerate()
        .map(|(i, acl)| {
            let is_selected = i == state.selected;
            let type_str = if acl.is_builtin { "builtin" } else { "custom" };
            let type_color = if acl.is_builtin { theme.muted } else { theme.accent };

            let line = Line::from(vec![
                Span::raw(format!("{:<24}", truncate(&acl.collection, 24))),
                Span::raw("  "),
                Span::raw(format!("{:<12}", format!("{} rules", acl.rules.len()))),
                Span::raw("  "),
                Span::styled(type_str, Style::default().fg(type_color)),
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

fn render_collection_mode(frame: &mut Frame, area: Rect, state: &AclState, theme: &Theme) {
    let acl = match state.get_selected() {
        Some(a) => a,
        None => return,
    };

    let chunks = Layout::vertical([
        Constraint::Length(2), // Help text
        Constraint::Min(8),    // Rules list
        Constraint::Length(1), // Status/error
    ])
    .split(area);

    // Help text
    let help = Paragraph::new(Line::from(vec![
        Span::styled("[j/k]", Style::default().fg(theme.accent)),
        Span::raw(" Nav  "),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(" Back"),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    // Rules list
    let block = Block::default()
        .title(format!(" ACL: {} ", acl.collection))
        .title_style(Style::default().fg(theme.fg))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent));

    let inner = block.inner(chunks[1]);
    frame.render_widget(block, chunks[1]);

    if acl.rules.is_empty() {
        let empty = Paragraph::new("No rules defined").style(Style::default().fg(theme.muted));
        frame.render_widget(empty, inner);
    } else {
        // Header
        let header_chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(inner);

        let header = Paragraph::new(Line::from(vec![
            Span::styled(format!("{:<28}", "Principal"), Style::default().fg(theme.muted)),
            Span::raw("  "),
            Span::styled("Permissions", Style::default().fg(theme.muted)),
        ]));
        frame.render_widget(header, header_chunks[0]);

        let items: Vec<ListItem> = acl
            .rules
            .iter()
            .enumerate()
            .map(|(i, rule)| {
                let is_selected = i == state.selected_rule;

                let line = Line::from(vec![
                    Span::raw(format!("{:<28}", truncate(&rule.principal, 28))),
                    Span::raw("  "),
                    Span::styled(&rule.permissions, Style::default().fg(theme.success)),
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

    // Status
    render_status(frame, chunks[2], state, theme);
}

fn render_edit_mode(frame: &mut Frame, area: Rect, state: &AclState, theme: &Theme) {
    let chunks = Layout::vertical([
        Constraint::Length(2),  // Help text
        Constraint::Min(6),     // Rules list
        Constraint::Length(9),  // Add rule form
        Constraint::Length(1),  // Status
    ])
    .split(area);

    // Help text
    let help_text = if state.is_adding_rule {
        Line::from(vec![
            Span::styled("[Tab]", Style::default().fg(theme.accent)),
            Span::raw(" Next  "),
            Span::styled("[Space]", Style::default().fg(theme.accent)),
            Span::raw(" Toggle  "),
            Span::styled("[Enter]", Style::default().fg(theme.accent)),
            Span::raw(" Add  "),
            Span::styled("[Esc]", Style::default().fg(theme.accent)),
            Span::raw(" Cancel"),
        ])
    } else {
        Line::from(vec![
            Span::styled("[j/k]", Style::default().fg(theme.accent)),
            Span::raw(" Nav  "),
            Span::styled("[n]", Style::default().fg(theme.accent)),
            Span::raw(" New  "),
            Span::styled("[d]", Style::default().fg(theme.accent)),
            Span::raw(" Del  "),
            Span::styled("[Enter]", Style::default().fg(theme.accent)),
            Span::raw(" Save  "),
            Span::styled("[Esc]", Style::default().fg(theme.accent)),
            Span::raw(" Cancel"),
        ])
    };
    let help = Paragraph::new(help_text).style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    // Rules list
    render_edit_rules_list(frame, chunks[1], state, theme);

    // Add rule form
    render_add_rule_form(frame, chunks[2], state, theme);

    // Status
    render_status(frame, chunks[3], state, theme);
}

fn render_edit_rules_list(frame: &mut Frame, area: Rect, state: &AclState, theme: &Theme) {
    let block = Block::default()
        .title(format!(" Rules ({}) ", state.edit_rules.len()))
        .title_style(Style::default().fg(theme.fg))
        .borders(Borders::ALL)
        .border_style(if state.is_adding_rule {
            Style::default().fg(theme.muted)
        } else {
            Style::default().fg(theme.accent)
        });

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.edit_rules.is_empty() {
        let empty = Paragraph::new("No rules yet. Press [n] to add.")
            .style(Style::default().fg(theme.muted));
        frame.render_widget(empty, inner);
        return;
    }

    // Header + list
    let list_chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(inner);

    let header = Paragraph::new(Line::from(vec![
        Span::styled(format!("{:<28}", "Principal"), Style::default().fg(theme.muted)),
        Span::raw("  "),
        Span::styled("Permissions", Style::default().fg(theme.muted)),
    ]));
    frame.render_widget(header, list_chunks[0]);

    let items: Vec<ListItem> = state
        .edit_rules
        .iter()
        .enumerate()
        .map(|(i, rule)| {
            let is_selected = i == state.edit_selected_rule && !state.is_adding_rule;

            let line = Line::from(vec![
                Span::raw(format!("{:<28}", truncate(&rule.display_principal(), 28))),
                Span::raw("  "),
                Span::styled(rule.permissions_string(), Style::default().fg(theme.success)),
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
    frame.render_widget(list, list_chunks[1]);
}

fn render_add_rule_form(frame: &mut Frame, area: Rect, state: &AclState, theme: &Theme) {
    let block = Block::default()
        .title(" Add Rule ")
        .title_style(Style::default().fg(theme.fg))
        .borders(Borders::ALL)
        .border_style(if state.is_adding_rule {
            Style::default().fg(theme.accent)
        } else {
            Style::default().fg(theme.muted)
        });

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let form_chunks = Layout::vertical([
        Constraint::Length(1), // Principal type
        Constraint::Length(1), // Spacer
        Constraint::Length(1), // Principal value
        Constraint::Length(1), // Spacer
        Constraint::Length(1), // Permissions
        Constraint::Min(1),    // Spacer
    ])
    .split(inner);

    let is_active = state.is_adding_rule;

    // Principal type selector
    let type_style = if is_active && state.edit_field == EditField::PrincipalType {
        Style::default().fg(theme.accent).bold()
    } else {
        Style::default().fg(if is_active { theme.fg } else { theme.muted })
    };
    let type_indicator = if is_active && state.edit_field == EditField::PrincipalType { ">" } else { " " };
    let type_line = Line::from(vec![
        Span::raw(type_indicator),
        Span::styled(" Type: ", type_style),
        Span::styled(
            format!("< {} >", state.current_rule.principal_type.as_str()),
            type_style,
        ),
        Span::styled("  (←/→ to change)", Style::default().fg(theme.muted)),
    ]);
    frame.render_widget(Paragraph::new(type_line), form_chunks[0]);

    // Principal value
    let value_style = if is_active && state.edit_field == EditField::PrincipalValue {
        Style::default().fg(theme.accent).bold()
    } else {
        Style::default().fg(if is_active { theme.fg } else { theme.muted })
    };
    let value_indicator = if is_active && state.edit_field == EditField::PrincipalValue { ">" } else { " " };
    let cursor = if is_active && state.edit_field == EditField::PrincipalValue { "_" } else { "" };
    let value_line = Line::from(vec![
        Span::raw(value_indicator),
        Span::styled(" Value: ", value_style),
        Span::styled(
            format!("{}{}", state.current_rule.principal_value, cursor),
            value_style,
        ),
    ]);
    frame.render_widget(Paragraph::new(value_line), form_chunks[2]);

    // Permissions checkboxes
    let perm_chunks = Layout::horizontal([
        Constraint::Length(18),
        Constraint::Length(18),
        Constraint::Length(18),
    ])
    .split(form_chunks[4]);

    let read_style = if is_active && state.edit_field == EditField::PermRead {
        Style::default().fg(theme.accent).bold()
    } else {
        Style::default().fg(if is_active { theme.fg } else { theme.muted })
    };
    let read_check = if state.current_rule.read { "[x]" } else { "[ ]" };
    let read_indicator = if is_active && state.edit_field == EditField::PermRead { ">" } else { " " };
    frame.render_widget(
        Paragraph::new(format!("{} {} Read", read_indicator, read_check)).style(read_style),
        perm_chunks[0],
    );

    let write_style = if is_active && state.edit_field == EditField::PermWrite {
        Style::default().fg(theme.accent).bold()
    } else {
        Style::default().fg(if is_active { theme.fg } else { theme.muted })
    };
    let write_check = if state.current_rule.write { "[x]" } else { "[ ]" };
    let write_indicator = if is_active && state.edit_field == EditField::PermWrite { ">" } else { " " };
    frame.render_widget(
        Paragraph::new(format!("{} {} Write", write_indicator, write_check)).style(write_style),
        perm_chunks[1],
    );

    let admin_style = if is_active && state.edit_field == EditField::PermAdmin {
        Style::default().fg(theme.accent).bold()
    } else {
        Style::default().fg(if is_active { theme.fg } else { theme.muted })
    };
    let admin_check = if state.current_rule.admin { "[x]" } else { "[ ]" };
    let admin_indicator = if is_active && state.edit_field == EditField::PermAdmin { ">" } else { " " };
    frame.render_widget(
        Paragraph::new(format!("{} {} Admin", admin_indicator, admin_check)).style(admin_style),
        perm_chunks[2],
    );
}

fn render_confirm_delete_mode(frame: &mut Frame, area: Rect, state: &AclState, theme: &Theme) {
    let chunks = Layout::vertical([
        Constraint::Length(3), // Message
        Constraint::Length(2), // Buttons
        Constraint::Min(1),    // Spacer
    ])
    .split(area);

    let collection = state
        .get_selected()
        .map(|a| a.collection.as_str())
        .unwrap_or("?");

    let message = Paragraph::new(format!(
        "Are you sure you want to DELETE the ACL for '{}'?\nThis will revert to default permissions.",
        collection
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

fn render_confirm_save_mode(frame: &mut Frame, area: Rect, state: &AclState, theme: &Theme) {
    let chunks = Layout::vertical([
        Constraint::Length(4), // Message
        Constraint::Length(2), // Buttons
        Constraint::Min(1),    // Spacer
    ])
    .split(area);

    let message = Paragraph::new(format!(
        "Save ACL for '{}'?\n\n{} rule(s) will be applied.",
        state.edit_collection,
        state.edit_rules.len()
    ))
    .style(Style::default().fg(theme.fg))
    .wrap(ratatui::widgets::Wrap { trim: true });
    frame.render_widget(message, chunks[0]);

    let buttons = Paragraph::new(Line::from(vec![
        Span::styled("[y]", Style::default().fg(theme.success)),
        Span::raw(" Save  "),
        Span::styled("[n/Esc]", Style::default().fg(theme.accent)),
        Span::raw(" Cancel"),
    ]));
    frame.render_widget(buttons, chunks[1]);
}

fn render_status(frame: &mut Frame, area: Rect, state: &AclState, theme: &Theme) {
    let status = if let Some(ref err) = state.error {
        Paragraph::new(err.as_str()).style(Style::default().fg(theme.error))
    } else if let Some(ref success) = state.success {
        Paragraph::new(success.as_str()).style(Style::default().fg(theme.success))
    } else if !state.is_localhost {
        Paragraph::new("Read-only: ACL editing requires localhost connection")
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
