use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};
use ratatui::Frame;

use crate::ui::app::App;

/// Render the integrated log stream pane for `AppTab::Log(card_id)`.
pub fn render_log_pane(frame: &mut Frame, area: Rect, app: &App, card_id: &str) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let status = app
        .card_by_id(card_id)
        .map(|card| card.state.as_str())
        .unwrap_or("missing");
    let elapsed = app
        .card_by_id(card_id)
        .and_then(|card| card.elapsed_s)
        .map(format_elapsed)
        .unwrap_or_else(|| "--".to_string());

    let mut title_spans = vec![
        Span::styled(
            format!(" LOG  {card_id}  •  {status}  {elapsed} "),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("[L] close ", Style::default().fg(Color::DarkGray)),
    ];

    if !app.log_pane_follow {
        title_spans.push(Span::styled(
            " ↓ following paused",
            Style::default().fg(Color::Yellow),
        ));
    }

    let lines: Vec<Line> = if app.log_buf.is_empty() {
        vec![Line::from(Span::styled(
            "no output yet",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        app.log_buf
            .iter()
            .map(|line| Line::from(Span::raw(line.clone())))
            .collect()
    };

    let visible_height = area.height.saturating_sub(2) as usize;
    let max_top = lines.len().saturating_sub(visible_height);
    let top = if app.log_pane_follow {
        max_top
    } else {
        max_top.saturating_sub(app.log_pane_scroll.min(max_top))
    };

    let pane = Paragraph::new(lines)
        .block(
            Block::bordered()
                .title(Line::from(title_spans))
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .wrap(Wrap { trim: false })
        .scroll((top.min(u16::MAX as usize) as u16, 0));

    frame.render_widget(pane, area);
}

fn format_elapsed(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60)
    }
}

#[cfg(test)]
mod tests {
    use super::format_elapsed;

    #[test]
    fn format_elapsed_seconds() {
        assert_eq!(format_elapsed(0), "0s");
        assert_eq!(format_elapsed(59), "59s");
    }

    #[test]
    fn format_elapsed_minutes() {
        assert_eq!(format_elapsed(60), "1m 00s");
        assert_eq!(format_elapsed(192), "3m 12s");
    }

    #[test]
    fn format_elapsed_hours() {
        assert_eq!(format_elapsed(3600), "1h 00m");
        assert_eq!(format_elapsed(7260), "2h 01m");
    }
}
