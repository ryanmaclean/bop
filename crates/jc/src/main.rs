use anyhow::Context;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use clap::{Parser, Subcommand, ValueEnum};
use jobcard_core::{write_meta, Meta, StageStatus, VcsEngine as CoreVcsEngine};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::time::Duration;
use tokio::process::Command as TokioCommand;
use walkdir::WalkDir;

#[derive(Parser, Debug)]
#[command(name = "bop")]
struct Cli {
    #[arg(long, default_value = ".cards")]
    cards_dir: String,

    #[command(subcommand)]
    cmd: Command,
}

const DEFAULT_MEMORY_TTL_SECONDS: i64 = 60 * 60 * 24 * 30;
const LEASE_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const LEASE_STALE_FLOOR: Duration = Duration::from_secs(30);
const DISPATCHER_LOCK_REL: &str = ".locks/dispatcher.lock";

#[derive(Subcommand, Debug)]
enum Command {
    Init,
    New {
        template: String,
        id: String,
    },
    Status {
        #[arg(default_value = "")]
        id: String,
    },
    Validate {
        id: String,
        /// Run realtime feed validation on the job's output records.
        #[arg(long)]
        realtime: bool,
    },
    Dispatcher {
        #[arg(long, default_value = "adapters/mock.zsh")]
        adapter: String,

        #[arg(long)]
        max_workers: Option<usize>,

        #[arg(long, default_value_t = 500)]
        poll_ms: u64,

        #[arg(long)]
        max_retries: Option<u32>,

        #[arg(long, default_value_t = 1000)]
        reap_ms: u64,

        #[arg(long)]
        no_reap: bool,

        #[arg(long)]
        once: bool,

        /// Error-rate threshold (0.0–1.0) above which a job with critical alerts
        /// is moved to failed/ instead of done/. Default 1.0 means never fail.
        #[arg(long, default_value_t = 1.0)]
        validation_fail_threshold: f64,

        /// VCS engine used for workspace preparation and publish.
        #[arg(long, value_enum, default_value_t = VcsEngine::GitGt)]
        vcs_engine: VcsEngine,
    },
    MergeGate {
        #[arg(long, default_value_t = 500)]
        poll_ms: u64,

        #[arg(long)]
        once: bool,

        /// VCS engine used for finalize/publish flow.
        #[arg(long, value_enum, default_value_t = VcsEngine::GitGt)]
        vcs_engine: VcsEngine,
    },
    /// Move a card back to pending/ so the dispatcher picks it up again.
    Retry {
        id: String,
    },
    /// Send SIGTERM to the running agent and mark the card as failed.
    Kill {
        id: String,
    },
    /// Approve a card that has decision_required set, unblocking it for dispatch.
    Approve {
        id: String,
    },
    /// Stream stdout and stderr logs for a card.
    Logs {
        id: String,
        /// Keep streaming as new output arrives (like tail -f).
        #[arg(short, long)]
        follow: bool,
    },
    /// Show meta, spec, and a log summary for a card.
    Inspect {
        id: String,
    },
    /// Safely mutate selected meta fields with schema validation.
    Meta {
        #[command(subcommand)]
        action: MetaAction,
    },
    /// Run policy gates.
    Policy {
        #[command(subcommand)]
        action: PolicyAction,
    },
    /// Check local toolchain/environment prerequisites.
    Doctor,
    /// Generate shell completion script.
    GenerateCompletion {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Async planning-poker estimation using playing-card glyphs.
    Poker {
        #[command(subcommand)]
        action: PokerAction,
    },
    /// Manage launchd services for dispatcher and merge-gate.
    Factory {
        #[command(subcommand)]
        action: FactoryAction,
    },
    /// Keep Finder folder icons in sync with card state.
    Icons {
        #[command(subcommand)]
        action: IconsAction,
    },
}

#[derive(Subcommand, Debug)]
enum PokerAction {
    /// Open a new estimation round for a card.
    Open { id: String },
    /// Submit your estimate (interactive picker if glyph omitted).
    Submit {
        id: String,
        /// Playing-card glyph, e.g. 🂻 (Jack of Hearts = effort 13pt).
        /// Omit for interactive picker.
        glyph: Option<String>,
        /// Your name/handle (defaults to $USER).
        #[arg(long)]
        name: Option<String>,
    },
    /// Reveal all estimates, print spread, detect outliers.
    Reveal { id: String },
    /// Show who has submitted (names only, not glyphs).
    Status { id: String },
    /// Commit the agreed glyph to meta.json and close the round.
    Consensus { id: String, glyph: String },
}

#[derive(Subcommand, Debug)]
enum FactoryAction {
    /// Generate and install launchd plists for dispatcher + merge-gate.
    Install,
    /// Start (bootstrap) both launchd services.
    Start,
    /// Stop both launchd services.
    Stop,
    /// Show whether dispatcher + merge-gate services are loaded/running.
    Status,
    /// Unload and remove the launchd plist files.
    Uninstall,
}

#[derive(Subcommand, Debug)]
enum IconsAction {
    /// Set icons on every .jobcard in .cards/ right now (batch).
    Sync,
    /// Watch .cards/ with FSEvents and update icons as cards move (foreground).
    Watch,
    /// Install a launchd WatchPaths agent that runs `bop icons sync` on changes.
    Install,
    /// Unload and remove the icon-watcher launchd agent.
    Uninstall,
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum VcsEngine {
    #[value(name = "git_gt")]
    GitGt,
    #[value(name = "jj")]
    Jj,
}

impl VcsEngine {
    fn as_core(self) -> CoreVcsEngine {
        match self {
            VcsEngine::GitGt => CoreVcsEngine::GitGt,
            VcsEngine::Jj => CoreVcsEngine::Jj,
        }
    }
}

#[derive(Subcommand, Debug)]
enum PolicyAction {
    /// Check policy for staged changes (default) or a specific card directory.
    Check {
        /// Card id to check (searches across states).
        id: Option<String>,
        /// Check staged changes in the current git index.
        #[arg(long)]
        staged: bool,
    },
}

#[derive(Subcommand, Debug)]
enum MetaAction {
    /// Update workflow routing fields in meta.json.
    Set {
        id: String,
        /// Workflow mode label (for stage routing / skill mapping).
        #[arg(long)]
        workflow_mode: Option<String>,
        /// 1-based workflow step index.
        #[arg(long)]
        step_index: Option<u32>,
        /// Clear workflow mode (also clears step index).
        #[arg(long)]
        clear_workflow_mode: bool,
        /// Clear step index.
        #[arg(long)]
        clear_step_index: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct MemoryStore {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    entries: BTreeMap<String, MemoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MemoryEntry {
    value: String,
    updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum MemoryOutput {
    Ops(MemoryOutputOps),
    Flat(BTreeMap<String, MemoryOutputValue>),
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct MemoryOutputOps {
    #[serde(default)]
    set: BTreeMap<String, MemoryOutputValue>,
    #[serde(default)]
    delete: Vec<String>,
    #[serde(default)]
    ttl_seconds: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum MemoryOutputValue {
    String(String),
    Detailed {
        value: String,
        #[serde(default)]
        ttl_seconds: Option<i64>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RunLease {
    run_id: String,
    pid: i32,
    pid_start_time: DateTime<Utc>,
    started_at: DateTime<Utc>,
    heartbeat_at: DateTime<Utc>,
    host: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DispatcherLockOwner {
    pid: i32,
    host: String,
    started_at: DateTime<Utc>,
}

#[derive(Debug)]
struct DispatcherLockGuard {
    path: PathBuf,
}

impl Drop for DispatcherLockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn lock_owner_path(lock_dir: &Path) -> PathBuf {
    lock_dir.join("owner.json")
}

fn lease_path(card_dir: &Path) -> PathBuf {
    card_dir.join("logs").join("lease.json")
}

fn host_name() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "unknown-host".to_string())
}

fn next_run_id(pid: Option<u32>) -> String {
    let ts = Utc::now()
        .timestamp_nanos_opt()
        .unwrap_or_else(|| Utc::now().timestamp_micros() * 1000);
    format!("{}-{}", ts, pid.unwrap_or(0))
}

fn pid_is_alive_sync(pid: i32) -> bool {
    StdCommand::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn acquire_dispatcher_lock(cards_dir: &Path) -> anyhow::Result<DispatcherLockGuard> {
    let lock_dir = cards_dir.join(DISPATCHER_LOCK_REL);
    if let Some(parent) = lock_dir.parent() {
        fs::create_dir_all(parent)?;
    }

    let owner = DispatcherLockOwner {
        pid: std::process::id() as i32,
        host: host_name(),
        started_at: Utc::now(),
    };
    let owner_json = serde_json::to_vec_pretty(&owner)?;

    for _ in 0..2 {
        match fs::create_dir(&lock_dir) {
            Ok(()) => {
                if let Err(err) = fs::write(lock_owner_path(&lock_dir), &owner_json) {
                    let _ = fs::remove_dir_all(&lock_dir);
                    return Err(err.into());
                }
                return Ok(DispatcherLockGuard { path: lock_dir });
            }
            Err(err) if err.kind() == ErrorKind::AlreadyExists => {
                let lock_owner = fs::read(lock_owner_path(&lock_dir))
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<DispatcherLockOwner>(&bytes).ok());

                let stale = lock_owner
                    .as_ref()
                    .map(|o| !pid_is_alive_sync(o.pid))
                    .unwrap_or(true);
                if stale {
                    let _ = fs::remove_dir_all(&lock_dir);
                    continue;
                }

                if let Some(owner) = lock_owner {
                    anyhow::bail!(
                        "dispatcher lock already held by pid {} on {} (started {})",
                        owner.pid,
                        owner.host,
                        owner.started_at
                    );
                }
                anyhow::bail!(
                    "dispatcher lock already exists at {}; remove stale lock if no dispatcher is running",
                    lock_dir.display()
                );
            }
            Err(err) => return Err(err.into()),
        }
    }

    anyhow::bail!(
        "failed to acquire dispatcher lock at {}",
        lock_dir.display()
    )
}

fn write_run_lease(card_dir: &Path, lease: &RunLease) -> anyhow::Result<()> {
    let path = lease_path(card_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(lease)?)?;
    Ok(())
}

fn read_run_lease(card_dir: &Path) -> Option<RunLease> {
    let bytes = fs::read(lease_path(card_dir)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn lease_is_stale(lease: &RunLease, stale_after: ChronoDuration) -> bool {
    Utc::now().signed_duration_since(lease.heartbeat_at) > stale_after
}

fn find_repo_script(start: &Path, script_rel: &str) -> Option<PathBuf> {
    start.ancestors().find_map(|dir| {
        let candidate = dir.join(script_rel);
        if candidate.exists() {
            Some(candidate)
        } else {
            None
        }
    })
}

fn card_state_from_path(card_dir: &Path) -> Option<String> {
    card_dir
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
}

fn infer_card_id_from_path(card_dir: &Path) -> Option<String> {
    let name = card_dir.file_name()?.to_str()?;
    let base = name.strip_suffix(".jobcard").unwrap_or(name);
    Some(base.to_string())
}

fn write_webloc(path: &Path, target_url: &str) -> anyhow::Result<()> {
    let body = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>URL</key>
  <string>{target_url}</string>
</dict>
</plist>
"#
    );
    fs::write(path, body)?;
    Ok(())
}

fn sync_card_action_links(card_dir: &Path) {
    let meta = jobcard_core::read_meta(card_dir).ok();
    let id = meta
        .as_ref()
        .map(|m| m.id.clone())
        .or_else(|| infer_card_id_from_path(card_dir))
        .unwrap_or_default();
    if id.trim().is_empty() {
        return;
    }

    let state = card_state_from_path(card_dir).unwrap_or_else(|| "unknown".to_string());
    let done_like = matches!(state.as_str(), "done" | "merged");
    let logs_action = if done_like { "logs" } else { "tail" };
    let logs_url = format!("bop://card/{id}/{logs_action}");
    let logs_label = if done_like { "Open logs" } else { "Tail logs" };
    let logs_cmd = if done_like {
        format!("bop logs {id}")
    } else {
        format!("bop logs {id} --follow")
    };

    let session = meta
        .as_ref()
        .and_then(|m| m.zellij_session.as_ref())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let mut links_md = String::from("# Card Links\n\n");
    links_md.push_str(&format!("- Logs: [{logs_label}]({logs_url})\n"));
    links_md.push_str(&format!("- Logs command: `{logs_cmd}`\n"));

    let session_webloc = card_dir.join("Session.webloc");
    if state == "running" {
        if let Some(session) = session {
            let session_url = format!("bop://card/{id}/session");
            links_md.push_str(&format!("- Session: [Attach zellij]({session_url})\n"));
            links_md.push_str(&format!("- Session command: `zellij attach {session}`\n"));
            let _ = write_webloc(&session_webloc, &session_url);
        } else {
            let _ = fs::remove_file(&session_webloc);
        }
    } else {
        let _ = fs::remove_file(&session_webloc);
    }

    let _ = fs::write(card_dir.join("links.md"), links_md);
    let _ = write_webloc(&card_dir.join("Logs.webloc"), &logs_url);
}

fn render_card_thumbnail(card_dir: &Path) {
    sync_card_action_links(card_dir);

    if !cfg!(target_os = "macos") {
        return;
    }
    let meta = card_dir.join("meta.json");
    if !meta.exists() {
        return;
    }
    let ql_dir = card_dir.join("QuickLook");
    let _ = fs::create_dir_all(&ql_dir);
    let out = ql_dir.join("Thumbnail.png");
    let Some(script) = find_repo_script(card_dir, "scripts/render_card_thumbnail.swift") else {
        return;
    };

    let _ = StdCommand::new("swift")
        .arg(script)
        .arg(&meta)
        .arg(out)
        .status();

    // Update Finder folder icon: stage colour + glyph, set on every state transition
    if let Some(icon_script) = find_repo_script(card_dir, "scripts/set_card_icon.swift") {
        let _ = StdCommand::new("swift")
            .arg(icon_script)
            .arg(card_dir)
            .status();
    }
}

fn maybe_hfs_compress_card(card_dir: &Path) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let enabled = std::env::var("BOP_HFS_COMPRESS_MERGED")
        .map(|v| matches!(v.as_str(), "1" | "true" | "yes"))
        .unwrap_or(false);
    if !enabled {
        return;
    }

    let Some(name) = card_dir.file_name().and_then(|s| s.to_str()) else {
        return;
    };
    let Some(parent) = card_dir.parent() else {
        return;
    };
    let compressed = parent.join(format!("{}.hfs.tmp", name));
    let backup = parent.join(format!("{}.bak.tmp", name));
    let _ = fs::remove_dir_all(&compressed);
    let _ = fs::remove_dir_all(&backup);

    let status = StdCommand::new("ditto")
        .arg("--clone")
        .arg("--hfsCompression")
        .arg(card_dir)
        .arg(&compressed)
        .status();
    if !matches!(status, Ok(s) if s.success()) {
        let _ = fs::remove_dir_all(&compressed);
        return;
    }

    if fs::rename(card_dir, &backup).is_err() {
        let _ = fs::remove_dir_all(&compressed);
        return;
    }
    if fs::rename(&compressed, card_dir).is_err() {
        let _ = fs::rename(&backup, card_dir);
        let _ = fs::remove_dir_all(&compressed);
        return;
    }
    let _ = fs::remove_dir_all(&backup);
}

fn ensure_cards_layout(root: &Path) -> anyhow::Result<()> {
    for dir in [
        "templates",
        "pending",
        "running",
        "done",
        "merged",
        "failed",
        "memory",
    ] {
        fs::create_dir_all(root.join(dir))?;
    }
    Ok(())
}

fn clone_template(src: &Path, dst: &Path) -> anyhow::Result<()> {
    // Ensure destination parent directory exists
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }

    // Prefer APFS clone semantics on macOS (ditto --clone), then cp -c.
    if cfg!(target_os = "macos") {
        let status = StdCommand::new("ditto")
            .arg("--clone")
            .arg(src)
            .arg(dst)
            .status();
        if matches!(status, Ok(s) if s.success()) {
            return Ok(());
        }

        let status = StdCommand::new("cp")
            .arg("-c") // COW clone on APFS
            .arg("-R") // Recursive
            .arg(src)
            .arg(dst)
            .status();
        if matches!(status, Ok(s) if s.success()) {
            return Ok(());
        }

        // Fallback to regular copy if COW fails
        let status = StdCommand::new("cp").arg("-R").arg(src).arg(dst).status();
        if matches!(status, Ok(s) if s.success()) {
            return Ok(());
        }
    } else {
        // Try Btrfs reflink on Linux
        let status = StdCommand::new("cp")
            .arg("--reflink=auto") // Try COW, fallback to regular copy
            .arg("-r")
            .arg(src)
            .arg(dst)
            .status();
        if matches!(status, Ok(s) if s.success()) {
            return Ok(());
        }

        // Fallback to regular copy if reflink fails
        let status = StdCommand::new("cp").arg("-r").arg(src).arg(dst).status();
        if matches!(status, Ok(s) if s.success()) {
            return Ok(());
        }
    }

    // Final fallback to manual copy
    copy_dir_all(src, dst)
}

fn ensure_mock_provider_command(cards_dir: &Path, adapter: &str) -> anyhow::Result<()> {
    let mut pf = read_providers(cards_dir)?;
    if let Some(p) = pf.providers.get_mut("mock") {
        p.command = adapter.to_string();
    }
    write_providers(cards_dir, &pf)?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Provider {
    command: String,
    #[serde(default)]
    rate_limit_exit: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cooldown_until_epoch_s: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ProvidersFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_provider: Option<String>,
    #[serde(default)]
    providers: std::collections::BTreeMap<String, Provider>,
}

#[derive(Debug, Clone)]
struct WorkspaceInfo {
    name: String,
    path: PathBuf,
    change_ref: Option<String>,
}

// ---------- changes.json types ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileChange {
    path: String,
    status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DiffStats {
    files_changed: usize,
    insertions: usize,
    deletions: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChangesManifest {
    branch: String,
    files_changed: Vec<FileChange>,
    stats: DiffStats,
}

/// Capture git diff summary and write `changes.json` into the card directory.
async fn write_changes_json(card_dir: &Path, workdir: &Path, branch: &str) -> anyhow::Result<()> {
    let name_status = TokioCommand::new("git")
        .args(["diff", "--name-status", "HEAD~1"])
        .current_dir(workdir)
        .output()
        .await
        .context("failed to run git diff --name-status")?;

    let ns_text = String::from_utf8_lossy(&name_status.stdout);
    let mut files: Vec<FileChange> = Vec::new();
    for line in ns_text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, '\t');
        let status_code = parts.next().unwrap_or("").trim();
        let path = parts.next().unwrap_or("").trim();
        if path.is_empty() {
            continue;
        }
        let status = match status_code.chars().next() {
            Some('A') => "added",
            Some('D') => "deleted",
            Some('M') => "modified",
            Some('R') => "renamed",
            Some('C') => "copied",
            _ => "modified",
        };
        files.push(FileChange {
            path: path.to_string(),
            status: status.to_string(),
        });
    }

    let stat_output = TokioCommand::new("git")
        .args(["diff", "--stat", "HEAD~1"])
        .current_dir(workdir)
        .output()
        .await
        .context("failed to run git diff --stat")?;

    let stat_text = String::from_utf8_lossy(&stat_output.stdout);
    let mut insertions: usize = 0;
    let mut deletions: usize = 0;
    if let Some(summary_line) = stat_text.lines().last() {
        for part in summary_line.split(',') {
            let part = part.trim();
            if part.contains("insertion") {
                if let Some(n) = part.split_whitespace().next() {
                    insertions = n.parse().unwrap_or(0);
                }
            } else if part.contains("deletion") {
                if let Some(n) = part.split_whitespace().next() {
                    deletions = n.parse().unwrap_or(0);
                }
            }
        }
    }

    let manifest = ChangesManifest {
        branch: branch.to_string(),
        files_changed: files.clone(),
        stats: DiffStats {
            files_changed: files.len(),
            insertions,
            deletions,
        },
    };

    let json = serde_json::to_string_pretty(&manifest)?;
    fs::write(card_dir.join("changes.json"), json)?;
    Ok(())
}

fn validate_provider(name: &str, p: &Provider) -> anyhow::Result<()> {
    if name.trim().is_empty() {
        anyhow::bail!("provider name cannot be empty");
    }
    if p.command.trim().is_empty() {
        anyhow::bail!("provider '{}': command/adapter cannot be empty", name);
    }
    Ok(())
}

fn providers_path(cards_dir: &Path) -> PathBuf {
    cards_dir.join("providers.json")
}

fn read_providers(cards_dir: &Path) -> anyhow::Result<ProvidersFile> {
    let p = providers_path(cards_dir);
    if !p.exists() {
        return Ok(ProvidersFile::default());
    }
    let bytes = fs::read(p)?;
    let pf: ProvidersFile = serde_json::from_slice(&bytes)?;
    for (name, provider) in &pf.providers {
        validate_provider(name, provider)?;
    }
    Ok(pf)
}

fn write_providers(cards_dir: &Path, pf: &ProvidersFile) -> anyhow::Result<()> {
    for (name, provider) in &pf.providers {
        validate_provider(name, provider)?;
    }
    let bytes = serde_json::to_vec_pretty(pf)?;
    fs::write(providers_path(cards_dir), bytes)?;
    Ok(())
}

fn seed_providers(cards_dir: &Path) -> anyhow::Result<()> {
    let p = providers_path(cards_dir);
    if p.exists() {
        return Ok(());
    }

    let mut pf = ProvidersFile {
        default_provider: Some("mock".to_string()),
        ..Default::default()
    };
    pf.providers.insert(
        "mock".to_string(),
        Provider {
            command: "adapters/mock.zsh".to_string(),
            rate_limit_exit: 75,
            cooldown_until_epoch_s: None,
            model: None,
        },
    );
    pf.providers.insert(
        "mock2".to_string(),
        Provider {
            command: "adapters/mock.zsh".to_string(),
            rate_limit_exit: 75,
            cooldown_until_epoch_s: None,
            model: None,
        },
    );
    write_providers(cards_dir, &pf)?;
    Ok(())
}

async fn reap_orphans(
    running_dir: &Path,
    pending_dir: &Path,
    failed_dir: &Path,
    max_retries: u32,
    stale_lease_after: Duration,
) -> anyhow::Result<()> {
    let stale_after_chrono =
        ChronoDuration::from_std(stale_lease_after).unwrap_or_else(|_| ChronoDuration::seconds(30));
    let entries = match fs::read_dir(running_dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };

    for ent in entries.flatten() {
        let card_dir = ent.path();
        if !card_dir.is_dir() {
            continue;
        }
        if card_dir.extension().and_then(|s| s.to_str()).unwrap_or("") != "jobcard" {
            continue;
        }

        let pid = read_pid(&card_dir).await?;
        let pid_dead = match pid {
            Some(pid) => !is_alive(pid).await?,
            None => false,
        };
        let lease = read_run_lease(&card_dir);
        let lease_stale = lease
            .as_ref()
            .map(|l| lease_is_stale(l, stale_after_chrono))
            .unwrap_or(false);
        if !pid_dead && !lease_stale {
            continue;
        }

        let mut meta = jobcard_core::read_meta(&card_dir).ok();
        let retry_count = meta.as_ref().and_then(|m| m.retry_count).unwrap_or(0);
        let next_retry = retry_count.saturating_add(1);
        let move_to_failed = next_retry > max_retries;
        if let Some(ref mut m) = meta {
            m.retry_count = Some(next_retry);
            if move_to_failed {
                m.failure_reason = Some("max_retries_exceeded".to_string());
            } else {
                m.failure_reason = None;
            }
            for stage in m.stages.values_mut() {
                if stage.status == StageStatus::Running {
                    stage.status = if move_to_failed {
                        StageStatus::Failed
                    } else {
                        StageStatus::Pending
                    };
                    stage.agent = None;
                    stage.provider = None;
                    stage.duration_s = None;
                    stage.started = None;
                    stage.blocked_by = None;
                }
            }
            let _ = write_meta(&card_dir, m);
        }

        let name = match card_dir.file_name().and_then(|s| s.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let target = if move_to_failed {
            failed_dir.join(&name)
        } else {
            pending_dir.join(&name)
        };
        let _ = fs::rename(&card_dir, &target);
        render_card_thumbnail(&target);
    }

    Ok(())
}

async fn read_pid(card_dir: &Path) -> anyhow::Result<Option<i32>> {
    let out = TokioCommand::new("xattr")
        .arg("-p")
        .arg("com.yourorg.agent-pid")
        .arg(card_dir)
        .output()
        .await;
    if let Ok(out) = out {
        if out.status.success() {
            if let Ok(s) = String::from_utf8(out.stdout) {
                if let Ok(pid) = s.trim().parse::<i32>() {
                    return Ok(Some(pid));
                }
            }
        }
    }

    let pid_path = card_dir.join("logs").join("pid");
    if let Ok(s) = fs::read_to_string(pid_path) {
        if let Ok(pid) = s.trim().parse::<i32>() {
            return Ok(Some(pid));
        }
    }

    if let Some(lease) = read_run_lease(card_dir) {
        return Ok(Some(lease.pid));
    }

    Ok(None)
}

async fn is_alive(pid: i32) -> anyhow::Result<bool> {
    let status = TokioCommand::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .await?;
    Ok(status.success())
}

fn seed_default_templates(cards_dir: &Path) -> anyhow::Result<()> {
    let templates_dir = cards_dir.join("templates");
    let implement = templates_dir.join("implement.jobcard");
    if !implement.exists() {
        fs::create_dir_all(implement.join("logs"))?;
        fs::create_dir_all(implement.join("output"))?;

        let meta = Meta {
            id: "template-implement".to_string(),
            created: Utc::now(),
            agent_type: None,
            stage: "implement".to_string(),
            workflow_mode: Some("default-feature".to_string()),
            step_index: Some(1),
            priority: None,
            provider_chain: vec![],
            stages: Default::default(),
            acceptance_criteria: vec![],
            worktree_branch: Some("job/template-implement".to_string()),
            template_namespace: Some("implement".to_string()),
            vcs_engine: None,
            workspace_name: None,
            workspace_path: None,
            change_ref: None,
            policy_scope: vec![],
            decision_required: false,
            decision_path: None,
            policy_result: None,
            timeout_seconds: None,
            retry_count: Some(0),
            failure_reason: None,
            validation_summary: None,
            glyph: None,
            title: None,
            description: None,
            labels: vec![],
            progress: None,
            subtasks: vec![],
            poker_round: None,
            estimates: Default::default(),
            zellij_session: None,
            zellij_pane: None,
            stage_chain: vec![],
            stage_models: Default::default(),
            stage_providers: Default::default(),
            stage_budgets: Default::default(),
        };
        write_meta(&implement, &meta)?;

        if !implement.join("spec.md").exists() {
            fs::write(implement.join("spec.md"), "")?;
        }
        if !implement.join("prompt.md").exists() {
            fs::write(
                implement.join("prompt.md"),
                "{{spec}}\n\nProject memory:\n{{memory}}\n\nAcceptance criteria:\n{{acceptance_criteria}}\n",
            )?;
        }
    }
    Ok(())
}

fn create_card(
    cards_dir: &Path,
    template: &str,
    id: &str,
    spec_override: Option<&str>,
) -> anyhow::Result<PathBuf> {
    ensure_cards_layout(cards_dir)?;

    let template = template.trim();
    if template.is_empty() {
        anyhow::bail!("template cannot be empty");
    }

    let id = id.trim();
    if id.is_empty() {
        anyhow::bail!("id cannot be empty");
    }
    if id.contains('/') || id.contains('\\') {
        anyhow::bail!("id cannot contain path separators");
    }

    let template_dir = cards_dir
        .join("templates")
        .join(format!("{}.jobcard", template));
    if !template_dir.exists() {
        anyhow::bail!("template not found: {}", template);
    }

    let card_dir = cards_dir.join("pending").join(format!("🂠-{}.jobcard", id));
    if card_dir.exists() {
        anyhow::bail!("card already exists: {}", id);
    }

    clone_template(&template_dir, &card_dir)
        .with_context(|| format!("failed to clone template {}", template))?;

    fs::create_dir_all(card_dir.join("logs"))?;
    fs::create_dir_all(card_dir.join("output"))?;

    let mut meta = jobcard_core::read_meta(&card_dir).unwrap_or_else(|_| Meta {
        id: id.to_string(),
        created: Utc::now(),
        agent_type: None,
        stage: "spec".to_string(),
        workflow_mode: None,
        step_index: None,
        priority: None,
        provider_chain: vec![],
        stages: Default::default(),
        acceptance_criteria: vec![],
        worktree_branch: Some(format!("job/{}", id)),
        template_namespace: Some(template.to_string()),
        vcs_engine: None,
        workspace_name: None,
        workspace_path: None,
        change_ref: None,
        policy_scope: vec![],
        decision_required: false,
        decision_path: None,
        policy_result: None,
        timeout_seconds: None,
        retry_count: Some(0),
        failure_reason: None,
        validation_summary: None,
        glyph: None,
        title: None,
        description: None,
        labels: vec![],
        progress: None,
        subtasks: vec![],
        poker_round: None,
        estimates: Default::default(),
        zellij_session: None,
        zellij_pane: None,
        stage_chain: vec![],
        stage_models: Default::default(),
        stage_providers: Default::default(),
        stage_budgets: Default::default(),
    });

    meta.id = id.to_string();
    meta.created = Utc::now();
    meta.worktree_branch = Some(format!("job/{}", id));
    meta.template_namespace = Some(template.to_string());
    meta.retry_count = Some(0);
    meta.failure_reason = None;
    meta.workflow_mode = Some(
        meta.workflow_mode
            .clone()
            .unwrap_or_else(|| workflow_mode_for_template(template).to_string()),
    );
    meta.step_index = Some(current_stage_step_index(&meta));

    write_meta(&card_dir, &meta)?;

    if !card_dir.join("spec.md").exists() {
        fs::write(card_dir.join("spec.md"), "")?;
    }
    if !card_dir.join("prompt.md").exists() {
        fs::write(card_dir.join("prompt.md"), "{{spec}}\n")?;
    }

    if let Some(spec) = spec_override {
        fs::write(card_dir.join("spec.md"), spec)?;
    }

    render_card_thumbnail(&card_dir);

    Ok(card_dir)
}

fn normalize_namespace(namespace: &str) -> String {
    let trimmed = namespace.trim();
    if trimmed.is_empty() {
        "default".to_string()
    } else {
        trimmed.to_string()
    }
}

fn sanitize_namespace(namespace: &str) -> String {
    let normalized = normalize_namespace(namespace);
    let sanitized: String = normalized
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "default".to_string()
    } else {
        sanitized
    }
}

fn memory_store_path(cards_dir: &Path, namespace: &str) -> PathBuf {
    cards_dir
        .join("memory")
        .join(format!("{}.json", sanitize_namespace(namespace)))
}

fn prune_memory_store(store: &mut MemoryStore, now: DateTime<Utc>) -> usize {
    let before = store.entries.len();
    store
        .entries
        .retain(|_, entry| entry.expires_at.map(|exp| exp > now).unwrap_or(true));
    before.saturating_sub(store.entries.len())
}

fn read_memory_store(cards_dir: &Path, namespace: &str) -> anyhow::Result<MemoryStore> {
    let namespace = normalize_namespace(namespace);
    let path = memory_store_path(cards_dir, &namespace);
    if !path.exists() {
        return Ok(MemoryStore::default());
    }

    let bytes = fs::read(&path)?;
    let mut store = if bytes.is_empty() {
        MemoryStore::default()
    } else {
        serde_json::from_slice::<MemoryStore>(&bytes)
            .with_context(|| format!("invalid memory store {}", path.display()))?
    };

    let pruned = prune_memory_store(&mut store, Utc::now());
    if pruned > 0 {
        write_memory_store(cards_dir, &namespace, &store)?;
    }

    Ok(store)
}

fn write_memory_store(
    cards_dir: &Path,
    namespace: &str,
    store: &MemoryStore,
) -> anyhow::Result<()> {
    fs::create_dir_all(cards_dir.join("memory"))?;
    let path = memory_store_path(cards_dir, namespace);
    let bytes = serde_json::to_vec_pretty(store)?;
    fs::write(path, bytes)?;
    Ok(())
}

fn set_memory_entry(
    store: &mut MemoryStore,
    key: &str,
    value: &str,
    ttl_seconds: i64,
    now: DateTime<Utc>,
) {
    let expires_at = now + ChronoDuration::seconds(ttl_seconds);
    store.entries.insert(
        key.to_string(),
        MemoryEntry {
            value: value.to_string(),
            updated_at: now,
            expires_at: Some(expires_at),
        },
    );
}

fn format_memory_for_prompt(store: &MemoryStore) -> String {
    if store.entries.is_empty() {
        return String::new();
    }

    let facts: BTreeMap<String, String> = store
        .entries
        .iter()
        .map(|(k, v)| (k.clone(), v.value.clone()))
        .collect();

    serde_json::to_string_pretty(&facts).unwrap_or_default()
}

fn memory_namespace_from_meta(meta: &Meta) -> String {
    meta.template_namespace
        .as_deref()
        .map(normalize_namespace)
        .filter(|ns| !ns.is_empty())
        .unwrap_or_else(|| normalize_namespace(&meta.stage))
}

fn parse_memory_output(path: &Path) -> anyhow::Result<MemoryOutputOps> {
    let bytes = fs::read(path)?;
    if bytes.iter().all(|b| b.is_ascii_whitespace()) {
        return Ok(MemoryOutputOps::default());
    }

    let parsed: MemoryOutput = serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid memory output {}", path.display()))?;
    Ok(match parsed {
        MemoryOutput::Ops(ops) => ops,
        MemoryOutput::Flat(set) => MemoryOutputOps {
            set,
            delete: vec![],
            ttl_seconds: None,
        },
    })
}

fn merge_memory_output(cards_dir: &Path, namespace: &str, path: &Path) -> anyhow::Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let ops = parse_memory_output(path)?;
    if ops.set.is_empty() && ops.delete.is_empty() {
        return Ok(());
    }

    let mut store = read_memory_store(cards_dir, namespace)?;
    let now = Utc::now();

    for key in ops.delete {
        let key = key.trim();
        if !key.is_empty() {
            store.entries.remove(key);
        }
    }

    for (key, value) in ops.set {
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let (value, item_ttl) = match value {
            MemoryOutputValue::String(v) => (v, None),
            MemoryOutputValue::Detailed { value, ttl_seconds } => (value, ttl_seconds),
        };
        let ttl_seconds = item_ttl
            .or(ops.ttl_seconds)
            .filter(|ttl| *ttl > 0)
            .unwrap_or(DEFAULT_MEMORY_TTL_SECONDS);
        set_memory_entry(&mut store, key, &value, ttl_seconds, now);
    }

    let _ = prune_memory_store(&mut store, now);
    write_memory_store(cards_dir, namespace, &store)?;
    Ok(())
}

fn append_log_line(path: &Path, line: &str) -> anyhow::Result<()> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{}", line)?;
    Ok(())
}

fn workflow_mode_for_template(template: &str) -> &'static str {
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

fn current_stage_step_index(meta: &Meta) -> u32 {
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

fn unique_failed_path(failed_dir: &Path, name: &str) -> PathBuf {
    let direct = failed_dir.join(name);
    if !direct.exists() {
        return direct;
    }
    let stem = name.strip_suffix(".jobcard").unwrap_or(name);
    let ts = Utc::now().timestamp_millis();
    failed_dir.join(format!("{stem}-rejected-{ts}.jobcard"))
}

fn quarantine_invalid_pending_card(
    pending_path: &Path,
    failed_dir: &Path,
    reason: &str,
) -> anyhow::Result<()> {
    let name = pending_path
        .file_name()
        .and_then(|n| n.to_str())
        .with_context(|| format!("invalid card path: {}", pending_path.display()))?;
    let failed_path = unique_failed_path(failed_dir, name);
    fs::rename(pending_path, &failed_path).with_context(|| {
        format!(
            "failed moving invalid pending card {} to failed/",
            pending_path.display()
        )
    })?;
    fs::create_dir_all(failed_path.join("logs"))?;
    fs::create_dir_all(failed_path.join("output"))?;
    let marker = format!("[{}] dispatcher rejected card: {}\n", Utc::now(), reason);
    append_log_line(
        &failed_path.join("logs").join("rejected.log"),
        marker.trim_end(),
    )?;
    let _ = fs::write(failed_path.join("output").join("qa_report.md"), marker);
    render_card_thumbnail(&failed_path);
    Ok(())
}

fn run_policy_script(cwd: &Path, args: &[&str]) -> anyhow::Result<std::process::Output> {
    // Prefer a script relative to the actual git root so the binary works
    // regardless of where it was compiled (avoids stale CARGO_MANIFEST_DIR).
    let git_root_candidate = find_git_root(cwd)
        .map(|r| r.join("scripts").join("policy_check.zsh"))
        .unwrap_or_else(|| cwd.join("scripts").join("policy_check.zsh"));
    let script_candidates = [
        git_root_candidate,
        cwd.join("scripts").join("policy_check.zsh"),
    ];
    let script = script_candidates
        .iter()
        .find(|p| p.exists())
        .cloned()
        .with_context(|| {
            format!(
                "policy script missing (checked: {}, {})",
                script_candidates[0].display(),
                script_candidates[1].display()
            )
        })?;
    let output = StdCommand::new("zsh")
        .arg(script)
        .args(args)
        .current_dir(cwd)
        .output()
        .context("failed to run policy_check.zsh")?;
    Ok(output)
}

fn cmd_policy_check(cards_root: &Path, id: Option<&str>, staged: bool) -> anyhow::Result<()> {
    let repo_root = find_git_root(cards_root).unwrap_or(std::env::current_dir()?);
    let cards_dir_arg = cards_root.to_string_lossy().to_string();

    let output = if staged || id.is_none() {
        run_policy_script(
            &repo_root,
            &["--staged", "--cards-dir", cards_dir_arg.as_str()],
        )?
    } else {
        let card_id = id.unwrap_or_default().trim();
        if card_id.is_empty() {
            anyhow::bail!("card id cannot be empty");
        }
        let card_dir = find_card(cards_root, card_id).context("card not found")?;
        let card_dir_arg = card_dir.to_string_lossy().to_string();
        run_policy_script(
            &repo_root,
            &[
                "--mode",
                "card",
                "--cards-dir",
                cards_dir_arg.as_str(),
                "--id",
                card_id,
                "--card-dir",
                card_dir_arg.as_str(),
            ],
        )?
    };

    if !output.stdout.is_empty() {
        print!("{}", String::from_utf8_lossy(&output.stdout));
    }
    if !output.stderr.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
    }
    if !output.status.success() {
        anyhow::bail!("policy check failed");
    }
    Ok(())
}

fn policy_check_card(cards_root: &Path, card_dir: &Path, card_id: &str) -> anyhow::Result<()> {
    let repo_root = find_git_root(cards_root).unwrap_or(std::env::current_dir()?);
    let cards_dir_arg = cards_root.to_string_lossy().to_string();
    let card_dir_arg = card_dir.to_string_lossy().to_string();
    let output = run_policy_script(
        &repo_root,
        &[
            "--mode",
            "card",
            "--cards-dir",
            cards_dir_arg.as_str(),
            "--id",
            card_id,
            "--card-dir",
            card_dir_arg.as_str(),
        ],
    )?;

    if !output.stdout.is_empty() {
        print!("{}", String::from_utf8_lossy(&output.stdout));
    }
    if !output.stderr.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
    }
    if !output.status.success() {
        anyhow::bail!("policy violation");
    }
    Ok(())
}

// ── factory (launchd lifecycle) ──────────────────────────────────────────────

const FACTORY_LABELS: [(&str, &str); 2] = [
    ("sh.bop.dispatcher", "dispatcher"),
    ("sh.bop.merge-gate", "merge-gate"),
];

fn zellij_plugin_src(repo_root: &Path) -> PathBuf {
    repo_root.join("crates/jc-zellij-plugin/target/wasm32-wasip1/release/jc_zellij_plugin.wasm")
}

fn zellij_plugin_dest() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    Path::new(&home).join(".config/zellij/plugins/bop.wasm")
}

fn launchd_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").expect("HOME not set")).join("Library/LaunchAgents")
}

fn plist_path(label: &str) -> PathBuf {
    launchd_dir().join(format!("{}.plist", label))
}

fn generate_plist(label: &str, subcommand: &str, repo_root: &Path) -> String {
    let bop_bin = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("/usr/local/bin/bop"));
    let cards_dir = repo_root.join(".cards");
    let log_base = format!("/tmp/bop-{}", subcommand);

