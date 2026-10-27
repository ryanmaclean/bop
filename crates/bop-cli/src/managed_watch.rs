//! A status watcher whose only terminal writer lives in a dedicated Zellij pane.
//! The native notify watcher is a killable same-executable child with fixed-byte IPC.

use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
#[cfg(unix)]
use std::{fs, os::unix::fs::DirBuilderExt, os::unix::net::UnixStream as StdUnixStream};

use chrono::Local;
use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use terminal_size::{terminal_size, Height, Width};
use tokio::io::AsyncReadExt;
#[cfg(unix)]
use tokio::{
    io::AsyncWriteExt,
    net::{UnixListener, UnixStream},
};
use unicode_width::UnicodeWidthChar;

use crate::{acplan, list, lock, render::CardView};

const READY: u8 = 1;
const EVENT: u8 = 2;
const ERROR: u8 = 3;
const START_TIMEOUT: Duration = Duration::from_secs(5);
const STOP_TIMEOUT: Duration = Duration::from_secs(2);
const RENDER_TIMEOUT: Duration = Duration::from_secs(2);
const RENDER_MAX_BYTES: u64 = 2 * 1024 * 1024;
#[cfg(unix)]
const ACK_TIMEOUT: Duration = Duration::from_secs(20);
#[cfg(unix)]
const GRANT: u8 = 1;
#[cfg(unix)]
const ACK_READY: u8 = 2;
#[cfg(unix)]
const ACK_ERROR: u8 = 3;

#[cfg(unix)]
struct LaunchSocket {
    listener: UnixListener,
    dir: PathBuf,
    path: PathBuf,
}

#[cfg(unix)]
impl LaunchSocket {
    fn bind() -> anyhow::Result<Self> {
        for _ in 0..5 {
            let dir = std::env::temp_dir().join(format!("bw-{:016x}", rand::random::<u64>()));
            match fs::DirBuilder::new().mode(0o700).create(&dir) {
                Ok(()) => {
                    let path = dir.join("s");
                    let listener = match UnixListener::bind(&path) {
                        Ok(listener) => listener,
                        Err(err) => {
                            let _ = fs::remove_dir(&dir);
                            return Err(err.into());
                        }
                    };
                    return Ok(Self {
                        listener,
                        dir,
                        path,
                    });
                }
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(err) => return Err(err.into()),
            }
        }
        anyhow::bail!("could not allocate a private watch launch socket")
    }
}

#[cfg(unix)]
impl Drop for LaunchSocket {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_dir(&self.dir);
    }
}

pub struct LaunchAck {
    #[cfg(unix)]
    stream: StdUnixStream,
    ready_sent: bool,
}

impl LaunchAck {
    fn ready(&mut self) -> anyhow::Result<()> {
        #[cfg(unix)]
        self.stream.write_all(&[ACK_READY])?;
        self.ready_sent = true;
        Ok(())
    }

    fn fail(&mut self, error: &anyhow::Error) {
        if self.ready_sent {
            return;
        }
        #[cfg(unix)]
        {
            let _ = self.stream.write_all(&[ACK_ERROR]);
            let message = format!("{error:#}");
            let _ = self
                .stream
                .write_all(&message.as_bytes()[..message.len().min(4000)]);
        }
    }
}

