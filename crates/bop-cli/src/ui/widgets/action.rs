/// Action popup overlay widget — centered command palette for card actions.
///
/// Renders as a centered popup overlay using ratatui `Clear` + bordered
/// `Block`. Shows a list of actions filtered by the selected card's state:
///
/// - **running**: `[F3]logs  [z]zellij  [p]pause  [k]kill`
/// - **done**: `[r]retry`
/// - **failed**: `[r]retry`
/// - **pending**: `[k]kill(remove)`
///
/// Each action is a selectable `List` item. Press `↵` in Normal mode to
/// enter `Mode::ActionPopup`; `Esc` returns to Normal. Selecting an action
/// with `↵` executes it (actions are dispatched via `ActionKind`).
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState};
use ratatui::Frame;

// ── Action types ────────────────────────────────────────────────────────────

/// The kind of action a user can perform on a card.
///
/// Each variant maps to an existing `cmd_*` function in `cards.rs` or
/// a mode transition (e.g. `LogTail` opens the log tail overlay).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    /// Open log tail overlay (F3 / `Mode::LogTail`).
    Logs,
    /// Open card worktree in a Zellij pane.
    Zellij,
    /// Pause a running card (`cmd_pause`).
    Pause,
    /// Kill a running card (`cmd_kill`).
    Kill,
    /// Retry a done/failed card (`cmd_retry`).
    Retry,
    /// Remove a pending card from the queue.
    Remove,
}

/// A single action entry in the popup list.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ActionEntry {
    /// The action kind (used for dispatch).
    pub kind: ActionKind,
    /// Keybinding hint shown in the list (e.g. "[F3]").
    pub key_hint: &'static str,
    /// Human-readable label (e.g. "logs").
    pub label: &'static str,
}

// ── State-filtered action lists ─────────────────────────────────────────────

/// Return the list of actions available for a given card state.
///
/// Actions are filtered based on what makes sense for the card's current
/// lifecycle state:
/// - `running` → logs, zellij, pause, kill
/// - `done` → retry
/// - `failed` → retry
/// - `pending` → kill(remove)
/// - `merged` → (no actions)
pub fn actions_for_state(state: &str) -> Vec<ActionEntry> {
    match state {
        "running" => vec![
            ActionEntry {
                kind: ActionKind::Logs,
                key_hint: "[F3]",
                label: "logs",
            },
            ActionEntry {
                kind: ActionKind::Zellij,
                key_hint: "[z]",
                label: "zellij",
            },
            ActionEntry {
                kind: ActionKind::Pause,
                key_hint: "[p]",
                label: "pause",
            },
            ActionEntry {
                kind: ActionKind::Kill,
                key_hint: "[k]",
                label: "kill",
            },
        ],
        "done" => vec![ActionEntry {
            kind: ActionKind::Retry,
            key_hint: "[r]",
            label: "retry",
        }],
        "failed" => vec![ActionEntry {
            kind: ActionKind::Retry,
            key_hint: "[r]",
            label: "retry",
        }],
        "pending" => vec![ActionEntry {
            kind: ActionKind::Remove,
            key_hint: "[k]",
            label: "kill (remove)",
        }],
        _ => vec![], // merged or unknown — no actions
    }
}

// ── Rendering ───────────────────────────────────────────────────────────────

