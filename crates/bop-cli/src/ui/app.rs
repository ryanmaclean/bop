/// TUI application state and event types.
///
/// Defines the core data model for the kanban TUI: columns, card views,
/// navigation state, and the event enum that unifies keyboard input,
/// filesystem changes, and timer ticks into a single async stream.
use std::collections::{HashSet, VecDeque};
use std::fs;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::Utc;
use crossterm::event::KeyEvent;
use crossterm::terminal;
use ratatui::widgets::ListState;
use serde_json::Value;

use crate::paths;
use crate::providers::read_providers;
use crate::render::CardView;
use crate::ui::factory_tab::{FactoryTabState, FACTORY_REFRESH_TICKS};
use crate::ui::widgets::action::actions_for_state;
use crate::ui::widgets::filter::FilterState;
use crate::ui::widgets::header::{ProviderMeter, ProviderStatus, TICKER_CAPACITY};

// ── Events ──────────────────────────────────────────────────────────────────

/// Unified event type for the TUI event loop.
///
/// Background tasks (fs watcher, log reader, timer) send these through
/// `tokio::sync::mpsc::unbounded_channel`. The UI loop processes them
/// sequentially — never blocking the render thread.
#[derive(Debug)]
pub enum AppEvent {
    /// Keyboard input from crossterm.
    Key(KeyEvent),
    /// Terminal resize (cols, rows).
    Resize(u16, u16),
    /// 250ms heartbeat for clock, sparkline, and animation updates.
    Tick,
    /// Filesystem watcher detected card changes — full refresh payload.
    Cards(Vec<KanbanColumn>),
    /// New complete line from the selected card's log tail.
    #[allow(dead_code)]
    LogLine(String),
}

// ── Mode ────────────────────────────────────────────────────────────────────

/// Current interaction mode. Determines which keybindings are active
/// and which overlay (if any) is drawn on top of the kanban body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Default: column/card navigation, action keys active.
    Normal,
    /// `/` filter active — keystrokes go to nucleo query input.
    Filter,
    /// Enter key opened the action popup overlay.
    ActionPopup,
    /// Enter opened the card detail panel.
    Detail,
    /// Log tail overlay — full-height scrollable log view.
    LogTail,
    /// `n` inline card creation prompt.
    NewCard,
    /// `!` subshell — TUI suspended, shell active in card worktree.
    Subshell,
}

/// Active tab inside the detail panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailTab {
    Meta,
    Diff,
    Replay,
    Output,
    Log,
}

/// Top-level body tab selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppTab {
    /// Default kanban board body.
    Kanban,
    /// Factory services panel body.
    Factory,
    /// Full-screen card detail panel for a card ID.
    Detail(String),
    /// Integrated log pane for a card (`logs/stdout.log`).
    Log(String),
}

// ── KanbanColumn ────────────────────────────────────────────────────────────

/// Column state names in display order.
pub const COLUMN_STATES: &[&str] = &["pending", "running", "done", "failed", "merged"];

/// One vertical column in the kanban board.
///
/// Each column corresponds to a card state directory. The `list_state`
/// tracks the currently selected row within the column for ratatui's
/// `StatefulWidget` rendering.
#[derive(Debug, Clone)]
pub struct KanbanColumn {
    /// State name (e.g. "pending", "running").
    pub state: String,
    /// Cards currently in this state.
    pub cards: Vec<CardView>,
    /// Ratatui selection state for the list widget.
    pub list_state: ListState,
    /// WIP limit for this column (running reads `max_workers`/`max_concurrent`).
    pub wip_limit: Option<usize>,
    /// Auto-collapsed when card count is 0.
    pub collapsed: bool,
}

impl KanbanColumn {
    /// Create a new column for the given state with cards.
    pub(crate) fn new(state: &str, cards: Vec<CardView>, wip_limit: Option<usize>) -> Self {
        let collapsed = cards.is_empty();
        let mut list_state = ListState::default();
        if !cards.is_empty() {
            list_state.select(Some(0));
        }
        Self {
            state: state.to_string(),
            cards,
            list_state,
            wip_limit,
            collapsed,
        }
    }

    /// Returns true when the column is at or over its WIP limit.
    #[allow(dead_code)]
    pub fn is_at_wip_limit(&self) -> bool {
        match self.wip_limit {
            Some(limit) => self.cards.len() >= limit,
            None => false,
        }
    }

    /// WIP saturation fraction (0.0–1.0). Returns 0.0 when no limit is set.
    #[allow(dead_code)]
    pub fn wip_saturation(&self) -> f32 {
        match self.wip_limit {
            Some(limit) if limit > 0 => (self.cards.len() as f32 / limit as f32).min(1.0),
            _ => 0.0,
        }
    }
}

// ── App ─────────────────────────────────────────────────────────────────────

/// Top-level TUI application state.
///
/// Holds all columns, focus indices, mode, log buffer, and throughput
/// sparkline data. The render layer reads this immutably; event handlers
/// mutate it.
pub struct App {
    /// Kanban columns in display order (pending → running → done → failed → merged).
    pub columns: Vec<KanbanColumn>,
    /// Index into `columns` for the currently focused column.
    pub col_focus: usize,
    /// Active nucleo filter query (None = no filter).
    pub filter: Option<String>,
    /// Nucleo filter state — holds the matcher and query for fuzzy filtering.
    /// Present when filter mode is active or a confirmed filter is applied.
    pub filter_state: Option<FilterState>,
    /// Current interaction mode.
    pub mode: Mode,
    /// Active body tab (`Kanban`, `Factory`, `Detail(card_id)`, or `Log(card_id)`).
    pub tab: AppTab,
    /// Factory services panel state.
    pub factory_tab: FactoryTabState,
    /// Circular buffer of recent log lines (last 200) from the selected card.
    pub log_buf: VecDeque<String>,
    /// Throughput sparkline samples (last 8 values, cards/hr).
    pub throughput: VecDeque<u8>,
    /// Root path of the .cards directory.
    pub cards_root: PathBuf,
    /// Provider status meters for the header bar.
    pub provider_meters: Vec<ProviderMeter>,
    /// Recent card completions for the scrolling event ticker.
    pub recent_completions: VecDeque<String>,
    /// Monotonic tick counter for ticker scroll animation.
    pub tick_count: u64,
    /// Previous "done" card count for throughput delta tracking.
    pub prev_done_count: usize,
    /// Scroll offset for the Detail overlay (lines scrolled from top).
    pub detail_scroll: usize,
    /// Active detail sub-tab (`Meta`, `Diff`, `Replay`, `Output`, `Log`).
    pub detail_tab: DetailTab,
    /// Follow mode for detail Log tab.
    pub detail_log_follow: bool,
    /// Ratatui selection state for the ActionPopup list widget.
    pub action_list_state: ListState,
    /// Scroll offset for the LogTail overlay (lines scrolled from top).
    pub log_scroll: usize,
    /// Follow mode for LogTail — auto-scrolls to show latest lines.
    pub log_follow: bool,
    /// Scroll offset for the integrated Log tab (lines above bottom).
    pub log_pane_scroll: usize,
    /// Follow mode for the integrated Log tab.
    pub log_pane_follow: bool,
    /// Card ID currently being tailed in LogTail mode (None when not tailing).
    pub log_tail_card_id: Option<String>,
    /// File position in `logs/stdout.log` for incremental reading.
    pub(crate) log_stdout_pos: u64,
    /// File position in `logs/stderr.log` for incremental reading.
    pub(crate) log_stderr_pos: u64,
    /// Incomplete line buffer for stdout (newline-gated streaming).
    pub(crate) log_stdout_incomplete: String,
    /// Incomplete line buffer for stderr (newline-gated streaming).
    pub(crate) log_stderr_incomplete: String,
    /// Input buffer for NewCard mode — holds the card ID being typed.
    pub newcard_input: String,
    /// Transient status message shown in the footer (cleared on next keypress).
    pub status_message: Option<String>,
    /// Short-lived toast message (used for invalid move errors).
    pub toast_message: Option<String>,
    /// Tick deadline for clearing `toast_message`.
    pub toast_deadline_tick: Option<u64>,
    /// Set of card IDs marked for bulk operations (Tab multi-select, yazi pattern).
    pub marked_cards: HashSet<String>,
    /// Shell cwd for the pending subshell (set by `Ctrl+O`, consumed by event loop).
    pub subshell_worktree: Option<PathBuf>,
    /// Card ID for the pending subshell pane name.
    pub subshell_card_id: Option<String>,
    /// Current terminal width in columns (updated on resize events).
    pub terminal_width: u16,
    /// Current terminal height in rows (updated on resize events).
    pub terminal_height: u16,
}

