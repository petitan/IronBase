//! Single-line text input: the value, a cursor, the editing keys and the
//! rendering with the terminal's own cursor

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthChar;

/// What a key did to the input
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputOutcome {
    /// The value changed
    Changed,
    /// Only the cursor moved
    Moved,
    /// Not an editing key: the caller handles it
    Ignored,
}

/// A single-line text input. The cursor is a char index (0..=char count),
/// never a byte index, so editing next to multi-byte characters is safe.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextInput {
    value: String,
    cursor: usize,
}

impl TextInput {
    /// An input holding `value`, the cursor at its end
    pub fn new(value: impl Into<String>) -> Self {
        let mut input = Self::default();
        input.set_value(value);
        input
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    /// The cursor as a char index
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    /// Replace the value; the cursor goes to its end
    pub fn set_value(&mut self, value: impl Into<String>) {
        self.value = value.into();
        self.cursor = self.len();
    }

    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }

    fn len(&self) -> usize {
        self.value.chars().count()
    }

    /// Byte offset of char index `pos` (the end for `pos == len`)
    fn byte_at(&self, pos: usize) -> usize {
        self.value
            .char_indices()
            .nth(pos)
            .map(|(i, _)| i)
            .unwrap_or(self.value.len())
    }

    pub fn insert(&mut self, c: char) {
        let at = self.byte_at(self.cursor);
        self.value.insert(at, c);
        self.cursor += 1;
    }

    /// Insert pasted text at the cursor. Line breaks become spaces and other
    /// control characters are dropped: the value stays one line.
    pub fn insert_str(&mut self, text: &str) {
        for c in text.chars() {
            match c {
                '\r' => {}
                '\n' | '\t' => self.insert(' '),
                c if c.is_control() => {}
                c => self.insert(c),
            }
        }
    }

    /// Delete the char before the cursor; false at the start
    pub fn backspace(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        let at = self.byte_at(self.cursor);
        self.value.remove(at);
        true
    }

    /// Delete the char under the cursor; false at the end
    pub fn delete(&mut self) -> bool {
        if self.cursor >= self.len() {
            return false;
        }
        let at = self.byte_at(self.cursor);
        self.value.remove(at);
        true
    }

    pub fn left(&mut self) -> bool {
        let moved = self.cursor > 0;
        self.cursor = self.cursor.saturating_sub(1);
        moved
    }

    pub fn right(&mut self) -> bool {
        let moved = self.cursor < self.len();
        if moved {
            self.cursor += 1;
        }
        moved
    }

    pub fn home(&mut self) -> bool {
        let moved = self.cursor > 0;
        self.cursor = 0;
        moved
    }

    pub fn end(&mut self) -> bool {
        let len = self.len();
        let moved = self.cursor < len;
        self.cursor = len;
        moved
    }

    /// Delete from the start to the cursor (Ctrl+U)
    pub fn delete_to_start(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        let at = self.byte_at(self.cursor);
        self.value.replace_range(..at, "");
        self.cursor = 0;
        true
    }

    /// Delete from the cursor to the end (Ctrl+K)
    pub fn delete_to_end(&mut self) -> bool {
        if self.cursor >= self.len() {
            return false;
        }
        let at = self.byte_at(self.cursor);
        self.value.truncate(at);
        true
    }

    /// Delete the word before the cursor, and the spaces after it (Ctrl+W)
    pub fn delete_word_before(&mut self) -> bool {
        let chars: Vec<char> = self.value.chars().collect();
        let mut start = self.cursor;
        while start > 0 && chars[start - 1].is_whitespace() {
            start -= 1;
        }
        while start > 0 && !chars[start - 1].is_whitespace() {
            start -= 1;
        }
        if start == self.cursor {
            return false;
        }
        let (from, to) = (self.byte_at(start), self.byte_at(self.cursor));
        self.value.replace_range(from..to, "");
        self.cursor = start;
        true
    }

