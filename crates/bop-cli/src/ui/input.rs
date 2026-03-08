/// Key input handling dispatched by current [`Mode`].
///
/// Each mode has its own handler function that processes [`KeyEvent`]s and
/// mutates [`App`] state accordingly. The main entry point [`handle_key`]
/// dispatches to the correct handler based on `app.mode`.
///
/// Normal mode keybindings:
/// - `h` / `l` — move column focus left / right (wrapping)
/// - `j` / `k` — move selected card down / up within current column
///
/// Other modes (Filter, ActionPopup, Detail, LogTail, NewCard) will be
/// wired in subsequent subtasks.
use std::fs;

use crossterm::event::{KeyCode, KeyEvent};

use super::app::{App, AppTab, Mode};
use crate::cards;
use crate::paths;
use crate::ui::widgets::action::{actions_for_state, ActionKind};
use crate::ui::widgets::newcard::create_card;

/// Dispatch a key event to the handler for the current mode.
///
/// Returns without effect for modes that don't yet have handlers wired
/// (they will be added in later subtasks). The quit keys (q, Ctrl-C) are
/// handled in the event loop before this function is called.
pub fn handle_key(app: &mut App, key: KeyEvent) {
    if key.code == KeyCode::F(2) {
        app.toggle_tab();
        return;
    }

    if app.tab == AppTab::Factory {
        handle_factory(app, key);
        return;
    }

    match app.mode {
        Mode::Normal => handle_normal(app, key),
        Mode::Filter => handle_filter(app, key),
        Mode::ActionPopup => handle_action_popup(app, key),
        Mode::Detail => handle_detail(app, key),
        Mode::LogTail => handle_logtail(app, key),
        Mode::NewCard => handle_newcard(app, key),
        Mode::Subshell => {
            // Subshell mode suspends the TUI — no key handling here.
        }
    }
}

// ── Normal mode ─────────────────────────────────────────────────────────────

/// Handle key events in Normal mode.
///
/// Navigation:
/// - `h` — move focus to previous column (wrapping, skipping collapsed)
/// - `l` — move focus to next column (wrapping, skipping collapsed)
/// - `j` — select next card in current column
/// - `k` — select previous card in current column
fn handle_normal(app: &mut App, key: KeyEvent) {
    // Clear any transient status message on new keypress.
    app.status_message = None;

    match key.code {
        KeyCode::Char('h') | KeyCode::Left => {
            move_col_focus(app, Direction::Left);
        }
        KeyCode::Char('l') | KeyCode::Right => {
            move_col_focus(app, Direction::Right);
        }
        KeyCode::Char('j') | KeyCode::Down => {
            move_card_selection(app, Direction::Down);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            move_card_selection(app, Direction::Up);
        }
        KeyCode::Char('d') => {
            // Only enter Detail mode if a card is selected.
            if app.selected_card().is_some() {
                app.detail_scroll = 0;
                app.mode = Mode::Detail;
            }
        }
        KeyCode::Char('/') => {
            app.enter_filter_mode();
        }
        KeyCode::Enter => {
            // Open action popup for the selected card.
            app.open_action_popup();
        }
        KeyCode::Char('n') => {
            app.newcard_input.clear();
            app.mode = Mode::NewCard;
        }
        KeyCode::Char('H') => {
            // Shift+H: move selected card left (toward pending).
            try_move_card(app, CardMoveDir::Left);
        }
        KeyCode::Char('L') => {
            // Shift+L: move selected card right (toward merged).
            try_move_card(app, CardMoveDir::Right);
        }
        KeyCode::Tab => {
            // Tab multi-select (yazi pattern): toggle mark on hovered card.
            app.toggle_mark();
        }
        KeyCode::Char('!') => {
            // Subshell (vim :! convention): drop to $SHELL in card worktree.
            app.prepare_subshell();
        }
        KeyCode::Esc => {
            // Esc clears all marks in Normal mode.
            app.clear_marks();
        }
        _ => {}
    }
}

// ── Factory tab mode ────────────────────────────────────────────────────────

