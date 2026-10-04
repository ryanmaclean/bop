//! Native BOP adapter for the separately built Moth `agent` CLI.
//!
//! BOP invokes this binary with exactly five positional paths. This adapter
//! deliberately does not accept a run-ID flag: BOP_RUN_ID is the identity
//! already persisted in the card's Meta.runs record before this process starts.
//! Moth must support `agent run --prompt-stdin` before this adapter ships.

use std::env;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;

const HELP_TIMEOUT: Duration = Duration::from_secs(3);
const HELP_MAX_BYTES: usize = 128 * 1024;

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("bop-moth-adapter: {error}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let args: Vec<OsString> = env::args_os().skip(1).collect();
    if args.len() != 5 {
        return Err(format!(
            "expected exactly five paths: workdir prompt_file stdout_log stderr_log memory_out; got {}",
            args.len()
        ));
    }

    let inherited_cwd = existing_dir(
        &env::current_dir().map_err(|error| format!("current directory: {error}"))?,
        "current directory",
    )?;
    let workdir = existing_dir(&absolute(&inherited_cwd, &args[0], "workdir")?, "workdir")?;
    let prompt_arg = absolute(&inherited_cwd, &args[1], "prompt_file")?;
    let stdout_log = absolute(&inherited_cwd, &args[2], "stdout_log")?;
    let stderr_log = absolute(&inherited_cwd, &args[3], "stderr_log")?;
    let memory_out = absolute(&inherited_cwd, &args[4], "memory_out")?;

    let card_env = required_path_env("BOP_CARD_DIR")?;
    let card_dir = existing_dir(&absolute_path(&inherited_cwd, card_env), "BOP_CARD_DIR")?;
    let logs_dir = existing_dir(&card_dir.join("logs"), "BOP_CARD_DIR/logs")?;
    if logs_dir != card_dir.join("logs") {
        return Err("BOP_CARD_DIR/logs must not redirect outside the card".into());
    }
    if logs_dir.join("moth").to_str().is_none() {
        return Err("Moth CLI path arguments must be valid UTF-8".into());
    }
    check_log_path(&stderr_log, &logs_dir, "stderr.log")?;
    let mut stderr_file = open_log(&stderr_log, "stderr_log")?;
    let outcome = run_validated(
        &workdir,
        &prompt_arg,
        &stdout_log,
        &memory_out,
        &card_dir,
        &logs_dir,
        &mut stderr_file,
    );
    if let Err(error) = &outcome {
        let _ = writeln!(stderr_file, "bop-moth-adapter: {error}");
    }
    outcome
}

