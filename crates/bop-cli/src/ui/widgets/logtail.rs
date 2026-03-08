/// Log tail overlay widget — full-height scrollable log view.
///
/// Replaces the body zone when `Mode::LogTail` is active. Shows the last
/// 200 lines from the selected card's `logs/stdout.log` and
/// `logs/stderr.log`. Implements newline-gated streaming: bytes are
/// buffered and only complete lines (ending in `\n`) are emitted to
/// prevent partial ANSI sequence flicker.
///
/// Keybindings (handled in `input.rs`):
/// - `↑` / `↓` — scroll up / down
/// - `f` — toggle follow mode (auto-scroll to bottom)
/// - `c` — clear buffer
/// - `Esc` — return to Normal mode
use std::collections::VecDeque;

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use ratatui::Frame;

// ── Line styling ────────────────────────────────────────────────────────────

/// Stylize a single log line for TUI display using ratatui `Style`.
///
/// Mirrors the colorization logic from `logs.rs::colorize_log_line` but
/// uses ratatui styles instead of raw ANSI escape codes, since the TUI
/// owns all terminal rendering via ratatui.
fn stylize_log_line(line: &str) -> Line<'static> {
    let style = if line.contains("ERROR") || line.contains("error:") || line.contains("FAILED") {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    } else if line.contains("WARN") || line.contains("warning:") {
        Style::default().fg(Color::Yellow)
    } else if line.contains("INFO") {
        Style::default().fg(Color::Cyan)
    } else if line.contains("DEBUG") || line.contains("TRACE") {
        Style::default().fg(Color::DarkGray)
    } else if line.contains("→ merged") || line.contains("-> merged") {
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD)
    } else if line.contains("→ done") || line.contains("-> done") {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else if line.contains("→ failed") || line.contains("-> failed") {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    } else if line.contains("→ running") || line.contains("-> running") {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    Line::from(Span::styled(line.to_string(), style))
}

// ── Public render function ──────────────────────────────────────────────────

/// Render the log tail overlay, replacing the full body zone.
///
/// Draws a bordered block with the card ID in the title, a follow-mode
/// indicator, and the buffered log lines with scroll support. When
/// `follow_mode` is enabled, the scroll offset is automatically computed
/// to show the most recent lines at the bottom of the viewport.
///
/// The overlay uses ratatui `Clear` to erase the underlying kanban board
/// content before rendering the log view.
pub fn render_logtail(
    frame: &mut Frame,
    body_area: Rect,
    card_id: &str,
    log_buf: &VecDeque<String>,
    log_scroll: usize,
    follow_mode: bool,
) {
    // Clear the entire body area so the kanban content is hidden.
    frame.render_widget(Clear, body_area);

    // Build title with follow-mode indicator.
    let follow_indicator = if follow_mode { " [FOLLOW]" } else { "" };
    let title = format!(" {} — logs{} ", card_id, follow_indicator);

    let title_style = if follow_mode {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    };

    let block = Block::bordered()
        .title(title)
        .title_style(title_style)
        .border_style(Style::default().fg(Color::Yellow));

    // Stylize each line from the circular buffer.
    let lines: Vec<Line> = log_buf.iter().map(|l| stylize_log_line(l)).collect();

    // Calculate effective scroll offset.
    // In follow mode, auto-scroll to show the latest lines at the bottom.
    let visible_height = body_area.height.saturating_sub(2) as usize; // minus top/bottom borders
    let effective_scroll = if follow_mode {
        lines.len().saturating_sub(visible_height)
    } else {
        log_scroll
    };

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((effective_scroll as u16, 0));

    frame.render_widget(paragraph, body_area);
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── stylize_log_line ────────────────────────────────────────────────

    #[test]
    fn error_lines_are_red_bold() {
        let line = stylize_log_line("2024-01-01 ERROR something broke");
        let style = line.spans[0].style;
        assert_eq!(style.fg, Some(Color::Red));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn warn_lines_are_yellow() {
        let line = stylize_log_line("WARN: disk space low");
        let style = line.spans[0].style;
        assert_eq!(style.fg, Some(Color::Yellow));
    }

    #[test]
    fn info_lines_are_cyan() {
        let line = stylize_log_line("INFO starting server");
        let style = line.spans[0].style;
        assert_eq!(style.fg, Some(Color::Cyan));
    }

    #[test]
    fn debug_lines_are_dim() {
        let line = stylize_log_line("DEBUG detailed trace");
        let style = line.spans[0].style;
        assert_eq!(style.fg, Some(Color::DarkGray));
    }

    #[test]
    fn transition_lines_have_correct_colors() {
        let merged = stylize_log_line("card-1 → merged");
        assert_eq!(merged.spans[0].style.fg, Some(Color::Magenta));

        let done = stylize_log_line("card-2 → done");
        assert_eq!(done.spans[0].style.fg, Some(Color::Green));

        let failed = stylize_log_line("card-3 → failed");
        assert_eq!(failed.spans[0].style.fg, Some(Color::Red));

        let running = stylize_log_line("card-4 → running");
        assert_eq!(running.spans[0].style.fg, Some(Color::Yellow));
    }

    #[test]
    fn plain_lines_are_white() {
        let line = stylize_log_line("just a regular log line");
        let style = line.spans[0].style;
        assert_eq!(style.fg, Some(Color::White));
    }

    #[test]
    fn arrow_ascii_transitions_also_match() {
        let merged = stylize_log_line("card-1 -> merged");
        assert_eq!(merged.spans[0].style.fg, Some(Color::Magenta));

        let done = stylize_log_line("card-2 -> done");
        assert_eq!(done.spans[0].style.fg, Some(Color::Green));
    }
}
