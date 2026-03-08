/// Level 2 — Extended renderer (16-color, single-line box drawing).
///
/// Uses `┌─┐│└─┘` borders and left-accent `▌` (U+258C) stripes in state color.
/// Progress bar: `████░░░░` 16-cell (filled U+2588, empty U+2591) with percentage.
/// Phase name + moon-quarter glyph (◔◑◕●) for phase progress.
use crate::acplan::half_circle_glyph;
use crate::colors::{BOLD, DIM, RESET};

use super::{CardRenderer, CardView, Stats};

pub struct ExtendedRenderer {
    pub width: u16,
}

/// 16-color ANSI foreground code for a given state (bright variants 90–97).
fn state_color_16(state: &str) -> &'static str {
    match state {
        "pending" => "\x1b[94m", // bright blue
        "running" => "\x1b[93m", // bright yellow
        "done" => "\x1b[92m",    // bright green
        "failed" => "\x1b[91m",  // bright red
        "merged" => "\x1b[95m",  // bright magenta
        _ => "\x1b[37m",         // white
    }
}

/// Moon-quarter glyph for phase progress fraction.
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

/// Format seconds into human-readable "XmYYs" or "Xs".
fn format_duration(secs: u64) -> String {
    if secs >= 60 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{}s", secs)
    }
}

impl CardRenderer for ExtendedRenderer {
    fn render_section_header(&self, label: &str, count: usize) -> String {
        let color = state_color_16(label);
        let title = format!(" {} ({}) ", label.to_uppercase(), count);
        let w = self.width as usize;
        // Account for ┌ and ┐ (1 column each).
        let inner = w.saturating_sub(2);
        let fill_len = inner.saturating_sub(title.len());
        let fill = "─".repeat(fill_len);
        format!("{}{}┌{}{}┐{}", BOLD, color, title, fill, RESET)
    }

    fn render_card_row(&self, card: &CardView) -> String {
        let color = state_color_16(&card.state);

        // Left accent stripe ▌ in state color.
        let accent = format!("{}▌{}", color, RESET);

        let marker = match card.state.as_str() {
            "running" => format!("{}▶{}", color, RESET),
            "done" | "merged" => format!("{}✓{}", color, RESET),
            "failed" => format!("{}✗{}", color, RESET),
            _ => "·".to_string(),
        };

        let mut parts = vec![format!("{} {} {}", accent, marker, card.title)];

        if let Some(ref provider) = card.provider {
            parts.push(format!("{}{}{}", DIM, provider, RESET));
        }

        if let Some(elapsed) = card.elapsed_s {
            parts.push(format!("{}{}{}", DIM, format_duration(elapsed), RESET));
        }

        if card.progress > 0 {
            parts.push(format!("{}%", card.progress));
        }

        if let Some(code) = card.exit_code {
            if card.state == "failed" {
                parts.push(format!("{}exit {}{}", color, code, RESET));
            }
        }

        parts.join("  ")
    }

    fn render_progress(
        &self,
        pct: u8,
        phase: Option<&str>,
        phase_frac: f32,
        ac_done: Option<usize>,
        ac_total: Option<usize>,
    ) -> String {
        let bar_width = 16;
        let filled = (pct as usize * bar_width) / 100;
        let empty = bar_width - filled;

        let bar = format!("{}{}", "█".repeat(filled), "░".repeat(empty));

        // When AC plan data is present, show "N/T" count and use half_circle_glyph.
        let count_str = match (ac_done, ac_total) {
            (Some(done), Some(total)) => format!("  {}/{}", done, total),
            _ => format!(" {}%", pct),
        };

        let phase_str = if let Some(phase) = phase {
            let glyph = if ac_done.is_some() && ac_total.is_some() {
                half_circle_glyph(phase_frac as f64)
            } else {
                moon_glyph(phase_frac)
            };
            format!("   {}  {}", phase, glyph)
        } else {
            String::new()
        };

        format!("│   {}{}{}", bar, count_str, phase_str)
    }