    // Extra args for dispatcher
    let mut extra_args = String::new();
    if subcommand == "dispatcher" {
        extra_args = r#"    <string>--adapter</string>
    <string>adapters/claude.zsh</string>
    <string>--max-workers</string>
    <string>3</string>
    <string>--poll-ms</string>
    <string>500</string>
    <string>--max-retries</string>
    <string>3</string>
    <string>--reap-ms</string>
    <string>1000</string>"#
            .to_string();
    }

    let args_block = if extra_args.is_empty() {
        format!(
            r#"    <string>{bin}</string>
    <string>{sub}</string>"#,
            bin = bop_bin.display(),
            sub = subcommand,
        )
    } else {
        format!(
            r#"    <string>{bin}</string>
    <string>{sub}</string>
{extra}"#,
            bin = bop_bin.display(),
            sub = subcommand,
            extra = extra_args,
        )
    };

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>

  <key>ProgramArguments</key>
  <array>
{args}
  </array>

  <key>WorkingDirectory</key>
  <string>{wd}</string>

  <key>EnvironmentVariables</key>
  <dict>
    <key>CARDS_DIR</key>
    <string>{cards}</string>
    <key>PATH</key>
    <string>/usr/local/bin:/usr/bin:/bin:{cargo_bin}</string>
    <key>RUST_LOG</key>
    <string>info</string>
  </dict>