/// Maximum number of log lines retained in the circular buffer.
const LOG_BUF_CAPACITY: usize = 200;
/// Read window for integrated log-pane tailing.
const LOG_PANE_READ_BYTES: i64 = 16384;
/// Refresh cadence for integrated log-pane updates (2 x 250ms = 500ms).
const LOG_PANE_REFRESH_TICKS: u64 = 2;

/// Number of throughput sparkline samples.
const THROUGHPUT_SAMPLES: usize = 8;
/// Toast visibility duration in ticks (8 * 250ms = 2s).
const TOAST_TTL_TICKS: u64 = 8;

impl App {
    /// Create a new App by scanning the filesystem for cards.
    ///
    /// Reads cards from the `.cards` directory using the same pattern as
    /// `list.rs::collect_card_views`. Reads the running WIP limit from
    /// config (`max_workers`/`max_concurrent`, default 3).
    pub fn new(cards_root: &Path) -> anyhow::Result<Self> {
        let columns = build_columns(cards_root);

        // Focus the first non-collapsed column, or 0 if all are collapsed.
        let col_focus = columns.iter().position(|c| !c.collapsed).unwrap_or(0);

        let done_count = columns
            .iter()
            .find(|c| c.state == "done")
            .map(|c| c.cards.len())
            .unwrap_or(0);

        // Query initial terminal size; default to 80×24 if unavailable.
        let (term_w, term_h) = terminal::size().unwrap_or((80, 24));

        let mut app = App {
            columns,
            col_focus,
            filter: None,
            filter_state: None,
            mode: Mode::Normal,
            tab: AppTab::Kanban,
            factory_tab: FactoryTabState::new(),
            log_buf: VecDeque::with_capacity(LOG_BUF_CAPACITY),
            throughput: VecDeque::from(vec![0u8; THROUGHPUT_SAMPLES]),
            cards_root: cards_root.to_path_buf(),
            provider_meters: Vec::new(),
            recent_completions: VecDeque::with_capacity(TICKER_CAPACITY),
            tick_count: 0,
            prev_done_count: done_count,
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
            marked_cards: HashSet::new(),
            subshell_worktree: None,
            subshell_card_id: None,
            terminal_width: term_w,
            terminal_height: term_h,
        };

        // Seed provider meters from providers.json.
        app.refresh_provider_meters();
        // Seed factory panel status/log snapshot.
        app.refresh_factory_tab();

        Ok(app)
    }

    /// Toggle between kanban and factory tabs.
    pub fn toggle_tab(&mut self) {
        match &self.tab {
            AppTab::Factory => self.switch_to_kanban_tab(),
            _ => self.switch_to_factory_tab(),
        }
    }

    /// Switch to the factory panel tab.
    pub fn switch_to_factory_tab(&mut self) {
        self.tab = AppTab::Factory;
        // Factory tab always uses its own keymap; close overlays.
        self.mode = Mode::Normal;
        self.refresh_factory_tab();
    }

    /// Switch back to the kanban tab.
    pub fn switch_to_kanban_tab(&mut self) {
        self.tab = AppTab::Kanban;
        // Return to base mode when leaving factory panel.
        self.mode = Mode::Normal;
    }

    /// Return the currently focused column, if any.
    pub fn focused_column(&self) -> Option<&KanbanColumn> {
        self.columns.get(self.col_focus)
    }

    /// Return a mutable reference to the currently focused column.
    pub fn focused_column_mut(&mut self) -> Option<&mut KanbanColumn> {
        self.columns.get_mut(self.col_focus)
    }

    /// Return the currently selected card in the focused column.
    pub fn selected_card(&self) -> Option<&CardView> {
        let col = self.focused_column()?;
        let idx = col.list_state.selected()?;
        col.cards.get(idx)
    }

    /// Return the card ID when the detail panel is active.
    pub fn detail_card_id(&self) -> Option<&str> {
        match &self.tab {
            AppTab::Detail(card_id) => Some(card_id.as_str()),
            _ => None,
        }
    }

    /// Return a card by ID from any column.
    pub fn card_by_id(&self, card_id: &str) -> Option<&CardView> {
        self.columns
            .iter()
            .flat_map(|column| column.cards.iter())
            .find(|card| card.id == card_id)
    }

    /// Focus and select a card by ID, returning true if found.
    pub fn select_card_by_id(&mut self, card_id: &str) -> bool {
        for (col_idx, column) in self.columns.iter_mut().enumerate() {
            if let Some(row_idx) = column.cards.iter().position(|card| card.id == card_id) {
                self.col_focus = col_idx;
                column.list_state.select(Some(row_idx));
                return true;
            }
        }
        false
    }

    /// Total card count across all columns.
    #[allow(dead_code)]
    pub fn total_cards(&self) -> usize {
        self.columns.iter().map(|c| c.cards.len()).sum()
    }

    /// Open the action popup for the currently selected card.
    ///
    /// Initialises `action_list_state` with the first item selected
    /// (if actions are available) and switches to `Mode::ActionPopup`.
    /// No-op if no card is selected.
    pub fn open_action_popup(&mut self) {
        if let Some(card) = self.selected_card() {
            let actions = actions_for_state(&card.state);
            let mut state = ListState::default();
            if !actions.is_empty() {
                state.select(Some(0));
            }
            self.action_list_state = state;
            self.mode = Mode::ActionPopup;
        }
    }

