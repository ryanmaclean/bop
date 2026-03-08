/// Detail overlay widget — card information panel on right 50% of body.
///
/// Renders on top of the kanban board using ratatui `Clear` + bordered
/// `Block`. Shows: card id, provider, elapsed time, progress bar,
/// phase/subtask tree from AC plan (`implementation_plan.json` in the
/// card directory, if present). Scrollable with j/k.
///
/// Footer keybinds (rendered by the footer widget, not here):
/// `[F3]logs [p]pause [r]retry [Esc]close`
use std::fs;
use std::path::Path;

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::render::CardView;

// ── AC plan types ───────────────────────────────────────────────────────────

/// A subtask entry from an AC `implementation_plan.json`.
#[derive(Debug, Clone)]
struct PlanSubtask {
    id: String,
    description: String,
    status: String,
}

/// A phase entry from an AC `implementation_plan.json`.
#[derive(Debug, Clone)]
struct PlanPhase {
    name: String,
    subtasks: Vec<PlanSubtask>,
}

/// Parsed AC plan data (phases + subtasks).
#[derive(Debug, Clone)]
struct AcPlan {
    phases: Vec<PlanPhase>,
}

// ── Plan parser ─────────────────────────────────────────────────────────────

/// Read and parse `implementation_plan.json` from a card directory.
///
/// Returns `None` if the file doesn't exist or can't be parsed.
/// Tolerant of missing fields — extracts what it can.
fn read_ac_plan(card_dir: &Path) -> Option<AcPlan> {
    let plan_path = card_dir.join("implementation_plan.json");
    let content = fs::read_to_string(&plan_path).ok()?;
    let root: serde_json::Value = serde_json::from_str(&content).ok()?;

    let phases_arr = root.get("phases")?.as_array()?;
    let mut phases = Vec::new();

    for phase_val in phases_arr {
        let name = phase_val
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unnamed")
            .to_string();

        let mut subtasks = Vec::new();
        if let Some(subs) = phase_val.get("subtasks").and_then(|v| v.as_array()) {
            for sub in subs {
                let id = sub
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let description = sub
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let status = sub
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("pending")
                    .to_string();

                subtasks.push(PlanSubtask {
                    id,
                    description,
                    status,
                });
            }
        }

        phases.push(PlanPhase { name, subtasks });
    }

    Some(AcPlan { phases })
}

// ── Rendering helpers ───────────────────────────────────────────────────────

/// Status glyph for a subtask status string.
fn status_glyph(status: &str) -> &'static str {
    match status {
        "completed" => "✓",
        "in_progress" => "▶",
        "failed" => "✗",
        _ => "·", // pending or unknown
    }
}

/// Status color for a subtask status string.
fn status_color(status: &str) -> Color {
    match status {
        "completed" => Color::Rgb(0x1E, 0x8A, 0x45),   // green
        "in_progress" => Color::Rgb(0xB8, 0x69, 0x0F), // amber
        "failed" => Color::Rgb(0xC4, 0x30, 0x30),      // red
        _ => Color::DarkGray,                          // pending
    }
}

/// Format seconds into human-readable "XmYYs" or "Xs".
fn format_duration(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{}s", secs)
    }
}

/// Build the progress bar line.
///
/// Format: `████████░░░░░░░░░░░░ 42%`
fn build_progress_bar(pct: u8, width: usize) -> Line<'static> {
    let bar_width = width.saturating_sub(6).min(30); // reserve space for " NNN%"
    let filled = (pct as usize * bar_width) / 100;
    let empty = bar_width.saturating_sub(filled);

    let bar = format!("{}{}", "█".repeat(filled), "░".repeat(empty));

    let color = if pct >= 75 {
        Color::Rgb(0x1E, 0x8A, 0x45) // green
    } else if pct >= 40 {
        Color::Rgb(0xB8, 0x69, 0x0F) // amber
    } else {
        Color::Rgb(0x3A, 0x5A, 0x8A) // blue
    };

    Line::from(vec![
        Span::styled(bar, Style::default().fg(color)),
        Span::styled(format!(" {:>3}%", pct), Style::default().fg(Color::White)),
    ])
}

