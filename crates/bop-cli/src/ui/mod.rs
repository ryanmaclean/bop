pub mod app;
pub mod event;
pub mod factory_tab;
pub mod input;
pub mod log_pane;
pub mod widgets;
pub mod wizard;

use std::io::{self, Stdout};
use std::path::Path;

use crossterm::cursor;
use crossterm::event::{KeyCode, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Terminal;

use crate::paths;
use app::{App, AppEvent, AppTab, Mode};
use event::{EventLoop, TerminalGuard};
use factory_tab::FactoryTabWidget;
use log_pane::render_log_pane;
use widgets::{render_detail, render_footer, render_header, render_kanban, render_logtail};

/// Launch the interactive TUI kanban board.
///
/// Sets up the terminal guard (alternate screen + raw mode), initialises
/// the application state, spawns background event producers, and enters
/// the main event loop. On exit (q or Ctrl-C), the `TerminalGuard`
/// restores the terminal via its `Drop` impl.
pub async fn run_ui(root: &Path) -> anyhow::Result<()> {
    // Initialise application state from the filesystem.
    let mut app = App::new(root)?;

    // Enter alternate screen + raw mode (restored on Drop).
    let _guard = TerminalGuard::new()?;

    // Create ratatui terminal with crossterm backend.
    let backend = ratatui::backend::CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    // Spawn background tasks: crossterm reader, tick timer, fs watcher.
    let mut events = EventLoop::new(root);

    // ── Initial render ─────────────────────────────────────────────────
    draw(&mut terminal, &mut app)?;

    // ── Main event loop ────────────────────────────────────────────────
    loop {
        let Some(event) = events.next().await else {
            break; // all senders dropped — clean shutdown
        };

        match event {
            AppEvent::Key(key) => {
                // Quit on 'q' (Normal mode) or Ctrl-C (any mode).
                let ctrl_c =
                    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c');
                let quit = app.mode == Mode::Normal && key.code == KeyCode::Char('q');

                if ctrl_c || quit {
                    break;
                }

                // Dispatch remaining keys to mode-specific handler.
                input::handle_key(&mut app, key);

                // If handler requested a subshell, suspend TUI and spawn shell.
                if app.mode == Mode::Subshell {
                    // Leave alternate screen + disable raw mode.
                    let _ = execute!(io::stdout(), cursor::Show, LeaveAlternateScreen);
                    let _ = disable_raw_mode();

                    // Run the subshell synchronously (blocks until shell exits).
                    app.run_subshell();

                    // Re-enter alternate screen + raw mode.
                    let _ = enable_raw_mode();
                    let _ = execute!(io::stdout(), EnterAlternateScreen, cursor::Hide);

                    // Force full redraw by clearing the terminal's back buffer.
                    let _ = terminal.clear();
                }
            }
            AppEvent::Resize(w, h) => {
                app.on_resize(w, h);
            }
            AppEvent::Tick => {
                app.on_tick();
            }
            AppEvent::Cards(columns) => {
                app.refresh_columns(columns);
            }
            AppEvent::LogLine(line) => {
                app.push_log_line(line);
            }
        }

        // Re-render after every event.
        draw(&mut terminal, &mut app)?;
    }

    // _guard drops here → restores alternate screen + raw mode.
    Ok(())
}

/// Render the three-zone layout: header (2 rows), body (remaining), footer (1–2 rows).
///
/// Uses ratatui's `Layout::vertical` to divide the terminal into the
/// k9s-style three-zone layout. The body zone renders the kanban board
/// via [`render_kanban`]; the footer shows context-sensitive keybinding
/// hints via [`render_footer`]. When terminal height > 30, the footer
/// gets 2 rows to display the secondary F-key bar (mc-style).
///
/// When terminal height < 10, renders a minimal single-line status bar
/// instead of the full TUI to remain usable in very small terminals.
fn draw(
    terminal: &mut Terminal<ratatui::backend::CrosstermBackend<Stdout>>,
    app: &mut App,
) -> anyhow::Result<()> {
    terminal.draw(|frame| {
        let area = frame.area();
        let terminal_height = area.height;

        // Update stored dimensions from the actual frame area.
        app.terminal_width = area.width;
        app.terminal_height = terminal_height;

        // ── Minimal mode: single-line status when terminal too short ──
        if app.is_minimal_height() {
            render_minimal_status(frame, area, app);
            return;
        }

        // Footer gets 2 rows when terminal is tall enough for the F-key bar.
        let footer_height = if terminal_height > 30 { 2 } else { 1 };

        // Three-zone layout: header (2), body (fill), footer (1–2).
        let zones = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(0),
            Constraint::Length(footer_height),
        ])
        .split(area);

        // ── Header (2 rows) ────────────────────────────────────────────
        render_header(frame, zones[0], app);

        // ── Body (remaining space) ─────────────────────────────────────
        match &app.tab {
            AppTab::Factory => {
                frame.render_widget(FactoryTabWidget::new(&app.factory_tab), zones[1]);
            }
            AppTab::Log(card_id) => {
                render_log_pane(frame, zones[1], app, card_id);
            }
            AppTab::Kanban => {
                render_kanban(frame, zones[1], app);

                // ── Detail overlay (right 50% of body) ────────────────
                if app.mode == Mode::Detail {
                    if let Some(card) = app.selected_card().cloned() {
                        let card_dir = paths::find_card(&app.cards_root, &card.id);
                        render_detail(
                            frame,
                            zones[1],
                            &card,
                            app.detail_scroll,
                            card_dir.as_deref(),
                        );
                    }
                }

                // ── LogTail overlay (full body zone) ───────────────────
                if app.mode == Mode::LogTail {
                    if let Some(ref card_id) = app.log_tail_card_id.clone() {
                        render_logtail(
                            frame,
                            zones[1],
                            card_id,
                            &app.log_buf,
                            app.log_scroll,
                            app.log_follow,
                        );
                    }
                }
            }
        }

        // ── Footer (1–2 rows) ──────────────────────────────────────────
        render_footer(frame, zones[2], app, terminal_height);
    })?;

    Ok(())
}

