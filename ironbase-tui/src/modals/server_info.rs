//! Server Info modal - MCP server information display

use super::render_modal_frame;
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;
use serde_json::Value;

/// Server info state - cached data from MCP server
#[derive(Debug, Clone, Default)]
pub struct ServerInfoState {
    /// Database statistics
    pub db_stats: Option<Value>,
    /// List of available tools
    pub tools: Vec<ToolInfo>,
    /// List of available prompts
    pub prompts: Vec<PromptInfo>,
    /// Total lines for scroll calculation
    pub total_lines: usize,
    /// Loading state
    pub loading: bool,
    /// Error message if loading failed
    pub error: Option<String>,
}

/// Tool information
#[derive(Debug, Clone)]
pub struct ToolInfo {
    pub name: String,
    pub title: Option<String>,
    #[allow(dead_code)]
    pub description: Option<String>,
}

/// Prompt information
#[derive(Debug, Clone)]
pub struct PromptInfo {
    pub name: String,
    pub description: Option<String>,
}

impl ServerInfoState {
    pub fn new() -> Self {
        Self {
            loading: true,
            ..Default::default()
        }
    }

    /// Update with data from MCP server
    pub fn update(&mut self, db_stats: Value, tools: Vec<Value>, prompts: Vec<Value>) {
        self.db_stats = Some(db_stats);

        // Parse tools
        self.tools = tools
            .into_iter()
            .map(|t| ToolInfo {
                name: t
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
                    .to_string(),
                title: t
                    .get("title")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                description: t
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            })
            .collect();

        // Parse prompts
        self.prompts = prompts
            .into_iter()
            .map(|p| PromptInfo {
                name: p
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
                    .to_string(),
                description: p
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            })
            .collect();

        // Calculate total lines
        self.total_lines = self.calculate_total_lines();
        self.loading = false;
        self.error = None;
    }

    /// Set error state
    pub fn set_error(&mut self, error: String) {
        self.loading = false;
        self.error = Some(error);
    }

    fn calculate_total_lines(&self) -> usize {
        let mut lines = 0;

        // Header + DB stats section
        lines += 10; // Header, separator, db info lines

        // Tools section
        lines += 3; // Header
        lines += self.tools.len();

        // Prompts section
        lines += 3; // Header
        lines += self.prompts.len();

        // Footer
        lines += 2;

        lines
    }
}

