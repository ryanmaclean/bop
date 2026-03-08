/// Level 1 — Basic renderer (8-color ANSI, ASCII borders).
///
/// Adds color to state labels and simple `+--+` box borders.
/// Uses standard 8-color ANSI codes (30–37) with ASCII `+`, `-`, `|` borders.
use crate::colors::{BOLD, DIM, RESET};

use super::{CardRenderer, CardView, Stats};

pub struct BasicRenderer;

/// 8-color ANSI foreground code for a given state.
fn state_color_8(state: &str) -> &'static str {
    match state {
        "pending" => "\x1b[34m", // blue
        "running" => "\x1b[33m", // yellow
        "done" => "\x1b[32m",    // green
        "failed" => "\x1b[31m",  // red
        "merged" => "\x1b[35m",  // magenta
        _ => "\x1b[37m",         // white
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

impl CardRenderer for BasicRenderer {
    fn render_section_header(&self, label: &str, count: usize) -> String {
        let color = state_color_8(label);
        format!(
            "{}{}+-- {} ({}) --+{}",
            BOLD,
            color,
            label.to_uppercase(),
            count,
            RESET,
        )
    }

    fn render_card_row(&self, card: &CardView) -> String {
        let color = state_color_8(&card.state);

        let marker = match card.state.as_str() {
            "running" => format!("{}[>]{}", color, RESET),
            "done" | "merged" => format!("{}[x]{}", color, RESET),
            "failed" => format!("{}[!]{}", color, RESET),
            _ => "[ ]".to_string(),
        };

        let mut parts = vec![format!("| {} {}", marker, card.title)];

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
        _phase_frac: f32,
        _ac_done: Option<usize>,
        _ac_total: Option<usize>,
    ) -> String {
        let bar_width = 10;
        let filled = (pct as usize * bar_width) / 100;
        let empty = bar_width - filled;

        let bar = format!("[{}{}] {}%", "=".repeat(filled), " ".repeat(empty), pct,);

        if let Some(phase) = phase {
            format!("|   {}  {}", bar, phase)
        } else {
            format!("|   {}", bar)
        }
    }

    fn render_footer(&self, stats: &Stats) -> String {
        let mut parts = vec![format!("{} total", stats.total)];

        if let Some(rate) = stats.success_rate_pct {
            parts.push(format!("{:.0}% success", rate));
        }

        if let Some(avg) = stats.avg_duration_s {
            parts.push(format!("avg {}", format_duration(avg as u64)));
        }

        let content = parts.join(" | ");
        format!("{}+-- {} --+{}", DIM, content, RESET)
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

    /// Assert the string contains ASCII box border characters.
    fn assert_has_borders(s: &str) {
        assert!(
            s.contains('+') || s.contains('|') || s.contains('-'),
            "expected ASCII border chars (+, -, |) in: {}",
            s
        );
    }

    /// Assert the string contains only ASCII characters (ANSI escapes are ASCII).
    fn assert_ascii_only(s: &str) {
        for ch in s.chars() {
            assert!(
                ch.is_ascii(),
                "non-ASCII character U+{:04X} '{}' found in: {}",
                ch as u32,
                ch,
                s
            );
        }
    }

    // ── Section header ──────────────────────────────────────────────────

    #[test]
    fn header_format() {
        let r = BasicRenderer;
        let h = r.render_section_header("running", 2);
        assert!(h.contains("RUNNING"));
        assert!(h.contains("(2)"));
    }

    #[test]
    fn header_uppercases_label() {
        let r = BasicRenderer;
        let h = r.render_section_header("pending", 0);
        assert!(h.contains("PENDING"));
    }

    #[test]
    fn header_zero_count() {
        let r = BasicRenderer;
        let h = r.render_section_header("done", 0);
        assert!(h.contains("DONE"));
        assert!(h.contains("(0)"));
    }

    #[test]
    fn header_has_ansi_colors() {
        let r = BasicRenderer;
        let h = r.render_section_header("running", 3);
        assert_has_ansi(&h);
    }

    #[test]
    fn header_has_box_borders() {
        let r = BasicRenderer;
        let h = r.render_section_header("done", 1);
        assert_has_borders(&h);
        assert!(h.contains("+--"), "expected +-- border in: {}", h);
        assert!(h.contains("--+"), "expected --+ border in: {}", h);
    }

    #[test]
    fn header_ascii_only() {
        let r = BasicRenderer;
        assert_ascii_only(&r.render_section_header("failed", 5));
    }

    #[test]
    fn header_colored_by_state() {
        let r = BasicRenderer;
        // Running uses yellow (\x1b[33m)
        let h = r.render_section_header("running", 1);
        assert!(h.contains("\x1b[33m"), "expected yellow for running: {}", h);

        // Failed uses red (\x1b[31m)
        let h = r.render_section_header("failed", 1);
        assert!(h.contains("\x1b[31m"), "expected red for failed: {}", h);

        // Done uses green (\x1b[32m)
        let h = r.render_section_header("done", 1);
        assert!(h.contains("\x1b[32m"), "expected green for done: {}", h);
    }

    // ── Card row markers ────────────────────────────────────────────────

    #[test]
    fn row_running_marker() {
        let r = BasicRenderer;
        let c = card("my-feature", "running");
        let row = r.render_card_row(&c);
        assert!(row.contains("[>]"), "expected [>] marker in: {}", row);
    }

    #[test]
    fn row_pending_marker() {
        let r = BasicRenderer;
        let c = card("docs-update", "pending");
        let row = r.render_card_row(&c);
        assert!(row.contains("[ ]"), "expected [ ] marker in: {}", row);
    }

    #[test]
    fn row_drafts_marker() {
        let r = BasicRenderer;
        let c = card("draft-idea", "drafts");
        let row = r.render_card_row(&c);
        assert!(row.contains("[ ]"), "expected [ ] marker in: {}", row);
    }

    #[test]
    fn row_done_marker() {
        let r = BasicRenderer;
        let c = card("perf-improvements", "done");
        let row = r.render_card_row(&c);
        assert!(row.contains("[x]"), "expected [x] marker in: {}", row);
    }

    #[test]
    fn row_merged_marker() {
        let r = BasicRenderer;
        let c = card("merged-card", "merged");
        let row = r.render_card_row(&c);
        assert!(row.contains("[x]"), "expected [x] marker in: {}", row);
    }

    #[test]
    fn row_failed_marker() {
        let r = BasicRenderer;
        let c = card("broken-card", "failed");
        let row = r.render_card_row(&c);
        assert!(row.contains("[!]"), "expected [!] marker in: {}", row);
    }

    #[test]
    fn row_has_left_border() {
        let r = BasicRenderer;
        let c = card("test-card", "running");
        let row = r.render_card_row(&c);
        assert!(
            row.starts_with("| "),
            "expected left border '| ' in: {}",
            row
        );
    }

    #[test]
    fn row_markers_colored() {
        let r = BasicRenderer;

        let c = card("run", "running");
        let row = r.render_card_row(&c);
        assert_has_ansi(&row);
        assert!(
            row.contains("\x1b[33m"),
            "running marker should be yellow: {}",
            row
        );

        let c = card("fail", "failed");
        let row = r.render_card_row(&c);
        assert!(
            row.contains("\x1b[31m"),
            "failed marker should be red: {}",
            row
        );
    }

    // ── Card row content ────────────────────────────────────────────────

    #[test]
    fn row_shows_title() {
        let r = BasicRenderer;
        let mut c = card("my-feature", "running");
        c.title = "My Feature".into();
        let row = r.render_card_row(&c);
        assert!(row.contains("My Feature"), "title missing in: {}", row);
    }

    #[test]
    fn row_shows_provider() {
        let r = BasicRenderer;
        let mut c = card("my-feature", "running");
        c.provider = Some("claude".into());
        let row = r.render_card_row(&c);
        assert!(row.contains("claude"), "provider missing in: {}", row);
    }

    #[test]
    fn row_shows_elapsed() {
        let r = BasicRenderer;
        let mut c = card("my-feature", "running");
        c.elapsed_s = Some(134);
        let row = r.render_card_row(&c);
        assert!(row.contains("2m14s"), "elapsed missing in: {}", row);
    }

    #[test]
    fn row_shows_elapsed_short() {
        let r = BasicRenderer;
        let mut c = card("quick-card", "done");
        c.elapsed_s = Some(43);
        let row = r.render_card_row(&c);
        assert!(row.contains("43s"), "short elapsed missing in: {}", row);
    }

    #[test]
    fn row_shows_progress() {
        let r = BasicRenderer;
        let mut c = card("my-feature", "running");
        c.progress = 67;
        let row = r.render_card_row(&c);
        assert!(row.contains("67%"), "progress missing in: {}", row);
    }

    #[test]
    fn row_hides_zero_progress() {
        let r = BasicRenderer;
        let c = card("pending-card", "pending");
        let row = r.render_card_row(&c);
        assert!(!row.contains('%'), "0% should be hidden in: {}", row);
    }

    #[test]
    fn row_shows_exit_code_on_failed() {
        let r = BasicRenderer;
        let mut c = card("broken", "failed");
        c.exit_code = Some(1);
        let row = r.render_card_row(&c);
        assert!(row.contains("exit 1"), "exit code missing in: {}", row);
    }

    #[test]
    fn row_hides_exit_code_on_non_failed() {
        let r = BasicRenderer;
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
        let r = BasicRenderer;
        let mut c = card("my-feature", "running");
        c.title = "my-feature".into();
        c.provider = Some("claude".into());
        c.elapsed_s = Some(134);
        c.progress = 67;

        let row = r.render_card_row(&c);
        assert!(row.starts_with("| "));
        assert!(row.contains("[>]"));
        assert!(row.contains("my-feature"));
        assert!(row.contains("claude"));
        assert!(row.contains("2m14s"));
        assert!(row.contains("67%"));
        assert_has_ansi(&row);
    }

    #[test]
    fn row_ascii_only() {
        let r = BasicRenderer;
        let mut c = card("test-card", "running");
        c.provider = Some("codex".into());
        c.elapsed_s = Some(300);
        c.progress = 50;
        assert_ascii_only(&r.render_card_row(&c));
    }

    // ── Progress bar ────────────────────────────────────────────────────

    #[test]
    fn progress_bar_format() {
        let r = BasicRenderer;
        let bar = r.render_progress(50, None, 0.0, None, None);
        assert!(bar.contains("[=====     ] 50%"), "unexpected bar: {}", bar);
    }

    #[test]
    fn progress_bar_zero() {
        let r = BasicRenderer;
        let bar = r.render_progress(0, None, 0.0, None, None);
        assert!(
            bar.contains("[          ] 0%"),
            "unexpected bar at 0%: {}",
            bar
        );
    }

    #[test]
    fn progress_bar_full() {
        let r = BasicRenderer;
        let bar = r.render_progress(100, None, 0.0, None, None);
        assert!(
            bar.contains("[==========] 100%"),
            "unexpected bar at 100%: {}",
            bar
        );
    }

    #[test]
    fn progress_bar_with_phase() {
        let r = BasicRenderer;
        let bar = r.render_progress(67, Some("Phase 2: Network"), 0.5, None, None);
        assert!(bar.contains("67%"), "pct missing in: {}", bar);
        assert!(
            bar.contains("Phase 2: Network"),
            "phase missing in: {}",
            bar
        );
    }

    #[test]
    fn progress_has_left_border() {
        let r = BasicRenderer;
        let bar = r.render_progress(50, None, 0.0, None, None);
        assert!(bar.starts_with("|"), "expected left border '|' in: {}", bar);
    }

    #[test]
    fn progress_ascii_only() {
        let r = BasicRenderer;
        assert_ascii_only(&r.render_progress(33, Some("Phase 1"), 0.25, None, None));
    }

    // ── Footer ──────────────────────────────────────────────────────────

    #[test]
    fn footer_all_stats() {
        let r = BasicRenderer;
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
        let r = BasicRenderer;
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
        let r = BasicRenderer;
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
    fn footer_has_box_borders() {
        let r = BasicRenderer;
        let stats = Stats {
            total: 5,
            by_state: HashMap::new(),
            success_rate_pct: Some(100.0),
            avg_duration_s: Some(60.0),
        };
        let footer = r.render_footer(&stats);
        assert_has_borders(&footer);
        assert!(
            footer.contains("+--"),
            "expected +-- border in footer: {}",
            footer
        );
    }

    #[test]
    fn footer_has_ansi() {
        let r = BasicRenderer;
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
    fn footer_ascii_only() {
        let r = BasicRenderer;
        let stats = Stats {
            total: 5,
            by_state: HashMap::new(),
            success_rate_pct: Some(100.0),
            avg_duration_s: Some(60.0),
        };
        assert_ascii_only(&r.render_footer(&stats));
    }

    // ── format_duration ─────────────────────────────────────────────────

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
    fn full_board_has_ansi_and_borders() {
        use crate::termcaps::TermCaps;

        let caps = TermCaps {
            level: crate::termcaps::TermLevel::Basic,
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

        // Verify ANSI codes present (unlike DumbRenderer)
        assert_has_ansi(&output);

        // Verify ASCII border characters
        assert!(output.contains("+--"), "expected +-- borders");
        assert!(output.contains("| "), "expected | card borders");

        // Verify structure
        assert!(output.contains("RUNNING"));
        assert!(output.contains("PENDING"));
        assert!(output.contains("FAILED"));
        assert!(output.contains("[>]"));
        assert!(output.contains("[ ]"));
        assert!(output.contains("[!]"));
        assert!(output.contains("3 total"));

        // Verify ASCII only (no unicode box drawing)
        assert_ascii_only(&output);
    }
}