fn run_validated(
    workdir: &Path,
    prompt_arg: &Path,
    stdout_log: &Path,
    memory_out: &Path,
    card_dir: &Path,
    logs_dir: &Path,
    stderr_file: &mut File,
) -> Result<ExitCode, String> {
    check_log_path(stdout_log, logs_dir, "stdout.log")?;
    check_memory_path(memory_out, card_dir)?;
    let prompt_metadata = fs::symlink_metadata(prompt_arg)
        .map_err(|error| format!("prompt_file: {error}"))?;
    if prompt_metadata.file_type().is_symlink() || !prompt_metadata.is_file() {
        return Err("prompt_file must be a regular non-symlink file".into());
    }
    let prompt_file = existing_file(prompt_arg, "prompt_file")?;
    if prompt_file.parent() != Some(card_dir) {
        return Err("prompt_file must be a regular file directly inside BOP_CARD_DIR".into());
    }
    let run_id = env::var("BOP_RUN_ID").map_err(|error| format!("BOP_RUN_ID: {error}"))?;
    if !valid_bop_run_id(&run_id) {
        return Err("BOP_RUN_ID must be exactly 32 lowercase hex characters".into());
    }
    let agent_raw = required_path_env("MOTH_AGENT_BIN")?;
    if !agent_raw.is_absolute() {
        return Err("MOTH_AGENT_BIN must be an absolute path".into());
    }
    let agent_bin = existing_file(&agent_raw, "MOTH_AGENT_BIN")?;
    let self_bin = existing_file(
        &env::current_exe().map_err(|error| format!("current executable: {error}"))?,
        "current executable",
    )?;
    if agent_bin == self_bin {
        return Err("MOTH_AGENT_BIN points back to bop-moth-adapter".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&agent_bin)
            .map_err(|error| format!("MOTH_AGENT_BIN metadata: {error}"))?
            .permissions()
            .mode();
        if mode & 0o111 == 0 {
            return Err("MOTH_AGENT_BIN is not executable".into());
        }
    }
    require_prompt_stdin_capability(&agent_bin)?;

    let mock = test_mock_mode(env::var_os("BOP_MOTH_TEST_MOCK"), cfg!(debug_assertions))?;
    let runlog_dir = prepare_runlog_dir(logs_dir)?;
    let receipt = runlog_dir.join(format!("{run_id}.jsonl"));
    match fs::symlink_metadata(&receipt) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Ok(_) => return Err("BOP_RUN_ID already has a runlog file; refusing to reuse it".into()),
        Err(error) => return Err(format!("runlog preflight: {error}")),
    }
    // Freeze the bytes before spawning Moth. This is a snapshot of the opened
    // file, not proof against a writer racing BOP's render/open boundary.
    let prompt = read_prompt_snapshot(&prompt_file)?;
    let stdout_file = open_log(stdout_log, "stdout_log")?;
    let child_stderr = stderr_file
        .try_clone()
        .map_err(|error| format!("stderr_log clone: {error}"))?;

    let mut child = Command::new(&agent_bin);
    child
        .arg("run")
        .arg("--runlog")
        .arg(&runlog_dir)
        .arg("--prompt-stdin")
        .current_dir(workdir)
        .env("BOP_RUN_ID", &run_id)
        .stdin(Stdio::piped())
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(child_stderr));
    if mock {
        child.arg("--mock");
    }

    let mut child = match child.spawn() {
        Ok(child) => child,
        Err(error) => return Err(format!("failed to launch MOTH_AGENT_BIN: {error}")),
    };
    let mut child_stdin = match child.stdin.take() {
        Some(stdin) => stdin,
        None => {
            terminate_and_reap(&mut child);
            return Err("Moth child has no stdin pipe".into());
        }
    };
    let write_result = child_stdin.write_all(&prompt);
    drop(child_stdin);
    if let Err(error) = write_result {
        terminate_and_reap(&mut child);
        return Err(format!("could not deliver complete prompt to Moth stdin: {error}"));
    }
    // ChildStdin is closed here, so Moth sees EOF before model
    // or runlog work under the paired --prompt-stdin contract.
    let status = match child.wait() {
        Ok(status) => status,
        Err(error) => {
            terminate_and_reap(&mut child);
            return Err(format!("wait for Moth: {error}"));
        }
    };
    let code = match status.code() {
        Some(code) => u8::try_from(code)
            .map_err(|_| format!("Moth returned an unsupported exit code: {code}"))?,
        None => return Err("Moth terminated without an exit code".into()),
    };
    if code == 0 {
        validate_success_receipt(&receipt, &run_id)?;
    }
    Ok(ExitCode::from(code))
}

enum ProbeEvent {
    Bytes(bool, Vec<u8>),
    Eof(bool),
    ReadError(String),
}

fn pump_probe(mut reader: impl Read + Send + 'static, is_stdout: bool, tx: SyncSender<ProbeEvent>) {
    let mut chunk = [0u8; 4096];
    loop {
        let event = match reader.read(&mut chunk) {
            Ok(0) => ProbeEvent::Eof(is_stdout),
            Ok(n) => ProbeEvent::Bytes(is_stdout, chunk[..n].to_vec()),
            Err(error) => ProbeEvent::ReadError(error.to_string()),
        };
        let terminal = !matches!(event, ProbeEvent::Bytes(_, _));
        if tx.send(event).is_err() || terminal {
            return;
        }
    }
}