/// Build all content lines for the detail overlay.
///
/// Returns a `Vec<Line>` that can be scrolled via `detail_scroll`.
fn build_detail_lines(card: &CardView, card_dir: Option<&Path>) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    // ── Card ID ─────────────────────────────────────────────────────────
    lines.push(Line::from(vec![
        Span::styled(
            "Card: ",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(card.id.clone(), Style::default().fg(Color::White)),
    ]));

    // ── State ───────────────────────────────────────────────────────────
    let state_color = match card.state.as_str() {
        "pending" => Color::Rgb(0x3A, 0x5A, 0x8A),
        "running" => Color::Rgb(0xB8, 0x69, 0x0F),
        "done" => Color::Rgb(0x1E, 0x8A, 0x45),
        "failed" => Color::Rgb(0xC4, 0x30, 0x30),
        "merged" => Color::Rgb(0x6B, 0x3D, 0xB8),
        _ => Color::Gray,
    };
    lines.push(Line::from(vec![
        Span::styled(
            "State: ",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(card.state.clone(), Style::default().fg(state_color)),
    ]));

    // ── Stage ───────────────────────────────────────────────────────────
    lines.push(Line::from(vec![
        Span::styled(
            "Stage: ",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(card.stage.clone(), Style::default().fg(Color::White)),
    ]));

    // ── Provider ────────────────────────────────────────────────────────
    if let Some(ref provider) = card.provider {
        lines.push(Line::from(vec![
            Span::styled(
                "Provider: ",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(provider.clone(), Style::default().fg(Color::Cyan)),
        ]));
    }

    // ── Elapsed ─────────────────────────────────────────────────────────
    if let Some(elapsed) = card.elapsed_s {
        lines.push(Line::from(vec![
            Span::styled(
                "Elapsed: ",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format_duration(elapsed), Style::default().fg(Color::White)),
        ]));
    }

    // ── Priority ────────────────────────────────────────────────────────
    if let Some(pri) = card.priority {
        let (label, color) = match pri {
            1 => ("Urgent", Color::Red),
            2 => ("High", Color::Yellow),
            3 => ("Normal", Color::White),
            4 => ("Low", Color::DarkGray),
            _ => ("Unknown", Color::Gray),
        };
        lines.push(Line::from(vec![
            Span::styled(
                "Priority: ",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(label, Style::default().fg(color)),
        ]));
    }

    // ── Failure reason ──────────────────────────────────────────────────
    if let Some(ref reason) = card.failure_reason {
        lines.push(Line::from(vec![
            Span::styled(
                "Failure: ",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                reason.clone(),
                Style::default().fg(Color::Rgb(0xC4, 0x30, 0x30)),
            ),
        ]));
    }

    // ── Exit code ───────────────────────────────────────────────────────
    if let Some(code) = card.exit_code {
        lines.push(Line::from(vec![
            Span::styled(
                "Exit code: ",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{}", code),
                Style::default().fg(if code == 0 {
                    Color::Rgb(0x1E, 0x8A, 0x45)
                } else {
                    Color::Rgb(0xC4, 0x30, 0x30)
                }),
            ),
        ]));
    }

    // ── Blank separator ─────────────────────────────────────────────────
    lines.push(Line::from(""));

    // ── Progress bar ────────────────────────────────────────────────────
    if card.progress > 0 {
        lines.push(Line::from(Span::styled(
            "Progress",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(build_progress_bar(card.progress, 40));
        lines.push(Line::from(""));
    }

    // ── AC plan phase/subtask tree ──────────────────────────────────────
    let plan = card_dir.and_then(read_ac_plan);
    if let Some(ac_plan) = plan {
        lines.push(Line::from(Span::styled(
            "Implementation Plan",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )));
        lines.push(Line::from(""));

        for phase in &ac_plan.phases {
            // Count completed / total subtasks in this phase.
            let total = phase.subtasks.len();
            let completed = phase
                .subtasks
                .iter()
                .filter(|s| s.status == "completed")
                .count();

            let phase_status = if total > 0 && completed == total {
                "✓"
            } else if phase.subtasks.iter().any(|s| s.status == "in_progress") {
                "▶"
            } else if completed > 0 {
                "◑"
            } else {
                "·"
            };

            let phase_color = if total > 0 && completed == total {
                Color::Rgb(0x1E, 0x8A, 0x45) // green
            } else if phase.subtasks.iter().any(|s| s.status == "in_progress") {
                Color::Rgb(0xB8, 0x69, 0x0F) // amber
            } else {
                Color::DarkGray
            };

            lines.push(Line::from(vec![
                Span::styled(
                    format!("{} ", phase_status),
                    Style::default().fg(phase_color),
                ),
                Span::styled(
                    phase.name.clone(),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" ({}/{})", completed, total),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));

            // Subtask tree lines.
            for (i, sub) in phase.subtasks.iter().enumerate() {
                let is_last = i == phase.subtasks.len() - 1;
                let branch = if is_last { "└─" } else { "├─" };
                let glyph = status_glyph(&sub.status);
                let color = status_color(&sub.status);

                // Truncate description to keep it readable.
                let desc = if sub.description.len() > 60 {
                    format!("{}…", &sub.description[..59])
                } else {
                    sub.description.clone()
                };

                lines.push(Line::from(vec![
                    Span::styled(
                        format!("  {} ", branch),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(format!("{} ", glyph), Style::default().fg(color)),
                    Span::styled(sub.id.clone(), Style::default().fg(color)),
                    Span::styled(format!(" {}", desc), Style::default().fg(Color::DarkGray)),
                ]));
            }

            lines.push(Line::from("")); // blank between phases
        }
    }

    lines
}

// ── Public render function ──────────────────────────────────────────────────

/// Render the detail overlay on the right 50% of the given body area.
///
/// Uses ratatui `Clear` to erase the underlying kanban content, then draws
/// a bordered `Block` with the card detail content. The overlay is scrolled
/// by `app.detail_scroll` lines — scroll up/down is handled by the Detail
/// mode key handler in `input.rs`.
///
/// The `card_dir` is the path to the card bundle on disk (for reading
/// `implementation_plan.json`). If `None`, the plan section is skipped.
pub fn render_detail(
    frame: &mut Frame,
    body_area: Rect,
    card: &CardView,
    detail_scroll: usize,
    card_dir: Option<&Path>,
) {
    // Overlay occupies the right 50% of the body area.
    let overlay_width = body_area.width / 2;
    let overlay_x = body_area.x + body_area.width - overlay_width;
    let overlay_area = Rect::new(overlay_x, body_area.y, overlay_width, body_area.height);

    // Clear the overlay region so the kanban content beneath is hidden.
    frame.render_widget(Clear, overlay_area);

    // Build the block with a title showing the card id.
    let title = format!(" {} — detail ", card.id);
    let block = Block::bordered()
        .title(title)
        .title_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .border_style(Style::default().fg(Color::Yellow));

    // Build content lines and apply scroll offset.
    let lines = build_detail_lines(card, card_dir);

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((detail_scroll as u16, 0));

    frame.render_widget(paragraph, overlay_area);
}

/// Return the total number of content lines for the detail view.
///
/// Used by the scroll handler to clamp `detail_scroll` so the user
/// can't scroll past the end.
#[allow(dead_code)]
pub fn detail_line_count(card: &CardView, card_dir: Option<&Path>) -> usize {
    build_detail_lines(card, card_dir).len()
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // ── format_duration ─────────────────────────────────────────────────

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
        assert_eq!(format_duration(3599), "59m59s");
    }

    #[test]
    fn format_duration_hours() {
        assert_eq!(format_duration(3600), "1h00m");
        assert_eq!(format_duration(3661), "1h01m");
        assert_eq!(format_duration(7200), "2h00m");
    }

    // ── status_glyph ────────────────────────────────────────────────────

    #[test]
    fn status_glyph_all_states() {
        assert_eq!(status_glyph("completed"), "✓");
        assert_eq!(status_glyph("in_progress"), "▶");
        assert_eq!(status_glyph("failed"), "✗");
        assert_eq!(status_glyph("pending"), "·");
        assert_eq!(status_glyph("unknown"), "·");
    }

    // ── build_detail_lines ──────────────────────────────────────────────

    #[test]
    fn detail_lines_basic_card() {
        let card = test_card("my-card", "running");
        let lines = build_detail_lines(&card, None);
        let text: String = lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("Card: my-card"));
        assert!(text.contains("State: running"));
        assert!(text.contains("Stage: implement"));
    }

    #[test]
    fn detail_lines_with_provider_and_elapsed() {
        let mut card = test_card("test-card", "running");
        card.provider = Some("claude".into());
        card.elapsed_s = Some(120);

        let lines = build_detail_lines(&card, None);
        let text: String = lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("Provider: claude"));
        assert!(text.contains("Elapsed: 2m00s"));
    }

    #[test]
    fn detail_lines_with_progress() {
        let mut card = test_card("prog-card", "running");
        card.progress = 67;

        let lines = build_detail_lines(&card, None);
        let text: String = lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("Progress"));
        assert!(text.contains("67%"));
    }

    #[test]
    fn detail_lines_with_failure() {
        let mut card = test_card("fail-card", "failed");
        card.failure_reason = Some("rate limit".into());
        card.exit_code = Some(75);

        let lines = build_detail_lines(&card, None);
        let text: String = lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("Failure: rate limit"));
        assert!(text.contains("Exit code: 75"));
    }

    // ── AC plan parsing ─────────────────────────────────────────────────

    #[test]
    fn read_ac_plan_valid() {
        let td = tempfile::tempdir().unwrap();
        let plan = serde_json::json!({
            "phases": [
                {
                    "name": "Phase 1",
                    "subtasks": [
                        {"id": "s1", "description": "Do thing", "status": "completed"},
                        {"id": "s2", "description": "Do other", "status": "pending"}
                    ]
                }
            ]
        });
        std::fs::write(td.path().join("implementation_plan.json"), plan.to_string()).unwrap();

        let result = read_ac_plan(td.path()).unwrap();
        assert_eq!(result.phases.len(), 1);
        assert_eq!(result.phases[0].name, "Phase 1");
        assert_eq!(result.phases[0].subtasks.len(), 2);
        assert_eq!(result.phases[0].subtasks[0].status, "completed");
    }

    #[test]
    fn read_ac_plan_missing_file() {
        let td = tempfile::tempdir().unwrap();
        assert!(read_ac_plan(td.path()).is_none());
    }

    #[test]
    fn read_ac_plan_invalid_json() {
        let td = tempfile::tempdir().unwrap();
        std::fs::write(td.path().join("implementation_plan.json"), "not json").unwrap();
        assert!(read_ac_plan(td.path()).is_none());
    }

    #[test]
    fn detail_lines_with_ac_plan() {
        let td = tempfile::tempdir().unwrap();
        let plan = serde_json::json!({
            "phases": [
                {
                    "name": "Foundation",
                    "subtasks": [
                        {"id": "s1-1", "description": "Setup deps", "status": "completed"},
                        {"id": "s1-2", "description": "Create module", "status": "in_progress"}
                    ]
                },
                {
                    "name": "Layout",
                    "subtasks": [
                        {"id": "s2-1", "description": "Kanban columns", "status": "pending"}
                    ]
                }
            ]
        });
        std::fs::write(td.path().join("implementation_plan.json"), plan.to_string()).unwrap();

        let card = test_card("plan-card", "running");
        let lines = build_detail_lines(&card, Some(td.path()));
        let text: String = lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("Implementation Plan"));
        assert!(text.contains("Foundation"));
        assert!(text.contains("(1/2)"));
        assert!(text.contains("s1-1"));
        assert!(text.contains("s1-2"));
        assert!(text.contains("Layout"));
        assert!(text.contains("(0/1)"));
    }

    #[test]
    fn detail_line_count_matches() {
        let card = test_card("count-card", "running");
        let lines = build_detail_lines(&card, None);
        let count = detail_line_count(&card, None);
        assert_eq!(lines.len(), count);
    }
}