/// Open a dedicated command pane; the invoking shell's PTY is never redrawn.
#[cfg(unix)]
pub async fn launch_cards_watch(root: &Path) -> anyhow::Result<()> {
    if std::env::var_os("ZELLIJ_SESSION_NAME").is_none() {
        anyhow::bail!("interactive bop status --watch requires a mediated Zellij pane");
    }
    let (dev, ino) = tty_identity()?;
    let launch = LaunchSocket::bind()?;
    let token = rand::random::<u128>();
    let mut command = tokio::process::Command::new("zellij");
    command
        .args(["run", "--in-place", "--close-on-exit", "--cwd"])
        .arg(std::env::current_dir()?)
        .arg("--")
        .arg(std::env::current_exe()?)
        .arg("--cards-dir")
        .arg(root.canonicalize()?)
        .arg("__watch-pane")
        .arg(dev.to_string())
        .arg(ino.to_string())
        .arg(&launch.path)
        .arg(token.to_string())
        .kill_on_drop(true);
    let status = tokio::time::timeout(START_TIMEOUT, command.status())
        .await
        .map_err(|_| anyhow::anyhow!("Zellij pane launch timed out"))??;
    if !status.success() {
        anyhow::bail!("Zellij could not open the mediated watch pane");
    }
    let accepted = tokio::time::timeout(ACK_TIMEOUT, async {
        let (mut stream, _) = launch.listener.accept().await?;
        let mut received = [0u8; 16];
        stream.read_exact(&mut received).await?;
        if received != token.to_be_bytes() {
            anyhow::bail!("watch pane launch capability mismatch");
        }
        stream.write_all(&[GRANT]).await?;
        match stream.read_u8().await? {
            ACK_READY => Ok(()),
            ACK_ERROR => {
                let mut message = Vec::new();
                (&mut stream).take(4000).read_to_end(&mut message).await?;
                anyhow::bail!(
                    "watch pane startup failed: {}",
                    String::from_utf8_lossy(&message)
                )
            }
            _ => anyhow::bail!("watch pane sent an invalid startup acknowledgment"),
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("watch pane readiness was not acknowledged within 20 seconds"))?;
    accepted
}

#[cfg(not(unix))]
pub async fn launch_cards_watch(_root: &Path) -> anyhow::Result<()> {
    anyhow::bail!("mediated watch requires a Unix PTY")
}

#[cfg(unix)]
pub async fn run_managed_pane(
    root: &Path,
    origin_dev: u64,
    origin_ino: u64,
    ack_socket: &Path,
    ack_token: u128,
) -> anyhow::Result<()> {
    let mut stream = tokio::time::timeout(START_TIMEOUT, UnixStream::connect(ack_socket))
        .await
        .map_err(|_| anyhow::anyhow!("watch pane launch socket connection timed out"))??;
    stream.write_all(&ack_token.to_be_bytes()).await?;
    let grant = tokio::time::timeout(START_TIMEOUT, stream.read_u8())
        .await
        .map_err(|_| anyhow::anyhow!("watch pane launch grant timed out"))??;
    if grant != GRANT {
        anyhow::bail!("watch pane launch was not granted");
    }
    let stream = stream.into_std()?;
    stream.set_nonblocking(false)?;
    stream.set_write_timeout(Some(START_TIMEOUT))?;
    let mut ack = LaunchAck {
        stream,
        ready_sent: false,
    };
    let result = async {
        verify_managed_pane(origin_dev, origin_ino)?;
        list_cards_watch(root, true, Some(&mut ack)).await
    }
    .await;
    if let Err(error) = &result {
        ack.fail(error);
    }
    result
}

#[cfg(not(unix))]
pub async fn run_managed_pane(
    _root: &Path,
    _origin_dev: u64,
    _origin_ino: u64,
    _ack_socket: &Path,
    _ack_token: u128,
) -> anyhow::Result<()> {
    anyhow::bail!("mediated watch requires a Unix PTY")
}

/// Check that the launched command has a new PTY, not the shell's old one.
pub fn verify_managed_pane(origin_dev: u64, origin_ino: u64) -> anyhow::Result<()> {
    if std::env::var_os("ZELLIJ_SESSION_NAME").is_none() || !io::stdout().is_terminal() {
        anyhow::bail!("watch pane requires a Zellij-owned terminal");
    }
    if tty_identity()? == (origin_dev, origin_ino) {
        anyhow::bail!("watch pane is still using the invoking shell's PTY");
    }
    Ok(())
}

#[cfg(unix)]
fn tty_identity() -> anyhow::Result<(u64, u64)> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(libc::STDOUT_FILENO, stat.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error().into());
    }
    let stat = unsafe { stat.assume_init() };
    Ok((tty_component(stat.st_dev)?, tty_component(stat.st_ino)?))
}

