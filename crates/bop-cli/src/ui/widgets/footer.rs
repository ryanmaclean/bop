/// Footer bar widget — bottom rows of the three-zone TUI layout.
///
/// Renders context-sensitive keybinding hints based on the current [`Mode`].
/// When the terminal height exceeds 30 rows, a secondary F-key bar is shown
/// on a second line (mc-style function key legend).
///
/// Mode-specific legends:
/// - **Normal**: `[h/l]col [j/k]card [↵]actions [Shift+H/L]move [/]filter [n]new [q]quit`
/// - **Detail**: `[j/k]scroll [F3]logs [p]pause [r]retry [Esc]close`
/// - **LogTail**: `[↑↓]scroll [f]follow [c]clear [Esc]close`
/// - **Filter**: `Filter: {query}█  [Esc]clear [↵]confirm`
/// - **ActionPopup**: `[↑↓]select [↵]run [Esc]cancel`
/// - **NewCard**: `New card id: {input}█  [↵]create [Esc]cancel`
/// - **Subshell**: `(subshell active — Ctrl-D to return)`
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::ui::app::{App, AppTab, Mode};

// ── Constants ──────────────────────────────────────────────────────────────

/// Minimum terminal height to show the secondary F-key bar.
const FKEY_BAR_MIN_HEIGHT: u16 = 30;

/// Block cursor character for input fields.
const BLOCK_CURSOR: &str = "█";

// ── Legend builders ────────────────────────────────────────────────────────

/// Build keybinding hint spans for Normal mode.
fn normal_legend() -> Vec<Span<'static>> {
    vec![
        Span::styled("[h/l]", Style::default().fg(Color::Cyan)),
        Span::styled("col ", Style::default().fg(Color::DarkGray)),
        Span::styled("[j/k]", Style::default().fg(Color::Cyan)),
        Span::styled("card ", Style::default().fg(Color::DarkGray)),
        Span::styled("[↵]", Style::default().fg(Color::Cyan)),
        Span::styled("actions ", Style::default().fg(Color::DarkGray)),
        Span::styled("[Shift+H/L]", Style::default().fg(Color::Cyan)),
        Span::styled("move ", Style::default().fg(Color::DarkGray)),
        Span::styled("[/]", Style::default().fg(Color::Cyan)),
        Span::styled("filter ", Style::default().fg(Color::DarkGray)),
        Span::styled("[n]", Style::default().fg(Color::Cyan)),
        Span::styled("new ", Style::default().fg(Color::DarkGray)),
        Span::styled("[q]", Style::default().fg(Color::Cyan)),
        Span::styled("quit", Style::default().fg(Color::DarkGray)),
    ]
}

/// Build keybinding hint spans for Detail mode.
fn detail_legend() -> Vec<Span<'static>> {
    vec![
        Span::styled("[j/k]", Style::default().fg(Color::Cyan)),
        Span::styled("scroll ", Style::default().fg(Color::DarkGray)),
        Span::styled("[F3]", Style::default().fg(Color::Cyan)),
        Span::styled("logs ", Style::default().fg(Color::DarkGray)),
        Span::styled("[p]", Style::default().fg(Color::Cyan)),
        Span::styled("pause ", Style::default().fg(Color::DarkGray)),
        Span::styled("[r]", Style::default().fg(Color::Cyan)),
        Span::styled("retry ", Style::default().fg(Color::DarkGray)),
        Span::styled("[Esc]", Style::default().fg(Color::Cyan)),
        Span::styled("close", Style::default().fg(Color::DarkGray)),
    ]
}

/// Build keybinding hint spans for LogTail mode.
fn logtail_legend() -> Vec<Span<'static>> {
    vec![
        Span::styled("[↑↓]", Style::default().fg(Color::Cyan)),
        Span::styled("scroll ", Style::default().fg(Color::DarkGray)),
        Span::styled("[f]", Style::default().fg(Color::Cyan)),
        Span::styled("follow ", Style::default().fg(Color::DarkGray)),
        Span::styled("[c]", Style::default().fg(Color::Cyan)),
        Span::styled("clear ", Style::default().fg(Color::DarkGray)),
        Span::styled("[Esc]", Style::default().fg(Color::Cyan)),
        Span::styled("close", Style::default().fg(Color::DarkGray)),
    ]
}

