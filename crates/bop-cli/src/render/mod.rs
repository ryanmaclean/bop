/// Card rendering with progressive terminal degradation.
///
/// Selects layout based on [`TermLevel`]: Dumb (ASCII) through TrueColor
/// (double-line box, RGB gradient). The caller assembles [`CardView`] from
/// `Meta` + optional AC plan data; renderers never read files.
use std::collections::HashMap;

use bop_core::Meta;

use crate::termcaps::{TermCaps, TermLevel};

mod basic;
mod dumb;
mod extended;
mod full;
mod truecolor;

// ── Data structs ────────────────────────────────────────────────────────────

/// Flat view struct built from `Meta` + optional AC plan data.
/// The renderer operates on this — no filesystem access needed.
///
/// Some fields (e.g. `glyph`, `token`, `stage`) are not consumed by every
/// renderer but are part of the public data model for future consumers
/// (spec 029 TUI, vibekanban provider).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct CardView {
    pub id: String,
    pub state: String,
    /// SMP playing-card glyph (U+1F0xx) — may be absent on older cards.
    pub glyph: Option<String>,
    /// BMP-safe suit+rank token (e.g. "♠A") for terminals that lack SMP.
    pub token: Option<String>,
    /// Human-readable title (falls back to `id`).
    pub title: String,
    /// Current pipeline stage (e.g. "implement", "qa").
    pub stage: String,
    /// Priority rank: 1=Urgent .. 4=Low; None = unset.
    pub priority: Option<i64>,
    /// Overall completion 0–100.
    pub progress: u8,
    /// Provider from the most recent run (e.g. "claude", "codex").
    pub provider: Option<String>,
    /// Wall-clock seconds from the most recent run.
    pub elapsed_s: Option<u64>,
    /// AC plan phase name (from `implementation_plan.json`).
    pub phase_name: Option<String>,
    /// Progress fraction within the current phase (0.0–1.0).
    pub phase_frac: f32,
    /// Human-readable failure reason (set on failed cards).
    pub failure_reason: Option<String>,
    /// Process exit code from the last run.
    pub exit_code: Option<i32>,
    /// Number of AC subtasks completed (from `implementation_plan.json`).
    pub ac_subtasks_done: Option<usize>,
    /// Total number of AC subtasks (from `implementation_plan.json`).
    pub ac_subtasks_total: Option<usize>,
}

/// Summary statistics for the rendered board.
///
/// `by_state` is populated for downstream consumers (e.g. dashboard, TUI)
/// even though current renderers use only aggregate fields.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Stats {
    pub total: usize,
    pub by_state: HashMap<String, usize>,
    pub avg_duration_s: Option<f64>,
    pub success_rate_pct: Option<f64>,
}

// ── Renderer trait ──────────────────────────────────────────────────────────

/// Trait implemented by each terminal-level renderer (dumb through truecolor).
pub trait CardRenderer {
    /// Section header line for a state group (e.g. "=== RUNNING (2) ===").
    fn render_section_header(&self, label: &str, count: usize) -> String;

    /// One row per card in the listing.
    fn render_card_row(&self, card: &CardView) -> String;

    /// Progress bar with optional phase annotation.
    ///
    /// When `ac_done` and `ac_total` are `Some`, renderers that support AC
    /// plan data show an "N/T" count and switch to [`crate::acplan::half_circle_glyph`]
    /// instead of the default moon-quarter glyph.
    fn render_progress(
        &self,
        pct: u8,
        phase: Option<&str>,
        phase_frac: f32,
        ac_done: Option<usize>,
        ac_total: Option<usize>,
    ) -> String;

    /// Footer with summary statistics.
    fn render_footer(&self, stats: &Stats) -> String;
}

// ── Conversion ──────────────────────────────────────────────────────────────

/// Build a [`CardView`] from a [`Meta`] and its filesystem state directory name.
///
/// Fields not available from `Meta` (e.g. `phase_name`, `phase_frac`) are set
/// to defaults — the caller may enrich them from AC plan data afterward.
pub fn from_meta(meta: &Meta, state: &str) -> CardView {
    let last_run = meta.runs.last();

    CardView {
        id: meta.id.clone(),
        state: state.to_string(),
        glyph: meta.glyph.clone(),
        token: meta.token.clone(),
        title: meta.title.clone().unwrap_or_else(|| meta.id.clone()),
        stage: meta.stage.clone(),
        priority: meta.priority,
        progress: meta.progress.unwrap_or(0),
        provider: last_run
            .map(|r| r.provider.clone())
            .filter(|s| !s.is_empty()),
        elapsed_s: last_run.and_then(|r| r.duration_s),
        phase_name: None,
        phase_frac: 0.0,
        failure_reason: meta.failure_reason.clone(),
        exit_code: meta.exit_code,
        ac_subtasks_done: None,
        ac_subtasks_total: None,
    }
}

// ── Board dispatch ──────────────────────────────────────────────────────────