#[cfg(unix)]
fn tty_component<T: TryInto<u64>>(value: T) -> anyhow::Result<u64> {
    value
        .try_into()
        .map_err(|_| anyhow::anyhow!("terminal identity component is negative"))
}

#[cfg(not(unix))]
fn tty_identity() -> anyhow::Result<(u64, u64)> {
    anyhow::bail!("mediated watch requires a Unix PTY")
}

fn watch_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![root.to_path_buf()];
    if let Some(git_root) = acplan::find_git_root(root) {
        let specs = git_root.join(".auto-claude").join("specs");
        if specs.exists() {
            dirs.push(specs);
        }
    }
    dirs
}

fn send_frame(frame: u8) -> io::Result<()> {
    let mut out = io::stdout().lock();
    out.write_all(&[frame])?;
    out.flush()
}

fn hold_failed_worker() -> ! {
    let _ = send_frame(ERROR);
    // The stdin monitor terminates the entire process after parent teardown.
    // Until then, keep the singleton even when native registration fails.
    loop {
        std::thread::park();
    }
}

/// Hidden worker. It owns the singleton: a parent crash cannot free the lock
/// while a blocked native registration or teardown still has live descriptors.
pub fn run_watch_worker(root: &Path) -> anyhow::Result<()> {
    if io::stdin().is_terminal() || io::stdout().is_terminal() {
        anyhow::bail!("watch worker requires private stdin and stdout pipes");
    }
    let _guard = lock::acquire_watch_lock(root)?;
    std::thread::spawn(|| {
        let mut byte = [0u8; 1];
        loop {
            match io::stdin().read(&mut byte) {
                Ok(0) | Err(_) => std::process::exit(1),
                Ok(_) => {}
            }
        }
    });

    let (tx, rx) = std::sync::mpsc::channel();
    let mut debouncer = match new_debouncer(Duration::from_millis(100), tx) {
        Ok(watcher) => watcher,
        Err(_) => hold_failed_worker(),
    };
    for dir in watch_dirs(root) {
        if debouncer
            .watcher()
            .watch(&dir, notify::RecursiveMode::Recursive)
            .is_err()
        {
            hold_failed_worker();
        }
    }
    send_frame(READY)?;
    loop {
        match rx.recv() {
            Ok(Ok(events)) => {
                if events.iter().any(|event| {
                    matches!(
                        event.kind,
                        DebouncedEventKind::Any | DebouncedEventKind::AnyContinuous
                    ) && (list::is_card_event_path(&event.path)
                        || event.path.file_name().and_then(|s| s.to_str())
                            == Some("implementation_plan.json"))
                }) {
                    send_frame(EVENT)?;
                }
            }
            Ok(Err(_)) | Err(_) => hold_failed_worker(),
        }
    }
}