    /// Apply an editing key: printable characters, Backspace / Delete, the
    /// arrows, Home / End and the readline keys Ctrl+A / E / U / K / W.
    /// Anything else is `Ignored` and left to the caller.
    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> InputOutcome {
        let changed = |done: bool| {
            if done {
                InputOutcome::Changed
            } else {
                InputOutcome::Moved
            }
        };
        let moved = |_: bool| InputOutcome::Moved;
        let ctrl = modifiers.contains(KeyModifiers::CONTROL);
        let alt = modifiers.contains(KeyModifiers::ALT);
        match code {
            KeyCode::Char('a') if ctrl => moved(self.home()),
            KeyCode::Char('e') if ctrl => moved(self.end()),
            KeyCode::Char('u') if ctrl => changed(self.delete_to_start()),
            KeyCode::Char('k') if ctrl => changed(self.delete_to_end()),
            KeyCode::Char('w') if ctrl => changed(self.delete_word_before()),
            // Other Ctrl / Alt chords are commands, not text (AltGr arrives as
            // Ctrl+Alt on Windows and is text)
            KeyCode::Char(_) if ctrl != alt => InputOutcome::Ignored,
            KeyCode::Char(c) => {
                self.insert(c);
                InputOutcome::Changed
            }
            KeyCode::Backspace => changed(self.backspace()),
            KeyCode::Delete => changed(self.delete()),
            KeyCode::Left => moved(self.left()),
            KeyCode::Right => moved(self.right()),
            KeyCode::Home => moved(self.home()),
            KeyCode::End => moved(self.end()),
            _ => InputOutcome::Ignored,
        }
    }

    /// The first char index shown in a field `width` columns wide, so that
    /// the cursor (one column past the text at the end) stays visible
    fn scroll_start(&self, width: usize) -> usize {
        if width == 0 {
            return self.cursor;
        }
        let chars: Vec<char> = self.value.chars().collect();
        // Walk back from the cursor while the text up to it (plus the cursor
        // cell) still fits
        let mut used = 1;
        let mut start = self.cursor;
        while start > 0 {
            let w = chars[start - 1].width().unwrap_or(0);
            if used + w > width {
                break;
            }
            used += w;
            start -= 1;
        }
        start
    }

    /// Display width of the text before the cursor (for a caller drawing the
    /// value inline, e.g. inside a chip)
    pub fn cursor_width(&self) -> usize {
        self.value
            .chars()
            .take(self.cursor)
            .map(|c| c.width().unwrap_or(0))
            .sum()
    }