/// Build keybinding hint spans for Filter mode with the current query.
fn filter_legend(query: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled("Filter: ", Style::default().fg(Color::Yellow)),
        Span::styled(
            format!("{}{}", query, BLOCK_CURSOR),
            Style::default().fg(Color::White),
        ),
        Span::styled("  ", Style::default().fg(Color::DarkGray)),
        Span::styled("[Esc]", Style::default().fg(Color::Cyan)),
        Span::styled("clear ", Style::default().fg(Color::DarkGray)),
        Span::styled("[↵]", Style::default().fg(Color::Cyan)),
        Span::styled("confirm", Style::default().fg(Color::DarkGray)),
    ]
}

/// Build keybinding hint spans for ActionPopup mode.
fn action_popup_legend() -> Vec<Span<'static>> {
    vec![
        Span::styled("[↑↓]", Style::default().fg(Color::Cyan)),
        Span::styled("select ", Style::default().fg(Color::DarkGray)),
        Span::styled("[↵]", Style::default().fg(Color::Cyan)),
        Span::styled("run ", Style::default().fg(Color::DarkGray)),
        Span::styled("[Esc]", Style::default().fg(Color::Cyan)),
        Span::styled("cancel", Style::default().fg(Color::DarkGray)),
    ]
}

/// Build keybinding hint spans for NewCard mode with the current input.
fn new_card_legend(input: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled("New card id: ", Style::default().fg(Color::Yellow)),
        Span::styled(
            format!("{}{}", input, BLOCK_CURSOR),
            Style::default().fg(Color::White),
        ),
        Span::styled("  ", Style::default().fg(Color::DarkGray)),
        Span::styled("[↵]", Style::default().fg(Color::Cyan)),
        Span::styled("create ", Style::default().fg(Color::DarkGray)),
        Span::styled("[Esc]", Style::default().fg(Color::Cyan)),
        Span::styled("cancel", Style::default().fg(Color::DarkGray)),
    ]
}

/// Build keybinding hint spans for Subshell mode.
fn subshell_legend() -> Vec<Span<'static>> {
    vec![Span::styled(
        "(subshell active — Ctrl-D to return)",
        Style::default().fg(Color::DarkGray),
    )]
}

/// Build keybinding hint spans for Factory tab.
fn factory_legend() -> Vec<Span<'static>> {
    vec![
        Span::styled("[j/k]", Style::default().fg(Color::Cyan)),
        Span::styled("service ", Style::default().fg(Color::DarkGray)),
        Span::styled("[s]", Style::default().fg(Color::Cyan)),
        Span::styled("stop ", Style::default().fg(Color::DarkGray)),
        Span::styled("[r]", Style::default().fg(Color::Cyan)),
        Span::styled("run ", Style::default().fg(Color::DarkGray)),
        Span::styled("[l]", Style::default().fg(Color::Cyan)),
        Span::styled("log ", Style::default().fg(Color::DarkGray)),
        Span::styled("[Esc/F2]", Style::default().fg(Color::Cyan)),
        Span::styled("kanban", Style::default().fg(Color::DarkGray)),
    ]
}

/// Build the secondary F-key bar (mc-style function key legend).
fn fkey_bar() -> Vec<Span<'static>> {
    vec![
        Span::styled("F3", Style::default().fg(Color::Black).bg(Color::Cyan)),
        Span::styled("=logs  ", Style::default().fg(Color::DarkGray)),
        Span::styled("F4", Style::default().fg(Color::Black).bg(Color::Cyan)),
        Span::styled("=inspect  ", Style::default().fg(Color::DarkGray)),
        Span::styled("F5", Style::default().fg(Color::Black).bg(Color::Cyan)),
        Span::styled("=pause  ", Style::default().fg(Color::DarkGray)),
        Span::styled("F8", Style::default().fg(Color::Black).bg(Color::Cyan)),
        Span::styled("=kill  ", Style::default().fg(Color::DarkGray)),
        Span::styled("F10", Style::default().fg(Color::Black).bg(Color::Cyan)),
        Span::styled("=quit", Style::default().fg(Color::DarkGray)),
    ]
}

// ── Main render ────────────────────────────────────────────────────────────