  <key>KeepAlive</key>
  <true/>

  <key>RunAtLoad</key>
  <true/>

  <key>StandardOutPath</key>
  <string>{log_base}.log</string>

  <key>StandardErrorPath</key>
  <string>{log_base}.err</string>

  <key>HardResourceLimits</key>
  <dict>
    <key>NumberOfFiles</key>
    <integer>1024</integer>
  </dict>

  <key>SoftResourceLimits</key>
  <dict>
    <key>NumberOfFiles</key>
    <integer>512</integer>
  </dict>
</dict>
</plist>
"#,
        label = label,
        args = args_block,
        wd = repo_root.display(),
        cards = cards_dir.display(),
        cargo_bin = PathBuf::from(std::env::var("HOME").unwrap_or_default())
            .join(".cargo/bin")
            .display(),
        log_base = log_base,
    )
}

fn cmd_factory_install(cards_root: &Path) -> anyhow::Result<()> {
    // Resolve repo root (cards_root is .cards, parent is repo)
    let repo_root = fs::canonicalize(cards_root)
        .unwrap_or_else(|_| cards_root.to_path_buf())
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    // Use cards_root's parent if cards_root ends with ".cards"
    let repo_root = if cards_root
        .file_name()
        .map(|f| f == ".cards")
        .unwrap_or(false)
    {
        cards_root.parent().unwrap_or(Path::new(".")).to_path_buf()
    } else {
        repo_root
    };
    let repo_root = fs::canonicalize(&repo_root).unwrap_or(repo_root);

    let la_dir = launchd_dir();
    fs::create_dir_all(&la_dir)?;

    for (label, subcmd) in &FACTORY_LABELS {
        let plist = generate_plist(label, subcmd, &repo_root);
        let dest = plist_path(label);
        fs::write(&dest, &plist)?;
        println!("✓ wrote {}", dest.display());
    }

    // Load both
    for (label, _) in &FACTORY_LABELS {
        let dest = plist_path(label);
        let out = StdCommand::new("launchctl")
            .args(["load", "-w"])
            .arg(&dest)
            .output()?;
        if out.status.success() {
            println!("✓ loaded {}", label);
        } else {
            let err = String::from_utf8_lossy(&out.stderr);
            eprintln!("⚠ load {}: {}", label, err.trim());
        }
    }

    // Icons watcher: default on, same lifecycle as factory
    if cfg!(target_os = "macos") {
        match cmd_icons_install(cards_root) {
            Ok(_) => {}
            Err(e) => eprintln!("⚠ icon watcher: {}", e),
        }
    }

    // Zellij plugin
    let wasm_src = zellij_plugin_src(&repo_root);
    if wasm_src.exists() {
        let dest = zellij_plugin_dest();
        if let Some(parent) = dest.parent() {
            let _ = fs::create_dir_all(parent);
        }
        match fs::copy(&wasm_src, &dest) {
            Ok(_) => println!("✓ zellij plugin installed: {}", dest.display()),
            Err(e) => eprintln!("  zellij plugin copy failed: {}", e),
        }
    } else {
        println!(
            "  (zellij plugin wasm not built — skipping)\n  build with: cargo build --manifest-path crates/jc-zellij-plugin/Cargo.toml --target wasm32-wasip1 --release"
        );
    }

    println!("\nFactory services installed. Run `bop factory status` to verify.");
    Ok(())
}

fn cmd_factory_start() -> anyhow::Result<()> {
    for (label, _) in &FACTORY_LABELS {
        let out = StdCommand::new("launchctl")
            .args(["start", label])
            .output()?;
        if out.status.success() {
            println!("✓ started {}", label);
        } else {
            let err = String::from_utf8_lossy(&out.stderr);
            eprintln!("⚠ start {}: {}", label, err.trim());
        }
    }
    Ok(())
}

fn cmd_factory_stop() -> anyhow::Result<()> {
    for (label, _) in &FACTORY_LABELS {
        let out = StdCommand::new("launchctl")
            .args(["stop", label])
            .output()?;
        if out.status.success() {
            println!("■ stopped {}", label);
        } else {
            let err = String::from_utf8_lossy(&out.stderr);
            eprintln!("⚠ stop {}: {}", label, err.trim());
        }
    }
    Ok(())
}

fn cmd_factory_status() -> anyhow::Result<()> {
    println!("── factory services ──");
    factory_status_one(ICONS_LABEL, "icons");
    for (label, subcmd) in &FACTORY_LABELS {
        let dest = plist_path(label);
        let installed = dest.exists();

        // Check if loaded via launchctl list
        let out = StdCommand::new("launchctl")
            .args(["list", label])
            .output()?;
        let loaded = out.status.success();

        let pid = if loaded {
            // Parse PID from launchctl list output (first field of matching line)
            let stdout = String::from_utf8_lossy(&out.stdout);
            stdout.lines().find(|l| l.contains("PID")).and_then(|_| {
                // launchctl list <label> outputs key-value pairs
                stdout.lines().find_map(|l| {
                    let l = l.trim();
                    if l.starts_with("\"PID\"") || l.starts_with("PID") {
                        l.split('=')
                            .nth(1)
                            .or_else(|| l.split_whitespace().nth(1))
                            .and_then(|v| v.trim().trim_matches(';').parse::<u32>().ok())
                    } else {
                        None
                    }
                })
            })
        } else {
            None
        };

        let status_str = match (installed, loaded, pid) {
            (true, true, Some(p)) => format!("● running (pid {})", p),
            (true, true, None) => "● loaded (waiting)".to_string(),
            (true, false, _) => "○ installed (not loaded)".to_string(),
            (false, _, _) => "□ not installed".to_string(),
        };
        println!("  {} {}: {}", subcmd, label, status_str);

        // Show log paths
        let log_path = format!("/tmp/bop-{}.log", subcmd);
        let err_path = format!("/tmp/bop-{}.err", subcmd);
        if Path::new(&log_path).exists() {
            println!("    stdout: {}", log_path);
        }
        if Path::new(&err_path).exists() {
            println!("    stderr: {}", err_path);
        }
    }
    Ok(())
}

fn cmd_factory_uninstall() -> anyhow::Result<()> {
    // Icons watcher travels with factory
    let _ = cmd_icons_uninstall();

    // Zellij plugin
    let zj_dest = zellij_plugin_dest();
    if zj_dest.exists() {
        let _ = fs::remove_file(&zj_dest);
        println!("✓ removed zellij plugin: {}", zj_dest.display());
    } else {
        println!("  (zellij plugin not installed)");
    }

    for (label, _) in &FACTORY_LABELS {
        let dest = plist_path(label);

        // Unload first (ignore errors if not loaded)
        let _ = StdCommand::new("launchctl")
            .args(["unload", "-w"])
            .arg(&dest)
            .output();

        if dest.exists() {
            fs::remove_file(&dest)?;
            println!("✓ removed {}", dest.display());
        } else {
            println!("  (not installed: {})", label);
        }
    }
    println!("\nFactory services uninstalled.");
    Ok(())
}

fn factory_status_one(label: &str, name: &str) {
    let dest = plist_path(label);
    let installed = dest.exists();
    let out = StdCommand::new("launchctl")
        .args(["list", label])
        .output()
        .ok();
    let loaded = out.as_ref().map(|o| o.status.success()).unwrap_or(false);
    let status_str = match (installed, loaded) {
        (true, true) => "● active",
        (true, false) => "○ installed (not loaded)",
        (false, _) => "□ not installed",
    };
    println!("  {} {}: {}", name, label, status_str);
}

// ── icons ─────────────────────────────────────────────────────────────────────

const ICONS_LABEL: &str = "sh.bop.iconwatcher";

/// Run set_card_icon.swift on a single path (card bundle or state dir).
fn set_card_icon(path: &Path) {
    if !cfg!(target_os = "macos") {
        return;
    }
    if let Some(script) = find_repo_script(path, "scripts/set_card_icon.swift") {
        let _ = StdCommand::new("swift").arg(script).arg(path).status();
    }
}

/// Stamp icons for all state dirs + cards within a single team root.
fn sync_icons_in_root(team_root: &Path, n_cards: &mut usize, n_dirs: &mut usize) {
    const STATES: &[&str] = &[
        "pending",
        "running",
        "done",
        "merged",
        "failed",
        "templates",
    ];
    for &state in STATES {
        let dir = team_root.join(state);
        if !dir.exists() {
            continue;
        }
        // State directory itself gets an icon.
        set_card_icon(&dir);
        *n_dirs += 1;
        // Each .jobcard inside gets an icon.
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("jobcard") && path.is_dir() {
                    set_card_icon(&path);
                    *n_cards += 1;
                }
            }
        }
    }
}

