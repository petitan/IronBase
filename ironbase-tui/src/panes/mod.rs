//! Pane modules - the three main panels of the TUI

pub mod collections;
pub mod detail;
pub mod documents;

use crate::app::App;
use crate::theme::Theme;
use ratatui::prelude::*;

/// Pane trait for consistent rendering and focus handling
pub trait Pane {
    fn render(&self, frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool);
}
