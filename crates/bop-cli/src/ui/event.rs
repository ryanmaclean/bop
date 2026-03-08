/// TUI event loop and terminal lifecycle management.
///
/// Provides the [`TerminalGuard`] RAII wrapper (alternate screen + raw mode)
/// and [`EventLoop`] that funnels crossterm keys, filesystem watcher
/// notifications, and a 250ms timer tick into a single
/// `tokio::sync::mpsc::unbounded_channel`.
use std::io::stdout;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::cursor;
use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use tokio::sync::mpsc;

use super::app::{build_columns, AppEvent, COLUMN_STATES};

// ── TerminalGuard ──────────────────────────────────────────────────────────

/// RAII guard that enters the alternate screen and enables raw mode on
/// creation, and restores the terminal on `Drop`.
///
/// This ensures clean terminal restoration even on panic — the guard's
/// destructor runs during stack unwinding. A panic hook is installed as
/// an additional safety net for `panic = abort` profiles.
pub struct TerminalGuard {
    _private: (), // prevent construction outside `new()`
}

impl TerminalGuard {
    /// Enter alternate screen, enable raw mode, and hide the cursor.
    ///
    /// Also installs a panic hook that performs terminal cleanup before
    /// the default handler runs, so the user's terminal is never left
    /// in a broken state.
    pub fn new() -> anyhow::Result<Self> {
        // Install panic hook for terminal cleanup (safety net).
        let original_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // Best-effort cleanup before the default handler.
            let _ = execute!(stdout(), cursor::Show, LeaveAlternateScreen);
            let _ = disable_raw_mode();
            original_hook(info);
        }));

        enable_raw_mode()?;
        execute!(stdout(), EnterAlternateScreen, cursor::Hide)?;
        Ok(Self { _private: () })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        // Best-effort cleanup — ignore errors during Drop.
        let _ = execute!(stdout(), cursor::Show, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

// ── EventLoop ──────────────────────────────────────────────────────────────

/// Encapsulates the three background event producers.
///
/// Call [`EventLoop::new`] to spawn the background tasks, then use
/// [`EventLoop::next`] to receive events in the main render loop.
///
/// # Background tasks
///
/// 1. **Crossterm reader** — polls terminal events on a dedicated thread,
///    converts `Key` and `Resize` events to [`AppEvent`] variants.
/// 2. **Tick timer** — sends [`AppEvent::Tick`] every 250ms via
///    `tokio::time::interval`.
/// 3. **Filesystem watcher** — watches `.cards/` state directories for
///    changes, re-reads all cards and sends [`AppEvent::Cards`] with
///    fresh column data.
pub struct EventLoop {
    rx: mpsc::UnboundedReceiver<AppEvent>,
}

impl EventLoop {
    /// Spawn the three background tasks and return the event loop handle.
    pub fn new(cards_root: &Path) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();

        // ── 1. Crossterm key/resize reader (blocking → dedicated thread) ──
        let key_tx = tx.clone();
        std::thread::spawn(move || {
            loop {
                // Poll with 50ms timeout so the thread exits promptly
                // when the receiver half is dropped (channel closed).
                match event::poll(Duration::from_millis(50)) {
                    Ok(true) => match event::read() {
                        Ok(Event::Key(key)) => {
                            if key_tx.send(AppEvent::Key(key)).is_err() {
                                return; // receiver dropped — exit
                            }
                        }
                        Ok(Event::Resize(w, h)) => {
                            if key_tx.send(AppEvent::Resize(w, h)).is_err() {
                                return;
                            }
                        }
                        Ok(_) => {} // mouse, focus, paste — ignored
                        Err(_) => return,
                    },
                    Ok(false) => {}   // timeout — loop and check again
                    Err(_) => return, // terminal gone
                }
            }
        });

        // ── 2. Tick timer (250ms) ─────────────────────────────────────────
        let tick_tx = tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(250));
            loop {
                interval.tick().await;
                if tick_tx.send(AppEvent::Tick).is_err() {
                    return; // receiver dropped
                }
            }
        });

        // ── 3. Filesystem watcher on .cards/ state dirs ───────────────────
        let watch_tx = tx;
        let root = cards_root.to_path_buf();
        spawn_fs_watcher(root, watch_tx);

        Self { rx }
    }

    /// Receive the next event from any background producer.
    ///
    /// Returns `None` when all senders have been dropped (shutdown).
    pub async fn next(&mut self) -> Option<AppEvent> {
        self.rx.recv().await
    }
}

// ── Filesystem watcher ─────────────────────────────────────────────────────

/// Spawn the notify filesystem watcher on a dedicated thread.
///
/// Watches all state directories under `cards_root` (pending, running,
/// done, failed, merged) plus any `team-*` subdirectories. On change,
/// re-reads all cards via [`build_columns`] and sends `AppEvent::Cards`.
///
/// Mirrors the watcher pattern from `list.rs::list_cards_watch`:
/// `std::thread::spawn` + `notify_debouncer_mini` + `std::sync::mpsc`
/// bridge into the tokio channel.
fn spawn_fs_watcher(cards_root: PathBuf, tx: mpsc::UnboundedSender<AppEvent>) {
    std::thread::spawn(move || {
        let (std_tx, std_rx) = std::sync::mpsc::channel();
        let mut debouncer = match new_debouncer(Duration::from_millis(200), std_tx) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("[bop ui] failed to create fs watcher: {}", e);
                return;
            }
        };

        // Watch each existing state dir (root-level and team-*)
        let watch_dirs = collect_watch_dirs(&cards_root);
        for dir in &watch_dirs {
            if let Err(e) = debouncer.watcher().watch(dir, RecursiveMode::Recursive) {
                eprintln!("[bop ui] failed to watch {}: {}", dir.display(), e);
            }
        }

        // Also watch the root itself for new state dirs being created.
        if let Err(e) = debouncer
            .watcher()
            .watch(&cards_root, RecursiveMode::NonRecursive)
        {
            eprintln!(
                "[bop ui] failed to watch root {}: {}",
                cards_root.display(),
                e
            );
        }

        for res in std_rx {
            match res {
                Ok(events) => {
                    let relevant = events.iter().any(|e| {
                        matches!(
                            e.kind,
                            DebouncedEventKind::Any | DebouncedEventKind::AnyContinuous
                        )
                    });
                    if !relevant {
                        continue;
                    }

                    // Re-read all columns from the filesystem.
                    let columns = build_columns(&cards_root);
                    if tx.send(AppEvent::Cards(columns)).is_err() {
                        return; // receiver dropped
                    }
                }
                Err(e) => {
                    eprintln!("[bop ui] fs watch error: {}", e);
                }
            }
        }
    });
}

/// Collect all state directories to watch under `cards_root`.
///
/// Includes root-level state dirs and team-* subdirectory state dirs,
/// matching the same discovery pattern used in `list.rs`.
fn collect_watch_dirs(cards_root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    // Root-level state directories
    for &state in COLUMN_STATES {
        let dir = cards_root.join(state);
        if dir.exists() {
            dirs.push(dir);
        }
    }

    // Team-* subdirectories
    if let Ok(entries) = std::fs::read_dir(cards_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && entry.file_name().to_string_lossy().starts_with("team-") {
                for &state in COLUMN_STATES {
                    let dir = path.join(state);
                    if dir.exists() {
                        dirs.push(dir);
                    }
                }
            }
        }
    }

    dirs
}