fn terminate_and_reap(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn require_prompt_stdin_capability(agent_bin: &Path) -> Result<(), String> {
    // A legacy CLI can interpret an unknown option as literal prompt text.
    // Help is a bounded capability hint, not executable attestation.
    let mut child = Command::new(agent_bin)
        .arg("run")
        .arg("--help")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("MOTH_AGENT_BIN capability probe: {error}"))?;
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            terminate_and_reap(&mut child);
            return Err("help probe stdout pipe missing".into());
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            terminate_and_reap(&mut child);
            return Err("help probe stderr pipe missing".into());
        }
    };
    let (tx, rx): (SyncSender<ProbeEvent>, Receiver<ProbeEvent>) = mpsc::sync_channel(8);
    let out_thread = {
        let tx = tx.clone();
        thread::spawn(move || pump_probe(stdout, true, tx))
    };
    let err_thread = thread::spawn(move || pump_probe(stderr, false, tx));
    let started = Instant::now();
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    let mut total_bytes = 0usize;
    let mut eof_count = 0;
    let mut status = None;
    let result = (|| -> Result<(), String> {
        loop {
            if status.is_none() {
                status = child
                    .try_wait()
                    .map_err(|error| format!("help probe wait: {error}"))?;
            }
            if status.is_some() && eof_count == 2 {
                break;
            }
            let remaining = HELP_TIMEOUT.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err("Moth help capability probe timed out".into());
            }
            match rx.recv_timeout(remaining.min(Duration::from_millis(25))) {
                Ok(ProbeEvent::Bytes(is_stdout, chunk)) => {
                    let new_len = total_bytes.checked_add(chunk.len())
                        .ok_or("Moth help output length overflow")?;
                    if new_len > HELP_MAX_BYTES {
                        return Err("Moth help capability probe exceeded output limit".into());
                    }
                    let stream = if is_stdout { &mut stdout_bytes } else { &mut stderr_bytes };
                    stream.try_reserve(chunk.len())
                        .map_err(|error| format!("Moth help output allocation: {error}"))?;
                    stream.extend_from_slice(&chunk);
                    total_bytes = new_len;
                }
                Ok(ProbeEvent::Eof(_is_stdout)) => eof_count += 1,
                Ok(ProbeEvent::ReadError(error)) => {
                    return Err(format!("Moth help output read: {error}"));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("Moth help output readers stopped without EOF".into());
                }
            }
        }
        let marker = b"--prompt-stdin";
        let advertised = stdout_bytes.windows(marker.len()).any(|window| window == marker)
            || stderr_bytes.windows(marker.len()).any(|window| window == marker);
        if !status.expect("status checked above").success() || !advertised
        {
            return Err("MOTH_AGENT_BIN lacks required agent run --prompt-stdin support".into());
        }
        Ok(())
    })();
    if result.is_err() {
        drop(rx); // Release bounded-channel senders before leaving.
        terminate_and_reap(&mut child);
        // A hostile grandchild may keep a pipe open. Do not join blocked
        // readers on this failure path; the adapter exits immediately.
        return result;
    }
    out_thread.join().map_err(|_| "Moth help stdout reader panicked")?;
    err_thread.join().map_err(|_| "Moth help stderr reader panicked")?;
    result
}

fn read_prompt_snapshot(prompt_file: &Path) -> Result<Vec<u8>, String> {
    let mut file = File::open(prompt_file).map_err(|error| format!("open prompt_file: {error}"))?;
    let opened = file.metadata().map_err(|error| format!("prompt_file metadata: {error}"))?;
    if !opened.is_file() {
        return Err("opened prompt_file is not a regular file".into());
    }
    let mut snapshot = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    let mut remaining = opened.len();
    while remaining > 0 {
        let limit = usize::try_from(remaining.min(chunk.len() as u64))
            .map_err(|_| "prompt_file length is not addressable")?;
        let n = file.read(&mut chunk[..limit])
            .map_err(|error| format!("read prompt_file: {error}"))?;
        if n == 0 {
            return Err("prompt_file ended before its opened length".into());
        }
        snapshot.try_reserve(n)
            .map_err(|error| format!("prompt snapshot allocation: {error}"))?;
        snapshot.extend_from_slice(&chunk[..n]);
        remaining -= n as u64;
    }
    let mut excess = [0u8; 1];
    if file.read(&mut excess).map_err(|error| format!("read prompt_file EOF: {error}"))? != 0 {
        return Err("prompt_file grew while creating snapshot".into());
    }
    if snapshot.is_empty() {
        return Err("prompt_file is empty".into());
    }
    std::str::from_utf8(&snapshot).map_err(|error| format!("prompt_file UTF-8: {error}"))?;
    let final_len = file.metadata()
        .map_err(|error| format!("prompt_file final metadata: {error}"))?.len();
    if opened.len() != final_len || final_len != snapshot.len() as u64 {
        return Err("prompt_file changed length while creating snapshot".into());
    }
    Ok(snapshot)
}

#[derive(Deserialize)]
struct ReceiptLine<'a> {
    seq: u64,
    run_id: &'a str,
    kind: &'a str,
    data: serde::de::IgnoredAny,
}