/// The parent alone writes to the pane; the worker writes only to a pipe.
/// kill_on_drop covers async cancellation, and the worker-held lock remains
/// until its process exits even if this future is abandoned.
pub async fn list_cards_watch(
    root: &Path,
    managed: bool,
    mut ack: Option<&mut LaunchAck>,
) -> anyhow::Result<()> {
    let mut child = tokio::process::Command::new(std::env::current_exe()?)
        .arg("--cards-dir")
        .arg(root)
        .arg("__watch-worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let mut worker_stdin = child.stdin.take();
    let mut worker_stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("watch worker stdout pipe missing"))?;

    let ready = tokio::time::timeout(START_TIMEOUT, worker_stdout.read_u8()).await;
    if !matches!(ready, Ok(Ok(READY))) {
        stop_worker(&mut child, worker_stdin.take()).await?;
        anyhow::bail!("watch worker failed to register every required path");
    }

    let run_result: anyhow::Result<()> = async {
        let Some(mut previous) = collect_frame(root, managed).await? else {
            return Ok(());
        };
        write_frame(&previous.0, managed)?;
        if let Some(ack) = ack.as_deref_mut() {
            ack.ready()?;
        }
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        loop {
            tokio::select! {
                signal = tokio::signal::ctrl_c() => {
                    signal?;
                    break Ok(());
                }
                frame = worker_stdout.read_u8() => {
                    match frame {
                        Ok(EVENT) => {}
                        Ok(ERROR) => anyhow::bail!("native watch worker reported an error"),
                        Ok(_) => anyhow::bail!("native watch worker sent an invalid frame"),
                        Err(err) => return Err(anyhow::anyhow!("native watch worker disconnected: {err}")),
                    }
                }
                _ = tick.tick(), if managed => {}
            }
            let Some(next) = collect_frame(root, managed).await? else {
                break Ok(());
            };
            if next != previous {
                write_frame(&next.0, managed)?;
                previous = next;
            }
        }
    }
    .await;

    stop_worker(&mut child, worker_stdin.take()).await?;
    run_result
}

async fn stop_worker(
    child: &mut tokio::process::Child,
    stdin: Option<tokio::process::ChildStdin>,
) -> anyhow::Result<()> {
    drop(stdin);
    if let Ok(status) = tokio::time::timeout(STOP_TIMEOUT, child.wait()).await {
        status?;
        return Ok(());
    }
    child.start_kill()?;
    tokio::time::timeout(STOP_TIMEOUT, child.wait())
        .await
        .map_err(|_| anyhow::anyhow!("watch worker reap unconfirmed; singleton retained"))??;
    Ok(())
}

fn frame_dimensions(managed: bool) -> anyhow::Result<Option<(u16, u16)>> {
    if managed {
        let (Width(width), Height(height)) =
            terminal_size().ok_or_else(|| anyhow::anyhow!("watch pane dimensions unavailable"))?;
        if width == 0 || height == 0 {
            anyhow::bail!("watch pane dimensions are zero");
        }
        Ok(Some((width, height)))
    } else {
        Ok(None)
    }
}

/// A blocked filesystem read remains confined to this killable child.
pub fn run_render_worker(root: &Path, width: u16, height: u16) -> anyhow::Result<()> {
    if io::stdout().is_terminal() {
        anyhow::bail!("watch render worker requires a private stdout pipe");
    }
    let dimensions = if width == 0 && height == 0 {
        None
    } else if width > 0 && height > 0 {
        Some((width, height))
    } else {
        anyhow::bail!("invalid watch render dimensions");
    };
    let rows = render_frame(root, dimensions)?;
    let mut out = io::stdout().lock();
    serde_json::to_writer(&mut out, &rows)?;
    out.flush()?;
    Ok(())
}