    /// Open detail panel for the currently selected card.
    pub fn open_detail_for_selected(&mut self) {
        let Some(card) = self.selected_card() else {
            return;
        };
        self.open_detail_for_card(card.id.clone());
    }

    /// Open detail panel for a specific card.
    pub fn open_detail_for_card(&mut self, card_id: String) {
        self.tab = AppTab::Detail(card_id);
        self.mode = Mode::Detail;
        self.detail_tab = DetailTab::Meta;
        self.detail_scroll = 0;
        self.detail_log_follow = true;
    }

    /// Close detail panel and restore kanban selection on the same card.
    pub fn close_detail_panel(&mut self) {
        let card_id = self.detail_card_id().map(str::to_owned);
        self.tab = AppTab::Kanban;
        self.mode = Mode::Normal;
        if let Some(card_id) = card_id {
            let _ = self.select_card_by_id(&card_id);
        }
    }

    /// Enter filter mode: initialises the [`FilterState`] and switches to
    /// `Mode::Filter`. If a confirmed filter was previously active, its query
    /// is used as the starting text.
    pub fn enter_filter_mode(&mut self) {
        let mut state = FilterState::new();
        if let Some(ref existing) = self.filter {
            state.query = existing.clone();
        }
        self.filter_state = Some(state);
        self.mode = Mode::Filter;
    }

    /// Confirm the current filter and return to Normal mode.
    ///
    /// The filter query is persisted in `self.filter` so that the kanban
    /// widget continues to apply it. If the query is empty, the filter is
    /// cleared entirely.
    pub fn confirm_filter(&mut self) {
        if let Some(ref state) = self.filter_state {
            if state.query.is_empty() {
                self.filter = None;
                self.filter_state = None;
            } else {
                self.filter = Some(state.query.clone());
                // Keep filter_state alive so kanban can use it for matching.
            }
        }
        self.mode = Mode::Normal;
        self.apply_filter_collapse();
    }

    /// Clear the filter and return to Normal mode — restores full columns.
    pub fn clear_filter(&mut self) {
        self.filter = None;
        self.filter_state = None;
        self.mode = Mode::Normal;
        // Restore collapse state based on card counts (not filter).
        self.update_collapse();
    }

    /// Show a toast message for 2 seconds.
    pub fn show_toast(&mut self, message: impl Into<String>) {
        self.toast_message = Some(message.into());
        self.toast_deadline_tick = Some(self.tick_count.wrapping_add(TOAST_TTL_TICKS));
    }

    // ── Multi-select (Tab / yazi pattern) ──────────────────────────────

    /// Toggle the mark on the currently selected card.
    ///
    /// If the card is already marked, unmarks it; otherwise marks it.
    /// Used by `Tab` in Normal mode (yazi-style multi-select).
    pub fn toggle_mark(&mut self) {
        if let Some(card) = self.selected_card() {
            let id = card.id.clone();
            if !self.marked_cards.remove(&id) {
                self.marked_cards.insert(id);
            }
        }
    }

    /// Clear all marks, returning to single-card selection.
    pub fn clear_marks(&mut self) {
        self.marked_cards.clear();
    }

    /// Returns the card IDs targeted by the current action.
    ///
    /// If cards are marked, returns the marked set. Otherwise returns
    /// just the selected card's ID (if any). Used by ActionPopup to
    /// support bulk operations.
    pub fn action_target_ids(&self) -> Vec<String> {
        if !self.marked_cards.is_empty() {
            self.marked_cards.iter().cloned().collect()
        } else if let Some(card) = self.selected_card() {
            vec![card.id.clone()]
        } else {
            Vec::new()
        }
    }

    /// Apply filter-based column collapsing.
    ///
    /// Columns where no cards match the active filter are collapsed.
    /// Called after filter confirm and on each keystroke in filter mode.
    pub fn apply_filter_collapse(&mut self) {
        let Some(ref mut state) = self.filter_state else {
            self.update_collapse();
            return;
        };

        for col in &mut self.columns {
            let has_match = col
                .cards
                .iter()
                .any(|card| state.matches(&card.id, &card.title).is_some());
            col.collapsed = col.cards.is_empty() || !has_match;
        }
    }

    /// Push a log line into the circular buffer, evicting the oldest if full.
    pub fn push_log_line(&mut self, line: String) {
        if self.log_buf.len() >= LOG_BUF_CAPACITY {
            self.log_buf.pop_front();
        }
        self.log_buf.push_back(line);
    }

    /// Push a throughput sample, evicting the oldest.
    #[allow(dead_code)]
    pub fn push_throughput(&mut self, value: u8) {
        if self.throughput.len() >= THROUGHPUT_SAMPLES {
            self.throughput.pop_front();
        }
        self.throughput.push_back(value);
    }

    /// Handle a tick event: advance animation counter, refresh provider
    /// meters, track throughput deltas, and poll log files in LogTail mode.
    pub fn on_tick(&mut self) {
        self.tick_count = self.tick_count.wrapping_add(1);
        if self
            .toast_deadline_tick
            .is_some_and(|deadline| self.tick_count >= deadline)
        {
            self.toast_message = None;
            self.toast_deadline_tick = None;
        }
        self.refresh_provider_meters();

        // Factory tab refreshes at 2s cadence.
        if self.tab == AppTab::Factory && self.tick_count.is_multiple_of(FACTORY_REFRESH_TICKS) {
            self.refresh_factory_tab();
        }

        // Integrated Log tab refreshes every 500ms.
        if matches!(self.tab, AppTab::Log(_))
            && self.tick_count.is_multiple_of(LOG_PANE_REFRESH_TICKS)
        {
            self.refresh_log_pane();
        }

        // Poll log files when in LogTail mode (250ms interval via tick).
        if self.mode == Mode::LogTail {
            self.poll_log_files();
        }
    }

    /// Refresh factory status rows and selected log tail.
    pub fn refresh_factory_tab(&mut self) {
        self.factory_tab.refresh();
    }

    /// Toggle integrated Log tab for the currently selected card.
    pub fn toggle_log_pane_for_selected(&mut self) {
        if matches!(self.tab, AppTab::Log(_)) {
            self.switch_to_kanban_tab();
            return;
        }

        let Some(card) = self.selected_card() else {
            return;
        };

        self.open_log_pane_for_card(card.id.clone());
    }

    /// Return the card ID when the integrated Log tab is active.
    pub fn log_tab_card_id(&self) -> Option<&str> {
        match &self.tab {
            AppTab::Log(card_id) => Some(card_id.as_str()),
            _ => None,
        }
    }

    /// Open integrated Log tab for a specific card ID.
    pub fn open_log_pane_for_card(&mut self, card_id: String) {
        self.tab = AppTab::Log(card_id);
        self.mode = Mode::Normal;
        self.log_pane_scroll = 0;
        self.log_pane_follow = true;
        self.refresh_log_pane();
    }

    /// Close integrated Log tab and return to kanban.
    pub fn close_log_pane(&mut self) {
        if matches!(self.tab, AppTab::Log(_)) {
            self.switch_to_kanban_tab();
        }
    }