/// Batch: update icons for every state dir + .jobcard under .cards/.
fn cmd_icons_sync(root: &Path) -> anyhow::Result<()> {
    if !cfg!(target_os = "macos") {
        println!("icons: macOS only");
        return Ok(());
    }
    let mut n_cards = 0usize;
    let mut n_dirs = 0usize;

    // Top-level state dirs
    sync_icons_in_root(root, &mut n_cards, &mut n_dirs);

    // team-* subdirs
    for entry in fs::read_dir(root)? {
        let team = entry?.path();
        if team.is_dir()
            && team
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("team-"))
                .unwrap_or(false)
        {
            sync_icons_in_root(&team, &mut n_cards, &mut n_dirs);
        }
    }
    println!("✓ icons synced: {} state dirs, {} cards", n_dirs, n_cards);
    Ok(())
}

/// FSEvents watcher (foreground): update icon immediately when any .jobcard dir moves.
fn cmd_icons_watch(root: &Path) -> anyhow::Result<()> {
    use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

    if !cfg!(target_os = "macos") {
        println!("icons watch: macOS only");
        return Ok(());
    }

    let cards_dir = root.to_path_buf();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = RecommendedWatcher::new(tx, Config::default())?;
    watcher.watch(&cards_dir, RecursiveMode::Recursive)?;

    println!(
        "bop icons watch — watching {} for card moves",
        cards_dir.display()
    );
    println!("Ctrl+C to stop\n");

    for res in rx {
        let event = match res {
            Ok(e) => e,
            Err(e) => {
                eprintln!("watch error: {}", e);
                continue;
            }
        };

        // Only care about create/rename events on .jobcard directories
        let is_relevant = matches!(
            event.kind,
            EventKind::Create(_) | EventKind::Modify(notify::event::ModifyKind::Name(_))
        );
        if !is_relevant {
            continue;
        }

        for path in &event.paths {
            if path.extension().and_then(|e| e.to_str()) == Some("jobcard") && path.is_dir() {
                set_card_icon(path);
                println!(
                    "  icon updated: {}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                );
            }
        }
    }
    Ok(())
}

/// Install a launchd WatchPaths agent: fires `bop icons sync` whenever .cards/ changes.
fn cmd_icons_install(root: &Path) -> anyhow::Result<()> {
    #[cfg(not(target_os = "macos"))]
    {
        println!("icons install: macOS only");
        return Ok(());
    }

    let bop_bin = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("/usr/local/bin/bop"));
    let cards_dir = root.to_string_lossy().to_string();

    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>

  <key>WatchPaths</key>
  <array>
    <string>{cards}</string>
  </array>

  <key>ProgramArguments</key>
  <array>
    <string>{bop}</string>
    <string>--cards-root</string>
    <string>{cards}</string>
    <string>icons</string>
    <string>sync</string>
  </array>

  <key>StandardOutPath</key>
  <string>/tmp/bop-iconwatcher.log</string>

  <key>StandardErrorPath</key>
  <string>/tmp/bop-iconwatcher.err</string>
</dict>
</plist>
"#,
        label = ICONS_LABEL,
        cards = cards_dir,
        bop = bop_bin.display(),
    );

    let dest = plist_path(ICONS_LABEL);
    fs::write(&dest, &plist)?;

    let _ = StdCommand::new("launchctl")
        .args(["unload", "-w"])
        .arg(&dest)
        .output();
    let status = StdCommand::new("launchctl")
        .args(["load", "-w"])
        .arg(&dest)
        .status()?;

    if status.success() {
        println!("✓ icon watcher installed: {}", dest.display());
        println!("  Fires `bop icons sync` whenever .cards/ changes.");
        println!("  Logs: /tmp/bop-iconwatcher.log");
        println!("\n  To remove: bop icons uninstall");
    } else {
        anyhow::bail!("launchctl load failed");
    }
    Ok(())
}

fn cmd_icons_uninstall() -> anyhow::Result<()> {
    let dest = plist_path(ICONS_LABEL);
    let _ = StdCommand::new("launchctl")
        .args(["unload", "-w"])
        .arg(&dest)
        .output();
    if dest.exists() {
        fs::remove_file(&dest)?;
        println!("✓ removed {}", dest.display());
    } else {
        println!("  (not installed)");
    }
    Ok(())
}

fn command_available(name: &str) -> bool {
    StdCommand::new(name)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn cmd_doctor(cards_root: &Path) -> anyhow::Result<()> {
    println!("bop doctor");

    // ── core tools ──────────────────────────────────────────────────────────
    println!("\n── tools ──");
    let checks = [
        ("git", command_available("git")),
        ("gt", command_available("gt")),
        ("jj", command_available("jj")),
        ("gh", command_available("gh")),
        ("zsh", command_available("zsh")),
        ("zellij", command_available("zellij")),
    ];

    let mut failed = 0;
    for (name, ok) in checks {
        if ok {
            println!("ok\t{}", name);
        } else {
            println!("missing\t{}", name);
            failed += 1;
        }
    }

    // ── zellij plugin ──────────────────────────────────────────────────────
    let home = std::env::var("HOME").unwrap_or_default();
    let plugin_path = Path::new(&home).join(".config/zellij/plugins/bop.wasm");
    if plugin_path.exists() {
        println!("ok\tzellij plugin ({})", plugin_path.display());
    } else {
        println!("missing\tzellij plugin (run `bop factory install`)");
    }

    // ── adapters ────────────────────────────────────────────────────────────
    println!("\n── adapters ──");
    let adapters_dir = cards_root.parent().unwrap_or(cards_root).join("adapters");

    // Map adapter name → CLI binary it requires
    let adapter_cli_map: &[(&str, &str)] = &[
        ("claude", "claude"),
        ("codex", "codex"),
        ("ollama-local", "ollama"),
        ("goose", "goose"),
        ("aider", "aider"),
        ("opencode", "opencode"),
        ("mock", "true"), // mock always works
    ];

    if adapters_dir.is_dir() {
        for (adapter, cli) in adapter_cli_map {
            let script = adapters_dir.join(format!("{}.zsh", adapter));
            if !script.exists() {
                continue; // adapter not installed, skip
            }
            let cli_ok = command_available(cli);
            if cli_ok {
                println!("ok\t{}\t({})", adapter, cli);
            } else {
                println!("missing\t{}\t({} not found)", adapter, cli);
                // Adapter missing is a warning, not a hard failure —
                // the system works with any subset of adapters
            }
        }
    } else {
        println!("warn\tadapters/ directory not found");
    }

    // ── cards layout ────────────────────────────────────────────────────────
    println!("\n── cards ──");
    let policy = cards_root.join("policy.toml");
    if policy.exists() {
        println!("ok\t{}", policy.display());
    } else {
        println!("missing\t{}", policy.display());
        failed += 1;
    }

    let system_ctx = cards_root.join("system_context.md");
    if system_ctx.exists() {
        println!("ok\tsystem_context.md");
    } else {
        println!("missing\tsystem_context.md");
    }

    let stages_dir = cards_root.join("stages");
    if stages_dir.is_dir() {
        let n_stages = fs::read_dir(&stages_dir)
            .map(|rd| {
                rd.filter(|e| {
                    e.as_ref()
                        .map(|e| e.path().extension().map(|x| x == "md").unwrap_or(false))
                        .unwrap_or(false)
                })
                .count()
            })
            .unwrap_or(0);
        println!("ok\tstages/ ({} files)", n_stages);
    } else {
        println!("missing\tstages/");
    }

    let templates_dir = cards_root.join("templates");
    if templates_dir.is_dir() {
        let n_templates = fs::read_dir(&templates_dir)
            .map(|rd| {
                rd.filter(|e| {
                    e.as_ref()
                        .map(|e| {
                            e.path()
                                .extension()
                                .map(|x| x == "jobcard")
                                .unwrap_or(false)
                        })
                        .unwrap_or(false)
                })
                .count()
            })
            .unwrap_or(0);
        println!("ok\ttemplates/ ({} templates)", n_templates);
    } else {
        println!("missing\ttemplates/");
    }

    if failed > 0 {
        anyhow::bail!("doctor found {} issue(s)", failed);
    }
    Ok(())
}

fn print_status_summary(root: &Path) -> anyhow::Result<()> {
    for dir in ["pending", "running", "done", "merged", "failed"] {
        let p = root.join(dir);
        if p.exists() {
            let count = fs::read_dir(&p).map(|rd| rd.count()).unwrap_or(0);
            println!("{}\t{}", dir, count);
        }
    }
    Ok(())
}

fn resolve_config_path() -> PathBuf {
    if let Ok(p) = std::env::var("JOBCARD_CONFIG") {
        return PathBuf::from(p);
    }
    jobcard_core::config::project_config_path()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let root = PathBuf::from(&cli.cards_dir);

    // Load merged global+project config (missing files silently skipped)
    let cfg = jobcard_core::load_config().unwrap_or_default();

    match cli.cmd {
        Command::Init => {
            ensure_cards_layout(&root)?;
            seed_default_templates(&root)?;
            seed_providers(&root)?;
            // Create config with sensible defaults if it doesn't exist
            let config_path = resolve_config_path();
            if !config_path.exists() {
                let defaults = jobcard_core::Config {
                    default_provider_chain: Some(vec!["mock".to_string()]),
                    max_concurrent: Some(1),
                    cooldown_seconds: Some(300),
                    log_retention_days: Some(30),
                    default_template: Some("implement".to_string()),
                };
                jobcard_core::config::write_config_file(&config_path, &defaults).with_context(
                    || {
                        format!(
                            "failed to create default config at {}",
                            config_path.display()
                        )
                    },
                )?;
            }
            Ok(())
        }
        Command::New { template, id } => {
            create_card(&root, &template, &id, None)?;
            Ok(())
        }
        Command::Status { id } => {
            if id.trim().is_empty() {
                return print_status_summary(&root);
            }

            let card = find_card(&root, &id).context("card not found")?;
            let state = card
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .unwrap_or("unknown");
            let meta = jobcard_core::read_meta(&card)?;
            let badge = meta
                .validation_summary
                .as_ref()
                .map(|s| s.badge())
                .unwrap_or("");
            if badge.is_empty() {
                println!("[{}] {}", state, meta.id);
            } else {
                println!("[{}] {} {}", state, meta.id, badge);
            }
            println!("{}", serde_json::to_string_pretty(&meta)?);
            Ok(())
        }
        Command::Validate { id, realtime } => {
            let card = find_card(&root, &id).context("card not found")?;
            let _ = jobcard_core::read_meta(&card)?;
            if realtime {
                let summary = validate_realtime_output(&card)?;
                println!(
                    "validation: {} ({}/{} valid, {} alerts, {} critical)",
                    summary.badge(),
                    summary.valid,
                    summary.total,
                    summary.alert_count,
                    summary.critical_alerts
                );
                let log = card.join("logs").join("validation.log");
                if log.exists() {
                    println!("{}", fs::read_to_string(log)?);
                }
            }
            Ok(())
        }
        Command::Dispatcher {
            adapter,
            max_workers,
            poll_ms,
            max_retries,
            reap_ms,
            no_reap,
            once,
            validation_fail_threshold,
            vcs_engine,
        } => {
            let effective_max_workers = max_workers.or(cfg.max_concurrent).unwrap_or(1);
            let effective_max_retries = max_retries.unwrap_or(3);
            run_dispatcher(
                &root,
                vcs_engine,
                &adapter,
                effective_max_workers,
                poll_ms,
                effective_max_retries,
                reap_ms,
                no_reap,
                once,
                validation_fail_threshold,
            )
            .await
        }
        Command::MergeGate {
            poll_ms,
            once,
            vcs_engine,
        } => run_merge_gate(&root, poll_ms, once, vcs_engine).await,
        Command::Retry { id } => cmd_retry(&root, &id),
        Command::Kill { id } => cmd_kill(&root, &id).await,
        Command::Approve { id } => cmd_approve(&root, &id),
        Command::Logs { id, follow } => cmd_logs(&root, &id, follow).await,
        Command::Inspect { id } => cmd_inspect(&root, &id),
        Command::Meta { action } => match action {
            MetaAction::Set {
                id,
                workflow_mode,
                step_index,
                clear_workflow_mode,
                clear_step_index,
            } => cmd_meta_set(
                &root,
                &id,
                workflow_mode.as_deref(),
                step_index,
                clear_workflow_mode,
                clear_step_index,
            ),
        },
        Command::Policy { action } => match action {
            PolicyAction::Check { id, staged } => cmd_policy_check(&root, id.as_deref(), staged),
        },
        Command::Doctor => cmd_doctor(&root),
        Command::GenerateCompletion { shell } => {
            use clap::CommandFactory;
            clap_complete::generate(shell, &mut Cli::command(), "bop", &mut std::io::stdout());
            Ok(())
        }
        Command::Poker { action } => match action {
            PokerAction::Open { id } => cmd_poker_open(&root, &id),
            PokerAction::Submit { id, glyph, name } => {
                cmd_poker_submit(&root, &id, glyph.as_deref(), name.as_deref())
            }
            PokerAction::Reveal { id } => cmd_poker_reveal(&root, &id),
            PokerAction::Status { id } => cmd_poker_status(&root, &id),
            PokerAction::Consensus { id, glyph } => cmd_poker_consensus(&root, &id, &glyph),
        },
        Command::Factory { action } => match action {
            FactoryAction::Install => cmd_factory_install(&root),
            FactoryAction::Start => cmd_factory_start(),
            FactoryAction::Stop => cmd_factory_stop(),
            FactoryAction::Status => cmd_factory_status(),
            FactoryAction::Uninstall => cmd_factory_uninstall(),
        },
        Command::Icons { action } => match action {
            IconsAction::Sync => cmd_icons_sync(&root),
            IconsAction::Watch => cmd_icons_watch(&root),
            IconsAction::Install => cmd_icons_install(&root),
            IconsAction::Uninstall => cmd_icons_uninstall(),
        },
    }
}

// ── poker ─────────────────────────────────────────────────────────────────────

fn glyph_rank(g: &str) -> (&'static str, u32) {
    let cp = g.chars().next().map(|c| c as u32).unwrap_or(0);
    match cp & 0xF {
        1 => ("Ace", 1),
        2 => ("2", 2),
        3 => ("3", 3),
        4 => ("4", 4),
        5 => ("5", 5),
        6 => ("6", 6),
        7 => ("7", 7),
        8 => ("8", 8),
        9 => ("9", 9),
        10 => ("10", 10),
        11 => ("Jack", 13),
        12 => ("Knight", 20),
        13 => ("Queen", 21),
        14 => ("King", 40),
        _ => ("Joker", 0),
    }
}

fn glyph_suit(g: &str) -> &'static str {
    let cp = g.chars().next().map(|c| c as u32).unwrap_or(0);
    match (cp >> 4) & 0xF {
        0xA => "♠ complexity",
        0xB => "♥ effort",
        0xC => "♦ risk",
        0xD => "♣ value",
        _ => "? unknown",
    }
}