/// Render the server info modal
pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &ServerInfoState,
    scroll: usize,
    theme: &Theme,
) {
    let inner = render_modal_frame(
        frame,
        area,
        "Szerver Info [j/k gorgetes, Esc bezar]",
        theme,
        70,
        80,
    );

    if state.loading {
        let loading = Paragraph::new("Betoltes...").style(Style::default().fg(theme.fg));
        frame.render_widget(loading, inner);
        return;
    }

    if let Some(ref error) = state.error {
        let error_text =
            Paragraph::new(format!("Hiba: {}", error)).style(Style::default().fg(theme.error));
        frame.render_widget(error_text, inner);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();

    // === Database Info Section ===
    lines.push(Line::from(Span::styled(
        "Adatbazis Informaciok",
        Style::default().fg(theme.accent).bold(),
    )));
    lines.push(Line::from(""));

    if let Some(ref stats) = state.db_stats {
        // Server info section
        if let Some(server) = stats.get("server") {
            if let Some(protocol) = server.get("protocol").and_then(|v| v.as_str()) {
                let host = server.get("host").and_then(|v| v.as_str()).unwrap_or("?");
                let port = server.get("port").and_then(|v| v.as_u64()).unwrap_or(0);
                lines.push(Line::from(vec![
                    Span::styled("Szerver:            ", Style::default().fg(theme.accent)),
                    Span::raw(format!("{}://{}:{}", protocol, host, port)),
                ]));
            }
            if let Some(require_key) = server.get("require_api_key").and_then(|v| v.as_bool()) {
                lines.push(Line::from(vec![
                    Span::styled("API kulcs:          ", Style::default().fg(theme.accent)),
                    Span::raw(if require_key {
                        "Kotelezo"
                    } else {
                        "Nem szukseges"
                    }),
                ]));
            }
        }

        // Database info section
        if let Some(database) = stats.get("database") {
            if let Some(path) = database.get("path").and_then(|v| v.as_str()) {
                lines.push(Line::from(vec![
                    Span::styled("Adatbazis:          ", Style::default().fg(theme.accent)),
                    Span::raw(path),
                ]));
            }
            if let Some(size) = database.get("file_size").and_then(|v| v.as_str()) {
                lines.push(Line::from(vec![
                    Span::styled("Fajlmeret:          ", Style::default().fg(theme.accent)),
                    Span::raw(size),
                ]));
            }
        }

        // Summary section
        if let Some(summary) = stats.get("summary") {
            if let Some(count) = summary.get("collection_count").and_then(|v| v.as_u64()) {
                lines.push(Line::from(vec![
                    Span::styled("Kollekcio szam:     ", Style::default().fg(theme.accent)),
                    Span::raw(count.to_string()),
                ]));
            }
            if let Some(docs) = summary.get("total_documents").and_then(|v| v.as_u64()) {
                lines.push(Line::from(vec![
                    Span::styled("Osszes dokumentum:  ", Style::default().fg(theme.accent)),
                    Span::raw(docs.to_string()),
                ]));
            }
            if let Some(indexes) = summary.get("total_indexes").and_then(|v| v.as_u64()) {
                lines.push(Line::from(vec![
                    Span::styled("Osszes index:       ", Style::default().fg(theme.accent)),
                    Span::raw(indexes.to_string()),
                ]));
            }
        }

        // Collection names (new structure: array of objects with "name" field)
        if let Some(collections) = stats.get("collections").and_then(|v| v.as_array()) {
            let names: Vec<&str> = collections
                .iter()
                .filter_map(|v| v.get("name").and_then(|n| n.as_str()))
                .collect();
            if !names.is_empty() {
                lines.push(Line::from(vec![
                    Span::styled("Kollekciok:         ", Style::default().fg(theme.accent)),
                    Span::raw(names.join(", ")),
                ]));
            }
        }
    }

    lines.push(Line::from(""));

    // === MCP Tools Section ===
    lines.push(Line::from(Span::styled(
        format!("Elerheto MCP Eszkozok ({})", state.tools.len()),
        Style::default().fg(theme.accent).bold(),
    )));
    lines.push(Line::from(""));

    // Group tools by category (based on name prefix)
    let categories = vec![
        ("db_", "Adatbazis"),
        ("collection_", "Kollekciok"),
        ("find", "Lekerdezes"),
        ("insert", "Beszuras"),
        ("update", "Frissites"),
        ("delete", "Torles"),
        ("count", "Szamolas"),
        ("aggregate", "Aggregacio"),
        ("index_", "Indexek"),
        ("schema_", "Sema"),
        ("script_", "Szkriptek"),
        ("fuzzy_", "Fuzzy kereses"),
    ];

    // First, show categorized tools
    let mut shown_tools: std::collections::HashSet<&str> = std::collections::HashSet::new();

    for (prefix, category_name) in &categories {
        let category_tools: Vec<&ToolInfo> = state
            .tools
            .iter()
            .filter(|t| t.name.starts_with(prefix))
            .collect();

        if !category_tools.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("  {} ", category_name),
                Style::default().fg(theme.secondary).italic(),
            )));

            for tool in category_tools {
                shown_tools.insert(&tool.name);
                let title = tool.title.as_deref().unwrap_or(&tool.name);
                lines.push(Line::from(vec![
                    Span::styled("    ", Style::default()),
                    Span::styled(&tool.name, Style::default().fg(theme.accent)),
                    Span::raw(" - "),
                    Span::raw(title),
                ]));
            }
        }
    }

    // Show remaining (uncategorized) tools
    let remaining: Vec<&ToolInfo> = state
        .tools
        .iter()
        .filter(|t| !shown_tools.contains(t.name.as_str()))
        .collect();

    if !remaining.is_empty() {
        lines.push(Line::from(Span::styled(
            "  Egyeb ",
            Style::default().fg(theme.secondary).italic(),
        )));
        for tool in remaining {
            let title = tool.title.as_deref().unwrap_or(&tool.name);
            lines.push(Line::from(vec![
                Span::styled("    ", Style::default()),
                Span::styled(&tool.name, Style::default().fg(theme.accent)),
                Span::raw(" - "),
                Span::raw(title),
            ]));
        }
    }

    lines.push(Line::from(""));

    // === MCP Prompts Section ===
    lines.push(Line::from(Span::styled(
        format!("Elerheto MCP Promptok ({})", state.prompts.len()),
        Style::default().fg(theme.accent).bold(),
    )));
    lines.push(Line::from(""));

    for prompt in &state.prompts {
        let desc = prompt.description.as_deref().unwrap_or("Nincs leiras");
        // Truncate description if too long
        let desc_truncated = if desc.chars().count() > 50 {
            format!("{}...", desc.chars().take(47).collect::<String>())
        } else {
            desc.to_string()
        };

        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(&prompt.name, Style::default().fg(theme.accent)),
            Span::raw(" - "),
            Span::raw(desc_truncated),
        ]));
    }

    lines.push(Line::from(""));

    // === Footer ===
    lines.push(Line::from(Span::styled(
        "MCP Protocol Version: 2024-11-05",
        Style::default().fg(theme.secondary).italic(),
    )));

    let content = Paragraph::new(lines)
        .style(Style::default().fg(theme.fg))
        .scroll((scroll as u16, 0));

    frame.render_widget(content, inner);
}
