/// Level 0 — Dumb renderer (no color, ASCII only).
///
/// Used when TERM=dumb or unset. Pure ASCII, no escape sequences.
use super::{CardRenderer, CardView, Stats};

pub struct DumbRenderer;

impl CardRenderer for DumbRenderer {
    fn render_section_header(&self, label: &str, count: usize) -> String {
        format!("=== {} ({}) ===", label.to_uppercase(), count)
    }

    fn render_card_row(&self, card: &CardView) -> String {
        let marker = match card.state.as_str() {
            "running" => "[>]",
            "done" | "merged" => "[x]",
            "failed" => "[!]",
            _ => "[ ]",
        };

        let mut parts = vec![format!("{} {}", marker, card.title)];

        if let Some(ref provider) = card.provider {
            parts.push(provider.clone());
        }

        if let Some(elapsed) = card.elapsed_s {
            parts.push(format_duration(elapsed));
        }

        if card.progress > 0 {
            parts.push(format!("{}%", card.progress));
        }

        if let Some(code) = card.exit_code {
            if card.state == "failed" {
                parts.push(format!("exit {}", code));
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
            format!("  {}  {}", bar, phase)
        } else {
            format!("  {}", bar)
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

        parts.join(" | ")
    }
}

/// Format seconds into human-readable "Xm Ys" or "Xs".
fn format_duration(secs: u64) -> String {
    if secs >= 60 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{}s", secs)
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

    /// Assert the string contains only ASCII characters (no box-drawing, no unicode blocks).
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

    /// Assert the string contains no ANSI escape sequences (\x1b[...).
    fn assert_no_ansi(s: &str) {
        assert!(!s.contains('\x1b'), "ANSI escape sequence found in: {}", s);
    }

    // ── Section header ──────────────────────────────────────────────────

    #[test]
    fn header_format() {
        let r = DumbRenderer;
        let h = r.render_section_header("running", 2);
        assert_eq!(h, "=== RUNNING (2) ===");
    }

    #[test]
    fn header_uppercases_label() {
        let r = DumbRenderer;
        let h = r.render_section_header("pending", 0);
        assert!(h.contains("PENDING"));
        assert!(!h.contains("pending"));
    }

    #[test]
    fn header_zero_count() {
        let r = DumbRenderer;
        let h = r.render_section_header("done", 0);
        assert_eq!(h, "=== DONE (0) ===");
    }

    #[test]
    fn header_ascii_only() {
        let r = DumbRenderer;
        assert_ascii_only(&r.render_section_header("failed", 5));
    }

    #[test]
    fn header_no_ansi() {
        let r = DumbRenderer;
        assert_no_ansi(&r.render_section_header("running", 3));
    }

    // ── Card row markers ────────────────────────────────────────────────

    #[test]
    fn row_running_marker() {
        let r = DumbRenderer;
        let c = card("my-feature", "running");
        let row = r.render_card_row(&c);
        assert!(row.starts_with("[>]"), "expected [>] marker, got: {}", row);
    }

    #[test]
    fn row_pending_marker() {
        let r = DumbRenderer;
        let c = card("docs-update", "pending");
        let row = r.render_card_row(&c);
        assert!(row.starts_with("[ ]"), "expected [ ] marker, got: {}", row);
    }

    #[test]
    fn row_drafts_marker() {
        let r = DumbRenderer;
        let c = card("draft-idea", "drafts");
        let row = r.render_card_row(&c);
        assert!(row.starts_with("[ ]"), "expected [ ] marker, got: {}", row);
    }

    #[test]
    fn row_done_marker() {
        let r = DumbRenderer;
        let c = card("perf-improvements", "done");
        let row = r.render_card_row(&c);
        assert!(row.starts_with("[x]"), "expected [x] marker, got: {}", row);
    }

    #[test]
    fn row_merged_marker() {
        let r = DumbRenderer;
        let c = card("merged-card", "merged");
        let row = r.render_card_row(&c);
        assert!(row.starts_with("[x]"), "expected [x] marker, got: {}", row);
    }

    #[test]
    fn row_failed_marker() {
        let r = DumbRenderer;
        let c = card("broken-card", "failed");
        let row = r.render_card_row(&c);
        assert!(row.starts_with("[!]"), "expected [!] marker, got: {}", row);
    }

    // ── Card row content ────────────────────────────────────────────────

    #[test]
    fn row_shows_title() {
        let r = DumbRenderer;
        let mut c = card("my-feature", "running");
        c.title = "My Feature".into();
        let row = r.render_card_row(&c);
        assert!(row.contains("My Feature"), "title missing in: {}", row);
    }

    #[test]
    fn row_shows_provider() {
        let r = DumbRenderer;
        let mut c = card("my-feature", "running");
        c.provider = Some("claude".into());
        let row = r.render_card_row(&c);
        assert!(row.contains("claude"), "provider missing in: {}", row);
    }

    #[test]
    fn row_shows_elapsed() {
        let r = DumbRenderer;
        let mut c = card("my-feature", "running");
        c.elapsed_s = Some(134);
        let row = r.render_card_row(&c);
        assert!(row.contains("2m14s"), "elapsed missing in: {}", row);
    }

    #[test]
    fn row_shows_elapsed_short() {
        let r = DumbRenderer;
        let mut c = card("quick-card", "done");
        c.elapsed_s = Some(43);
        let row = r.render_card_row(&c);
        assert!(row.contains("43s"), "short elapsed missing in: {}", row);
    }

    #[test]
    fn row_shows_progress() {
        let r = DumbRenderer;
        let mut c = card("my-feature", "running");
        c.progress = 67;
        let row = r.render_card_row(&c);
        assert!(row.contains("67%"), "progress missing in: {}", row);
    }

    #[test]
    fn row_hides_zero_progress() {
        let r = DumbRenderer;
        let c = card("pending-card", "pending");
        let row = r.render_card_row(&c);
        assert!(!row.contains('%'), "0% should be hidden in: {}", row);
    }

    #[test]
    fn row_shows_exit_code_on_failed() {
        let r = DumbRenderer;
        let mut c = card("broken", "failed");
        c.exit_code = Some(1);
        let row = r.render_card_row(&c);
        assert!(row.contains("exit 1"), "exit code missing in: {}", row);
    }

    #[test]
    fn row_hides_exit_code_on_non_failed() {
        let r = DumbRenderer;
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
        let r = DumbRenderer;
        let mut c = card("my-feature", "running");
        c.title = "my-feature".into();
        c.provider = Some("claude".into());
        c.elapsed_s = Some(134);
        c.progress = 67;

        let row = r.render_card_row(&c);
        assert!(row.starts_with("[>]"));
        assert!(row.contains("my-feature"));
        assert!(row.contains("claude"));
        assert!(row.contains("2m14s"));
        assert!(row.contains("67%"));
    }

    #[test]
    fn row_ascii_only() {
        let r = DumbRenderer;
        let mut c = card("test-card", "running");
        c.provider = Some("codex".into());
        c.elapsed_s = Some(300);
        c.progress = 50;
        assert_ascii_only(&r.render_card_row(&c));
    }

    #[test]
    fn row_no_ansi() {
        let r = DumbRenderer;
        let mut c = card("test-card", "failed");
        c.exit_code = Some(75);
        c.failure_reason = Some("rate limit".into());
        assert_no_ansi(&r.render_card_row(&c));
    }

    // ── Progress bar ────────────────────────────────────────────────────

    #[test]
    fn progress_bar_format() {
        let r = DumbRenderer;
        let bar = r.render_progress(50, None, 0.0, None, None);
        assert!(bar.contains("[=====     ] 50%"), "unexpected bar: {}", bar);
    }

    #[test]
    fn progress_bar_zero() {
        let r = DumbRenderer;
        let bar = r.render_progress(0, None, 0.0, None, None);
        assert!(
            bar.contains("[          ] 0%"),
            "unexpected bar at 0%: {}",
            bar
        );
    }

    #[test]
    fn progress_bar_full() {
        let r = DumbRenderer;
        let bar = r.render_progress(100, None, 0.0, None, None);
        assert!(
            bar.contains("[==========] 100%"),
            "unexpected bar at 100%: {}",
            bar
        );
    }

    #[test]
    fn progress_bar_with_phase() {
        let r = DumbRenderer;
        let bar = r.render_progress(67, Some("Phase 2: Network"), 0.5, None, None);
        assert!(bar.contains("67%"), "pct missing in: {}", bar);
        assert!(
            bar.contains("Phase 2: Network"),
            "phase missing in: {}",
            bar
        );
    }

    #[test]
    fn progress_bar_ascii_only() {
        let r = DumbRenderer;
        assert_ascii_only(&r.render_progress(33, Some("Phase 1"), 0.25, None, None));
    }

    #[test]
    fn progress_bar_no_ansi() {
        let r = DumbRenderer;
        assert_no_ansi(&r.render_progress(75, Some("Phase 3: QA"), 0.8, None, None));
    }

    // ── Footer ──────────────────────────────────────────────────────────

    #[test]
    fn footer_all_stats() {
        let r = DumbRenderer;
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
        // Parts joined by " | "
        assert!(footer.contains(" | "), "pipe separator missing");
    }

    #[test]
    fn footer_no_optional_stats() {
        let r = DumbRenderer;
        let stats = Stats {
            total: 3,
            by_state: HashMap::new(),
            success_rate_pct: None,
            avg_duration_s: None,
        };
        let footer = r.render_footer(&stats);
        assert_eq!(footer, "3 total");
        assert!(!footer.contains('|'));
    }

    #[test]
    fn footer_only_success_rate() {
        let r = DumbRenderer;
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
    fn footer_ascii_only() {
        let r = DumbRenderer;
        let stats = Stats {
            total: 5,
            by_state: HashMap::new(),
            success_rate_pct: Some(100.0),
            avg_duration_s: Some(60.0),
        };
        assert_ascii_only(&r.render_footer(&stats));
    }

    #[test]
    fn footer_no_ansi() {
        let r = DumbRenderer;
        let stats = Stats {
            total: 5,
            by_state: HashMap::new(),
            success_rate_pct: Some(50.0),
            avg_duration_s: Some(120.0),
        };
        assert_no_ansi(&r.render_footer(&stats));
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
    fn full_board_ascii_only() {
        use crate::termcaps::TermCaps;

        let caps = TermCaps {
            level: crate::termcaps::TermLevel::Dumb,
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

        // Verify ASCII only, no ANSI
        assert_ascii_only(&output);
        assert_no_ansi(&output);

        // Verify structure
        assert!(output.contains("=== RUNNING (1) ==="));
        assert!(output.contains("=== PENDING (1) ==="));
        assert!(output.contains("=== FAILED (1) ==="));
        assert!(output.contains("[>]"));
        assert!(output.contains("[ ]"));
        assert!(output.contains("[!]"));
        assert!(output.contains("3 total"));
    }
}
