/// Level 4 — TrueColor renderer (24-bit RGB, double-line box, column layout).
///
/// Uses `╔═╗║╚═╝` double-line borders. Two-column layout at width >= 100
/// (headers and boxes sized to column width). Progress bar with per-cell RGB
/// gradient (amber #B8690F → green #1E8A45 as percentage rises).
/// Card separators within section boxes use rounded single-line (╭─╮│╰╯).
/// All glyphs BMP (U+0000–U+FFFF).
use crate::acplan::half_circle_glyph;
use crate::colors::{BOLD, DIM, RESET};

use super::{CardRenderer, CardView, Stats};

pub struct TrueColorRenderer {
    pub width: u16,
    pub two_column: bool,
}

// ── RGB color helpers ────────────────────────────────────────────────────────

/// 24-bit RGB foreground escape sequence for a given card state.
///
/// Colors match the hex values in `colors::state_color()`.
fn state_rgb(state: &str) -> String {
    let (r, g, b) = match state {
        "pending" => (0x3A, 0x5A, 0x8A),
        "running" => (0xB8, 0x69, 0x0F),
        "done" => (0x1E, 0x8A, 0x45),
        "failed" => (0xC4, 0x30, 0x30),
        "merged" => (0x6B, 0x3D, 0xB8),
        _ => (0x55, 0x55, 0x55),
    };
    format!("\x1b[38;2;{};{};{}m", r, g, b)
}

/// Linear interpolation between two `u8` values at fraction `t` (clamped 0–1).
fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    let t = t.clamp(0.0, 1.0);
    (a as f32 + (b as f32 - a as f32) * t) as u8
}

/// 24-bit RGB foreground escape for a single progress-bar cell at position
/// `t` (0.0 = amber, 1.0 = green).
fn gradient_fg(t: f32) -> String {
    // Amber #B8690F → Green #1E8A45.
    let r = lerp_u8(0xB8, 0x1E, t);
    let g = lerp_u8(0x69, 0x8A, t);
    let b = lerp_u8(0x0F, 0x45, t);
    format!("\x1b[38;2;{};{};{}m", r, g, b)
}

// ── Glyph helpers ────────────────────────────────────────────────────────────

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

// ── Layout helpers ───────────────────────────────────────────────────────────

/// Effective column width: full terminal width in single-column mode,
/// half width in two-column mode.
fn col_width(width: u16, two_column: bool) -> usize {
    let w = width as usize;
    if two_column {
        w / 2
    } else {
        w
    }
}

impl TrueColorRenderer {
    /// Inner width available for rounded card-separator lines.
    ///
    /// Accounts for `║ ╭` prefix (4 columns) and `╮` suffix (1 column).
    fn inner_width(&self) -> usize {
        col_width(self.width, self.two_column).saturating_sub(5)
    }
}

impl CardRenderer for TrueColorRenderer {
    fn render_section_header(&self, label: &str, count: usize) -> String {
        let color = state_rgb(label);
        let title = format!(" {} ({}) ", label.to_uppercase(), count);
        let w = col_width(self.width, self.two_column);
        let inner = w.saturating_sub(2); // ╔ and ╗
        let fill_len = inner.saturating_sub(title.len());
        let fill = "═".repeat(fill_len);
        format!("{}{}╔{}{}╗{}", BOLD, color, title, fill, RESET)
    }

    fn render_card_row(&self, card: &CardView) -> String {
        let color = state_rgb(&card.state);
        let iw = self.inner_width();

        let marker = match card.state.as_str() {
            "running" => format!("{}▶{}", color, RESET),
            "done" | "merged" => format!("{}✓{}", color, RESET),
            "failed" => format!("{}✗{}", color, RESET),
            _ => "·".to_string(),
        };

        let mut parts = vec![format!("{} {}", marker, card.title)];

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

        let content = parts.join("  ");
        let has_progress = card.progress > 0 || card.phase_name.is_some();

        // Rounded card separator top + content line.
        let top = format!("║ ╭{}╮", "─".repeat(iw));
        let mid = format!("║ │ {}", content);

        if has_progress {
            // Leave rounded box open — progress bar will close it.
            format!("{}\n{}", top, mid)
        } else {
            // Close rounded box immediately.
            let bot = format!("║ ╰{}╯", "─".repeat(iw));
            format!("{}\n{}\n{}", top, mid, bot)
        }
    }

