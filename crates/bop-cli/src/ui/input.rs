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
/// Additional tab contexts and mode handlers are wired below.
use std::fs;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::app::{find_card_dir, App, AppTab, DetailTab, Mode};
use crate::cards;
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

    match &app.tab {
        AppTab::Factory => {
            handle_factory(app, key);
            return;
        }
        AppTab::Detail(_) => {
            handle_detail(app, key);
            return;
        }
        AppTab::Log(_) => {
            handle_log_tab(app, key);
            return;
        }
        AppTab::Kanban => {}
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
/// - `Enter` — open full-screen detail panel for selected card
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
            // Back-compat alias for opening detail panel.
            app.open_detail_for_selected();
        }
        KeyCode::Char('/') => {
            app.enter_filter_mode();
        }
        KeyCode::Enter => {
            app.open_detail_for_selected();
        }
        KeyCode::Char('a') => {
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
            // Shift+L: toggle integrated log pane for selected card.
            app.toggle_log_pane_for_selected();
        }
        KeyCode::Char('>') => {
            // Shift+.: move selected card right (toward merged).
            try_move_card(app, CardMoveDir::Right);
        }
        KeyCode::Tab => {
            // Tab multi-select (yazi pattern): toggle mark on hovered card.
            app.toggle_mark();
        }
        KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Ctrl+O: open card shell (zellij pane if available, shell fallback).
            app.prepare_subshell();
        }
        KeyCode::Char('!') => {
            // Back-compat alias for opening a card shell.
            app.prepare_subshell();
        }
        KeyCode::Esc => {
            // Esc clears filter and marks in Normal mode.
            if app.filter.is_some() || app.filter_state.is_some() {
                app.clear_filter();
            }
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

// ── Integrated Log tab ──────────────────────────────────────────────────────

/// Handle key events in integrated Log tab mode.
///
/// Keybindings:
/// - `k` / `Up` — scroll up one line and pause follow mode
/// - `j` / `Down` — scroll down one line (stays paused)
/// - `f` — resume following (jump to bottom)
/// - `Tab` — cycle to next running card log
/// - `L` / `Esc` — return to kanban tab
fn handle_log_tab(app: &mut App, key: KeyEvent) {
    app.status_message = None;

    match key.code {
        KeyCode::Char('k') | KeyCode::Up => {
            app.log_pane_scroll = app.log_pane_scroll.saturating_add(1);
            app.log_pane_follow = false;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            app.log_pane_scroll = app.log_pane_scroll.saturating_sub(1);
            app.log_pane_follow = false;
        }
        KeyCode::Char('f') => {
            app.log_pane_follow = true;
            app.log_pane_scroll = 0;
        }
        KeyCode::Tab => {
            app.cycle_log_pane_running_card();
        }
        KeyCode::Char('L') | KeyCode::Esc => {
            app.close_log_pane();
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
/// - `M` / `D` / `R` / `O` / `L` — switch detail tab
/// - `G` — jump to bottom of current tab
/// - `f` — toggle follow mode (Log tab only)
/// - `Esc` / `Enter` — return to kanban
fn handle_detail(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            app.detail_scroll = app.detail_scroll.saturating_add(1);
            if app.detail_tab == DetailTab::Log {
                app.detail_log_follow = false;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.detail_scroll = app.detail_scroll.saturating_sub(1);
            if app.detail_tab == DetailTab::Log {
                app.detail_log_follow = false;
            }
        }
        KeyCode::Char('G') => {
            app.detail_scroll = usize::MAX;
            if app.detail_tab == DetailTab::Log {
                app.detail_log_follow = false;
            }
        }
        KeyCode::Char('f') => {
            if app.detail_tab == DetailTab::Log {
                app.detail_log_follow = !app.detail_log_follow;
                if app.detail_log_follow {
                    app.detail_scroll = usize::MAX;
                }
            }
        }
        KeyCode::Char('m') | KeyCode::Char('M') => {
            app.detail_tab = DetailTab::Meta;
            app.detail_scroll = 0;
        }
        KeyCode::Char('d') | KeyCode::Char('D') => {
            app.detail_tab = DetailTab::Diff;
            app.detail_scroll = 0;
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            app.detail_tab = DetailTab::Replay;
            app.detail_scroll = 0;
        }
        KeyCode::Char('o') | KeyCode::Char('O') => {
            app.detail_tab = DetailTab::Output;
            app.detail_scroll = 0;
        }
        KeyCode::Char('l') | KeyCode::Char('L') => {
            app.detail_tab = DetailTab::Log;
            app.detail_scroll = 0;
            app.detail_log_follow = true;
        }
        KeyCode::Esc | KeyCode::Enter => {
            app.close_detail_panel();
        }
        _ => {}
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

// ── Card movement (H / Shift+.) ────────────────────────────────────────────

/// Direction for card state movement (`H` = left, `>` = right).
enum CardMoveDir {
    Left,
    Right,
}

/// Return the adjacent state in column order for manual movement.
fn adjacent_state(source_state: &str, dir: &CardMoveDir) -> Option<&'static str> {
    const ORDER: [&str; 5] = ["pending", "running", "done", "failed", "merged"];
    let idx = ORDER.iter().position(|s| *s == source_state)?;

    match dir {
        CardMoveDir::Left if idx > 0 => Some(ORDER[idx - 1]),
        CardMoveDir::Right if idx + 1 < ORDER.len() => Some(ORDER[idx + 1]),
        _ => None,
    }
}

/// Validate move against the card lifecycle graph.
fn is_valid_transition(source_state: &str, target_state: &str) -> bool {
    matches!(
        (source_state, target_state),
        ("pending", "running")
            | ("running", "done")
            | ("running", "failed")
            | ("done", "merged")
            | ("running", "pending")
            | ("done", "running")
            | ("failed", "running")
            | ("merged", "done")
    )
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

    // Manual move keys always target the adjacent rendered column.
    let target_state = match adjacent_state(&source_state, &dir) {
        Some(state) => state,
        None => {
            app.show_toast(format!(
                "invalid move: {} has no adjacent state",
                source_state
            ));
            return;
        }
    };

    // Enforce valid state machine transitions.
    if !is_valid_transition(&source_state, target_state) {
        app.show_toast(format!(
            "invalid move: {} -> {}",
            source_state, target_state
        ));
        return;
    }

    // Find the card's actual directory on disk.
    let card_path = match find_card_dir(&app.cards_root, &card_id) {
        Some(p) => p,
        None => {
            app.show_toast(format!("card not found: {}", card_id));
            return;
        }
    };

    // Compute target path preserving team directory structure.
    // card_path = .../state_dir/card-name.bop
    // state_dir.parent() = cards_root or cards_root/team-xxx
    let card_dir_name = match card_path.file_name() {
        Some(n) => n.to_os_string(),
        None => {
            app.show_toast("invalid card path");
            return;
        }
    };
    let state_dir = match card_path.parent() {
        Some(p) => p,
        None => {
            app.show_toast("invalid card path");
            return;
        }
    };
    let base_dir = match state_dir.parent() {
        Some(p) => p,
        None => {
            app.show_toast("invalid card path");
            return;
        }
    };

    let target_dir = base_dir.join(target_state);

    // Ensure target state directory exists.
    if fs::create_dir_all(&target_dir).is_err() {
        app.show_toast("failed to create target directory");
        return;
    }

    let target_path = target_dir.join(&card_dir_name);

    // Check if target already exists.
    if target_path.exists() {
        app.show_toast(format!("card already exists in {}", target_state));
        return;
    }

    // Perform the rename.
    match fs::rename(&card_path, &target_path) {
        Ok(()) => {
            // Refresh columns from disk.
            let new_columns = super::app::build_columns(&app.cards_root);
            app.refresh_columns(new_columns);
            app.toast_message = None;
            app.toast_deadline_tick = None;
            app.status_message = Some(format!("moved {} → {}", card_id, target_state));
        }
        Err(e) => {
            app.show_toast(format!("move failed: {}", e));
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
    use crate::ui::app::{AppTab, DetailTab, KanbanColumn};
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
            detail_tab: DetailTab::Meta,
            detail_log_follow: true,
            action_list_state: ListState::default(),
            log_scroll: 0,
            log_follow: false,
            log_pane_scroll: 0,
            log_pane_follow: true,
            log_tail_card_id: None,
            log_stdout_pos: 0,
            log_stderr_pos: 0,
            log_stdout_incomplete: String::new(),
            log_stderr_incomplete: String::new(),
            newcard_input: String::new(),
            status_message: None,
            toast_message: None,
            toast_deadline_tick: None,
            marked_cards: std::collections::HashSet::new(),
            subshell_worktree: None,
            subshell_card_id: None,
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
        assert_eq!(app.tab, AppTab::Detail("a".to_string()));
        assert_eq!(app.detail_tab, DetailTab::Meta);
        assert_eq!(app.detail_scroll, 0);
    }

    #[test]
    fn enter_enters_detail_mode_when_card_selected() {
        let columns = vec![KanbanColumn::new(
            "pending",
            vec![test_card("a", "pending")],
            None,
        )];
        let mut app = test_app(columns);

        handle_key(&mut app, key(KeyCode::Enter));
        assert_eq!(app.mode, Mode::Detail);
        assert_eq!(app.tab, AppTab::Detail("a".to_string()));
        assert_eq!(app.detail_tab, DetailTab::Meta);
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
        app.open_detail_for_card("a".to_string());

        handle_key(&mut app, key(KeyCode::Esc));
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.tab, AppTab::Kanban);
    }

    #[test]
    fn j_scrolls_detail_down() {
        let columns = vec![KanbanColumn::new(
            "pending",
            vec![test_card("a", "pending")],
            None,
        )];
        let mut app = test_app(columns);
        app.open_detail_for_card("a".to_string());
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
        app.open_detail_for_card("a".to_string());
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
        app.open_detail_for_card("a".to_string());
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
        app.open_detail_for_card("a".to_string());

        handle_key(&mut app, key(KeyCode::Char('l')));
        // Should NOT have moved column focus.
        assert_eq!(app.col_focus, 0);
        assert_eq!(app.detail_tab, DetailTab::Log);
    }

    #[test]
    fn detail_tab_keys_switch_tabs() {
        let columns = vec![KanbanColumn::new(
            "pending",
            vec![test_card("a", "pending")],
            None,
        )];
        let mut app = test_app(columns);
        app.open_detail_for_card("a".to_string());

        handle_key(&mut app, key(KeyCode::Char('D')));
        assert_eq!(app.detail_tab, DetailTab::Diff);

        handle_key(&mut app, key(KeyCode::Char('R')));
        assert_eq!(app.detail_tab, DetailTab::Replay);

        handle_key(&mut app, key(KeyCode::Char('O')));
        assert_eq!(app.detail_tab, DetailTab::Output);

        handle_key(&mut app, key(KeyCode::Char('M')));
        assert_eq!(app.detail_tab, DetailTab::Meta);
    }

    #[test]
    fn detail_log_follow_toggle() {
        let columns = vec![KanbanColumn::new(
            "pending",
            vec![test_card("a", "pending")],
            None,
        )];
        let mut app = test_app(columns);
        app.open_detail_for_card("a".to_string());

        handle_key(&mut app, key(KeyCode::Char('L')));
        assert_eq!(app.detail_tab, DetailTab::Log);
        assert!(app.detail_log_follow);

        handle_key(&mut app, key(KeyCode::Char('f')));
        assert!(!app.detail_log_follow);

        handle_key(&mut app, key(KeyCode::Char('f')));
        assert!(app.detail_log_follow);

        handle_key(&mut app, key(KeyCode::Char('G')));
        assert_eq!(app.detail_scroll, usize::MAX);
        assert!(!app.detail_log_follow);
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

    #[test]
    fn uppercase_l_toggles_integrated_log_tab() {
        let columns = vec![KanbanColumn::new(
            "running",
            vec![test_card("card-a", "running")],
            None,
        )];
        let mut app = test_app(columns);

        handle_key(&mut app, key(KeyCode::Char('L')));
        assert_eq!(app.tab, AppTab::Log("card-a".to_string()));

        handle_key(&mut app, key(KeyCode::Char('L')));
        assert_eq!(app.tab, AppTab::Kanban);
    }

    #[test]
    fn esc_closes_integrated_log_tab() {
        let columns = vec![KanbanColumn::new(
            "running",
            vec![test_card("card-a", "running")],
            None,
        )];
        let mut app = test_app(columns);
        app.open_log_pane_for_card("card-a".to_string());

        handle_key(&mut app, key(KeyCode::Esc));
        assert_eq!(app.tab, AppTab::Kanban);
    }

    #[test]
    fn k_pauses_follow_and_f_resumes() {
        let columns = vec![KanbanColumn::new(
            "running",
            vec![test_card("card-a", "running")],
            None,
        )];
        let mut app = test_app(columns);
        app.open_log_pane_for_card("card-a".to_string());

        handle_key(&mut app, key(KeyCode::Char('k')));
        assert!(!app.log_pane_follow);
        assert_eq!(app.log_pane_scroll, 1);

        handle_key(&mut app, key(KeyCode::Char('f')));
        assert!(app.log_pane_follow);
        assert_eq!(app.log_pane_scroll, 0);
    }

    #[test]
    fn tab_cycles_between_running_logs() {
        let columns = vec![
            KanbanColumn::new("pending", vec![test_card("p1", "pending")], None),
            KanbanColumn::new(
                "running",
                vec![test_card("run-a", "running"), test_card("run-b", "running")],
                None,
            ),
        ];
        let mut app = test_app(columns);
        app.open_log_pane_for_card("run-a".to_string());

        handle_key(&mut app, key(KeyCode::Tab));
        assert_eq!(app.tab, AppTab::Log("run-b".to_string()));

        handle_key(&mut app, key(KeyCode::Tab));
        assert_eq!(app.tab, AppTab::Log("run-a".to_string()));
    }
}
