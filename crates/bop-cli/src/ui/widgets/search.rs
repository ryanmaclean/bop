/// Global search overlay widget for kanban card lookup.
///
/// Renders a centered popup above the kanban body with:
/// - one-line search input (`Search: <query>█`)
/// - flat result list across all states with state badges
///   (`[running] card-id`)
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::ui::app::SearchState;

/// Render the global search overlay centered in the body area.
pub fn render_search_overlay(frame: &mut Frame, body_area: Rect, search: &SearchState) {
    if body_area.width < 24 || body_area.height < 5 {
        return;
    }

    let max_width = body_area.width.saturating_sub(2);
    let popup_width = 72u16.min(max_width).max(24u16.min(max_width));

    let list_rows = search.results.len().clamp(1, 8) as u16;
    let desired_height = list_rows + 4; // border + input + list
    let max_height = body_area.height.saturating_sub(2);
    let popup_height = desired_height.min(max_height).max(5);

    let popup_area = centered_rect(popup_width, popup_height, body_area);

    frame.render_widget(Clear, popup_area);

    let block = Block::bordered()
        .title(" Global Search ")
        .title_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .border_style(Style::default().fg(Color::Yellow));
    frame.render_widget(block, popup_area);

    let inner = Rect::new(
        popup_area.x.saturating_add(1),
        popup_area.y.saturating_add(1),
        popup_area.width.saturating_sub(2),
        popup_area.height.saturating_sub(2),
    );
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let zones = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(inner);

    let query_line = Line::from(vec![
        Span::styled("Search: ", Style::default().fg(Color::Yellow)),
        Span::styled(
            format!("{}█", search.query),
            Style::default().fg(Color::White),
        ),
    ]);
    frame.render_widget(Paragraph::new(query_line), zones[0]);

    let items: Vec<ListItem> = if search.results.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "No matches",
            Style::default().fg(Color::DarkGray),
        )))]
    } else {
        search
            .results
            .iter()
            .map(|result| {
                let state = format!("[{}]", result.state);
                ListItem::new(Line::from(vec![
                    Span::styled(state, Style::default().fg(state_color(&result.state))),
                    Span::raw(" "),
                    Span::styled(result.id.clone(), Style::default().fg(Color::White)),
                ]))
            })
            .collect()
    };

    let mut list_state = ListState::default();
    if !search.results.is_empty() {
        list_state.select(Some(search.selected.min(search.results.len() - 1)));
    }

    let list = List::new(items)
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");
    frame.render_stateful_widget(list, zones[1], &mut list_state);
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}

fn state_color(state: &str) -> Color {
    match state {
        "pending" => Color::Blue,
        "running" => Color::Yellow,
        "done" => Color::Green,
        "failed" => Color::Red,
        "merged" => Color::Magenta,
        _ => Color::DarkGray,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::app::{SearchResult, SearchState};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn buffer_to_string(buf: &ratatui::buffer::Buffer) -> String {
        let area = buf.area();
        let mut lines = Vec::new();
        for y in area.y..area.y + area.height {
            let mut row = String::new();
            for x in area.x..area.x + area.width {
                row.push_str(buf[(x, y)].symbol());
            }
            lines.push(row.trim_end().to_string());
        }
        lines.join("\n")
    }

    #[test]
    fn renders_query_and_state_badges() {
        let search = SearchState {
            query: "build".into(),
            results: vec![
                SearchResult {
                    id: "build-ui".into(),
                    state: "running".into(),
                },
                SearchResult {
                    id: "build-tests".into(),
                    state: "pending".into(),
                },
            ],
            selected: 1,
            previous_card_id: Some("build-ui".into()),
        };

        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_search_overlay(frame, frame.area(), &search);
            })
            .unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(content.contains("Global Search"));
        assert!(content.contains("Search: build"));
        assert!(content.contains("[running] build-ui"));
        assert!(content.contains("[pending] build-tests"));
    }
}
