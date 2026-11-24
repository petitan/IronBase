//! Help modal: the main screen's keys, generated from the key map
//! (`crate::keymap::MAIN`), so the help cannot drift from what the keys do

use super::render_modal_frame;
use crate::i18n::t;
use crate::keymap::{self, Section, MAIN};
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

const SECTIONS: [Section; 3] = [Section::Navigation, Section::General, Section::Documents];

/// The help's lines: each section's title and its bindings, then a note
/// on the windows' own keys
fn help_lines(theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for section in SECTIONS {
        lines.push(Line::from(Span::styled(
            t().section_title(section),
            Style::default().fg(theme.accent).bold(),
        )));
        lines.push(Line::from(""));
        for binding in MAIN.iter().filter(|binding| binding.section == section) {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{:<14}", keymap::keys_label(binding.keys)),
                    Style::default().fg(theme.accent),
                ),
                Span::raw(t().action_label(binding.action)),
            ]));
        }
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled(
        t().help_menu_note(),
        Style::default().fg(theme.muted),
    )));
    lines
}

/// The number of help lines (for the scroll bounds)
pub fn help_line_count() -> usize {
    help_lines(&Theme::from_name(crate::theme::ThemeName::Claude)).len()
}

/// Render the help modal
pub fn render(frame: &mut Frame, area: Rect, scroll: usize, theme: &Theme) {
    let inner = render_modal_frame(frame, area, t().help_title(), theme, 60, 70);
    let help = Paragraph::new(help_lines(theme))
        .style(Style::default().fg(theme.fg))
        .wrap(ratatui::widgets::Wrap { trim: false })
        .scroll((scroll as u16, 0));
    frame.render_widget(help, inner);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every binding of the main screen is in the help, in its section
    #[test]
    fn test_help_lists_every_main_binding() {
        let lines: Vec<String> = help_lines(&Theme::from_name(crate::theme::ThemeName::Claude))
            .iter()
            .map(|line| line.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        for binding in MAIN {
            let label = t().action_label(binding.action);
            assert!(
                lines.iter().any(|line| line.contains(label)
                    && line.starts_with(&keymap::keys_label(binding.keys))),
                "{label}"
            );
        }
        assert_eq!(help_line_count(), lines.len());
    }
}
