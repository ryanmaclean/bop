use std::cmp::Ordering;
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::time::Duration;

use chrono::{DateTime, Local, NaiveDateTime, Utc};
use crossterm::cursor;
use crossterm::event::{self, Event, KeyCode};
use crossterm::execute;
use crossterm::terminal::{
    self, disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
    LeaveAlternateScreen,
};

use crate::project;

const REFRESH_MS: u64 = 500;
const LOG_LINES: usize = 8;
const LOG_READ_BYTES: i64 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CardState {
    Running,
    Done,
}

#[derive(Debug, Clone)]
struct WatchSource {
    project_name: Option<String>,
    cards_root: PathBuf,
}

#[derive(Debug, Clone)]
struct CardEntry {
    key: String,
    id: String,
    project_name: Option<String>,
    provider: String,
    state: CardState,
    card_dir: PathBuf,
    log_path: PathBuf,
    elapsed_s: Option<u64>,
    cost_usd: Option<f64>,
    sort_ts: Option<DateTime<Utc>>,
}

#[derive(Debug, Default)]
struct Snapshot {
    running: Vec<CardEntry>,
    done: Vec<CardEntry>,
    today_cost_usd: f64,
}

#[derive(Debug, Default)]
struct UiState {
    selected_idx: usize,
    selected_card_key: Option<String>,
    log_card_key: Option<String>,
    status_msg: Option<String>,
}

struct TerminalGuard {
    _private: (),
}

impl TerminalGuard {
    fn new() -> anyhow::Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, cursor::Hide)?;
        Ok(Self { _private: () })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), cursor::Show, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

pub async fn cmd_watch(root: &Path, all: bool) -> anyhow::Result<()> {
    let (sources, all_mode) = if all {
        let projects = project::registered_watch_projects()?;
        if projects.is_empty() {
            anyhow::bail!("no registered projects (run: bop project add <path> --alias <alias>)");
        }
        let sources = projects
            .into_iter()
            .map(|p| WatchSource {
                project_name: Some(p.name),
                cards_root: p.cards_root,
            })
            .collect();
        (sources, true)
    } else {
        (
            vec![WatchSource {
                project_name: None,
                cards_root: root.to_path_buf(),
            }],
            false,
        )
    };

    run_watch(&sources, all_mode)
}

fn run_watch(sources: &[WatchSource], all_mode: bool) -> anyhow::Result<()> {
    let _guard = TerminalGuard::new()?;
    let mut stdout = io::stdout();
    let mut state = UiState::default();

    loop {
        let (term_width, term_height) = terminal::size().unwrap_or((100, 30));
        let width = term_width as usize;
        let height = term_height as usize;

        let snapshot = collect_snapshot(sources);
        let rows = rows_for_render(&snapshot, height);
        reconcile_selection(&mut state, &rows);

        let log_card = resolve_log_card(&snapshot, &state, &rows);
        let log_title = log_card
            .as_ref()
            .map(display_card_label)
            .unwrap_or_else(|| "(no card selected)".to_string());
        let log_lines = log_card
            .as_ref()
            .map(|c| read_log_tail(&c.log_path, LOG_LINES))
            .unwrap_or_else(|| vec!["(no logs available)".to_string()]);

        render_frame(
            &mut stdout,
            width,
            &snapshot,
            &rows,
            &state,
            &log_title,
            &log_lines,
            all_mode,
        )?;

        if !event::poll(Duration::from_millis(REFRESH_MS))? {
            continue;
        }

        match event::read()? {
            Event::Key(key) => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Char('j') | KeyCode::Down => move_selection(&rows, &mut state, 1),
                KeyCode::Char('k') | KeyCode::Up => move_selection(&rows, &mut state, -1),
                KeyCode::Char('l') => cycle_log_running(&snapshot.running, &mut state),
                KeyCode::Char('p') => handle_pane_jump(&rows, &mut state),
                _ => {}
            },
            Event::Resize(_, _) => {}
            _ => {}
        }
    }

    Ok(())
}