    fn render_progress(
        &self,
        pct: u8,
        phase: Option<&str>,
        phase_frac: f32,
        ac_done: Option<usize>,
        ac_total: Option<usize>,
    ) -> String {
        let bar_width: usize = 16;
        let iw = self.inner_width();
        let filled = (pct as usize * bar_width) / 100;
        let empty = bar_width - filled;

        // RGB gradient bar: each filled cell gets a per-cell color
        // interpolated from amber (#B8690F) → green (#1E8A45).
        let mut bar = String::new();
        for i in 0..filled {
            let t = if bar_width <= 1 {
                0.0
            } else {
                i as f32 / (bar_width - 1) as f32
            };
            bar.push_str(&gradient_fg(t));
            bar.push('█');
        }
        if filled > 0 {
            bar.push_str(RESET);
        }
        bar.push_str(&format!("{}{}{}", DIM, "░".repeat(empty), RESET));

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

        let progress_line = format!("║ │   {}{}{}", bar, count_str, phase_str);
        let bottom = format!("║ ╰{}╯", "─".repeat(iw));
        format!("{}\n{}", progress_line, bottom)
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
        let w = col_width(self.width, self.two_column);
        let inner = w.saturating_sub(2); // ╚ and ╝
                                         // "═ " prefix (2) + content + " " suffix (1) = 3 extra chars.
        let content_len = content.chars().count() + 3;
        let fill_len = inner.saturating_sub(content_len);
        let fill = "═".repeat(fill_len);
        format!("{}╚═ {} {}╝{}", DIM, content, fill, RESET)
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

    /// Assert the string contains 24-bit RGB escape sequences.
    fn assert_has_rgb(s: &str) {
        assert!(
            s.contains("\x1b[38;2;"),
            "expected 24-bit RGB escape \\x1b[38;2; in: {}",
            s
        );
    }

    /// Assert the string contains double-line box-drawing characters.
    fn assert_has_double_box(s: &str) {
        assert!(
            s.contains('╔')
                || s.contains('═')
                || s.contains('╗')
                || s.contains('║')
                || s.contains('╚')
                || s.contains('╝'),
            "expected double-line box chars (╔═╗║╚╝) in: {}",
            s
        );
    }

    // ── Section header ──────────────────────────────────────────────────

    #[test]
    fn header_has_double_box_chars() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let h = r.render_section_header("running", 2);
        assert!(h.contains('╔'), "expected ╔ in: {}", h);
        assert!(h.contains('╗'), "expected ╗ in: {}", h);
        assert!(h.contains('═'), "expected ═ in: {}", h);
    }

