//! Help modal - keyboard shortcuts reference

use super::render_modal_frame;
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

/// Total number of help lines (for scroll bounds)
pub const HELP_LINES: usize = 34;

/// Render the help modal
pub fn render(frame: &mut Frame, area: Rect, scroll: usize, theme: &Theme) {
    let inner = render_modal_frame(frame, area, "Sugo [j/k gorgetes]", theme, 60, 70);

    let help_text = vec![
        Line::from(Span::styled(
            "Navigacio",
            Style::default().fg(theme.accent).bold(),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Tab       ", Style::default().fg(theme.accent)),
            Span::raw("Kovetkezo panel"),
        ]),
        Line::from(vec![
            Span::styled("Shift+Tab ", Style::default().fg(theme.accent)),
            Span::raw("Elozo panel"),
        ]),
        Line::from(vec![
            Span::styled("↑/↓ j/k   ", Style::default().fg(theme.accent)),
            Span::raw("Listaban mozgas"),
        ]),
        Line::from(vec![
            Span::styled("PgUp/PgDn ", Style::default().fg(theme.accent)),
            Span::raw("Lapozas"),
        ]),
        Line::from(vec![
            Span::styled("Home/g    ", Style::default().fg(theme.accent)),
            Span::raw("Lista elejere"),
        ]),
        Line::from(vec![
            Span::styled("End/G     ", Style::default().fg(theme.accent)),
            Span::raw("Lista vegere"),
        ]),
        Line::from(vec![
            Span::styled("Enter     ", Style::default().fg(theme.accent)),
            Span::raw("Kivalaszt"),
        ]),
        Line::from(vec![
            Span::styled("Esc       ", Style::default().fg(theme.accent)),
            Span::raw("Vissza / Bezar"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Funkciok",
            Style::default().fg(theme.accent).bold(),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("/         ", Style::default().fg(theme.accent)),
            Span::raw("Kereses megnyitasa"),
        ]),
        Line::from(vec![
            Span::styled("a         ", Style::default().fg(theme.accent)),
            Span::raw("Akciok menu"),
        ]),
        Line::from(vec![
            Span::styled("?         ", Style::default().fg(theme.accent)),
            Span::raw("Sugo be/ki"),
        ]),
        Line::from(vec![
            Span::styled("Shift+I   ", Style::default().fg(theme.accent)),
            Span::raw("Szerver informaciok"),
        ]),
        Line::from(vec![
            Span::styled("Shift+U   ", Style::default().fg(theme.accent)),
            Span::raw("Frissites ellenorzes"),
        ]),
        Line::from(vec![
            Span::styled("r         ", Style::default().fg(theme.accent)),
            Span::raw("Adatok ujratoltese"),
        ]),
        Line::from(vec![
            Span::styled("t         ", Style::default().fg(theme.accent)),
            Span::raw("Tema valtas"),
        ]),
        Line::from(vec![
            Span::styled("q         ", Style::default().fg(theme.accent)),
            Span::raw("Kilepes"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Detail panel",
            Style::default().fg(theme.accent).bold(),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("e         ", Style::default().fg(theme.accent)),
            Span::raw("Dokumentum szerkesztese"),
        ]),
        Line::from(vec![
            Span::styled("d         ", Style::default().fg(theme.accent)),
            Span::raw("Dokumentum torlese"),
        ]),
        Line::from(vec![
            Span::styled("f         ", Style::default().fg(theme.accent)),
            Span::raw("Vizualis szuro"),
        ]),
        Line::from(vec![
            Span::styled("i         ", Style::default().fg(theme.accent)),
            Span::raw("Dokumentum beszurasa"),
        ]),
        Line::from(vec![
            Span::styled("x         ", Style::default().fg(theme.accent)),
            Span::raw("Index kezeles"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "API Key kezeles",
            Style::default().fg(theme.accent).bold(),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Shift+K   ", Style::default().fg(theme.accent)),
            Span::raw("API Key modal megnyitasa"),
        ]),
    ];

    let help = Paragraph::new(help_text)
        .style(Style::default().fg(theme.fg))
        .scroll((scroll as u16, 0));

    frame.render_widget(help, inner);
}