fn render_frame(
    stdout: &mut io::Stdout,
    width: usize,
    snapshot: &Snapshot,
    rows: &[CardEntry],
    state: &UiState,
    log_title: &str,
    log_lines: &[String],
    all_mode: bool,
) -> anyhow::Result<()> {
    execute!(stdout, cursor::MoveTo(0, 0), Clear(ClearType::All))?;

    let hline = "━".repeat(width.max(1));
    writeln!(stdout, "{hline}")?;
    let title = if all_mode {
        "bop watch --all"
    } else {
        "bop watch"
    };
    writeln!(
        stdout,
        "{}",
        trim_to_width(
            &format!(
                "  {title}  •  {} running  •  {} done  •  ${:.2} today",
                snapshot.running.len(),
                snapshot.done.len(),
                snapshot.today_cost_usd
            ),
            width
        )
    )?;
    writeln!(stdout, "{hline}")?;

    if rows.is_empty() {
        writeln!(
            stdout,
            "{}",
            trim_to_width("  (no running or done cards)", width)
        )?;
    } else {
        for card in rows {
            let is_selected = state
                .selected_card_key
                .as_deref()
                .is_some_and(|key| key == card.key);
            let prefix = if is_selected { ">" } else { " " };
            writeln!(
                stdout,
                "{}",
                trim_to_width(&format_row(prefix, card, width), width)
            )?;
        }
    }

    writeln!(stdout, "{hline}")?;
    writeln!(
        stdout,
        "{}",
        trim_to_width(
            &format!("  LOG  {log_title}  (last {LOG_LINES} lines)"),
            width
        )
    )?;
    writeln!(stdout, "{hline}")?;
    for line in log_lines {
        writeln!(stdout, "{}", trim_to_width(&format!("  {line}"), width))?;
    }

    let mut hints = String::from("  [j/k] move  [l] cycle logs  [p] pane  [q/Esc] quit");
    if let Some(msg) = &state.status_msg {
        hints.push_str("  |  ");
        hints.push_str(msg);
    }
    writeln!(stdout, "{}", trim_to_width(&hints, width))?;
    stdout.flush()?;
    Ok(())
}

fn display_card_label(card: &CardEntry) -> String {
    card.project_name
        .as_deref()
        .map(|name| format!("[{name}] {}", card.id))
        .unwrap_or_else(|| card.id.clone())
}

fn format_row(prefix: &str, card: &CardEntry, width: usize) -> String {
    let id_width = width.saturating_sub(46).clamp(20, 52);
    let provider = pad_or_trim(&card.provider, 8);
    let elapsed = card
        .elapsed_s
        .map(format_duration)
        .unwrap_or_else(|| "-".to_string());
    let label = display_card_label(card);

    match card.state {
        CardState::Running => format!(
            "{prefix} ● {:id_width$} {provider} {:7} {:>8}   ↳ [p]ane",
            pad_or_trim(&label, id_width),
            "running",
            elapsed
        ),
        CardState::Done => {
            let cost = card
                .cost_usd
                .map(|c| format!("${c:.2}"))
                .unwrap_or_else(|| "-".to_string());
            format!(
                "{prefix} ✓ {:id_width$} {provider} {:7} {:>8}   {cost}",
                pad_or_trim(&label, id_width),
                "done",
                elapsed
            )
        }
    }
}

fn trim_to_width(input: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let len = input.chars().count();
    if len <= width {
        return input.to_string();
    }
    if width <= 3 {
        return ".".repeat(width);
    }
    let mut out = String::with_capacity(width);
    for ch in input.chars().take(width - 3) {
        out.push(ch);
    }
    out.push_str("...");
    out
}

fn pad_or_trim(input: &str, width: usize) -> String {
    let len = input.chars().count();
    if len == width {
        return input.to_string();
    }
    if len < width {
        return format!("{input:<width$}");
    }
    if width <= 3 {
        return ".".repeat(width);
    }
    let mut out = String::with_capacity(width);
    for ch in input.chars().take(width - 3) {
        out.push(ch);
    }
    out.push_str("...");
    out
}

fn reconcile_selection(state: &mut UiState, rows: &[CardEntry]) {
    if rows.is_empty() {
        state.selected_idx = 0;
        state.selected_card_key = None;
        state.log_card_key = None;
        return;
    }

    if let Some(ref selected_key) = state.selected_card_key {
        if let Some(idx) = rows.iter().position(|c| &c.key == selected_key) {
            state.selected_idx = idx;
        } else {
            state.selected_idx = state.selected_idx.min(rows.len() - 1);
            state.selected_card_key = Some(rows[state.selected_idx].key.clone());
        }
    } else {
        state.selected_idx = state.selected_idx.min(rows.len() - 1);
        state.selected_card_key = Some(rows[state.selected_idx].key.clone());
    }

    if state.log_card_key.is_none() {
        state.log_card_key = state.selected_card_key.clone();
    }
}