async fn collect_frame(
    root: &Path,
    managed: bool,
) -> anyhow::Result<Option<(Vec<String>, Option<(u16, u16)>)>> {
    let dimensions = frame_dimensions(managed)?;
    let (width, height) = dimensions.unwrap_or((0, 0));
    let mut child = tokio::process::Command::new(std::env::current_exe()?)
        .arg("--cards-dir")
        .arg(root)
        .arg("__watch-render")
        .arg(width.to_string())
        .arg(height.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("watch renderer stdout pipe missing"))?;
    enum Outcome {
        Done(anyhow::Result<Vec<String>>),
        Interrupted(io::Result<()>),
        TimedOut,
    }
    let outcome = {
        let read = async {
            let mut bytes = Vec::new();
            (&mut stdout)
                .take(RENDER_MAX_BYTES)
                .read_to_end(&mut bytes)
                .await?;
            let status = child.wait().await?;
            if !status.success() {
                anyhow::bail!("watch board collector failed");
            }
            let rows = serde_json::from_slice::<Vec<String>>(&bytes)?;
            Ok(rows)
        };
        tokio::pin!(read);
        tokio::select! {
            signal = tokio::signal::ctrl_c() => Outcome::Interrupted(signal),
            result = tokio::time::timeout(RENDER_TIMEOUT, &mut read) => {
                match result {
                    Ok(rows) => Outcome::Done(rows),
                    Err(_) => Outcome::TimedOut,
                }
            }
        }
    };
    match outcome {
        Outcome::Done(Ok(rows)) => Ok(Some((rows, dimensions))),
        Outcome::Done(Err(err)) => {
            stop_worker(&mut child, None).await?;
            Err(err)
        }
        Outcome::Interrupted(signal) => {
            stop_worker(&mut child, None).await?;
            signal?;
            Ok(None)
        }
        Outcome::TimedOut => {
            stop_worker(&mut child, None).await?;
            anyhow::bail!("watch board collection timed out")
        }
    }
}

fn render_frame(root: &Path, dimensions: Option<(u16, u16)>) -> anyhow::Result<Vec<String>> {
    let groups = list::collect_card_groups_watch(root, "active")?;
    let stats = list::calculate_stats(root)?;
    let mut rows = vec![format!(
        "bop | {} | watching .cards/",
        Local::now().format("%H:%M:%S")
    )];
    for (label, cards) in groups {
        rows.push(format!("{label} ({})", cards.len()));
        for card in cards {
            rows.extend(card_rows(
                &card,
                dimensions.map(|(width, _)| usize::from(width.saturating_sub(1))),
            ));
        }
    }
    let mut footer = format!("{} total", stats.total);
    if let Some(rate) = stats.success_rate_pct {
        footer.push_str(&format!(" | {rate:.0}% success"));
    }
    if let Some(avg) = stats.avg_duration_s {
        footer.push_str(&format!(" | avg {}", format_duration(avg as u64)));
    }
    rows.push(footer);
    if let Some((width, height)) = dimensions {
        rows = fit_rows(rows, width, height);
    } else {
        rows = rows
            .iter()
            .map(|row| sanitize_row(row, usize::MAX))
            .collect();
    }
    Ok(rows)
}

fn format_duration(seconds: u64) -> String {
    if seconds >= 60 {
        format!("{}m{:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{seconds}s")
    }
}

fn card_rows(card: &CardView, max_cells: Option<usize>) -> Vec<String> {
    let token = card.token.as_deref().unwrap_or("");
    let marker = match card.state.as_str() {
        "running" => "[>]",
        "done" | "merged" => "[x]",
        "failed" => "[!]",
        _ => "[ ]",
    };
    let mut identity = format!("  {marker} {token} {}", card.id);
    if let Some(elapsed) = card.elapsed_s {
        identity.push_str(&format!("  {}", format_duration(elapsed)));
    }
    if let Some(provider) = &card.provider {
        identity.push_str(&format!("  {provider}"));
    }
    if card.state == "failed" {
        if let Some(code) = card.exit_code {
            identity.push_str(&format!("  exit {code}"));
        }
    }
    identity.push_str(&format!("  {}", card.title));
    let mut rows = vec![identity];
    if card.state == "running" || card.progress > 0 || card.phase_name.is_some() {
        rows.push(progress_row(
            card.progress,
            card.phase_name.as_deref(),
            max_cells.unwrap_or(usize::MAX),
        ));
    }
    if card.state == "failed" {
        if let Some(reason) = &card.failure_reason {
            rows.push(format!("    {reason}"));
        }
    }
    rows
}

