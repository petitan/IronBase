//! Shared widgets: one implementation per kind of control, used by every
//! modal (instead of each modal editing its own strings)

pub mod text_area;
pub mod text_input;

pub use text_area::TextArea;
pub use text_input::{Input, InputOutcome, TextInput};

use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

/// `s` cut to at most `max` terminal columns, with `…` marking the cut
/// (display width, not chars or bytes: wide characters take two columns)
pub fn truncate_width(s: &str, max: usize) -> String {
    if unicode_width::UnicodeWidthStr::width(s) <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut width = 0;
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        // Room for the `…` (one column) after the kept part
        if width + w > max - 1 {
            break;
        }
        out.push(c);
        width += w;
    }
    out.push('…');
    out
}

/// The mark in front of a list's selected row, and the same width of space
/// in front of the others: the selection shows without color too (`NO_COLOR`,
/// a monochrome terminal, color blindness)
pub fn selection_mark(selected: bool) -> &'static str {
    if selected {
        "▶ "
    } else {
        "  "
    }
}

/// A list row with the selection mark in front
pub fn marked<'a>(selected: bool, line: impl Into<Line<'a>>) -> Line<'a> {
    let mut line = line.into();
    line.spans.insert(0, Span::raw(selection_mark(selected)));
    line
}

/// A choice among a few options (a button, a mode): the chosen one in
/// brackets, the others padded to the same width, so the choice shows
/// without color too
pub fn choice(label: &str, chosen: bool) -> String {
    if chosen {
        format!("[{label}]")
    } else {
        format!(" {label} ")
    }
}

/// `value` with the ASCII digits of `text` appended, as typed into a number
/// field (other characters are skipped; it stops growing at `usize::MAX`
/// instead of overflowing)
pub fn push_digits(value: usize, text: &str) -> usize {
    text.chars()
        .filter(char::is_ascii_digit)
        .filter_map(|c| c.to_digit(10))
        .fold(value, |n, d| {
            n.saturating_mul(10).saturating_add(d as usize)
        })
}

#[cfg(test)]
mod tests {
    use super::{choice, marked, push_digits};

    #[test]
    fn test_push_digits() {
        assert_eq!(push_digits(7, "68"), 768);
        assert_eq!(push_digits(0, " 1,0x2\n"), 102);
        assert_eq!(push_digits(5, "abc٣"), 5);
        assert_eq!(push_digits(1, "99999999999999999999999"), usize::MAX);
    }

    /// The selection shows as text, not only as color
    #[test]
    fn test_selection_shows_without_color() {
        let text = |line: ratatui::text::Line| -> String {
            line.spans.iter().map(|s| s.content.as_ref()).collect()
        };
        assert_eq!(text(marked(true, "users")), "▶ users");
        assert_eq!(text(marked(false, "users")), "  users");
        assert_eq!(choice("Mégse", true), "[Mégse]");
        assert_eq!(choice("Mégse", false), " Mégse ");
        assert_eq!(
            choice("x", true).chars().count(),
            choice("x", false).chars().count()
        );
    }
}