    /// Cycle to the next running card's integrated log.
    pub fn cycle_log_pane_running_card(&mut self) {
        let running_ids: Vec<String> = self
            .columns
            .iter()
            .find(|column| column.state == "running")
            .map(|column| column.cards.iter().map(|card| card.id.clone()).collect())
            .unwrap_or_default();

        if running_ids.is_empty() {
            return;
        }

        let current_idx = self
            .log_tab_card_id()
            .and_then(|current| running_ids.iter().position(|id| id == current));
        let next_idx = current_idx
            .map(|idx| (idx + 1) % running_ids.len())
            .unwrap_or(0);
        self.open_log_pane_for_card(running_ids[next_idx].clone());
    }

    /// Refresh integrated log-pane buffer from `logs/stdout.log`.
    pub fn refresh_log_pane(&mut self) {
        let Some(card_id) = self.log_tab_card_id() else {
            return;
        };
        let Some(card_dir) = paths::find_card(&self.cards_root, card_id) else {
            self.log_buf.clear();
            return;
        };

        let stdout_path = card_dir.join("logs").join("stdout.log");
        let lines = read_log_tail(&stdout_path, LOG_PANE_READ_BYTES, LOG_BUF_CAPACITY);
        self.log_buf = lines.into();
    }

    /// Enter LogTail mode for the currently selected card.
    ///
    /// Reads existing log content (last 200 lines from `logs/stdout.log`
    /// and `logs/stderr.log`), records file positions for incremental
    /// polling, and switches to `Mode::LogTail` with follow mode enabled.
    /// No-op if no card is selected.
    pub fn enter_log_tail(&mut self) {
        let card_id = match self.selected_card() {
            Some(c) => c.id.clone(),
            None => return,
        };

        // Reset LogTail state.
        self.log_buf.clear();
        self.log_scroll = 0;
        self.log_follow = true;
        self.log_stdout_pos = 0;
        self.log_stderr_pos = 0;
        self.log_stdout_incomplete.clear();
        self.log_stderr_incomplete.clear();
        self.log_tail_card_id = Some(card_id.clone());

        // Load existing log content from the card's log files.
        if let Some(dir) = paths::find_card(&self.cards_root, &card_id) {
            let stdout_path = dir.join("logs").join("stdout.log");
            let stderr_path = dir.join("logs").join("stderr.log");

            let mut all_lines = Vec::new();

            if let Ok(content) = fs::read_to_string(&stdout_path) {
                for line in content.lines() {
                    all_lines.push(line.to_string());
                }
                if let Ok(meta) = fs::metadata(&stdout_path) {
                    self.log_stdout_pos = meta.len();
                }
            }

            if let Ok(content) = fs::read_to_string(&stderr_path) {
                for line in content.lines() {
                    all_lines.push(line.to_string());
                }
                if let Ok(meta) = fs::metadata(&stderr_path) {
                    self.log_stderr_pos = meta.len();
                }
            }

            // Take last LOG_BUF_CAPACITY lines.
            let start = all_lines.len().saturating_sub(LOG_BUF_CAPACITY);
            for line in &all_lines[start..] {
                self.log_buf.push_back(line.clone());
            }
        }

        self.mode = Mode::LogTail;
    }

    /// Exit LogTail mode, clearing tailing state and returning to Normal.
    pub fn exit_log_tail(&mut self) {
        self.log_tail_card_id = None;
        self.log_stdout_incomplete.clear();
        self.log_stderr_incomplete.clear();
        self.mode = Mode::Normal;
    }

    /// Poll log files for new content (called on each tick in LogTail mode).
    ///
    /// Reads new bytes from the card's `logs/stdout.log` and
    /// `logs/stderr.log` since the last recorded position. Uses newline-
    /// gated streaming: bytes are buffered and only complete lines
    /// (ending in `\n`) are emitted to `log_buf` to prevent partial
    /// ANSI escape sequence flicker.
    fn poll_log_files(&mut self) {
        let card_id = match &self.log_tail_card_id {
            Some(id) => id.clone(),
            None => return,
        };

        let card_dir = match paths::find_card(&self.cards_root, &card_id) {
            Some(dir) => dir,
            None => return,
        };

        let stdout_path = card_dir.join("logs").join("stdout.log");
        let stderr_path = card_dir.join("logs").join("stderr.log");

        // Take ownership of state to avoid borrow conflicts with push_log_line.
        let mut stdout_pos = self.log_stdout_pos;
        let mut stderr_pos = self.log_stderr_pos;
        let mut stdout_incomplete = std::mem::take(&mut self.log_stdout_incomplete);
        let mut stderr_incomplete = std::mem::take(&mut self.log_stderr_incomplete);

        let stdout_lines = read_log_chunk(&stdout_path, &mut stdout_pos, &mut stdout_incomplete);
        let stderr_lines = read_log_chunk(&stderr_path, &mut stderr_pos, &mut stderr_incomplete);

        // Restore state.
        self.log_stdout_pos = stdout_pos;
        self.log_stderr_pos = stderr_pos;
        self.log_stdout_incomplete = stdout_incomplete;
        self.log_stderr_incomplete = stderr_incomplete;

        for line in stdout_lines {
            self.push_log_line(line);
        }
        for line in stderr_lines {
            self.push_log_line(line);
        }
    }

    /// Refresh provider meters by reading `providers.json` and cross-
    /// referencing with running cards to determine status.
    pub fn refresh_provider_meters(&mut self) {
        let pf = match read_providers(&self.cards_root) {
            Ok(pf) => pf,
            Err(_) => return, // silently skip if providers.json is missing/invalid
        };

        let now = Utc::now().timestamp();

        // Collect provider names from running cards.
        let running_providers: Vec<String> = self
            .columns
            .iter()
            .filter(|c| c.state == "running")
            .flat_map(|c| c.cards.iter())
            .filter_map(|card| card.provider.clone())
            .collect();

        self.provider_meters = pf
            .providers
            .iter()
            .map(|(name, provider)| {
                let status = if provider
                    .cooldown_until_epoch_s
                    .is_some_and(|until| until > now)
                {
                    ProviderStatus::RateLimited
                } else if running_providers.iter().any(|rp| rp == name) {
                    ProviderStatus::Busy
                } else {
                    ProviderStatus::Available
                };

                ProviderMeter {
                    name: name.clone(),
                    status,
                }
            })
            .collect();
    }

    /// Track completions when columns are refreshed: detect new "done" cards
    /// and push them to the recent_completions ticker + update throughput.
    fn track_completions(&mut self) {
        let done_count = self
            .columns
            .iter()
            .find(|c| c.state == "done")
            .map(|c| c.cards.len())
            .unwrap_or(0);

        if done_count > self.prev_done_count {
            let delta = done_count - self.prev_done_count;
            // Push new completion entries to the ticker.
            if let Some(done_col) = self.columns.iter().find(|c| c.state == "done") {
                // Take the last `delta` cards as new completions.
                for card in done_col.cards.iter().rev().take(delta) {
                    let entry = format!("✓ {}", card.title);
                    if self.recent_completions.len() >= TICKER_CAPACITY {
                        self.recent_completions.pop_front();
                    }
                    self.recent_completions.push_back(entry);
                }
            }
        }
        self.prev_done_count = done_count;
    }

