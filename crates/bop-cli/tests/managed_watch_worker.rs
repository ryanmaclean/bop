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

const READY: u8 = 1;
const EVENT: u8 = 2;

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
        // Let creation settle, then write metadata below a directory that did
        // not exist when the recursive watcher registered. A stale creation
        // event cannot satisfy the second assertion.
        let team = cards.join("team-cli").join("pending");
        fs::create_dir_all(&team).map_err(|err| err.to_string())?;
        std::thread::sleep(Duration::from_millis(300));
        while rx.try_recv().is_ok() {}
        let nested = team.join("new.jobcard");
        fs::create_dir(&nested).map_err(|err| err.to_string())?;
        std::thread::sleep(Duration::from_millis(300));
        while rx.try_recv().is_ok() {}
        fs::write(nested.join("meta.json"), b"{\"progress\":42}").map_err(|err| err.to_string())?;
        let nested_event = rx.recv_timeout(Duration::from_secs(5));
        if !matches!(nested_event, Ok(EVENT)) {
            return Err(format!(
                "worker missed metadata in newly created recursive directory: {nested_event:?}"
            ));
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
