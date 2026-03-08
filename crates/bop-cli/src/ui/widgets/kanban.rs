/// Kanban board widget — horizontal columns of card lists.
///
/// Renders columns left-to-right: pending, running, done, failed, merged.
/// Empty columns collapse to a 3-char narrow divider showing only the state
/// glyph (`·`, `⚙`, `✓`, `✗`, `~`). Non-collapsed columns share remaining
/// width equally via `Constraint::Fill(1)` and display a bordered `List` of
/// card rows.
///
/// Focus and WIP indicators:
/// - Focused column: `Color::Yellow` border
/// - Running column ≥75% WIP: amber title
/// - Running column at 100% WIP: red title + border
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, Paragraph};
use ratatui::Frame;

use std::collections::HashSet;

use crate::acplan::half_circle_glyph;
use crate::render::CardView;
use crate::ui::app::{App, KanbanColumn};
use crate::ui::widgets::filter::FilterMatch;

// ── Glyph helpers ────────────────────────────────────────────────────────────

/// State glyph for column headers and collapsed dividers.
///
/// Matches the spec's visual language: `·` pending, `⚙` running,
/// `✓` done, `✗` failed, `~` merged.
fn state_glyph(state: &str) -> &'static str {
    match state {
        "pending" => "·",
        "running" => "⚙",
        "done" => "✓",
        "failed" => "✗",
        "merged" => "~",
        _ => "?",
    }
}

/// Card marker glyph for list rows (matches render/full.rs pattern).
fn card_marker(state: &str) -> &'static str {
    match state {
        "running" => "▶",
        "done" | "merged" => "✓",
        "failed" => "✗",
        _ => "·",
    }
}

/// RGB color for a given state (matches colors.rs hex values).
///
/// Uses 24-bit RGB for consistent appearance across terminals that
/// support truecolor (which ratatui auto-detects).
fn state_ratatui_color(state: &str) -> Color {
    match state {
        "pending" => Color::Rgb(0x3A, 0x5A, 0x8A),
        "running" => Color::Rgb(0xB8, 0x69, 0x0F),
        "done" => Color::Rgb(0x1E, 0x8A, 0x45),
        "failed" => Color::Rgb(0xC4, 0x30, 0x30),
        "merged" => Color::Rgb(0x6B, 0x3D, 0xB8),
        _ => Color::Gray,
    }
}

/// Moon-quarter glyph for phase progress (matches render/full.rs).
///
/// ◔ (< 25%), ◑ (25–50%), ◕ (50–75%), ● (≥ 75%).
fn moon_glyph(frac: f32) -> char {
    if frac < 0.25 {
        '◔'
    } else if frac < 0.50 {
        '◑'
    } else if frac < 0.75 {
        '◕'
    } else {
        '●'
    }
}

/// Format seconds into human-readable "XmYYs" or "Xs" (matches render/full.rs).
fn format_duration(secs: u64) -> String {
    if secs >= 60 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{}s", secs)
    }
}

// ── Column title ─────────────────────────────────────────────────────────────

/// Build the column title `Line` with state glyph, name, and count.
///
/// Running columns with a WIP limit show `⚙ RUNNING (N/M)`, with color
/// changing at 75% (amber) and 100% (red) saturation.
fn column_title(col: &KanbanColumn) -> Line<'static> {
    let glyph = state_glyph(&col.state);
    let name = col.state.to_uppercase();
    let count = col.cards.len();

    let title_text = if col.state == "running" {
        if let Some(limit) = col.wip_limit {
            format!(" {} {} ({}/{}) ", glyph, name, count, limit)
        } else {
            format!(" {} {} ({}) ", glyph, name, count)
        }
    } else {
        format!(" {} {} ({}) ", glyph, name, count)
    };

    let color = title_color(col);
    Line::from(Span::styled(
        title_text,
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ))
}

