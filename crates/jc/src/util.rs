use chrono::Utc;
use jobcard_core::Meta;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::sync::atomic::AtomicU64;
use walkdir::WalkDir;

pub static RUN_ID_SEQ: AtomicU64 = AtomicU64::new(0);

pub fn host_name() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "unknown-host".to_string())
}

pub fn next_run_id(pid: Option<u32>) -> String {
    let ts = Utc::now()
        .timestamp_nanos_opt()
        .unwrap_or_else(|| Utc::now().timestamp_micros() * 1000);
    format!("{}-{}", ts, pid.unwrap_or(0))
}

pub fn pid_is_alive_sync(pid: i32) -> bool {
    StdCommand::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn append_log_line(path: &Path, line: &str) -> anyhow::Result<()> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{}", line)?;
    Ok(())
}

pub fn copy_dir_all(src: &Path, dst: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in WalkDir::new(src).min_depth(1) {
        let entry = entry?;
        let rel = entry.path().strip_prefix(src)?;
        let target = dst.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

pub fn unique_failed_path(failed_dir: &Path, name: &str) -> PathBuf {
    let direct = failed_dir.join(name);
    if !direct.exists() {
        return direct;
    }
    let stem = name.strip_suffix(".jobcard").unwrap_or(name);
    let ts = Utc::now().timestamp_millis();
    failed_dir.join(format!("{stem}-rejected-{ts}.jobcard"))
}

pub fn workflow_mode_for_template(template: &str) -> &'static str {
    match template {
        "full" => "full-spec",
        "qa-only" => "qa-only",
        "ideation" => "ideation",
        "roadmap" => "roadmap",
        "pr-fix" => "pr-fix",
        "mr-fix" => "mr-fix",
        _ => "default-feature",
    }
}

pub fn current_stage_step_index(meta: &Meta) -> u32 {
    if let Some(idx) = meta
        .stage_chain
        .iter()
        .position(|stage| stage == &meta.stage)
        .map(|i| i + 1)
    {
        idx as u32
    } else {
        meta.step_index.unwrap_or(1).max(1)
    }
}

pub fn find_repo_script(start: &Path, script_rel: &str) -> Option<PathBuf> {
    start.ancestors().find_map(|dir| {
        let candidate = dir.join(script_rel);
        if candidate.exists() {
            Some(candidate)
        } else {
            None
        }
    })
}
