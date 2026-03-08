/// Footer bar widget — bottom rows of the three-zone TUI layout.
///
/// Renders context-sensitive keybinding hints based on the current [`Mode`].
/// When the terminal height exceeds 30 rows, a secondary F-key bar is shown
/// on a second line (mc-style function key legend).
///
/// Mode-specific legends:
/// - **Normal**: `[h/l]col [j/k]card [↵]detail [a]actions [H/>]move [L]logs [/]filter [Ctrl+O]shell [n]new [q]quit`
/// - **Log tab**: `[j/k]scroll [f]follow [Tab]next [L/Esc]close`
/// - **Detail**: `[M/D/R/O/L]tabs [j/k]scroll [G]end [f]follow [Esc/↵]close`
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

use crate::ui::app::{App, AppTab, DetailTab, Mode};

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
        Span::styled("detail ", Style::default().fg(Color::DarkGray)),
        Span::styled("[a]", Style::default().fg(Color::Cyan)),
        Span::styled("actions ", Style::default().fg(Color::DarkGray)),
        Span::styled("[H/>]", Style::default().fg(Color::Cyan)),
        Span::styled("move ", Style::default().fg(Color::DarkGray)),
        Span::styled("[L]", Style::default().fg(Color::Cyan)),
        Span::styled("logs ", Style::default().fg(Color::DarkGray)),
        Span::styled("[/]", Style::default().fg(Color::Cyan)),
        Span::styled("filter ", Style::default().fg(Color::DarkGray)),
        Span::styled("[Ctrl+O]", Style::default().fg(Color::Cyan)),
        Span::styled("shell ", Style::default().fg(Color::DarkGray)),
        Span::styled("[n]", Style::default().fg(Color::Cyan)),
        Span::styled("new ", Style::default().fg(Color::DarkGray)),
        Span::styled("[q]", Style::default().fg(Color::Cyan)),
        Span::styled("quit", Style::default().fg(Color::DarkGray)),
    ]
}

/// Build keybinding hint spans for integrated Log tab.
fn log_pane_legend(follow: bool) -> Vec<Span<'static>> {
    let mut spans = vec![
        Span::styled("[j/k]", Style::default().fg(Color::Cyan)),
        Span::styled("scroll ", Style::default().fg(Color::DarkGray)),
        Span::styled("[f]", Style::default().fg(Color::Cyan)),
        Span::styled("follow ", Style::default().fg(Color::DarkGray)),
        Span::styled("[Tab]", Style::default().fg(Color::Cyan)),
        Span::styled("next running ", Style::default().fg(Color::DarkGray)),
        Span::styled("[L/Esc]", Style::default().fg(Color::Cyan)),
        Span::styled("close", Style::default().fg(Color::DarkGray)),
    ];
    if !follow {
        spans.push(Span::styled("  ", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled("↓ paused", Style::default().fg(Color::Yellow)));
    }
    spans
}

/// Build keybinding hint spans for Detail mode.
fn detail_legend(tab: DetailTab, follow: bool) -> Vec<Span<'static>> {
    let tab_name = match tab {
        DetailTab::Meta => "meta",
        DetailTab::Diff => "diff",
        DetailTab::Replay => "replay",
        DetailTab::Output => "output",
        DetailTab::Log => "log",
    };

    let mut spans = vec![
        Span::styled("[M/D/R/O/L]", Style::default().fg(Color::Cyan)),
        Span::styled("tabs ", Style::default().fg(Color::DarkGray)),
        Span::styled(format!("({tab_name}) "), Style::default().fg(Color::Yellow)),
        Span::styled("[j/k]", Style::default().fg(Color::Cyan)),
        Span::styled("scroll ", Style::default().fg(Color::DarkGray)),
        Span::styled("[G]", Style::default().fg(Color::Cyan)),
        Span::styled("end ", Style::default().fg(Color::DarkGray)),
        Span::styled("[f]", Style::default().fg(Color::Cyan)),
        Span::styled("follow ", Style::default().fg(Color::DarkGray)),
        Span::styled("[Esc/↵]", Style::default().fg(Color::Cyan)),
        Span::styled("close", Style::default().fg(Color::DarkGray)),
    ];

    if tab == DetailTab::Log {
        spans.push(Span::styled("  ", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(
            if follow { "FOLLOW" } else { "PAUSED" },
            Style::default().fg(if follow { Color::Green } else { Color::Yellow }),
        ));
    }

    spans
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

/// Build keybinding hint spans for an active (confirmed) filter.
fn filter_active_legend(query: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled("Filter: ", Style::default().fg(Color::Yellow)),
        Span::styled(query.to_string(), Style::default().fg(Color::White)),
        Span::styled("  ", Style::default().fg(Color::DarkGray)),
        Span::styled("[/]", Style::default().fg(Color::Cyan)),
        Span::styled("edit ", Style::default().fg(Color::DarkGray)),
        Span::styled("[Esc]", Style::default().fg(Color::Cyan)),
        Span::styled("clear", Style::default().fg(Color::DarkGray)),
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
        Span::styled("=detail  ", Style::default().fg(Color::DarkGray)),
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
    let active_filter_query = app
        .filter_state
        .as_ref()
        .map(|s| s.query.as_str())
        .or(app.filter.as_deref())
        .filter(|q| !q.is_empty());

    let mut legend_spans = match &app.tab {
        AppTab::Factory => factory_legend(),
        AppTab::Detail(_) => detail_legend(app.detail_tab, app.detail_log_follow),
        AppTab::Log(_) => log_pane_legend(app.log_pane_follow),
        AppTab::Kanban => match app.mode {
            Mode::Normal => {
                if let Some(query) = active_filter_query {
                    filter_active_legend(query)
                } else {
                    normal_legend()
                }
            }
            Mode::Detail => detail_legend(app.detail_tab, app.detail_log_follow),
            Mode::LogTail => logtail_legend(),
            Mode::Filter => {
                let query = app
                    .filter_state
                    .as_ref()
                    .map(|s| s.query.as_str())
                    .or(app.filter.as_deref())
                    .unwrap_or("");
                filter_legend(query)
            }
            Mode::ActionPopup => action_popup_legend(),
            Mode::NewCard => new_card_legend(&app.newcard_input),
            Mode::Subshell => subshell_legend(),
        },
    };

    // Prepend transient status message if present.
    let toast_msg = app.toast_message.as_ref();
    let status_msg = app.status_message.as_ref();
    if let Some(msg) = toast_msg.or(status_msg) {
        let msg_color = if toast_msg.is_some() {
            Color::Red
        } else {
            Color::Yellow
        };
        let mut with_status = vec![
            Span::styled(format!("{} ", msg), Style::default().fg(msg_color)),
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
        assert!(text.contains("[a]"));
        assert!(text.contains("[H/>]"));
        assert!(text.contains("[L]"));
        assert!(text.contains("[/]"));
        assert!(text.contains("[Ctrl+O]"));
        assert!(text.contains("[n]"));
        assert!(text.contains("[q]"));
    }

    #[test]
    fn detail_legend_contains_all_keys() {
        let spans = detail_legend(DetailTab::Meta, true);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("[M/D/R/O/L]"));
        assert!(text.contains("[j/k]"));
        assert!(text.contains("[G]"));
        assert!(text.contains("[f]"));
        assert!(text.contains("[Esc/↵]"));
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
        assert!(text.contains("=detail"));
        assert!(text.contains("=pause"));
        assert!(text.contains("=kill"));
        assert!(text.contains("=quit"));
    }
}
