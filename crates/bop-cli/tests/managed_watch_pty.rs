//! Real PTY/Zellij watch fixture. Run only on an approved disposable Linux host.

#![cfg(target_os = "linux")]

use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(20);
const POLL: Duration = Duration::from_millis(100);

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
    drain: Option<thread::JoinHandle<()>>,
    master: File,
}

impl Session {
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

    fn type_line(&self, line: &str) {
        assert!(
            self.action(&["write-chars", line]),
            "Zellij did not accept shell input"
        );
        assert!(self.action(&["write", "13"]), "Zellij did not accept Enter");
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
    }
}

fn duplicate(file: &File) -> File {
    let fd = unsafe { libc::dup(file.as_raw_fd()) };
    assert!(fd >= 0, "could not duplicate PTY slave");
    unsafe { File::from_raw_fd(fd) }
}

fn start_session(root: &Path, zellij: &Path, session: String) -> Session {
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
        .env("BOP_BEAT", root.join("shell-beat"))
        .env("BOP_SHELL_PID", root.join("shell-pid"))
        .env("BOP_ORIGIN", root.join("origin-tty"))
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
    let drain = thread::spawn(move || {
        let mut bytes = [0u8; 8192];
        while !drain_stop.load(Ordering::Relaxed) {
            match reader.read(&mut bytes) {
                Ok(0) => break,
                Ok(_) => {}
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
        drain: Some(drain),
        master,
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
    // its private startup PID/TTY and join the typed sentinel to both.
    until("Zellij session", || session.dump(root));
    assert!(
        session.action(&[
            "new-pane",
            "--in-place",
            "--",
            "/bin/sh",
            "-c",
            "printf '%s' \"$$\" > \"$1/invoker-start-pid\"; tty > \"$1/invoker-start-tty\"; printf 'BOP_INVOKER_READY_9V\\n'; exec /bin/sh",
            "bop-invoker",
            root.to_str().expect("temp path must be UTF-8"),
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
    // The marker is useful diagnostically, but a focused dump need not show
    // this pane. The same PID/TTY join below is the required pane identity.
    let focused_marker = session
        .dump(root)
        .is_some_and(|screen| screen.contains("BOP_INVOKER_READY_9V"));
    session.type_line("printf '%s' \"$$\" > \"$BOP_SHELL_PID\"; tty > \"$BOP_ORIGIN\"; (while :; do printf x >> \"$BOP_BEAT\"; sleep 0.1; done) &");
    let deadline = Instant::now() + TIMEOUT;
    while !(shell_live(root) && beat_len(root) > 2) && Instant::now() < deadline {
        thread::sleep(POLL);
    }
    if !(shell_live(root) && beat_len(root) > 2) {
        let pid_file = fs::read_to_string(root.join("shell-pid")).ok();
        let focused = session.dump(root);
        let child_state = match session.child.try_wait() {
            Ok(None) => "running",
            Ok(Some(_)) => "exited",
            Err(_) => "unknown",
        };
        eprintln!(
            "physical shell readiness: startup_pid_tty=true focused_marker={} pid_file={} pid_parseable={} shell_live={} beat_bytes={} origin_file={} focused_dump={} command_echo={} zellij_child_state={}",
            focused_marker,
            pid_file.is_some(),
            pid_file
                .as_ref()
                .is_some_and(|pid| pid.trim().parse::<i32>().is_ok()),
            shell_live(root),
            beat_len(root),
            root.join("origin-tty").is_file(),
            focused.is_some(),
            focused.as_ref().is_some_and(|screen| screen.contains("BOP_SHELL_PID")),
            child_state,
        );
        panic!("timed out waiting for invoking shell and ticking sentinel");
    }
    let origin = fs::read_to_string(root.join("origin-tty")).unwrap();
    let typed_pid = fs::read_to_string(root.join("shell-pid"))
        .unwrap()
        .trim()
        .parse::<i32>()
        .unwrap();
    assert_eq!(
        typed_pid, startup.0,
        "typed sentinel came from another shell"
    );
    assert_eq!(
        origin.trim(),
        startup.1.trim(),
        "typed sentinel came from another tty"
    );
    let origin_path = Path::new(origin.trim());
    assert!(
        origin_path.starts_with("/dev/pts/"),
        "invoking shell lacks real tty"
    );
    use std::os::unix::fs::MetadataExt;
    let origin_meta = fs::metadata(origin_path).unwrap();
    let origin_id = (origin_meta.dev(), origin_meta.ino());
    let command = format!(
        "{} --cards-dir {} status --watch",
        env!("CARGO_BIN_EXE_bop"),
        cards.display()
    );
    session.type_line(&command);
    let pane_id = until("distinct live watch pane tty", || watch_tty(&cards));
    assert_ne!(pane_id, origin_id, "watch reused the invoking shell PTY");
    let initial = until("initial watch cells", || {
        let screen = session.dump(root)?;
        (screen.contains("DELETED_UNIQUE_7Q")
            && screen.contains("REMAINING_UNIQUE_8R")
            && screen.contains("OLD_TAIL_7Q")
            && screen.contains("watching .cards/"))
        .then_some(screen)
    });
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
    println!("physical PTY, distinct watch tty, live shell sentinel, create/resize/delete cells, Ctrl-C and lock release verified");
}