    /// Draw `label`, then the input in the rest of `area` (see `render`)
    pub fn render_labeled<'a>(
        &self,
        frame: &mut Frame,
        area: Rect,
        label: impl Into<Line<'a>>,
        style: Style,
        focused: bool,
    ) {
        let label = label.into();
        let [label_area, input_area] =
            Layout::horizontal([Constraint::Length(label.width() as u16), Constraint::Min(1)])
                .areas(area);
        frame.render_widget(Paragraph::new(label), label_area);
        self.render(frame, input_area, style, focused);
    }

    /// Draw the value in `area` (its first line) with `style`, scrolled so
    /// that the cursor is visible; `focused` puts the terminal cursor there
    pub fn render(&self, frame: &mut Frame, area: Rect, style: Style, focused: bool) {
        let width = area.width as usize;
        let start = self.scroll_start(width);
        let shown: String = self.value.chars().skip(start).collect();
        frame.render_widget(Paragraph::new(Line::from(Span::styled(shown, style))), area);
        if focused && area.width > 0 && area.height > 0 {
            let before: usize = self
                .value
                .chars()
                .skip(start)
                .take(self.cursor - start)
                .map(|c| c.width().unwrap_or(0))
                .sum();
            let x = area.x + (before as u16).min(area.width - 1);
            frame.set_cursor_position(Position::new(x, area.y));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn key(input: &mut TextInput, code: KeyCode) -> InputOutcome {
        input.handle_key(code, KeyModifiers::NONE)
    }

    fn ctrl(input: &mut TextInput, c: char) -> InputOutcome {
        input.handle_key(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn test_editing_next_to_multibyte_chars() {
        let mut input = TextInput::new("/tmp/á");
        assert_eq!(input.cursor(), 6);
        key(&mut input, KeyCode::Char('é'));
        key(&mut input, KeyCode::Char('x'));
        assert_eq!(input.value(), "/tmp/áéx");
        key(&mut input, KeyCode::Left);
        key(&mut input, KeyCode::Left);
        key(&mut input, KeyCode::Char('ő'));
        assert_eq!(input.value(), "/tmp/áőéx");
        assert_eq!(key(&mut input, KeyCode::Delete), InputOutcome::Changed);
        assert_eq!(input.value(), "/tmp/áőx");
        assert_eq!(key(&mut input, KeyCode::Backspace), InputOutcome::Changed);
        assert_eq!(input.value(), "/tmp/áx");
        key(&mut input, KeyCode::End);
        assert_eq!(input.cursor(), 7);
        assert_eq!(key(&mut input, KeyCode::Delete), InputOutcome::Moved);
        key(&mut input, KeyCode::Home);
        assert_eq!(key(&mut input, KeyCode::Backspace), InputOutcome::Moved);
        assert_eq!(input.value(), "/tmp/áx");
    }

    #[test]
    fn test_readline_keys() {
        let mut input = TextInput::new("find users  where");
        assert_eq!(ctrl(&mut input, 'w'), InputOutcome::Changed);
        assert_eq!(input.value(), "find users  ");
        assert_eq!(ctrl(&mut input, 'w'), InputOutcome::Changed);
        assert_eq!(input.value(), "find ");
        ctrl(&mut input, 'a');
        assert_eq!(input.cursor(), 0);
        key(&mut input, KeyCode::Right);
        assert_eq!(ctrl(&mut input, 'k'), InputOutcome::Changed);
        assert_eq!(input.value(), "f");
        ctrl(&mut input, 'e');
        assert_eq!(ctrl(&mut input, 'u'), InputOutcome::Changed);
        assert!(input.is_empty());
        assert_eq!(ctrl(&mut input, 'u'), InputOutcome::Moved);
    }

    #[test]
    fn test_command_chords_are_not_text() {
        let mut input = TextInput::new("");
        assert_eq!(ctrl(&mut input, 's'), InputOutcome::Ignored);
        assert_eq!(
            input.handle_key(KeyCode::Char('x'), KeyModifiers::ALT),
            InputOutcome::Ignored
        );
        assert_eq!(key(&mut input, KeyCode::Enter), InputOutcome::Ignored);
        assert_eq!(key(&mut input, KeyCode::Esc), InputOutcome::Ignored);
        // Shift is text, and so is AltGr (Ctrl+Alt)
        input.handle_key(KeyCode::Char('A'), KeyModifiers::SHIFT);
        input.handle_key(
            KeyCode::Char('@'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        );
        assert_eq!(input.value(), "A@");
    }

    #[test]
    fn test_paste_stays_one_line() {
        let mut input = TextInput::new("a");
        input.insert_str("b\r\nc\td\u{7}é");
        assert_eq!(input.value(), "ab c dé");
        assert_eq!(input.cursor(), 7);
    }

    fn rendered(input: &TextInput, width: u16) -> (String, Position) {
        let mut terminal = Terminal::new(TestBackend::new(width, 1)).unwrap();
        terminal
            .draw(|frame| input.render(frame, frame.area(), Style::default(), true))
            .unwrap();
        let line: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        (line, terminal.get_cursor_position().unwrap())
    }

    #[test]
    fn test_render_puts_the_terminal_cursor_at_the_cursor() {
        let mut input = TextInput::new("abc");
        input.left();
        let (line, cursor) = rendered(&input, 10);
        assert_eq!(line.trim_end(), "abc");
        assert_eq!(cursor, Position::new(2, 0));
    }

    /// A value longer than the field scrolls so the cursor stays visible
    #[test]
    fn test_render_scrolls_to_the_cursor() {
        let input = TextInput::new("0123456789");
        let (line, cursor) = rendered(&input, 5);
        assert_eq!(line, "6789 ");
        assert_eq!(cursor, Position::new(4, 0));

        let mut input = input;
        input.home();
        let (line, cursor) = rendered(&input, 5);
        assert_eq!(line, "01234");
        assert_eq!(cursor, Position::new(0, 0));
    }

    /// Wide characters count two columns
    #[test]
    fn test_render_counts_wide_chars() {
        let input = TextInput::new("日本語");
        let (_, cursor) = rendered(&input, 10);
        assert_eq!(cursor, Position::new(6, 0));
        let (_, cursor) = rendered(&input, 5);
        assert_eq!(cursor, Position::new(4, 0));
    }
}
