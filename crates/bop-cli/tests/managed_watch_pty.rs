//! Real PTY/Zellij watch fixture. Run only on an approved disposable Linux host.

#![cfg(target_os = "linux")]

use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    Arc,
};
use std::thread;
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(20);
const POLL: Duration = Duration::from_millis(100);
const STDERR_CAPTURE_CAP: usize = 16 * 1024;

// Observe only fixed CLI failure phrases in the existing PTY drain. No raw
// terminal bytes, error text, or paths are retained in the diagnostic receipt.
const WATCH_ERRORS: [(&[u8], u32); 9] = [
    (
        b"interactive bop status --watch requires a mediated Zellij pane",
        1,
    ),
    (b"Zellij pane launch timed out", 2),
    (b"Zellij could not open the mediated watch pane", 4),
    (
        b"watch pane readiness was not acknowledged within 20 seconds",
        8,
    ),
    (b"watch pane launch capability mismatch", 16),
    (b"watch pane startup failed:", 32),
    (b"watch pane sent an invalid startup acknowledgment", 64),
    (b"error: unexpected argument", 128),
    (b"Error: ", 256),
];

const STARTUP_ERROR_PREFIX: &[u8] = b"watch pane startup failed:";
const INNER_STARTUP_ERRORS: [(&[u8], &str); 9] = [
    (
        b"watch pane requires a Zellij-owned terminal",
        "pane_terminal_missing",
    ),
    (
        b"watch pane is still using the invoking shell's PTY",
        "pane_reused_origin_pty",
    ),
    (
        b"terminal identity component is negative",
        "invalid_terminal_identity",
    ),
    (b"watch worker stdout pipe missing", "worker_pipe_missing"),
    (
        b"watch worker failed to register every required path",
        "worker_registration_failed",
    ),
    (
        b"watch pane dimensions unavailable",
        "dimensions_unavailable",
    ),
    (b"watch pane dimensions are zero", "dimensions_zero"),
    (b"watch board collector failed", "initial_render_failed"),
    (
        b"watch board collection timed out",
        "initial_render_timeout",
    ),
];

fn classify_watch_bytes(window: &[u8]) -> u32 {
    WATCH_ERRORS.iter().fold(0, |flags, (needle, bit)| {
        if window.windows(needle.len()).any(|part| part == *needle) {
            flags | bit
        } else {
            flags
        }
    })
}

fn fixed_error_class(flags: u32) -> &'static str {
    match flags & !256 {
        1 => "missing_zellij_session",
        2 => "zellij_launch_timeout",
        4 => "zellij_open_failed",
        8 => "readiness_timeout",
        16 => "capability_mismatch",
        32 => "pane_startup_failed",
        64 => "invalid_acknowledgment",
        128 => "zellij_argument_error",
        0 if flags & 256 != 0 => "other_error_text",
        0 => "no_error_text_seen",
        _ => "multiple_error_classes",
    }
}

fn fixed_inner_startup_class(raw: &[u8]) -> &'static str {
    let Some(prefix_at) = raw
        .windows(STARTUP_ERROR_PREFIX.len())
        .position(|part| part == STARTUP_ERROR_PREFIX)
    else {
        return "no_startup_prefix";
    };
    let detail = &raw[prefix_at + STARTUP_ERROR_PREFIX.len()..];
    let mut selected = None;
    for (phrase, class) in INNER_STARTUP_ERRORS {
        if detail.windows(phrase.len()).any(|part| part == phrase) {
            if selected.is_some() {
                return "multiple_inner_classes";
            }
            selected = Some(class);
        }
    }
    selected.unwrap_or("unclassified_inner_error")
}

fn fixed_dimension_fd_class(raw: &[u8], label: &[u8]) -> &'static str {
    let mut selected = None;
    for class in ["not_tty", "ioctl_failed", "zero", "positive_late"] {
        let needle = [label, &b"="[..], class.as_bytes()].concat();
        if raw
            .windows(needle.len())
            .any(|part| part == needle.as_slice())
        {
            if selected.is_some() {
                return "ambiguous";
            }
            selected = Some(class);
        }
    }
    selected.unwrap_or("missing")
}

fn initial_cell_mask(screen: &str) -> u8 {
    u8::from(screen.contains("DELETED_UNIQUE_7Q"))
        | (u8::from(screen.contains("REMAINING_UNIQUE_8R")) << 1)
        | (u8::from(screen.contains("OLD_TAIL_7Q")) << 2)
        | (u8::from(screen.contains("watching .cards/")) << 3)
}

fn until<T>(description: &str, mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Some(value) = probe() {
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {description}"
        );
        thread::sleep(POLL);
    }
}

fn run_bounded(mut command: Command) -> bool {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let Ok(mut child) = command.spawn() else {
        return false;
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return status.success();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return false;
        }
        thread::sleep(POLL);
    }
}

struct Session {
    child: Child,
    session: String,
    home: PathBuf,
    zellij: PathBuf,
    stop: Arc<AtomicBool>,
    watch_errors: Arc<AtomicU32>,
    stderr_dir: tempfile::TempDir,
    stderr_truncated: Arc<AtomicBool>,
    stderr_reader_error: Arc<AtomicBool>,
    stderr_reader: Option<thread::JoinHandle<()>>,
    drain: Option<thread::JoinHandle<()>>,
    master: File,
    outer_slave_id: (u64, u64),
}