/// Handle key events while the Factory tab is active.
///
/// Keybindings:
/// - `j` / `Down` — select next service row
/// - `k` / `Up` — select previous service row
/// - `s` — stop selected service
/// - `r` — start/restart selected service
/// - `l` — switch log pane between dispatcher and merge-gate
/// - `Esc` — return to kanban tab
/// - `Tab` — cycle to the next tab (same behavior as `F2`)
fn handle_factory(app: &mut App, key: KeyEvent) {
    // Clear any transient status message on new keypress.
    app.status_message = None;

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            app.factory_tab.select_next();
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.factory_tab.select_prev();
        }
        KeyCode::Char('s') => match app.factory_tab.stop_selected() {
            Ok(label) => {
                app.status_message = Some(format!("stopped {}", label));
            }
            Err(err) => {
                app.status_message = Some(format!("stop failed: {}", err));
            }
        },
        KeyCode::Char('r') => match app.factory_tab.start_selected() {
            Ok(label) => {
                app.status_message = Some(format!("started {}", label));
            }
            Err(err) => {
                app.status_message = Some(format!("start failed: {}", err));
            }
        },
        KeyCode::Char('l') => {
            app.factory_tab.toggle_log_source();
        }
        KeyCode::Esc => {
            app.switch_to_kanban_tab();
        }
        KeyCode::Tab => {
            app.toggle_tab();
        }
        _ => {}
    }
}

// ── Filter mode ─────────────────────────────────────────────────────────────

/// Handle key events in Filter mode.
///
/// Keystrokes are appended to the filter query string. Special keys:
/// - `Esc` — clear the filter and return to Normal mode
/// - `Enter` — confirm the filter (stay in Normal with filter active)
/// - `Backspace` — delete last character from query
/// - Any printable `Char` — append to query
///
/// On each keystroke that modifies the query, the filter collapse state
/// is recalculated so columns with no matches collapse in real-time.
fn handle_filter(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.clear_filter();
        }
        KeyCode::Enter => {
            app.confirm_filter();
        }
        KeyCode::Backspace => {
            if let Some(ref mut state) = app.filter_state {
                state.query.pop();
                app.filter = if state.query.is_empty() {
                    None
                } else {
                    Some(state.query.clone())
                };
            }
            app.apply_filter_collapse();
        }
        KeyCode::Char(c) => {
            if let Some(ref mut state) = app.filter_state {
                state.query.push(c);
                app.filter = Some(state.query.clone());
            }
            app.apply_filter_collapse();
        }
        _ => {}
    }
}

// ── Detail mode ─────────────────────────────────────────────────────────────

/// Handle key events in Detail mode.
///
/// Navigation:
/// - `j` / `Down` — scroll detail content down one line
/// - `k` / `Up` — scroll detail content up one line
/// - `Esc` — return to Normal mode
fn handle_detail(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            app.detail_scroll = app.detail_scroll.saturating_add(1);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.detail_scroll = app.detail_scroll.saturating_sub(1);
        }
        KeyCode::Esc => {
            app.mode = Mode::Normal;
        }
        _ => {
            // Other Detail-mode keys (F3, p, r) will trigger actions
            // in subsequent subtasks.
        }
    }
}

// ── ActionPopup mode ────────────────────────────────────────────────────────

/// Handle key events in ActionPopup mode.
///
/// Navigation:
/// - `j` / `Down` — select next action
/// - `k` / `Up` — select previous action
/// - `Enter` — execute the selected action
/// - `Esc` — close the popup, return to Normal mode
fn handle_action_popup(app: &mut App, key: KeyEvent) {
    // Get the action count for the current card's state.
    let action_count = app
        .selected_card()
        .map(|c| actions_for_state(&c.state).len())
        .unwrap_or(0);

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if action_count > 0 {
                let current = app.action_list_state.selected().unwrap_or(0);
                let next = if current >= action_count - 1 {
                    0
                } else {
                    current + 1
                };
                app.action_list_state.select(Some(next));
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if action_count > 0 {
                let current = app.action_list_state.selected().unwrap_or(0);
                let prev = if current == 0 {
                    action_count - 1
                } else {
                    current - 1
                };
                app.action_list_state.select(Some(prev));
            }
        }
        KeyCode::Enter => {
            // Execute the selected action.
            // When cards are marked, bulk actions apply to all marked cards.
            if let Some(selected_idx) = app.action_list_state.selected() {
                if let Some(card) = app.selected_card() {
                    let actions = actions_for_state(&card.state);
                    if let Some(action) = actions.get(selected_idx) {
                        match action.kind {
                            ActionKind::Logs => {
                                // Logs only apply to the selected card (not bulk).
                                app.enter_log_tail();
                                return;
                            }
                            ActionKind::Pause => {
                                let targets = app.action_target_ids();
                                for id in &targets {
                                    dispatch_action_pause(app, id);
                                }
                                app.clear_marks();
                                app.mode = Mode::Normal;
                                return;
                            }
                            ActionKind::Kill => {
                                let targets = app.action_target_ids();
                                for id in &targets {
                                    dispatch_action_kill(app, id);
                                }
                                app.clear_marks();
                                app.mode = Mode::Normal;
                                return;
                            }
                            ActionKind::Retry => {
                                let targets = app.action_target_ids();
                                for id in &targets {
                                    dispatch_action_retry(app, id);
                                }
                                app.clear_marks();
                                app.mode = Mode::Normal;
                                return;
                            }
                            ActionKind::Zellij | ActionKind::Remove => {
                                // Zellij and Remove will be wired in subsequent
                                // subtasks.
                                app.mode = Mode::Normal;
                                return;
                            }
                        }
                    }
                }
            }
            app.mode = Mode::Normal;
        }
        KeyCode::Esc => {
            app.mode = Mode::Normal;
        }
        _ => {}
    }
}

