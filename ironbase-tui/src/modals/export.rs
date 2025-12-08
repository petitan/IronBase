//! Export modal - export documents to JSON/CSV

use super::render_modal_frame;
use crate::app::ExportState;
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

/// Render the export modal
pub fn render(frame: &mut Frame, area: Rect, state: &ExportState, theme: &Theme) {
    let inner = render_modal_frame(frame, area, "Export", theme, 60, 50);

    // Split into sections
    let chunks = Layout::vertical([
        Constraint::Length(2), // Help text
        Constraint::Length(3), // Format selection
        Constraint::Length(3), // File path input
        Constraint::Length(3), // Status info
        Constraint::Min(1),    // Status/error
    ])
    .split(inner);

    // Help text
    let help = Paragraph::new(Line::from(vec![
        Span::styled("[Tab]", Style::default().fg(theme.accent)),
        Span::raw(" Format  "),
        Span::styled("[Shift+J/C]", Style::default().fg(theme.accent)),
        Span::raw(" JSON/CSV  "),
        Span::styled("[Enter]", Style::default().fg(theme.accent)),
        Span::raw(" Export  "),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(" Close"),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    // Format selection
    let json_style = if state.format == ExportFormat::Json {
        Style::default().fg(theme.accent).bold()
    } else {
        Style::default().fg(theme.muted)
    };
    let csv_style = if state.format == ExportFormat::Csv {
        Style::default().fg(theme.accent).bold()
    } else {
        Style::default().fg(theme.muted)
    };

    let format_line = Line::from(vec![
        Span::raw(" Formatum: "),
        Span::styled("[J] JSON", json_style),
        Span::raw("  "),
        Span::styled("[C] CSV", csv_style),
    ]);
    frame.render_widget(Paragraph::new(format_line), chunks[1]);

    // File path input
    let cursor = if state.editing_path { "|" } else { "" };
    let path_line = Line::from(vec![
        Span::raw(" Fajl: "),
        Span::styled(&state.file_path, Style::default().fg(theme.fg)),
        Span::styled(cursor, Style::default().fg(theme.accent)),
    ]);
    frame.render_widget(Paragraph::new(path_line), chunks[2]);

    // Status info
    let info_line = Line::from(vec![Span::styled(
        format!(
            " Kollekció: {} ({} dokumentum)",
            state.collection, state.doc_count
        ),
        Style::default().fg(theme.muted),
    )]);
    frame.render_widget(Paragraph::new(info_line), chunks[3]);

    // Status/error line
    let status_text = if let Some(ref err) = state.error {
        Line::from(vec![
            Span::styled(" Hiba: ", Style::default().fg(theme.error)),
            Span::styled(err.clone(), Style::default().fg(theme.error)),
        ])
    } else if let Some(ref msg) = state.message {
        Line::from(vec![Span::styled(
            format!(" {}", msg),
            Style::default().fg(theme.accent),
        )])
    } else {
        Line::from(vec![Span::styled(
            " Nyomd meg Enter-t az exportáláshoz",
            Style::default().fg(theme.muted),
        )])
    };
    frame.render_widget(Paragraph::new(status_text), chunks[4]);
}

/// Export format enum
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExportFormat {
    #[default]
    Json,
    Csv,
}

impl ExportFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            ExportFormat::Json => "json",
            ExportFormat::Csv => "csv",
        }
    }
}
