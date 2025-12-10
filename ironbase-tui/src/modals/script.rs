//! IronRhai Script Editor modal - browse, edit, run scripts

use super::{centered_rect, render_modal_frame};
use crate::app::{ScriptConfirmAction, ScriptFocus, ScriptMode, ScriptState};
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

/// Render the script modal (dispatches to correct mode)
pub fn render(frame: &mut Frame, area: Rect, state: &ScriptState, theme: &Theme) {
    // First render the main content
    match state.mode {
        ScriptMode::Browse => render_browse(frame, area, state, theme),
        ScriptMode::Edit | ScriptMode::New | ScriptMode::Inline => {
            render_editor(frame, area, state, theme)
        }
        ScriptMode::History => render_history(frame, area, state, theme),
    }

    // Render confirmation dialog on top if active
    if let Some(action) = &state.confirm_action {
        render_confirm_dialog(frame, area, action, state, theme);
    }

    // Render loading overlay if loading
    if state.loading {
        render_loading_overlay(frame, area, theme);
    }
}

/// Render confirmation dialog overlay
fn render_confirm_dialog(
    frame: &mut Frame,
    area: Rect,
    action: &ScriptConfirmAction,
    state: &ScriptState,
    theme: &Theme,
) {
    let dialog_area = centered_rect(40, 20, area);
    frame.render_widget(Clear, dialog_area);

    let (title, message) = match action {
        ScriptConfirmAction::DiscardChanges => (
            "Elvetés megerősítése",
            "Biztosan elveted a nem mentett változtatásokat?",
        ),
        ScriptConfirmAction::DeleteScript => (
            "Törlés megerősítése",
            "Biztosan törölni akarod ezt a scriptet?",
        ),
    };

    let block = Block::default()
        .title(format!(" {} ", title))
        .title_style(Style::default().fg(theme.warning).bold())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.warning))
        .style(Style::default().bg(theme.bg));

    let inner = block.inner(dialog_area);
    frame.render_widget(block, dialog_area);

    let script_info = if matches!(action, ScriptConfirmAction::DeleteScript) {
        state
            .scripts
            .get(state.selected_script)
            .map(|s| format!("\n\nScript: \"{}\"", s.name))
            .unwrap_or_default()
    } else {
        String::new()
    };

    let text = Paragraph::new(vec![
        Line::from(message),
        Line::from(script_info),
        Line::from(""),
        Line::from(vec![
            Span::styled("[Y]", Style::default().fg(theme.success).bold()),
            Span::raw(" Igen  "),
            Span::styled("[N/Esc]", Style::default().fg(theme.error).bold()),
            Span::raw(" Nem"),
        ]),
    ])
    .alignment(Alignment::Center)
    .style(Style::default().fg(theme.fg));

    frame.render_widget(text, inner);
}

/// Render loading overlay
fn render_loading_overlay(frame: &mut Frame, area: Rect, theme: &Theme) {
    let spinner_area = centered_rect(20, 10, area);
    frame.render_widget(Clear, spinner_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent))
        .style(Style::default().bg(theme.bg));

    let inner = block.inner(spinner_area);
    frame.render_widget(block, spinner_area);

    let loading = Paragraph::new("Betöltés...")
        .alignment(Alignment::Center)
        .style(Style::default().fg(theme.accent));
    frame.render_widget(loading, inner);
}