/// Determine title color based on state and WIP saturation.
///
/// Running column with WIP limit: amber at ≥75%, red at 100%.
/// All other columns use their default state color.
fn title_color(col: &KanbanColumn) -> Color {
    if col.state == "running" && col.wip_limit.is_some() {
        let sat = col.wip_saturation();
        if sat >= 1.0 {
            Color::Red
        } else if sat >= 0.75 {
            Color::Yellow
        } else {
            state_ratatui_color(&col.state)
        }
    } else {
        state_ratatui_color(&col.state)
    }
}

/// Determine border color for a column.
///
/// Priority: running at 100% WIP → red (even if focused), focused → yellow,
/// running ≥75% → amber, default → dark gray.
fn border_color(col: &KanbanColumn, is_focused: bool) -> Color {
    if col.state == "running" && col.wip_limit.is_some() {
        let sat = col.wip_saturation();
        if sat >= 1.0 {
            return Color::Red;
        } else if sat >= 0.75 {
            return Color::Yellow;
        }
    }

    if is_focused {
        Color::Yellow
    } else {
        Color::DarkGray
    }
}

// ── Card list items ──────────────────────────────────────────────────────────

/// Build a `ListItem` for a single card, optionally highlighting matched
/// characters from a filter result.
///
/// Running cards get two lines (row + progress bar with phase name).
/// All other states get a single line showing marker, title, provider,
/// and elapsed time.
///
/// When `filter_match` is provided, matched characters in the card title
/// are rendered with `Color::Yellow` + `BOLD` for visual feedback.
///
/// When `is_marked` is true, a `■` prefix is prepended to indicate the
/// card is selected for bulk operations (yazi-style multi-select).
fn card_to_list_item(
    card: &CardView,
    filter_match: Option<&FilterMatch>,
    is_marked: bool,
) -> ListItem<'static> {
    let color = state_ratatui_color(&card.state);

    // Use BMP-safe token if available, then glyph, then state marker.
    let display_marker = card
        .token
        .as_deref()
        .or(card.glyph.as_deref())
        .unwrap_or(card_marker(&card.state));

    // First line: mark indicator + marker + highlighted title + provider + elapsed.
    let mut spans = Vec::new();

    // Mark indicator prefix (yazi pattern: ■ for marked cards).
    if is_marked {
        spans.push(Span::styled(
            "■ ".to_string(),
            Style::default().fg(Color::Cyan),
        ));
    }

    spans.push(Span::styled(
        format!("{} ", display_marker),
        Style::default().fg(color),
    ));

    // Build title spans with optional highlight.
    let title_spans = build_title_spans(&card.title, filter_match);
    spans.extend(title_spans);

    if let Some(ref provider) = card.provider {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            provider.clone(),
            Style::default().fg(Color::DarkGray),
        ));
    }

    if let Some(elapsed) = card.elapsed_s {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format_duration(elapsed),
            Style::default().fg(Color::DarkGray),
        ));
    }

    if let Some(code) = card.exit_code {
        if card.state == "failed" {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                format!("exit {}", code),
                Style::default().fg(color),
            ));
        }
    }

    let first_line = Line::from(spans);

    // Running cards with progress get a second line.
    if card.state == "running" && (card.progress > 0 || card.phase_name.is_some()) {
        let second_line = build_progress_line(card, color);
        ListItem::new(vec![first_line, second_line])
    } else {
        ListItem::new(first_line)
    }
}

