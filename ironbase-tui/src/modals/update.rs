//! Update modal - check for and display server update information

use super::render_modal_frame;
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

/// Update check state
#[derive(Debug, Clone, Default)]
pub struct UpdateState {
    /// Current server version
    pub current_version: String,
    /// Latest available version from GitHub
    pub latest_version: Option<String>,
    /// Whether an update is available
    pub update_available: bool,
    /// Loading state
    pub loading: bool,
    /// Error message if check failed
    pub error: Option<String>,
    /// Download URL for the update
    pub download_url: Option<String>,
    /// Release notes (truncated)
    pub release_notes: Option<String>,
}

impl UpdateState {
    pub fn new(current_version: String) -> Self {
        Self {
            current_version,
            loading: true,
            ..Default::default()
        }
    }

    /// Update with data from GitHub API
    pub fn update_from_github(&mut self, latest_version: String, download_url: String, release_notes: Option<String>) {
        self.latest_version = Some(latest_version.clone());
        self.download_url = Some(download_url);
        self.release_notes = release_notes;

        // Compare versions (simple string compare, assumes semantic versioning)
        let current = self.current_version.trim_start_matches('v');
        let latest = latest_version.trim_start_matches('v');
        self.update_available = latest > current;

        self.loading = false;
        self.error = None;
    }

    /// Set error state
    pub fn set_error(&mut self, error: String) {
        self.loading = false;
        self.error = Some(error);
    }
}

/// Render the update modal
pub fn render(frame: &mut Frame, area: Rect, state: &UpdateState, theme: &Theme) {
    let inner = render_modal_frame(frame, area, "Frissites Ellenorzes [Esc bezar]", theme, 60, 50);

    if state.loading {
        let loading = Paragraph::new("Frissites ellenorzese...")
            .style(Style::default().fg(theme.fg));
        frame.render_widget(loading, inner);
        return;
    }

    if let Some(ref error) = state.error {
        let error_text = Paragraph::new(format!("Hiba: {}", error))
            .style(Style::default().fg(theme.error));
        frame.render_widget(error_text, inner);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();

    // Current version
    lines.push(Line::from(vec![
        Span::styled("Jelenlegi verzio: ", Style::default().fg(theme.accent)),
        Span::styled(&state.current_version, Style::default().fg(theme.fg).bold()),
    ]));
    lines.push(Line::from(""));

    // Latest version
    if let Some(ref latest) = state.latest_version {
        lines.push(Line::from(vec![
            Span::styled("Legujabb verzio:  ", Style::default().fg(theme.accent)),
            Span::styled(latest, Style::default().fg(theme.fg).bold()),
        ]));
        lines.push(Line::from(""));
    }

    // Update status
    if state.update_available {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Uj verzio elerheto!",
            Style::default().fg(theme.success).bold(),
        )));
        lines.push(Line::from(""));

        // Instructions
        lines.push(Line::from(Span::styled(
            "Frissiteshez futtasd PowerShell-ben:",
            Style::default().fg(theme.secondary),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "irm https://github.com/petitan/IronBase/releases/latest/download/install.ps1 | iex",
            Style::default().fg(theme.accent),
        )));

        // Release notes if available
        if let Some(ref notes) = state.release_notes {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Ujdonsagok:",
                Style::default().fg(theme.accent).bold(),
            )));
            // Show first few lines of release notes
            for line in notes.lines().take(10) {
                lines.push(Line::from(Span::raw(format!("  {}", line))));
            }
        }
    } else {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "A legujabb verzio fut!",
            Style::default().fg(theme.success).bold(),
        )));
    }

    let content = Paragraph::new(lines)
        .style(Style::default().fg(theme.fg));

    frame.render_widget(content, inner);
}