fn move_selection(rows: &[CardEntry], state: &mut UiState, step: i32) {
    if rows.is_empty() {
        state.status_msg = Some("no cards".to_string());
        return;
    }

    let max_idx = rows.len().saturating_sub(1);
    if step > 0 {
        state.selected_idx = (state.selected_idx + 1).min(max_idx);
    } else if step < 0 {
        state.selected_idx = state.selected_idx.saturating_sub(1);
    }

    let selected_key = rows[state.selected_idx].key.clone();
    state.selected_card_key = Some(selected_key.clone());
    state.log_card_key = Some(selected_key);
    state.status_msg = None;
}

fn cycle_log_running(running: &[CardEntry], state: &mut UiState) {
    if running.is_empty() {
        state.status_msg = Some("no running cards".to_string());
        return;
    }

    let next_idx = state
        .log_card_key
        .as_ref()
        .and_then(|key| running.iter().position(|c| &c.key == key))
        .map(|idx| (idx + 1) % running.len())
        .unwrap_or(0);

    let next = &running[next_idx];
    state.log_card_key = Some(next.key.clone());
    state.status_msg = Some(format!("log: {}", display_card_label(next)));
}

fn handle_pane_jump(rows: &[CardEntry], state: &mut UiState) {
    let Some(selected_key) = state.selected_card_key.as_ref() else {
        state.status_msg = Some("no selected card".to_string());
        return;
    };
    let Some(card) = rows.iter().find(|c| &c.key == selected_key) else {
        state.status_msg = Some("selected card not visible".to_string());
        return;
    };

    if card.state != CardState::Running {
        state.status_msg = Some("selected card is not running".to_string());
        return;
    }

    if std::env::var_os("ZELLIJ").is_some() {
        let lock_ok = StdCommand::new("zellij")
            .args(["action", "switch-mode", "locked"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !lock_ok {
            state.status_msg = Some("zellij switch-mode failed".to_string());
            return;
        }

        let focus_ok = StdCommand::new("zellij")
            .args(["action", "focus-pane", "--name", &card.id])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if focus_ok {
            state.status_msg = Some(format!("focused pane {}", card.id));
        } else {
            state.status_msg = Some(format!("failed to focus pane {}", card.id));
        }
    } else {
        state.status_msg = Some(card.card_dir.display().to_string());
    }
}

fn resolve_log_card(snapshot: &Snapshot, state: &UiState, rows: &[CardEntry]) -> Option<CardEntry> {
    if let Some(key) = state.log_card_key.as_deref() {
        if let Some(card) = find_card_by_key(snapshot, key) {
            return Some(card.clone());
        }
    }
    if let Some(key) = state.selected_card_key.as_deref() {
        if let Some(card) = find_card_by_key(snapshot, key) {
            return Some(card.clone());
        }
    }
    rows.first().cloned()
}

fn find_card_by_key<'a>(snapshot: &'a Snapshot, key: &str) -> Option<&'a CardEntry> {
    snapshot
        .running
        .iter()
        .chain(snapshot.done.iter())
        .find(|c| c.key == key)
}

fn rows_for_render(snapshot: &Snapshot, term_height: usize) -> Vec<CardEntry> {
    let reserved = 3 + 3 + LOG_LINES + 1;
    let available_rows = term_height.saturating_sub(reserved);

    let mut rows = snapshot.running.clone();
    let done_slots = available_rows.saturating_sub(rows.len());
    rows.extend(snapshot.done.iter().take(done_slots).cloned());
    rows
}