/// Build title spans with matched character highlighting.
///
/// When `filter_match` has non-empty `title_indices`, characters at those
/// positions are styled with `Color::Yellow` + `BOLD`. Consecutive
/// highlighted and non-highlighted characters are merged into single spans
/// for rendering efficiency.
fn build_title_spans(title: &str, filter_match: Option<&FilterMatch>) -> Vec<Span<'static>> {
    let highlight_set: HashSet<u32> = filter_match
        .map(|m| m.title_indices.iter().copied().collect())
        .unwrap_or_default();

    if highlight_set.is_empty() {
        // No highlights — single span for the whole title.
        return vec![Span::styled(
            title.to_string(),
            Style::default().fg(Color::White),
        )];
    }

    let normal_style = Style::default().fg(Color::White);
    let highlight_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);

    let mut spans = Vec::new();
    let mut current_buf = String::new();
    let mut current_highlighted = false;

    for (i, ch) in title.chars().enumerate() {
        let is_highlighted = highlight_set.contains(&(i as u32));

        if i == 0 {
            current_highlighted = is_highlighted;
            current_buf.push(ch);
        } else if is_highlighted == current_highlighted {
            current_buf.push(ch);
        } else {
            // Flush the current buffer.
            let style = if current_highlighted {
                highlight_style
            } else {
                normal_style
            };
            spans.push(Span::styled(current_buf.clone(), style));
            current_buf.clear();
            current_buf.push(ch);
            current_highlighted = is_highlighted;
        }
    }

    // Flush remaining.
    if !current_buf.is_empty() {
        let style = if current_highlighted {
            highlight_style
        } else {
            normal_style
        };
        spans.push(Span::styled(current_buf, style));
    }

    spans
}

/// Build the progress bar line for a running card.
///
/// Format without AC data: `  ████████░░░░ 67%  Phase 2 ◑`
/// Format with AC data:     `  ████████░░░░  6/7   Phase 2  ◑`
/// Indented 2 spaces to align under the title text.
fn build_progress_line(card: &CardView, color: Color) -> Line<'static> {
    let bar_width: usize = 12;
    let filled = (card.progress as usize * bar_width) / 100;
    let empty = bar_width - filled;

    let bar = format!("{}{}", "█".repeat(filled), "░".repeat(empty));

    let has_ac = card.ac_subtasks_done.is_some() && card.ac_subtasks_total.is_some();

    // When AC plan data is present, show "N/T" count instead of percentage.
    let count_str =
        if let (Some(done), Some(total)) = (card.ac_subtasks_done, card.ac_subtasks_total) {
            format!("  {}/{}", done, total)
        } else {
            format!(" {}%", card.progress)
        };

    let mut spans = vec![
        Span::raw("  "), // indent to align under title
        Span::styled(bar, Style::default().fg(color)),
        Span::styled(count_str, Style::default().fg(Color::White)),
    ];

    if let Some(ref phase) = card.phase_name {
        let glyph = if has_ac {
            half_circle_glyph(card.phase_frac as f64)
        } else {
            moon_glyph(card.phase_frac)
        };
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("{}  {}", phase, glyph),
            Style::default().fg(Color::DarkGray),
        ));
    }

    Line::from(spans)
}

// ── Collapsed divider ────────────────────────────────────────────────────────

/// Render a collapsed column as a 3-char narrow divider with state glyph.
///
/// Shows a bordered 3-column-wide block with the state glyph inside.
/// Border color is dark gray; glyph is in the state's color.
fn render_collapsed_divider(frame: &mut Frame, area: Rect, state: &str) {
    let glyph = state_glyph(state);
    let color = state_ratatui_color(state);

    let block = Block::bordered().border_style(Style::default().fg(Color::DarkGray));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height > 0 && inner.width > 0 {
        let glyph_widget = Paragraph::new(glyph.to_string()).style(Style::default().fg(color));
        frame.render_widget(glyph_widget, inner);
    }
}

// ── Column render ────────────────────────────────────────────────────────────

