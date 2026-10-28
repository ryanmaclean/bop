//! Native worker lifecycle fixture. Run only on an approved build host.

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::net::UnixListener;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[cfg(unix)]
use notify::Watcher;
#[cfg(unix)]
use notify_debouncer_mini::new_debouncer;

const READY: u8 = 1;
const EVENT: u8 = 2;

#[cfg(unix)]
struct ReapWorkerOnDrop(std::process::Child);

#[cfg(unix)]
impl Drop for ReapWorkerOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(unix)]
#[test]
fn direct_pane_without_launch_capability_refuses_before_drawing() {
    let temp = tempfile::tempdir().unwrap();
    let cards = temp.path().join(".cards");
    fs::create_dir(&cards).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_bop"))
        .arg("--cards-dir")
        .arg(&cards)
        .arg("__watch-pane")
        .args(["1", "2"])
        .arg(temp.path().join("absent.sock"))
        .arg("3")
        .env("ZELLIJ_SESSION_NAME", "fixture")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!output.stdout.contains(&0x1b));
}

#[cfg(unix)]
#[test]
fn pane_reports_initial_verification_failure_over_launch_socket() {
    let temp = tempfile::tempdir().unwrap();
    let cards = temp.path().join(".cards");
    fs::create_dir(&cards).unwrap();
    let socket = temp.path().join("launch.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let token = 0x123456789abcdef0u128;
    let mut child = Command::new(env!("CARGO_BIN_EXE_bop"))
        .arg("--cards-dir")
        .arg(&cards)
        .arg("__watch-pane")
        .args(["1", "2"])
        .arg(&socket)
        .arg(token.to_string())
        .env("ZELLIJ_SESSION_NAME", "fixture")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "pane did not connect");
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(err) => panic!("launch socket accept failed: {err}"),
        }
    };
    // The EOF assertion below needs a blocking stream even if accept inherited the listener mode.
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut received = [0u8; 16];
    stream.read_exact(&mut received).unwrap();
    assert_eq!(received, token.to_be_bytes());
    stream.write_all(&[1]).unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    assert_eq!(response.first().copied(), Some(3));
    assert!(String::from_utf8_lossy(&response).contains("requires a Zellij-owned terminal"));
    let deadline = Instant::now() + Duration::from_secs(2);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("failed watch pane did not exit");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let mut stdout = Vec::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut stdout)
        .unwrap();
    assert!(!stdout.contains(&0x1b));
    assert!(!status.success());
}

