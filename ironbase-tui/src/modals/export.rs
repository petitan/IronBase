//! Export modal - export documents to JSON/CSV

use super::render_modal_frame;
use crate::app::ExportState;
use crate::i18n::t;
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

/// Render the export modal
pub fn render(frame: &mut Frame, area: Rect, state: &ExportState, theme: &Theme) {
    let inner = render_modal_frame(frame, area, t().export_title(), theme, 60, 50);

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
        Span::raw(format!(" {}  ", t().key_format())),
        Span::styled("[Enter]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_export())),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}", t().key_close())),
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
        Span::raw(format!(" {}: ", t().label_format())),
        Span::styled("JSON", json_style),
        Span::raw("  "),
        Span::styled("CSV", csv_style),
    ]);
    frame.render_widget(Paragraph::new(format_line), chunks[1]);

    // File path input
    state.file_path.render_labeled(
        frame,
        chunks[2],
        format!(" {}: ", t().label_file()),
        Style::default().fg(theme.fg),
        true,
    );

    // Status info
    let info_line = Line::from(vec![Span::styled(
        format!(
            " {}",
            t().export_collection_info(&state.collection, state.doc_count, state.filtered)
        ),
        Style::default().fg(theme.muted),
    )]);
    frame.render_widget(Paragraph::new(info_line), chunks[3]);

    // Status/error line
    let status_text = if let Some(ref err) = state.error {
        Line::from(vec![
            Span::styled(
                format!(" {}: ", t().label_error()),
                Style::default().fg(theme.error),
            ),
            Span::styled(err.clone(), Style::default().fg(theme.error)),
        ])
    } else if let Some(ref msg) = state.message {
        Line::from(vec![Span::styled(
            format!(" {}", msg),
            Style::default().fg(theme.accent),
        )])
    } else {
        Line::from(vec![Span::styled(
            format!(" {}", t().press_enter_to_export()),
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