// ── LogTail mode ────────────────────────────────────────────────────────────

/// Handle key events in LogTail mode.
///
/// Navigation:
/// - `↑` / `k` — scroll up one line (disables follow mode)
/// - `↓` / `j` — scroll down one line (disables follow mode)
/// - `f` — toggle follow mode (auto-scroll to bottom)
/// - `c` — clear the log buffer
/// - `Esc` — exit LogTail, return to Normal mode
fn handle_logtail(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            app.log_scroll = app.log_scroll.saturating_sub(1);
            app.log_follow = false;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.log_scroll = app.log_scroll.saturating_add(1);
            app.log_follow = false;
        }
        KeyCode::Char('f') => {
            app.log_follow = !app.log_follow;
        }
        KeyCode::Char('c') => {
            app.log_buf.clear();
            app.log_scroll = 0;
        }
        KeyCode::Esc => {
            app.exit_log_tail();
        }
        _ => {}
    }
}

// ── NewCard mode ────────────────────────────────────────────────────────

/// Handle key events in NewCard mode.
///
/// Input:
/// - Any printable `Char` — append to the card ID input buffer
/// - `Backspace` — delete last character from input
/// - `Enter` — create the card via `bop new default <id>`, then refresh
/// - `Esc` — cancel and return to Normal mode
fn handle_newcard(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.newcard_input.clear();
            app.mode = Mode::Normal;
        }
        KeyCode::Enter => {
            let id = app.newcard_input.trim().to_string();
            if !id.is_empty() {
                // Best-effort card creation — log errors but don't crash the TUI.
                if let Err(e) = create_card(&id) {
                    eprintln!("bop new failed: {}", e);
                }
                // Trigger a cards refresh by rebuilding columns from disk.
                let new_columns = super::app::build_columns(&app.cards_root);
                app.refresh_columns(new_columns);
            }
            app.newcard_input.clear();
            app.mode = Mode::Normal;
        }
        KeyCode::Backspace => {
            app.newcard_input.pop();
        }
        KeyCode::Char(c) => {
            app.newcard_input.push(c);
        }
        _ => {}
    }
}

// ── Navigation helpers ──────────────────────────────────────────────────────

/// Direction for navigation movement.
enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// Move column focus left or right, wrapping at boundaries.
///
/// Skips collapsed (empty) columns — if all columns are collapsed,
/// focus stays at index 0.
fn move_col_focus(app: &mut App, dir: Direction) {
    let len = app.columns.len();
    if len == 0 {
        return;
    }

    let step: isize = match dir {
        Direction::Left => -1,
        Direction::Right => 1,
        _ => return,
    };

    // Try up to `len` positions to find a non-collapsed column.
    let mut candidate = app.col_focus as isize;
    for _ in 0..len {
        candidate = (candidate + step).rem_euclid(len as isize);
        let idx = candidate as usize;
        if !app.columns[idx].collapsed {
            app.col_focus = idx;
            return;
        }
    }

    // All columns collapsed — stay where we are.
}

