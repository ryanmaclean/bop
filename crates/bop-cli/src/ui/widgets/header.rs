/// Header bar widget — top 2 rows of the three-zone TUI layout.
///
/// Line 1: `bop · HH:MM:SS · {provider meters} · {sparkline} N/hr`
/// Line 2 (width ≥ 120): scrolling event ticker of recent card completions.
///
/// Provider meters show one bar per provider from `providers.json`, colored:
/// - Green: available
/// - Amber: busy (has a running card)
/// - Red: rate-limited / in cooldown
///
/// Sparkline uses last 8 throughput samples: ▁▂▃▄▅▆▇█
use chrono::Local;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::ui::app::App;

// ── Provider status ─────────────────────────────────────────────────────────

/// Provider status for header bar meters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderStatus {
    /// Provider is available (green).
    Available,
    /// Provider has a running card (amber).
    Busy,
    /// Provider is rate-limited or in cooldown (red).
    RateLimited,
}

/// One entry in the provider meter bar.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ProviderMeter {
    pub name: String,
    pub status: ProviderStatus,
}

// ── Colors ──────────────────────────────────────────────────────────────────

/// Color for a provider meter based on status.
fn meter_color(status: ProviderStatus) -> Color {
    match status {
        ProviderStatus::Available => Color::Green,
        ProviderStatus::Busy => Color::Yellow,
        ProviderStatus::RateLimited => Color::Red,
    }
}

// ── Sparkline ───────────────────────────────────────────────────────────────

/// Sparkline block characters ordered by magnitude (index 0 = lowest).
const SPARK_CHARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Convert a throughput value to a sparkline character.
///
/// Maps `value` into the 0–7 range based on `max_val`. If `max_val` is 0
/// all values map to the lowest bar (▁).
fn spark_char(value: u8, max_val: u8) -> char {
    if max_val == 0 {
        return SPARK_CHARS[0];
    }
    let idx = ((value as usize) * 7) / (max_val as usize);
    SPARK_CHARS[idx.min(7)]
}

/// Build a sparkline string from a slice of throughput samples.
fn build_sparkline(samples: &[u8]) -> String {
    let max_val = samples.iter().copied().max().unwrap_or(0);
    samples.iter().map(|&v| spark_char(v, max_val)).collect()
}

// ── Provider meter bar ──────────────────────────────────────────────────────

/// Width of each provider meter in block characters.
const METER_WIDTH: usize = 4;

/// Build Spans for all provider meters.
///
/// Each provider gets `METER_WIDTH` block chars: filled (█) in the
/// provider's status color, followed by empty (░) in dark gray to fill
/// any remaining width. Since each meter is binary (status indicator,
/// not a gauge), we fill all blocks in the status color.
fn build_meter_spans(meters: &[ProviderMeter]) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (i, meter) in meters.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        let color = meter_color(meter.status);
        let filled = "█".repeat(METER_WIDTH);
        spans.push(Span::styled(filled, Style::default().fg(color)));
    }
    spans
}

// ── Event ticker ────────────────────────────────────────────────────────────

/// Maximum number of recent completions stored for the event ticker.
pub const TICKER_CAPACITY: usize = 20;

/// Separator between ticker entries.
const TICKER_SEP: &str = "  ·  ";

/// Build the scrolling event ticker string from recent completions.
///
/// Joins all entries with a separator, then uses `tick_offset` to scroll
/// the text left across the available width (demoscene-style marquee).
fn build_ticker_text(completions: &[String], tick_offset: usize, width: usize) -> String {
    if completions.is_empty() || width == 0 {
        return String::new();
    }

    let joined = completions.join(TICKER_SEP);
    if joined.is_empty() {
        return String::new();
    }

    // Create a repeating buffer so the scroll wraps smoothly.
    let padded = format!("{}{}{}", joined, TICKER_SEP, joined);
    let total_len = padded.chars().count();
    let offset = tick_offset % total_len;

    padded.chars().cycle().skip(offset).take(width).collect()
}

// ── Main render ─────────────────────────────────────────────────────────────