fn next_jsonl_line(reader: &mut impl BufRead, line: &mut Vec<u8>) -> Result<bool, String> {
    line.clear();
    loop {
        let available = reader.fill_buf().map_err(|error| format!("read Moth runlog: {error}"))?;
        if available.is_empty() {
            return if line.is_empty() { Ok(false) }
                else { Err("Moth runlog ends in an unterminated record".into()) };
        }
        let len = available.iter().position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        line.try_reserve(len)
            .map_err(|error| format!("Moth runlog line allocation: {error}"))?;
        line.extend_from_slice(&available[..len]);
        reader.consume(len);
        if line.last() == Some(&b'\n') {
            return Ok(true);
        }
    }
}

fn validate_success_receipt(receipt: &Path, run_id: &str) -> Result<(), String> {
    let path_meta = fs::symlink_metadata(receipt)
        .map_err(|error| format!("missing Moth runlog after successful exit: {error}"))?;
    if path_meta.file_type().is_symlink() || !path_meta.is_file() {
        return Err("Moth runlog must be a regular non-symlink file".into());
    }
    let file = File::open(receipt).map_err(|error| format!("open Moth runlog: {error}"))?;
    if !file.metadata().map_err(|error| format!("Moth runlog metadata: {error}"))?.is_file() {
        return Err("opened Moth runlog is not a regular file".into());
    }
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    let mut next_seq = 0u64;
    let mut saw_done = false;
    while next_jsonl_line(&mut reader, &mut line)? {
        if saw_done {
            return Err("Moth runlog has records after terminal done".into());
        }
        let record: ReceiptLine<'_> = serde_json::from_slice(&line[..line.len() - 1])
            .map_err(|error| format!("Moth runlog malformed JSON: {error}"))?;
        let _ = &record.data;
        if record.seq != next_seq {
            return Err(format!("Moth runlog sequence gap: expected {next_seq}, got {}", record.seq));
        }
        if record.run_id != run_id {
            return Err("Moth runlog record has a different run_id".into());
        }
        if next_seq == 0 && record.kind != "start" {
            return Err("Moth runlog does not begin with start".into());
        }
        match record.kind {
            "done" => saw_done = true,
            "error" | "cancelled" => {
                return Err("Moth returned zero after an error/cancelled record".into());
            }
            _ => {}
        }
        next_seq = next_seq.checked_add(1).ok_or("Moth runlog sequence overflow")?;
    }
    if !saw_done || next_seq < 2 {
        return Err("Moth returned success without a terminal done record".into());
    }
    Ok(())
}

fn required_path_env(name: &str) -> Result<PathBuf, String> {
    let raw = env::var_os(name).ok_or_else(|| format!("{name} is required"))?;
    if raw.is_empty() {
        return Err(format!("{name} cannot be empty"));
    }
    Ok(PathBuf::from(raw))
}

fn absolute(base: &Path, raw: &OsString, name: &str) -> Result<PathBuf, String> {
    if raw.is_empty() {
        return Err(format!("{name} cannot be empty"));
    }
    Ok(absolute_path(base, PathBuf::from(raw.clone())))
}

fn absolute_path(base: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() { path } else { base.join(path) }
}

fn existing_dir(path: &Path, name: &str) -> Result<PathBuf, String> {
    let resolved = fs::canonicalize(path).map_err(|error| format!("{name}: {error}"))?;
    if !resolved.is_dir() {
        return Err(format!("{name} is not a directory"));
    }
    Ok(resolved)
}

fn existing_file(path: &Path, name: &str) -> Result<PathBuf, String> {
    let resolved = fs::canonicalize(path).map_err(|error| format!("{name}: {error}"))?;
    if !resolved.is_file() {
        return Err(format!("{name} is not a regular file"));
    }
    Ok(resolved)
}

fn check_log_path(path: &Path, logs_dir: &Path, expected_name: &str) -> Result<(), String> {
    if path.file_name() != Some(OsStr::new(expected_name)) {
        return Err(format!("{expected_name}: unexpected log filename"));
    }
    let parent = path.parent().ok_or_else(|| format!("{expected_name}: missing parent"))?;
    if existing_dir(parent, expected_name)? != logs_dir {
        return Err(format!("{expected_name}: path escapes BOP_CARD_DIR/logs"));
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(format!("{expected_name}: existing path must be a regular non-symlink file"))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("{expected_name}: {error}")),
    }
}