    fn render_footer(&self, stats: &Stats) -> String {
        let mut parts = vec![format!("{} total", stats.total)];

        if let Some(rate) = stats.success_rate_pct {
            parts.push(format!("{:.0}% success", rate));
        }

        if let Some(avg) = stats.avg_duration_s {
            parts.push(format!("avg {}", format_duration(avg as u64)));
        }

        let content = parts.join(" │ ");
        let content_len = content.chars().count();
        let w = self.width as usize;
        // └─ content ─┘ → 4 extra chars for └─ and ─┘.
        let fill_len = w.saturating_sub(content_len + 4);
        let fill = "─".repeat(fill_len);
        format!("{}└─ {} {}┘{}", DIM, content, fill, RESET)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    /// Helper: build a minimal CardView with sensible defaults.
    fn card(id: &str, state: &str) -> CardView {
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

    /// Assert the string contains ANSI escape sequences.
    fn assert_has_ansi(s: &str) {
        assert!(
            s.contains('\x1b'),
            "expected ANSI escape sequence but found none in: {}",
            s
        );
    }

    /// Assert the string contains Unicode box-drawing characters.
    fn assert_has_box_chars(s: &str) {
        assert!(
            s.contains('┌')
                || s.contains('─')
                || s.contains('┐')
                || s.contains('│')
                || s.contains('└')
                || s.contains('┘'),
            "expected box drawing chars (┌─┐│└┘) in: {}",
            s
        );
    }

    // ── Section header ──────────────────────────────────────────────────

    #[test]
    fn header_format() {
        let r = ExtendedRenderer { width: 80 };
        let h = r.render_section_header("running", 2);
        assert!(h.contains("RUNNING"));
        assert!(h.contains("(2)"));
    }

    #[test]
    fn header_uppercases_label() {
        let r = ExtendedRenderer { width: 80 };
        let h = r.render_section_header("pending", 0);
        assert!(h.contains("PENDING"));
    }

    #[test]
    fn header_zero_count() {
        let r = ExtendedRenderer { width: 80 };
        let h = r.render_section_header("done", 0);
        assert!(h.contains("DONE"));
        assert!(h.contains("(0)"));
    }

    #[test]
    fn header_has_ansi_colors() {
        let r = ExtendedRenderer { width: 80 };
        let h = r.render_section_header("running", 3);
        assert_has_ansi(&h);
    }

    #[test]
    fn header_has_box_drawing() {
        let r = ExtendedRenderer { width: 80 };
        let h = r.render_section_header("done", 1);
        assert!(h.contains('┌'), "expected ┌ in: {}", h);
        assert!(h.contains('┐'), "expected ┐ in: {}", h);
        assert!(h.contains('─'), "expected ─ in: {}", h);
        assert_has_box_chars(&h);
    }

    #[test]
    fn header_colored_by_state() {
        let r = ExtendedRenderer { width: 80 };

        // Running uses bright yellow (\x1b[93m)
        let h = r.render_section_header("running", 1);
        assert!(
            h.contains("\x1b[93m"),
            "expected bright yellow for running: {}",
            h
        );

        // Failed uses bright red (\x1b[91m)
        let h = r.render_section_header("failed", 1);
        assert!(
            h.contains("\x1b[91m"),
            "expected bright red for failed: {}",
            h
        );

        // Done uses bright green (\x1b[92m)
        let h = r.render_section_header("done", 1);
        assert!(
            h.contains("\x1b[92m"),
            "expected bright green for done: {}",
            h
        );
    }

    #[test]
    fn header_fill_adapts_to_width() {
        let narrow = ExtendedRenderer { width: 40 };
        let wide = ExtendedRenderer { width: 120 };
        let h_narrow = narrow.render_section_header("running", 1);
        let h_wide = wide.render_section_header("running", 1);
        // Wider terminal should produce more ─ fill chars.
        let count_narrow = h_narrow.matches('─').count();
        let count_wide = h_wide.matches('─').count();
        assert!(
            count_wide > count_narrow,
            "wide ({}) should have more ─ than narrow ({})",
            count_wide,
            count_narrow
        );
    }

    // ── Card row markers ────────────────────────────────────────────────

    #[test]
    fn row_has_accent_stripe() {
        let r = ExtendedRenderer { width: 80 };
        let c = card("test-card", "running");
        let row = r.render_card_row(&c);
        assert!(row.contains('▌'), "expected accent stripe ▌ in: {}", row);
    }

    #[test]
    fn row_accent_stripe_colored() {
        let r = ExtendedRenderer { width: 80 };
        let c = card("test", "running");
        let row = r.render_card_row(&c);
        // Bright yellow (\x1b[93m) immediately before ▌.
        assert!(
            row.contains("\x1b[93m▌"),
            "accent stripe should be colored yellow: {}",
            row
        );
    }

    #[test]
    fn row_running_marker() {
        let r = ExtendedRenderer { width: 80 };
        let c = card("my-feature", "running");
        let row = r.render_card_row(&c);
        assert!(row.contains('▶'), "expected ▶ marker in: {}", row);
    }

    #[test]
    fn row_pending_marker() {
        let r = ExtendedRenderer { width: 80 };
        let c = card("docs-update", "pending");
        let row = r.render_card_row(&c);
        assert!(row.contains('·'), "expected · marker in: {}", row);
    }

    #[test]
    fn row_done_marker() {
        let r = ExtendedRenderer { width: 80 };
        let c = card("perf-improvements", "done");
        let row = r.render_card_row(&c);
        assert!(row.contains('✓'), "expected ✓ marker in: {}", row);
    }

    #[test]
    fn row_merged_marker() {
        let r = ExtendedRenderer { width: 80 };
        let c = card("merged-card", "merged");
        let row = r.render_card_row(&c);
        assert!(row.contains('✓'), "expected ✓ marker in: {}", row);
    }

    #[test]
    fn row_failed_marker() {
        let r = ExtendedRenderer { width: 80 };
        let c = card("broken-card", "failed");
        let row = r.render_card_row(&c);
        assert!(row.contains('✗'), "expected ✗ marker in: {}", row);
    }

    #[test]
    fn row_has_ansi() {
        let r = ExtendedRenderer { width: 80 };
        let c = card("test-card", "running");
        let row = r.render_card_row(&c);
        assert_has_ansi(&row);
    }

    // ── Card row content ────────────────────────────────────────────────

    #[test]
    fn row_shows_title() {
        let r = ExtendedRenderer { width: 80 };
        let mut c = card("my-feature", "running");
        c.title = "My Feature".into();
        let row = r.render_card_row(&c);
        assert!(row.contains("My Feature"), "title missing in: {}", row);
    }

    #[test]
    fn row_shows_provider() {
        let r = ExtendedRenderer { width: 80 };
        let mut c = card("my-feature", "running");
        c.provider = Some("claude".into());
        let row = r.render_card_row(&c);
        assert!(row.contains("claude"), "provider missing in: {}", row);
    }

    #[test]
    fn row_shows_elapsed() {
        let r = ExtendedRenderer { width: 80 };
        let mut c = card("my-feature", "running");
        c.elapsed_s = Some(134);
        let row = r.render_card_row(&c);
        assert!(row.contains("2m14s"), "elapsed missing in: {}", row);
    }

    #[test]
    fn row_shows_elapsed_short() {
        let r = ExtendedRenderer { width: 80 };
        let mut c = card("quick-card", "done");
        c.elapsed_s = Some(43);
        let row = r.render_card_row(&c);
        assert!(row.contains("43s"), "short elapsed missing in: {}", row);
    }

    #[test]
    fn row_shows_progress() {
        let r = ExtendedRenderer { width: 80 };
        let mut c = card("my-feature", "running");
        c.progress = 67;
        let row = r.render_card_row(&c);
        assert!(row.contains("67%"), "progress missing in: {}", row);
    }

    #[test]
    fn row_hides_zero_progress() {
        let r = ExtendedRenderer { width: 80 };
        let c = card("pending-card", "pending");
        let row = r.render_card_row(&c);
        assert!(!row.contains('%'), "0% should be hidden in: {}", row);
    }

    #[test]
    fn row_shows_exit_code_on_failed() {
        let r = ExtendedRenderer { width: 80 };
        let mut c = card("broken", "failed");
        c.exit_code = Some(1);
        let row = r.render_card_row(&c);
        assert!(row.contains("exit 1"), "exit code missing in: {}", row);
    }

    #[test]
    fn row_hides_exit_code_on_non_failed() {
        let r = ExtendedRenderer { width: 80 };
        let mut c = card("done-card", "done");
        c.exit_code = Some(0);
        let row = r.render_card_row(&c);
        assert!(
            !row.contains("exit"),
            "exit code should be hidden for done: {}",
            row
        );
    }

    #[test]
    fn row_full_running_card() {
        let r = ExtendedRenderer { width: 80 };
        let mut c = card("my-feature", "running");
        c.title = "my-feature".into();
        c.provider = Some("claude".into());
        c.elapsed_s = Some(134);
        c.progress = 67;

        let row = r.render_card_row(&c);
        assert!(row.contains('▌'), "accent stripe missing");
        assert!(row.contains('▶'), "marker missing");
        assert!(row.contains("my-feature"), "title missing");
        assert!(row.contains("claude"), "provider missing");
        assert!(row.contains("2m14s"), "elapsed missing");
        assert!(row.contains("67%"), "progress missing");
        assert_has_ansi(&row);
    }

    // ── Progress bar ────────────────────────────────────────────────────

    #[test]
    fn progress_bar_uses_block_chars() {
        let r = ExtendedRenderer { width: 80 };
        let bar = r.render_progress(50, None, 0.0, None, None);
        assert!(bar.contains('█'), "expected █ in: {}", bar);
        assert!(bar.contains('░'), "expected ░ in: {}", bar);
    }

    #[test]
    fn progress_bar_16_cells() {
        let r = ExtendedRenderer { width: 80 };
        let bar = r.render_progress(50, None, 0.0, None, None);
        let filled = bar.matches('█').count();
        let empty = bar.matches('░').count();
        assert_eq!(
            filled + empty,
            16,
            "expected 16 cells, got {} filled + {} empty",
            filled,
            empty
        );
    }

    #[test]
    fn progress_bar_zero() {
        let r = ExtendedRenderer { width: 80 };
        let bar = r.render_progress(0, None, 0.0, None, None);
        let filled = bar.matches('█').count();
        let empty = bar.matches('░').count();
        assert_eq!(filled, 0, "0% should have 0 filled cells");
        assert_eq!(empty, 16, "0% should have 16 empty cells");
        assert!(bar.contains("0%"), "should show 0%");
    }

    #[test]
    fn progress_bar_full() {
        let r = ExtendedRenderer { width: 80 };
        let bar = r.render_progress(100, None, 0.0, None, None);
        let filled = bar.matches('█').count();
        let empty = bar.matches('░').count();
        assert_eq!(filled, 16, "100% should have 16 filled cells");
        assert_eq!(empty, 0, "100% should have 0 empty cells");
        assert!(bar.contains("100%"), "should show 100%");
    }

    #[test]
    fn progress_has_left_border() {
        let r = ExtendedRenderer { width: 80 };
        let bar = r.render_progress(50, None, 0.0, None, None);
        assert!(bar.starts_with('│'), "expected │ left border in: {}", bar);
    }

    #[test]
    fn progress_with_phase_and_moon_glyph() {
        let r = ExtendedRenderer { width: 80 };
        let bar = r.render_progress(67, Some("Phase 2: Network"), 0.5, None, None);
        assert!(bar.contains("67%"), "pct missing in: {}", bar);
        assert!(
            bar.contains("Phase 2: Network"),
            "phase missing in: {}",
            bar
        );
        // frac 0.5 → ◕ (50–75% bracket)
        assert!(
            bar.contains('◕'),
            "expected ◕ moon glyph for 0.5 frac in: {}",
            bar
        );
    }

    #[test]
    fn progress_without_phase() {
        let r = ExtendedRenderer { width: 80 };
        let bar = r.render_progress(33, None, 0.0, None, None);
        assert!(bar.contains("33%"), "pct missing in: {}", bar);
        // No moon glyph when there's no phase.
        assert!(
            !bar.contains('◔') && !bar.contains('◑') && !bar.contains('◕') && !bar.contains('●'),
            "moon glyph should not appear without phase in: {}",
            bar
        );
    }

    // ── Moon glyph ──────────────────────────────────────────────────────

    #[test]
    fn moon_glyph_quarter() {
        assert_eq!(moon_glyph(0.0), '◔');
        assert_eq!(moon_glyph(0.24), '◔');
    }

    #[test]
    fn moon_glyph_half() {
        assert_eq!(moon_glyph(0.25), '◑');
        assert_eq!(moon_glyph(0.49), '◑');
    }

    #[test]
    fn moon_glyph_three_quarter() {
        assert_eq!(moon_glyph(0.50), '◕');
        assert_eq!(moon_glyph(0.74), '◕');
    }

    #[test]
    fn moon_glyph_full() {
        assert_eq!(moon_glyph(0.75), '●');
        assert_eq!(moon_glyph(1.0), '●');
    }

    // ── Footer ──────────────────────────────────────────────────────────

    #[test]
    fn footer_all_stats() {
        let r = ExtendedRenderer { width: 80 };
        let stats = Stats {
            total: 14,
            by_state: HashMap::new(),
            success_rate_pct: Some(92.0),
            avg_duration_s: Some(224.0),
        };
        let footer = r.render_footer(&stats);
        assert!(footer.contains("14 total"), "total missing in: {}", footer);
        assert!(
            footer.contains("92% success"),
            "success rate missing in: {}",
            footer
        );
        assert!(
            footer.contains("avg 3m44s"),
            "avg duration missing in: {}",
            footer
        );
    }

    #[test]
    fn footer_no_optional_stats() {
        let r = ExtendedRenderer { width: 80 };
        let stats = Stats {
            total: 3,
            by_state: HashMap::new(),
            success_rate_pct: None,
            avg_duration_s: None,
        };
        let footer = r.render_footer(&stats);
        assert!(footer.contains("3 total"));
    }

    #[test]
    fn footer_only_success_rate() {
        let r = ExtendedRenderer { width: 80 };
        let stats = Stats {
            total: 10,
            by_state: HashMap::new(),
            success_rate_pct: Some(80.0),
            avg_duration_s: None,
        };
        let footer = r.render_footer(&stats);
        assert!(footer.contains("10 total"));
        assert!(footer.contains("80% success"));
        assert!(!footer.contains("avg"));
    }

    #[test]
    fn footer_has_box_drawing() {
        let r = ExtendedRenderer { width: 80 };
        let stats = Stats {
            total: 5,
            by_state: HashMap::new(),
            success_rate_pct: Some(100.0),
            avg_duration_s: Some(60.0),
        };
        let footer = r.render_footer(&stats);
        assert!(footer.contains('└'), "expected └ in footer: {}", footer);
        assert!(footer.contains('┘'), "expected ┘ in footer: {}", footer);
        assert!(footer.contains('─'), "expected ─ in footer: {}", footer);
        assert_has_box_chars(&footer);
    }

    #[test]
    fn footer_pipe_separator() {
        let r = ExtendedRenderer { width: 80 };
        let stats = Stats {
            total: 10,
            by_state: HashMap::new(),
            success_rate_pct: Some(80.0),
            avg_duration_s: Some(120.0),
        };
        let footer = r.render_footer(&stats);
        assert!(
            footer.contains('│'),
            "expected │ separator in footer: {}",
            footer
        );
    }

    #[test]
    fn footer_has_ansi() {
        let r = ExtendedRenderer { width: 80 };
        let stats = Stats {
            total: 5,
            by_state: HashMap::new(),
            success_rate_pct: None,
            avg_duration_s: None,
        };
        let footer = r.render_footer(&stats);
        assert_has_ansi(&footer);
    }

    // ── Duration formatting ─────────────────────────────────────────────

    #[test]
    fn duration_seconds_only() {
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(59), "59s");
    }

    #[test]
    fn duration_minutes_and_seconds() {
        assert_eq!(format_duration(60), "1m00s");
        assert_eq!(format_duration(134), "2m14s");
        assert_eq!(format_duration(3600), "60m00s");
    }

    // ── Integration: full render through render_board ────────────────────

    #[test]
    fn full_board_has_box_drawing_and_accents() {
        use crate::termcaps::TermCaps;

        let caps = TermCaps {
            level: crate::termcaps::TermLevel::Extended,
            width: 80,
            two_column: false,
        };

        let mut running = card("my-feature", "running");
        running.provider = Some("claude".into());
        running.elapsed_s = Some(134);
        running.progress = 67;

        let pending = card("docs-update", "pending");

        let mut failed = card("broken-card", "failed");
        failed.exit_code = Some(1);

        let views = vec![
            ("running".into(), vec![running]),
            ("pending".into(), vec![pending]),
            ("failed".into(), vec![failed]),
        ];

        let stats = Stats {
            total: 3,
            by_state: HashMap::new(),
            success_rate_pct: Some(66.0),
            avg_duration_s: Some(134.0),
        };

        let output = super::super::render_board(&caps, &views, &stats);

        // Box drawing characters present.
        assert!(output.contains('┌'), "expected ┌ in board");
        assert!(output.contains('┐'), "expected ┐ in board");
        assert!(output.contains('└'), "expected └ in board");
        assert!(output.contains('┘'), "expected ┘ in board");
        assert!(output.contains('─'), "expected ─ in board");

        // Accent stripe present.
        assert!(output.contains('▌'), "expected ▌ accent stripe in board");

        // Progress bar chars present (running card has progress=67).
        assert!(output.contains('█'), "expected █ progress fill in board");

        // State sections.
        assert!(output.contains("RUNNING"));
        assert!(output.contains("PENDING"));
        assert!(output.contains("FAILED"));

        // Stats.
        assert!(output.contains("3 total"));

        // ANSI codes.
        assert_has_ansi(&output);
    }
}