    /// Replace all column data (e.g. after a filesystem change notification).
    ///
    /// Preserves the selected row index in each column where possible:
    /// if the old column had a selection at index N and the new column has
    /// at least N+1 cards, the selection is restored. Otherwise, the
    /// selection is clamped to the last card (or cleared if empty).
    pub fn refresh_columns(&mut self, mut new_columns: Vec<KanbanColumn>) {
        // Preserve selection state where possible.
        for new_col in &mut new_columns {
            if let Some(old_col) = self.columns.iter().find(|c| c.state == new_col.state) {
                if let Some(old_idx) = old_col.list_state.selected() {
                    if new_col.cards.is_empty() {
                        new_col.list_state.select(None);
                    } else {
                        let clamped = old_idx.min(new_col.cards.len() - 1);
                        new_col.list_state.select(Some(clamped));
                    }
                }
            }
        }
        self.columns = new_columns;

        // Update collapsed state for all columns.
        self.update_collapse();

        // Clamp col_focus if columns shrunk (shouldn't happen with fixed states).
        if self.col_focus >= self.columns.len() {
            self.col_focus = self.columns.len().saturating_sub(1);
        }

        // If the focused column is now collapsed, find the nearest non-collapsed.
        if self.col_focus < self.columns.len() && self.columns[self.col_focus].collapsed {
            if let Some(idx) = self.columns.iter().position(|c| !c.collapsed) {
                self.col_focus = idx;
            }
        }

        // Keep selection anchored to the detail card while panel is open.
        if let Some(card_id) = self.detail_card_id().map(str::to_owned) {
            let _ = self.select_card_by_id(&card_id);
        }

        // Detect new completions for the event ticker.
        self.track_completions();
    }

    // ── Subshell (`Ctrl+O`) ───────────────────────────────────────────

    /// Prepare to enter a subshell for the currently selected card.
    ///
    /// Uses `<card>/worktree/` when present, otherwise falls back to the
    /// card directory itself. The selected directory is stored for the
    /// event loop to consume in `run_subshell()`.
    pub fn prepare_subshell(&mut self) {
        let card = match self.selected_card() {
            Some(c) => c.clone(),
            None => return,
        };

        let card_dir = match find_card_dir(&self.cards_root, &card.id) {
            Some(p) => p,
            None => {
                self.status_message = Some(format!("card not found: {}", card.id));
                return;
            }
        };

        let worktree_dir = card_dir.join("worktree");
        let shell_cwd = if worktree_dir.is_dir() {
            worktree_dir
        } else {
            card_dir
        };

        self.subshell_worktree = Some(shell_cwd);
        self.subshell_card_id = Some(card.id);
        self.mode = Mode::Subshell;
    }

    /// Run the subshell synchronously in the selected card directory.
    ///
    /// If already inside Zellij, opens a new pane via:
    /// `zellij run --name <card-id> -- $SHELL` with cwd set to the card path.
    /// Otherwise falls back to spawning `$SHELL` directly and waiting for exit.
    pub fn run_subshell(&mut self) {
        let worktree = match self.subshell_worktree.take() {
            Some(p) => p,
            None => {
                self.mode = Mode::Normal;
                return;
            }
        };
        let card_id = self
            .subshell_card_id
            .take()
            .unwrap_or_else(|| "card".to_string());

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());

        if std::env::var_os("ZELLIJ").is_some() {
            let status = Command::new("zellij")
                .arg("run")
                .arg("--name")
                .arg(&card_id)
                .arg("--")
                .arg(&shell)
                .current_dir(&worktree)
                .status();

            if matches!(status, Ok(s) if s.success()) {
                self.mode = Mode::Normal;
                return;
            }
        }

        eprintln!("\x1b[1m[bop] shell in: {}\x1b[0m", worktree.display());
        eprintln!("\x1b[2m(exit to return to bop ui)\x1b[0m");
        let _ = Command::new(&shell).current_dir(&worktree).status();

        self.mode = Mode::Normal;
    }

    /// Update the `collapsed` flag on all columns.
    ///
    /// A column is collapsed when it has zero cards. This is called after
    /// every `refresh_columns()` and can be called after any mutation that
    /// might change card counts (e.g. card movement via Shift+H/L).
    pub fn update_collapse(&mut self) {
        for col in &mut self.columns {
            col.collapsed = col.cards.is_empty();
        }
    }

    // ── Resize handling ─────────────────────────────────────────────────

    /// Handle a terminal resize event.
    ///
    /// Stores the new dimensions. Layout recalculation happens at render
    /// time via `visible_columns()` — ratatui handles the cell-level diff.
    pub fn on_resize(&mut self, width: u16, height: u16) {
        self.terminal_width = width;
        self.terminal_height = height;
    }

    /// Returns true when the terminal is too short for the full TUI layout.
    ///
    /// When height < 10, only a single-line minimal status should be shown
    /// instead of the full three-zone kanban layout.
    pub fn is_minimal_height(&self) -> bool {
        self.terminal_height < 10
    }

    /// Compute the set of column indices that should be visible given the
    /// current terminal width.
    ///
    /// Each non-collapsed column needs at least `MIN_COL_WIDTH` chars.
    /// Collapsed columns need `COLLAPSED_COL_WIDTH` chars. When the total
    /// required width exceeds the terminal, lowest-priority columns are
    /// hidden first: merged, then done.
    pub fn visible_column_indices(&self) -> Vec<usize> {
        /// Minimum usable width for a non-collapsed column (border + content).
        const MIN_COL_WIDTH: u16 = 15;
        /// Width of a collapsed column glyph divider.
        const COLLAPSED_COL_WIDTH: u16 = 3;
        /// Column states hidden first when terminal is too narrow, in order.
        const HIDE_PRIORITY: &[&str] = &["merged", "done"];

        let width = self.terminal_width;

        // Start with all columns visible.
        let mut visible: Vec<usize> = (0..self.columns.len()).collect();

        // Iteratively hide lowest-priority columns until layout fits.
        for &hide_state in HIDE_PRIORITY {
            let total = required_width(&self.columns, &visible, MIN_COL_WIDTH, COLLAPSED_COL_WIDTH);
            if total <= width {
                break;
            }
            // Remove the column matching hide_state from visible.
            visible.retain(|&i| self.columns[i].state != hide_state);
        }

        visible
    }
}

// ── Column builder (shared with event.rs fs watcher) ────────────────────────

/// Build all kanban columns by reading the filesystem.
///
/// Used by both `App::new()` and the filesystem watcher in `event.rs`
/// to construct a fresh set of columns from the current card state on disk.
pub(crate) fn build_columns(cards_root: &Path) -> Vec<KanbanColumn> {
    let wip_limit = read_wip_limit(cards_root);
    COLUMN_STATES
        .iter()
        .map(|&state| {
            let cards = collect_card_views(cards_root, state).unwrap_or_default();
            let limit = if state == "running" {
                Some(wip_limit)
            } else {
                None
            };
            KanbanColumn::new(state, cards, limit)
        })
        .collect()
}

