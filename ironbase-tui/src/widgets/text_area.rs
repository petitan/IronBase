//! Multi-line text editor: lines, a cursor, the editing keys and the
//! rendering with the terminal's own cursor. Each line is a [`TextInput`],
//! so the in-line editing (char-index cursor, readline keys) is the
//! single-line input's; this type adds what crosses lines.

use super::text_input::{Input, InputOutcome, TextInput};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use std::cell::Cell;
use unicode_width::UnicodeWidthChar;

/// What Tab inserts
const INDENT: &str = "  ";
/// Lines PageUp / PageDown move
const PAGE: usize = 10;

/// A multi-line text editor. There is always at least one line; the cursor
/// is a line index and the char index inside that line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextArea {
    lines: Vec<TextInput>,
    row: usize,
    /// First line drawn last time: the view scrolls only as far as the
    /// cursor needs, not to keep it at the bottom
    top: Cell<usize>,
}

impl Default for TextArea {
    fn default() -> Self {
        Self {
            lines: vec![TextInput::default()],
            row: 0,
            top: Cell::new(0),
        }
    }
}

impl TextArea {
    /// An editor holding `text` (lines split at `\n`), the cursor at the start
    pub fn new(text: &str) -> Self {
        let mut area = Self::default();
        area.set_text(text);
        area
    }

    /// An editor holding `text`, the cursor at (`row`, `col`) (clamped)
    pub fn with_cursor(text: &str, row: usize, col: usize) -> Self {
        let mut area = Self::new(text);
        area.move_to(row, col);
        area
    }

    /// Replace the text; the cursor goes to the start
    pub fn set_text(&mut self, text: &str) {
        self.lines = text
            .split('\n')
            .map(|line| {
                let mut input = TextInput::new(line.strip_suffix('\r').unwrap_or(line));
                input.set_cursor(0);
                input
            })
            .collect();
        self.row = 0;
        self.top.set(0);
    }

    /// The text, lines joined with `\n`
    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(TextInput::value)
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// The text of line `row`
    pub fn line(&self, row: usize) -> &str {
        self.lines.get(row).map(TextInput::value).unwrap_or("")
    }

    /// Whether the text is empty (one empty line)
    pub fn is_empty(&self) -> bool {
        self.lines.len() == 1 && self.lines[0].is_empty()
    }

    /// The cursor: line index, char index in the line
    pub fn cursor(&self) -> (usize, usize) {
        (self.row, self.lines[self.row].cursor())
    }

    /// Put the cursor at (`row`, `col`), clamped to the text
    pub fn move_to(&mut self, row: usize, col: usize) {
        self.row = row.min(self.lines.len() - 1);
        self.lines[self.row].set_cursor(col);
    }

    fn current(&mut self) -> &mut TextInput {
        &mut self.lines[self.row]
    }

    pub fn insert(&mut self, c: char) {
        self.current().insert(c);
    }

    /// Split the line at the cursor; the cursor goes to the new line's start
    pub fn newline(&mut self) {
        let rest = self.current().split_off();
        let mut next = TextInput::new(rest);
        next.set_cursor(0);
        self.row += 1;
        self.lines.insert(self.row, next);
    }

    /// Insert two spaces
    pub fn indent(&mut self) {
        for c in INDENT.chars() {
            self.insert(c);
        }
    }