/// Move card selection up or down within the currently focused column.
///
/// Wraps around at both ends: pressing `j` on the last card selects the
/// first, and pressing `k` on the first card selects the last.
fn move_card_selection(app: &mut App, dir: Direction) {
    let Some(col) = app.focused_column_mut() else {
        return;
    };

    let card_count = col.cards.len();
    if card_count == 0 {
        return;
    }

    let current = col.list_state.selected().unwrap_or(0);

    let new_idx = match dir {
        Direction::Up => {
            if current == 0 {
                card_count - 1
            } else {
                current - 1
            }
        }
        Direction::Down => {
            if current >= card_count - 1 {
                0
            } else {
                current + 1
            }
        }
        _ => return,
    };

    col.list_state.select(Some(new_idx));
}

// ── Card movement (Shift+H/L) ──────────────────────────────────────────────

/// Direction for card state movement (Shift+H = left, Shift+L = right).
enum CardMoveDir {
    Left,
    Right,
}

/// Allowed card state transitions from the TUI.
///
/// Only safe moves are permitted:
/// - `done → pending` (Shift+H from done column)
/// - `failed → pending` (Shift+H from failed column)
///
/// `pending → running` is NOT allowed — that's the dispatcher's job.
fn allowed_target_state(source_state: &str, dir: &CardMoveDir) -> Option<&'static str> {
    match (source_state, dir) {
        ("done", CardMoveDir::Left) => Some("pending"),
        ("failed", CardMoveDir::Left) => Some("pending"),
        _ => None,
    }
}

/// Try to move the selected card to an adjacent state column via `fs::rename`.
///
/// On success, rebuilds columns from disk and sets a status message.
/// On failure (card not found, rename error, disallowed move), sets an
/// error status message in the footer.
fn try_move_card(app: &mut App, dir: CardMoveDir) {
    // Get the selected card info.
    let (card_id, source_state) = match (app.selected_card(), app.focused_column()) {
        (Some(card), Some(col)) => (card.id.clone(), col.state.clone()),
        _ => return,
    };

    // Check if the move is allowed.
    let target_state = match allowed_target_state(&source_state, &dir) {
        Some(state) => state,
        None => {
            app.status_message = Some(format!("cannot move card from {}", source_state));
            return;
        }
    };

    // Find the card's actual directory on disk.
    let card_path = match paths::find_card(&app.cards_root, &card_id) {
        Some(p) => p,
        None => {
            app.status_message = Some(format!("card not found: {}", card_id));
            return;
        }
    };

    // Compute target path preserving team directory structure.
    // card_path = .../state_dir/card-name.bop
    // state_dir.parent() = cards_root or cards_root/team-xxx
    let card_dir_name = match card_path.file_name() {
        Some(n) => n.to_os_string(),
        None => {
            app.status_message = Some("invalid card path".to_string());
            return;
        }
    };
    let state_dir = match card_path.parent() {
        Some(p) => p,
        None => {
            app.status_message = Some("invalid card path".to_string());
            return;
        }
    };
    let base_dir = match state_dir.parent() {
        Some(p) => p,
        None => {
            app.status_message = Some("invalid card path".to_string());
            return;
        }
    };

    let target_dir = base_dir.join(target_state);

    // Ensure target state directory exists.
    if fs::create_dir_all(&target_dir).is_err() {
        app.status_message = Some("failed to create target directory".to_string());
        return;
    }

    let target_path = target_dir.join(&card_dir_name);

    // Check if target already exists.
    if target_path.exists() {
        app.status_message = Some(format!("card already exists in {}", target_state));
        return;
    }

    // Perform the rename.
    match fs::rename(&card_path, &target_path) {
        Ok(()) => {
            // Refresh columns from disk.
            let new_columns = super::app::build_columns(&app.cards_root);
            app.refresh_columns(new_columns);
            app.status_message = Some(format!("moved {} → {}", card_id, target_state));
        }
        Err(e) => {
            app.status_message = Some(format!("move failed: {}", e));
        }
    }
}

// ── Action dispatch (async, fire-and-forget) ────────────────────────────────

/// Dispatch the Pause action for a card (spawns async task).
fn dispatch_action_pause(app: &App, card_id: &str) {
    let root = app.cards_root.clone();
    let id = card_id.to_string();
    tokio::spawn(async move {
        if let Err(e) = cards::cmd_pause(&root, &id).await {
            eprintln!("pause failed: {}", e);
        }
    });
}