fn progress_row(progress: u8, phase: Option<&str>, max_cells: usize) -> String {
    let pct = progress.min(100);
    let percent = format!("{pct}%");
    let fixed_cells = 2 + 1 + 1 + 1 + percent.len(); // "  [" + "] " + percent
    if max_cells < fixed_cells + 1 {
        return percent;
    }
    let bar_width = 12.min(max_cells - fixed_cells);
    let filled = usize::from(pct) * bar_width / 100;
    let mut row = format!(
        "  [{}{}] {percent}",
        "=".repeat(filled),
        " ".repeat(bar_width - filled)
    );
    if let Some(phase) = phase {
        row.push_str(&format!("  {phase}"));
    }
    row
}

fn fit_rows(mut rows: Vec<String>, width: u16, height: u16) -> Vec<String> {
    let max_rows = usize::from(height).saturating_sub(1);
    let max_width = usize::from(width).saturating_sub(1);
    if max_rows == 0 || max_width == 0 {
        return Vec::new();
    }
    if rows.len() > max_rows {
        let hidden = rows.len() - max_rows;
        rows.truncate(max_rows);
        rows[max_rows - 1] = format!("... {hidden} more rows");
    }
    rows.iter()
        .map(|row| sanitize_row(row, max_width))
        .collect()
}

fn sanitize_row(input: &str, max_cells: usize) -> String {
    let mut out = String::new();
    let mut cells = 0usize;
    for ch in input.chars() {
        if ch.is_control() || matches!(ch, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') {
            continue;
        }
        let width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width == 0 {
            continue;
        }
        if cells.saturating_add(width) > max_cells {
            break;
        }
        out.push(ch);
        cells += width;
    }
    out
}

fn write_frame(rows: &[String], managed: bool) -> io::Result<()> {
    let mut out = io::stdout().lock();
    out.write_all(&encode_frame(rows, managed))?;
    out.flush()
}

fn encode_frame(rows: &[String], managed: bool) -> Vec<u8> {
    if managed {
        let mut bytes = b"\x1b[H\x1b[J".to_vec();
        for (index, row) in rows.iter().enumerate() {
            if index > 0 {
                bytes.extend_from_slice(b"\r\n");
            }
            let color = row_color(row);
            if let Some(color) = color {
                bytes.extend_from_slice(color);
            }
            bytes.extend_from_slice(row.as_bytes());
            if color.is_some() {
                bytes.extend_from_slice(b"\x1b[0m");
            }
        }
        bytes
    } else {
        let mut bytes = rows.join("\n").into_bytes();
        bytes.push(b'\n');
        bytes
    }
}