    /// Insert pasted text at the cursor: line breaks (`\n`, `\r\n`, `\r`)
    /// start new lines, a tab indents, other control characters are dropped.
    /// False when nothing was inserted.
    pub fn insert_str(&mut self, text: &str) -> bool {
        let mut inserted = false;
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    self.newline();
                }
                '\n' => self.newline(),
                '\t' => self.indent(),
                c if c.is_control() => continue,
                c => self.insert(c),
            }
            inserted = true;
        }
        inserted
    }

    /// Apply a key (see `handle_key`) or a pasted text (see `insert_str`)
    pub fn apply(&mut self, input: Input) -> InputOutcome {
        match input {
            Input::Key(code, modifiers) => self.handle_key(code, modifiers),
            Input::Paste(text) if self.insert_str(text) => InputOutcome::Changed,
            Input::Paste(_) => InputOutcome::Ignored,
        }
    }

    /// Delete the char before the cursor; at a line start, join the line to
    /// the previous one. False at the very start.
    pub fn backspace(&mut self) -> bool {
        if self.current().backspace() {
            return true;
        }
        if self.row == 0 {
            return false;
        }
        let line = self.lines.remove(self.row);
        self.row -= 1;
        let prev = self.current();
        prev.end();
        prev.push_str(line.value());
        true
    }

    /// Delete the char under the cursor; at a line end, join the next line
    /// to it. False at the very end.
    pub fn delete(&mut self) -> bool {
        if self.current().delete() {
            return true;
        }
        if self.row + 1 >= self.lines.len() {
            return false;
        }
        let next = self.lines.remove(self.row + 1);
        self.current().push_str(next.value());
        true
    }

    /// Left; from a line start to the previous line's end
    pub fn left(&mut self) -> bool {
        if self.current().left() {
            return true;
        }
        if self.row == 0 {
            return false;
        }
        self.row -= 1;
        self.current().end();
        true
    }

    /// Right; from a line end to the next line's start
    pub fn right(&mut self) -> bool {
        if self.current().right() {
            return true;
        }
        if self.row + 1 >= self.lines.len() {
            return false;
        }
        self.row += 1;
        self.current().home();
        true
    }

    /// Move `lines` lines up (negative) or down, keeping the column where
    /// the target line is long enough
    fn vertical(&mut self, lines: isize) -> bool {
        let target = self
            .row
            .saturating_add_signed(lines)
            .min(self.lines.len() - 1);
        if target == self.row {
            return false;
        }
        let col = self.current().cursor();
        self.row = target;
        self.current().set_cursor(col);
        true
    }

    pub fn up(&mut self) -> bool {
        self.vertical(-1)
    }

    pub fn down(&mut self) -> bool {
        self.vertical(1)
    }

    /// Apply an editing key: what [`TextInput::handle_key`] takes on the
    /// current line, plus Enter (new line), Tab (indent), Up / Down,
    /// PageUp / PageDown, Ctrl+Home / End (start / end of the text), and
    /// Backspace / Delete / Left / Right across line ends. Anything else is
    /// `Ignored` and left to the caller.
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
        match code {
            KeyCode::Enter => {
                self.newline();
                InputOutcome::Changed
            }
            KeyCode::Tab if modifiers.is_empty() => {
                self.indent();
                InputOutcome::Changed
            }
            KeyCode::Backspace => changed(self.backspace()),
            KeyCode::Delete => changed(self.delete()),
            KeyCode::Left => moved(self.left()),
            KeyCode::Right => moved(self.right()),
            KeyCode::Up => moved(self.up()),
            KeyCode::Down => moved(self.down()),
            KeyCode::PageUp => moved(self.vertical(-(PAGE as isize))),
            KeyCode::PageDown => moved(self.vertical(PAGE as isize)),
            KeyCode::Home if ctrl => {
                self.move_to(0, 0);
                InputOutcome::Moved
            }
            KeyCode::End if ctrl => {
                let last = self.lines.len() - 1;
                self.move_to(last, usize::MAX);
                InputOutcome::Moved
            }
            _ => self.current().handle_key(code, modifiers),
        }
    }

    /// Draw the visible lines in `area` with `style`. `gutter` adds line
    /// numbers in that style; `spans` turns a (visible part of a) line into
    /// styled spans, e.g. for syntax highlighting. The view follows the
    /// cursor, scrolling down and right only as far as needed; `focused`
    /// puts the terminal cursor at it.
    pub fn render(
        &self,
        frame: &mut Frame,
        area: Rect,
        style: Style,
        gutter: Option<Style>,
        spans: &dyn Fn(&str) -> Vec<Span<'static>>,
        focused: bool,
    ) {
        let height = area.height as usize;
        // Line numbers: at least three digits, as many as the last line needs
        let digits = self.lines.len().to_string().len().max(3);
        let gutter_width = if gutter.is_some() { digits + 1 } else { 0 };
        let width = (area.width as usize).saturating_sub(gutter_width);
        if height == 0 || width == 0 {
            return;
        }

        let mut top = self.top.get().min(self.row);
        if self.row >= top + height {
            top = self.row + 1 - height;
        }
        self.top.set(top);
        // Horizontal scroll: the cursor line's, for every line
        let current = &self.lines[self.row];
        let first_col = current.scroll_start(width);

        let lines: Vec<Line> = self
            .lines
            .iter()
            .enumerate()
            .skip(top)
            .take(height)
            .map(|(idx, line)| {
                let mut row = Vec::new();
                if let Some(gutter) = gutter {
                    row.push(Span::styled(format!("{:>digits$} ", idx + 1), gutter));
                }
                // Style the whole line (a highlighter must see a string or a
                // comment from its start), then cut the scrolled-off part
                row.extend(skip_chars(spans(line.value()), first_col));
                Line::from(row)
            })
            .collect();
        frame.render_widget(Paragraph::new(lines).style(style), area);

        if focused {
            let before: usize = current
                .value()
                .chars()
                .skip(first_col)
                .take(current.cursor().saturating_sub(first_col))
                .map(|c| c.width().unwrap_or(0))
                .sum();
            let x = area.x + (gutter_width + before).min(area.width as usize - 1) as u16;
            let y = area.y + (self.row - top) as u16;
            frame.set_cursor_position(Position::new(x, y));
        }
    }
}