/// Dispatch the Kill action for a card (spawns async task).
fn dispatch_action_kill(app: &App, card_id: &str) {
    let root = app.cards_root.clone();
    let id = card_id.to_string();
    tokio::spawn(async move {
        if let Err(e) = cards::cmd_kill(&root, &id).await {
            eprintln!("kill failed: {}", e);
        }
    });
}

/// Dispatch the Retry action for a card (spawns blocking task).
fn dispatch_action_retry(app: &App, card_id: &str) {
    let root = app.cards_root.clone();
    let id = card_id.to_string();
    tokio::task::spawn_blocking(move || {
        if let Err(e) = cards::cmd_retry(&root, &id) {
            eprintln!("retry failed: {}", e);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::CardView;
    use crate::ui::app::{AppTab, KanbanColumn};
    use crate::ui::factory_tab::FactoryTabState;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use ratatui::widgets::ListState;
    use std::collections::VecDeque;
    use std::path::PathBuf;

    /// Build a minimal KeyEvent for testing.
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

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

    /// Build an App with custom columns for testing (bypasses filesystem).
    fn test_app(columns: Vec<KanbanColumn>) -> App {
        let col_focus = columns.iter().position(|c| !c.collapsed).unwrap_or(0);
        App {
            columns,
            col_focus,
            filter: None,
            filter_state: None,
            mode: Mode::Normal,
            tab: AppTab::Kanban,
            factory_tab: FactoryTabState::new(),
            log_buf: VecDeque::new(),
            throughput: VecDeque::from(vec![0u8; 8]),
            cards_root: PathBuf::from("/tmp/test-cards"),
            provider_meters: Vec::new(),
            recent_completions: VecDeque::new(),
            tick_count: 0,
            prev_done_count: 0,
            detail_scroll: 0,
            action_list_state: ListState::default(),
            log_scroll: 0,
            log_follow: false,
            log_tail_card_id: None,
            log_stdout_pos: 0,
            log_stderr_pos: 0,
            log_stdout_incomplete: String::new(),
            log_stderr_incomplete: String::new(),
            newcard_input: String::new(),
            status_message: None,
            marked_cards: std::collections::HashSet::new(),
            subshell_worktree: None,
            terminal_width: 120,
            terminal_height: 40,
        }
    }

    // ── Column focus tests ──────────────────────────────────────────────

    #[test]
    fn h_moves_focus_left() {
        let columns = vec![
            KanbanColumn::new("pending", vec![test_card("a", "pending")], None),
            KanbanColumn::new("running", vec![test_card("b", "running")], None),
            KanbanColumn::new("done", vec![test_card("c", "done")], None),
        ];
        let mut app = test_app(columns);
        app.col_focus = 1;

        handle_key(&mut app, key(KeyCode::Char('h')));
        assert_eq!(app.col_focus, 0);
    }

    #[test]
    fn l_moves_focus_right() {
        let columns = vec![
            KanbanColumn::new("pending", vec![test_card("a", "pending")], None),
            KanbanColumn::new("running", vec![test_card("b", "running")], None),
            KanbanColumn::new("done", vec![test_card("c", "done")], None),
        ];
        let mut app = test_app(columns);
        app.col_focus = 0;

        handle_key(&mut app, key(KeyCode::Char('l')));
        assert_eq!(app.col_focus, 1);
    }

    #[test]
    fn h_wraps_from_first_to_last() {
        let columns = vec![
            KanbanColumn::new("pending", vec![test_card("a", "pending")], None),
            KanbanColumn::new("running", vec![test_card("b", "running")], None),
            KanbanColumn::new("done", vec![test_card("c", "done")], None),
        ];
        let mut app = test_app(columns);
        app.col_focus = 0;

        handle_key(&mut app, key(KeyCode::Char('h')));
        assert_eq!(app.col_focus, 2);
    }

    #[test]
    fn l_wraps_from_last_to_first() {
        let columns = vec![
            KanbanColumn::new("pending", vec![test_card("a", "pending")], None),
            KanbanColumn::new("running", vec![test_card("b", "running")], None),
            KanbanColumn::new("done", vec![test_card("c", "done")], None),
        ];
        let mut app = test_app(columns);
        app.col_focus = 2;

        handle_key(&mut app, key(KeyCode::Char('l')));
        assert_eq!(app.col_focus, 0);
    }

    #[test]
    fn h_skips_collapsed_columns() {
        let columns = vec![
            KanbanColumn::new("pending", vec![test_card("a", "pending")], None),
            KanbanColumn::new("running", vec![], None), // collapsed
            KanbanColumn::new("done", vec![test_card("c", "done")], None),
        ];
        let mut app = test_app(columns);
        app.col_focus = 2; // done

        handle_key(&mut app, key(KeyCode::Char('h')));
        // Should skip collapsed "running" and land on "pending"
        assert_eq!(app.col_focus, 0);
    }

    #[test]
    fn l_skips_collapsed_columns() {
        let columns = vec![
            KanbanColumn::new("pending", vec![test_card("a", "pending")], None),
            KanbanColumn::new("running", vec![], None), // collapsed
            KanbanColumn::new("done", vec![test_card("c", "done")], None),
        ];
        let mut app = test_app(columns);
        app.col_focus = 0; // pending

        handle_key(&mut app, key(KeyCode::Char('l')));
        // Should skip collapsed "running" and land on "done"
        assert_eq!(app.col_focus, 2);
    }

    #[test]
    fn focus_stays_when_all_collapsed() {
        let columns = vec![
            KanbanColumn::new("pending", vec![], None),
            KanbanColumn::new("running", vec![], None),
        ];
        let mut app = test_app(columns);
        app.col_focus = 0;

        handle_key(&mut app, key(KeyCode::Char('l')));
        assert_eq!(app.col_focus, 0);
    }

    #[test]
    fn arrow_keys_also_work() {
        let columns = vec![
            KanbanColumn::new("pending", vec![test_card("a", "pending")], None),
            KanbanColumn::new("running", vec![test_card("b", "running")], None),
        ];
        let mut app = test_app(columns);
        app.col_focus = 0;

        handle_key(&mut app, key(KeyCode::Right));
        assert_eq!(app.col_focus, 1);

        handle_key(&mut app, key(KeyCode::Left));
        assert_eq!(app.col_focus, 0);
    }

    // ── Card selection tests ────────────────────────────────────────────

    #[test]
    fn j_moves_selection_down() {
        let columns = vec![KanbanColumn::new(
            "pending",
            vec![
                test_card("a", "pending"),
                test_card("b", "pending"),
                test_card("c", "pending"),
            ],
            None,
        )];
        let mut app = test_app(columns);

        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.columns[0].list_state.selected(), Some(1));
    }

    #[test]
    fn k_moves_selection_up() {
        let columns = vec![KanbanColumn::new(
            "pending",
            vec![
                test_card("a", "pending"),
                test_card("b", "pending"),
                test_card("c", "pending"),
            ],
            None,
        )];
        let mut app = test_app(columns);
        app.columns[0].list_state.select(Some(2));

        handle_key(&mut app, key(KeyCode::Char('k')));
        assert_eq!(app.columns[0].list_state.selected(), Some(1));
    }

    #[test]
    fn j_wraps_from_last_to_first() {
        let columns = vec![KanbanColumn::new(
            "pending",
            vec![test_card("a", "pending"), test_card("b", "pending")],
            None,
        )];
        let mut app = test_app(columns);
        app.columns[0].list_state.select(Some(1));

        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.columns[0].list_state.selected(), Some(0));
    }

    #[test]
    fn k_wraps_from_first_to_last() {
        let columns = vec![KanbanColumn::new(
            "pending",
            vec![test_card("a", "pending"), test_card("b", "pending")],
            None,
        )];
        let mut app = test_app(columns);

        handle_key(&mut app, key(KeyCode::Char('k')));
        assert_eq!(app.columns[0].list_state.selected(), Some(1));
    }

    #[test]
    fn jk_noop_on_empty_column() {
        let columns = vec![KanbanColumn::new("pending", vec![], None)];
        let mut app = test_app(columns);

        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.columns[0].list_state.selected(), None);

        handle_key(&mut app, key(KeyCode::Char('k')));
        assert_eq!(app.columns[0].list_state.selected(), None);
    }

    #[test]
    fn down_up_arrows_also_work() {
        let columns = vec![KanbanColumn::new(
            "pending",
            vec![test_card("a", "pending"), test_card("b", "pending")],
            None,
        )];
        let mut app = test_app(columns);

        handle_key(&mut app, key(KeyCode::Down));
        assert_eq!(app.columns[0].list_state.selected(), Some(1));

        handle_key(&mut app, key(KeyCode::Up));
        assert_eq!(app.columns[0].list_state.selected(), Some(0));
    }

    // ── Mode dispatch tests ─────────────────────────────────────────────

    #[test]
    fn non_normal_modes_dont_navigate() {
        let columns = vec![
            KanbanColumn::new("pending", vec![test_card("a", "pending")], None),
            KanbanColumn::new("running", vec![test_card("b", "running")], None),
        ];
        let mut app = test_app(columns);
        app.mode = Mode::Filter;

        handle_key(&mut app, key(KeyCode::Char('l')));
        // Should not have moved focus
        assert_eq!(app.col_focus, 0);
    }

    // ── Detail mode tests ───────────────────────────────────────────────

    #[test]
    fn d_enters_detail_mode_when_card_selected() {
        let columns = vec![KanbanColumn::new(
            "pending",
            vec![test_card("a", "pending")],
            None,
        )];
        let mut app = test_app(columns);

        handle_key(&mut app, key(KeyCode::Char('d')));
        assert_eq!(app.mode, Mode::Detail);
        assert_eq!(app.detail_scroll, 0);
    }

    #[test]
    fn d_noop_when_no_card_selected() {
        // All columns empty — no card selected.
        let columns = vec![KanbanColumn::new("pending", vec![], None)];
        let mut app = test_app(columns);

        handle_key(&mut app, key(KeyCode::Char('d')));
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn esc_exits_detail_mode() {
        let columns = vec![KanbanColumn::new(
            "pending",
            vec![test_card("a", "pending")],
            None,
        )];
        let mut app = test_app(columns);
        app.mode = Mode::Detail;

        handle_key(&mut app, key(KeyCode::Esc));
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn j_scrolls_detail_down() {
        let columns = vec![KanbanColumn::new(
            "pending",
            vec![test_card("a", "pending")],
            None,
        )];
        let mut app = test_app(columns);
        app.mode = Mode::Detail;
        app.detail_scroll = 0;

        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.detail_scroll, 1);

        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.detail_scroll, 2);
    }

    #[test]
    fn k_scrolls_detail_up() {
        let columns = vec![KanbanColumn::new(
            "pending",
            vec![test_card("a", "pending")],
            None,
        )];
        let mut app = test_app(columns);
        app.mode = Mode::Detail;
        app.detail_scroll = 5;

        handle_key(&mut app, key(KeyCode::Char('k')));
        assert_eq!(app.detail_scroll, 4);
    }

    #[test]
    fn k_scroll_doesnt_underflow() {
        let columns = vec![KanbanColumn::new(
            "pending",
            vec![test_card("a", "pending")],
            None,
        )];
        let mut app = test_app(columns);
        app.mode = Mode::Detail;
        app.detail_scroll = 0;

        handle_key(&mut app, key(KeyCode::Char('k')));
        assert_eq!(app.detail_scroll, 0);
    }

    #[test]
    fn detail_mode_does_not_navigate_columns() {
        let columns = vec![
            KanbanColumn::new("pending", vec![test_card("a", "pending")], None),
            KanbanColumn::new("running", vec![test_card("b", "running")], None),
        ];
        let mut app = test_app(columns);
        app.mode = Mode::Detail;

        handle_key(&mut app, key(KeyCode::Char('l')));
        // Should NOT have moved column focus — 'l' is not bound in Detail mode.
        assert_eq!(app.col_focus, 0);
    }

    #[test]
    fn f2_toggles_factory_tab() {
        let columns = vec![KanbanColumn::new(
            "pending",
            vec![test_card("a", "pending")],
            None,
        )];
        let mut app = test_app(columns);
        assert_eq!(app.tab, AppTab::Kanban);

        handle_key(&mut app, key(KeyCode::F(2)));
        assert_eq!(app.tab, AppTab::Factory);

        handle_key(&mut app, key(KeyCode::F(2)));
        assert_eq!(app.tab, AppTab::Kanban);
    }

    #[test]
    fn esc_exits_factory_tab() {
        let columns = vec![KanbanColumn::new(
            "pending",
            vec![test_card("a", "pending")],
            None,
        )];
        let mut app = test_app(columns);
        app.switch_to_factory_tab();

        handle_key(&mut app, key(KeyCode::Esc));
        assert_eq!(app.tab, AppTab::Kanban);
    }
}