impl Session {
    fn watch_error_class(&self) -> &'static str {
        fixed_error_class(self.watch_errors.load(Ordering::Relaxed))
    }

    fn stderr_fifo(&self) -> PathBuf {
        self.stderr_dir.path().join("pipe")
    }

    fn child_stderr_summary(
        &self,
    ) -> (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
    ) {
        if self.stderr_reader_error.load(Ordering::Relaxed) {
            return (
                "capture_error",
                "untrusted",
                "untrusted",
                "untrusted",
                "untrusted",
                "untrusted",
            );
        }
        let path = self.stderr_dir.path().join("captured");
        let Ok(meta) = fs::symlink_metadata(&path) else {
            return (
                "capture_missing",
                "untrusted",
                "untrusted",
                "untrusted",
                "untrusted",
                "untrusted",
            );
        };
        if !meta.file_type().is_file() || meta.len() > STDERR_CAPTURE_CAP as u64 {
            return (
                "capture_invalid",
                "untrusted",
                "untrusted",
                "untrusted",
                "untrusted",
                "untrusted",
            );
        }
        let Ok(raw) = fs::read(&path) else {
            return (
                "capture_unreadable",
                "untrusted",
                "untrusted",
                "untrusted",
                "untrusted",
                "untrusted",
            );
        };
        if raw.len() > STDERR_CAPTURE_CAP {
            return (
                "capture_invalid",
                "untrusted",
                "untrusted",
                "untrusted",
                "untrusted",
                "untrusted",
            );
        }
        let state = if self.stderr_truncated.load(Ordering::Relaxed) {
            "truncated"
        } else {
            "bounded"
        };
        let class = if raw.is_empty() {
            "empty"
        } else {
            fixed_error_class(classify_watch_bytes(&raw))
        };
        let inner = if state == "bounded" && class == "pane_startup_failed" {
            fixed_inner_startup_class(&raw)
        } else {
            "not_classified"
        };
        let dimensions = if inner == "dimensions_unavailable" {
            (
                fixed_dimension_fd_class(&raw, b"stdout"),
                fixed_dimension_fd_class(&raw, b"stderr"),
                fixed_dimension_fd_class(&raw, b"stdin"),
            )
        } else {
            ("not_classified", "not_classified", "not_classified")
        };
        (
            state,
            class,
            inner,
            dimensions.0,
            dimensions.1,
            dimensions.2,
        )
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.zellij);
        command
            .env_clear()
            .env("HOME", &self.home)
            .env("TERM", "xterm-256color")
            .env("SHELL", "/bin/sh")
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.zellij.parent().unwrap().display()),
            )
            .arg("-s")
            .arg(&self.session);
        command
    }

    fn action(&self, args: &[&str]) -> bool {
        let mut command = self.command();
        command.arg("action").args(args);
        run_bounded(command)
    }

    fn dump(&self, root: &Path) -> Option<String> {
        let path = root.join("focused-pane.txt");
        let _ = fs::remove_file(&path);
        if !self.action(&["dump-screen", path.to_str()?]) {
            return None;
        }
        let metadata = fs::symlink_metadata(&path).ok()?;
        if !metadata.file_type().is_file() || metadata.len() > 32768 {
            return None;
        }
        fs::read_to_string(path).ok()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let mut command = self.command();
        command.arg("kill-session").arg(&self.session);
        let _ = run_bounded(command);
        unsafe { libc::kill(-(self.child.id() as i32), libc::SIGKILL) };
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.stop.store(true, Ordering::Relaxed);
        if let Some(drain) = self.drain.take() {
            let _ = drain.join();
        }
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
    }
}

fn duplicate(file: &File) -> File {
    let fd = unsafe { libc::dup(file.as_raw_fd()) };
    assert!(fd >= 0, "could not duplicate PTY slave");
    unsafe { File::from_raw_fd(fd) }
}