/// `spans` without their first `count` chars
fn skip_chars(spans: Vec<Span<'static>>, mut count: usize) -> Vec<Span<'static>> {
    let mut kept = Vec::new();
    for span in spans {
        let len = span.content.chars().count();
        if count >= len {
            count -= len;
            continue;
        }
        let rest: String = span.content.chars().skip(count).collect();
        count = 0;
        kept.push(Span::styled(rest, span.style));
    }
    kept
}

/// The whole line in one span of `style` (for [`TextArea::render`])
pub fn plain(style: Style) -> impl Fn(&str) -> Vec<Span<'static>> {
    move |text: &str| vec![Span::styled(text.to_string(), style)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn key(area: &mut TextArea, code: KeyCode) -> InputOutcome {
        area.handle_key(code, KeyModifiers::NONE)
    }

    fn type_text(area: &mut TextArea, text: &str) {
        for c in text.chars() {
            key(area, KeyCode::Char(c));
        }
    }

    #[test]
    fn test_new_and_text_round_trip() {
        let area = TextArea::new("{\n  \"a\": 1\n}");
        assert_eq!(area.line_count(), 3);
        assert_eq!(area.line(1), "  \"a\": 1");
        assert_eq!(area.text(), "{\n  \"a\": 1\n}");
        assert_eq!(area.cursor(), (0, 0));
        assert!(TextArea::default().is_empty());
        assert_eq!(TextArea::new("a\r\nb").text(), "a\nb");
    }

    #[test]
    fn test_insert_is_by_character_not_byte() {
        let mut area = TextArea::new("ő");
        area.move_to(0, 1);
        type_text(&mut area, "ű!");
        assert_eq!(area.text(), "őű!");
        assert_eq!(area.cursor(), (0, 3));
    }

    #[test]
    fn test_newline_splits_at_character_position() {
        let mut area = TextArea::new("árvíz");
        area.move_to(0, 2);
        assert_eq!(key(&mut area, KeyCode::Enter), InputOutcome::Changed);
        assert_eq!(area.text(), "ár\nvíz");
        assert_eq!(area.cursor(), (1, 0));
    }

    #[test]
    fn test_backspace_and_delete_join_lines() {
        let mut area = TextArea::new("ab\ncd");
        area.move_to(1, 0);
        assert_eq!(key(&mut area, KeyCode::Backspace), InputOutcome::Changed);
        assert_eq!((area.text(), area.cursor()), ("abcd".to_string(), (0, 2)));
        // Nothing before the start
        area.move_to(0, 0);
        assert_eq!(key(&mut area, KeyCode::Backspace), InputOutcome::Moved);

        let mut area = TextArea::new("ab\ncd");
        area.move_to(0, 2);
        assert_eq!(key(&mut area, KeyCode::Delete), InputOutcome::Changed);
        assert_eq!((area.text(), area.cursor()), ("abcd".to_string(), (0, 2)));
        area.move_to(0, 4);
        assert_eq!(key(&mut area, KeyCode::Delete), InputOutcome::Moved);
    }

    #[test]
    fn test_cursor_movement_wraps_and_clamps() {
        let mut area = TextArea::new("long line\nab\nxyz");
        area.move_to(0, 9);
        key(&mut area, KeyCode::Down);
        assert_eq!(area.cursor(), (1, 2), "the column is clamped to the line");
        key(&mut area, KeyCode::Right);
        assert_eq!(
            area.cursor(),
            (2, 0),
            "Right at a line end goes to the next line"
        );
        key(&mut area, KeyCode::Left);
        assert_eq!(
            area.cursor(),
            (1, 2),
            "Left at a line start goes to the previous end"
        );
        key(&mut area, KeyCode::Down);
        key(&mut area, KeyCode::Down);
        assert_eq!(area.cursor(), (2, 2), "no line below the last");
        area.handle_key(KeyCode::Home, KeyModifiers::CONTROL);
        assert_eq!(area.cursor(), (0, 0));
        area.handle_key(KeyCode::End, KeyModifiers::CONTROL);
        assert_eq!(area.cursor(), (2, 3));
        key(&mut area, KeyCode::PageUp);
        assert_eq!(area.cursor(), (0, 3));
    }

    /// The single-line keys work on the current line
    #[test]
    fn test_line_keys_and_chords() {
        let mut area = TextArea::new("first\nhello world");
        area.move_to(1, 11);
        area.handle_key(KeyCode::Char('w'), KeyModifiers::CONTROL);
        assert_eq!(area.line(1), "hello ");
        key(&mut area, KeyCode::Home);
        assert_eq!(area.cursor(), (1, 0));
        area.handle_key(KeyCode::Char('k'), KeyModifiers::CONTROL);
        assert_eq!(area.text(), "first\n");
        // A command chord is not text
        assert_eq!(
            area.handle_key(KeyCode::Char('s'), KeyModifiers::CONTROL),
            InputOutcome::Ignored
        );
        assert_eq!(key(&mut area, KeyCode::Tab), InputOutcome::Changed);
        assert_eq!(area.line(1), "  ");
        assert_eq!(key(&mut area, KeyCode::Esc), InputOutcome::Ignored);
    }

    #[test]
    fn test_apply_paste() {
        let mut area = TextArea::default();
        assert_eq!(area.apply(Input::Paste("a\nb")), InputOutcome::Changed);
        assert_eq!(area.text(), "a\nb");
        assert_eq!(area.apply(Input::Paste("\u{7}")), InputOutcome::Ignored);
    }

    #[test]
    fn test_insert_str_keeps_the_lines() {
        let mut area = TextArea::new("[]");
        area.move_to(0, 1);
        area.insert_str("1,\r\n\t2\r3\u{7}");
        assert_eq!(area.text(), "[1,\n  2\n3]");
        assert_eq!(area.cursor(), (2, 1));
    }

    fn rendered(area: &TextArea, width: u16, height: u16, gutter: bool) -> (Vec<String>, Position) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                area.render(
                    frame,
                    frame.area(),
                    Style::default(),
                    gutter.then(Style::default),
                    &plain(Style::default()),
                    true,
                )
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let rows = (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect();
        (rows, terminal.get_cursor_position().unwrap())
    }

    #[test]
    fn test_render_scrolls_to_the_cursor_and_sets_the_terminal_cursor() {
        let mut area = TextArea::new("a\nb\nc\nd\ne");
        area.move_to(4, 1);
        let (rows, cursor) = rendered(&area, 6, 2, true);
        assert_eq!(rows, vec!["  4 d ", "  5 e "]);
        assert_eq!(cursor, Position::new(5, 1));
        // Moving up inside the view does not scroll it
        area.move_to(3, 0);
        let (rows, cursor) = rendered(&area, 6, 2, true);
        assert_eq!(rows[0], "  4 d ");
        assert_eq!(cursor, Position::new(4, 0));
        area.move_to(0, 0);
        let (rows, _) = rendered(&area, 6, 2, true);
        assert_eq!(rows[0], "  1 a ");
    }

    /// From line 1000 on the numbers are wider: the gutter widens with them
    /// and the cursor stays after the text
    #[test]
    fn test_render_gutter_fits_the_last_line_number() {
        let text: Vec<String> = (0..1000).map(|i| format!("x{i}")).collect();
        let mut area = TextArea::new(&text.join("\n"));
        area.move_to(999, 4);
        let (rows, cursor) = rendered(&area, 12, 2, true);
        assert_eq!(rows, vec![" 999 x998   ", "1000 x999   "]);
        assert_eq!(cursor, Position::new(9, 1));
    }

    /// The styler gets the whole line, the scrolled-off part is cut after
    #[test]
    fn test_render_styles_the_whole_line() {
        let mut area = TextArea::new("abcdefghij");
        area.move_to(0, 10);
        let mut terminal = Terminal::new(TestBackend::new(5, 1)).unwrap();
        let seen = std::cell::RefCell::new(Vec::new());
        terminal
            .draw(|frame| {
                area.render(
                    frame,
                    frame.area(),
                    Style::default(),
                    None,
                    &|line: &str| {
                        seen.borrow_mut().push(line.to_string());
                        vec![
                            Span::raw(line[..3].to_string()),
                            Span::raw(line[3..].to_string()),
                        ]
                    },
                    true,
                )
            })
            .unwrap();
        assert_eq!(seen.into_inner(), vec!["abcdefghij"]);
        let row: String = (0..5)
            .map(|x| terminal.backend().buffer()[(x, 0)].symbol().to_string())
            .collect();
        assert_eq!(row, "ghij ");
    }

    #[test]
    fn test_render_scrolls_right_for_a_long_line() {
        let mut area = TextArea::new("0123456789\nab");
        area.move_to(0, 10);
        let (rows, cursor) = rendered(&area, 5, 2, false);
        assert_eq!(rows, vec!["6789 ", "     "]);
        assert_eq!(cursor, Position::new(4, 0));
    }
}