/// Locate a card directory by id in both root and `team-*` state trees.
pub(crate) fn find_card_dir(cards_root: &Path, card_id: &str) -> Option<PathBuf> {
    if let Some(path) = paths::find_card(cards_root, card_id) {
        return Some(path);
    }

    if let Ok(entries) = fs::read_dir(cards_root) {
        for entry in entries.flatten() {
            let team_path = entry.path();
            if !team_path.is_dir() || !entry.file_name().to_string_lossy().starts_with("team-") {
                continue;
            }
            for state in COLUMN_STATES {
                let state_dir = team_path.join(state);
                if let Some(path) = paths::find_card_in_dir(&state_dir, card_id) {
                    return Some(path);
                }
            }
        }
    }
    None
}

// ── Filesystem helpers (mirroring list.rs patterns) ─────────────────────────

/// Collect [`CardView`] structs from a single state directory.
///
/// Mirrors the pattern from `list.rs::collect_card_views` — scans both
/// root-level and team-* directories for `.bop` / `.jobcard` card bundles.
pub(crate) fn collect_card_views(root: &Path, state: &str) -> anyhow::Result<Vec<CardView>> {
    let mut cards = Vec::new();

    // Root-level state directory
    collect_state_cards(root, state, &mut cards)?;

    // Team-* directories
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let team_path = entry.path();
            if team_path.is_dir() && entry.file_name().to_string_lossy().starts_with("team-") {
                collect_state_cards(&team_path, state, &mut cards)?;
            }
        }
    }

    Ok(cards)
}

/// Scan a single directory's state subdirectory for card bundles.
fn collect_state_cards(dir: &Path, state: &str, cards: &mut Vec<CardView>) -> anyhow::Result<()> {
    let state_dir = dir.join(state);
    if !state_dir.exists() {
        return Ok(());
    }

    let Ok(entries) = fs::read_dir(&state_dir) else {
        return Ok(());
    };

    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() && p.extension().is_some_and(|e| e == "bop" || e == "jobcard") {
            if let Ok(meta) = bop_core::read_meta(&p) {
                cards.push(crate::render::from_meta(&meta, state));
            }
        }
    }

    Ok(())
}

/// Calculate the minimum required width for the given visible columns.
///
/// Non-collapsed columns need at least `min_col_w` chars each; collapsed
/// columns need `collapsed_w` chars. This is used by `visible_column_indices`
/// to determine which columns to hide when the terminal is too narrow.
fn required_width(
    columns: &[KanbanColumn],
    visible: &[usize],
    min_col_w: u16,
    collapsed_w: u16,
) -> u16 {
    visible
        .iter()
        .map(|&i| {
            if columns[i].collapsed {
                collapsed_w
            } else {
                min_col_w
            }
        })
        .sum()
}

/// Read the running-column WIP limit.
///
/// Precedence:
/// 1. `<cards_root>/.bop/config.json` (`max_workers` or `max_concurrent`)
/// 2. `<cards_root>/../.bop/config.json` (`max_workers` or `max_concurrent`)
/// 3. `~/.bop/config.json` (`max_workers` or `max_concurrent`)
/// 4. default `3`
fn read_wip_limit(cards_root: &Path) -> usize {
    const DEFAULT_WIP_LIMIT: usize = 3;

    let cards_local = cards_root.join(".bop").join("config.json");
    if let Some(limit) = read_wip_limit_from_json(&cards_local) {
        return limit;
    }

    if let Some(project_config) = cards_root
        .parent()
        .map(|p| p.join(".bop").join("config.json"))
    {
        if let Some(limit) = read_wip_limit_from_json(&project_config) {
            return limit;
        }
    }

    if let Some(global_path) = bop_core::config::global_config_path() {
        if let Some(limit) = read_wip_limit_from_json(&global_path) {
            return limit;
        }
    }

    DEFAULT_WIP_LIMIT
}

fn read_wip_limit_from_json(path: &Path) -> Option<usize> {
    let json = fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&json).ok()?;

    let max_workers = value
        .get("max_workers")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);
    let max_concurrent = value
        .get("max_concurrent")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);

    max_workers.or(max_concurrent)
}

/// Read new bytes from a log file and extract complete lines.
///
/// Implements newline-gated streaming: bytes are appended to the
/// `incomplete` buffer and only complete lines (ending in `\n`) are
/// extracted and returned. Partial lines remain in the buffer for the
/// next poll cycle. This prevents partial ANSI escape sequence flicker
/// in the TUI.
///
/// Handles file truncation (e.g. log rotation) by resetting the position
/// and clearing the incomplete buffer.
fn read_log_chunk(path: &Path, pos: &mut u64, incomplete: &mut String) -> Vec<String> {
    let Ok(mut file) = fs::File::open(path) else {
        return Vec::new();
    };

    let Ok(metadata) = file.metadata() else {
        return Vec::new();
    };

    let file_len = metadata.len();

    // Handle file truncation (e.g. log rotation).
    if file_len < *pos {
        *pos = 0;
        incomplete.clear();
    }

    // No new data.
    if file_len <= *pos {
        return Vec::new();
    }

    if file.seek(SeekFrom::Start(*pos)).is_err() {
        return Vec::new();
    }

    let to_read = (file_len - *pos) as usize;
    let mut buf = vec![0u8; to_read];
    let n = match file.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return Vec::new(),
    };
    buf.truncate(n);
    *pos += n as u64;

    // Newline-gated streaming: append to incomplete buffer, extract complete lines.
    let chunk = String::from_utf8_lossy(&buf);
    incomplete.push_str(&chunk);

    let mut lines = Vec::new();
    while let Some(newline_pos) = incomplete.find('\n') {
        let line = incomplete[..newline_pos].to_string();
        *incomplete = incomplete[newline_pos + 1..].to_string();
        lines.push(line);
    }

    lines
}