fn collect_snapshot(sources: &[WatchSource]) -> Snapshot {
    let now = Utc::now();
    let mut running = Vec::new();
    let mut done = Vec::new();

    for source in sources {
        running.extend(collect_state_cards(source, CardState::Running, now));
        done.extend(collect_state_cards(source, CardState::Done, now));
    }

    running.sort_by(|a, b| {
        a.project_name
            .cmp(&b.project_name)
            .then_with(|| a.id.cmp(&b.id))
    });
    done.sort_by(|a, b| match (a.sort_ts, b.sort_ts) {
        (Some(at), Some(bt)) => bt
            .cmp(&at)
            .then_with(|| a.project_name.cmp(&b.project_name))
            .then_with(|| a.id.cmp(&b.id)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => a
            .project_name
            .cmp(&b.project_name)
            .then_with(|| a.id.cmp(&b.id)),
    });

    let today = Local::now().date_naive();
    let today_cost_usd = done
        .iter()
        .filter_map(|card| {
            let ts = card.sort_ts?;
            let cost = card.cost_usd?;
            if ts.with_timezone(&Local).date_naive() == today {
                Some(cost)
            } else {
                None
            }
        })
        .sum::<f64>();
    let today_cost_usd = if today_cost_usd.abs() < 0.000_001 {
        0.0
    } else {
        today_cost_usd
    };

    Snapshot {
        running,
        done,
        today_cost_usd,
    }
}

fn collect_state_cards(
    source: &WatchSource,
    state: CardState,
    now: DateTime<Utc>,
) -> Vec<CardEntry> {
    let state_name = match state {
        CardState::Running => "running",
        CardState::Done => "done",
    };

    let mut out = Vec::new();
    for state_dir in state_dirs(&source.cards_root, state_name) {
        let Ok(entries) = fs::read_dir(&state_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let card_dir = entry.path();
            if !is_card_dir(&card_dir) {
                continue;
            }

            let Ok(meta) = bop_core::read_meta(&card_dir) else {
                continue;
            };
            let last_run = meta.runs.last();
            let pid_mtime = file_mtime_utc(&card_dir.join("logs").join("pid"));

            let run_started = last_run
                .and_then(|r| parse_utc_ts(&r.started_at))
                .or(pid_mtime);
            let run_ended = last_run
                .and_then(|r| r.ended_at.as_deref())
                .and_then(parse_utc_ts);

            let elapsed_s = match state {
                CardState::Running => run_started
                    .map(|start| now.signed_duration_since(start).num_seconds().max(0) as u64)
                    .or_else(|| last_run.and_then(|r| r.duration_s)),
                CardState::Done => last_run
                    .and_then(|r| r.duration_s)
                    .or_else(|| duration_from_timestamps(run_started, run_ended)),
            };

            let sort_ts = run_ended.or(run_started).or(pid_mtime);
            let provider = last_run
                .map(|r| r.provider.trim().to_string())
                .filter(|s| !s.is_empty())
                .or_else(|| meta.provider_chain.first().cloned())
                .unwrap_or_else(|| "-".to_string());
            let key = source
                .project_name
                .as_deref()
                .map(|name| format!("{name}:{}", meta.id))
                .unwrap_or_else(|| meta.id.clone());

            out.push(CardEntry {
                key,
                id: meta.id,
                project_name: source.project_name.clone(),
                provider,
                state,
                card_dir: card_dir.clone(),
                log_path: card_dir.join("logs").join("stdout.log"),
                elapsed_s,
                cost_usd: last_run.and_then(|r| r.cost_usd),
                sort_ts,
            });
        }
    }
    out
}

fn state_dirs(root: &Path, state: &str) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let root_state = root.join(state);
    if root_state.exists() {
        dirs.push(root_state);
    }

    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if !entry.file_name().to_string_lossy().starts_with("team-") {
                continue;
            }
            let scoped_state = path.join(state);
            if scoped_state.exists() {
                dirs.push(scoped_state);
            }
        }
    }

    dirs.sort();
    dirs
}

fn is_card_dir(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("bop") | Some("jobcard")
    )
}

fn parse_utc_ts(s: &str) -> Option<DateTime<Utc>> {
    if s.trim().is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|| {
            NaiveDateTime::parse_from_str(s.trim_end_matches('Z'), "%Y-%m-%dT%H:%M:%S")
                .ok()
                .map(|n| n.and_utc())
        })
}

