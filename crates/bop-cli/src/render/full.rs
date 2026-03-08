/// Level 3 — Full renderer (256-color, demoscene block headers).
///
/// Section headers use `░▒▓ LABEL ▓▒░` dithered fade filled to terminal width.
/// Left-accent `▌` in state color; dice glyphs ⚀⚁⚂⚃⚄⚅ encode priority P1–P6.
/// Progress bar: 16-cell `████░░░░` (U+2588 filled, U+2591 empty).
/// Phase name + moon quarter glyph. Footer with same dithered pattern.
use crate::acplan::half_circle_glyph;
use crate::colors::{state_ansi, BOLD, DIM, RESET};

use super::{CardRenderer, CardView, Stats};

pub struct FullRenderer {
    pub width: u16,
}

/// Dice glyph for priority 1–6: ⚀⚁⚂⚃⚄⚅.
///
/// Returns `None` for out-of-range or unset priority.
fn dice_glyph(priority: Option<i64>) -> Option<char> {
    match priority {
        Some(1) => Some('⚀'),
        Some(2) => Some('⚁'),
        Some(3) => Some('⚂'),
        Some(4) => Some('⚃'),
        Some(5) => Some('⚄'),
        Some(6) => Some('⚅'),
        _ => None,
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

/// Build a dithered header/footer line: `░▒▓ LABEL ▓▒░` + `░` fill to width.
fn dithered_line(label: &str, width: usize) -> String {
    let prefix = format!("░▒▓ {} ▓▒░", label);
    let prefix_len = prefix.chars().count();
    let fill_len = width.saturating_sub(prefix_len);
    let fill = "░".repeat(fill_len);
    format!("{}{}", prefix, fill)
}

impl CardRenderer for FullRenderer {
    fn render_section_header(&self, label: &str, count: usize) -> String {
        let color = state_ansi(label);
        let title = format!("{} ({})", label.to_uppercase(), count);
        let w = self.width as usize;
        let header = dithered_line(&title, w);
        format!("{}{}{}{}", BOLD, color, header, RESET)
    }

    fn render_card_row(&self, card: &CardView) -> String {
        let color = state_ansi(&card.state);

        // Left accent stripe ▌ in state color (256-color).
        let accent = format!("{}▌{}", color, RESET);

        let marker = match card.state.as_str() {
            "running" => format!("{}▶{}", color, RESET),
            "done" | "merged" => format!("{}✓{}", color, RESET),
            "failed" => format!("{}✗{}", color, RESET),
            _ => "·".to_string(),
        };

        let mut parts = vec![format!("{} {} {}", accent, marker, card.title)];

        // Dice glyph for priority.
        if let Some(die) = dice_glyph(card.priority) {
            parts.push(format!("{}{}{}", color, die, RESET));
        }

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

        format!("▌   {}{}{}", bar, count_str, phase_str)
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
        let w = self.width as usize;
        let footer = dithered_line(&content, w);
        format!("{}{}{}", DIM, footer, RESET)
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

    // ── Demoscene header pattern ────────────────────────────────────────

    #[test]
    fn header_has_dithered_pattern() {
        let r = FullRenderer { width: 80 };
        let h = r.render_section_header("running", 2);
        assert!(h.contains("░▒▓"), "expected ░▒▓ dither prefix in: {}", h);
        assert!(h.contains("▓▒░"), "expected ▓▒░ dither suffix in: {}", h);
    }

    #[test]
    fn header_uppercases_label() {
        let r = FullRenderer { width: 80 };
        let h = r.render_section_header("pending", 0);
        assert!(
            h.contains("PENDING"),
            "expected uppercase PENDING in: {}",
            h
        );
    }

    #[test]
    fn header_includes_count() {
        let r = FullRenderer { width: 80 };
        let h = r.render_section_header("done", 5);
        assert!(h.contains("(5)"), "expected count (5) in: {}", h);
    }

    #[test]
    fn header_zero_count() {
        let r = FullRenderer { width: 80 };
        let h = r.render_section_header("done", 0);
        assert!(h.contains("DONE"), "expected DONE in: {}", h);
        assert!(h.contains("(0)"), "expected (0) in: {}", h);
    }

    #[test]
    fn header_has_256_color_ansi() {
        let r = FullRenderer { width: 80 };
        let h = r.render_section_header("running", 1);
        // 256-color codes use \x1b[38;5;NNNm
        assert!(
            h.contains("\x1b[38;5;"),
            "expected 256-color ANSI code in: {}",
            h
        );
    }

    #[test]
    fn header_colored_by_state() {
        let r = FullRenderer { width: 80 };

        // Running uses amber (\x1b[38;5;172m)
        let h = r.render_section_header("running", 1);
        assert!(
            h.contains("\x1b[38;5;172m"),
            "expected amber for running: {}",
            h
        );

        // Failed uses red (\x1b[38;5;160m)
        let h = r.render_section_header("failed", 1);
        assert!(
            h.contains("\x1b[38;5;160m"),
            "expected red for failed: {}",
            h
        );

        // Done uses green (\x1b[38;5;71m)
        let h = r.render_section_header("done", 1);
        assert!(
            h.contains("\x1b[38;5;71m"),
            "expected green for done: {}",
            h
        );
    }

    #[test]
    fn header_fills_to_width_with_dither() {
        let r = FullRenderer { width: 80 };
        let h = r.render_section_header("running", 1);
        // The fill portion uses ░ characters after the ▓▒░ suffix.
        // Count all ░ characters — should be substantial for width=80.
        let dither_count = h.matches('░').count();
        assert!(
            dither_count > 10,
            "expected significant ░ fill, got {} in: {}",
            dither_count,
            h
        );
    }

    #[test]
    fn header_fill_adapts_to_width() {
        let narrow = FullRenderer { width: 40 };
        let wide = FullRenderer { width: 120 };
        let h_narrow = narrow.render_section_header("running", 1);
        let h_wide = wide.render_section_header("running", 1);
        let count_narrow = h_narrow.matches('░').count();
        let count_wide = h_wide.matches('░').count();
        assert!(
            count_wide > count_narrow,
            "wide ({}) should have more ░ fill than narrow ({})",
            count_wide,
            count_narrow
        );
    }

    #[test]
    fn header_has_bold() {
        let r = FullRenderer { width: 80 };
        let h = r.render_section_header("running", 1);
        assert!(h.contains(BOLD), "expected BOLD in header");
        assert!(h.contains(RESET), "expected RESET in header");
    }

    // ── Dice glyphs ─────────────────────────────────────────────────────

    #[test]
    fn dice_glyph_p1_urgent() {
        assert_eq!(dice_glyph(Some(1)), Some('⚀'));
    }

    #[test]
    fn dice_glyph_p2_high() {
        assert_eq!(dice_glyph(Some(2)), Some('⚁'));
    }

    #[test]
    fn dice_glyph_p3_normal() {
        assert_eq!(dice_glyph(Some(3)), Some('⚂'));
    }

    #[test]
    fn dice_glyph_p4_low() {
        assert_eq!(dice_glyph(Some(4)), Some('⚃'));
    }

    #[test]
    fn dice_glyph_p5() {
        assert_eq!(dice_glyph(Some(5)), Some('⚄'));
    }

    #[test]
    fn dice_glyph_p6() {
        assert_eq!(dice_glyph(Some(6)), Some('⚅'));
    }

    #[test]
    fn dice_glyph_none_for_unset() {
        assert_eq!(dice_glyph(None), None);
    }

    #[test]
    fn dice_glyph_none_for_out_of_range() {
        assert_eq!(dice_glyph(Some(0)), None);
        assert_eq!(dice_glyph(Some(7)), None);
        assert_eq!(dice_glyph(Some(-1)), None);
    }

    // ── Card row ────────────────────────────────────────────────────────

    #[test]
    fn row_has_accent_stripe() {
        let r = FullRenderer { width: 80 };
        let c = card("test-card", "running");
        let row = r.render_card_row(&c);
        assert!(row.contains('▌'), "expected accent stripe ▌ in: {}", row);
    }

    #[test]
    fn row_accent_stripe_colored_256() {
        let r = FullRenderer { width: 80 };
        let c = card("test", "running");
        let row = r.render_card_row(&c);
        assert!(
            row.contains("\x1b[38;5;172m▌"),
            "accent stripe should be 256-color amber: {}",
            row
        );
    }

    #[test]
    fn row_running_marker() {
        let r = FullRenderer { width: 80 };
        let c = card("my-feature", "running");
        let row = r.render_card_row(&c);
        assert!(row.contains('▶'), "expected ▶ marker in: {}", row);
    }

    #[test]
    fn row_pending_marker() {
        let r = FullRenderer { width: 80 };
        let c = card("docs-update", "pending");
        let row = r.render_card_row(&c);
        assert!(row.contains('·'), "expected · marker in: {}", row);
    }

    #[test]
    fn row_done_marker() {
        let r = FullRenderer { width: 80 };
        let c = card("perf-improvements", "done");
        let row = r.render_card_row(&c);
        assert!(row.contains('✓'), "expected ✓ marker in: {}", row);
    }

    #[test]
    fn row_failed_marker() {
        let r = FullRenderer { width: 80 };
        let c = card("broken-card", "failed");
        let row = r.render_card_row(&c);
        assert!(row.contains('✗'), "expected ✗ marker in: {}", row);
    }

    #[test]
    fn row_shows_dice_for_priority() {
        let r = FullRenderer { width: 80 };
        let mut c = card("urgent-task", "running");
        c.priority = Some(1);
        let row = r.render_card_row(&c);
        assert!(row.contains('⚀'), "expected ⚀ dice glyph in: {}", row);
    }

    #[test]
    fn row_no_dice_without_priority() {
        let r = FullRenderer { width: 80 };
        let c = card("no-priority", "pending");
        let row = r.render_card_row(&c);
        assert!(
            !row.contains('⚀')
                && !row.contains('⚁')
                && !row.contains('⚂')
                && !row.contains('⚃')
                && !row.contains('⚄')
                && !row.contains('⚅'),
            "dice glyph should not appear without priority in: {}",
            row
        );
    }

    #[test]
    fn row_shows_title() {
        let r = FullRenderer { width: 80 };
        let mut c = card("my-feature", "running");
        c.title = "My Feature".into();
        let row = r.render_card_row(&c);
        assert!(row.contains("My Feature"), "title missing in: {}", row);
    }

    #[test]
    fn row_shows_provider() {
        let r = FullRenderer { width: 80 };
        let mut c = card("my-feature", "running");
        c.provider = Some("claude".into());
        let row = r.render_card_row(&c);
        assert!(row.contains("claude"), "provider missing in: {}", row);
    }

    #[test]
    fn row_shows_elapsed() {
        let r = FullRenderer { width: 80 };
        let mut c = card("my-feature", "running");
        c.elapsed_s = Some(134);
        let row = r.render_card_row(&c);
        assert!(row.contains("2m14s"), "elapsed missing in: {}", row);
    }

    #[test]
    fn row_shows_progress() {
        let r = FullRenderer { width: 80 };
        let mut c = card("my-feature", "running");
        c.progress = 67;
        let row = r.render_card_row(&c);
        assert!(row.contains("67%"), "progress missing in: {}", row);
    }

    #[test]
    fn row_hides_zero_progress() {
        let r = FullRenderer { width: 80 };
        let c = card("pending-card", "pending");
        let row = r.render_card_row(&c);
        assert!(!row.contains('%'), "0% should be hidden in: {}", row);
    }

    #[test]
    fn row_shows_exit_code_on_failed() {
        let r = FullRenderer { width: 80 };
        let mut c = card("broken", "failed");
        c.exit_code = Some(1);
        let row = r.render_card_row(&c);
        assert!(row.contains("exit 1"), "exit code missing in: {}", row);
    }

    #[test]
    fn row_hides_exit_code_on_non_failed() {
        let r = FullRenderer { width: 80 };
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
    fn row_has_ansi() {
        let r = FullRenderer { width: 80 };
        let c = card("test-card", "running");
        let row = r.render_card_row(&c);
        assert_has_ansi(&row);
    }

    #[test]
    fn row_full_running_card() {
        let r = FullRenderer { width: 80 };
        let mut c = card("my-feature", "running");
        c.title = "my-feature".into();
        c.provider = Some("claude".into());
        c.elapsed_s = Some(134);
        c.progress = 67;
        c.priority = Some(2);

        let row = r.render_card_row(&c);
        assert!(row.contains('▌'), "accent stripe missing");
        assert!(row.contains('▶'), "marker missing");
        assert!(row.contains("my-feature"), "title missing");
        assert!(row.contains('⚁'), "dice glyph missing");
        assert!(row.contains("claude"), "provider missing");
        assert!(row.contains("2m14s"), "elapsed missing");
        assert!(row.contains("67%"), "progress missing");
        assert_has_ansi(&row);
    }

    // ── Progress bar ────────────────────────────────────────────────────

    #[test]
    fn progress_bar_uses_block_chars() {
        let r = FullRenderer { width: 80 };
        let bar = r.render_progress(50, None, 0.0, None, None);
        assert!(bar.contains('█'), "expected █ in: {}", bar);
        assert!(bar.contains('░'), "expected ░ in: {}", bar);
    }

    #[test]
    fn progress_bar_16_cells() {
        let r = FullRenderer { width: 80 };
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
        let r = FullRenderer { width: 80 };
        let bar = r.render_progress(0, None, 0.0, None, None);
        let filled = bar.matches('█').count();
        let empty = bar.matches('░').count();
        assert_eq!(filled, 0, "0% should have 0 filled cells");
        assert_eq!(empty, 16, "0% should have 16 empty cells");
        assert!(bar.contains("0%"), "should show 0%");
    }

    #[test]
    fn progress_bar_full() {
        let r = FullRenderer { width: 80 };
        let bar = r.render_progress(100, None, 0.0, None, None);
        let filled = bar.matches('█').count();
        let empty = bar.matches('░').count();
        assert_eq!(filled, 16, "100% should have 16 filled cells");
        assert_eq!(empty, 0, "100% should have 0 empty cells");
        assert!(bar.contains("100%"), "should show 100%");
    }

    #[test]
    fn progress_has_accent_left_border() {
        let r = FullRenderer { width: 80 };
        let bar = r.render_progress(50, None, 0.0, None, None);
        assert!(
            bar.starts_with('▌'),
            "expected ▌ left accent border in: {}",
            bar
        );
    }

    #[test]
    fn progress_with_phase_and_moon_glyph() {
        let r = FullRenderer { width: 80 };
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
        let r = FullRenderer { width: 80 };
        let bar = r.render_progress(33, None, 0.0, None, None);
        assert!(bar.contains("33%"), "pct missing in: {}", bar);
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
    fn footer_has_dithered_pattern() {
        let r = FullRenderer { width: 80 };
        let stats = Stats {
            total: 5,
            by_state: HashMap::new(),
            success_rate_pct: None,
            avg_duration_s: None,
        };
        let footer = r.render_footer(&stats);
        assert!(
            footer.contains("░▒▓"),
            "expected ░▒▓ dither prefix in footer: {}",
            footer
        );
        assert!(
            footer.contains("▓▒░"),
            "expected ▓▒░ dither suffix in footer: {}",
            footer
        );
    }

    #[test]
    fn footer_all_stats() {
        let r = FullRenderer { width: 80 };
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
        let r = FullRenderer { width: 80 };
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
        let r = FullRenderer { width: 80 };
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
    fn footer_pipe_separator() {
        let r = FullRenderer { width: 80 };
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
        let r = FullRenderer { width: 80 };
        let stats = Stats {
            total: 5,
            by_state: HashMap::new(),
            success_rate_pct: None,
            avg_duration_s: None,
        };
        let footer = r.render_footer(&stats);
        assert_has_ansi(&footer);
    }

    #[test]
    fn footer_fills_to_width() {
        let r = FullRenderer { width: 80 };
        let stats = Stats {
            total: 5,
            by_state: HashMap::new(),
            success_rate_pct: None,
            avg_duration_s: None,
        };
        let footer = r.render_footer(&stats);
        // Should have ░ fill chars after the ▓▒░ suffix
        let dither_count = footer.matches('░').count();
        assert!(
            dither_count > 10,
            "expected significant ░ fill in footer, got {} in: {}",
            dither_count,
            footer
        );
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
    fn full_board_has_demoscene_headers_and_accents() {
        use crate::termcaps::TermCaps;

        let caps = TermCaps {
            level: crate::termcaps::TermLevel::Full,
            width: 80,
            two_column: false,
        };

        let mut running = card("my-feature", "running");
        running.provider = Some("claude".into());
        running.elapsed_s = Some(134);
        running.progress = 67;
        running.priority = Some(1);

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

        // Demoscene dithered headers.
        assert!(output.contains("░▒▓"), "expected ░▒▓ dither in board");
        assert!(output.contains("▓▒░"), "expected ▓▒░ dither in board");

        // Accent stripe present.
        assert!(output.contains('▌'), "expected ▌ accent stripe in board");

        // Dice glyph for priority-1 card.
        assert!(output.contains('⚀'), "expected ⚀ dice glyph in board");

        // Progress bar chars present (running card has progress=67).
        assert!(output.contains('█'), "expected █ progress fill in board");

        // State sections.
        assert!(output.contains("RUNNING"));
        assert!(output.contains("PENDING"));
        assert!(output.contains("FAILED"));

        // Stats.
        assert!(output.contains("3 total"));

        // 256-color ANSI codes.
        assert!(
            output.contains("\x1b[38;5;"),
            "expected 256-color ANSI codes in board"
        );
        assert_has_ansi(&output);
    }
}