/// Browse mode - script list
fn render_browse(frame: &mut Frame, area: Rect, state: &ScriptState, theme: &Theme) {
    let inner = render_modal_frame(frame, area, "IronRhai Scripts", theme, 80, 70);

    let chunks = Layout::vertical([
        Constraint::Length(2), // Help
        Constraint::Min(5),    // Script list
        Constraint::Length(3), // Selected script info
    ])
    .split(inner);

    // Help text
    let help = Paragraph::new(Line::from(vec![
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(" Bezar  "),
        Span::styled("[n]", Style::default().fg(theme.accent)),
        Span::raw(" Uj  "),
        Span::styled("[Enter]", Style::default().fg(theme.accent)),
        Span::raw(" Szerk  "),
        Span::styled("[d]", Style::default().fg(theme.accent)),
        Span::raw(" Torol  "),
        Span::styled("[F5]", Style::default().fg(theme.accent)),
        Span::raw(" Futtat  "),
        Span::styled("[h]", Style::default().fg(theme.accent)),
        Span::raw(" History  "),
        Span::styled("[i]", Style::default().fg(theme.accent)),
        Span::raw(" Inline  "),
        Span::styled("[r]", Style::default().fg(theme.accent)),
        Span::raw(" Frissit"),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    // Script list
    if state.scripts.is_empty() {
        let empty = Paragraph::new("Nincsenek scriptek. Nyomj [n]-t uj script letrehozasahoz.")
            .style(Style::default().fg(theme.muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty, chunks[1]);
    } else {
        let visible_height = chunks[1].height as usize;
        let scroll_offset = if state.selected_script >= visible_height {
            state.selected_script - visible_height + 1
        } else {
            0
        };

        let mut lines: Vec<Line> = Vec::new();
        for (idx, script) in state.scripts.iter().enumerate().skip(scroll_offset).take(visible_height) {
            let is_selected = idx == state.selected_script;
            let prefix = if is_selected { "> " } else { "  " };

            // Format: name  v3  [tag1][tag2]  12 runs
            let tags_str: String = script.tags.iter().map(|t| format!("[{}]", t)).collect::<Vec<_>>().join("");

            let line = Line::from(vec![
                Span::styled(
                    prefix,
                    Style::default().fg(if is_selected { theme.accent } else { theme.fg }),
                ),
                Span::styled(
                    format!("{:<24}", script.name),
                    Style::default()
                        .fg(if is_selected { theme.accent } else { theme.fg })
                        .add_modifier(if is_selected { Modifier::BOLD } else { Modifier::empty() }),
                ),
                Span::styled(format!(" v{:<3}", script.version), Style::default().fg(theme.muted)),
                Span::styled(format!(" {:<20}", tags_str), Style::default().fg(theme.secondary)),
                Span::styled(
                    format!("{:>5} runs", script.execution_count),
                    Style::default().fg(theme.muted),
                ),
            ]);
            lines.push(line);
        }

        let list = Paragraph::new(lines);
        frame.render_widget(list, chunks[1]);
    }

    // Selected script info
    if let Some(script) = state.scripts.get(state.selected_script) {
        let desc = script.description.as_deref().unwrap_or("(nincs leiras)");
        let last_run = script.last_run_at.as_deref().unwrap_or("soha");
        let info = Paragraph::new(vec![
            Line::from(vec![
                Span::styled("Leiras: ", Style::default().fg(theme.muted)),
                Span::raw(desc),
            ]),
            Line::from(vec![
                Span::styled("Utolso futtas: ", Style::default().fg(theme.muted)),
                Span::raw(last_run),
            ]),
        ]);
        frame.render_widget(info, chunks[2]);
    }
}

/// Editor mode - edit/new/inline script
fn render_editor(frame: &mut Frame, area: Rect, state: &ScriptState, theme: &Theme) {
    let title = match state.mode {
        ScriptMode::New => "IronRhai - Uj script",
        ScriptMode::Inline => "IronRhai - Inline exec",
        _ => "IronRhai Editor",
    };
    let inner = render_modal_frame(frame, area, title, theme, 85, 80);

    let chunks = Layout::vertical([
        Constraint::Length(2), // Help
        Constraint::Length(3), // Metadata (name, desc, tags)
        Constraint::Min(8),    // Code editor
        Constraint::Length(2), // Params
        Constraint::Length(4), // Result/logs
        Constraint::Length(1), // Status
    ])
    .split(inner);

    // Help text
    let help = Paragraph::new(Line::from(vec![
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(" Vissza  "),
        Span::styled("[F5]", Style::default().fg(theme.accent)),
        Span::raw(" Futtat  "),
        Span::styled("[Ctrl+S]", Style::default().fg(theme.accent)),
        Span::raw(" Ment  "),
        Span::styled("[Tab]", Style::default().fg(theme.accent)),
        Span::raw(" Fokusz  "),
        Span::styled("[h]", Style::default().fg(theme.accent)),
        Span::raw(" History"),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    // Metadata section
    render_metadata(frame, chunks[1], state, theme);

    // Code editor
    render_code_editor(frame, chunks[2], state, theme);

    // Params input
    render_params(frame, chunks[3], state, theme);

    // Result/logs
    render_result(frame, chunks[4], state, theme);

    // Status line
    render_status(frame, chunks[5], state, theme);
}

fn render_metadata(frame: &mut Frame, area: Rect, state: &ScriptState, theme: &Theme) {
    let chunks = Layout::horizontal([
        Constraint::Percentage(30), // Name
        Constraint::Percentage(40), // Description
        Constraint::Percentage(30), // Tags
    ])
    .split(area);

    // Name field
    let name_style = if state.focus == ScriptFocus::Name {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.fg)
    };
    let name_cursor = if state.focus == ScriptFocus::Name { "|" } else { "" };
    let name_text = format!("Nev: {}{}", state.name, name_cursor);
    let name = Paragraph::new(name_text).style(name_style);
    frame.render_widget(name, chunks[0]);

    // Description field
    let desc_style = if state.focus == ScriptFocus::Description {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.fg)
    };
    let desc_cursor = if state.focus == ScriptFocus::Description { "|" } else { "" };
    let desc_text = format!("Leiras: {}{}", state.description, desc_cursor);
    let desc = Paragraph::new(desc_text).style(desc_style);
    frame.render_widget(desc, chunks[1]);

    // Tags field
    let tags_style = if state.focus == ScriptFocus::Tags {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.fg)
    };
    let mut tag_spans: Vec<Span> = vec![Span::raw("Tagek: ")];
    for (i, tag) in state.tags.iter().enumerate() {
        let is_selected = state.focus == ScriptFocus::Tags && i == state.selected_tag;
        tag_spans.push(Span::styled(
            format!("[{}]", tag),
            if is_selected {
                Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.secondary)
            },
        ));
    }
    if state.tag_input_active {
        tag_spans.push(Span::styled(
            format!("[{}|]", state.tag_input),
            Style::default().fg(theme.accent),
        ));
    } else if state.focus == ScriptFocus::Tags {
        tag_spans.push(Span::styled("[+]", Style::default().fg(theme.muted)));
    }
    let tags = Paragraph::new(Line::from(tag_spans)).style(tags_style);
    frame.render_widget(tags, chunks[2]);
}

fn render_code_editor(frame: &mut Frame, area: Rect, state: &ScriptState, theme: &Theme) {
    let is_focused = state.focus == ScriptFocus::Editor;
    let border_color = if is_focused { theme.accent } else { theme.muted };

    let visible_height = area.height.saturating_sub(2) as usize; // account for border
    let start_line = state.scroll_offset;

    let mut lines: Vec<Line> = Vec::new();
    for (idx, line) in state.lines.iter().enumerate().skip(start_line).take(visible_height) {
        let line_num = format!("{:3}| ", idx + 1);
        let is_cursor_line = idx == state.cursor_line;

        if is_cursor_line && is_focused {
            // Show cursor with highlighting
            let before: String = line.chars().take(state.cursor_col).collect();
            let after: String = line.chars().skip(state.cursor_col).collect();

            let mut spans = vec![Span::styled(line_num, Style::default().fg(theme.muted))];
            spans.extend(highlight_rhai_spans(&before, theme));
            spans.push(Span::styled("|", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)));
            spans.extend(highlight_rhai_spans(&after, theme));

            lines.push(Line::from(spans));
        } else {
            // No cursor - full line highlighting
            let mut spans = vec![Span::styled(line_num, Style::default().fg(theme.muted))];
            spans.extend(highlight_rhai_spans(line, theme));
            lines.push(Line::from(spans));
        }
    }

    let editor = Paragraph::new(lines)
        .block(
            ratatui::widgets::Block::default()
                .borders(ratatui::widgets::Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .title(" Kod ")
                .title_style(Style::default().fg(border_color)),
        )
        .style(Style::default().bg(theme.bg));
    frame.render_widget(editor, area);
}

/// Rhai syntax highlighting - returns spans for a single line
fn highlight_rhai_spans(line: &str, theme: &Theme) -> Vec<Span<'static>> {
    use ratatui::style::Color;

    // Keywords
    const KEYWORDS: &[&str] = &[
        "let", "const", "fn", "if", "else", "for", "while", "loop", "break", "continue",
        "return", "throw", "try", "catch", "in", "true", "false", "nil", "null",
        "import", "export", "as", "private", "this", "switch", "case", "default",
    ];

    // DB function names (IronRhai specific)
    const DB_FUNCS: &[&str] = &[
        "db_find", "db_find_one", "db_insert_one", "db_update_one", "db_update_many",
        "db_delete_one", "db_delete_many", "db_count", "db_aggregate",
        "base64_encode", "base64_decode", "print",
    ];

    let keyword_color = Color::Magenta;
    let string_color = Color::Green;
    let number_color = Color::Yellow;
    let comment_color = theme.muted;
    let func_color = Color::Cyan;
    let normal_color = theme.fg;

    let mut spans: Vec<Span<'static>> = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Check for comment
        if i + 1 < len && chars[i] == '/' && chars[i + 1] == '/' {
            // Line comment - rest of line
            let rest: String = chars[i..].iter().collect();
            spans.push(Span::styled(rest, Style::default().fg(comment_color)));
            break;
        }

        // Check for string (double or single quote)
        if chars[i] == '"' || chars[i] == '\'' {
            let quote = chars[i];
            let mut j = i + 1;
            while j < len && chars[j] != quote {
                if chars[j] == '\\' && j + 1 < len {
                    j += 2; // Skip escaped char
                } else {
                    j += 1;
                }
            }
            if j < len {
                j += 1; // Include closing quote
            }
            let s: String = chars[i..j].iter().collect();
            spans.push(Span::styled(s, Style::default().fg(string_color)));
            i = j;
            continue;
        }

        // Check for number
        if chars[i].is_ascii_digit() || (chars[i] == '-' && i + 1 < len && chars[i + 1].is_ascii_digit()) {
            let mut j = i;
            if chars[j] == '-' {
                j += 1;
            }
            while j < len && (chars[j].is_ascii_digit() || chars[j] == '.' || chars[j] == '_') {
                j += 1;
            }
            let s: String = chars[i..j].iter().collect();
            spans.push(Span::styled(s, Style::default().fg(number_color)));
            i = j;
            continue;
        }

        // Check for identifier (keyword, function, or variable)
        if chars[i].is_alphabetic() || chars[i] == '_' {
            let mut j = i;
            while j < len && (chars[j].is_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            let word: String = chars[i..j].iter().collect();

            let style = if KEYWORDS.contains(&word.as_str()) {
                Style::default().fg(keyword_color).add_modifier(Modifier::BOLD)
            } else if DB_FUNCS.contains(&word.as_str()) {
                Style::default().fg(func_color)
            } else {
                Style::default().fg(normal_color)
            };

            spans.push(Span::styled(word, style));
            i = j;
            continue;
        }

        // Other character (operators, brackets, etc.)
        spans.push(Span::styled(chars[i].to_string(), Style::default().fg(normal_color)));
        i += 1;
    }

    spans
}

fn render_params(frame: &mut Frame, area: Rect, state: &ScriptState, theme: &Theme) {
    let is_focused = state.focus == ScriptFocus::Params;
    let style = if is_focused {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.fg)
    };
    let cursor = if is_focused { "|" } else { "" };
    let params_text = format!("Parameterek (JSON): {}{}", state.params_input, cursor);
    let params = Paragraph::new(params_text).style(style);
    frame.render_widget(params, area);
}

fn render_result(frame: &mut Frame, area: Rect, state: &ScriptState, theme: &Theme) {
    if let Some(ref result) = state.result {
        let result_style = if result.success {
            Style::default().fg(theme.success)
        } else {
            Style::default().fg(theme.error)
        };

        // Show error message if present, otherwise show result
        let output_display = if let Some(ref err) = result.error {
            err.clone()
        } else {
            let result_str = serde_json::to_string(&result.result).unwrap_or_default();
            if result_str.len() > 60 {
                format!("{}...", &result_str[..60])
            } else {
                result_str
            }
        };

        let log_count = result.logs.len();
        let log_preview = if !result.logs.is_empty() {
            format!(" | Logok: [{}] {}", log_count, result.logs.first().unwrap_or(&String::new()))
        } else {
            String::new()
        };

        let text = Paragraph::new(vec![
            Line::from(vec![
                Span::styled(
                    if result.success { "OK" } else { "HIBA" },
                    result_style.add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" ({}ms)", result.execution_time_ms), Style::default().fg(theme.muted)),
                Span::raw(": "),
                Span::styled(output_display, if result.success { Style::default().fg(theme.fg) } else { Style::default().fg(theme.error) }),
            ]),
            Line::from(Span::styled(log_preview, Style::default().fg(theme.muted))),
        ]);
        frame.render_widget(text, area);
    } else {
        let placeholder = Paragraph::new("Eredmeny: (nyomj F5-ot a futtatashoz)")
            .style(Style::default().fg(theme.muted));
        frame.render_widget(placeholder, area);
    }
}