fn is_joker(g: &str) -> bool {
    let cp = g.chars().next().map(|c| c as u32).unwrap_or(0);
    matches!(cp, 0x1F0BF | 0x1F0CF | 0x1F0DF | 0x1F093)
}

fn cmd_poker_open(root: &Path, id: &str) -> anyhow::Result<()> {
    let card = find_card(root, id).with_context(|| format!("card not found: {}", id))?;
    let mut meta = jobcard_core::read_meta(&card)?;
    if meta.poker_round.as_deref() == Some("open") {
        println!("Round already open for {}", id);
        return Ok(());
    }
    meta.poker_round = Some("open".into());
    meta.estimates.clear();
    write_meta(&card, &meta)?;
    println!("🂠  Poker round opened for {id}. Submit with: bop poker submit {id}");
    Ok(())
}

fn cmd_poker_submit(
    root: &Path,
    id: &str,
    glyph: Option<&str>,
    name: Option<&str>,
) -> anyhow::Result<()> {
    let card = find_card(root, id).with_context(|| format!("card not found: {}", id))?;
    let mut meta = jobcard_core::read_meta(&card)?;
    if meta.poker_round.as_deref() != Some("open") {
        anyhow::bail!("no open round for {id}. Run: bop poker open {id}");
    }
    let participant = name
        .map(str::to_owned)
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| "anonymous".into());

    let chosen = if let Some(g) = glyph {
        g.to_owned()
    } else {
        // Simple fallback: prompt for glyph when no TTY picker available
        eprint!("Enter glyph (e.g. 🂻): ");
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        line.trim().to_owned()
    };

    if chosen.is_empty() {
        anyhow::bail!("no glyph provided");
    }
    meta.estimates.insert(participant.clone(), chosen);
    write_meta(&card, &meta)?;
    println!("🂠  {participant} submitted (face-down until reveal)");
    Ok(())
}

fn cmd_poker_reveal(root: &Path, id: &str) -> anyhow::Result<()> {
    let card = find_card(root, id).with_context(|| format!("card not found: {}", id))?;
    let mut meta = jobcard_core::read_meta(&card)?;
    if meta.poker_round.as_deref() != Some("open") {
        anyhow::bail!("no open round for {id}");
    }
    meta.poker_round = Some("revealed".into());
    write_meta(&card, &meta)?;

    println!("\n  Estimates for {id}:\n");
    let mut joker_players: Vec<String> = vec![];
    let mut points: Vec<u32> = vec![];

    for (participant, glyph) in &meta.estimates {
        if is_joker(glyph) {
            joker_players.push(participant.clone());
            println!("  {participant:<12} {glyph}  Joker — needs breakdown");
        } else {
            let (rank_label, pts) = glyph_rank(glyph);
            let suit = glyph_suit(glyph);
            println!("  {participant:<12} {glyph}  {rank_label} of {suit} — {pts}pt");
            points.push(pts);
        }
    }

    if !joker_players.is_empty() {
        println!(
            "\n  ⊘ {} played 🃏 — break down the card first",
            joker_players.join(", ")
        );
        return Ok(());
    }

    if points.len() > 1 {
        let mut sorted = points.clone();
        sorted.sort_unstable();
        let median = sorted[sorted.len() / 2];
        let spread = sorted.last().unwrap_or(&0) - sorted.first().unwrap_or(&0);
        println!("\n  Spread: {spread}pt  Median: {median}pt");
        for (participant, glyph) in &meta.estimates {
            let (rank_label, pts) = glyph_rank(glyph);
            if median > 0 && (pts < median / 2 || pts > median * 2) {
                println!("  ⚡ outlier: {participant} ({glyph} {rank_label}  {pts}pt vs median {median}pt)");
            }
        }
    }
    println!("\n  Run: bop poker consensus {id} <glyph>");
    Ok(())
}

fn cmd_poker_status(root: &Path, id: &str) -> anyhow::Result<()> {
    let card = find_card(root, id).with_context(|| format!("card not found: {}", id))?;
    let meta = jobcard_core::read_meta(&card)?;
    match meta.poker_round.as_deref() {
        Some("open") => {
            println!("Round: open  ({} submitted)", meta.estimates.len());
            for name in meta.estimates.keys() {
                println!("  🂠 {name}");
            }
        }
        Some("revealed") => {
            println!("Round: revealed");
            for (name, glyph) in &meta.estimates {
                println!("  {glyph} {name}");
            }
        }
        _ => println!("No active round for {id}"),
    }
    Ok(())
}

fn cmd_poker_consensus(root: &Path, id: &str, glyph: &str) -> anyhow::Result<()> {
    let card = find_card(root, id).with_context(|| format!("card not found: {}", id))?;
    let mut meta = jobcard_core::read_meta(&card)?;
    if meta.poker_round.is_none() {
        anyhow::bail!("no active round for {id}");
    }
    if is_joker(glyph) {
        println!("⊘ {glyph} is a Joker — cannot commit. Break down the card first.");
        return Ok(());
    }
    let (rank_label, pts) = glyph_rank(glyph);
    let suit = glyph_suit(glyph);
    meta.glyph = Some(glyph.to_owned());
    meta.poker_round = None;
    meta.estimates.clear();
    write_meta(&card, &meta)?;

    // Rename dir: {old-glyph}-{id}.jobcard → {glyph}-{id}.jobcard
    let new_name = format!("{}-{}.jobcard", glyph, id);
    let new_card = card.parent().unwrap_or(root).join(&new_name);
    if new_card != card {
        fs::rename(&card, &new_card)?;
        println!("  renamed → {}", new_name);
    }

    println!("∴ Consensus: {glyph} — {rank_label} of {suit} — {pts}pt");
    println!("  Committed to {id}/meta.json");
    Ok(())
}

fn find_git_root(start: &Path) -> Option<std::path::PathBuf> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(start)
        .output()
        .ok()?;
    if out.status.success() {
        Some(std::path::PathBuf::from(
            String::from_utf8_lossy(&out.stdout).trim(),
        ))
    } else {
        None
    }
}

fn sanitize_workspace_component(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "job".to_string()
    } else {
        trimmed.to_string()
    }
}

fn next_workspace_name(card_id: &str) -> String {
    let base = sanitize_workspace_component(card_id);
    let ts = Utc::now().timestamp_millis();
    format!("ws-{}-{}", base, ts)
}