/// Read the tail of a log file using a bounded seek-from-end window.
///
/// Uses the same `BufReader + SeekFrom::End` pattern as `factory_tab.rs`.
/// Returns at most `max_lines` from the end of the file.
fn read_log_tail(path: &Path, read_bytes: i64, max_lines: usize) -> Vec<String> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return Vec::new(),
    };

    let mut reader = BufReader::new(file);
    let file_len = match reader.get_ref().metadata() {
        Ok(meta) => meta.len(),
        Err(_) => return Vec::new(),
    };

    let seek_result = if file_len > read_bytes as u64 {
        reader.seek(SeekFrom::End(-read_bytes))
    } else {
        reader.seek(SeekFrom::Start(0))
    };
    if seek_result.is_err() {
        return Vec::new();
    }

    let mut chunk = String::new();
    if reader.read_to_string(&mut chunk).is_err() {
        return Vec::new();
    }

    let mut lines: Vec<String> = chunk.lines().map(str::to_owned).collect();
    if lines.len() > max_lines {
        let split_at = lines.len() - max_lines;
        lines = lines.split_off(split_at);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn setup_card_in_state(root: &Path, state: &str, id: &str) {
        let card_dir = root.join(state).join(format!("{}.bop", id));
        fs::create_dir_all(&card_dir).unwrap();
        let meta = bop_core::Meta {
            id: id.into(),
            stage: "implement".into(),
            ..Default::default()
        };
        bop_core::write_meta(&card_dir, &meta).unwrap();
    }

    #[test]
    fn app_new_empty_workspace() {
        let td = tempdir().unwrap();
        let app = App::new(td.path()).unwrap();
        assert_eq!(app.columns.len(), COLUMN_STATES.len());
        assert_eq!(app.total_cards(), 0);
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.filter.is_none());
    }

    #[test]
    fn app_new_loads_cards() {
        let td = tempdir().unwrap();
        setup_card_in_state(td.path(), "pending", "card-a");
        setup_card_in_state(td.path(), "pending", "card-b");
        setup_card_in_state(td.path(), "running", "card-c");
        setup_card_in_state(td.path(), "done", "card-d");

        let app = App::new(td.path()).unwrap();
        assert_eq!(app.total_cards(), 4);

        let pending = app.columns.iter().find(|c| c.state == "pending").unwrap();
        assert_eq!(pending.cards.len(), 2);
        assert!(!pending.collapsed);

        let running = app.columns.iter().find(|c| c.state == "running").unwrap();
        assert_eq!(running.cards.len(), 1);

        let failed = app.columns.iter().find(|c| c.state == "failed").unwrap();
        assert_eq!(failed.cards.len(), 0);
        assert!(failed.collapsed);
    }

    #[test]
    fn app_new_focuses_first_nonempty_column() {
        let td = tempdir().unwrap();
        // Only "running" has cards — it's index 1 in COLUMN_STATES
        setup_card_in_state(td.path(), "running", "card-a");

        let app = App::new(td.path()).unwrap();
        assert_eq!(app.col_focus, 1);
    }

    #[test]
    fn kanban_column_wip_limit() {
        let col = KanbanColumn::new("running", vec![], Some(4));
        assert!(!col.is_at_wip_limit());
        assert_eq!(col.wip_saturation(), 0.0);
    }

    #[test]
    fn kanban_column_wip_at_limit() {
        let td = tempdir().unwrap();
        setup_card_in_state(td.path(), "running", "a");
        setup_card_in_state(td.path(), "running", "b");
        let cards = collect_card_views(td.path(), "running").unwrap();
        let col = KanbanColumn::new("running", cards, Some(2));
        assert!(col.is_at_wip_limit());
        assert_eq!(col.wip_saturation(), 1.0);
    }

    #[test]
    fn push_log_line_respects_capacity() {
        let td = tempdir().unwrap();
        let mut app = App::new(td.path()).unwrap();
        for i in 0..250 {
            app.push_log_line(format!("line {}", i));
        }
        assert_eq!(app.log_buf.len(), LOG_BUF_CAPACITY);
        // Oldest lines should have been evicted
        assert_eq!(app.log_buf.front().unwrap(), "line 50");
    }

    #[test]
    fn push_throughput_respects_capacity() {
        let td = tempdir().unwrap();
        let mut app = App::new(td.path()).unwrap();
        for i in 0..16u8 {
            app.push_throughput(i);
        }
        assert_eq!(app.throughput.len(), THROUGHPUT_SAMPLES);
        assert_eq!(*app.throughput.back().unwrap(), 15);
    }

    #[test]
    fn selected_card_returns_none_on_empty() {
        let td = tempdir().unwrap();
        let app = App::new(td.path()).unwrap();
        assert!(app.selected_card().is_none());
    }

    #[test]
    fn selected_card_returns_card() {
        let td = tempdir().unwrap();
        setup_card_in_state(td.path(), "pending", "my-card");
        let app = App::new(td.path()).unwrap();
        let card = app.selected_card().unwrap();
        assert_eq!(card.id, "my-card");
    }

    #[test]
    fn mode_enum_variants() {
        // Verify all modes are distinct.
        let modes = [
            Mode::Normal,
            Mode::Filter,
            Mode::ActionPopup,
            Mode::Detail,
            Mode::LogTail,
            Mode::NewCard,
            Mode::Subshell,
        ];
        for (i, a) in modes.iter().enumerate() {
            for (j, b) in modes.iter().enumerate() {
                if i == j {
                    assert_eq!(a, b);
                } else {
                    assert_ne!(a, b);
                }
            }
        }
    }

    #[test]
    fn collect_card_views_includes_team_dirs() {
        let td = tempdir().unwrap();
        setup_card_in_state(td.path(), "pending", "root-card");

        let team_root = td.path().join("team-alpha");
        fs::create_dir_all(&team_root).unwrap();
        setup_card_in_state(&team_root, "pending", "team-card");

        let cards = collect_card_views(td.path(), "pending").unwrap();
        assert_eq!(cards.len(), 2);
    }

    #[test]
    fn collect_card_views_empty_state() {
        let td = tempdir().unwrap();
        let cards = collect_card_views(td.path(), "running").unwrap();
        assert!(cards.is_empty());
    }

    #[test]
    fn read_wip_limit_from_config() {
        let td = tempdir().unwrap();
        let bop_dir = td.path().join(".bop");
        fs::create_dir_all(&bop_dir).unwrap();
        let cfg = bop_core::config::Config {
            max_concurrent: Some(4),
            ..Default::default()
        };
        bop_core::config::write_config_file(&bop_dir.join("config.json"), &cfg).unwrap();

        // cards_root is <project>/.cards, so parent is <project>
        let cards_root = td.path().join(".cards");
        fs::create_dir_all(&cards_root).unwrap();
        let limit = read_wip_limit(&cards_root);
        assert_eq!(limit, 4);
    }

    #[test]
    fn read_wip_limit_defaults_to_three_when_missing() {
        let td = tempdir().unwrap();
        let limit = read_wip_limit(td.path());
        assert_eq!(limit, 3);
    }

    #[test]
    fn read_wip_limit_prefers_cards_local_max_workers() {
        let td = tempdir().unwrap();
        let cards_root = td.path().join(".cards");
        let cards_bop_dir = cards_root.join(".bop");
        let project_bop_dir = td.path().join(".bop");
        fs::create_dir_all(&cards_bop_dir).unwrap();
        fs::create_dir_all(&project_bop_dir).unwrap();

        fs::write(
            cards_bop_dir.join("config.json"),
            r#"{"max_workers": 7, "max_concurrent": 2}"#,
        )
        .unwrap();
        fs::write(project_bop_dir.join("config.json"), r#"{"max_workers": 4}"#).unwrap();

        let limit = read_wip_limit(&cards_root);
        assert_eq!(limit, 7);
    }

    // ── read_log_chunk (newline-gated streaming) ─────────────────────

    #[test]
    fn read_log_chunk_extracts_complete_lines() {
        let td = tempdir().unwrap();
        let log_path = td.path().join("test.log");
        fs::write(&log_path, "line one\nline two\n").unwrap();

        let mut pos = 0u64;
        let mut incomplete = String::new();
        let lines = read_log_chunk(&log_path, &mut pos, &mut incomplete);

        assert_eq!(lines, vec!["line one", "line two"]);
        assert!(incomplete.is_empty());
        assert_eq!(pos, 18); // "line one\nline two\n" = 18 bytes
    }

    #[test]
    fn read_log_chunk_buffers_incomplete_lines() {
        let td = tempdir().unwrap();
        let log_path = td.path().join("test.log");
        fs::write(&log_path, "complete\npartial").unwrap();

        let mut pos = 0u64;
        let mut incomplete = String::new();
        let lines = read_log_chunk(&log_path, &mut pos, &mut incomplete);

        assert_eq!(lines, vec!["complete"]);
        assert_eq!(incomplete, "partial");
    }

    #[test]
    fn read_log_chunk_completes_buffered_line_on_next_read() {
        let td = tempdir().unwrap();
        let log_path = td.path().join("test.log");
        fs::write(&log_path, "hello ").unwrap();

        let mut pos = 0u64;
        let mut incomplete = String::new();

        // First read: "hello " — no newline, stays in buffer.
        let lines = read_log_chunk(&log_path, &mut pos, &mut incomplete);
        assert!(lines.is_empty());
        assert_eq!(incomplete, "hello ");

        // Append "world\n" to the file.
        use std::io::Write;
        let mut f = fs::OpenOptions::new().append(true).open(&log_path).unwrap();
        f.write_all(b"world\n").unwrap();
        drop(f);

        // Second read: completes "hello world".
        let lines = read_log_chunk(&log_path, &mut pos, &mut incomplete);
        assert_eq!(lines, vec!["hello world"]);
        assert!(incomplete.is_empty());
    }

    #[test]
    fn read_log_chunk_handles_missing_file() {
        let td = tempdir().unwrap();
        let log_path = td.path().join("nonexistent.log");

        let mut pos = 0u64;
        let mut incomplete = String::new();
        let lines = read_log_chunk(&log_path, &mut pos, &mut incomplete);

        assert!(lines.is_empty());
    }

    #[test]
    fn read_log_chunk_handles_file_truncation() {
        let td = tempdir().unwrap();
        let log_path = td.path().join("test.log");
        fs::write(&log_path, "long content here\n").unwrap();

        let mut pos = 0u64;
        let mut incomplete = String::new();
        let _ = read_log_chunk(&log_path, &mut pos, &mut incomplete);
        assert!(pos > 0);

        // Truncate the file (simulate log rotation).
        fs::write(&log_path, "new\n").unwrap();

        let lines = read_log_chunk(&log_path, &mut pos, &mut incomplete);
        assert_eq!(lines, vec!["new"]);
        assert_eq!(pos, 4);
    }

    #[test]
    fn read_log_chunk_no_new_data() {
        let td = tempdir().unwrap();
        let log_path = td.path().join("test.log");
        fs::write(&log_path, "done\n").unwrap();

        let mut pos = 0u64;
        let mut incomplete = String::new();
        let _ = read_log_chunk(&log_path, &mut pos, &mut incomplete);

        // Second read with no new data.
        let lines = read_log_chunk(&log_path, &mut pos, &mut incomplete);
        assert!(lines.is_empty());
    }

    // ── Resize handling ─────────────────────────────────────────────────

    #[test]
    fn on_resize_stores_dimensions() {
        let td = tempdir().unwrap();
        let mut app = App::new(td.path()).unwrap();
        app.on_resize(120, 40);
        assert_eq!(app.terminal_width, 120);
        assert_eq!(app.terminal_height, 40);
    }

    #[test]
    fn is_minimal_height_below_10() {
        let td = tempdir().unwrap();
        let mut app = App::new(td.path()).unwrap();
        app.on_resize(80, 9);
        assert!(app.is_minimal_height());
    }

    #[test]
    fn is_minimal_height_at_10() {
        let td = tempdir().unwrap();
        let mut app = App::new(td.path()).unwrap();
        app.on_resize(80, 10);
        assert!(!app.is_minimal_height());
    }

    #[test]
    fn visible_columns_all_fit_wide_terminal() {
        let td = tempdir().unwrap();
        setup_card_in_state(td.path(), "pending", "card-a");
        setup_card_in_state(td.path(), "running", "card-b");
        let mut app = App::new(td.path()).unwrap();
        app.on_resize(200, 40);
        let visible = app.visible_column_indices();
        // All 5 columns should be visible.
        assert_eq!(visible.len(), COLUMN_STATES.len());
    }

    #[test]
    fn visible_columns_hides_merged_first() {
        let td = tempdir().unwrap();
        setup_card_in_state(td.path(), "pending", "card-a");
        setup_card_in_state(td.path(), "running", "card-b");
        setup_card_in_state(td.path(), "done", "card-c");
        setup_card_in_state(td.path(), "merged", "card-d");
        let mut app = App::new(td.path()).unwrap();
        // 4 non-collapsed columns × 15 = 60, 1 collapsed (failed) × 3 = 3 → 63 needed
        // Set to 50 — not enough for all, should hide merged first.
        app.on_resize(50, 40);
        let visible = app.visible_column_indices();
        let visible_states: Vec<&str> = visible
            .iter()
            .map(|&i| app.columns[i].state.as_str())
            .collect();
        assert!(!visible_states.contains(&"merged"));
    }

    #[test]
    fn visible_columns_hides_done_after_merged() {
        let td = tempdir().unwrap();
        setup_card_in_state(td.path(), "pending", "card-a");
        setup_card_in_state(td.path(), "running", "card-b");
        setup_card_in_state(td.path(), "done", "card-c");
        setup_card_in_state(td.path(), "merged", "card-d");
        let mut app = App::new(td.path()).unwrap();
        // After hiding merged: 3 non-collapsed × 15 + 1 collapsed (failed) × 3 = 48
        // Set to 35 — must also hide done.
        app.on_resize(35, 40);
        let visible = app.visible_column_indices();
        let visible_states: Vec<&str> = visible
            .iter()
            .map(|&i| app.columns[i].state.as_str())
            .collect();
        assert!(!visible_states.contains(&"merged"));
        assert!(!visible_states.contains(&"done"));
    }

    #[test]
    fn required_width_counts_correctly() {
        let td = tempdir().unwrap();
        setup_card_in_state(td.path(), "running", "card-a");
        setup_card_in_state(td.path(), "running", "card-b");
        let app = App::new(td.path()).unwrap();
        // "running" has 2 cards (non-collapsed=15), others are empty (collapsed=3 each).
        let visible = vec![0, 1, 2, 3, 4]; // all 5 columns
        let width = required_width(&app.columns, &visible, 15, 3);
        // 1 non-collapsed(15) + 4 collapsed(3×4=12) = 27
        assert_eq!(width, 27);
    }
}