#[test]
#[cfg(unix)]
#[ignore = "requires approved native host and selected license/tool closure"]
fn worker_registers_events_and_releases_after_parent_pipe_closes() {
    let temp = tempfile::tempdir().unwrap();
    let cards = temp.path().join(".cards");
    fs::create_dir_all(cards.join("pending")).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_bop"))
        .arg("--cards-dir")
        .arg(&cards)
        .arg("__watch-worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut byte = [0u8; 1];
        while stdout.read_exact(&mut byte).is_ok() {
            if tx.send(byte[0]).is_err() {
                break;
            }
        }
    });

    let lock_path = cards.join(".locks/watch.lock");
    let result = (|| -> Result<(), String> {
        let ready = rx.recv_timeout(Duration::from_secs(5));
        if !matches!(ready, Ok(READY)) {
            return Err(format!("worker did not become ready: {ready:?}"));
        }
        if !lock_path.is_file() {
            return Err("worker did not own the singleton".into());
        }
        let probe = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|err| err.to_string())?;
        if unsafe { libc::flock(probe.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Err("a second writer acquired the live worker's lock".into());
        }
        let card = cards.join("pending").join("fixture.jobcard");
        fs::create_dir(&card).map_err(|err| err.to_string())?;
        fs::write(card.join("meta.json"), b"{}").map_err(|err| err.to_string())?;
        let event = rx.recv_timeout(Duration::from_secs(5));
        if !matches!(event, Ok(EVENT)) {
            return Err(format!("worker did not forward event: {event:?}"));
        }
        Ok(())
    })();

    drop(stdin);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("watch worker did not exit after parent pipe closed");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    reader.join().unwrap();
    // The lock file stays in place; the kernel releases its lock on exit.
    assert!(lock_path.is_file());
    let probe = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    assert_eq!(
        unsafe { libc::flock(probe.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    result.unwrap();
}

/// The worker's IPC event is pathless, but a preexisting card bundle leaves
/// the metadata write as the only post-readiness filesystem stimulus.
#[test]
#[cfg(unix)]
#[ignore = "requires approved native host and selected license/tool closure"]
fn worker_ipc_forwards_preexisting_card_metadata_write_and_releases_lock() {
    let temp = tempfile::tempdir().unwrap();
    let cards = temp.path().join(".cards");
    let card = cards.join("pending/fixture.jobcard");
    fs::create_dir_all(&card).unwrap();
    let lock_path = cards.join(".locks/watch.lock");
    let metadata = card.join("meta.json");

    let mut child = ReapWorkerOnDrop(
        Command::new(env!("CARGO_BIN_EXE_bop"))
            .arg("--cards-dir")
            .arg(&cards)
            .arg("__watch-worker")
            .env_clear()
            .env("HOME", temp.path())
            .env("TMPDIR", temp.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let stdin = child.0.stdin.take().unwrap();
    let mut stdout = child.0.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut byte = [0u8; 1];
        while stdout.read_exact(&mut byte).is_ok() {
            if tx.send(byte[0]).is_err() {
                break;
            }
        }
    });

    assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(READY));
    assert!(lock_path.is_file(), "worker did not create singleton lock");
    let probe = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    assert_ne!(
        unsafe { libc::flock(probe.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0,
        "second writer acquired live worker lock"
    );
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(300)),
        Err(mpsc::RecvTimeoutError::Timeout),
        "worker emitted an event before metadata write"
    );

    fs::write(&metadata, b"{\"fixture\":\"worker-ipc\"}").unwrap();
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(5)),
        Ok(EVENT),
        "worker did not forward metadata stimulus"
    );
    assert_eq!(
        fs::read(&metadata).unwrap(),
        b"{\"fixture\":\"worker-ipc\"}"
    );

    drop(stdin);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if child.0.try_wait().unwrap().is_some() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "worker did not exit after parent pipe closed"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    reader.join().unwrap();
    let probe = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    assert_eq!(
        unsafe { libc::flock(probe.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0,
        "worker lock was not released"
    );
    println!("worker_ipc ready=true metadata_event=true lock_released=true");
}

/// Path-aware native witness for newly created subtrees. The worker's EVENT
/// byte intentionally carries no path, so its lifecycle fixture cannot prove
/// that a particular metadata write was the event it forwarded.
#[cfg(unix)]
#[test]
#[ignore = "requires approved native host and selected license/tool closure"]
fn recursive_notify_observes_metadata_under_new_team_directory() {
    let temp = tempfile::tempdir().unwrap();
    let cards = temp.path().join(".cards");
    fs::create_dir(&cards).unwrap();
    let (tx, rx) = mpsc::channel();
    let mut debouncer = new_debouncer(Duration::from_millis(100), tx).unwrap();
    debouncer
        .watcher()
        .watch(&cards, notify::RecursiveMode::Recursive)
        .unwrap();

    let metadata = cards
        .join("team-cli")
        .join("pending")
        .join("fresh.jobcard")
        .join("meta.json");
    fs::create_dir_all(metadata.parent().unwrap()).unwrap();

    let started = Instant::now();
    let mut traced = 0usize;
    let mut suppressed = 0usize;
    let mut receive_until = |target: &std::path::Path, deadline: Instant| {
        let mut matched = false;
        while Instant::now() < deadline {
            match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(Ok(events)) => {
                    for event in events {
                        if traced < 64 {
                            let path_label = event
                                .path
                                .strip_prefix(&cards)
                                .ok()
                                .filter(|relative| {
                                    relative.components().all(|component| {
                                        matches!(component, std::path::Component::Normal(_))
                                    })
                                })
                                .and_then(|relative| relative.to_str())
                                .filter(|relative| {
                                    !relative.is_empty()
                                        && relative.bytes().all(|byte| {
                                            byte.is_ascii_alphanumeric()
                                                || matches!(byte, b'/' | b'.' | b'_' | b'-')
                                        })
                                })
                                .unwrap_or("<outside-or-unsafe-path>");
                            println!(
                                "native_event elapsed_ms={} kind={:?} path={}",
                                started.elapsed().as_millis(),
                                event.kind,
                                path_label
                            );
                            traced += 1;
                        } else {
                            suppressed += 1;
                        }
                        matched |= event.path.as_path() == target;
                    }
                    if matched {
                        break;
                    }
                }
                Ok(Err(err)) => panic!("native watch error: {err}"),
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => panic!("native watcher disconnected"),
            }
        }
        matched
    };
    // Directory creation and inotify's recursive watch registration are not
    // atomic. Establish an observable event from inside the new bundle before
    // testing a distinct metadata path. Each retry is driven by a bounded
    // receive timeout; no fixed sleep or pathless event can satisfy the gate.
    let ready_probe = metadata.with_file_name("ready-probe.json");
    let ready_deadline = Instant::now() + Duration::from_secs(5);
    let mut attempt = 0usize;
    let mut ready = false;
    while Instant::now() < ready_deadline && !ready {
        attempt += 1;
        fs::write(&ready_probe, format!("probe-{attempt}")).unwrap();
        let window = (Instant::now() + Duration::from_millis(400)).min(ready_deadline);
        ready = receive_until(&ready_probe, window);
    }
    println!("native_watch_ready observed={ready} attempts={attempt}");
    assert!(
        ready,
        "no path-attributed event inside new bundle before deadline"
    );

    fs::write(&metadata, b"{\"fixture\":\"new-subtree\"}").unwrap();
    let observed = receive_until(&metadata, Instant::now() + Duration::from_secs(5));
    // A separately named late probe distinguishes a target-specific miss
    // after readiness from a broader loss of subsequent subtree events.
    let late_probe = metadata.with_file_name("late-probe.json");
    let late_observed = if observed {
        false
    } else {
        fs::write(&late_probe, b"late-probe").unwrap();
        receive_until(&late_probe, Instant::now() + Duration::from_secs(3))
    };
    println!(
        "native_event_summary ready={ready} metadata_after_ready={observed} late_probe={late_observed} traced={traced} suppressed={suppressed}"
    );
    assert!(
        observed,
        "no path-attributed metadata event after observed subtree readiness; late_probe={late_observed}"
    );
    assert_eq!(
        fs::read(&metadata).unwrap(),
        b"{\"fixture\":\"new-subtree\"}"
    );
}