/// Render the action popup overlay centered in the given body area.
///
/// Uses ratatui `Clear` to erase the underlying kanban content, then draws
/// a bordered `Block` with a selectable `List` of state-filtered actions.
///
/// The popup is sized to fit the action list: width is clamped between
/// 30 and 50 characters, height is the action count + 2 (for borders).
#[allow(dead_code)]
pub fn render_action_popup(
    frame: &mut Frame,
    body_area: Rect,
    card_state: &str,
    card_id: &str,
    action_list_state: &mut ListState,
) {
    let actions = actions_for_state(card_state);

    if actions.is_empty() {
        // No actions available — render a small "no actions" popup.
        render_empty_popup(frame, body_area, card_id);
        return;
    }

    // Popup dimensions.
    let popup_width = 36u16.min(body_area.width.saturating_sub(4));
    let popup_height = (actions.len() as u16 + 2).min(body_area.height.saturating_sub(2));

    let popup_area = centered_rect(popup_width, popup_height, body_area);

    // Clear the underlying content.
    frame.render_widget(Clear, popup_area);

    // Build the bordered block with title.
    let title = format!(" {} ", card_id);
    let block = Block::bordered()
        .title(title)
        .title_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .border_style(Style::default().fg(Color::Yellow));

    // Build list items from actions.
    let items: Vec<ListItem> = actions
        .iter()
        .map(|action| {
            let line = Line::from(vec![
                Span::styled(
                    format!("{} ", action.key_hint),
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(action.label, Style::default().fg(Color::White)),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    frame.render_stateful_widget(list, popup_area, action_list_state);
}

/// Render a small popup indicating no actions are available.
#[allow(dead_code)]
fn render_empty_popup(frame: &mut Frame, body_area: Rect, card_id: &str) {
    let popup_width = 30u16.min(body_area.width.saturating_sub(4));
    let popup_height = 3u16.min(body_area.height.saturating_sub(2));

    let popup_area = centered_rect(popup_width, popup_height, body_area);
    frame.render_widget(Clear, popup_area);

    let title = format!(" {} ", card_id);
    let block = Block::bordered()
        .title(title)
        .title_style(
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .border_style(Style::default().fg(Color::DarkGray));

    let paragraph = ratatui::widgets::Paragraph::new(Line::from(Span::styled(
        "no actions available",
        Style::default().fg(Color::DarkGray),
    )))
    .block(block);

    frame.render_widget(paragraph, popup_area);
}

/// Compute a centered `Rect` of the given width and height within `area`.
#[allow(dead_code)]
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── actions_for_state ───────────────────────────────────────────────

    #[test]
    fn running_has_four_actions() {
        let actions = actions_for_state("running");
        assert_eq!(actions.len(), 4);
        assert_eq!(actions[0].kind, ActionKind::Logs);
        assert_eq!(actions[1].kind, ActionKind::Zellij);
        assert_eq!(actions[2].kind, ActionKind::Pause);
        assert_eq!(actions[3].kind, ActionKind::Kill);
    }

    #[test]
    fn done_has_retry() {
        let actions = actions_for_state("done");
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].kind, ActionKind::Retry);
    }

    #[test]
    fn failed_has_retry() {
        let actions = actions_for_state("failed");
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].kind, ActionKind::Retry);
    }

    #[test]
    fn pending_has_remove() {
        let actions = actions_for_state("pending");
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].kind, ActionKind::Remove);
    }

    #[test]
    fn merged_has_no_actions() {
        let actions = actions_for_state("merged");
        assert!(actions.is_empty());
    }

    #[test]
    fn unknown_state_has_no_actions() {
        let actions = actions_for_state("bogus");
        assert!(actions.is_empty());
    }

    // ── centered_rect ───────────────────────────────────────────────────

    #[test]
    fn centered_rect_basic() {
        let area = Rect::new(0, 0, 80, 24);
        let popup = centered_rect(30, 10, area);
        assert_eq!(popup.x, 25);
        assert_eq!(popup.y, 7);
        assert_eq!(popup.width, 30);
        assert_eq!(popup.height, 10);
    }

    #[test]
    fn centered_rect_clamps_to_area() {
        let area = Rect::new(0, 0, 20, 10);
        let popup = centered_rect(40, 20, area);
        // Width and height are clamped to area dimensions.
        assert_eq!(popup.width, 20);
        assert_eq!(popup.height, 10);
    }

    #[test]
    fn centered_rect_with_offset_area() {
        let area = Rect::new(10, 5, 60, 20);
        let popup = centered_rect(20, 8, area);
        assert_eq!(popup.x, 30); // 10 + (60-20)/2
        assert_eq!(popup.y, 11); // 5 + (20-8)/2
    }

    // ── action_entry fields ─────────────────────────────────────────────

    #[test]
    fn action_entries_have_key_hints() {
        for state in &["running", "done", "failed", "pending"] {
            for action in actions_for_state(state) {
                assert!(
                    !action.key_hint.is_empty(),
                    "missing key_hint for {:?}",
                    action.kind
                );
                assert!(
                    !action.label.is_empty(),
                    "missing label for {:?}",
                    action.kind
                );
            }
        }
    }
}