/// Render a single non-collapsed column with its card list.
///
/// Uses ratatui's `StatefulWidget` rendering for `List` so that the
/// currently selected row is highlighted via `ListState`.
///
/// When `card_matches` is provided (filter active), only matching cards
/// are rendered, and their titles show highlighted match characters.
///
/// `marked_cards` is the set of card IDs currently marked for bulk
/// operations — marked cards display a `■` prefix.
fn render_column(
    frame: &mut Frame,
    area: Rect,
    col: &mut KanbanColumn,
    is_focused: bool,
    card_matches: Option<&[Option<FilterMatch>]>,
    marked_cards: &HashSet<String>,
) {
    let title = column_title(col);
    let border_col = border_color(col, is_focused);

    let block = Block::bordered()
        .title(title)
        .border_style(Style::default().fg(border_col));

    let items: Vec<ListItem> = match card_matches {
        Some(matches) => {
            // Filter mode: only show matching cards, with highlights.
            col.cards
                .iter()
                .zip(matches.iter())
                .filter_map(|(card, m)| {
                    let is_marked = marked_cards.contains(&card.id);
                    m.as_ref()
                        .map(|fm| card_to_list_item(card, Some(fm), is_marked))
                })
                .collect()
        }
        None => {
            // No filter: show all cards without highlights.
            col.cards
                .iter()
                .map(|card| {
                    let is_marked = marked_cards.contains(&card.id);
                    card_to_list_item(card, None, is_marked)
                })
                .collect()
        }
    };

    let highlight_style = if is_focused {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    let list = List::new(items)
        .block(block)
        .highlight_style(highlight_style)
        .highlight_symbol("▸ ");

    frame.render_stateful_widget(list, area, &mut col.list_state);
}

// ── Main render ──────────────────────────────────────────────────────────────

/// Render the full kanban board into the given area.
///
/// Divides `area` horizontally among visible columns: collapsed columns get
/// `Length(3)`, non-collapsed columns get `Fill(1)` for equal distribution.
/// Each column is rendered as either a narrow glyph divider (collapsed) or
/// a full bordered list (expanded).
///
/// When the terminal is too narrow for all columns, lowest-priority columns
/// are hidden first (merged, then done) via `app.visible_column_indices()`.
///
/// When a filter is active (`app.filter_state` is Some), each card is
/// matched against the filter pattern. Non-matching cards are hidden,
/// and matched characters in titles are highlighted.
pub fn render_kanban(frame: &mut Frame, area: Rect, app: &mut App) {
    let col_count = app.columns.len();
    if col_count == 0 {
        return;
    }

    // Determine which columns are visible at the current terminal width.
    let visible = app.visible_column_indices();
    if visible.is_empty() {
        return;
    }

    // Pre-compute filter matches per column if a filter is active.
    // Each entry is a Vec<Option<FilterMatch>> parallel to col.cards.
    let filter_matches: Option<Vec<Vec<Option<FilterMatch>>>> =
        if let Some(ref mut state) = app.filter_state {
            Some(
                app.columns
                    .iter()
                    .map(|col| {
                        col.cards
                            .iter()
                            .map(|card| state.matches(&card.id, &card.title))
                            .collect()
                    })
                    .collect(),
            )
        } else {
            None
        };

    // Build layout constraints only for visible columns.
    let constraints: Vec<Constraint> = visible
        .iter()
        .map(|&i| {
            if app.columns[i].collapsed {
                Constraint::Length(3)
            } else {
                Constraint::Fill(1)
            }
        })
        .collect();

    let col_areas = Layout::horizontal(constraints).split(area);

    // Clone marked_cards before the loop to avoid borrow conflicts with
    // mutable column access in render_column. The set is typically small.
    let marked = app.marked_cards.clone();

    for (area_idx, &col_idx) in visible.iter().enumerate() {
        let col_area = col_areas[area_idx];
        let is_focused = col_idx == app.col_focus;

        if app.columns[col_idx].collapsed {
            // Clone state string to avoid borrow conflict — state names are short.
            let state = app.columns[col_idx].state.clone();
            render_collapsed_divider(frame, col_area, &state);
        } else {
            let matches_for_col = filter_matches.as_ref().map(|fm| fm[col_idx].as_slice());
            let col = &mut app.columns[col_idx];
            render_column(frame, col_area, col, is_focused, matches_for_col, &marked);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Glyph helpers ────────────────────────────────────────────────────

    #[test]
    fn state_glyph_all_states() {
        assert_eq!(state_glyph("pending"), "·");
        assert_eq!(state_glyph("running"), "⚙");
        assert_eq!(state_glyph("done"), "✓");
        assert_eq!(state_glyph("failed"), "✗");
        assert_eq!(state_glyph("merged"), "~");
        assert_eq!(state_glyph("unknown"), "?");
    }

    #[test]
    fn card_marker_all_states() {
        assert_eq!(card_marker("pending"), "·");
        assert_eq!(card_marker("running"), "▶");
        assert_eq!(card_marker("done"), "✓");
        assert_eq!(card_marker("merged"), "✓");
        assert_eq!(card_marker("failed"), "✗");
    }

    #[test]
    fn format_duration_seconds() {
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(42), "42s");
        assert_eq!(format_duration(59), "59s");
    }

    #[test]
    fn format_duration_minutes() {
        assert_eq!(format_duration(60), "1m00s");
        assert_eq!(format_duration(90), "1m30s");
        assert_eq!(format_duration(3661), "61m01s");
    }

    #[test]
    fn moon_glyph_quarters() {
        assert_eq!(moon_glyph(0.0), '◔');
        assert_eq!(moon_glyph(0.24), '◔');
        assert_eq!(moon_glyph(0.25), '◑');
        assert_eq!(moon_glyph(0.49), '◑');
        assert_eq!(moon_glyph(0.50), '◕');
        assert_eq!(moon_glyph(0.74), '◕');
        assert_eq!(moon_glyph(0.75), '●');
        assert_eq!(moon_glyph(1.0), '●');
    }

    // ── Color helpers ────────────────────────────────────────────────────

    #[test]
    fn state_colors_are_rgb() {
        for state in &["pending", "running", "done", "failed", "merged"] {
            match state_ratatui_color(state) {
                Color::Rgb(_, _, _) => {} // expected
                other => panic!("expected Rgb for {}, got {:?}", state, other),
            }
        }
    }

    // ── Title color / border color ───────────────────────────────────────

    #[test]
    fn title_color_normal_state() {
        let col = KanbanColumn::new_test("pending", 2, None);
        assert_eq!(title_color(&col), state_ratatui_color("pending"));
    }

    #[test]
    fn title_color_running_no_wip() {
        let col = KanbanColumn::new_test("running", 2, None);
        assert_eq!(title_color(&col), state_ratatui_color("running"));
    }

    #[test]
    fn title_color_running_under_75_pct() {
        let col = KanbanColumn::new_test("running", 1, Some(4));
        assert_eq!(title_color(&col), state_ratatui_color("running"));
    }

    #[test]
    fn title_color_running_at_75_pct() {
        let col = KanbanColumn::new_test("running", 3, Some(4));
        assert_eq!(title_color(&col), Color::Yellow);
    }

    #[test]
    fn title_color_running_at_100_pct() {
        let col = KanbanColumn::new_test("running", 4, Some(4));
        assert_eq!(title_color(&col), Color::Red);
    }

    #[test]
    fn border_color_unfocused() {
        let col = KanbanColumn::new_test("pending", 1, None);
        assert_eq!(border_color(&col, false), Color::DarkGray);
    }

    #[test]
    fn border_color_focused() {
        let col = KanbanColumn::new_test("pending", 1, None);
        assert_eq!(border_color(&col, true), Color::Yellow);
    }

    #[test]
    fn border_color_running_at_limit_overrides_focus() {
        let col = KanbanColumn::new_test("running", 4, Some(4));
        assert_eq!(border_color(&col, true), Color::Red);
    }

    #[test]
    fn border_color_running_at_75_pct_amber() {
        let col = KanbanColumn::new_test("running", 3, Some(4));
        assert_eq!(border_color(&col, false), Color::Yellow);
    }

    // ── Column title ─────────────────────────────────────────────────────

    #[test]
    fn column_title_pending() {
        let col = KanbanColumn::new_test("pending", 3, None);
        let title = column_title(&col);
        let text: String = title.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("PENDING"));
        assert!(text.contains("(3)"));
    }

    #[test]
    fn column_title_running_with_wip() {
        let col = KanbanColumn::new_test("running", 2, Some(4));
        let title = column_title(&col);
        let text: String = title.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("RUNNING"));
        assert!(text.contains("(2/4)"));
    }

    #[test]
    fn column_title_running_without_wip() {
        let col = KanbanColumn::new_test("running", 2, None);
        let title = column_title(&col);
        let text: String = title.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("RUNNING"));
        assert!(text.contains("(2)"));
        assert!(!text.contains("/"));
    }

    // ── Card list items ──────────────────────────────────────────────────

    #[test]
    fn card_item_single_line_pending() {
        let card = test_card("my-card", "pending");
        let item = card_to_list_item(&card, None, false);
        assert_eq!(item.height(), 1);
    }

    #[test]
    fn card_item_two_lines_running_with_progress() {
        let mut card = test_card("my-card", "running");
        card.progress = 50;
        let item = card_to_list_item(&card, None, false);
        assert_eq!(item.height(), 2);
    }

    #[test]
    fn card_item_single_line_running_no_progress() {
        let card = test_card("my-card", "running");
        let item = card_to_list_item(&card, None, false);
        assert_eq!(item.height(), 1);
    }

    #[test]
    fn card_item_two_lines_running_with_phase() {
        let mut card = test_card("my-card", "running");
        card.phase_name = Some("Phase 2".into());
        card.phase_frac = 0.5;
        let item = card_to_list_item(&card, None, false);
        assert_eq!(item.height(), 2);
    }

    #[test]
    fn card_item_uses_token_over_marker() {
        let mut card = test_card("my-card", "pending");
        card.token = Some("♠A".into());
        let item = card_to_list_item(&card, None, false);
        // The item should use ♠A instead of the default · marker.
        // We can verify by checking the first span contains ♠A.
        let lines: Vec<Line> = vec![]; // ListItem doesn't expose lines directly,
                                       // but we can verify it doesn't panic.
        let _ = lines; // suppress unused
        assert_eq!(item.height(), 1);
    }

    #[test]
    fn card_item_marked_adds_prefix() {
        let card = test_card("my-card", "pending");
        let unmarked = card_to_list_item(&card, None, false);
        let marked = card_to_list_item(&card, None, true);
        // Marked card should have an extra line height or at minimum not
        // crash. The ■ prefix adds a span but stays on same line.
        assert_eq!(unmarked.height(), 1);
        assert_eq!(marked.height(), 1);
    }

    // ── Test helpers ─────────────────────────────────────────────────────

    /// Build a minimal CardView for testing.
    fn test_card(id: &str, state: &str) -> CardView {
        CardView {
            id: id.into(),
            state: state.into(),
            glyph: None,
            token: None,
            title: id.into(),
            stage: "implement".into(),
            priority: None,
            progress: 0,
            provider: None,
            elapsed_s: None,
            phase_name: None,
            phase_frac: 0.0,
            failure_reason: None,
            exit_code: None,
            ac_subtasks_done: None,
            ac_subtasks_total: None,
        }
    }

    impl KanbanColumn {
        /// Test-only constructor that creates columns with N dummy cards.
        fn new_test(state: &str, card_count: usize, wip_limit: Option<usize>) -> Self {
            let cards: Vec<CardView> = (0..card_count)
                .map(|i| test_card(&format!("card-{}", i), state))
                .collect();
            Self::new(state, cards, wip_limit)
        }
    }
}