/// Render the header bar into the given area.
///
/// Uses the first row for the main status line and the second row for
/// the scrolling event ticker (only if terminal width ≥ 120).
pub fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let width = area.width as usize;

    // ── Line 1: bop · HH:MM:SS · meters · sparkline N/hr ──────────
    let time_str = Local::now().format("%H:%M:%S").to_string();

    let mut spans: Vec<Span<'static>> = vec![
        Span::styled("bop", Style::default().fg(Color::Cyan)),
        Span::styled(" · ", Style::default().fg(Color::DarkGray)),
        Span::styled(time_str, Style::default().fg(Color::White)),
    ];

    // Provider meters.
    if !app.provider_meters.is_empty() {
        spans.push(Span::styled(" · ", Style::default().fg(Color::DarkGray)));
        spans.extend(build_meter_spans(&app.provider_meters));
    }

    // Sparkline + throughput.
    let samples: Vec<u8> = app.throughput.iter().copied().collect();
    let sparkline = build_sparkline(&samples);
    let latest = samples.last().copied().unwrap_or(0);

    spans.push(Span::styled(" · ", Style::default().fg(Color::DarkGray)));
    spans.push(Span::styled(sparkline, Style::default().fg(Color::Cyan)));
    spans.push(Span::styled(
        format!(" {}/hr", latest),
        Style::default().fg(Color::DarkGray),
    ));

    let line1 = Line::from(spans);

    // ── Line 2: event ticker (width >= 120 only) ───────────────────
    let lines = if area.height >= 2 && width >= 120 {
        let completions: Vec<String> = app.recent_completions.iter().cloned().collect();
        let ticker_text = build_ticker_text(&completions, app.tick_count as usize, width);
        let line2 = Line::from(Span::styled(
            ticker_text,
            Style::default().fg(Color::DarkGray),
        ));
        vec![line1, line2]
    } else if area.height >= 2 {
        // Second line is blank when width < 120.
        vec![line1, Line::from("")]
    } else {
        vec![line1]
    };

    let header = Paragraph::new(lines);
    frame.render_widget(header, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── spark_char ──────────────────────────────────────────────────────

    #[test]
    fn spark_char_zero_max() {
        assert_eq!(spark_char(0, 0), '▁');
        assert_eq!(spark_char(5, 0), '▁');
    }

    #[test]
    fn spark_char_at_max() {
        assert_eq!(spark_char(10, 10), '█');
    }

    #[test]
    fn spark_char_at_zero() {
        assert_eq!(spark_char(0, 10), '▁');
    }

    #[test]
    fn spark_char_mid() {
        let c = spark_char(5, 10);
        // 5/10 * 7 = 3.5 → index 3 → '▄'
        assert_eq!(c, '▄');
    }

    // ── build_sparkline ─────────────────────────────────────────────────

    #[test]
    fn sparkline_all_zeros() {
        let s = build_sparkline(&[0, 0, 0, 0]);
        assert_eq!(s, "▁▁▁▁");
    }

    #[test]
    fn sparkline_ascending() {
        let s = build_sparkline(&[0, 1, 2, 3, 4, 5, 6, 7]);
        // max=7, each maps to index i*7/7 = i
        assert_eq!(s, "▁▂▃▄▅▆▇█");
    }

    #[test]
    fn sparkline_single_value() {
        let s = build_sparkline(&[5]);
        assert_eq!(s, "█"); // single value = max
    }

    #[test]
    fn sparkline_empty() {
        let s = build_sparkline(&[]);
        assert_eq!(s, "");
    }

    // ── meter_color ─────────────────────────────────────────────────────

    #[test]
    fn meter_colors() {
        assert_eq!(meter_color(ProviderStatus::Available), Color::Green);
        assert_eq!(meter_color(ProviderStatus::Busy), Color::Yellow);
        assert_eq!(meter_color(ProviderStatus::RateLimited), Color::Red);
    }

    // ── build_meter_spans ───────────────────────────────────────────────

    #[test]
    fn meter_spans_empty() {
        let spans = build_meter_spans(&[]);
        assert!(spans.is_empty());
    }

    #[test]
    fn meter_spans_single() {
        let meters = vec![ProviderMeter {
            name: "mock".into(),
            status: ProviderStatus::Available,
        }];
        let spans = build_meter_spans(&meters);
        assert_eq!(spans.len(), 1);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "████");
    }

    #[test]
    fn meter_spans_two_with_separator() {
        let meters = vec![
            ProviderMeter {
                name: "mock".into(),
                status: ProviderStatus::Available,
            },
            ProviderMeter {
                name: "claude".into(),
                status: ProviderStatus::Busy,
            },
        ];
        let spans = build_meter_spans(&meters);
        // [filled, sep, filled] = 3 spans
        assert_eq!(spans.len(), 3);
    }

    // ── build_ticker_text ───────────────────────────────────────────────

    #[test]
    fn ticker_empty_completions() {
        let text = build_ticker_text(&[], 0, 80);
        assert_eq!(text, "");
    }

    #[test]
    fn ticker_zero_width() {
        let completions = vec!["done: card-1".to_string()];
        let text = build_ticker_text(&completions, 0, 0);
        assert_eq!(text, "");
    }

    #[test]
    fn ticker_basic_scroll() {
        let completions = vec!["card-1 ✓".to_string(), "card-2 ✓".to_string()];
        let text = build_ticker_text(&completions, 0, 20);
        assert_eq!(text.chars().count(), 20);
    }

    #[test]
    fn ticker_scrolls_with_offset() {
        let completions = vec!["ABCDE".to_string()];
        let t0 = build_ticker_text(&completions, 0, 5);
        let t1 = build_ticker_text(&completions, 1, 5);
        // After offset 1, the first char should differ.
        assert_ne!(t0, t1);
    }

    #[test]
    fn ticker_wraps_around() {
        let completions = vec!["AB".to_string()];
        // Total padded = "AB  ·  AB" (len 9), offset wraps at 9.
        let t0 = build_ticker_text(&completions, 0, 5);
        let t9 = build_ticker_text(&completions, 9, 5);
        assert_eq!(t0, t9);
    }

    // ── ProviderStatus equality ─────────────────────────────────────────

    #[test]
    fn provider_status_eq() {
        assert_eq!(ProviderStatus::Available, ProviderStatus::Available);
        assert_ne!(ProviderStatus::Available, ProviderStatus::Busy);
        assert_ne!(ProviderStatus::Busy, ProviderStatus::RateLimited);
    }
}