fn start_session(root: &Path, zellij: &Path, session: String) -> Session {
    let stderr_dir = tempfile::Builder::new()
        .prefix("watch-stderr-")
        .tempdir_in(zellij.parent().expect("Zellij tool directory"))
        .expect("private watch stderr directory");
    let stderr_pipe = stderr_dir.path().join("pipe");
    let pipe_c = CString::new(stderr_pipe.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(pipe_c.as_ptr(), 0o600) }, 0);
    let mut pipe_reader = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&stderr_pipe)
        .expect("private watch stderr pipe reader");
    let captured = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(stderr_dir.path().join("captured"))
        .expect("private bounded watch stderr capture");
    let mut master_fd = -1;
    let mut slave_fd = -1;
    let mut size = libc::winsize {
        ws_row: 12,
        ws_col: 90,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    assert_eq!(
        unsafe {
            libc::openpty(
                &mut master_fd,
                &mut slave_fd,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut size,
            )
        },
        0
    );
    let master = unsafe { File::from_raw_fd(master_fd) };
    let slave = unsafe { File::from_raw_fd(slave_fd) };
    use std::os::unix::fs::MetadataExt;
    let slave_meta = slave.metadata().expect("outer PTY slave metadata");
    let outer_slave_id = (slave_meta.dev(), slave_meta.ino());
    let stdin = duplicate(&slave);
    let stdout = duplicate(&slave);
    let stderr = duplicate(&slave);
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg("exec \"$BOP_ZELLIJ\" -s \"$BOP_SESSION\"")
        .env_clear()
        .env("HOME", root)
        .env("TERM", "xterm-256color")
        .env("SHELL", "/bin/sh")
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", zellij.parent().unwrap().display()),
        )
        .env("BOP_ZELLIJ", zellij)
        .env("BOP_SESSION", &session)
        .stdin(Stdio::from(stdin))
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 || libc::ioctl(libc::STDIN_FILENO, libc::TIOCSCTTY, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = command.spawn().expect("spawn Zellij under a real PTY");
    drop(slave);
    let mut reader = master.try_clone().unwrap();
    let flags = unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_GETFL) };
    assert!(flags >= 0);
    assert_eq!(
        unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) },
        0
    );
    let stop = Arc::new(AtomicBool::new(false));
    let drain_stop = stop.clone();
    let stderr_stop = stop.clone();
    let stderr_truncated = Arc::new(AtomicBool::new(false));
    let stderr_reader_error = Arc::new(AtomicBool::new(false));
    let reader_truncated = stderr_truncated.clone();
    let reader_error = stderr_reader_error.clone();
    let stderr_reader = thread::spawn(move || {
        let mut captured = captured;
        let mut saved = 0usize;
        let mut bytes = [0u8; 8192];
        let mut can_save = true;
        while !stderr_stop.load(Ordering::Relaxed) {
            match pipe_reader.read(&mut bytes) {
                Ok(0) => thread::sleep(POLL),
                Ok(n) => {
                    let keep = n.min(STDERR_CAPTURE_CAP.saturating_sub(saved));
                    if keep < n {
                        reader_truncated.store(true, Ordering::Relaxed);
                    }
                    if can_save && keep > 0 {
                        if captured.write_all(&bytes[..keep]).is_err() {
                            reader_error.store(true, Ordering::Relaxed);
                            can_save = false;
                        } else {
                            saved += keep;
                        }
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(POLL),
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    reader_error.store(true, Ordering::Relaxed);
                    break;
                }
            }
        }
    });
    let watch_errors = Arc::new(AtomicU32::new(0));
    let drain_errors = watch_errors.clone();
    let drain = thread::spawn(move || {
        let mut bytes = [0u8; 8192];
        let mut tail = Vec::with_capacity(128);
        let mut window = Vec::with_capacity(8192 + 128);
        while !drain_stop.load(Ordering::Relaxed) {
            match reader.read(&mut bytes) {
                Ok(0) => break,
                Ok(n) => {
                    window.clear();
                    window.extend_from_slice(&tail);
                    window.extend_from_slice(&bytes[..n]);
                    drain_errors.fetch_or(classify_watch_bytes(&window), Ordering::Relaxed);
                    tail.clear();
                    let start = window.len().saturating_sub(128);
                    tail.extend_from_slice(&window[start..]);
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(POLL),
                Err(_) => break,
            }
        }
    });
    Session {
        child,
        session,
        home: root.to_path_buf(),
        zellij: zellij.to_path_buf(),
        stop,
        watch_errors,
        stderr_dir,
        stderr_truncated,
        stderr_reader_error,
        stderr_reader: Some(stderr_reader),
        drain: Some(drain),
        master,
        outer_slave_id,
    }
}

fn fixed_tty_size(file: &File, expected: (u16, u16)) -> &'static str {
    let mut size = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    if unsafe { libc::ioctl(file.as_raw_fd(), libc::TIOCGWINSZ, &mut size) } != 0 {
        "unreadable"
    } else if (size.ws_row, size.ws_col) == expected {
        "expected"
    } else if size.ws_row == 0 || size.ws_col == 0 {
        "zero"
    } else {
        "other"
    }
}

fn fixed_proc_fd0(client_pid: i32, outer_slave_id: (u64, u64)) -> &'static str {
    use std::os::unix::fs::MetadataExt;
    match fs::metadata(format!("/proc/{client_pid}/fd/0")) {
        Ok(meta) if (meta.dev(), meta.ino()) == outer_slave_id => "outer",
        Ok(_) => "other",
        Err(_) => "unreadable",
    }
}

fn shell_live(root: &Path) -> bool {
    let Ok(pid) = fs::read_to_string(root.join("shell-pid")) else {
        return false;
    };
    let Ok(pid) = pid.trim().parse::<i32>() else {
        return false;
    };
    let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    let (Ok(actual), Ok(expected)) = (
        fs::canonicalize(format!("/proc/{pid}/exe")),
        fs::canonicalize("/bin/sh"),
    ) else {
        return false;
    };
    if actual != expected {
        return false;
    }
    stat.split(") ")
        .nth(1)
        .is_some_and(|tail| !tail.starts_with('Z'))
}