/// Only fixed escapes are added after input rows have been sanitized and clipped.
fn row_color(row: &str) -> Option<&'static [u8]> {
    let state = if row.starts_with("  [>]") {
        "running"
    } else if row.starts_with("  [!]") {
        "failed"
    } else if row.starts_with("  [x]") {
        "done"
    } else if row.starts_with("  [ ]") {
        "pending"
    } else {
        let (label, count) = row.rsplit_once(" (")?;
        if !count.ends_with(')')
            || !count[..count.len() - 1]
                .chars()
                .all(|ch| ch.is_ascii_digit())
        {
            return None;
        }
        label.rsplit('/').next().unwrap_or(label)
    };
    match state {
        "running" => Some(b"\x1b[33m"),
        "pending" => Some(b"\x1b[34m"),
        "done" => Some(b"\x1b[32m"),
        "failed" => Some(b"\x1b[31m"),
        "merged" => Some(b"\x1b[35m"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_is_bounded_and_control_free() {
        let row = sanitize_row("id \x1b[31m a界\u{202e}b", 8);
        assert_eq!(row, "id [31m ");
    }

    #[test]
    fn wide_glyph_does_not_enter_reserved_column() {
        assert_eq!(sanitize_row("ab界", 3), "ab");
    }

    #[test]
    fn frame_caps_rows_and_never_reaches_final_column() {
        let rows = vec!["123456789".to_string(); 10];
        let fitted = fit_rows(rows, 8, 3);
        assert_eq!(fitted.len(), 2);
        assert!(fitted.iter().all(|row| row.chars().count() <= 7));
        assert!(fitted[1].starts_with("..."));
    }

    #[test]
    fn one_cell_pane_never_writes_a_reserved_cell() {
        let rows = vec!["visible".to_string()];
        assert!(fit_rows(rows.clone(), 1, 10).is_empty());
        assert!(fit_rows(rows, 10, 1).is_empty());
        assert_eq!(encode_frame(&[], true).as_slice(), b"\x1b[H\x1b[J");
    }

    #[test]
    fn running_card_keeps_elapsed_and_bar_with_room() {
        let card = CardView {
            id: "sample".into(),
            state: "running".into(),
            glyph: None,
            token: Some("\u{2660}A".into()),
            title: "Example task".into(),
            stage: "implement".into(),
            priority: None,
            progress: 67,
            provider: Some("codex".into()),
            elapsed_s: Some(134),
            phase_name: Some("QA".into()),
            phase_frac: 0.0,
            failure_reason: None,
            exit_code: None,
            ac_subtasks_done: None,
            ac_subtasks_total: None,
        };
        let rows = card_rows(&card, Some(40));
        assert!(rows[0].contains("2m14s"));
        assert!(rows[0].contains("codex"));
        assert!(rows[1].contains("[========"));
        assert!(rows[1].contains("67%"));
        assert_eq!(progress_row(67, None, 3), "67%");
        let fitted = fit_rows(rows, 40, 5);
        assert!(fitted.iter().all(|row| {
            row.chars()
                .map(|ch| UnicodeWidthChar::width(ch).unwrap_or(0))
                .sum::<usize>()
                <= 39
        }));
    }

    #[test]
    fn pipe_frame_has_no_cursor_control() {
        let rows = vec!["bop".into(), "one card".into()];
        assert_eq!(encode_frame(&rows, false).as_slice(), b"bop\none card\n");
        assert!(encode_frame(&rows, true).starts_with(b"\x1b[H\x1b[J"));
    }

    #[test]
    fn pane_color_is_fixed_and_added_after_sanitization() {
        let row = sanitize_row("running (1)", 20);
        let frame = encode_frame(&[row], true);
        assert!(frame.windows(5).any(|window| window == b"\x1b[33m"));
        let unsafe_row = sanitize_row("evil\x1b[2J", 20);
        let safe_frame = encode_frame(&[unsafe_row], true);
        assert!(!safe_frame.windows(4).any(|window| window == b"\x1b[2J"));
    }

    #[test]
    fn encoder_clips_unicode_and_clears_before_smaller_frame() {
        let long = vec![
            "running (2)".to_string(),
            "  [>] ♠A long 界界 title \x1b[31m formerly wrapped".to_string(),
            "  [>] ♠K second card".to_string(),
            "3 total".to_string(),
        ];
        let wide = fit_rows(long, 28, 6);
        assert_eq!(wide.len(), 4);
        assert!(wide.iter().all(|row| {
            row.chars()
                .map(|ch| UnicodeWidthChar::width(ch).unwrap_or(0))
                .sum::<usize>()
                <= 27
        }));
        assert!(wide[1].contains("界"));
        assert!(!wide[1].contains('\x1b'));

        // This checks frame bytes only. A native PTY readback is still needed
        // to establish physical row clearing and shell isolation.
        let narrow = fit_rows(vec!["running (1)".into(), "  [>] ♠A 界".into()], 12, 3);
        let repaint = encode_frame(&narrow, true);
        assert!(repaint.starts_with(b"\x1b[H\x1b[J"));
        assert_eq!(
            repaint
                .windows(3)
                .filter(|chunk| *chunk == b"\x1b[H")
                .count(),
            1
        );
        assert!(narrow.iter().all(|row| {
            row.chars()
                .map(|ch| UnicodeWidthChar::width(ch).unwrap_or(0))
                .sum::<usize>()
                <= 11
        }));
    }
}
