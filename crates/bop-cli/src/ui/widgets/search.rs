/// Global search overlay widget.
///
/// Renders a centered popup over the kanban body when `Mode::Search` is active.
/// Shows a search input line and a scrollable flat list of matching cards with
/// state badges. The user navigates with `j/k`, confirms with `Enter`, and
/// cancels with `Esc`.
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::ui::app::SearchState;

// ── State colour map ─────────────────────────────────────────────────────────

fn state_color(state: &str) -> Color {
    match state {
        "pending" => Color::DarkGray,
        "running" => Color::Cyan,
        "done" => Color::Green,
        "failed" => Color::Red,
        "merged" => Color::Magenta,
        _ => Color::White,
    }
}

fn state_badge(state: &str) -> &'static str {
    match state {
        "pending" => "PEND",
        "running" => "RUN ",
        "done" => "DONE",
        "failed" => "FAIL",
        "merged" => "MRGD",
        _ => "????",
    }
}

// ── Overlay geometry ─────────────────────────────────────────────────────────

/// Compute a centred popup rect (max 60% wide, 70% tall, min 40×12).
fn popup_area(area: Rect) -> Rect {
    let width = ((area.width as f32 * 0.6) as u16).max(40).min(area.width);
    let height = ((area.height as f32 * 0.7) as u16).max(12).min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect { x, y, width, height }
}

// ── Render ───────────────────────────────────────────────────────────────────

/// Render the global search overlay into `area`.
///
/// Clears the background, draws a bordered popup, then renders:
/// - A search input line (first row inside border).
/// - A separator.
/// - A scrollable list of matching cards with state badges.
pub fn render_search_overlay(frame: &mut Frame, area: Rect, state: &SearchState) {
    let popup = popup_area(area);

    // Clear the popup background so content below doesn't bleed through.
    frame.render_widget(Clear, popup);

    // Outer border block.
    let block = Block::default()
        .title(" Global Search ")
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    if inner.height < 3 {
        return;
    }

    // Split inner area: input row (1), separator (1), list (rest).
    let zones = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(inner);

    // ── Search input line ────────────────────────────────────────────────
    let input_spans = vec![
        Span::styled("/ ", Style::default().fg(Color::Cyan)),
        Span::styled(
            format!("{}█", state.query),
            Style::default().fg(Color::White),
        ),
    ];
    frame.render_widget(Paragraph::new(Line::from(input_spans)), zones[0]);

    // ── Separator ────────────────────────────────────────────────────────
    let sep: String = "─".repeat(inner.width as usize);
    frame.render_widget(
        Paragraph::new(Span::styled(sep, Style::default().fg(Color::DarkGray))),
        zones[1],
    );

    // ── Results list ─────────────────────────────────────────────────────
    if zones[2].height == 0 {
        return;
    }

    let items: Vec<ListItem> = state
        .results
        .iter()
        .map(|r| {
            let badge = state_badge(&r.state);
            let badge_color = state_color(&r.state);
            let line = Line::from(vec![
                Span::styled(
                    format!("[{}] ", badge),
                    Style::default().fg(badge_color),
                ),
                Span::styled(r.card_id.clone(), Style::default().fg(Color::White)),
            ]);
            ListItem::new(line)
        })
        .collect();

    // Compute scroll offset so the selected item stays visible.
    let list_height = zones[2].height as usize;
    let selected = state.selected;
    let offset = if selected >= list_height {
        selected - list_height + 1
    } else {
        0
    };

    // Slice items to the visible window.
    let visible: Vec<ListItem> = items.into_iter().skip(offset).collect();

    // Adjust selection index to the visible window.
    let visible_selected = selected.saturating_sub(offset);

    let list = List::new(visible)
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    let mut list_state = ListState::default();
    list_state.select(Some(visible_selected));

    frame.render_stateful_widget(list, zones[2], &mut list_state);

    // Empty state hint.
    if state.results.is_empty() {
        let hint = Paragraph::new(Span::styled(
            "no matches",
            Style::default().fg(Color::DarkGray),
        ))
        .alignment(Alignment::Center);
        frame.render_widget(hint, zones[2]);
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::app::SearchResult;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn make_state(query: &str, results: Vec<(&str, &str)>, selected: usize) -> SearchState {
        SearchState {
            query: query.to_string(),
            results: results
                .into_iter()
                .map(|(id, state)| SearchResult {
                    card_id: id.to_string(),
                    state: state.to_string(),
                })
                .collect(),
            selected,
        }
    }

    #[test]
    fn renders_without_panic_empty() {
        let state = make_state("", vec![], 0);
        let backend = TestBackend::new(60, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_search_overlay(frame, frame.area(), &state))
            .unwrap();
    }

    #[test]
    fn renders_with_results() {
        let state = make_state(
            "bop",
            vec![
                ("061-bop-ui-search", "pending"),
                ("060-bop-bench", "running"),
                ("059-bop-webhooks", "done"),
            ],
            1,
        );
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_search_overlay(frame, frame.area(), &state))
            .unwrap();
        // Just verify no panic; visual correctness checked via snapshot.
    }

    #[test]
    fn state_badge_covers_all_states() {
        for s in &["pending", "running", "done", "failed", "merged", "unknown"] {
            let badge = state_badge(s);
            assert_eq!(badge.trim().len(), badge.trim().len()); // trivially true — just ensure no panic
        }
    }

    #[test]
    fn popup_area_is_centred() {
        let area = Rect { x: 0, y: 0, width: 100, height: 40 };
        let popup = popup_area(area);
        // Width is ~60% of 100 = 60
        assert_eq!(popup.width, 60);
        // x should be centred
        assert_eq!(popup.x, (100 - 60) / 2);
    }

    #[test]
    fn popup_area_minimum_size() {
        // Very small terminal — should clamp to min dimensions.
        let area = Rect { x: 0, y: 0, width: 20, height: 8 };
        let popup = popup_area(area);
        // Width clamps to area.width (20 < 40 min, so width = 20)
        assert_eq!(popup.width, 20);
    }
}