/// Render the footer bar into the given area.
///
/// Builds context-sensitive keybinding hints based on the current mode.
/// When the terminal height (passed via `terminal_height`) exceeds
/// [`FKEY_BAR_MIN_HEIGHT`], a secondary F-key bar is rendered on a second
/// line — the caller must allocate 2 rows instead of 1 in that case.
pub fn render_footer(frame: &mut Frame, area: Rect, app: &App, terminal_height: u16) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    // Build the primary legend line based on current mode.
    let mut legend_spans = if app.tab == AppTab::Factory {
        factory_legend()
    } else {
        match app.mode {
            Mode::Normal => normal_legend(),
            Mode::Detail => detail_legend(),
            Mode::LogTail => logtail_legend(),
            Mode::Filter => {
                let query = app.filter.as_deref().unwrap_or("");
                filter_legend(query)
            }
            Mode::ActionPopup => action_popup_legend(),
            Mode::NewCard => new_card_legend(&app.newcard_input),
            Mode::Subshell => subshell_legend(),
        }
    };

    // Prepend transient status message if present.
    if let Some(ref msg) = app.status_message {
        let mut with_status = vec![
            Span::styled(format!("{} ", msg), Style::default().fg(Color::Yellow)),
            Span::styled("│ ", Style::default().fg(Color::DarkGray)),
        ];
        with_status.extend(legend_spans);
        legend_spans = with_status;
    }

    let line1 = Line::from(legend_spans);

    // If terminal is tall enough and we have room, add the F-key bar.
    let lines = if area.height >= 2 && terminal_height > FKEY_BAR_MIN_HEIGHT {
        let line2 = Line::from(fkey_bar());
        vec![line1, line2]
    } else {
        vec![line1]
    };

    let footer = Paragraph::new(lines);
    frame.render_widget(footer, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── legend builders ────────────────────────────────────────────────────

    #[test]
    fn normal_legend_contains_all_keys() {
        let spans = normal_legend();
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("[h/l]"));
        assert!(text.contains("[j/k]"));
        assert!(text.contains("[↵]"));
        assert!(text.contains("[Shift+H/L]"));
        assert!(text.contains("[/]"));
        assert!(text.contains("[n]"));
        assert!(text.contains("[q]"));
    }

    #[test]
    fn detail_legend_contains_all_keys() {
        let spans = detail_legend();
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("[j/k]"));
        assert!(text.contains("[F3]"));
        assert!(text.contains("[p]"));
        assert!(text.contains("[r]"));
        assert!(text.contains("[Esc]"));
    }

    #[test]
    fn logtail_legend_contains_all_keys() {
        let spans = logtail_legend();
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("[↑↓]"));
        assert!(text.contains("[f]"));
        assert!(text.contains("[c]"));
        assert!(text.contains("[Esc]"));
    }

    #[test]
    fn filter_legend_shows_query() {
        let spans = filter_legend("hello");
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("Filter: "));
        assert!(text.contains("hello"));
        assert!(text.contains(BLOCK_CURSOR));
        assert!(text.contains("[Esc]"));
        assert!(text.contains("[↵]"));
    }

    #[test]
    fn filter_legend_empty_query() {
        let spans = filter_legend("");
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("Filter: "));
        assert!(text.contains(BLOCK_CURSOR));
    }

    #[test]
    fn action_popup_legend_contains_all_keys() {
        let spans = action_popup_legend();
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("[↑↓]"));
        assert!(text.contains("[↵]"));
        assert!(text.contains("[Esc]"));
    }

    #[test]
    fn new_card_legend_shows_input() {
        let spans = new_card_legend("my-card");
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("New card id: "));
        assert!(text.contains("my-card"));
        assert!(text.contains(BLOCK_CURSOR));
        assert!(text.contains("[↵]"));
        assert!(text.contains("[Esc]"));
    }

    #[test]
    fn new_card_legend_empty_input() {
        let spans = new_card_legend("");
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("New card id: "));
        assert!(text.contains(BLOCK_CURSOR));
    }

    #[test]
    fn subshell_legend_shows_hint() {
        let spans = subshell_legend();
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("subshell"));
        assert!(text.contains("Ctrl-D"));
    }

    // ── fkey_bar ───────────────────────────────────────────────────────────

    #[test]
    fn fkey_bar_contains_all_keys() {
        let spans = fkey_bar();
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("F3"));
        assert!(text.contains("F4"));
        assert!(text.contains("F5"));
        assert!(text.contains("F8"));
        assert!(text.contains("F10"));
    }

    #[test]
    fn fkey_bar_has_labels() {
        let spans = fkey_bar();
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("=logs"));
        assert!(text.contains("=inspect"));
        assert!(text.contains("=pause"));
        assert!(text.contains("=kill"));
        assert!(text.contains("=quit"));
    }
}