fn check_memory_path(path: &Path, card_dir: &Path) -> Result<(), String> {
    if path.file_name() != Some(OsStr::new("memory-out.json")) {
        return Err("memory_out must name memory-out.json".into());
    }
    let parent = path.parent().ok_or("memory_out has no parent")?;
    if existing_dir(parent, "memory_out parent")? != card_dir {
        return Err("memory_out must be directly inside BOP_CARD_DIR".into());
    }
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Ok(_) => return Err("memory_out must be absent because this adapter does not write it".into()),
        Err(error) => return Err(format!("memory_out: {error}")),
    }
    // Moth currently has no BOP memory-output protocol. The adapter must not
    // fabricate a file: BOP removed any stale memory-out.json before spawn.
    Ok(())
}

fn prepare_runlog_dir(logs_dir: &Path) -> Result<PathBuf, String> {
    let path = logs_dir.join("moth");
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err("moth runlog path must be a non-symlink directory".into());
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(&path).map_err(|error| format!("create moth runlog dir: {error}"))?;
        }
        Err(error) => return Err(format!("moth runlog dir: {error}")),
    }
    let resolved = existing_dir(&path, "moth runlog dir")?;
    if resolved != path {
        return Err("moth runlog directory redirects outside BOP_CARD_DIR/logs".into());
    }
    Ok(resolved)
}

fn open_log(path: &Path, name: &str) -> Result<File, String> {
    OpenOptions::new()
        .create(true)
        .write(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("{name}: {error}"))
}

fn valid_bop_run_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn test_mock_mode(value: Option<OsString>, debug_assertions: bool) -> Result<bool, String> {
    match value.as_deref().and_then(OsStr::to_str) {
        None if value.is_none() => Ok(false),
        Some("0") => Ok(false),
        Some("1") if debug_assertions => Ok(true),
        Some("1") => Err("BOP_MOTH_TEST_MOCK is unavailable in release builds".into()),
        _ => Err("BOP_MOTH_TEST_MOCK must be unset, 0, or 1".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::{test_mock_mode, valid_bop_run_id, validate_success_receipt};
    use std::ffi::OsString;
    use std::fs;

    #[test]
    fn rejects_non_bop_run_identity_before_moth_spawn() {
        assert!(valid_bop_run_id("0123456789abcdef0123456789abcdef"));
        assert!(!valid_bop_run_id("agent-8"));
        assert!(!valid_bop_run_id("0123456789ABCDEF0123456789ABCDEF"));
        assert!(!valid_bop_run_id("../0123456789abcdef0123456789abcdef"));
    }

    #[test]
    fn mock_mode_requires_explicit_debug_only_switch() {
        assert!(!test_mock_mode(None, true).unwrap());
        assert!(!test_mock_mode(Some(OsString::from("0")), true).unwrap());
        assert!(test_mock_mode(Some(OsString::from("1")), true).unwrap());
        assert!(test_mock_mode(Some(OsString::from("1")), false).is_err());
        assert!(test_mock_mode(Some(OsString::from("yes")), true).is_err());
    }

    #[test]
    fn a_success_receipt_requires_complete_matching_terminal_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let receipt = dir.path().join("run.jsonl");
        let id = "0123456789abcdef0123456789abcdef";
        let start = format!("{{\"seq\":0,\"run_id\":\"{id}\",\"kind\":\"start\",\"data\":{{}}}}\n");
        let done = format!("{{\"seq\":1,\"run_id\":\"{id}\",\"kind\":\"done\",\"data\":{{}}}}\n");

        fs::write(&receipt, &start).unwrap();
        assert!(validate_success_receipt(&receipt, id).is_err());
        fs::write(&receipt, format!("{start}{}", done.trim_end())).unwrap();
        assert!(validate_success_receipt(&receipt, id).is_err());
        fs::write(&receipt, format!("{start}{done}")).unwrap();
        validate_success_receipt(&receipt, id).unwrap();
        assert!(validate_success_receipt(&receipt, "ffffffffffffffffffffffffffffffff").is_err());
        fs::write(&receipt, format!("{start}{{\"seq\":2,\"run_id\":\"{id}\",\"kind\":\"done\",\"data\":{{}}}}\n")).unwrap();
        assert!(validate_success_receipt(&receipt, id).is_err());
        fs::write(&receipt, format!("{start}{done}{{\"seq\":2,\"run_id\":\"{id}\",\"kind\":\"text_delta\",\"data\":{{}}}}\n")).unwrap();
        assert!(validate_success_receipt(&receipt, id).is_err());
    }
}