fn beat_len(root: &Path) -> u64 {
    fs::metadata(root.join("shell-beat"))
        .map(|m| m.len())
        .unwrap_or(0)
}

fn watch_ttys(root: &Path) -> Option<Vec<(u64, u64)>> {
    let bop = fs::canonicalize(env!("CARGO_BIN_EXE_bop")).ok()?;
    let mut found = Vec::new();
    for entry in fs::read_dir("/proc").ok()?.flatten() {
        let pid = entry.file_name();
        if !pid
            .to_string_lossy()
            .bytes()
            .all(|byte| byte.is_ascii_digit())
        {
            continue;
        }
        if fs::canonicalize(entry.path().join("exe")).ok().as_deref() != Some(&bop) {
            continue;
        }
        let Ok(args) = fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        if !args
            .split(|byte| *byte == 0)
            .any(|arg| arg == b"__watch-pane")
            || !args
                .split(|byte| *byte == 0)
                .any(|arg| arg == root.as_os_str().as_encoded_bytes())
        {
            continue;
        }
        let tty = fs::metadata(entry.path().join("fd/1")).ok()?;
        use std::os::unix::fs::MetadataExt;
        found.push((tty.dev(), tty.ino()));
    }
    Some(found)
}

fn watch_tty(root: &Path) -> Option<(u64, u64)> {
    let mut found = watch_ttys(root)?;
    if found.len() == 1 {
        found.pop()
    } else {
        None
    }
}

fn watch_child_exit(root: &Path) -> String {
    match fs::read_to_string(root.join("watch-child-exit")) {
        Ok(status) if status.trim() == "0" => "success".into(),
        Ok(status) => match status.trim().parse::<u8>() {
            Ok(code) => format!("exit_{code}"),
            Err(_) => "invalid".into(),
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => "absent".into(),
        Err(_) => "unreadable".into(),
    }
}

fn fixed_marker(root: &Path, name: &str) -> &'static str {
    match fs::read_to_string(root.join(name)) {
        Ok(value) if value == "yes" => "yes",
        Ok(value) if value == "no" => "no",
        Ok(_) => "invalid",
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => "absent",
        Err(_) => "unreadable",
    }
}

fn fixed_pre_stty_size(root: &Path) -> &'static str {
    match fs::read_to_string(root.join("watch-pane-pre-stty-size")) {
        Ok(value) if value == "positive" => "positive",
        Ok(value) if value == "zero" => "zero",
        Ok(value) if value == "unavailable" => "unavailable",
        Ok(_) => "invalid",
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => "absent",
        Err(_) => "unreadable",
    }
}

fn report_watch_probe(cards: &Path, temp_root: &Path, origin_id: (u64, u64), session: &Session) {
    // Diagnostics only: this never supplies a pane ID to the acceptance gate.
    let (stderr_state, stderr_class, inner_class, size_stdout, size_stderr, size_stdin) =
        session.child_stderr_summary();
    let Ok(bop) = fs::canonicalize(env!("CARGO_BIN_EXE_bop")) else {
        eprintln!(
            "watch probe: executable=unreadable child_exit={} stderr_state={} stderr_class={} inner_class={} size_stdout={} size_stderr={} size_stdin={} pre_stty_size={}",
            watch_child_exit(temp_root),
            stderr_state,
            stderr_class,
            inner_class,
            size_stdout,
            size_stderr,
            size_stdin,
            fixed_pre_stty_size(temp_root)
        );
        return;
    };
    let Ok(entries) = fs::read_dir("/proc") else {
        eprintln!(
            "watch probe: proc=unreadable child_exit={} stderr_state={} stderr_class={} inner_class={} size_stdout={} size_stderr={} size_stdin={} pre_stty_size={}",
            watch_child_exit(temp_root),
            stderr_state,
            stderr_class,
            inner_class,
            size_stdout,
            size_stderr,
            size_stdin,
            fixed_pre_stty_size(temp_root)
        );
        return;
    };
    let mut processes = 0usize;
    let mut executable = 0usize;
    let mut command_unreadable = 0usize;
    let mut status_watch = 0usize;
    let mut helper = 0usize;
    let mut cards_match = 0usize;
    let mut stdout_fd_meta = 0usize;
    let mut distinct_fd_id = 0usize;
    let mut proc_scan_truncated = false;
    use std::os::unix::fs::MetadataExt;
    for entry in entries.flatten() {
        let pid = entry.file_name();
        if !pid
            .to_string_lossy()
            .bytes()
            .all(|byte| byte.is_ascii_digit())
        {
            continue;
        }
        if processes == 10000 {
            proc_scan_truncated = true;
            break;
        }
        processes = processes.saturating_add(1);
        if fs::canonicalize(entry.path().join("exe")).ok().as_deref() != Some(&bop) {
            continue;
        }
        executable = executable.saturating_add(1);
        let Ok(args) = fs::read(entry.path().join("cmdline")) else {
            command_unreadable = command_unreadable.saturating_add(1);
            continue;
        };
        if args.split(|byte| *byte == 0).any(|arg| arg == b"status")
            && args.split(|byte| *byte == 0).any(|arg| arg == b"--watch")
        {
            status_watch = status_watch.saturating_add(1);
        }
        if !args
            .split(|byte| *byte == 0)
            .any(|arg| arg == b"__watch-pane")
        {
            continue;
        }
        helper = helper.saturating_add(1);
        if !args
            .split(|byte| *byte == 0)
            .any(|arg| arg == cards.as_os_str().as_encoded_bytes())
        {
            continue;
        }
        cards_match = cards_match.saturating_add(1);
        if let Ok(tty) = fs::metadata(entry.path().join("fd/1")) {
            stdout_fd_meta = stdout_fd_meta.saturating_add(1);
            if (tty.dev(), tty.ino()) != origin_id {
                distinct_fd_id = distinct_fd_id.saturating_add(1);
            }
        }
    }
    eprintln!(
        "watch probe: child_exit={} error_class={} stderr_state={} stderr_class={} inner_class={} size_stdout={} size_stderr={} size_stdin={} pre_stty_size={} session_env={} session_match={} shell_stdout_tty={} zellij_on_path={} dev_tty_openable={} pane_size_ready={} shell_live={} beat_nonempty={} proc_scan_truncated={} proc={} bop_exe={} cmdline_unreadable={} status_watch={} helper={} cards_match={} stdout_fd_meta={} distinct_fd_id={}",
        watch_child_exit(temp_root),
        session.watch_error_class(),
        stderr_state,
        stderr_class,
        inner_class,
        size_stdout,
        size_stderr,
        size_stdin,
        fixed_pre_stty_size(temp_root),
        fixed_marker(temp_root, "watch-session-env"),
        fixed_marker(temp_root, "watch-session-match"),
        fixed_marker(temp_root, "watch-shell-stdout-tty"),
        fixed_marker(temp_root, "watch-zellij-on-path"),
        fixed_marker(temp_root, "watch-dev-tty-openable"),
        fixed_marker(temp_root, "watch-pane-size-ready"),
        shell_live(temp_root),
        beat_len(temp_root) > 0,
        proc_scan_truncated,
        processes,
        executable,
        command_unreadable,
        status_watch,
        helper,
        cards_match,
        stdout_fd_meta,
        distinct_fd_id,
    );
}