fn render_status(frame: &mut Frame, area: Rect, state: &ScriptState, theme: &Theme) {
    let text = if let Some(ref err) = state.error {
        Line::from(vec![
            Span::styled("Hiba: ", Style::default().fg(theme.error)),
            Span::styled(err.clone(), Style::default().fg(theme.error)),
        ])
    } else if let Some(ref msg) = state.message {
        Line::from(Span::styled(msg.clone(), Style::default().fg(theme.success)))
    } else {
        let dirty_marker = if state.dirty { " [*]" } else { "" };
        Line::from(vec![
            Span::styled(
                format!("v{}{}", state.version, dirty_marker),
                Style::default().fg(theme.muted),
            ),
            Span::raw(" | "),
            Span::styled(
                format!("Sor: {} Oszlop: {}", state.cursor_line + 1, state.cursor_col + 1),
                Style::default().fg(theme.muted),
            ),
        ])
    };

    let status = Paragraph::new(text);
    frame.render_widget(status, area);
}

/// History mode - version list
fn render_history(frame: &mut Frame, area: Rect, state: &ScriptState, theme: &Theme) {
    let title = format!("IronRhai History: {}", state.name);
    let inner = render_modal_frame(frame, area, &title, theme, 80, 70);

    let chunks = Layout::vertical([
        Constraint::Length(2), // Help
        Constraint::Min(5),    // Version list
        Constraint::Length(6), // Code preview
    ])
    .split(inner);

    // Help text
    let help = Paragraph::new(Line::from(vec![
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(" Vissza  "),
        Span::styled("[Enter]", Style::default().fg(theme.accent)),
        Span::raw(" Megtekint  "),
        Span::styled("[r]", Style::default().fg(theme.accent)),
        Span::raw(" Rollback"),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    // Version list
    if state.versions.is_empty() {
        let empty = Paragraph::new("Nincs verzio tortenet.")
            .style(Style::default().fg(theme.muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty, chunks[1]);
    } else {
        let visible_height = chunks[1].height as usize;
        let mut lines: Vec<Line> = Vec::new();

        for (idx, ver) in state.versions.iter().enumerate().take(visible_height) {
            let is_selected = idx == state.selected_version;
            let prefix = if is_selected { "> " } else { "  " };
            let current_marker = if idx == 0 { " (current)" } else { "" };

            let desc = ver.description.as_deref().unwrap_or("");
            let line = Line::from(vec![
                Span::styled(
                    prefix,
                    Style::default().fg(if is_selected { theme.accent } else { theme.fg }),
                ),
                Span::styled(
                    format!("v{}", ver.version),
                    Style::default()
                        .fg(if is_selected { theme.accent } else { theme.fg })
                        .add_modifier(if is_selected { Modifier::BOLD } else { Modifier::empty() }),
                ),
                Span::styled(current_marker, Style::default().fg(theme.success)),
                Span::styled(format!("  {}", ver.created_at), Style::default().fg(theme.muted)),
                Span::raw("  "),
                Span::styled(desc, Style::default().fg(theme.fg)),
            ]);
            lines.push(line);
        }

        let list = Paragraph::new(lines);
        frame.render_widget(list, chunks[1]);
    }

    // Code preview
    if let Some(ver) = state.versions.get(state.selected_version) {
        let preview_lines: Vec<Line> = ver
            .code
            .lines()
            .take(5)
            .enumerate()
            .map(|(i, line)| {
                Line::from(vec![
                    Span::styled(format!("{:3}| ", i + 1), Style::default().fg(theme.muted)),
                    Span::styled(line, Style::default().fg(theme.fg)),
                ])
            })
            .collect();

        let preview = Paragraph::new(preview_lines)
            .block(
                ratatui::widgets::Block::default()
                    .borders(ratatui::widgets::Borders::ALL)
                    .border_style(Style::default().fg(theme.muted))
                    .title(" Kod elonezet ")
                    .title_style(Style::default().fg(theme.muted)),
            )
            .style(Style::default().bg(theme.bg));
        frame.render_widget(preview, chunks[2]);
    }
}