fn duration_from_timestamps(
    start: Option<DateTime<Utc>>,
    end: Option<DateTime<Utc>>,
) -> Option<u64> {
    let (Some(start), Some(end)) = (start, end) else {
        return None;
    };
    let secs = end.signed_duration_since(start).num_seconds();
    if secs < 0 {
        None
    } else {
        Some(secs as u64)
    }
}

fn file_mtime_utc(path: &Path) -> Option<DateTime<Utc>> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    Some(DateTime::<Utc>::from(modified))
}

fn format_duration(total_s: u64) -> String {
    let h = total_s / 3600;
    let m = (total_s % 3600) / 60;
    let s = total_s % 60;
    if h > 0 {
        format!("{h}h {m:02}m")
    } else {
        format!("{m}m {s:02}s")
    }
}

fn read_log_tail(path: &Path, max_lines: usize) -> Vec<String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(_) => return vec![format!("(log not found: {})", path.display())],
    };

    let mut reader = BufReader::new(file);
    let file_len = match reader.get_ref().metadata() {
        Ok(meta) => meta.len(),
        Err(_) => return vec!["(unable to read log metadata)".to_string()],
    };

    let started_mid_file = file_len > LOG_READ_BYTES as u64;
    let seek_result = if started_mid_file {
        reader.seek(SeekFrom::End(-LOG_READ_BYTES))
    } else {
        reader.seek(SeekFrom::Start(0))
    };
    if seek_result.is_err() {
        return vec!["(unable to seek log tail)".to_string()];
    }

    let mut chunk = String::new();
    if reader.read_to_string(&mut chunk).is_err() {
        return vec!["(unable to read log file)".to_string()];
    }

    let mut lines: Vec<String> = chunk.lines().map(|line| line.to_string()).collect();
    if started_mid_file && !chunk.starts_with('\n') && !lines.is_empty() {
        lines.remove(0);
    }

    if lines.len() > max_lines {
        let split_at = lines.len() - max_lines;
        lines = lines.split_off(split_at);
    }

    if lines.is_empty() {
        vec!["(no log lines yet)".to_string()]
    } else {
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn parse_utc_ts_accepts_rfc3339() {
        let ts = parse_utc_ts("2026-03-08T14:22:01Z").expect("parse");
        assert_eq!(ts.to_rfc3339(), "2026-03-08T14:22:01+00:00");
    }

    #[test]
    fn format_duration_minute_and_second() {
        assert_eq!(format_duration(22), "0m 22s");
        assert_eq!(format_duration(64), "1m 04s");
    }

    #[test]
    fn format_duration_hour() {
        assert_eq!(format_duration(3720), "1h 02m");
    }

    #[test]
    fn rows_for_render_limits_done_rows() {
        let mk = |id: &str, state: CardState| CardEntry {
            key: id.to_string(),
            id: id.to_string(),
            project_name: None,
            provider: "codex".to_string(),
            state,
            card_dir: PathBuf::from("/tmp"),
            log_path: PathBuf::from("/tmp/stdout.log"),
            elapsed_s: Some(1),
            cost_usd: None,
            sort_ts: None,
        };
        let snapshot = Snapshot {
            running: vec![mk("r1", CardState::Running), mk("r2", CardState::Running)],
            done: vec![
                mk("d1", CardState::Done),
                mk("d2", CardState::Done),
                mk("d3", CardState::Done),
                mk("d4", CardState::Done),
            ],
            today_cost_usd: 0.0,
        };

        let rows = rows_for_render(&snapshot, 18);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].id, "r1");
        assert_eq!(rows[1].id, "r2");
        assert_eq!(rows[2].id, "d1");
    }

    #[test]
    fn read_log_tail_returns_last_lines() {
        let td = tempdir().expect("tempdir");
        let path = td.path().join("stdout.log");

        let mut content = String::new();
        for i in 1..=20 {
            content.push_str(&format!("line {i}\n"));
        }
        fs::write(&path, content).expect("write");

        let lines = read_log_tail(&path, 8);
        assert_eq!(lines.len(), 8);
        assert_eq!(lines.first().map(String::as_str), Some("line 13"));
        assert_eq!(lines.last().map(String::as_str), Some("line 20"));
    }

    #[test]
    fn read_log_tail_missing_file() {
        let td = tempdir().expect("tempdir");
        let path = td.path().join("missing.log");
        let lines = read_log_tail(&path, 8);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("log not found"));
    }
}