#[test]
#[ignore = "requires reviewed disposable Linux PTY/Zellij toolchain and hosted artifact audit"]
fn physical_zellij_watch_preserves_invoking_shell_and_erases_deleted_cells() {
    let zellij =
        PathBuf::from(std::env::var_os("BOP_TEST_ZELLIJ").expect("pinned Zellij path required"));
    assert!(zellij.is_absolute() && zellij.is_file());
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let cards = root.join(".cards");
    let old = cards.join("running/deleted.bop");
    let keep = cards.join("running/remaining.bop");
    fs::create_dir_all(&old).unwrap();
    fs::create_dir_all(&keep).unwrap();
    bop_core::write_meta(
        &old,
        &bop_core::Meta {
            id: "DELETED_UNIQUE_7Q".into(),
            stage: "implement".into(),
            title: Some("Old card across a wide terminal with OLD_TAIL_7Q".into()),
            token: Some("⚀".into()),
            ..Default::default()
        },
    )
    .unwrap();
    bop_core::write_meta(
        &keep,
        &bop_core::Meta {
            id: "REMAINING_UNIQUE_8R".into(),
            stage: "implement".into(),
            title: Some("Remaining card".into()),
            token: Some("⚁".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let session_name = format!("bop-pty-{}", std::process::id());
    let mut session = start_session(root, &zellij, session_name);
    // v0.43.1's dump-screen/write actions address the focused pane only.
    // A successful dump can be a plugin, so identify the explicit shell by
    // its private PID/TTY and have that same shell launch the watch command.
    until("Zellij session", || session.dump(root));
    // openpty supplied 12x90 before Zellij started. Check the outer size;
    // the differential resize below must occur after the invoking pane exists.
    let mut outer_size = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    assert_eq!(
        unsafe {
            libc::ioctl(
                session.master.as_raw_fd(),
                libc::TIOCGWINSZ,
                &mut outer_size,
            )
        },
        0,
        "outer PTY size unavailable"
    );
    assert_eq!(
        (outer_size.ws_row, outer_size.ws_col),
        (12, 90),
        "outer PTY initial size incorrect"
    );
    let stderr_fifo = session.stderr_fifo();
    assert!(
        session.action(&[
            "new-pane",
            "--in-place",
            "--",
            "/bin/sh",
            "-c",
            "printf '%s' \"$$\" > \"$1/invoker-start-pid\"; tty > \"$1/invoker-start-tty\"; printf '%s' \"$$\" > \"$1/shell-pid\"; tty > \"$1/origin-tty\"; printf x >> \"$1/shell-beat\"; if [ -n \"${ZELLIJ_SESSION_NAME:-}\" ]; then printf yes > \"$1/watch-session-env\"; else printf no > \"$1/watch-session-env\"; fi; if [ \"${ZELLIJ_SESSION_NAME:-}\" = \"$4\" ]; then printf yes > \"$1/watch-session-match\"; else printf no > \"$1/watch-session-match\"; fi; if [ -t 1 ]; then printf yes > \"$1/watch-shell-stdout-tty\"; else printf no > \"$1/watch-shell-stdout-tty\"; fi; if command -v zellij >/dev/null 2>&1; then printf yes > \"$1/watch-zellij-on-path\"; else printf no > \"$1/watch-zellij-on-path\"; fi; if (: < /dev/tty) 2>/dev/null; then printf yes > \"$1/watch-dev-tty-openable\"; else printf no > \"$1/watch-dev-tty-openable\"; fi; pre_size=\"$(stty size < /dev/tty 2>/dev/null || true)\"; pre_class=unavailable; case \"$pre_size\" in *\" \"*) pre_rows=\"${pre_size%% *}\"; pre_cols=\"${pre_size#* }\"; case \"$pre_rows:$pre_cols\" in *[!0-9:]*|:*|*:) ;; *) if [ \"$pre_rows\" -gt 0 ] && [ \"$pre_cols\" -gt 0 ]; then pre_class=positive; else pre_class=zero; fi ;; esac ;; esac; printf '%s' \"$pre_class\" > \"$1/watch-pane-pre-stty-size\"; size_count=0; while [ ! -f \"$1/watch-launch\" ] && [ \"$size_count\" -lt 250 ]; do pane_size=\"$(stty size < /dev/tty 2>/dev/null || true)\"; if [ \"$pane_size\" = \"13 91\" ]; then printf yes > \"$1/watch-pane-changed-size\"; fi; if [ -f \"$1/watch-pane-changed-size\" ] && [ \"$pane_size\" = \"12 90\" ]; then printf yes > \"$1/watch-pane-restored-size\"; fi; printf x >> \"$1/shell-beat\"; sleep 0.1; size_count=$((size_count + 1)); done; if [ ! -f \"$1/watch-launch\" ] || [ ! -f \"$1/watch-pane-changed-size\" ] || [ ! -f \"$1/watch-pane-restored-size\" ]; then exit 1; fi; printf yes > \"$1/watch-pane-size-ready\"; { \"$2\" --cards-dir \"$3\" status --watch < /dev/tty 2> \"$5\"; printf '%s' \"$?\" > \"$1/watch-child-exit\"; } & while :; do printf x >> \"$1/shell-beat\"; sleep 0.1; done",
            "bop-invoker",
            root.to_str().expect("temp path must be UTF-8"),
            env!("CARGO_BIN_EXE_bop"),
            cards.to_str().expect("cards path must be UTF-8"),
            &session.session,
            stderr_fifo.to_str().expect("stderr pipe path must be UTF-8"),
        ]),
        "Zellij did not create invoking shell pane"
    );
    let startup = until("invoking shell startup PID and TTY", || {
        let pid = fs::read_to_string(root.join("invoker-start-pid"))
            .ok()?
            .trim()
            .parse::<i32>()
            .ok()?;
        let tty = fs::read_to_string(root.join("invoker-start-tty")).ok()?;
        (pid > 0 && tty.trim().starts_with("/dev/pts/")).then_some((pid, tty))
    });
    // The invoking shell itself records the origin and keeps the beat ticking
    // while its child opens the mediated watch pane.
    let deadline = Instant::now() + TIMEOUT;
    while !(shell_live(root) && beat_len(root) > 2) && Instant::now() < deadline {
        thread::sleep(POLL);
    }
    if !(shell_live(root) && beat_len(root) > 2) {
        let pid_file = fs::read_to_string(root.join("shell-pid")).ok();
        let child_state = match session.child.try_wait() {
            Ok(None) => "running",
            Ok(Some(_)) => "exited",
            Err(_) => "unknown",
        };
        eprintln!(
            "physical shell readiness: startup_pid_tty=true pid_file={} pid_parseable={} shell_live={} beat_bytes={} origin_file={} zellij_child_state={}",
            pid_file.is_some(),
            pid_file
                .as_ref()
                .is_some_and(|pid| pid.trim().parse::<i32>().is_ok()),
            shell_live(root),
            beat_len(root),
            root.join("origin-tty").is_file(),
            child_state,
        );
        panic!("timed out waiting for invoking shell and ticking sentinel");
    }
    let origin = fs::read_to_string(root.join("origin-tty")).unwrap();
    let shell_pid = fs::read_to_string(root.join("shell-pid"))
        .unwrap()
        .trim()
        .parse::<i32>()
        .unwrap();
    assert!(
        shell_pid == startup.0,
        "watch launcher came from another shell"
    );
    assert!(
        origin.trim() == startup.1.trim(),
        "watch launcher came from another tty"
    );
    let origin_path = Path::new(origin.trim());
    assert!(
        origin_path.starts_with("/dev/pts/"),
        "invoking shell lacks real tty"
    );
    use std::os::unix::fs::MetadataExt;
    let origin_meta = fs::metadata(origin_path).unwrap();
    let origin_id = (origin_meta.dev(), origin_meta.ino());
    let client_pid = session.child.id() as i32;
    let client_running = matches!(session.child.try_wait(), Ok(None));
    let client_executable = fs::canonicalize(format!("/proc/{client_pid}/exe")).ok();
    let pinned_executable = fs::canonicalize(&session.zellij).ok();
    let client_class = if !client_running {
        "gone"
    } else if pinned_executable.is_some() && client_executable == pinned_executable {
        "matched"
    } else {
        "mismatch"
    };
    let pgid_class = if unsafe { libc::getpgid(client_pid) } == client_pid {
        "matched"
    } else {
        "mismatch"
    };
    let client_fd0 = File::open(format!("/proc/{client_pid}/fd/0")).ok();
    let fd0_class = match client_fd0.as_ref().and_then(|fd| fd.metadata().ok()) {
        Some(meta) if (meta.dev(), meta.ino()) == session.outer_slave_id => "outer",
        Some(_) => "other",
        None => "unreadable",
    };
    let initial_fd0_size = client_fd0
        .as_ref()
        .map(|fd| fixed_tty_size(fd, (12, 90)))
        .unwrap_or("unreadable");
    eprintln!(
        "physical client preflight: client={} pgid={} fd0={} fd0_initial={}",
        client_class, pgid_class, fd0_class, initial_fd0_size,
    );
    assert!(
        client_class == "matched"
            && pgid_class == "matched"
            && fd0_class == "outer"
            && initial_fd0_size == "expected",
        "attached Zellij client preflight failed"
    );
    let client_fd0 = client_fd0.expect("matched client FD 0 must be readable");
    // Zellij v0.43.1 ignores unchanged resize events. Exercise two real outer
    // PTY size transitions and require the same invoking shell to read both
    // sizes from its own /dev/tty before it may launch the watch child.
    let mut changed_size = libc::winsize {
        ws_row: 13,
        ws_col: 91,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    assert_eq!(
        unsafe {
            libc::ioctl(
                session.master.as_raw_fd(),
                libc::TIOCSWINSZ,
                &mut changed_size,
            )
        },
        0,
        "outer PTY differential resize failed"
    );
    let master_changed_size = fixed_tty_size(&session.master, (13, 91));
    let client_changed_size = fixed_tty_size(&client_fd0, (13, 91));
    let current_fd0_class = fixed_proc_fd0(client_pid, session.outer_slave_id);
    eprintln!(
        "physical client resize source: master={} fd0={} fd0_identity={}",
        master_changed_size, client_changed_size, current_fd0_class,
    );
    assert!(
        master_changed_size == "expected"
            && client_changed_size == "expected"
            && current_fd0_class == "outer",
        "attached Zellij client resize source unavailable"
    );
    assert!(
        matches!(session.child.try_wait(), Ok(None))
            && fs::canonicalize(format!("/proc/{client_pid}/exe")).ok() == pinned_executable,
        "attached Zellij client changed before resize signal"
    );
    assert_eq!(
        unsafe { libc::kill(client_pid, libc::SIGWINCH) },
        0,
        "direct Zellij client resize notification failed"
    );
    let deadline = Instant::now() + Duration::from_secs(8);
    while fixed_marker(root, "watch-pane-changed-size") != "yes" && Instant::now() < deadline {
        thread::sleep(POLL);
    }
    if fixed_marker(root, "watch-pane-changed-size") != "yes" {
        eprintln!(
            "physical size handshake: stage=changed pre_stty={} changed={} restored={} shell_live={} beat_nonempty={}",
            fixed_pre_stty_size(root),
            fixed_marker(root, "watch-pane-changed-size"),
            fixed_marker(root, "watch-pane-restored-size"),
            shell_live(root),
            beat_len(root) > 0,
        );
        panic!("invoking shell did not observe differential PTY size");
    }
    let mut restored_size = libc::winsize {
        ws_row: 12,
        ws_col: 90,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    assert_eq!(
        unsafe {
            libc::ioctl(
                session.master.as_raw_fd(),
                libc::TIOCSWINSZ,
                &mut restored_size,
            )
        },
        0,
        "outer PTY restore failed"
    );
    let master_restored_size = fixed_tty_size(&session.master, (12, 90));
    let client_restored_size = fixed_tty_size(&client_fd0, (12, 90));
    let current_fd0_class = fixed_proc_fd0(client_pid, session.outer_slave_id);
    eprintln!(
        "physical client restore source: master={} fd0={} fd0_identity={}",
        master_restored_size, client_restored_size, current_fd0_class,
    );
    assert!(
        master_restored_size == "expected"
            && client_restored_size == "expected"
            && current_fd0_class == "outer",
        "attached Zellij client restore source unavailable"
    );
    assert!(
        matches!(session.child.try_wait(), Ok(None))
            && fs::canonicalize(format!("/proc/{client_pid}/exe")).ok() == pinned_executable,
        "attached Zellij client changed before restore signal"
    );
    assert_eq!(
        unsafe { libc::kill(client_pid, libc::SIGWINCH) },
        0,
        "direct Zellij client restore notification failed"
    );
    let deadline = Instant::now() + Duration::from_secs(8);
    while fixed_marker(root, "watch-pane-restored-size") != "yes" && Instant::now() < deadline {
        thread::sleep(POLL);
    }
    if fixed_marker(root, "watch-pane-restored-size") != "yes" {
        eprintln!(
            "physical size handshake: stage=restored pre_stty={} changed={} restored={} shell_live={} beat_nonempty={}",
            fixed_pre_stty_size(root),
            fixed_marker(root, "watch-pane-changed-size"),
            fixed_marker(root, "watch-pane-restored-size"),
            shell_live(root),
            beat_len(root) > 0,
        );
        panic!("invoking shell did not observe restored PTY size");
    }
    assert!(
        shell_live(root),
        "invoking shell exited before watch launch"
    );
    let beat_before = beat_len(root);
    until("invoking shell sentinel before watch launch", || {
        (shell_live(root) && beat_len(root) > beat_before).then_some(())
    });
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(root.join("watch-launch"))
        .unwrap()
        .write_all(b"go")
        .unwrap();
    let pane_id = {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(id) = watch_tty(&cards) {
                break id;
            }
            if Instant::now() >= deadline {
                report_watch_probe(&cards, root, origin_id, &session);
                panic!("timed out waiting for distinct live watch pane tty");
            }
            thread::sleep(POLL);
        }
    };
    assert_ne!(pane_id, origin_id, "watch reused the invoking shell PTY");
    let initial = {
        let deadline = Instant::now() + TIMEOUT;
        let mut dump_ok = 0u8;
        let mut dump_missing = 0u8;
        let mut ever_mask = 0u8;
        let mut last_mask = 0u8;
        let mut last_frame = "unavailable";
        loop {
            if let Some(screen) = session.dump(root) {
                dump_ok = dump_ok.saturating_add(1);
                last_mask = initial_cell_mask(&screen);
                ever_mask |= last_mask;
                last_frame = if screen.is_empty() {
                    "empty"
                } else {
                    "nonempty"
                };
                if last_mask == 0b1111 {
                    break screen;
                }
            } else {
                dump_missing = dump_missing.saturating_add(1);
                last_frame = "unavailable";
            }
            if Instant::now() >= deadline {
                report_watch_probe(&cards, root, origin_id, &session);
                eprintln!(
                    "initial watch cells: dump_ok={} dump_missing={} ever_mask={} last_mask={} last_frame={} watch_tty_same={} shell_live={} beat_nonempty={} child_exit={} error_class={}",
                    dump_ok,
                    dump_missing,
                    ever_mask,
                    last_mask,
                    last_frame,
                    watch_tty(&cards) == Some(pane_id),
                    shell_live(root),
                    beat_len(root) > 0,
                    watch_child_exit(root),
                    session.watch_error_class(),
                );
                panic!("timed out waiting for initial watch cells");
            }
            thread::sleep(POLL);
        }
    };
    assert!(initial.len() < 32768);
    let before = beat_len(root);
    until("invoking shell sentinel during watch", || {
        (shell_live(root) && beat_len(root) > before).then_some(())
    });
    let mut size = libc::winsize {
        ws_row: 12,
        ws_col: 38,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    assert_eq!(
        unsafe { libc::ioctl(session.master.as_raw_fd(), libc::TIOCSWINSZ, &mut size) },
        0
    );
    assert_eq!(
        unsafe { libc::kill(-(session.child.id() as i32), libc::SIGWINCH) },
        0
    );
    until("narrow Zellij watch cells", || {
        let screen = session.dump(root)?;
        (screen.contains("DELETED_UNIQUE_7Q")
            && screen.contains("REMAINING_UNIQUE_8R")
            && !screen.contains("OLD_TAIL_7Q")
            && watch_tty(&cards) == Some(pane_id))
        .then_some(())
    });
    fs::remove_dir_all(&old).unwrap();
    let before_delete = beat_len(root);
    until("deleted cells cleared from real watch pane", || {
        let screen = session.dump(root)?;
        (screen.contains("REMAINING_UNIQUE_8R")
            && screen.contains("watching .cards/")
            && !screen.contains("DELETED_UNIQUE_7Q")
            && !screen.contains("OLD_TAIL_7Q")
            && !screen.contains('⚀')
            && watch_tty(&cards) == Some(pane_id)
            && shell_live(root)
            && beat_len(root) > before_delete)
            .then_some(())
    });
    assert!(
        session.action(&["write", "3"]),
        "Ctrl-C was not delivered to watch pane"
    );
    until("watch exit after Ctrl-C", || {
        watch_ttys(&cards)
            .is_some_and(|ttys| ttys.is_empty())
            .then_some(())
    });
    let after = beat_len(root);
    until("invoking shell survival after Ctrl-C", || {
        (shell_live(root) && beat_len(root) > after).then_some(())
    });
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(cards.join(".locks/watch.lock"))
        .unwrap();
    assert_eq!(
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0,
        "worker lock retained after Ctrl-C"
    );
    assert!(
        session.child_stderr_summary().0 == "bounded",
        "child stderr diagnostic capture incomplete"
    );
    println!("physical PTY, distinct watch tty, live shell sentinel, create/resize/delete cells, Ctrl-C and lock release verified");
}