/// Render a minimal single-line status bar for very short terminals (height < 10).
///
/// Displays: `bop · P:N R:N D:N F:N M:N · {mode}`
/// This ensures the TUI remains informative even in extremely constrained
/// terminal sizes (e.g. split panes, embedded terminal widgets).
fn render_minimal_status(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, app: &App) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    // Count cards per state.
    let counts: Vec<(&str, usize)> = app
        .columns
        .iter()
        .map(|c| {
            let label = match c.state.as_str() {
                "pending" => "P",
                "running" => "R",
                "done" => "D",
                "failed" => "F",
                "merged" => "M",
                _ => "?",
            };
            (label, c.cards.len())
        })
        .collect();

    let mut spans: Vec<Span<'static>> = vec![
        Span::styled("bop", Style::default().fg(Color::Cyan)),
        Span::styled(" · ", Style::default().fg(Color::DarkGray)),
    ];

    for (i, (label, count)) in counts.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" ", Style::default().fg(Color::DarkGray)));
        }
        spans.push(Span::styled(
            format!("{}:{}", label, count),
            Style::default().fg(Color::White),
        ));
    }

    let mode_str = match &app.tab {
        AppTab::Factory => "factory",
        AppTab::Log(_) => "log",
        AppTab::Kanban => match app.mode {
            Mode::Normal => "normal",
            Mode::Filter => "filter",
            Mode::ActionPopup => "action",
            Mode::Detail => "detail",
            Mode::LogTail => "logs",
            Mode::NewCard => "new",
            Mode::Subshell => "shell",
        },
    };

    spans.push(Span::styled(" · ", Style::default().fg(Color::DarkGray)));
    spans.push(Span::styled(
        mode_str.to_string(),
        Style::default().fg(Color::Yellow),
    ));

    let line = Line::from(spans);
    let status = Paragraph::new(line);

    // Render into just the first row.
    let status_area = ratatui::layout::Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 1,
    };
    frame.render_widget(status, status_area);
}

