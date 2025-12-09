//! Progress modal - shows progress for long-running operations

use crate::app::ProgressState;
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Gauge, Paragraph};

/// Spinner characters for indeterminate progress
const SPINNERS: [char; 8] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧'];

/// Render a progress overlay in the center of the screen
pub fn render(frame: &mut Frame, area: Rect, progress: &ProgressState, theme: &Theme) {
    // Calculate centered modal area (40% width, fixed height)
    let modal_width = (area.width as f32 * 0.4).max(40.0) as u16;
    let modal_height = 5;
    let x = (area.width.saturating_sub(modal_width)) / 2;
    let y = (area.height.saturating_sub(modal_height)) / 2;

    let modal_area = Rect {
        x,
        y,
        width: modal_width,
        height: modal_height,
    };

    // Semi-transparent background overlay
    let overlay = Block::default().style(Style::default().bg(Color::Black));
    frame.render_widget(overlay, area);

    // Modal border
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent))
        .style(Style::default().bg(theme.bg));

    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);

    match progress {
        ProgressState::Indeterminate { message, frame: f } => {
            render_indeterminate(frame, inner, message, *f, theme);
        }
        ProgressState::Determinate {
            message,
            current,
            total,
        } => {
            render_determinate(frame, inner, message, *current, *total, theme);
        }
    }
}

/// Render indeterminate progress (spinner + message)
fn render_indeterminate(
    frame: &mut Frame,
    area: Rect,
    message: &str,
    spinner_frame: usize,
    theme: &Theme,
) {
    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(area);

    // Spinner + message
    let spinner = SPINNERS[spinner_frame % SPINNERS.len()];
    let text = Line::from(vec![
        Span::styled(format!("{} ", spinner), Style::default().fg(theme.accent)),
        Span::styled(message, Style::default().fg(theme.fg)),
    ]);

    let paragraph = Paragraph::new(text).alignment(Alignment::Center);
    frame.render_widget(paragraph, chunks[0]);

    // Animated dots below
    let dots = ".".repeat((spinner_frame / 3) % 4);
    let dots_text = Paragraph::new(dots)
        .style(Style::default().fg(theme.muted))
        .alignment(Alignment::Center);
    frame.render_widget(dots_text, chunks[1]);
}

/// Render determinate progress (bar + percentage)
fn render_determinate(
    frame: &mut Frame,
    area: Rect,
    message: &str,
    current: usize,
    total: usize,
    theme: &Theme,
) {
    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(area);

    // Message with count
    let count_text = format!("{} ({}/{})", message, current, total);
    let paragraph = Paragraph::new(count_text)
        .style(Style::default().fg(theme.fg))
        .alignment(Alignment::Center);
    frame.render_widget(paragraph, chunks[0]);

    // Progress bar
    let percentage = if total > 0 {
        ((current as f64 / total as f64) * 100.0) as u16
    } else {
        0
    };

    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(theme.accent).bg(theme.secondary))
        .percent(percentage)
        .label(format!("{}%", percentage));

    frame.render_widget(gauge, chunks[1]);
}

/// Render progress for splash screen (during startup)
pub fn render_splash(
    frame: &mut Frame,
    area: Rect,
    progress: &ProgressState,
    theme: &Theme,
    logo: &str,
) {
    // Full screen background
    let block = Block::default().style(Style::default().bg(theme.bg));
    frame.render_widget(block, area);

    // Calculate center position
    let logo_height = logo.lines().count();
    let content_height = logo_height + 6;
    let vertical_margin = (area.height.saturating_sub(content_height as u16)) / 2;

    // Build content
    let mut lines: Vec<Line> = Vec::new();

    // Logo lines
    for line in logo.lines().skip(1) {
        lines.push(Line::from(Span::styled(
            line,
            Style::default().fg(theme.accent).bold(),
        )));
    }

    // Empty line
    lines.push(Line::from(""));

    // Progress content
    match progress {
        ProgressState::Indeterminate { message, frame: f } => {
            let spinner = SPINNERS[*f % SPINNERS.len()];
            lines.push(Line::from(vec![
                Span::styled(format!("{} ", spinner), Style::default().fg(theme.accent)),
                Span::styled(message.as_str(), Style::default().fg(theme.muted)),
            ]));
        }
        ProgressState::Determinate {
            message,
            current,
            total,
        } => {
            let percentage = if *total > 0 {
                ((*current as f64 / *total as f64) * 100.0) as u8
            } else {
                0
            };

            // Message with percentage
            lines.push(Line::from(vec![Span::styled(
                format!("{} ({}/{})", message, current, total),
                Style::default().fg(theme.muted),
            )]));

            // Progress bar characters
            let bar_width = 30;
            let filled = (bar_width as f64 * (*current as f64 / *total as f64)) as usize;
            let empty = bar_width - filled;
            let bar = format!(
                "[{}{}] {}%",
                "█".repeat(filled),
                "░".repeat(empty),
                percentage
            );

            lines.push(Line::from(vec![Span::styled(
                bar,
                Style::default().fg(theme.accent),
            )]));
        }
    }

    // Version
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("v{}", env!("CARGO_PKG_VERSION")),
        Style::default().fg(theme.muted),
    )));

    let paragraph = Paragraph::new(lines).alignment(Alignment::Center).block(
        Block::default()
            .borders(Borders::NONE)
            .style(Style::default().bg(theme.bg)),
    );

    // Create centered area
    let centered_area = Rect {
        x: 0,
        y: vertical_margin,
        width: area.width,
        height: content_height as u16,
    };

    frame.render_widget(paragraph, centered_area);
}