fn git_branch_exists(repo_root: &Path, branch: &str) -> bool {
    StdCommand::new("git")
        .args(["show-ref", "--verify", &format!("refs/heads/{}", branch)])
        .current_dir(repo_root)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn git_head_ref(repo_root: &Path) -> Option<String> {
    let out = StdCommand::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_root)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn jj_head_ref(workspace_path: &Path) -> Option<String> {
    let out = StdCommand::new("jj")
        .args(["log", "-r", "@", "-T", "change_id.short()"])
        .current_dir(workspace_path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn prepare_workspace(
    vcs_engine: VcsEngine,
    cards_dir: &Path,
    card_dir: &Path,
    card_id: &str,
    meta: &mut Option<Meta>,
) -> anyhow::Result<Option<WorkspaceInfo>> {
    let ws_path = card_dir.join("workspace");
    if ws_path.exists() {
        let existing_name = meta
            .as_ref()
            .and_then(|m| m.workspace_name.clone())
            .unwrap_or_else(|| "workspace".to_string());
        let change_ref = match vcs_engine {
            VcsEngine::GitGt => find_git_root(cards_dir).and_then(|r| git_head_ref(&r)),
            VcsEngine::Jj => jj_head_ref(&ws_path),
        };
        return Ok(Some(WorkspaceInfo {
            name: existing_name,
            path: ws_path,
            change_ref,
        }));
    }

    match vcs_engine {
        VcsEngine::GitGt => {
            let Some(git_root) = find_git_root(cards_dir) else {
                return Ok(None);
            };
            let branch = meta
                .as_ref()
                .and_then(|m| m.worktree_branch.clone())
                .unwrap_or_else(|| format!("job/{}", card_id));
            let ws_name = branch.replace('/', "-");
            let stable_ws = git_root.join(".worktrees").join(&ws_name);
            let legacy_ws = card_dir.join("workspace");
            let ws_path = if stable_ws.exists() {
                stable_ws
            } else if legacy_ws.exists() {
                legacy_ws
            } else {
                stable_ws
            };
            let status = if git_branch_exists(&git_root, &branch) {
                StdCommand::new("git")
                    .args([
                        "worktree",
                        "add",
                        ws_path.to_string_lossy().as_ref(),
                        branch.as_str(),
                    ])
                    .current_dir(&git_root)
                    .status()
            } else {
                StdCommand::new("git")
                    .args([
                        "worktree",
                        "add",
                        "-b",
                        branch.as_str(),
                        ws_path.to_string_lossy().as_ref(),
                    ])
                    .current_dir(&git_root)
                    .status()
            };

            if !matches!(status, Ok(s) if s.success()) {
                anyhow::bail!("git worktree add failed for card {}", card_id);
            }
            let change_ref = git_head_ref(&ws_path).or_else(|| git_head_ref(&git_root));
            Ok(Some(WorkspaceInfo {
                name: branch,
                path: ws_path,
                change_ref,
            }))
        }
        VcsEngine::Jj => {
            let repo_root = find_git_root(cards_dir).unwrap_or_else(|| cards_dir.to_path_buf());
            jobcard_core::worktree::ensure_jj_repo(&repo_root)?;
            let ws_name = next_workspace_name(card_id);
            jobcard_core::worktree::create_workspace_with_name(&repo_root, &ws_path, &ws_name)?;
            let change_ref = jj_head_ref(&ws_path);
            Ok(Some(WorkspaceInfo {
                name: ws_name,
                path: ws_path,
                change_ref,
            }))
        }
    }
}

fn persist_workspace_meta(
    meta: &mut Option<Meta>,
    card_dir: &Path,
    vcs_engine: VcsEngine,
    ws: Option<&WorkspaceInfo>,
) {
    if let Some(m) = meta.as_mut() {
        m.vcs_engine = Some(vcs_engine.as_core());
        if let Some(info) = ws {
            m.workspace_name = Some(info.name.clone());
            m.workspace_path = Some(info.path.to_string_lossy().to_string());
            m.change_ref = info.change_ref.clone();
        } else {
            m.workspace_name = None;
            m.workspace_path = None;
            m.change_ref = None;
        }
        let _ = write_meta(card_dir, m);
    }
}

fn is_zellij_interactive() -> bool {
    if std::env::var("ZELLIJ").is_err() {
        return false;
    }
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}

fn count_running_cards(cards_dir: &Path) -> usize {
    let running = cards_dir.join("running");
    std::fs::read_dir(&running)
        .map(|d| {
            d.filter_map(Result::ok)
                .filter(|e| {
                    e.path()
                        .extension()
                        .map(|x| x == "jobcard")
                        .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0)
}

fn zellij_open_card_pane(card_id: &str, card_dir: &Path) {
    let log = card_dir.join("logs").join("stdout.log");
    let Some(log_str) = log.to_str() else { return };
    let _ = std::process::Command::new("zellij")
        .args([
            "action", "new-pane", "--name", card_id, "--", "tail", "-f", log_str,
        ])
        .output();
}

#[allow(clippy::too_many_arguments)]
async fn run_dispatcher(
    cards_dir: &Path,
    vcs_engine: VcsEngine,
    adapter: &str,
    max_workers: usize,
    poll_ms: u64,
    max_retries: u32,
    reap_ms: u64,
    no_reap: bool,
    once: bool,
    validation_fail_threshold: f64,
) -> anyhow::Result<()> {
    ensure_cards_layout(cards_dir)?;
    seed_providers(cards_dir)?;
    ensure_mock_provider_command(cards_dir, adapter)?;
    let _dispatcher_lock = acquire_dispatcher_lock(cards_dir)?;

    let pending_dir = cards_dir.join("pending");
    let running_dir = cards_dir.join("running");
    let done_dir = cards_dir.join("done");
    let failed_dir = cards_dir.join("failed");
    let stale_lease_after = std::cmp::max(
        LEASE_STALE_FLOOR,
        Duration::from_millis(reap_ms.saturating_mul(3)),
    );

    let mut last_reap = std::time::Instant::now()
        .checked_sub(Duration::from_millis(reap_ms))
        .unwrap_or_else(std::time::Instant::now);

    loop {
        if !no_reap && last_reap.elapsed() >= Duration::from_millis(reap_ms) {
            reap_orphans(
                &running_dir,
                &pending_dir,
                &failed_dir,
                max_retries,
                stale_lease_after,
            )
            .await?;
            last_reap = std::time::Instant::now();
        }

        let running_count = fs::read_dir(&running_dir).map(|rd| rd.count()).unwrap_or(0);
        let mut available_slots = max_workers.saturating_sub(running_count);

        if available_slots > 0 {
            if let Ok(entries) = fs::read_dir(&pending_dir) {
                for ent in entries.flatten() {
                    if available_slots == 0 {
                        break;
                    }

                    let pending_path = ent.path();
                    if !pending_path.is_dir() {
                        continue;
                    }
                    if pending_path
                        .extension()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        != "jobcard"
                    {
                        continue;
                    }

                    let name = match pending_path.file_name().and_then(|s| s.to_str()) {
                        Some(n) => n.to_string(),
                        None => continue,
                    };

                    let mut meta = match jobcard_core::read_meta(&pending_path) {
                        Ok(m) => Some(m),
                        Err(err) => {
                            let reason = format!("invalid_meta: {err}");
                            eprintln!(
                                "[dispatcher] rejecting invalid pending card {}: {}",
                                name, err
                            );
                            let _ = quarantine_invalid_pending_card(
                                &pending_path,
                                &failed_dir,
                                &reason,
                            );
                            continue;
                        }
                    };

                    let running_path = running_dir.join(&name);
                    // Count before rename so the current card is not included in the tally
                    let active = count_running_cards(cards_dir);
                    if fs::rename(&pending_path, &running_path).is_err() {
                        continue;
                    }
                    render_card_thumbnail(&running_path);

                    let card_id = meta
                        .as_ref()
                        .map(|m| m.id.clone())
                        .unwrap_or_else(|| name.trim_end_matches(".jobcard").to_string());
                    let ws_info = match prepare_workspace(
                        vcs_engine,
                        cards_dir,
                        &running_path,
                        &card_id,
                        &mut meta,
                    ) {
                        Ok(info) => info,
                        Err(err) => {
                            eprintln!("[dispatcher] workspace prepare failed: {err}");
                            if let Some(ref mut m) = meta {
                                m.failure_reason = Some(format!("workspace_prepare_failed: {err}"));
                                let _ = write_meta(&running_path, m);
                            }
                            let failed_path = failed_dir.join(&name);
                            let _ = fs::rename(&running_path, &failed_path);
                            render_card_thumbnail(&failed_path);
                            continue;
                        }
                    };
                    persist_workspace_meta(&mut meta, &running_path, vcs_engine, ws_info.as_ref());

                    // Adaptive zellij pane management
                    if is_zellij_interactive() {
                        match active {
                            0..=5 => zellij_open_card_pane(&name, &running_path),
                            6..=20 => { /* team pane already open per layout */ }
                            _ => { /* tier 3: status bar only */ }
                        }
                    }

                    available_slots = available_slots.saturating_sub(1);

                    let stage = meta
                        .as_ref()
                        .map(|m| m.stage.clone())
                        .unwrap_or_else(|| "implement".to_string());

                    let (provider_name, provider_cmd, rate_limit_exit) =
                        match select_provider(cards_dir, meta.as_mut(), &stage)? {
                            Some(v) => v,
                            None => {
                                let pending_path = pending_dir.join(&name);
                                let _ = fs::rename(&running_path, &pending_path);
                                render_card_thumbnail(&pending_path);
                                continue;
                            }
                        };

                    if let Some(ref mut meta) = meta {
                        let _ = write_meta(&running_path, meta);
                    }

                    let (exit_code, mut meta) =
                        run_card(cards_dir, &running_path, &provider_cmd, &provider_name)
                            .await
                            .unwrap_or((1, None));

                    let is_rate_limited = exit_code == rate_limit_exit;

                    // Run realtime validation on job output when the job succeeded.
                    let mut validation_triggered_fail = false;
                    if exit_code == 0 {
                        if let Ok(summary) = validate_realtime_output(&running_path) {
                            let error_rate = if summary.total == 0 {
                                0.0
                            } else {
                                summary.invalid as f64 / summary.total as f64
                            };
                            if summary.critical_alerts > 0 && error_rate > validation_fail_threshold
                            {
                                validation_triggered_fail = true;
                                if let Some(ref mut meta) = meta {
                                    meta.failure_reason =
                                        Some("validation_threshold_exceeded".to_string());
                                }
                            }
                            if let Some(ref mut meta) = meta {
                                meta.validation_summary = Some(summary);
                            }
                        }
                    }

                    if let Some(ref mut meta) = meta {
                        if is_rate_limited {
                            let next = meta.retry_count.unwrap_or(0).saturating_add(1);
                            meta.retry_count = Some(next);

                            rotate_provider_chain(meta);
                            let _ = set_provider_cooldown(cards_dir, &provider_name, 300);
                        }

                        let _ = write_meta(&running_path, meta);
                    }
                    let target = if validation_triggered_fail {
                        failed_dir.join(&name)
                    } else if exit_code == 0 {
                        done_dir.join(&name)
                    } else if is_rate_limited {
                        pending_dir.join(&name)
                    } else {
                        failed_dir.join(&name)
                    };

                    let _ = fs::rename(&running_path, &target);
                    render_card_thumbnail(&target);
                    if exit_code == 0 && !validation_triggered_fail {
                        maybe_advance_stage(cards_dir, &target);
                        spawn_child_cards(cards_dir, &target);
                    }
                }
            }
        }

        if once {
            break;
        }
        tokio::time::sleep(Duration::from_millis(poll_ms)).await;
    }

    Ok(())
}

// ── stage auto-progression ────────────────────────────────────────────────────

/// If the completed card has a `stage_chain` with a next stage, create a
/// next-stage card in `pending/` inheriting spec, glyph, pipeline config,
/// and prior stage output.
fn maybe_advance_stage(cards_dir: &Path, done_card_dir: &Path) {
    let Ok(meta) = jobcard_core::read_meta(done_card_dir) else {
        return;
    };
    if meta.stage_chain.is_empty() {
        return;
    }

    let current_idx = match meta.stage_chain.iter().position(|s| s == &meta.stage) {
        Some(i) => i,
        None => return,
    };

    // If this is the final stage, nothing to advance
    if current_idx + 1 >= meta.stage_chain.len() {
        return;
    }

    let next_stage = &meta.stage_chain[current_idx + 1];
    let next_id = format!("{}-{}", meta.id, next_stage);

    let pending_dir = cards_dir.join("pending");
    let _ = fs::create_dir_all(&pending_dir);
    let next_card_dir = pending_dir.join(format!("{}.jobcard", next_id));
    if next_card_dir.exists() {
        return; // don't overwrite existing card
    }

    let _ = fs::create_dir_all(next_card_dir.join("logs"));
    let _ = fs::create_dir_all(next_card_dir.join("output"));

    // Determine provider for next stage
    let next_provider = meta
        .stage_providers
        .get(next_stage)
        .cloned()
        .unwrap_or_else(|| meta.provider_chain.first().cloned().unwrap_or_default());
    let provider_chain = if next_provider.is_empty() {
        meta.provider_chain.clone()
    } else {
        // Put stage-specific provider first, rest as fallback
        let mut chain = vec![next_provider];
        for p in &meta.provider_chain {
            if !chain.contains(p) {
                chain.push(p.clone());
            }
        }
        chain
    };

    let next_meta = Meta {
        id: next_id.clone(),
        created: Utc::now(),
        stage: next_stage.clone(),
        workflow_mode: meta.workflow_mode.clone(),
        step_index: Some((current_idx + 2) as u32),
        glyph: meta.glyph.clone(),
        title: meta.title.clone(),
        description: meta.description.clone(),
        labels: meta.labels.clone(),
        provider_chain,
        acceptance_criteria: meta.acceptance_criteria.clone(),
        worktree_branch: Some(format!("job/{}", next_id)),
        template_namespace: meta.template_namespace.clone(),
        stage_chain: meta.stage_chain.clone(),
        stage_models: meta.stage_models.clone(),
        stage_providers: meta.stage_providers.clone(),
        stage_budgets: meta.stage_budgets.clone(),
        timeout_seconds: meta.timeout_seconds,
        ..Default::default()
    };
    let _ = write_meta(&next_card_dir, &next_meta);

    // Copy spec.md from parent
    let spec_src = done_card_dir.join("spec.md");
    if spec_src.exists() {
        let _ = fs::copy(&spec_src, next_card_dir.join("spec.md"));
    }

    // Copy prompt.md template from parent
    let prompt_src = done_card_dir.join("prompt.md");
    if prompt_src.exists() {
        let _ = fs::copy(&prompt_src, next_card_dir.join("prompt.md"));
    }

    // Carry prior stage output: copy done card's output/result.md → next card's output/prior_result.md
    let result_src = done_card_dir.join("output").join("result.md");
    if result_src.exists() {
        let _ = fs::copy(
            &result_src,
            next_card_dir.join("output").join("prior_result.md"),
        );
    }

    render_card_thumbnail(&next_card_dir);
    eprintln!(
        "[stage-advance] {} ({}) → {} ({})",
        meta.id, meta.stage, next_id, next_stage
    );
}

// ── child-card-pipeline ───────────────────────────────────────────────────────

fn spawn_child_cards(cards_dir: &Path, done_card_dir: &Path) {
    let yaml_path = done_card_dir.join("output/cards.yaml");
    if !yaml_path.exists() {
        return;
    }

    let Ok(text) = fs::read_to_string(&yaml_path) else {
        return;
    };
    let Ok(entries) = serde_yaml::from_str::<Vec<serde_yaml::Value>>(&text) else {
        return;
    };

    let pending_dir = cards_dir.join("pending");
    let _ = fs::create_dir_all(&pending_dir);

    for entry in entries {
        let Some(id) = entry["id"].as_str() else {
            continue;
        };

        let child_dir = pending_dir.join(format!("{}.jobcard", id));
        if child_dir.exists() {
            continue; // don't overwrite
        }
        let _ = fs::create_dir_all(child_dir.join("logs"));
        let _ = fs::create_dir_all(child_dir.join("output"));

        let meta = serde_json::json!({
            "id": id,
            "title": entry["title"].as_str().unwrap_or(id),
            "description": entry["description"].as_str().unwrap_or(""),
            "stage": entry["stage"].as_str().unwrap_or("spec"),
            "priority": entry["priority"].as_i64().unwrap_or(3),
            "created": chrono::Utc::now().to_rfc3339(),
            "provider_chain": entry["provider_chain"].as_sequence()
                .map(|s| s.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
                .unwrap_or_else(|| vec!["claude"]),
            "stages": {
                "spec": {"status": "pending", "agent": null},
                "plan": {"status": "blocked", "agent": null},
                "implement": {"status": "blocked", "agent": null},
                "qa": {"status": "blocked", "agent": null}
            },
            "acceptance_criteria": entry["acceptance_criteria"].as_sequence()
                .map(|s| s.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
                .unwrap_or_default(),
            "retry_count": 0
        });
        let _ = fs::write(
            child_dir.join("meta.json"),
            serde_json::to_vec_pretty(&meta).unwrap(),
        );

        if let Some(desc) = entry["description"].as_str() {
            let _ = fs::write(
                child_dir.join("spec.md"),
                format!("# {}\n\n{}\n", entry["title"].as_str().unwrap_or(id), desc),
            );
        }

        render_card_thumbnail(&child_dir);

        eprintln!("[child-cards] created {}", id);
    }
}

async fn run_card(
    cards_dir: &Path,
    card_dir: &Path,
    adapter: &str,
    provider_name: &str,
) -> anyhow::Result<(i32, Option<Meta>)> {
    fs::create_dir_all(card_dir.join("logs"))?;
    fs::create_dir_all(card_dir.join("output"))?;

    let prompt_file = card_dir.join("prompt.md");
    if !prompt_file.exists() {
        fs::write(&prompt_file, "")?;
    }

    let stdout_log = card_dir.join("logs").join("stdout.log");
    let stderr_log = card_dir.join("logs").join("stderr.log");
    let memory_out_file = card_dir.join("memory-out.json");
    let _ = fs::remove_file(&memory_out_file);

    // Render prompt template with actual values
    let mut meta = jobcard_core::read_meta(card_dir).ok();
    let memory_namespace = meta
        .as_ref()
        .map(memory_namespace_from_meta)
        .unwrap_or_else(|| "default".to_string());
    if let Some(ref m) = meta {
        let mut ctx = jobcard_core::PromptContext::from_files(card_dir, m)?;
        match read_memory_store(cards_dir, &memory_namespace) {
            Ok(store) => {
                ctx.memory = format_memory_for_prompt(&store);
            }
            Err(err) => {
                let _ = append_log_line(
                    &stderr_log,
                    &format!(
                        "memory load failed (namespace={}): {}",
                        memory_namespace, err
                    ),
                );
            }
        }
        let template = fs::read_to_string(&prompt_file)?;
        let rendered = jobcard_core::render_prompt(&template, &ctx);
        fs::write(&prompt_file, rendered)?;
    }

    let workdir = {
        if let Some(ref m) = meta {
            if let Some(ref p) = m.workspace_path {
                let candidate = PathBuf::from(p);
                if candidate.exists() {
                    candidate
                } else {
                    let ws = card_dir.join("workspace");
                    if ws.exists() {
                        ws
                    } else {
                        card_dir.to_path_buf()
                    }
                }
            } else {
                let ws = card_dir.join("workspace");
                if ws.exists() {
                    ws
                } else {
                    card_dir.to_path_buf()
                }
            }
        } else {
            let ws = card_dir.join("workspace");
            if ws.exists() {
                ws
            } else {
                card_dir.to_path_buf()
            }
        }
    };

    let stage = meta
        .as_ref()
        .map(|m| m.stage.clone())
        .unwrap_or_else(|| "implement".to_string());
    let started_at = Utc::now();
    if let Some(ref mut m) = meta {
        let rec = m
            .stages
            .entry(stage.clone())
            .or_insert(jobcard_core::StageRecord {
                status: jobcard_core::StageStatus::Pending,
                agent: None,
                provider: None,
                duration_s: None,
                started: None,
                blocked_by: None,
            });
        rec.status = jobcard_core::StageStatus::Running;
        rec.started = Some(started_at);
        rec.agent = Some(adapter.to_string());
        rec.provider = Some(provider_name.to_string());
        let _ = write_meta(card_dir, m);
    }

    let mut cmd = if adapter.ends_with(".zsh") {
        let mut c = TokioCommand::new("zsh");
        let adapter_path = if std::path::Path::new(adapter).is_absolute() {
            adapter.to_string()
        } else {
            format!("{}/{}", std::env::current_dir()?.display(), adapter)
        };
        c.arg(adapter_path);
        c
    } else {
        TokioCommand::new(adapter)
    };

    let mut child = cmd
        .arg(&workdir)
        .arg(&prompt_file)
        .arg(&stdout_log)
        .arg(&stderr_log)
        .arg(&memory_out_file)
        .env("JOBCARD_MEMORY_OUT", &memory_out_file)
        .env("JOBCARD_MEMORY_NAMESPACE", &memory_namespace)
        .spawn()
        .with_context(|| format!("failed to spawn adapter: {}", adapter))?;

    let timeout_seconds = meta
        .as_ref()
        .and_then(|m| m.timeout_seconds)
        .unwrap_or(3600);
    let pid = child
        .id()
        .map(|v| v as i32)
        .with_context(|| "spawned adapter without a child PID")?;
    let pid_str = pid.to_string();
    let _ = fs::write(card_dir.join("logs").join("pid"), &pid_str);
    let _ = TokioCommand::new("xattr")
        .arg("-w")
        .arg("com.yourorg.agent-pid")
        .arg(&pid_str)
        .arg(card_dir)
        .status()
        .await;

    let mut lease = RunLease {
        run_id: next_run_id(child.id()),
        pid,
        pid_start_time: started_at,
        started_at,
        heartbeat_at: started_at,
        host: host_name(),
    };
    let _ = write_run_lease(card_dir, &lease);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_seconds);
    let status = loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            let _ = child.kill().await;
            anyhow::bail!("adapter timed out after {} seconds", timeout_seconds);
        }
        let remaining = deadline.saturating_duration_since(now);
        let wait_slice = std::cmp::min(LEASE_HEARTBEAT_INTERVAL, remaining);
        match tokio::time::timeout(wait_slice, child.wait()).await {
            Ok(res) => break res?,
            Err(_) => {
                lease.heartbeat_at = Utc::now();
                let _ = write_run_lease(card_dir, &lease);
            }
        }
    };
    let exit_code = status.code().unwrap_or(1);

    if let Err(err) = merge_memory_output(cards_dir, &memory_namespace, &memory_out_file) {
        let _ = append_log_line(
            &stderr_log,
            &format!(
                "memory merge failed (namespace={}): {}",
                memory_namespace, err
            ),
        );
    }

    let finished_at = Utc::now();
    if let Some(ref mut m) = meta {
        let rec = m.stages.entry(stage).or_insert(jobcard_core::StageRecord {
            status: jobcard_core::StageStatus::Pending,
            agent: None,
            provider: None,
            duration_s: None,
            started: None,
            blocked_by: None,
        });
        rec.status = if exit_code == 0 {
            jobcard_core::StageStatus::Done
        } else if exit_code == 75 {
            jobcard_core::StageStatus::Pending
        } else {
            jobcard_core::StageStatus::Failed
        };
        let duration = finished_at.signed_duration_since(started_at).num_seconds();
        if duration >= 0 {
            rec.duration_s = Some(duration as u64);
        }
    }

    Ok((exit_code, meta))
}

fn rotate_provider_chain(meta: &mut Meta) {
    if meta.provider_chain.len() <= 1 {
        return;
    }
    let first = meta.provider_chain.remove(0);
    meta.provider_chain.push(first);
}

fn select_provider(
    cards_dir: &Path,
    meta: Option<&mut Meta>,
    stage: &str,
) -> anyhow::Result<Option<(String, String, i32)>> {
    let pf = read_providers(cards_dir)?;
    let now = Utc::now().timestamp();

    let avoid_provider = if stage == "qa" {
        meta.as_ref()
            .and_then(|m| m.stages.get("implement"))
            .and_then(|r| r.provider.clone())
    } else {
        None
    };

    let chain: Vec<String> = match meta {
        Some(m) => {
            if m.provider_chain.is_empty() {
                m.provider_chain = vec!["mock".to_string(), "mock2".to_string()];
            }
            m.provider_chain.clone()
        }
        None => vec!["mock".to_string(), "mock2".to_string()],
    };

    let mut fallback: Option<(String, String)> = None;
    for name in chain {
        let Some(p) = pf.providers.get(&name) else {
            continue;
        };
        if let Some(until) = p.cooldown_until_epoch_s {
            if until > now {
                continue;
            }
        }

        if let Some(ref avoid) = avoid_provider {
            if &name == avoid {
                if fallback.is_none() {
                    fallback = Some((name, p.command.clone()));
                }
                continue;
            }
        }

        return Ok(Some((name, p.command.clone(), p.rate_limit_exit)));
    }

    if let Some((name, cmd)) = fallback {
        if let Some(p) = pf.providers.get(&name) {
            return Ok(Some((name, cmd, p.rate_limit_exit)));
        }
    }
    Ok(None)
}

fn set_provider_cooldown(cards_dir: &Path, provider: &str, cooldown_s: i64) -> anyhow::Result<()> {
    let mut pf = read_providers(cards_dir)?;
    let now = Utc::now().timestamp();
    if let Some(p) = pf.providers.get_mut(provider) {
        p.cooldown_until_epoch_s = Some(now + cooldown_s);
    }
    write_providers(cards_dir, &pf)?;
    Ok(())
}

fn remove_worktree(path: &Path, git_root: Option<&Path>) -> anyhow::Result<()> {
    if let Some(root) = git_root {
        let status = StdCommand::new("git")
            .args(["worktree", "remove", "--force", path.to_str().unwrap_or("")])
            .current_dir(root)
            .status();
        if matches!(status, Ok(s) if s.success()) {
            return Ok(());
        }
    }
    fs::remove_dir_all(path).with_context(|| format!("failed to remove {}", path.display()))
}

async fn run_merge_gate(
    cards_dir: &Path,
    poll_ms: u64,
    once: bool,
    vcs_engine: VcsEngine,
) -> anyhow::Result<()> {
    ensure_cards_layout(cards_dir)?;

    let done_dir = cards_dir.join("done");
    let merged_dir = cards_dir.join("merged");
    let failed_dir = cards_dir.join("failed");

    loop {
        if let Ok(entries) = fs::read_dir(&done_dir) {
            for ent in entries.flatten() {
                let card_dir = ent.path();
                if !card_dir.is_dir() {
                    continue;
                }
                if card_dir.extension().and_then(|s| s.to_str()).unwrap_or("") != "jobcard" {
                    continue;
                }

                let name = match card_dir.file_name().and_then(|s| s.to_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };

                let mut meta = match jobcard_core::read_meta(&card_dir) {
                    Ok(m) => m,
                    Err(_) => {
                        let failed_path = failed_dir.join(&name);
                        let _ = fs::rename(&card_dir, &failed_path);
                        render_card_thumbnail(&failed_path);
                        continue;
                    }
                };

                fs::create_dir_all(card_dir.join("logs"))?;
                fs::create_dir_all(card_dir.join("output"))?;

                let workdir = {
                    if let Some(ref p) = meta.workspace_path {
                        let candidate = PathBuf::from(p);
                        if candidate.exists() {
                            candidate
                        } else {
                            let ws = card_dir.join("workspace");
                            if ws.exists() {
                                ws
                            } else {
                                card_dir.clone()
                            }
                        }
                    } else {
                        let ws = card_dir.join("workspace");
                        if ws.exists() {
                            ws
                        } else {
                            card_dir.clone()
                        }
                    }
                };

                let qa_log = card_dir.join("logs").join("qa.log");

                let mut acceptance_ok = true;
                let mut failed_criterion: Option<String> = None;
                for criterion in meta.acceptance_criteria.iter() {
                    let output = TokioCommand::new("sh")
                        .arg("-lc")
                        .arg(criterion)
                        .current_dir(&workdir)
                        .output()
                        .await;

                    match output {
                        Ok(out) => {
                            let mut f = fs::OpenOptions::new()
                                .create(true)
                                .append(true)
                                .open(&qa_log)?;
                            writeln!(f, "--- criterion: {} ---", criterion.replace('\n', "\\n"))?;
                            f.write_all(&out.stdout)?;
                            f.write_all(&out.stderr)?;
                            writeln!(f)?;

                            if !out.status.success() {
                                acceptance_ok = false;
                                failed_criterion = Some(criterion.to_string());
                                break;
                            }
                        }
                        Err(_) => {
                            acceptance_ok = false;
                            failed_criterion = Some(criterion.to_string());
                            break;
                        }
                    }
                }

                if !acceptance_ok {
                    meta.failure_reason = Some("acceptance_criteria_failed".to_string());
                    let report = format!(
                        "criterion failed: {}\n",
                        failed_criterion.unwrap_or_else(|| "<unknown>".to_string())
                    );
                    let _ = fs::write(card_dir.join("output").join("qa_report.md"), report);
                    let _ = write_meta(&card_dir, &meta);
                    let failed_path = failed_dir.join(&name);
                    let _ = fs::rename(&card_dir, &failed_path);
                    render_card_thumbnail(&failed_path);
                    continue;
                }

                if let Err(err) = policy_check_card(cards_dir, &card_dir, &meta.id) {
                    meta.failure_reason = Some("policy_violation".to_string());
                    meta.policy_result = Some(format!("failed: {err}"));
                    let _ = fs::write(
                        card_dir.join("output").join("qa_report.md"),
                        format!("policy violation: {err}\n"),
                    );
                    let _ = write_meta(&card_dir, &meta);
                    let failed_path = failed_dir.join(&name);
                    let _ = fs::rename(&card_dir, &failed_path);
                    render_card_thumbnail(&failed_path);
                    continue;
                }
                meta.policy_result = Some("pass".to_string());

                let ws_path = if let Some(ref p) = meta.workspace_path {
                    PathBuf::from(p)
                } else {
                    card_dir.join("workspace")
                };

                if ws_path.exists() {
                    let mut vcs_err: Option<String> = None;
                    match vcs_engine {
                        VcsEngine::GitGt => {
                            let Some(git_root) = find_git_root(cards_dir) else {
                                meta.failure_reason = Some("git_root_not_found".to_string());
                                meta.policy_result = Some("failed".to_string());
                                let _ = write_meta(&card_dir, &meta);
                                let failed_path = failed_dir.join(&name);
                                let _ = fs::rename(&card_dir, &failed_path);
                                render_card_thumbnail(&failed_path);
                                continue;
                            };

                            let add_status = StdCommand::new("git")
                                .args(["add", "-A"])
                                .current_dir(&ws_path)
                                .status();
                            if !matches!(add_status, Ok(s) if s.success()) {
                                vcs_err = Some("git add -A failed".to_string());
                            }

                            if vcs_err.is_none() {
                                let diff_cached = StdCommand::new("git")
                                    .args(["diff", "--cached", "--quiet"])
                                    .current_dir(&ws_path)
                                    .status();
                                let has_staged =
                                    matches!(diff_cached, Ok(s) if s.code() == Some(1));
                                if has_staged {
                                    let msg = format!("jobcard: {}", meta.id);
                                    let commit_status = StdCommand::new("git")
                                        .args(["commit", "-m", &msg])
                                        .current_dir(&ws_path)
                                        .status();
                                    if !matches!(commit_status, Ok(s) if s.success()) {
                                        vcs_err = Some("git commit failed".to_string());
                                    }
                                }
                            }

                            if vcs_err.is_none() {
                                let restack = StdCommand::new("gt")
                                    .args(["stack", "restack", "--no-interactive"])
                                    .current_dir(&ws_path)
                                    .status();
                                if !matches!(restack, Ok(s) if s.success()) {
                                    vcs_err = Some("gt stack restack failed".to_string());
                                }
                            }

                            if vcs_err.is_none() {
                                let submit = StdCommand::new("gt")
                                    .args([
                                        "submit",
                                        "--stack",
                                        "--no-interactive",
                                        "--no-edit",
                                        "--draft",
                                    ])
                                    .current_dir(&ws_path)
                                    .status();
                                if !matches!(submit, Ok(s) if s.success()) {
                                    vcs_err = Some("gt submit failed".to_string());
                                }
                            }

                            let _ = remove_worktree(&ws_path, Some(&git_root));
                        }
                        VcsEngine::Jj => {
                            let repo_root =
                                find_git_root(cards_dir).unwrap_or_else(|| cards_dir.to_path_buf());
                            if let Err(e) = jobcard_core::worktree::squash_workspace(&ws_path) {
                                vcs_err = Some(format!("jj squash failed: {e}"));
                            }
                            if vcs_err.is_none() {
                                let ws_name = meta
                                    .workspace_name
                                    .clone()
                                    .unwrap_or_else(|| "workspace".to_string());
                                if let Err(e) =
                                    jobcard_core::worktree::forget_workspace(&repo_root, &ws_name)
                                {
                                    vcs_err = Some(format!("jj workspace forget failed: {e}"));
                                }
                            }
                            if vcs_err.is_none() {
                                if let Err(e) =
                                    jobcard_core::worktree::push_stack(&repo_root, "origin")
                                {
                                    vcs_err = Some(format!("jj git push failed: {e}"));
                                }
                            }
                            if vcs_err.is_none() {
                                let pr_result = StdCommand::new("gh")
                                    .args(["pr", "create", "--fill", "--draft"])
                                    .current_dir(&repo_root)
                                    .output();
                                match pr_result {
                                    Ok(out) if out.status.success() => {}
                                    Ok(out) => {
                                        vcs_err = Some(format!(
                                            "gh pr create failed: {}",
                                            String::from_utf8_lossy(&out.stderr).trim()
                                        ));
                                    }
                                    Err(e) => {
                                        vcs_err = Some(format!("gh pr create failed: {e}"));
                                    }
                                }
                            }
                        }
                    }

                    if let Some(err) = vcs_err {
                        let _ = fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&qa_log)
                            .and_then(|mut f| f.write_all(format!("{err}\n").as_bytes()));
                        meta.failure_reason = Some("vcs_publish_failed".to_string());
                        meta.policy_result = Some("failed".to_string());
                        let _ = write_meta(&card_dir, &meta);
                        let failed_path = failed_dir.join(&name);
                        let _ = fs::rename(&card_dir, &failed_path);
                        render_card_thumbnail(&failed_path);
                        continue;
                    }
                }

                // Best-effort: capture file-change manifest for Quick Look
                let branch = meta.change_ref.clone().unwrap_or_else(|| meta.id.clone());
                let _ = write_changes_json(&card_dir, &workdir, &branch).await;

                let _ = write_meta(&card_dir, &meta);
                let merged_path = merged_dir.join(&name);
                let _ = fs::rename(&card_dir, &merged_path);
                maybe_hfs_compress_card(&merged_path);
                render_card_thumbnail(&merged_path);
            }
        }

        if once {
            break;
        }
        tokio::time::sleep(Duration::from_millis(poll_ms)).await;
    }

    Ok(())
}

fn copy_dir_all(src: &Path, dst: &Path) -> anyhow::Result<()> {
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

// ── retry ────────────────────────────────────────────────────────────────────

fn cmd_retry(root: &Path, id: &str) -> anyhow::Result<()> {
    let message = retry_card(root, id)?;
    println!("{}", message);
    Ok(())
}

fn retry_card(root: &Path, id: &str) -> anyhow::Result<String> {
    let card = find_card(root, id).with_context(|| format!("card not found: {}", id))?;
    let state = card
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    if state == "running" {
        anyhow::bail!("card '{}' is currently running; kill it first", id);
    }
    if state == "pending" {
        anyhow::bail!("card '{}' is already pending", id);
    }

    // Update meta before rename so the write is in the stable card location
    if let Ok(mut meta) = jobcard_core::read_meta(&card) {
        meta.retry_count = Some(meta.retry_count.unwrap_or(0).saturating_add(1));
        meta.failure_reason = None;
        for stage in meta.stages.values_mut() {
            if matches!(stage.status, StageStatus::Running | StageStatus::Failed) {
                stage.status = StageStatus::Pending;
                stage.agent = None;
                stage.provider = None;
                stage.duration_s = None;
                stage.started = None;
                stage.blocked_by = None;
            }
        }
        let _ = write_meta(&card, &meta);
    }

    let target = root.join("pending").join(format!("{}.jobcard", id));
    fs::rename(&card, &target)
        .with_context(|| format!("failed to move card to pending/: {}", id))?;
    render_card_thumbnail(&target);
    Ok(format!("retrying: {} -> pending/", id))
}

// ── kill ─────────────────────────────────────────────────────────────────────

async fn cmd_kill(root: &Path, id: &str) -> anyhow::Result<()> {
    let message = kill_card(root, id).await?;
    println!("{}", message);
    Ok(())
}

async fn kill_card(root: &Path, id: &str) -> anyhow::Result<String> {
    let card = find_card(root, id).with_context(|| format!("card not found: {}", id))?;
    let state = card
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    if state != "running" {
        anyhow::bail!("card '{}' is not running (state: {})", id, state);
    }

    let pid = read_pid(&card)
        .await?
        .with_context(|| format!("no PID found for card '{}'", id))?;

    let mut was_running = is_alive(pid).await.unwrap_or(false);
    if was_running {
        // Send SIGTERM (kill -15)
        let sent = TokioCommand::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .status()
            .await
            .with_context(|| format!("failed to send SIGTERM to pid {}", pid))?;

        if !sent.success() {
            // The process may have exited between the liveness check and kill.
            was_running = is_alive(pid).await.unwrap_or(false);
            if was_running {
                anyhow::bail!("kill -TERM {} returned non-zero", pid);
            }
        }
    }

    // Update meta with failure reason
    if let Ok(mut meta) = jobcard_core::read_meta(&card) {
        meta.failure_reason = Some("killed".to_string());
        let _ = write_meta(&card, &meta);
    }

    let failed_dir = root.join("failed");
    let target = failed_dir.join(format!("{}.jobcard", id));
    fs::rename(&card, &target)
        .with_context(|| format!("failed to move card to failed/: {}", id))?;
    render_card_thumbnail(&target);

    if was_running {
        Ok(format!("killed pid {} and moved '{}' to failed/", pid, id))
    } else {
        Ok(format!(
            "pid {} was not alive; moved '{}' to failed as stale running card",
            pid, id
        ))
    }
}

// ── approve ───────────────────────────────────────────────────────────────────

fn cmd_approve(root: &Path, id: &str) -> anyhow::Result<()> {
    approve_card(root, id)?;
    Ok(())
}

fn approve_card(root: &Path, id: &str) -> anyhow::Result<()> {
    let card = find_card(root, id).with_context(|| format!("card not found: {}", id))?;
    let state = card
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    let mut meta = jobcard_core::read_meta(&card)?;
    meta.decision_required = false;

    if state == "pending" {
        if let Some(record) = meta.stages.get_mut(&meta.stage) {
            if record.status == jobcard_core::StageStatus::Blocked {
                record.status = jobcard_core::StageStatus::Pending;
            }
        }
    }

    write_meta(&card, &meta)?;
    render_card_thumbnail(&card);
    println!("Approved {}", id);
    Ok(())
}

// ── logs ─────────────────────────────────────────────────────────────────────

async fn cmd_logs(root: &Path, id: &str, follow: bool) -> anyhow::Result<()> {
    use std::io::IsTerminal;
    let card = find_card(root, id).with_context(|| format!("card not found: {}", id))?;
    let stdout_log = card.join("logs").join("stdout.log");
    let stderr_log = card.join("logs").join("stderr.log");
    let is_tty = std::io::stdout().is_terminal();

    if !follow {
        // Print all existing content once
        print_log_section("stdout", &stdout_log, is_tty)?;
        print_log_section("stderr", &stderr_log, is_tty)?;
        return Ok(());
    }

    // --follow: open both files and stream new bytes as they arrive
    let mut stdout_file = fs::File::open(&stdout_log)
        .unwrap_or_else(|_| fs::File::open("/dev/null").expect("open /dev/null"));
    let mut stderr_file = fs::File::open(&stderr_log)
        .unwrap_or_else(|_| fs::File::open("/dev/null").expect("open /dev/null"));

    // Drain any existing content first
    let mut buf = Vec::new();
    stdout_file.read_to_end(&mut buf)?;
    if !buf.is_empty() {
        print!("{}", colorize_chunk(&buf, is_tty));
    }
    let mut stdout_pos = stdout_file.stream_position()?;
    buf.clear();

    stderr_file.read_to_end(&mut buf)?;
    if !buf.is_empty() {
        eprint!("{}", colorize_chunk(&buf, is_tty));
    }
    let mut stderr_pos = stderr_file.stream_position()?;

    loop {
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Re-open if file was rotated/created after we started
        if !stdout_log.exists() {
            if let Ok(f) = fs::File::open(&stdout_log) {
                stdout_file = f;
                stdout_pos = 0;
            }
        }
        if !stderr_log.exists() {
            if let Ok(f) = fs::File::open(&stderr_log) {
                stderr_file = f;
                stderr_pos = 0;
            }
        }

        // Read any new bytes from stdout
        stdout_file.seek(SeekFrom::Start(stdout_pos))?;
        buf.clear();
        stdout_file.read_to_end(&mut buf)?;
        if !buf.is_empty() {
            print!("{}", colorize_chunk(&buf, is_tty));
            std::io::stdout().flush()?;
            stdout_pos += buf.len() as u64;
        }

        // Read any new bytes from stderr
        stderr_file.seek(SeekFrom::Start(stderr_pos))?;
        buf.clear();
        stderr_file.read_to_end(&mut buf)?;
        if !buf.is_empty() {
            eprint!("{}", colorize_chunk(&buf, is_tty));
            std::io::stderr().flush()?;
            stderr_pos += buf.len() as u64;
        }

        // Stop following once the card leaves running/
        let still_running = find_card_in_state(root, id, "running");
        if !still_running {
            break;
        }
    }

    Ok(())
}

fn colorize_log_line(line: &str) -> String {
    const R: &str = "\x1b[0m";
    if line.contains("ERROR") || line.contains("error:") || line.contains("FAILED") {
        return format!("\x1b[1;31m{}{}", line, R);
    }
    if line.contains("WARN") || line.contains("warning:") {
        return format!("\x1b[33m{}{}", line, R);
    }
    if line.contains("INFO") {
        return format!("\x1b[36m{}{}", line, R);
    }
    if line.contains("DEBUG") || line.contains("TRACE") {
        return format!("\x1b[2m{}{}", line, R);
    }
    if line.contains("→ merged") || line.contains("-> merged") {
        return format!("\x1b[1;35m{}{}", line, R);
    }
    if line.contains("→ done") || line.contains("-> done") {
        return format!("\x1b[1;32m{}{}", line, R);
    }
    if line.contains("→ failed") || line.contains("-> failed") {
        return format!("\x1b[1;31m{}{}", line, R);
    }
    if line.contains("→ running") || line.contains("-> running") {
        return format!("\x1b[1;33m{}{}", line, R);
    }
    line.to_string()
}

fn colorize_chunk(bytes: &[u8], is_tty: bool) -> String {
    let text = String::from_utf8_lossy(bytes);
    if !is_tty {
        return text.into_owned();
    }
    text.lines()
        .map(colorize_log_line)
        .collect::<Vec<_>>()
        .join("\n")
        + if text.ends_with('\n') { "\n" } else { "" }
}

fn print_log_section(label: &str, path: &Path, is_tty: bool) -> anyhow::Result<()> {
    if !path.exists() {
        println!("=== {} (no file) ===", label);
        return Ok(());
    }
    let content = fs::read_to_string(path)?;
    if is_tty {
        println!("\x1b[1m=== {} ===\x1b[0m", label);
    } else {
        println!("=== {} ===", label);
    }
    if is_tty {
        for line in content.lines() {
            println!("{}", colorize_log_line(line));
        }
        if !content.ends_with('\n') && !content.is_empty() {
            // already printed trailing newline via println
        }
    } else {
        print!("{}", content);
        if !content.ends_with('\n') && !content.is_empty() {
            println!();
        }
    }
    Ok(())
}

fn find_card_in_state(root: &Path, id: &str, state: &str) -> bool {
    let state_dir = root.join(state);
    if state_dir.join(format!("{}.jobcard", id)).exists() {
        return true;
    }
    let suffix = format!("-{}.jobcard", id);
    fs::read_dir(&state_dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| {
            e.path()
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.ends_with(&suffix))
                .unwrap_or(false)
        })
}

// ── inspect ──────────────────────────────────────────────────────────────────

fn cmd_inspect(root: &Path, id: &str) -> anyhow::Result<()> {
    let card = find_card(root, id).with_context(|| format!("card not found: {}", id))?;
    let state = card
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    println!("=== meta ({}) ===", state);
    let meta = jobcard_core::read_meta(&card)?;
    println!("{}", serde_json::to_string_pretty(&meta)?);

    let spec_path = card.join("spec.md");
    if spec_path.exists() {
        let spec = fs::read_to_string(&spec_path)?;
        println!("\n=== spec.md ===");
        print!("{}", spec);
        if !spec.ends_with('\n') && !spec.is_empty() {
            println!();
        }
    }

    for (label, filename) in [("stdout", "stdout.log"), ("stderr", "stderr.log")] {
        let log_path = card.join("logs").join(filename);
        if log_path.exists() {
            let content = fs::read_to_string(&log_path)?;
            let lines: Vec<&str> = content.lines().collect();
            let tail_lines = if lines.len() > 20 {
                &lines[lines.len() - 20..]
            } else {
                &lines[..]
            };
            println!("\n=== {} (last {} lines) ===", label, tail_lines.len());
            for line in tail_lines {
                println!("{}", line);
            }
        }
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────

fn cmd_meta_set(
    root: &Path,
    id: &str,
    workflow_mode: Option<&str>,
    step_index: Option<u32>,
    clear_workflow_mode: bool,
    clear_step_index: bool,
) -> anyhow::Result<()> {
    if clear_workflow_mode && workflow_mode.is_some() {
        anyhow::bail!("cannot set and clear workflow_mode in the same command");
    }
    if clear_step_index && step_index.is_some() {
        anyhow::bail!("cannot set and clear step_index in the same command");
    }
    if workflow_mode.is_none() && step_index.is_none() && !clear_workflow_mode && !clear_step_index
    {
        anyhow::bail!("no changes requested; use --workflow-mode/--step-index or clear flags");
    }

    let card = find_card(root, id).with_context(|| format!("card not found: {}", id))?;
    let mut meta = jobcard_core::read_meta(&card)?;

    if clear_workflow_mode {
        meta.workflow_mode = None;
        meta.step_index = None;
    }
    if let Some(mode) = workflow_mode {
        let mode = mode.trim();
        if mode.is_empty() {
            anyhow::bail!("workflow_mode cannot be empty");
        }
        meta.workflow_mode = Some(mode.to_string());
        if meta.step_index.is_none() {
            meta.step_index = Some(1);
        }
    }
    if clear_step_index {
        meta.step_index = None;
    }
    if let Some(idx) = step_index {
        meta.step_index = Some(idx);
    }

    write_meta(&card, &meta)?;
    println!(
        "updated {}: workflow_mode={:?} step_index={:?}",
        id, meta.workflow_mode, meta.step_index
    );
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────

fn find_card(root: &Path, id: &str) -> Option<PathBuf> {
    let suffix = format!("-{}.jobcard", id);
    let exact = format!("{}.jobcard", id);
    for dir in ["pending", "running", "done", "merged", "failed"] {
        let state_dir = root.join(dir);
        // Exact match (legacy / no-glyph prefix)
        let p = state_dir.join(&exact);
        if p.exists() {
            return Some(p);
        }
        // Glyph-prefixed match: {glyph}-{id}.jobcard
        if let Ok(entries) = fs::read_dir(&state_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir()
                    && path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.ends_with(&suffix))
                        .unwrap_or(false)
                {
                    return Some(path);
                }
            }
        }
    }
    None
}

// ── realtime validation ───────────────────────────────────────────────────────

/// Scan `output/*.json` in a job card directory, validate each file as a
/// [`FeedRecord`], write a structured audit log to `logs/validation.log`, and
/// return an aggregated [`ValidationSummary`].
///
/// If `feed_config.json` exists in the card directory it is used as the
/// [`FeedConfig`]; otherwise a permissive default is applied.
fn validate_realtime_output(
    card_dir: &Path,
) -> anyhow::Result<jobcard_core::realtime::ValidationSummary> {
    use jobcard_core::realtime::{
        check_alerts, validate_record, AlertSeverity, FeedMetrics, FeedRecord, ValidationSummary,
    };

    let output_dir = card_dir.join("output");
    let validation_log = card_dir.join("logs").join("validation.log");
    let config = load_feed_config(card_dir);

    let feed_id = card_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();
    let mut metrics = FeedMetrics::new(feed_id);
    let mut error_entries: Vec<serde_json::Value> = Vec::new();

    if output_dir.exists() {
        for entry in fs::read_dir(&output_dir)?.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let bytes = match fs::read(&path) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let record: FeedRecord = match serde_json::from_slice(&bytes) {
                Ok(r) => r,
                Err(_) => continue,
            };
            let result = validate_record(&record, &config);
            if !result.valid {
                error_entries.push(serde_json::json!({
                    "file": path.file_name().and_then(|s| s.to_str()),
                    "feed_id": record.feed_id,
                    "errors": result.errors,
                }));
            }
            metrics.record_received(result.valid);
        }
    }

    let alerts = check_alerts(&metrics);
    let alert_count = alerts.len() as u64;
    let critical_alerts = alerts
        .iter()
        .filter(|a| a.severity == AlertSeverity::Critical)
        .count() as u64;

    let log_content = serde_json::json!({
        "total": metrics.records_received,
        "valid": metrics.records_valid,
        "invalid": metrics.records_invalid,
        "health": metrics.health,
        "alerts": alerts.iter().map(|a| serde_json::json!({
            "severity": a.severity,
            "message": a.message,
            "timestamp": a.timestamp,
        })).collect::<Vec<_>>(),
        "validation_errors": error_entries,
    });
    let _ = fs::create_dir_all(card_dir.join("logs"));
    let _ = fs::write(&validation_log, serde_json::to_vec_pretty(&log_content)?);

    Ok(ValidationSummary {
        total: metrics.records_received,
        valid: metrics.records_valid,
        invalid: metrics.records_invalid,
        alert_count,
        critical_alerts,
        health: metrics.health,
    })
}

/// Load a [`FeedConfig`] from `card_dir/feed_config.json`, or return a
/// permissive default that accepts any well-formed [`FeedRecord`].
fn load_feed_config(card_dir: &Path) -> jobcard_core::realtime::FeedConfig {
    use jobcard_core::realtime::{FeedConfig, FeedSourceType, ValidationConfig};

    let path = card_dir.join("feed_config.json");
    if let Ok(bytes) = fs::read(&path) {
        if let Ok(cfg) = serde_json::from_slice::<FeedConfig>(&bytes) {
            return cfg;
        }
    }

    let id = card_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("job")
        .to_string();

    FeedConfig {
        id,
        source_type: FeedSourceType::File,
        endpoint: card_dir.display().to_string(),
        poll_interval_secs: 0,
        validation: ValidationConfig {
            required_fields: vec!["feed_id".to_string()],
            // Large but not u64::MAX to avoid overflow in signed arithmetic.
            max_staleness_secs: 60 * 60 * 24 * 365 * 10,
            value_ranges: std::collections::HashMap::new(),
        },
    }
}