/// Render a full board to a `String`, selecting the renderer by [`TermLevel`].
///
/// `views` is a list of `(section_label, cards)` pairs — one per state group.
/// Width is re-queried from `caps` on every call (never cached).
pub fn render_board(caps: &TermCaps, views: &[(String, Vec<CardView>)], stats: &Stats) -> String {
    let renderer: Box<dyn CardRenderer> = select_renderer(caps);
    let mut out = String::new();

    for (i, (label, cards)) in views.iter().enumerate() {
        out.push_str(&renderer.render_section_header(label, cards.len()));
        out.push('\n');

        for card in cards {
            out.push_str(&renderer.render_card_row(card));
            out.push('\n');

            if card.progress > 0 || card.phase_name.is_some() {
                out.push_str(&renderer.render_progress(
                    card.progress,
                    card.phase_name.as_deref(),
                    card.phase_frac,
                    card.ac_subtasks_done,
                    card.ac_subtasks_total,
                ));
                out.push('\n');
            }
        }

        // Blank line between sections (but not after the last).
        if i < views.len() - 1 {
            out.push('\n');
        }
    }

    out.push_str(&renderer.render_footer(stats));
    out.push('\n');

    out
}

/// Pick the concrete renderer for the detected terminal level.
fn select_renderer(caps: &TermCaps) -> Box<dyn CardRenderer> {
    match caps.level {
        TermLevel::Dumb => Box::new(dumb::DumbRenderer),
        TermLevel::Basic => Box::new(basic::BasicRenderer),
        TermLevel::Extended => Box::new(extended::ExtendedRenderer { width: caps.width }),
        TermLevel::Full => Box::new(full::FullRenderer { width: caps.width }),
        TermLevel::TrueColor => Box::new(truecolor::TrueColorRenderer {
            width: caps.width,
            two_column: caps.two_column,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bop_core::{Meta, RunRecord};

    #[test]
    fn from_meta_basic_fields() {
        let meta = Meta {
            id: "test-card".into(),
            stage: "implement".into(),
            glyph: Some("\u{1F0AB}".into()),
            token: Some("\u{2660}".into()),
            title: Some("My Feature".into()),
            priority: Some(2),
            progress: Some(67),
            created: chrono::Utc::now(),
            ..Default::default()
        };

        let view = from_meta(&meta, "running");
        assert_eq!(view.id, "test-card");
        assert_eq!(view.state, "running");
        assert_eq!(view.title, "My Feature");
        assert_eq!(view.progress, 67);
        assert_eq!(view.priority, Some(2));
        assert!(view.provider.is_none());
        assert!(view.elapsed_s.is_none());
    }

    #[test]
    fn from_meta_title_defaults_to_id() {
        let meta = Meta {
            id: "no-title".into(),
            stage: "pending".into(),
            created: chrono::Utc::now(),
            ..Default::default()
        };

        let view = from_meta(&meta, "pending");
        assert_eq!(view.title, "no-title");
    }

    #[test]
    fn from_meta_extracts_last_run() {
        let meta = Meta {
            id: "run-card".into(),
            stage: "done".into(),
            created: chrono::Utc::now(),
            runs: vec![
                RunRecord {
                    provider: "codex".into(),
                    duration_s: Some(60),
                    ..Default::default()
                },
                RunRecord {
                    provider: "claude".into(),
                    duration_s: Some(180),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let view = from_meta(&meta, "done");
        assert_eq!(view.provider.as_deref(), Some("claude"));
        assert_eq!(view.elapsed_s, Some(180));
    }

    #[test]
    fn from_meta_empty_provider_becomes_none() {
        let meta = Meta {
            id: "empty-provider".into(),
            stage: "done".into(),
            created: chrono::Utc::now(),
            runs: vec![RunRecord {
                provider: String::new(),
                duration_s: Some(42),
                ..Default::default()
            }],
            ..Default::default()
        };

        let view = from_meta(&meta, "done");
        assert!(view.provider.is_none());
        assert_eq!(view.elapsed_s, Some(42));
    }

    #[test]
    fn from_meta_failure_fields() {
        let meta = Meta {
            id: "failed-card".into(),
            stage: "implement".into(),
            created: chrono::Utc::now(),
            failure_reason: Some("rate limit".into()),
            exit_code: Some(75),
            ..Default::default()
        };

        let view = from_meta(&meta, "failed");
        assert_eq!(view.failure_reason.as_deref(), Some("rate limit"));
        assert_eq!(view.exit_code, Some(75));
    }

    #[test]
    fn stats_default() {
        let stats = Stats {
            total: 0,
            by_state: HashMap::new(),
            avg_duration_s: None,
            success_rate_pct: None,
        };
        assert_eq!(stats.total, 0);
    }

    #[test]
    fn render_board_empty() {
        let caps = TermCaps {
            level: TermLevel::Dumb,
            width: 80,
            two_column: false,
        };
        let views: Vec<(String, Vec<CardView>)> = vec![];
        let stats = Stats {
            total: 0,
            by_state: HashMap::new(),
            avg_duration_s: None,
            success_rate_pct: None,
        };

        let output = render_board(&caps, &views, &stats);
        // Should at least contain the footer
        assert!(!output.is_empty());
    }

    #[test]
    fn select_renderer_matches_level() {
        // Just verify select_renderer doesn't panic for each level.
        for level in [
            TermLevel::Dumb,
            TermLevel::Basic,
            TermLevel::Extended,
            TermLevel::Full,
            TermLevel::TrueColor,
        ] {
            let caps = TermCaps {
                level,
                width: 120,
                two_column: level == TermLevel::TrueColor,
            };
            let _ = select_renderer(&caps);
        }
    }
}