// ── Snapshot tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod snapshot_tests {
    use std::collections::{HashSet, VecDeque};

    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;

    use crate::render::CardView;
    use crate::ui::app::{App, AppTab, KanbanColumn, Mode};
    use crate::ui::factory_tab::FactoryTabState;
    use crate::ui::widgets::header::{ProviderMeter, ProviderStatus};
    use crate::ui::widgets::{
        render_detail, render_footer, render_header, render_kanban, render_logtail,
    };

    // ── Helpers ─────────────────────────────────────────────────────────

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

    /// Build a CardView with richer data (provider, elapsed, progress).
    fn rich_card(id: &str, state: &str, provider: &str, elapsed: u64, progress: u8) -> CardView {
        CardView {
            id: id.into(),
            state: state.into(),
            glyph: None,
            token: Some("♠A".into()),
            title: format!("Feature: {}", id),
            stage: "implement".into(),
            priority: Some(2),
            progress,
            provider: Some(provider.into()),
            elapsed_s: Some(elapsed),
            phase_name: if progress > 0 {
                Some("Phase 2".into())
            } else {
                None
            },
            phase_frac: 0.5,
            failure_reason: None,
            exit_code: None,
            ac_subtasks_done: None,
            ac_subtasks_total: None,
        }
    }

    /// Build a test App with mock columns — no filesystem access.
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
            cards_root: std::path::PathBuf::from("/tmp/test-cards"),
            provider_meters: Vec::new(),
            recent_completions: VecDeque::new(),
            tick_count: 0,
            prev_done_count: 0,
            detail_scroll: 0,
            action_list_state: ratatui::widgets::ListState::default(),
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
            terminal_width: 80,
            terminal_height: 24,
        }
    }

    /// Build default 5 columns with given card distributions.
    fn build_test_columns(
        pending: Vec<CardView>,
        running: Vec<CardView>,
        done: Vec<CardView>,
        failed: Vec<CardView>,
        merged: Vec<CardView>,
    ) -> Vec<KanbanColumn> {
        vec![
            KanbanColumn::new("pending", pending, None),
            KanbanColumn::new("running", running, Some(4)),
            KanbanColumn::new("done", done, None),
            KanbanColumn::new("failed", failed, None),
            KanbanColumn::new("merged", merged, None),
        ]
    }

    /// Extract text content from a ratatui Buffer as a multi-line string.
    ///
    /// Each row becomes one line. Trailing spaces are trimmed per-row to
    /// keep snapshots readable. Empty trailing rows are preserved to
    /// maintain dimensional fidelity.
    fn buffer_to_string(buf: &Buffer) -> String {
        let area = buf.area();
        let mut lines = Vec::with_capacity(area.height as usize);
        for y in area.y..area.y + area.height {
            let mut row = String::new();
            for x in area.x..area.x + area.width {
                let cell = &buf[(x, y)];
                row.push_str(cell.symbol());
            }
            lines.push(row.trim_end().to_string());
        }
        lines.join("\n")
    }

    // ── Kanban columns: normal view ─────────────────────────────────────

    #[test]
    fn snapshot_kanban_normal_view() {
        let columns = build_test_columns(
            vec![
                test_card("setup-deps", "pending"),
                test_card("add-tests", "pending"),
            ],
            vec![rich_card("build-ui", "running", "claude", 120, 67)],
            vec![test_card("init-repo", "done")],
            vec![],
            vec![],
        );
        let mut app = test_app(columns);
        app.terminal_width = 100;
        app.terminal_height = 20;

        let backend = TestBackend::new(100, 18);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_kanban(frame, frame.area(), &mut app);
            })
            .unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        insta::assert_snapshot!("kanban_normal_view", content);
    }

    // ── Kanban with collapsed empty columns ─────────────────────────────

    #[test]
    fn snapshot_kanban_collapsed_empty_columns() {
        let columns = build_test_columns(
            vec![test_card("card-a", "pending")],
            vec![],
            vec![],
            vec![],
            vec![],
        );
        let mut app = test_app(columns);
        app.terminal_width = 80;
        app.terminal_height = 16;

        let backend = TestBackend::new(80, 14);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_kanban(frame, frame.area(), &mut app);
            })
            .unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        insta::assert_snapshot!("kanban_collapsed_empty_columns", content);
    }

    // ── Detail overlay ──────────────────────────────────────────────────

    #[test]
    fn snapshot_detail_overlay() {
        let mut card = rich_card("build-ui", "running", "claude", 300, 75);
        card.priority = Some(1);

        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_detail(frame, area, &card, 0, None);
            })
            .unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        insta::assert_snapshot!("detail_overlay", content);
    }

    // ── Header with provider meters ─────────────────────────────────────

    #[test]
    fn snapshot_header_with_providers() {
        let columns = build_test_columns(
            vec![test_card("card-a", "pending")],
            vec![test_card("card-b", "running")],
            vec![],
            vec![],
            vec![],
        );
        let mut app = test_app(columns);
        app.terminal_width = 120;
        app.terminal_height = 24;
        app.provider_meters = vec![
            ProviderMeter {
                name: "claude".into(),
                status: ProviderStatus::Available,
            },
            ProviderMeter {
                name: "codex".into(),
                status: ProviderStatus::Busy,
            },
            ProviderMeter {
                name: "ollama".into(),
                status: ProviderStatus::RateLimited,
            },
        ];
        app.throughput = VecDeque::from(vec![0, 1, 2, 3, 4, 5, 6, 7]);

        let backend = TestBackend::new(120, 2);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_header(frame, frame.area(), &app);
            })
            .unwrap();

        // Replace the dynamic time portion with a fixed placeholder
        // to make snapshots deterministic.
        let content = buffer_to_string(terminal.backend().buffer());
        let stable = stabilize_time(&content);
        insta::assert_snapshot!("header_with_providers", stable);
    }

    // ── Footer for each mode ────────────────────────────────────────────

    #[test]
    fn snapshot_footer_normal_mode() {
        let columns = build_test_columns(vec![], vec![], vec![], vec![], vec![]);
        let app = test_app(columns);

        let backend = TestBackend::new(80, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_footer(frame, frame.area(), &app, 24);
            })
            .unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        insta::assert_snapshot!("footer_normal_mode", content);
    }

    #[test]
    fn snapshot_footer_detail_mode() {
        let columns = build_test_columns(vec![], vec![], vec![], vec![], vec![]);
        let mut app = test_app(columns);
        app.mode = Mode::Detail;

        let backend = TestBackend::new(80, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_footer(frame, frame.area(), &app, 24);
            })
            .unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        insta::assert_snapshot!("footer_detail_mode", content);
    }

    #[test]
    fn snapshot_footer_logtail_mode() {
        let columns = build_test_columns(vec![], vec![], vec![], vec![], vec![]);
        let mut app = test_app(columns);
        app.mode = Mode::LogTail;

        let backend = TestBackend::new(80, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_footer(frame, frame.area(), &app, 24);
            })
            .unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        insta::assert_snapshot!("footer_logtail_mode", content);
    }

    #[test]
    fn snapshot_footer_filter_mode() {
        let columns = build_test_columns(vec![], vec![], vec![], vec![], vec![]);
        let mut app = test_app(columns);
        app.mode = Mode::Filter;
        app.filter = Some("build".into());

        let backend = TestBackend::new(80, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_footer(frame, frame.area(), &app, 24);
            })
            .unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        insta::assert_snapshot!("footer_filter_mode", content);
    }

    #[test]
    fn snapshot_footer_action_popup_mode() {
        let columns = build_test_columns(vec![], vec![], vec![], vec![], vec![]);
        let mut app = test_app(columns);
        app.mode = Mode::ActionPopup;

        let backend = TestBackend::new(80, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_footer(frame, frame.area(), &app, 24);
            })
            .unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        insta::assert_snapshot!("footer_action_popup_mode", content);
    }

    #[test]
    fn snapshot_footer_newcard_mode() {
        let columns = build_test_columns(vec![], vec![], vec![], vec![], vec![]);
        let mut app = test_app(columns);
        app.mode = Mode::NewCard;
        app.newcard_input = "my-feature".into();

        let backend = TestBackend::new(80, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_footer(frame, frame.area(), &app, 24);
            })
            .unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        insta::assert_snapshot!("footer_newcard_mode", content);
    }

    #[test]
    fn snapshot_footer_with_fkey_bar() {
        let columns = build_test_columns(vec![], vec![], vec![], vec![], vec![]);
        let app = test_app(columns);

        let backend = TestBackend::new(80, 2);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                // terminal_height > 30 triggers F-key bar
                render_footer(frame, frame.area(), &app, 35);
            })
            .unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        insta::assert_snapshot!("footer_with_fkey_bar", content);
    }

    // ── LogTail overlay ─────────────────────────────────────────────────

    #[test]
    fn snapshot_logtail_overlay() {
        let mut log_buf = VecDeque::new();
        log_buf.push_back("INFO starting build".into());
        log_buf.push_back("DEBUG compiling crate".into());
        log_buf.push_back("WARN unused import".into());
        log_buf.push_back("ERROR build failed".into());
        log_buf.push_back("card-1 → done".into());

        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_logtail(frame, frame.area(), "build-ui", &log_buf, 0, true);
            })
            .unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        insta::assert_snapshot!("logtail_overlay", content);
    }

    // ── Filter with highlighted matches ─────────────────────────────────

    #[test]
    fn snapshot_kanban_with_filter() {
        let columns = build_test_columns(
            vec![
                test_card("setup-deps", "pending"),
                test_card("add-tests", "pending"),
                test_card("fix-build", "pending"),
            ],
            vec![test_card("build-ui", "running")],
            vec![],
            vec![],
            vec![],
        );
        let mut app = test_app(columns);
        app.terminal_width = 80;
        app.terminal_height = 16;
        app.filter = Some("build".into());
        app.enter_filter_mode();
        if let Some(ref mut fs) = app.filter_state {
            fs.query = "build".into();
        }
        app.apply_filter_collapse();

        let backend = TestBackend::new(80, 14);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_kanban(frame, frame.area(), &mut app);
            })
            .unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        insta::assert_snapshot!("kanban_with_filter", content);
    }

    // ── Minimal status bar (height < 10) ────────────────────────────────

    #[test]
    fn snapshot_minimal_status() {
        let columns = build_test_columns(
            vec![test_card("card-a", "pending")],
            vec![test_card("card-b", "running")],
            vec![test_card("card-c", "done"), test_card("card-d", "done")],
            vec![],
            vec![],
        );
        let mut app = test_app(columns);
        app.terminal_width = 60;
        app.terminal_height = 5;

        let backend = TestBackend::new(60, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                super::render_minimal_status(frame, area, &app);
            })
            .unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        insta::assert_snapshot!("minimal_status", content);
    }

    // ── Full TUI three-zone layout ──────────────────────────────────────

    #[test]
    fn snapshot_full_tui_layout() {
        let columns = build_test_columns(
            vec![
                test_card("setup-deps", "pending"),
                test_card("add-tests", "pending"),
            ],
            vec![rich_card("build-ui", "running", "claude", 120, 50)],
            vec![test_card("init-repo", "done")],
            vec![],
            vec![test_card("landed-pr", "merged")],
        );
        let mut app = test_app(columns);
        app.terminal_width = 100;
        app.terminal_height = 20;

        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                app.terminal_width = area.width;
                app.terminal_height = area.height;

                let footer_height = if area.height > 30 { 2 } else { 1 };
                let zones = ratatui::layout::Layout::vertical([
                    ratatui::layout::Constraint::Length(2),
                    ratatui::layout::Constraint::Min(0),
                    ratatui::layout::Constraint::Length(footer_height),
                ])
                .split(area);

                render_header(frame, zones[0], &app);
                render_kanban(frame, zones[1], &mut app);
                render_footer(frame, zones[2], &app, area.height);
            })
            .unwrap();

        // Stabilize the dynamic time portion.
        let content = buffer_to_string(terminal.backend().buffer());
        let stable = stabilize_time(&content);
        insta::assert_snapshot!("full_tui_layout", stable);
    }

    // ── Time stabilizer ─────────────────────────────────────────────────

    /// Replace HH:MM:SS time patterns with a fixed placeholder.
    ///
    /// The header renders `Local::now()` which makes snapshots flaky.
    /// This regex-free approach finds `NN:NN:NN` patterns and replaces them.
    fn stabilize_time(s: &str) -> String {
        let mut result = s.to_string();
        // Find patterns like "12:34:56" (exactly HH:MM:SS)
        let bytes = result.as_bytes();
        let mut i = 0;
        let mut replacements = Vec::new();
        while i + 7 < bytes.len() {
            if bytes[i].is_ascii_digit()
                && bytes[i + 1].is_ascii_digit()
                && bytes[i + 2] == b':'
                && bytes[i + 3].is_ascii_digit()
                && bytes[i + 4].is_ascii_digit()
                && bytes[i + 5] == b':'
                && bytes[i + 6].is_ascii_digit()
                && bytes[i + 7].is_ascii_digit()
            {
                replacements.push((i, i + 8));
                i += 8;
            } else {
                i += 1;
            }
        }
        // Apply replacements in reverse to preserve indices.
        for (start, end) in replacements.into_iter().rev() {
            result.replace_range(start..end, "HH:MM:SS");
        }
        result
    }
}