    #[test]
    fn header_has_rgb_escape() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let h = r.render_section_header("running", 1);
        assert_has_rgb(&h);
    }

    #[test]
    fn header_uppercases_label() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let h = r.render_section_header("pending", 0);
        assert!(
            h.contains("PENDING"),
            "expected uppercase PENDING in: {}",
            h
        );
    }

    #[test]
    fn header_includes_count() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let h = r.render_section_header("done", 5);
        assert!(h.contains("(5)"), "expected count (5) in: {}", h);
    }

    #[test]
    fn header_zero_count() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let h = r.render_section_header("done", 0);
        assert!(h.contains("DONE"), "expected DONE in: {}", h);
        assert!(h.contains("(0)"), "expected (0) in: {}", h);
    }

    #[test]
    fn header_colored_by_state_rgb() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };

        // Running uses amber RGB (184, 105, 15).
        let h = r.render_section_header("running", 1);
        assert!(
            h.contains("\x1b[38;2;184;105;15m"),
            "expected amber RGB for running: {}",
            h
        );

        // Failed uses red RGB (196, 48, 48).
        let h = r.render_section_header("failed", 1);
        assert!(
            h.contains("\x1b[38;2;196;48;48m"),
            "expected red RGB for failed: {}",
            h
        );

        // Done uses green RGB (30, 138, 69).
        let h = r.render_section_header("done", 1);
        assert!(
            h.contains("\x1b[38;2;30;138;69m"),
            "expected green RGB for done: {}",
            h
        );
    }

    #[test]
    fn header_fill_adapts_to_width() {
        let narrow = TrueColorRenderer {
            width: 40,
            two_column: false,
        };
        let wide = TrueColorRenderer {
            width: 120,
            two_column: false,
        };
        let h_narrow = narrow.render_section_header("running", 1);
        let h_wide = wide.render_section_header("running", 1);
        let count_narrow = h_narrow.matches('═').count();
        let count_wide = h_wide.matches('═').count();
        assert!(
            count_wide > count_narrow,
            "wide ({}) should have more ═ fill than narrow ({})",
            count_wide,
            count_narrow
        );
    }

    #[test]
    fn header_has_bold() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let h = r.render_section_header("running", 1);
        assert!(h.contains(BOLD), "expected BOLD in header");
        assert!(h.contains(RESET), "expected RESET in header");
    }

    // ── Two-column activation logic ─────────────────────────────────────

    #[test]
    fn two_column_halves_header_width() {
        let single = TrueColorRenderer {
            width: 120,
            two_column: false,
        };
        let dual = TrueColorRenderer {
            width: 120,
            two_column: true,
        };
        let h_single = single.render_section_header("running", 1);
        let h_dual = dual.render_section_header("running", 1);
        let fill_single = h_single.matches('═').count();
        let fill_dual = h_dual.matches('═').count();
        assert!(
            fill_single > fill_dual,
            "two-column header ({}) should have less ═ fill than single-column ({})",
            fill_dual,
            fill_single
        );
    }

    #[test]
    fn two_column_col_width_at_120() {
        assert_eq!(col_width(120, true), 60);
        assert_eq!(col_width(120, false), 120);
    }

    #[test]
    fn two_column_threshold_at_100() {
        // Width 100 → two_column feasible.
        assert_eq!(col_width(100, true), 50);
        // Width 99 → single-column (two_column would be false from TermCaps).
        assert_eq!(col_width(99, false), 99);
    }

    #[test]
    fn two_column_halves_inner_width() {
        let single = TrueColorRenderer {
            width: 120,
            two_column: false,
        };
        let dual = TrueColorRenderer {
            width: 120,
            two_column: true,
        };
        assert!(
            single.inner_width() > dual.inner_width(),
            "single inner ({}) should be wider than dual inner ({})",
            single.inner_width(),
            dual.inner_width()
        );
    }

    #[test]
    fn two_column_halves_footer_width() {
        let single = TrueColorRenderer {
            width: 120,
            two_column: false,
        };
        let dual = TrueColorRenderer {
            width: 120,
            two_column: true,
        };
        let stats = Stats {
            total: 5,
            by_state: HashMap::new(),
            success_rate_pct: None,
            avg_duration_s: None,
        };
        let f_single = single.render_footer(&stats);
        let f_dual = dual.render_footer(&stats);
        let fill_single = f_single.matches('═').count();
        let fill_dual = f_dual.matches('═').count();
        assert!(
            fill_single > fill_dual,
            "two-column footer ({}) should have less ═ fill than single-column ({})",
            fill_dual,
            fill_single
        );
    }

    // ── Dice glyphs ─────────────────────────────────────────────────────

    #[test]
    fn dice_glyph_p1_through_p6() {
        assert_eq!(dice_glyph(Some(1)), Some('⚀'));
        assert_eq!(dice_glyph(Some(2)), Some('⚁'));
        assert_eq!(dice_glyph(Some(3)), Some('⚂'));
        assert_eq!(dice_glyph(Some(4)), Some('⚃'));
        assert_eq!(dice_glyph(Some(5)), Some('⚄'));
        assert_eq!(dice_glyph(Some(6)), Some('⚅'));
    }

    #[test]
    fn dice_glyph_none_for_unset_or_out_of_range() {
        assert_eq!(dice_glyph(None), None);
        assert_eq!(dice_glyph(Some(0)), None);
        assert_eq!(dice_glyph(Some(7)), None);
        assert_eq!(dice_glyph(Some(-1)), None);
    }

    // ── Card row ────────────────────────────────────────────────────────

    #[test]
    fn row_has_double_line_border() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let c = card("test-card", "running");
        let row = r.render_card_row(&c);
        assert!(
            row.contains('║'),
            "expected ║ double-line border in: {}",
            row
        );
    }

    #[test]
    fn row_has_rounded_card_separator() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let c = card("test-card", "running");
        let row = r.render_card_row(&c);
        assert!(
            row.contains('╭') && row.contains('╮'),
            "expected ╭╮ rounded separator in: {}",
            row
        );
    }

    #[test]
    fn row_has_rounded_bottom_when_no_progress() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let c = card("no-progress", "pending");
        let row = r.render_card_row(&c);
        assert!(
            row.contains('╰') && row.contains('╯'),
            "expected ╰╯ rounded bottom for card without progress in: {}",
            row
        );
    }

    #[test]
    fn row_no_rounded_bottom_when_progress() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let mut c = card("with-progress", "running");
        c.progress = 50;
        let row = r.render_card_row(&c);
        assert!(
            !row.contains('╰') && !row.contains('╯'),
            "should NOT have ╰╯ bottom when progress follows: {}",
            row
        );
    }

    #[test]
    fn row_has_rgb_escape() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let c = card("test-card", "running");
        let row = r.render_card_row(&c);
        assert_has_rgb(&row);
    }

    #[test]
    fn row_running_marker() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let c = card("my-feature", "running");
        let row = r.render_card_row(&c);
        assert!(row.contains('▶'), "expected ▶ marker in: {}", row);
    }

    #[test]
    fn row_pending_marker() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let c = card("docs-update", "pending");
        let row = r.render_card_row(&c);
        assert!(row.contains('·'), "expected · marker in: {}", row);
    }

    #[test]
    fn row_done_marker() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let c = card("perf-improvements", "done");
        let row = r.render_card_row(&c);
        assert!(row.contains('✓'), "expected ✓ marker in: {}", row);
    }

    #[test]
    fn row_merged_marker() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let c = card("merged-card", "merged");
        let row = r.render_card_row(&c);
        assert!(
            row.contains('✓'),
            "expected ✓ marker for merged in: {}",
            row
        );
    }

    #[test]
    fn row_failed_marker() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let c = card("broken-card", "failed");
        let row = r.render_card_row(&c);
        assert!(row.contains('✗'), "expected ✗ marker in: {}", row);
    }

    #[test]
    fn row_shows_dice_for_priority() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let mut c = card("urgent-task", "running");
        c.priority = Some(1);
        let row = r.render_card_row(&c);
        assert!(row.contains('⚀'), "expected ⚀ dice glyph in: {}", row);
    }

    #[test]
    fn row_no_dice_without_priority() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
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
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let mut c = card("my-feature", "running");
        c.title = "My Feature".into();
        let row = r.render_card_row(&c);
        assert!(row.contains("My Feature"), "title missing in: {}", row);
    }

    #[test]
    fn row_shows_provider() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let mut c = card("my-feature", "running");
        c.provider = Some("claude".into());
        let row = r.render_card_row(&c);
        assert!(row.contains("claude"), "provider missing in: {}", row);
    }

    #[test]
    fn row_shows_elapsed() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let mut c = card("my-feature", "running");
        c.elapsed_s = Some(134);
        let row = r.render_card_row(&c);
        assert!(row.contains("2m14s"), "elapsed missing in: {}", row);
    }

    #[test]
    fn row_shows_progress() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let mut c = card("my-feature", "running");
        c.progress = 67;
        let row = r.render_card_row(&c);
        assert!(row.contains("67%"), "progress missing in: {}", row);
    }

    #[test]
    fn row_hides_zero_progress() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let c = card("pending-card", "pending");
        let row = r.render_card_row(&c);
        assert!(!row.contains('%'), "0% should be hidden in: {}", row);
    }

    #[test]
    fn row_shows_exit_code_on_failed() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let mut c = card("broken", "failed");
        c.exit_code = Some(1);
        let row = r.render_card_row(&c);
        assert!(row.contains("exit 1"), "exit code missing in: {}", row);
    }

    #[test]
    fn row_hides_exit_code_on_non_failed() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
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
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let mut c = card("my-feature", "running");
        c.title = "my-feature".into();
        c.provider = Some("claude".into());
        c.elapsed_s = Some(134);
        c.progress = 67;
        c.priority = Some(2);

        let row = r.render_card_row(&c);
        assert!(row.contains('║'), "double-line border missing");
        assert!(row.contains('╭'), "rounded separator missing");
        assert!(row.contains('▶'), "marker missing");
        assert!(row.contains("my-feature"), "title missing");
        assert!(row.contains('⚁'), "dice glyph missing");
        assert!(row.contains("claude"), "provider missing");
        assert!(row.contains("2m14s"), "elapsed missing");
        assert!(row.contains("67%"), "progress missing");
        assert_has_rgb(&row);
    }

    // ── Progress bar ────────────────────────────────────────────────────

    #[test]
    fn progress_bar_uses_block_chars() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let bar = r.render_progress(50, None, 0.0, None, None);
        assert!(bar.contains('█'), "expected █ in: {}", bar);
        assert!(bar.contains('░'), "expected ░ in: {}", bar);
    }

    #[test]
    fn progress_bar_16_cells() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
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
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let bar = r.render_progress(0, None, 0.0, None, None);
        let filled = bar.matches('█').count();
        let empty = bar.matches('░').count();
        assert_eq!(filled, 0, "0% should have 0 filled cells");
        assert_eq!(empty, 16, "0% should have 16 empty cells");
        assert!(bar.contains("0%"), "should show 0%");
    }

    #[test]
    fn progress_bar_full() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let bar = r.render_progress(100, None, 0.0, None, None);
        let filled = bar.matches('█').count();
        let empty = bar.matches('░').count();
        assert_eq!(filled, 16, "100% should have 16 filled cells");
        assert_eq!(empty, 0, "100% should have 0 empty cells");
        assert!(bar.contains("100%"), "should show 100%");
    }

    #[test]
    fn progress_has_rgb_gradient() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let bar = r.render_progress(100, None, 0.0, None, None);
        // With 16 filled cells, there should be multiple distinct RGB codes.
        let rgb_count = bar.matches("\x1b[38;2;").count();
        assert!(
            rgb_count >= 2,
            "expected multiple RGB gradient escapes, got {} in: {}",
            rgb_count,
            bar
        );
    }

    #[test]
    fn progress_gradient_starts_amber() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let bar = r.render_progress(50, None, 0.0, None, None);
        // First filled cell should be amber (184, 105, 15).
        assert!(
            bar.contains("\x1b[38;2;184;105;15m"),
            "expected amber RGB at start of gradient in: {}",
            bar
        );
    }

    #[test]
    fn progress_gradient_ends_near_green() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let bar = r.render_progress(100, None, 0.0, None, None);
        // Last filled cell at t=15/15=1.0 should be green (30, 138, 69).
        assert!(
            bar.contains("\x1b[38;2;30;138;69m"),
            "expected green RGB at end of gradient in: {}",
            bar
        );
    }

    #[test]
    fn progress_has_double_line_border() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let bar = r.render_progress(50, None, 0.0, None, None);
        assert!(
            bar.contains('║'),
            "expected ║ double-line border in progress: {}",
            bar
        );
    }

    #[test]
    fn progress_has_rounded_bottom() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let bar = r.render_progress(50, None, 0.0, None, None);
        assert!(
            bar.contains('╰') && bar.contains('╯'),
            "expected ╰╯ rounded bottom in progress: {}",
            bar
        );
    }

    #[test]
    fn progress_with_phase_and_moon_glyph() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
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
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
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

    // ── Lerp ────────────────────────────────────────────────────────────

    #[test]
    fn lerp_u8_endpoints() {
        assert_eq!(lerp_u8(0, 255, 0.0), 0);
        assert_eq!(lerp_u8(0, 255, 1.0), 255);
    }

    #[test]
    fn lerp_u8_midpoint() {
        // 0 + (255 - 0) * 0.5 = 127.5 → 127
        assert_eq!(lerp_u8(0, 255, 0.5), 127);
    }

    #[test]
    fn lerp_u8_clamps() {
        assert_eq!(lerp_u8(100, 200, -1.0), 100);
        assert_eq!(lerp_u8(100, 200, 2.0), 200);
    }

    // ── Footer ──────────────────────────────────────────────────────────

    #[test]
    fn footer_has_double_box_chars() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let stats = Stats {
            total: 5,
            by_state: HashMap::new(),
            success_rate_pct: None,
            avg_duration_s: None,
        };
        let footer = r.render_footer(&stats);
        assert!(footer.contains('╚'), "expected ╚ in footer: {}", footer);
        assert!(footer.contains('╝'), "expected ╝ in footer: {}", footer);
        assert!(footer.contains('═'), "expected ═ in footer: {}", footer);
        assert_has_double_box(&footer);
    }

    #[test]
    fn footer_all_stats() {
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
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
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
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
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
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
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
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
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
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
        let r = TrueColorRenderer {
            width: 80,
            two_column: false,
        };
        let stats = Stats {
            total: 5,
            by_state: HashMap::new(),
            success_rate_pct: None,
            avg_duration_s: None,
        };
        let footer = r.render_footer(&stats);
        let fill_count = footer.matches('═').count();
        assert!(
            fill_count > 10,
            "expected significant ═ fill in footer, got {} in: {}",
            fill_count,
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

    // ── RGB color helpers ───────────────────────────────────────────────

    #[test]
    fn state_rgb_format() {
        let c = state_rgb("running");
        assert!(c.starts_with("\x1b[38;2;"), "expected RGB prefix in: {}", c);
        assert!(c.ends_with('m'), "expected trailing 'm' in: {}", c);
    }

    #[test]
    fn state_rgb_known_states() {
        assert_eq!(state_rgb("pending"), "\x1b[38;2;58;90;138m");
        assert_eq!(state_rgb("running"), "\x1b[38;2;184;105;15m");
        assert_eq!(state_rgb("done"), "\x1b[38;2;30;138;69m");
        assert_eq!(state_rgb("failed"), "\x1b[38;2;196;48;48m");
        assert_eq!(state_rgb("merged"), "\x1b[38;2;107;61;184m");
    }

    #[test]
    fn state_rgb_unknown_state() {
        let c = state_rgb("unknown");
        assert_eq!(c, "\x1b[38;2;85;85;85m");
    }

    #[test]
    fn gradient_fg_amber_at_zero() {
        let c = gradient_fg(0.0);
        // Amber: RGB(184, 105, 15)
        assert_eq!(c, "\x1b[38;2;184;105;15m");
    }

    #[test]
    fn gradient_fg_green_at_one() {
        let c = gradient_fg(1.0);
        // Green: RGB(30, 138, 69)
        assert_eq!(c, "\x1b[38;2;30;138;69m");
    }

    // ── BMP-only glyphs ─────────────────────────────────────────────────

    #[test]
    fn all_glyphs_are_bmp() {
        let glyphs = "╔═╗║╠╣╚╝╭─╮│╰╯█░▶✓✗·⚀⚁⚂⚃⚄⚅◔◑◕●▌";
        for ch in glyphs.chars() {
            assert!(
                (ch as u32) <= 0xFFFF,
                "glyph '{}' (U+{:04X}) is outside BMP",
                ch,
                ch as u32
            );
        }
    }

    // ── Integration: full render through render_board ────────────────────

    #[test]
    fn truecolor_board_has_double_box_and_rgb() {
        use crate::termcaps::TermCaps;

        let caps = TermCaps {
            level: crate::termcaps::TermLevel::TrueColor,
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

        // Double-line box chars in header and footer.
        assert!(output.contains('╔'), "expected ╔ in board");
        assert!(output.contains('╗'), "expected ╗ in board");
        assert!(output.contains('╚'), "expected ╚ in board");
        assert!(output.contains('╝'), "expected ╝ in board");
        assert!(output.contains('═'), "expected ═ in board");
        assert!(output.contains('║'), "expected ║ in board");

        // Rounded card separators.
        assert!(output.contains('╭'), "expected ╭ in board");
        assert!(output.contains('╰'), "expected ╰ in board");

        // Progress bar chars (running card has progress=67).
        assert!(output.contains('█'), "expected █ progress fill in board");

        // State sections.
        assert!(output.contains("RUNNING"));
        assert!(output.contains("PENDING"));
        assert!(output.contains("FAILED"));

        // Stats.
        assert!(output.contains("3 total"));

        // 24-bit RGB ANSI codes.
        assert!(
            output.contains("\x1b[38;2;"),
            "expected 24-bit RGB ANSI codes in board"
        );
        assert_has_ansi(&output);
    }
}
