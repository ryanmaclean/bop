use anyhow::Context;
use async_stream::stream;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::{
    sse::{Event, Sse},
    Html, IntoResponse,
};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use clap::{Parser, Subcommand};
use crossterm::{
    event::{self, Event as CEvent, KeyCode, KeyEventKind},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use jobcard_core::{write_meta, Meta};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap};
use ratatui::{Frame, Terminal};
use serde::{Deserialize, Serialize};
use std::cmp::min;
use std::collections::BTreeMap;
use std::convert::Infallible;
use std::fmt::Write as _;
use std::fs;
use std::io::{IsTerminal, Read, Seek, SeekFrom, Write};
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};
use tokio::net::TcpListener;
use tokio::process::Command as TokioCommand;
use tower_http::cors::CorsLayer;
use utoipa::{OpenApi, ToSchema};
use walkdir::WalkDir;

#[derive(Parser, Debug)]
#[command(name = "jc")]
struct Cli {
    #[arg(long, default_value = ".cards")]
    cards_dir: String,

    #[command(subcommand)]
    cmd: Command,
}

const DEFAULT_MEMORY_TTL_SECONDS: i64 = 60 * 60 * 24 * 30;
const UI_SSE_POLL_MS: u64 = 1000;
const UI_MAX_FILE_PREVIEW_BYTES: usize = 64 * 1024;

#[derive(Subcommand, Debug)]
enum Command {
    Init,
    New {
        template: String,
        id: String,
    },
    /// Create a new job draft from a natural-language description.
    Create {
        /// Plain-language task description used to generate the card draft.
        #[arg(long = "from-description")]
        from_description: String,
        /// Optional explicit id for the generated card.
        #[arg(long)]
        id: Option<String>,
        /// Skip confirmation prompt and write draft immediately.
        #[arg(long)]
        yes: bool,
    },
    Status {
        #[arg(default_value = "")]
        id: String,
    },
    /// Open a live terminal dashboard for all jobs.
    Dashboard,
    Validate {
        id: String,
        /// Run realtime feed validation on the job's output records.
        #[arg(long)]
        realtime: bool,
    },
    Dispatcher {
        #[arg(long, default_value = "adapters/mock.sh")]
        adapter: String,

        #[arg(long, default_value_t = 1)]
        max_workers: usize,

        #[arg(long, default_value_t = 500)]
        poll_ms: u64,

        #[arg(long, default_value_t = 3)]
        max_retries: u32,

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
    },
    MergeGate {
        #[arg(long, default_value_t = 500)]
        poll_ms: u64,

        #[arg(long)]
        once: bool,
    },
    /// Move a card back to pending/ so the dispatcher picks it up again.
    Retry {
        id: String,
    },
    /// Send SIGTERM to the running agent and mark the card as failed.
    Kill {
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
    /// Manage per-template persistent memory.
    Memory {
        #[command(subcommand)]
        cmd: MemoryCommand,
    },
    /// Start the REST API server for CI/CD integration.
    Serve {
        /// Port to listen on.
        #[arg(long, default_value_t = 8080)]
        port: u16,
        /// Bind host or IP (default localhost). WARNING: non-localhost exposes
        /// unauthenticated job control endpoints.
        #[arg(long, default_value = "127.0.0.1")]
        bind: String,
        /// Serve the browser dashboard at /ui.
        #[arg(long)]
        ui: bool,
    },
    /// Manage git worktrees associated with job cards.
    Worktree {
        #[command(subcommand)]
        action: WorktreeAction,
    },
    /// Manage AI providers (list, add, remove, status).
    Providers {
        #[command(subcommand)]
        cmd: ProvidersCommand,
    },
    /// Read and write global/project config settings.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Subcommand, Debug)]
enum ConfigAction {
    /// Print the current value of a config key.
    Get { key: String },
    /// Set a config key to a value (writes to the config file).
    Set { key: String, value: String },
}

#[derive(Subcommand, Debug)]
enum ProvidersCommand {
    /// List all configured providers.
    List,
    /// Add a new provider.
    Add {
        name: String,
        #[arg(long)]
        adapter: String,
        #[arg(long)]
        model: Option<String>,
    },
    /// Remove a provider.
    Remove {
        name: String,
        #[arg(long)]
        force: bool,
    },
    /// Show per-provider job statistics.
    Status,
}

#[derive(Subcommand, Debug)]
enum WorktreeAction {
    /// List all job card worktrees and flag orphans.
    List,
    /// Create a git worktree for a pending or running job card.
    Create { id: String },
    /// Remove worktrees for done/merged cards or orphaned git worktrees.
    Clean {
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand, Debug)]
enum MemoryCommand {
    /// List all memory entries in a namespace.
    List { namespace: String },
    /// Get a single memory entry value by key.
    Get { namespace: String, key: String },
    /// Set a memory entry with a TTL.
    Set {
        namespace: String,
        key: String,
        value: String,
        #[arg(long, default_value_t = DEFAULT_MEMORY_TTL_SECONDS)]
        ttl_seconds: i64,
    },
    /// Delete a memory entry by key.
    Delete { namespace: String, key: String },
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

    // Try APFS COW clone on macOS
    if cfg!(target_os = "macos") {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GeneratedCardDraft {
    #[serde(default, alias = "template")]
    suggested_template: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default, alias = "spec")]
    spec_md: String,
    #[serde(default)]
    acceptance_criteria: Vec<String>,
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
            command: "adapters/mock.sh".to_string(),
            rate_limit_exit: 75,
            cooldown_until_epoch_s: None,
            model: None,
        },
    );
    pf.providers.insert(
        "mock2".to_string(),
        Provider {
            command: "adapters/mock.sh".to_string(),
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
) -> anyhow::Result<()> {
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
        let Some(pid) = pid else {
            continue;
        };

        if is_alive(pid).await? {
            continue;
        }

        let mut meta = jobcard_core::read_meta(&card_dir).ok();
        let retry_count = meta.as_ref().and_then(|m| m.retry_count).unwrap_or(0);
        let next_retry = retry_count.saturating_add(1);
        if let Some(ref mut m) = meta {
            m.retry_count = Some(next_retry);
            if next_retry > max_retries {
                m.failure_reason = Some("max_retries_exceeded".to_string());
            }
            let _ = write_meta(&card_dir, m);
        }

        let name = match card_dir.file_name().and_then(|s| s.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let target = if next_retry > max_retries {
            failed_dir.join(&name)
        } else {
            pending_dir.join(&name)
        };
        let _ = fs::rename(&card_dir, &target);
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
            priority: None,
            provider_chain: vec![],
            stages: Default::default(),
            acceptance_criteria: vec![],
            worktree_branch: Some("job/template-implement".to_string()),
            template_namespace: Some("implement".to_string()),
            retry_count: Some(0),
            failure_reason: None,
            validation_summary: None,
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

    let card_dir = cards_dir.join("pending").join(format!("{}.jobcard", id));
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
        priority: None,
        provider_chain: vec![],
        stages: Default::default(),
        acceptance_criteria: vec![],
        worktree_branch: Some(format!("job/{}", id)),
        template_namespace: Some(template.to_string()),
        retry_count: Some(0),
        failure_reason: None,
        validation_summary: None,
    });

    meta.id = id.to_string();
    meta.created = Utc::now();
    meta.worktree_branch = Some(format!("job/{}", id));
    meta.template_namespace = Some(template.to_string());
    meta.retry_count = Some(0);
    meta.failure_reason = None;

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

    Ok(card_dir)
}

fn list_templates(cards_dir: &Path) -> anyhow::Result<Vec<String>> {
    let templates_dir = cards_dir.join("templates");
    let entries = fs::read_dir(&templates_dir).with_context(|| {
        format!(
            "failed to read templates directory: {}",
            templates_dir.display()
        )
    })?;

    let mut templates = Vec::new();
    for ent in entries.flatten() {
        let path = ent.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if let Some(stripped) = name.strip_suffix(".jobcard") {
            if !stripped.trim().is_empty() {
                templates.push(stripped.to_string());
            }
        }
    }
    templates.sort();
    if templates.is_empty() {
        anyhow::bail!("no templates found under {}", templates_dir.display());
    }
    Ok(templates)
}

fn select_default_provider(cards_dir: &Path) -> anyhow::Result<(String, Provider)> {
    let pf = read_providers(cards_dir)?;
    if pf.providers.is_empty() {
        anyhow::bail!(
            "no providers configured in {}",
            providers_path(cards_dir).display()
        );
    }

    if let Some(name) = pf
        .default_provider
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let provider = pf
            .providers
            .get(name)
            .cloned()
            .with_context(|| format!("default provider '{}' not found", name))?;
        return Ok((name.to_string(), provider));
    }

    if let Some(provider) = pf.providers.get("mock").cloned() {
        return Ok(("mock".to_string(), provider));
    }

    let (name, provider) = pf
        .providers
        .iter()
        .next()
        .map(|(name, provider)| (name.clone(), provider.clone()))
        .context("no providers configured")?;
    Ok((name, provider))
}

fn build_generation_prompt(description: &str, templates: &[String]) -> String {
    let available = templates.join(", ");
    format!(
        "You generate JobCard drafts.\n\
Return ONLY JSON and no markdown.\n\
Required keys: suggested_template, id, spec_md, acceptance_criteria.\n\
- suggested_template: choose one of: {available}\n\
- id: kebab-case short id\n\
- spec_md: complete markdown spec\n\
- acceptance_criteria: array of concrete acceptance criteria strings\n\
Task description:\n\
{description}\n"
    )
}

async fn run_provider_prompt(adapter: &str, prompt: &str) -> anyhow::Result<(i32, String, String)> {
    let nonce = Utc::now()
        .timestamp_nanos_opt()
        .unwrap_or_else(|| Utc::now().timestamp_millis().saturating_mul(1_000_000));
    let temp_dir =
        std::env::temp_dir().join(format!("jc-create-draft-{}-{}", std::process::id(), nonce));
    fs::create_dir_all(&temp_dir)?;

    let prompt_file = temp_dir.join("prompt.md");
    let stdout_log = temp_dir.join("stdout.log");
    let stderr_log = temp_dir.join("stderr.log");
    let memory_out = temp_dir.join("memory-out.json");

    fs::write(&prompt_file, prompt)?;

    let mut cmd = if adapter.ends_with(".sh") {
        let mut c = TokioCommand::new("bash");
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

    let status = cmd
        .arg(&temp_dir)
        .arg(&prompt_file)
        .arg(&stdout_log)
        .arg(&stderr_log)
        .arg(&memory_out)
        .env("JOBCARD_MEMORY_OUT", &memory_out)
        .status()
        .await
        .with_context(|| format!("failed to spawn adapter: {}", adapter))?;

    let code = status.code().unwrap_or(1);
    let stdout = fs::read_to_string(&stdout_log).unwrap_or_default();
    let stderr = fs::read_to_string(&stderr_log).unwrap_or_default();
    let _ = fs::remove_dir_all(&temp_dir);

    Ok((code, stdout, stderr))
}

fn parse_generated_draft(stdout: &str) -> anyhow::Result<GeneratedCardDraft> {
    let trimmed = stdout.trim();
    if let Ok(draft) = serde_json::from_str::<GeneratedCardDraft>(trimmed) {
        return Ok(draft);
    }

    for (idx, ch) in stdout.char_indices() {
        if ch != '{' {
            continue;
        }
        let slice = &stdout[idx..];
        let mut de = serde_json::Deserializer::from_str(slice);
        let Ok(value) = serde_json::Value::deserialize(&mut de) else {
            continue;
        };
        let Ok(draft) = serde_json::from_value::<GeneratedCardDraft>(value) else {
            continue;
        };
        return Ok(draft);
    }

    anyhow::bail!("provider output did not contain parseable draft JSON");
}

fn sanitize_card_id_candidate(raw: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;

    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }

    while out.starts_with('-') {
        out.remove(0);
    }
    while out.ends_with('-') {
        out.pop();
    }

    if out.is_empty() {
        "job".to_string()
    } else {
        out
    }
}

fn card_exists_anywhere(cards_dir: &Path, id: &str) -> bool {
    for dir in ["pending", "running", "done", "merged", "failed"] {
        if cards_dir.join(dir).join(format!("{}.jobcard", id)).exists() {
            return true;
        }
    }
    false
}

fn ensure_unique_card_id(cards_dir: &Path, base_id: &str) -> String {
    let base = sanitize_card_id_candidate(base_id);
    if !card_exists_anywhere(cards_dir, &base) {
        return base;
    }

    let mut i = 2_u64;
    loop {
        let candidate = format!("{}-{}", base, i);
        if !card_exists_anywhere(cards_dir, &candidate) {
            return candidate;
        }
        i = i.saturating_add(1);
    }
}

fn choose_template(templates: &[String], suggested: &str) -> String {
    let trimmed = suggested.trim();
    if templates.iter().any(|t| t == trimmed) {
        return trimmed.to_string();
    }

    if templates.iter().any(|t| t == "implement") {
        return "implement".to_string();
    }

    templates[0].clone()
}

fn read_confirmation() -> anyhow::Result<bool> {
    print!("Write this draft to pending/? [y/N]: ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    let n = std::io::stdin().read_line(&mut line)?;
    if n == 0 {
        return Ok(false);
    }
    let answer = line.trim().to_ascii_lowercase();
    Ok(answer == "y" || answer == "yes")
}

async fn cmd_create_from_description(
    cards_dir: &Path,
    description: &str,
    id_override: Option<&str>,
    auto_confirm: bool,
) -> anyhow::Result<()> {
    let description = description.trim();
    if description.is_empty() {
        anyhow::bail!("description cannot be empty");
    }

    ensure_cards_layout(cards_dir)?;
    seed_default_templates(cards_dir)?;
    seed_providers(cards_dir)?;

    let templates = list_templates(cards_dir)?;
    let (provider_name, provider) = select_default_provider(cards_dir)?;
    let prompt = build_generation_prompt(description, &templates);
    let (code, stdout, stderr) = run_provider_prompt(&provider.command, &prompt).await?;
    if code != 0 {
        anyhow::bail!(
            "provider '{}' failed with exit code {}: {}",
            provider_name,
            code,
            stderr.trim()
        );
    }

    let draft = parse_generated_draft(&stdout)
        .with_context(|| format!("provider '{}' returned invalid draft output", provider_name))?;

    let template = choose_template(&templates, &draft.suggested_template);

    let spec = if draft.spec_md.trim().is_empty() {
        description.to_string()
    } else {
        draft.spec_md.trim().to_string()
    };

    let mut acceptance_criteria: Vec<String> = draft
        .acceptance_criteria
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect();
    if acceptance_criteria.is_empty() {
        acceptance_criteria.push(
            "Implementation matches the requested description and passes validation.".to_string(),
        );
    }

    let id_source = id_override
        .filter(|s| !s.trim().is_empty())
        .map(str::trim)
        .or_else(|| draft.id.as_deref().map(str::trim))
        .filter(|s| !s.is_empty())
        .unwrap_or(description);
    let card_id = ensure_unique_card_id(cards_dir, id_source);

    println!("Generated job card draft:");
    println!("provider: {}", provider_name);
    println!("template: {}", template);
    println!("id: {}", card_id);
    println!();
    println!("=== spec.md ===");
    println!("{}", spec);
    println!();
    println!("=== acceptance_criteria ===");
    for criterion in &acceptance_criteria {
        println!("- {}", criterion);
    }
    println!();

    let confirmed = if auto_confirm {
        true
    } else {
        read_confirmation()?
    };
    if !confirmed {
        println!("aborted: draft was not written");
        return Ok(());
    }

    let card_dir = create_card(cards_dir, &template, &card_id, Some(&spec))?;
    let mut meta = jobcard_core::read_meta(&card_dir)?;
    meta.acceptance_criteria = acceptance_criteria;
    write_meta(&card_dir, &meta)?;

    println!("created: {}", card_id);
    Ok(())
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

fn cmd_memory_list(root: &Path, namespace: &str) -> anyhow::Result<()> {
    let namespace = normalize_namespace(namespace);
    let store = read_memory_store(root, &namespace)?;
    if store.entries.is_empty() {
        println!("(empty)");
        return Ok(());
    }

    for (key, entry) in store.entries {
        let expires = entry
            .expires_at
            .map(|t| t.to_rfc3339())
            .unwrap_or_else(|| "never".to_string());
        println!("{}\t{}\t{}", key, entry.value, expires);
    }

    Ok(())
}

fn cmd_memory_get(root: &Path, namespace: &str, key: &str) -> anyhow::Result<()> {
    let namespace = normalize_namespace(namespace);
    let key = key.trim();
    if key.is_empty() {
        anyhow::bail!("key cannot be empty");
    }

    let store = read_memory_store(root, &namespace)?;
    let entry = store
        .entries
        .get(key)
        .with_context(|| format!("memory key not found: {}", key))?;
    println!("{}", entry.value);
    Ok(())
}

fn cmd_memory_set(
    root: &Path,
    namespace: &str,
    key: &str,
    value: &str,
    ttl_seconds: i64,
) -> anyhow::Result<()> {
    if ttl_seconds <= 0 {
        anyhow::bail!("ttl_seconds must be > 0");
    }

    let namespace = normalize_namespace(namespace);
    let key = key.trim();
    if key.is_empty() {
        anyhow::bail!("key cannot be empty");
    }

    let mut store = read_memory_store(root, &namespace)?;
    set_memory_entry(&mut store, key, value, ttl_seconds, Utc::now());
    write_memory_store(root, &namespace, &store)?;
    Ok(())
}

fn cmd_memory_delete(root: &Path, namespace: &str, key: &str) -> anyhow::Result<()> {
    let namespace = normalize_namespace(namespace);
    let key = key.trim();
    if key.is_empty() {
        anyhow::bail!("key cannot be empty");
    }

    let mut store = read_memory_store(root, &namespace)?;
    store.entries.remove(key);
    write_memory_store(root, &namespace, &store)?;
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let root = PathBuf::from(&cli.cards_dir);

    match cli.cmd {
        Command::Init => {
            ensure_cards_layout(&root)?;
            seed_default_templates(&root)?;
            seed_providers(&root)?;
            Ok(())
        }
        Command::New { template, id } => {
            create_card(&root, &template, &id, None)?;
            Ok(())
        }
        Command::Create {
            from_description,
            id,
            yes,
        } => cmd_create_from_description(&root, &from_description, id.as_deref(), yes).await,
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
        Command::Dashboard => cmd_dashboard(&root).await,
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
        } => {
            run_dispatcher(
                &root,
                &adapter,
                max_workers,
                poll_ms,
                max_retries,
                reap_ms,
                no_reap,
                once,
                validation_fail_threshold,
            )
            .await
        }
        Command::MergeGate { poll_ms, once } => run_merge_gate(&root, poll_ms, once).await,
        Command::Retry { id } => cmd_retry(&root, &id),
        Command::Kill { id } => cmd_kill(&root, &id).await,
        Command::Logs { id, follow } => cmd_logs(&root, &id, follow).await,
        Command::Inspect { id } => cmd_inspect(&root, &id),
        Command::Memory { cmd } => match cmd {
            MemoryCommand::List { namespace } => cmd_memory_list(&root, &namespace),
            MemoryCommand::Get { namespace, key } => cmd_memory_get(&root, &namespace, &key),
            MemoryCommand::Set {
                namespace,
                key,
                value,
                ttl_seconds,
            } => cmd_memory_set(&root, &namespace, &key, &value, ttl_seconds),
            MemoryCommand::Delete { namespace, key } => cmd_memory_delete(&root, &namespace, &key),
        },
        Command::Serve { port, bind, ui } => cmd_serve(&root, &bind, port, ui).await,
        Command::Worktree { action } => match action {
            WorktreeAction::List => cmd_worktree_list(&root),
            WorktreeAction::Create { id } => cmd_worktree_create(&root, &id),
            WorktreeAction::Clean { dry_run } => cmd_worktree_clean(&root, dry_run),
        },
        Command::Providers { cmd } => match cmd {
            ProvidersCommand::List => cmd_providers_list(&root),
            ProvidersCommand::Add {
                name,
                adapter,
                model,
            } => cmd_providers_add(&root, &name, &adapter, model.as_deref()),
            ProvidersCommand::Remove { name, force } => cmd_providers_remove(&root, &name, force),
            ProvidersCommand::Status => cmd_providers_status(&root),
        },
        Command::Config { action } => {
            let config_path = resolve_config_path();
            match action {
                ConfigAction::Get { key } => cmd_config_get(&config_path, &key),
                ConfigAction::Set { key, value } => cmd_config_set(&config_path, &key, &value),
            }
        }
    }
}

#[derive(Debug, Clone)]
struct ApiState {
    cards_dir: PathBuf,
}

#[derive(Debug, Serialize, ToSchema)]
struct ApiErrorBody {
    error: String,
}

type ApiErrorResponse = (StatusCode, Json<ApiErrorBody>);
type ApiResult<T> = Result<T, ApiErrorResponse>;

#[derive(Debug, Serialize, ToSchema)]
struct JobSummaryResponse {
    id: String,
    state: String,
    stage: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<String>,
    created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    finished_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure_reason: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
struct JobDetailsResponse {
    job: JobSummaryResponse,
    #[schema(value_type = Object)]
    meta: serde_json::Value,
    spec: String,
}

#[derive(Debug, Deserialize, ToSchema)]
struct CreateJobRequest {
    template: String,
    id: String,
    #[serde(default)]
    spec: Option<String>,
    #[serde(default)]
    acceptance_criteria: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
struct JobOutputResponse {
    id: String,
    state: String,
    /// Map of relative file path → UTF-8 content (output/ files and logs/).
    files: BTreeMap<String, String>,
}

#[derive(OpenApi)]
#[openapi(
    paths(
        api_list_jobs,
        api_get_job,
        api_create_job,
        api_get_job_output,
        api_retry_job,
        api_kill_job,
        api_stream_logs,
        api_openapi
    ),
    components(schemas(
        ApiErrorBody,
        JobSummaryResponse,
        JobDetailsResponse,
        JobOutputResponse,
        CreateJobRequest
    )),
    tags(
        (name = "jobs", description = "JobCard job management API"),
        (name = "meta", description = "API metadata")
    )
)]
struct ApiDoc;

#[utoipa::path(
    get,
    path = "/jobs",
    responses(
        (status = 200, description = "List all jobs", body = [JobSummaryResponse]),
        (status = 500, description = "Internal server error", body = ApiErrorBody)
    ),
    tag = "jobs"
)]
async fn api_list_jobs(State(state): State<ApiState>) -> ApiResult<Json<Vec<JobSummaryResponse>>> {
    Ok(Json(list_jobs(&state.cards_dir)))
}

#[utoipa::path(
    get,
    path = "/jobs/{id}",
    params(
        ("id" = String, Path, description = "Job id")
    ),
    responses(
        (status = 200, description = "Inspect a single job", body = JobDetailsResponse),
        (status = 404, description = "Job not found", body = ApiErrorBody),
        (status = 500, description = "Internal server error", body = ApiErrorBody)
    ),
    tag = "jobs"
)]
async fn api_get_job(
    State(state): State<ApiState>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Json<JobDetailsResponse>> {
    let details = read_job_details(&state.cards_dir, &id).map_err(map_lookup_error)?;
    Ok(Json(details))
}

#[utoipa::path(
    post,
    path = "/jobs",
    request_body = CreateJobRequest,
    responses(
        (status = 201, description = "Created a new job", body = JobDetailsResponse),
        (status = 400, description = "Invalid request", body = ApiErrorBody),
        (status = 404, description = "Template not found", body = ApiErrorBody),
        (status = 409, description = "Job already exists", body = ApiErrorBody),
        (status = 500, description = "Internal server error", body = ApiErrorBody)
    ),
    tag = "jobs"
)]
async fn api_create_job(
    State(state): State<ApiState>,
    Json(payload): Json<CreateJobRequest>,
) -> ApiResult<(StatusCode, Json<JobDetailsResponse>)> {
    let card_dir = create_card(
        &state.cards_dir,
        &payload.template,
        &payload.id,
        payload.spec.as_deref(),
    )
    .map_err(map_create_error)?;

    if !payload.acceptance_criteria.is_empty() {
        if let Ok(mut meta) = jobcard_core::read_meta(&card_dir) {
            meta.acceptance_criteria = payload.acceptance_criteria;
            let _ = write_meta(&card_dir, &meta);
        }
    }

    let details = read_job_details(&state.cards_dir, &payload.id).map_err(map_lookup_error)?;
    Ok((StatusCode::CREATED, Json(details)))
}

#[utoipa::path(
    post,
    path = "/jobs/{id}/retry",
    params(
        ("id" = String, Path, description = "Job id")
    ),
    responses(
        (status = 200, description = "Moved job back to pending", body = JobDetailsResponse),
        (status = 404, description = "Job not found", body = ApiErrorBody),
        (status = 409, description = "Job cannot be retried in current state", body = ApiErrorBody),
        (status = 500, description = "Internal server error", body = ApiErrorBody)
    ),
    tag = "jobs"
)]
async fn api_retry_job(
    State(state): State<ApiState>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Json<JobDetailsResponse>> {
    cmd_retry(&state.cards_dir, &id).map_err(map_retry_error)?;
    let details = read_job_details(&state.cards_dir, &id).map_err(map_lookup_error)?;
    Ok(Json(details))
}

#[utoipa::path(
    delete,
    path = "/jobs/{id}",
    params(
        ("id" = String, Path, description = "Job id")
    ),
    responses(
        (status = 200, description = "Killed a running job and moved it to failed", body = JobDetailsResponse),
        (status = 404, description = "Job not found", body = ApiErrorBody),
        (status = 409, description = "Job is not running", body = ApiErrorBody),
        (status = 500, description = "Internal server error", body = ApiErrorBody)
    ),
    tag = "jobs"
)]
async fn api_kill_job(
    State(state): State<ApiState>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Json<JobDetailsResponse>> {
    cmd_kill(&state.cards_dir, &id)
        .await
        .map_err(map_kill_error)?;
    let details = read_job_details(&state.cards_dir, &id).map_err(map_lookup_error)?;
    Ok(Json(details))
}

#[utoipa::path(
    get,
    path = "/jobs/{id}/logs",
    params(
        ("id" = String, Path, description = "Job id")
    ),
    responses(
        (status = 200, description = "Server-Sent Events stream of stdout/stderr logs", content_type = "text/event-stream"),
        (status = 404, description = "Job not found", body = ApiErrorBody)
    ),
    tag = "jobs"
)]
async fn api_stream_logs(
    State(state): State<ApiState>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<impl IntoResponse> {
    if find_card(&state.cards_dir, &id).is_none() {
        return Err(api_error(
            StatusCode::NOT_FOUND,
            format!("card not found: {}", id),
        ));
    }

    let cards_dir = state.cards_dir.clone();
    let stream_id = id.clone();
    let stream = stream! {
        let mut stdout_pos: u64 = 0;
        let mut stderr_pos: u64 = 0;
        let mut post_exit_idle_rounds: u8 = 0;

        loop {
            let Some(card_dir) = find_card(&cards_dir, &stream_id) else {
                yield Ok::<Event, Infallible>(Event::default().event("end").data("card not found"));
                break;
            };

            let stdout_path = card_dir.join("logs").join("stdout.log");
            let stderr_path = card_dir.join("logs").join("stderr.log");

            let mut emitted = false;

            if let Ok(Some(chunk)) = read_new_log_chunk(&stdout_path, &mut stdout_pos) {
                emitted = true;
                yield Ok(Event::default().event("stdout").data(chunk));
            }

            if let Ok(Some(chunk)) = read_new_log_chunk(&stderr_path, &mut stderr_pos) {
                emitted = true;
                yield Ok(Event::default().event("stderr").data(chunk));
            }

            let is_running = card_dir
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                == Some("running");

            if is_running || emitted {
                post_exit_idle_rounds = 0;
            } else {
                post_exit_idle_rounds = post_exit_idle_rounds.saturating_add(1);
                if post_exit_idle_rounds >= 2 {
                    yield Ok(Event::default().event("end").data("complete"));
                    break;
                }
            }

            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    };

    Ok(Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(10))
            .text("keep-alive"),
    ))
}

#[utoipa::path(
    get,
    path = "/jobs/{id}/output",
    params(("id" = String, Path, description = "Job id")),
    responses(
        (status = 200, description = "Output files and logs", body = JobOutputResponse),
        (status = 404, description = "Job not found", body = ApiErrorBody),
        (status = 500, description = "Internal error", body = ApiErrorBody)
    ),
    tag = "jobs"
)]
async fn api_get_job_output(
    State(state): State<ApiState>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Json<JobOutputResponse>> {
    let card_dir = find_card(&state.cards_dir, &id)
        .with_context(|| format!("card not found: {}", id))
        .map_err(map_lookup_error)?;

    let job_state = card_dir
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    let mut files: BTreeMap<String, String> = BTreeMap::new();

    let output_dir = card_dir.join("output");
    if output_dir.exists() {
        for entry in fs::read_dir(&output_dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_file() {
                if let (Some(name), Ok(content)) = (
                    path.file_name()
                        .and_then(|s| s.to_str())
                        .map(str::to_string),
                    fs::read_to_string(&path),
                ) {
                    files.insert(name, content);
                }
            }
        }
    }

    for log_name in ["stdout.log", "stderr.log", "validation.log"] {
        let log_path = card_dir.join("logs").join(log_name);
        if log_path.exists() {
            if let Ok(content) = fs::read_to_string(&log_path) {
                files.insert(format!("logs/{}", log_name), content);
            }
        }
    }

    Ok(Json(JobOutputResponse {
        id,
        state: job_state,
        files,
    }))
}

#[utoipa::path(
    get,
    path = "/openapi.json",
    responses(
        (status = 200, description = "Generated OpenAPI specification")
    ),
    tag = "meta"
)]
async fn api_openapi() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

#[derive(Debug, Deserialize, Default)]
struct UiEventsQuery {
    #[serde(default)]
    once: bool,
}

#[derive(Debug, Serialize)]
struct UiJobsEventPayload {
    generated_at: String,
    jobs: Vec<JobSummaryResponse>,
}

async fn ui_dashboard(State(state): State<ApiState>) -> Html<String> {
    let jobs = list_jobs(&state.cards_dir);
    let generated_at = Utc::now().to_rfc3339();
    let pending_count = jobs.iter().filter(|job| job.state == "pending").count();
    let running_count = jobs.iter().filter(|job| job.state == "running").count();
    let done_count = jobs.iter().filter(|job| job.state == "done").count();
    let failed_count = jobs.iter().filter(|job| job.state == "failed").count();
    let merged_count = jobs.iter().filter(|job| job.state == "merged").count();
    let rows = render_ui_job_rows(&jobs);

    let body = format!(
        r#"<section class="page-header">
<h1>Job Dashboard</h1>
<p class="muted">Last update: <span id="updated-at">{generated_at}</span> · <span id="live-status">live stream connecting...</span></p>
<div class="chip-row">
<span class="chip">pending {pending_count}</span>
<span class="chip">running {running_count}</span>
<span class="chip">done {done_count}</span>
<span class="chip">failed {failed_count}</span>
<span class="chip">merged {merged_count}</span>
</div>
</section>
<noscript><p class="notice">JavaScript is disabled. The page still works server-side; refresh manually for updates.</p></noscript>
<table>
<thead>
<tr>
<th>Job</th>
<th>State</th>
<th>Stage</th>
<th>Provider</th>
<th>Created</th>
<th>Retries</th>
<th>Failure</th>
</tr>
</thead>
<tbody id="jobs-body">
{rows}
</tbody>
</table>"#,
    );

    let script = r#"<script>
(() => {
  const body = document.getElementById("jobs-body");
  const updatedAt = document.getElementById("updated-at");
  const liveStatus = document.getElementById("live-status");

  function stateClass(state) {
    if (state === "running") return "state-running";
    if (state === "pending") return "state-pending";
    if (state === "done") return "state-done";
    if (state === "merged") return "state-merged";
    if (state === "failed") return "state-failed";
    return "state-unknown";
  }

  function toText(value) {
    if (value === null || value === undefined || value === "") return "-";
    return String(value);
  }

  function appendCell(row, valueOrNode) {
    const cell = document.createElement("td");
    if (valueOrNode instanceof Node) {
      cell.appendChild(valueOrNode);
    } else {
      cell.textContent = toText(valueOrNode);
    }
    row.appendChild(cell);
    return cell;
  }

  function renderRows(jobs) {
    body.replaceChildren();
    if (!Array.isArray(jobs) || jobs.length === 0) {
      const row = document.createElement("tr");
      const cell = document.createElement("td");
      cell.colSpan = 7;
      cell.className = "empty";
      cell.textContent = "No jobs found.";
      row.appendChild(cell);
      body.appendChild(row);
      return;
    }

    for (const job of jobs) {
      const row = document.createElement("tr");
      const link = document.createElement("a");
      link.href = `/ui/jobs/${encodeURIComponent(job.id)}`;
      link.textContent = toText(job.id);
      appendCell(row, link);

      const badge = document.createElement("span");
      badge.className = `state-badge ${stateClass(job.state)}`;
      badge.textContent = toText(job.state);
      appendCell(row, badge);

      appendCell(row, job.stage);
      appendCell(row, job.provider || "-");
      appendCell(row, job.created_at);
      appendCell(row, job.retry_count ?? "-");
      appendCell(row, job.failure_reason || "-");
      body.appendChild(row);
    }
  }

  const stream = new EventSource("/ui/events");
  stream.addEventListener("jobs", (event) => {
    const payload = JSON.parse(event.data);
    renderRows(payload.jobs || []);
    updatedAt.textContent = payload.generated_at || "";
    const total = Array.isArray(payload.jobs) ? payload.jobs.length : 0;
    liveStatus.textContent = `live (${total} jobs)`;
  });
  stream.onerror = () => {
    liveStatus.textContent = "stream reconnecting...";
  };
})();
</script>"#;

    Html(render_ui_shell(
        "jc Web UI",
        &body,
        r#"<noscript><meta http-equiv="refresh" content="5"></noscript>"#,
        script,
    ))
}

async fn ui_stream_jobs(
    State(state): State<ApiState>,
    Query(query): Query<UiEventsQuery>,
) -> impl IntoResponse {
    let cards_dir = state.cards_dir.clone();
    let stream_once = query.once;
    let stream = stream! {
        loop {
            let payload = UiJobsEventPayload {
                generated_at: Utc::now().to_rfc3339(),
                jobs: list_jobs(&cards_dir),
            };
            match serde_json::to_string(&payload) {
                Ok(data) => yield Ok::<Event, Infallible>(Event::default().event("jobs").data(data)),
                Err(err) => yield Ok::<Event, Infallible>(Event::default().event("error").data(err.to_string())),
            }

            if stream_once {
                break;
            }

            tokio::time::sleep(Duration::from_millis(UI_SSE_POLL_MS)).await;
        }
    };

    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(10))
            .text("keep-alive"),
    )
}

async fn ui_job_details(
    State(state): State<ApiState>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Html<String>> {
    let card_dir = find_card(&state.cards_dir, &id)
        .with_context(|| format!("card not found: {}", id))
        .map_err(map_lookup_error)?;
    let details = read_job_details(&state.cards_dir, &id).map_err(map_lookup_error)?;
    let meta = jobcard_core::read_meta(&card_dir).map_err(map_lookup_error)?;
    let logs = collect_text_files(&card_dir.join("logs"), "logs");
    let output_files = collect_text_files(&card_dir.join("output"), "output");
    let audit_lines = build_audit_trail(&meta, &details.job.state);
    let meta_json =
        serde_json::to_string_pretty(&details.meta).unwrap_or_else(|_| "{}".to_string());

    let body = format!(
        r#"<section class="page-header">
<h1>Job: {id}</h1>
<p class="muted">state: <span class="state-badge {state_class}">{state}</span> · stage: {stage} · provider: {provider}</p>
</section>
<section>
<h2>Spec</h2>
<pre>{spec}</pre>
</section>
<section>
<h2>Logs</h2>
{logs_section}
</section>
<section>
<h2>Output</h2>
{output_section}
</section>
<section>
<h2>Audit Trail</h2>
<pre>{audit}</pre>
<details>
<summary>raw meta.json</summary>
<pre>{meta_json}</pre>
</details>
</section>"#,
        id = escape_html(&details.job.id),
        state = escape_html(&details.job.state),
        stage = escape_html(&details.job.stage),
        provider = escape_html(details.job.provider.as_deref().unwrap_or("-")),
        state_class = ui_state_class(&details.job.state),
        spec = escape_html(&details.spec),
        logs_section = render_ui_file_sections(&logs, "No logs found."),
        output_section = render_ui_file_sections(&output_files, "No output files found."),
        audit = escape_html(&audit_lines.join("\n")),
        meta_json = escape_html(&meta_json),
    );

    Ok(Html(render_ui_shell("jc Job Details", &body, "", "")))
}

async fn ui_providers(State(state): State<ApiState>) -> Html<String> {
    let providers = read_providers(&state.cards_dir).unwrap_or_default();
    let mut rows = String::new();
    let now_epoch = Utc::now().timestamp();

    for (name, provider) in &providers.providers {
        let (status, cooldown_until) = provider_status(provider.cooldown_until_epoch_s, now_epoch);
        let _ = write!(
            rows,
            "<tr><td>{}</td><td><code>{}</code></td><td>{}</td><td>{}</td><td>{}</td></tr>",
            escape_html(name),
            escape_html(&provider.command),
            provider.rate_limit_exit,
            escape_html(&status),
            escape_html(&cooldown_until)
        );
    }

    if rows.is_empty() {
        rows.push_str(r#"<tr><td colspan="5" class="empty">No providers configured.</td></tr>"#);
    }

    let default_provider = providers
        .default_provider
        .unwrap_or_else(|| "-".to_string());
    let body = format!(
        r#"<section class="page-header">
<h1>Providers</h1>
<p class="muted">default provider: <code>{}</code></p>
</section>
<table>
<thead>
<tr>
<th>Name</th>
<th>Command</th>
<th>Rate Limit Exit</th>
<th>Status</th>
<th>Cooldown Until (UTC)</th>
</tr>
</thead>
<tbody>
{}
</tbody>
</table>"#,
        escape_html(&default_provider),
        rows
    );

    Html(render_ui_shell("jc Providers", &body, "", ""))
}

fn render_ui_shell(title: &str, body: &str, head_extra: &str, script_extra: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
{head_extra}
<style>
:root {{
  --bg: #f2f4f7;
  --surface: #ffffff;
  --ink: #0f172a;
  --muted: #5b6573;
  --line: #d7dde5;
  --accent: #0f766e;
  --warn: #b45309;
  --ok: #166534;
  --bad: #991b1b;
}}
* {{ box-sizing: border-box; }}
body {{
  margin: 0;
  background: linear-gradient(160deg, #f9fbfd 0%, #eef2f6 100%);
  color: var(--ink);
  font-family: "IBM Plex Sans", "Segoe UI", sans-serif;
}}
main {{
  max-width: 1100px;
  margin: 0 auto;
  padding: 1rem;
}}
nav {{
  background: var(--surface);
  border-bottom: 1px solid var(--line);
}}
nav .inner {{
  max-width: 1100px;
  margin: 0 auto;
  padding: 0.75rem 1rem;
  display: flex;
  gap: 1rem;
  align-items: center;
}}
nav a {{
  color: var(--accent);
  text-decoration: none;
  font-weight: 600;
}}
section {{
  background: var(--surface);
  border: 1px solid var(--line);
  border-radius: 12px;
  padding: 1rem;
  margin-bottom: 1rem;
}}
.page-header {{
  background: linear-gradient(135deg, #ffffff 0%, #eef7f5 100%);
}}
.chip-row {{
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem;
  margin-top: 0.5rem;
}}
.chip {{
  padding: 0.2rem 0.6rem;
  border-radius: 999px;
  border: 1px solid var(--line);
  background: #f8fafc;
  font-size: 0.9rem;
}}
.muted {{
  color: var(--muted);
  margin: 0;
}}
.notice {{
  margin: 0.5rem 0 0;
  color: var(--warn);
}}
table {{
  width: 100%;
  border-collapse: collapse;
  background: var(--surface);
  border: 1px solid var(--line);
  border-radius: 12px;
  overflow: hidden;
}}
th, td {{
  border-bottom: 1px solid var(--line);
  padding: 0.6rem;
  text-align: left;
  vertical-align: top;
  font-size: 0.95rem;
}}
th {{
  background: #f5f9fc;
}}
tr:last-child td {{
  border-bottom: 0;
}}
.empty {{
  color: var(--muted);
  text-align: center;
  padding: 1rem;
}}
pre {{
  margin: 0;
  padding: 0.75rem;
  background: #0b1020;
  color: #e2e8f0;
  border-radius: 8px;
  overflow: auto;
  white-space: pre-wrap;
  word-break: break-word;
}}
details {{
  border: 1px solid var(--line);
  border-radius: 8px;
  padding: 0.5rem;
  margin-bottom: 0.5rem;
}}
details > summary {{
  cursor: pointer;
  font-weight: 600;
  margin-bottom: 0.5rem;
}}
.state-badge {{
  display: inline-block;
  padding: 0.15rem 0.45rem;
  border-radius: 999px;
  border: 1px solid transparent;
  font-size: 0.85rem;
  font-weight: 600;
}}
.state-running {{ color: #075985; background: #e0f2fe; border-color: #7dd3fc; }}
.state-pending {{ color: #92400e; background: #fef3c7; border-color: #fcd34d; }}
.state-done {{ color: var(--ok); background: #dcfce7; border-color: #86efac; }}
.state-merged {{ color: #6d28d9; background: #ede9fe; border-color: #c4b5fd; }}
.state-failed {{ color: var(--bad); background: #fee2e2; border-color: #fca5a5; }}
.state-unknown {{ color: #334155; background: #e2e8f0; border-color: #cbd5e1; }}
@media (max-width: 800px) {{
  th, td {{ font-size: 0.85rem; padding: 0.45rem; }}
  nav .inner {{ flex-wrap: wrap; }}
}}
</style>
</head>
<body>
<nav>
  <div class="inner">
    <a href="/ui">Dashboard</a>
    <a href="/ui/providers">Providers</a>
    <a href="/openapi.json">OpenAPI</a>
  </div>
</nav>
<main>
{body}
</main>
{script_extra}
</body>
</html>"#,
        title = escape_html(title),
        body = body,
        head_extra = head_extra,
        script_extra = script_extra,
    )
}

fn render_ui_job_rows(jobs: &[JobSummaryResponse]) -> String {
    let mut rows = String::new();
    for job in jobs {
        let id = escape_html(&job.id);
        let id_href = percent_encode_path_segment(&job.id);
        let provider = escape_html(job.provider.as_deref().unwrap_or("-"));
        let failure_reason = escape_html(job.failure_reason.as_deref().unwrap_or("-"));
        let retry_count = job
            .retry_count
            .map(|count| count.to_string())
            .unwrap_or_else(|| "-".to_string());
        let _ = write!(
            rows,
            "<tr>\
<td><a href=\"/ui/jobs/{id_href}\">{id}</a></td>\
<td><span class=\"state-badge {state_class}\">{state}</span></td>\
<td>{stage}</td>\
<td>{provider}</td>\
<td>{created}</td>\
<td>{retry_count}</td>\
<td>{failure_reason}</td>\
</tr>",
            state_class = ui_state_class(&job.state),
            state = escape_html(&job.state),
            stage = escape_html(&job.stage),
            created = escape_html(&job.created_at),
            retry_count = escape_html(&retry_count),
        );
    }

    if rows.is_empty() {
        rows.push_str(r#"<tr><td colspan="7" class="empty">No jobs found.</td></tr>"#);
    }
    rows
}

fn render_ui_file_sections(files: &BTreeMap<String, String>, empty_message: &str) -> String {
    if files.is_empty() {
        return format!(r#"<p class="muted">{}</p>"#, escape_html(empty_message));
    }

    let mut out = String::new();
    for (name, content) in files {
        let _ = write!(
            out,
            "<details><summary>{}</summary><pre>{}</pre></details>",
            escape_html(name),
            escape_html(content)
        );
    }
    out
}

fn collect_text_files(dir: &Path, prefix: &str) -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    if !dir.exists() {
        return files;
    }

    for entry in WalkDir::new(dir).min_depth(1).into_iter().flatten() {
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let rel = match path.strip_prefix(dir) {
            Ok(rel) => rel,
            Err(_) => continue,
        };
        let key = format!("{}/{}", prefix, rel.to_string_lossy());
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };

        let (shown, truncated) = if bytes.len() > UI_MAX_FILE_PREVIEW_BYTES {
            (&bytes[..UI_MAX_FILE_PREVIEW_BYTES], true)
        } else {
            (&bytes[..], false)
        };
        let mut content = String::from_utf8_lossy(shown).to_string();
        if truncated {
            let _ = write!(
                content,
                "\n\n[truncated: showing first {} bytes of {}]",
                UI_MAX_FILE_PREVIEW_BYTES,
                bytes.len()
            );
        }
        files.insert(key, content);
    }
    files
}

fn build_audit_trail(meta: &Meta, state: &str) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(format!("created: {}", meta.created.to_rfc3339()));
    lines.push(format!("current_state: {}", state));
    lines.push(format!("current_stage: {}", meta.stage));

    for (stage, record) in &meta.stages {
        let started = record
            .started
            .as_ref()
            .map(DateTime::<Utc>::to_rfc3339)
            .unwrap_or_else(|| "-".to_string());
        let duration = record
            .duration_s
            .map(|seconds| format!("{}s", seconds))
            .unwrap_or_else(|| "-".to_string());
        lines.push(format!(
            "stage={} status={} provider={} agent={} started={} duration={}",
            stage,
            stage_status_label(&record.status),
            record.provider.as_deref().unwrap_or("-"),
            record.agent.as_deref().unwrap_or("-"),
            started,
            duration
        ));
    }

    if let Some(retry_count) = meta.retry_count {
        lines.push(format!("retry_count: {}", retry_count));
    }
    if let Some(reason) = &meta.failure_reason {
        lines.push(format!("failure_reason: {}", reason));
    }
    if let Some(summary) = &meta.validation_summary {
        lines.push(format!(
            "validation: badge={} valid={}/{} invalid={} alerts={} critical={} health={:?}",
            summary.badge(),
            summary.valid,
            summary.total,
            summary.invalid,
            summary.alert_count,
            summary.critical_alerts,
            summary.health
        ));
    }
    if !meta.acceptance_criteria.is_empty() {
        lines.push("acceptance_criteria:".to_string());
        for criterion in &meta.acceptance_criteria {
            lines.push(format!("  - {}", criterion));
        }
    }

    lines
}

fn stage_status_label(status: &jobcard_core::StageStatus) -> &'static str {
    match status {
        jobcard_core::StageStatus::Pending => "pending",
        jobcard_core::StageStatus::Running => "running",
        jobcard_core::StageStatus::Done => "done",
        jobcard_core::StageStatus::Blocked => "blocked",
        jobcard_core::StageStatus::Failed => "failed",
    }
}

fn provider_status(cooldown_until_epoch_s: Option<i64>, now_epoch: i64) -> (String, String) {
    match cooldown_until_epoch_s {
        Some(until) if until > now_epoch => {
            let remaining = until.saturating_sub(now_epoch);
            (
                format!("cooldown ({}s)", remaining),
                format_epoch_utc(until),
            )
        }
        Some(until) => ("ready".to_string(), format_epoch_utc(until)),
        None => ("ready".to_string(), "-".to_string()),
    }
}

fn format_epoch_utc(epoch_s: i64) -> String {
    DateTime::<Utc>::from_timestamp(epoch_s, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| epoch_s.to_string())
}

fn ui_state_class(state: &str) -> &'static str {
    match state {
        "running" => "state-running",
        "pending" => "state-pending",
        "done" => "state-done",
        "merged" => "state-merged",
        "failed" => "state-failed",
        _ => "state-unknown",
    }
}

fn escape_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

fn percent_encode_path_segment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{:02X}", byte);
        }
    }
    out
}

async fn cmd_serve(root: &Path, bind: &str, port: u16, ui: bool) -> anyhow::Result<()> {
    ensure_cards_layout(root)?;
    seed_default_templates(root)?;
    seed_providers(root)?;

    if !is_loopback_bind(bind) {
        eprintln!(
            "WARNING: --bind {} exposes unauthenticated job control endpoints to remote clients.",
            bind
        );
    }

    let addr = resolve_bind_addr(bind, port)?;
    let state = ApiState {
        cards_dir: root.to_path_buf(),
    };

    let mut app = Router::new()
        .route("/jobs", get(api_list_jobs).post(api_create_job))
        .route("/jobs/:id", get(api_get_job).delete(api_kill_job))
        .route("/jobs/:id/retry", post(api_retry_job))
        .route("/jobs/:id/output", get(api_get_job_output))
        .route("/jobs/:id/logs", get(api_stream_logs))
        .route("/openapi.json", get(api_openapi));

    if ui {
        app = app
            .route("/ui", get(ui_dashboard))
            .route("/ui/events", get(ui_stream_jobs))
            .route("/ui/jobs/:id", get(ui_job_details))
            .route("/ui/providers", get(ui_providers));
    }

    let app = app.layer(CorsLayer::permissive()).with_state(state);

    let listener = TcpListener::bind(addr).await?;
    println!("REST API listening on http://{}:{}/", bind, port);
    if ui {
        println!("Web UI listening on http://{}:{}/ui", bind, port);
    }
    axum::serve(listener, app).await?;
    Ok(())
}

fn resolve_bind_addr(bind: &str, port: u16) -> anyhow::Result<SocketAddr> {
    let mut addrs = (bind, port)
        .to_socket_addrs()
        .with_context(|| format!("failed to resolve bind address {}:{}", bind, port))?;
    addrs
        .next()
        .with_context(|| format!("no bind address resolved for {}:{}", bind, port))
}

fn is_loopback_bind(bind: &str) -> bool {
    if bind.eq_ignore_ascii_case("localhost") {
        return true;
    }
    bind.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

fn api_error(status: StatusCode, error: impl Into<String>) -> ApiErrorResponse {
    (
        status,
        Json(ApiErrorBody {
            error: error.into(),
        }),
    )
}

fn map_lookup_error(err: anyhow::Error) -> ApiErrorResponse {
    let msg = err.to_string();
    if msg.contains("card not found") {
        api_error(StatusCode::NOT_FOUND, msg)
    } else {
        api_error(StatusCode::INTERNAL_SERVER_ERROR, msg)
    }
}

fn map_create_error(err: anyhow::Error) -> ApiErrorResponse {
    let msg = err.to_string();
    if msg.contains("card already exists") {
        api_error(StatusCode::CONFLICT, msg)
    } else if msg.contains("template not found") {
        api_error(StatusCode::NOT_FOUND, msg)
    } else if msg.contains("cannot be empty") || msg.contains("path separators") {
        api_error(StatusCode::BAD_REQUEST, msg)
    } else {
        api_error(StatusCode::INTERNAL_SERVER_ERROR, msg)
    }
}

fn map_retry_error(err: anyhow::Error) -> ApiErrorResponse {
    let msg = err.to_string();
    if msg.contains("card not found") {
        api_error(StatusCode::NOT_FOUND, msg)
    } else if msg.contains("already pending") || msg.contains("currently running") {
        api_error(StatusCode::CONFLICT, msg)
    } else {
        api_error(StatusCode::BAD_REQUEST, msg)
    }
}

fn map_kill_error(err: anyhow::Error) -> ApiErrorResponse {
    let msg = err.to_string();
    if msg.contains("card not found") || msg.contains("no PID found") {
        api_error(StatusCode::NOT_FOUND, msg)
    } else if msg.contains("is not running") {
        api_error(StatusCode::CONFLICT, msg)
    } else {
        api_error(StatusCode::INTERNAL_SERVER_ERROR, msg)
    }
}

fn list_jobs(root: &Path) -> Vec<JobSummaryResponse> {
    let mut jobs = Vec::new();
    for state in ["pending", "running", "done", "merged", "failed"] {
        let state_dir = root.join(state);
        let entries = match fs::read_dir(state_dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let card_dir = entry.path();
            if !card_dir.is_dir() {
                continue;
            }
            if card_dir.extension().and_then(|s| s.to_str()) != Some("jobcard") {
                continue;
            }
            let Ok(meta) = jobcard_core::read_meta(&card_dir) else {
                continue;
            };
            jobs.push(job_summary_from_meta(state, &meta));
        }
    }
    jobs.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    jobs
}

fn read_job_details(root: &Path, id: &str) -> anyhow::Result<JobDetailsResponse> {
    let card_dir = find_card(root, id).with_context(|| format!("card not found: {}", id))?;
    let state = card_dir
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();
    let meta = jobcard_core::read_meta(&card_dir)?;
    let meta_json = serde_json::to_value(&meta)?;
    let spec = fs::read_to_string(card_dir.join("spec.md")).unwrap_or_default();

    Ok(JobDetailsResponse {
        job: job_summary_from_meta(&state, &meta),
        meta: meta_json,
        spec,
    })
}

fn job_summary_from_meta(state: &str, meta: &Meta) -> JobSummaryResponse {
    let stage_record = meta.stages.get(&meta.stage);
    let started_dt = stage_record.and_then(|rec| rec.started.as_ref().cloned());
    let duration_secs = stage_record.and_then(|rec| rec.duration_s);
    let finished_at = match (started_dt, duration_secs) {
        (Some(started), Some(duration)) => i64::try_from(duration)
            .ok()
            .map(|secs| (started + ChronoDuration::seconds(secs)).to_rfc3339()),
        _ => None,
    };

    JobSummaryResponse {
        id: meta.id.clone(),
        state: state.to_string(),
        stage: meta.stage.clone(),
        provider: stage_record
            .and_then(|rec| rec.provider.clone())
            .or_else(|| meta.provider_chain.first().cloned()),
        created_at: meta.created.to_rfc3339(),
        started_at: started_dt.map(|dt| dt.to_rfc3339()),
        finished_at,
        retry_count: meta.retry_count,
        failure_reason: meta.failure_reason.clone(),
    }
}

fn read_new_log_chunk(path: &Path, position: &mut u64) -> anyhow::Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }

    let mut file = fs::File::open(path)?;
    let len = file.metadata()?.len();
    if *position > len {
        *position = 0;
    }

    file.seek(SeekFrom::Start(*position))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    *position += buf.len() as u64;

    if buf.is_empty() {
        Ok(None)
    } else {
        Ok(Some(String::from_utf8_lossy(&buf).to_string()))
    }
}

const DASHBOARD_MIN_WIDTH: u16 = 80;
const DASHBOARD_MIN_HEIGHT: u16 = 20;
const DASHBOARD_TICK_MS: u64 = 250;
const DASHBOARD_LOG_TAIL_BYTES: u64 = 8192;
const DASHBOARD_MAX_LOG_LINES: usize = 4000;
const DASHBOARD_MAX_LAST_LOG_CHARS: usize = 120;

#[derive(Debug, Clone)]
struct DashboardJob {
    id: String,
    state: String,
    provider: String,
    elapsed: String,
    last_log_line: String,
    card_dir: PathBuf,
    created: DateTime<Utc>,
}

#[derive(Debug, Clone, Default)]
enum DashboardMode {
    #[default]
    Table,
    Logs(LogViewState),
}

#[derive(Debug, Clone)]
struct LogViewState {
    job_id: String,
    card_dir: PathBuf,
    stdout_pos: u64,
    stderr_pos: u64,
    stdout_partial: String,
    stderr_partial: String,
    lines: Vec<String>,
    scroll: usize,
    follow: bool,
}

impl LogViewState {
    fn new(job: &DashboardJob) -> Self {
        Self {
            job_id: job.id.clone(),
            card_dir: job.card_dir.clone(),
            stdout_pos: 0,
            stderr_pos: 0,
            stdout_partial: String::new(),
            stderr_partial: String::new(),
            lines: Vec::new(),
            scroll: 0,
            follow: true,
        }
    }

    fn refresh(&mut self, root: &Path) -> anyhow::Result<()> {
        if let Some(card_dir) = find_card(root, &self.job_id) {
            self.card_dir = card_dir;
        }

        let stdout_path = self.card_dir.join("logs").join("stdout.log");
        let stderr_path = self.card_dir.join("logs").join("stderr.log");

        if let Some(chunk) = read_new_log_chunk(&stdout_path, &mut self.stdout_pos)? {
            Self::append_chunk_lines(
                &mut self.lines,
                &mut self.stdout_partial,
                "stdout",
                chunk.as_str(),
            );
        }
        if let Some(chunk) = read_new_log_chunk(&stderr_path, &mut self.stderr_pos)? {
            Self::append_chunk_lines(
                &mut self.lines,
                &mut self.stderr_partial,
                "stderr",
                chunk.as_str(),
            );
        }

        self.trim_line_buffer();
        Ok(())
    }

    fn append_chunk_lines(
        lines: &mut Vec<String>,
        partial: &mut String,
        source: &str,
        chunk: &str,
    ) {
        let mut combined = String::new();
        combined.push_str(partial);
        combined.push_str(chunk);

        let ends_with_newline = combined.ends_with('\n');
        let mut pieces: Vec<&str> = combined.split('\n').collect();

        partial.clear();
        if !ends_with_newline {
            if let Some(last) = pieces.pop() {
                partial.push_str(last);
            }
        } else if matches!(pieces.last(), Some(last) if last.is_empty()) {
            pieces.pop();
        }

        for line in pieces {
            lines.push(format!("[{}] {}", source, line.trim_end_matches('\r')));
        }
    }

    fn trim_line_buffer(&mut self) {
        if self.lines.len() <= DASHBOARD_MAX_LOG_LINES {
            return;
        }
        let to_drop = self.lines.len() - DASHBOARD_MAX_LOG_LINES;
        self.lines.drain(0..to_drop);
        if !self.follow {
            self.scroll = self.scroll.saturating_sub(to_drop);
        }
    }

    fn max_scroll(&self, viewport_height: usize) -> usize {
        self.lines.len().saturating_sub(viewport_height.max(1))
    }

    fn effective_scroll(&self, viewport_height: usize) -> usize {
        let max_scroll = self.max_scroll(viewport_height);
        if self.follow {
            max_scroll
        } else {
            min(self.scroll, max_scroll)
        }
    }

    fn scroll_up(&mut self, delta: usize, viewport_height: usize) {
        let current = self.effective_scroll(viewport_height);
        self.follow = false;
        self.scroll = current.saturating_sub(delta);
    }

    fn scroll_down(&mut self, delta: usize, viewport_height: usize) {
        let max_scroll = self.max_scroll(viewport_height);
        let current = self.effective_scroll(viewport_height);
        let next = min(current.saturating_add(delta), max_scroll);
        self.scroll = next;
        self.follow = next >= max_scroll;
    }

    fn scroll_home(&mut self) {
        self.follow = false;
        self.scroll = 0;
    }

    fn scroll_end(&mut self, viewport_height: usize) {
        self.follow = true;
        self.scroll = self.max_scroll(viewport_height);
    }

    fn render_text(&self) -> String {
        if self.lines.is_empty() {
            "(no log output yet)".to_string()
        } else {
            self.lines.join("\n")
        }
    }
}

#[derive(Debug, Default)]
struct DashboardApp {
    jobs: Vec<DashboardJob>,
    selected: usize,
    mode: DashboardMode,
    status_line: String,
    log_view_height: usize,
}

struct DashboardTerminalGuard;

impl Drop for DashboardTerminalGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let mut stdout = std::io::stdout();
        let _ = execute!(stdout, LeaveAlternateScreen);
    }
}

async fn cmd_dashboard(root: &Path) -> anyhow::Result<()> {
    ensure_cards_layout(root)?;
    if dashboard_should_fallback() {
        return print_status_summary(root);
    }

    let (watch_tx, watch_rx) = mpsc::channel::<notify::Result<notify::Event>>();
    let mut _watcher = RecommendedWatcher::new(
        move |event| {
            let _ = watch_tx.send(event);
        },
        notify::Config::default(),
    )
    .context("failed to initialize watcher")?;
    _watcher
        .watch(root, RecursiveMode::Recursive)
        .context("failed to watch cards directory")?;

    let mut app = DashboardApp {
        log_view_height: 10,
        ..Default::default()
    };
    refresh_dashboard(root, &mut app)?;

    let mut fallback_to_status = false;
    {
        terminal::enable_raw_mode().context("failed to enable raw mode")?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen).context("failed to enter alternate screen")?;
        let _guard = DashboardTerminalGuard;

        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend).context("failed to initialize terminal")?;
        terminal.clear()?;

        let tick = Duration::from_millis(DASHBOARD_TICK_MS);
        let mut last_tick = Instant::now();
        let mut needs_refresh = true;

        loop {
            while let Ok(event_result) = watch_rx.try_recv() {
                match event_result {
                    Ok(_) => needs_refresh = true,
                    Err(err) => app.status_line = format!("watch error: {}", err),
                }
            }

            if last_tick.elapsed() >= tick {
                needs_refresh = true;
                last_tick = Instant::now();
            }

            if needs_refresh {
                if let Err(err) = refresh_dashboard(root, &mut app) {
                    app.status_line = format!("refresh error: {}", err);
                }
                needs_refresh = false;
            }

            terminal.draw(|frame| draw_dashboard(frame, &mut app))?;

            if event::poll(Duration::from_millis(100))? {
                match event::read()? {
                    CEvent::Key(key) if key.kind == KeyEventKind::Press => {
                        if handle_dashboard_key(root, &mut app, key.code).await? {
                            break;
                        }
                        needs_refresh = true;
                    }
                    CEvent::Resize(width, height) => {
                        if width < DASHBOARD_MIN_WIDTH || height < DASHBOARD_MIN_HEIGHT {
                            fallback_to_status = true;
                            break;
                        }
                        needs_refresh = true;
                    }
                    _ => {}
                }
            }
        }
    }

    if fallback_to_status {
        return print_status_summary(root);
    }

    Ok(())
}

fn dashboard_should_fallback() -> bool {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return true;
    }

    match terminal::size() {
        Ok((width, height)) => width < DASHBOARD_MIN_WIDTH || height < DASHBOARD_MIN_HEIGHT,
        Err(_) => true,
    }
}

fn refresh_dashboard(root: &Path, app: &mut DashboardApp) -> anyhow::Result<()> {
    let selected_id = app.jobs.get(app.selected).map(|job| job.id.clone());
    app.jobs = collect_dashboard_jobs(root);

    if app.jobs.is_empty() {
        app.selected = 0;
    } else if let Some(id) = selected_id {
        if let Some(index) = app.jobs.iter().position(|job| job.id == id) {
            app.selected = index;
        } else {
            app.selected = min(app.selected, app.jobs.len().saturating_sub(1));
        }
    } else {
        app.selected = min(app.selected, app.jobs.len().saturating_sub(1));
    }

    if let DashboardMode::Logs(log_view) = &mut app.mode {
        log_view.refresh(root)?;
    }

    Ok(())
}

fn collect_dashboard_jobs(root: &Path) -> Vec<DashboardJob> {
    let now = Utc::now();
    let mut jobs = Vec::new();

    for state in ["running", "pending", "failed", "done", "merged"] {
        let state_dir = root.join(state);
        let entries = match fs::read_dir(state_dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let card_dir = entry.path();
            if !card_dir.is_dir() {
                continue;
            }
            if card_dir.extension().and_then(|s| s.to_str()) != Some("jobcard") {
                continue;
            }

            let Ok(meta) = jobcard_core::read_meta(&card_dir) else {
                continue;
            };

            let stage_record = meta.stages.get(&meta.stage);
            let provider = stage_record
                .and_then(|record| record.provider.clone())
                .or_else(|| meta.provider_chain.first().cloned())
                .unwrap_or_else(|| "-".to_string());

            jobs.push(DashboardJob {
                id: meta.id.clone(),
                state: state.to_string(),
                provider,
                elapsed: format_elapsed_duration(dashboard_elapsed_seconds(state, &meta, now)),
                last_log_line: read_dashboard_last_log_line(&card_dir),
                card_dir,
                created: meta.created,
            });
        }
    }

    jobs.sort_by(|a, b| {
        dashboard_state_rank(a.state.as_str())
            .cmp(&dashboard_state_rank(b.state.as_str()))
            .then_with(|| b.created.cmp(&a.created))
            .then_with(|| a.id.cmp(&b.id))
    });
    jobs
}

fn dashboard_elapsed_seconds(state: &str, meta: &Meta, now: DateTime<Utc>) -> Option<u64> {
    let stage_record = meta.stages.get(&meta.stage);
    if state != "running" {
        if let Some(duration) = stage_record.and_then(|record| record.duration_s) {
            return Some(duration);
        }
    }

    let started = stage_record.and_then(|record| record.started.as_ref())?;
    let elapsed = now.signed_duration_since(*started).num_seconds();
    if elapsed < 0 {
        Some(0)
    } else {
        Some(elapsed as u64)
    }
}

fn format_elapsed_duration(seconds: Option<u64>) -> String {
    let Some(seconds) = seconds else {
        return "-".to_string();
    };

    if seconds < 60 {
        return format!("{}s", seconds);
    }

    let minutes = seconds / 60;
    let secs = seconds % 60;
    if minutes < 60 {
        return format!("{}m{:02}s", minutes, secs);
    }

    let hours = minutes / 60;
    let mins = minutes % 60;
    if hours < 24 {
        return format!("{}h{:02}m", hours, mins);
    }

    let days = hours / 24;
    let hrs = hours % 24;
    format!("{}d{:02}h", days, hrs)
}

fn read_dashboard_last_log_line(card_dir: &Path) -> String {
    let stdout_path = card_dir.join("logs").join("stdout.log");
    let stderr_path = card_dir.join("logs").join("stderr.log");

    let stdout_line = read_last_nonempty_line(&stdout_path);
    let stderr_line = read_last_nonempty_line(&stderr_path);
    let stdout_mtime = file_modified_time(&stdout_path);
    let stderr_mtime = file_modified_time(&stderr_path);

    let selected = match (stdout_line, stderr_line) {
        (Some(out), Some(err)) => match (stdout_mtime, stderr_mtime) {
            (Some(out_t), Some(err_t)) if err_t >= out_t => err,
            (Some(_), Some(_)) => out,
            (None, Some(_)) => err,
            _ => out,
        },
        (Some(out), None) => out,
        (None, Some(err)) => err,
        (None, None) => "-".to_string(),
    };

    truncate_for_dashboard_cell(selected.as_str(), DASHBOARD_MAX_LAST_LOG_CHARS)
}

fn read_last_nonempty_line(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    if len == 0 {
        return None;
    }

    let start = len.saturating_sub(DASHBOARD_LOG_TAIL_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;

    let text = String::from_utf8_lossy(&buf);
    text.lines().rev().find_map(|line| {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn file_modified_time(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

fn truncate_for_dashboard_cell(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }

    let mut out = String::new();
    for ch in value.chars().take(limit) {
        out.push(ch);
    }
    out.push_str("...");
    out
}

fn dashboard_state_rank(state: &str) -> usize {
    match state {
        "running" => 0,
        "pending" => 1,
        "failed" => 2,
        "done" => 3,
        "merged" => 4,
        _ => 5,
    }
}

fn dashboard_state_color(state: &str) -> Color {
    match state {
        "running" => Color::Cyan,
        "pending" => Color::Yellow,
        "done" => Color::Green,
        "merged" => Color::Magenta,
        "failed" => Color::Red,
        _ => Color::Gray,
    }
}

fn draw_dashboard(frame: &mut Frame<'_>, app: &mut DashboardApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(2)])
        .split(frame.area());

    match &app.mode {
        DashboardMode::Table => draw_dashboard_table(frame, chunks[0], app),
        DashboardMode::Logs(log_view) => {
            let viewport_height = usize::from(chunks[0].height.saturating_sub(2).max(1));
            app.log_view_height = viewport_height;
            draw_dashboard_logs(frame, chunks[0], log_view, viewport_height);
        }
    }

    draw_dashboard_footer(frame, chunks[1], app);
}

fn draw_dashboard_table(frame: &mut Frame<'_>, area: Rect, app: &DashboardApp) {
    if app.jobs.is_empty() {
        let empty = Paragraph::new("No job cards found.")
            .block(Block::default().borders(Borders::ALL).title("jc dashboard"));
        frame.render_widget(empty, area);
        return;
    }

    let rows = app.jobs.iter().map(|job| {
        Row::new(vec![
            Cell::from(job.id.clone()),
            Cell::from(job.state.clone())
                .style(Style::default().fg(dashboard_state_color(job.state.as_str()))),
            Cell::from(job.provider.clone()),
            Cell::from(job.elapsed.clone()),
            Cell::from(job.last_log_line.clone()),
        ])
    });

    let header = Row::new(vec![
        "Job ID",
        "State",
        "Provider",
        "Elapsed",
        "Last log line",
    ])
    .style(Style::default().add_modifier(Modifier::BOLD));

    let table = Table::new(
        rows,
        [
            Constraint::Length(24),
            Constraint::Length(10),
            Constraint::Length(14),
            Constraint::Length(10),
            Constraint::Min(10),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title("jc dashboard"))
    .row_highlight_style(
        Style::default()
            .add_modifier(Modifier::REVERSED)
            .add_modifier(Modifier::BOLD),
    );

    let mut table_state = TableState::default();
    table_state.select(Some(app.selected));
    frame.render_stateful_widget(table, area, &mut table_state);
}

fn draw_dashboard_logs(
    frame: &mut Frame<'_>,
    area: Rect,
    log_view: &LogViewState,
    viewport_height: usize,
) {
    let text = log_view.render_text();
    let scroll = log_view.effective_scroll(viewport_height);
    let scroll_u16 = min(scroll, u16::MAX as usize) as u16;
    let logs = Paragraph::new(text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!("logs: {}", log_view.job_id)),
        )
        .wrap(Wrap { trim: false })
        .scroll((scroll_u16, 0));
    frame.render_widget(logs, area);
}

fn draw_dashboard_footer(frame: &mut Frame<'_>, area: Rect, app: &DashboardApp) {
    let mut help = match app.mode {
        DashboardMode::Table => "q quit | r retry | k kill | l logs | Up/Down select".to_string(),
        DashboardMode::Logs(_) => "q quit | l back | Up/Down/PgUp/PgDn/Home/End scroll".to_string(),
    };
    if !app.status_line.is_empty() {
        help.push_str(" | ");
        help.push_str(app.status_line.as_str());
    }
    let footer = Paragraph::new(help).block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, area);
}

async fn handle_dashboard_key(
    root: &Path,
    app: &mut DashboardApp,
    key_code: KeyCode,
) -> anyhow::Result<bool> {
    if key_code == KeyCode::Char('q') {
        return Ok(true);
    }

    if matches!(app.mode, DashboardMode::Table) {
        match key_code {
            KeyCode::Up => {
                if !app.jobs.is_empty() {
                    app.selected = app.selected.saturating_sub(1);
                }
            }
            KeyCode::Down => {
                if !app.jobs.is_empty() {
                    app.selected = min(app.selected.saturating_add(1), app.jobs.len() - 1);
                }
            }
            KeyCode::Char('r') => {
                if let Some(id) = app.jobs.get(app.selected).map(|job| job.id.clone()) {
                    app.status_line = match retry_card(root, id.as_str()) {
                        Ok(message) => message,
                        Err(err) => format!("retry failed: {}", err),
                    };
                }
            }
            KeyCode::Char('k') => {
                if let Some(id) = app.jobs.get(app.selected).map(|job| job.id.clone()) {
                    app.status_line = match kill_card(root, id.as_str()).await {
                        Ok(message) => message,
                        Err(err) => format!("kill failed: {}", err),
                    };
                }
            }
            KeyCode::Char('l') => {
                if let Some(job) = app.jobs.get(app.selected).cloned() {
                    let mut log_view = LogViewState::new(&job);
                    if let Err(err) = log_view.refresh(root) {
                        app.status_line = format!("log stream error: {}", err);
                    }
                    app.mode = DashboardMode::Logs(log_view);
                }
            }
            _ => {}
        }
        return Ok(false);
    }

    let page = app.log_view_height.max(1);
    let mut close_logs = false;
    if let DashboardMode::Logs(log_view) = &mut app.mode {
        match key_code {
            KeyCode::Char('l') | KeyCode::Esc => close_logs = true,
            KeyCode::Up => log_view.scroll_up(1, page),
            KeyCode::Down => log_view.scroll_down(1, page),
            KeyCode::PageUp => log_view.scroll_up(page, page),
            KeyCode::PageDown => log_view.scroll_down(page, page),
            KeyCode::Home => log_view.scroll_home(),
            KeyCode::End => log_view.scroll_end(page),
            _ => {}
        }
    }
    if close_logs {
        app.mode = DashboardMode::Table;
    }
    Ok(false)
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

#[allow(clippy::too_many_arguments)]
async fn run_dispatcher(
    cards_dir: &Path,
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

    let pending_dir = cards_dir.join("pending");
    let running_dir = cards_dir.join("running");
    let done_dir = cards_dir.join("done");
    let failed_dir = cards_dir.join("failed");

    let mut last_reap = std::time::Instant::now()
        .checked_sub(Duration::from_millis(reap_ms))
        .unwrap_or_else(std::time::Instant::now);

    loop {
        if !no_reap && last_reap.elapsed() >= Duration::from_millis(reap_ms) {
            reap_orphans(&running_dir, &pending_dir, &failed_dir, max_retries).await?;
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

                    let running_path = running_dir.join(&name);
                    if fs::rename(&pending_path, &running_path).is_err() {
                        continue;
                    }

                    // Create git worktree for isolation (best-effort; non-fatal if git not available)
                    if let Some(git_root) = find_git_root(&running_path) {
                        let wt_path = running_path.join("worktree");
                        let branch = format!(
                            "jobs/{}",
                            running_path
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("unknown")
                                .trim_end_matches(".jobcard")
                        );
                        if let Err(e) =
                            jobcard_core::worktree::create_worktree(&git_root, &wt_path, &branch)
                        {
                            eprintln!("[dispatcher] worktree create failed for {branch}: {e}");
                        }
                    }

                    available_slots = available_slots.saturating_sub(1);

                    let mut meta = jobcard_core::read_meta(&running_path).ok();
                    let stage = meta
                        .as_ref()
                        .map(|m| m.stage.clone())
                        .unwrap_or_else(|| "implement".to_string());

                    let (provider_name, provider_cmd, rate_limit_exit) =
                        match select_provider(cards_dir, meta.as_mut(), &stage)? {
                            Some(v) => v,
                            None => {
                                let _ = fs::rename(&running_path, pending_dir.join(&name));
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
        let wt = card_dir.join("worktree");
        if wt.exists() {
            wt
        } else {
            card_dir.to_path_buf()
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

    let mut cmd = if adapter.ends_with(".sh") {
        let mut c = TokioCommand::new("bash");
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

    if let Some(pid) = child.id() {
        let pid_str = pid.to_string();
        let _ = fs::write(card_dir.join("logs").join("pid"), &pid_str);
        let _ = TokioCommand::new("xattr")
            .arg("-w")
            .arg("com.yourorg.agent-pid")
            .arg(&pid_str)
            .arg(card_dir)
            .status()
            .await;
    }

    let status = child.wait().await?;
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

async fn run_merge_gate(cards_dir: &Path, poll_ms: u64, once: bool) -> anyhow::Result<()> {
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
                        let _ = fs::rename(&card_dir, failed_dir.join(&name));
                        continue;
                    }
                };

                fs::create_dir_all(card_dir.join("logs"))?;
                fs::create_dir_all(card_dir.join("output"))?;

                let workdir = {
                    let wt = card_dir.join("worktree");
                    if wt.exists() {
                        wt
                    } else {
                        card_dir.clone()
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
                    let _ = fs::rename(&card_dir, failed_dir.join(&name));
                    continue;
                }

                let wt_path = card_dir.join("worktree");
                if wt_path.exists() {
                    // Derive card_id by stripping the ".jobcard" extension from the filename.
                    let card_id = name.trim_end_matches(".jobcard");

                    // Step 1: Stage and commit all agent changes from inside the worktree.
                    if let Err(e) = jobcard_core::worktree::commit_worktree(&wt_path, card_id) {
                        let _ =
                            fs::write(&qa_log, format!("commit_worktree failed: {e}\n").as_bytes());
                        meta.failure_reason = Some("worktree_commit_failed".to_string());
                        let _ = write_meta(&card_dir, &meta);
                        let _ = fs::rename(&card_dir, failed_dir.join(&name));
                        continue;
                    }

                    // Step 2: Find the main repo root (works from any path inside the repo).
                    let git_root = match find_git_root(&card_dir) {
                        Some(r) => r,
                        None => {
                            let _ = fs::write(&qa_log, b"find_git_root failed\n");
                            meta.failure_reason = Some("git_root_not_found".to_string());
                            let _ = write_meta(&card_dir, &meta);
                            let _ = fs::rename(&card_dir, failed_dir.join(&name));
                            continue;
                        }
                    };

                    // Step 3: Determine the branch name.
                    // Prefer the canonical dispatcher format; fall back to meta.worktree_branch.
                    let branch = {
                        let preferred = format!("jobs/{}", card_id);
                        // Check if the preferred branch exists in the repo.
                        let exists = std::process::Command::new("git")
                            .args(["rev-parse", "--verify", &preferred])
                            .current_dir(&git_root)
                            .output()
                            .map(|o| o.status.success())
                            .unwrap_or(false);
                        if exists {
                            preferred
                        } else {
                            meta.worktree_branch.clone().unwrap_or(preferred)
                        }
                    };

                    // Step 4: Merge the card branch into main from the git root.
                    match jobcard_core::worktree::merge_card_branch(&git_root, &branch) {
                        Ok(true) => {
                            // Merge succeeded — clean up the worktree, then move to merged/.
                            let _ = jobcard_core::worktree::remove_worktree(&git_root, &wt_path);
                            let _ = write_meta(&card_dir, &meta);
                            let _ = fs::rename(&card_dir, merged_dir.join(&name));
                            continue;
                        }
                        Ok(false) => {
                            // Merge conflict — write conflicts.diff and abort the merge.
                            meta.failure_reason = Some("merge_conflict".to_string());
                            let conflicts = std::process::Command::new("git")
                                .args(["diff", "--name-only", "--diff-filter=U"])
                                .current_dir(&git_root)
                                .output()
                                .ok();
                            if let Some(c) = conflicts {
                                let _ = fs::create_dir_all(card_dir.join("output"));
                                let _ = fs::write(
                                    card_dir.join("output").join("conflicts.diff"),
                                    c.stdout,
                                );
                            }
                            // Abort the merge so the repo stays clean.
                            let _ = std::process::Command::new("git")
                                .args(["merge", "--abort"])
                                .current_dir(&git_root)
                                .status();
                            let _ = write_meta(&card_dir, &meta);
                            let _ = fs::rename(&card_dir, failed_dir.join(&name));
                            continue;
                        }
                        Err(e) => {
                            let _ = fs::write(
                                &qa_log,
                                format!("merge_card_branch error: {e}\n").as_bytes(),
                            );
                            meta.failure_reason = Some("merge_failed".to_string());
                            let _ = write_meta(&card_dir, &meta);
                            let _ = fs::rename(&card_dir, failed_dir.join(&name));
                            continue;
                        }
                    }
                }

                let _ = write_meta(&card_dir, &meta);
                let _ = fs::rename(&card_dir, merged_dir.join(&name));
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
        let _ = write_meta(&card, &meta);
    }

    let target = root.join("pending").join(format!("{}.jobcard", id));
    fs::rename(&card, &target)
        .with_context(|| format!("failed to move card to pending/: {}", id))?;
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

    // Send SIGTERM (kill -15)
    let sent = TokioCommand::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()
        .await
        .with_context(|| format!("failed to send SIGTERM to pid {}", pid))?;

    if !sent.success() {
        anyhow::bail!("kill -TERM {} returned non-zero", pid);
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

    Ok(format!("killed pid {} and moved '{}' to failed/", pid, id))
}

// ── logs ─────────────────────────────────────────────────────────────────────

async fn cmd_logs(root: &Path, id: &str, follow: bool) -> anyhow::Result<()> {
    let card = find_card(root, id).with_context(|| format!("card not found: {}", id))?;
    let stdout_log = card.join("logs").join("stdout.log");
    let stderr_log = card.join("logs").join("stderr.log");

    if !follow {
        // Print all existing content once
        print_log_section("stdout", &stdout_log)?;
        print_log_section("stderr", &stderr_log)?;
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
        print!("{}", String::from_utf8_lossy(&buf));
    }
    let mut stdout_pos = stdout_file.stream_position()?;
    buf.clear();

    stderr_file.read_to_end(&mut buf)?;
    if !buf.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&buf));
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
            print!("{}", String::from_utf8_lossy(&buf));
            std::io::stdout().flush()?;
            stdout_pos += buf.len() as u64;
        }

        // Read any new bytes from stderr
        stderr_file.seek(SeekFrom::Start(stderr_pos))?;
        buf.clear();
        stderr_file.read_to_end(&mut buf)?;
        if !buf.is_empty() {
            eprint!("{}", String::from_utf8_lossy(&buf));
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

fn print_log_section(label: &str, path: &Path) -> anyhow::Result<()> {
    if !path.exists() {
        println!("=== {} (no file) ===", label);
        return Ok(());
    }
    let content = fs::read_to_string(path)?;
    println!("=== {} ===", label);
    print!("{}", content);
    if !content.ends_with('\n') && !content.is_empty() {
        println!();
    }
    Ok(())
}

fn find_card_in_state(root: &Path, id: &str, state: &str) -> bool {
    root.join(state).join(format!("{}.jobcard", id)).exists()
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

fn find_card(root: &Path, id: &str) -> Option<PathBuf> {
    let name = format!("{}.jobcard", id);
    for dir in ["pending", "running", "done", "merged", "failed"] {
        let p = root.join(dir).join(&name);
        if p.exists() {
            return Some(p);
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

fn cmd_worktree_list(_root: &Path) -> anyhow::Result<()> {
    todo!()
}

fn cmd_worktree_create(_root: &Path, _id: &str) -> anyhow::Result<()> {
    todo!()
}

fn cmd_worktree_clean(_root: &Path, _dry_run: bool) -> anyhow::Result<()> {
    todo!()
}

// ---------------------------------------------------------------------------
// Providers subcommand stubs (implementation by providers-cli agent)
// ---------------------------------------------------------------------------

fn cmd_providers_list(_root: &Path) -> anyhow::Result<()> {
    todo!("providers list not yet implemented")
}

fn cmd_providers_add(
    _root: &Path,
    _name: &str,
    _adapter: &str,
    _model: Option<&str>,
) -> anyhow::Result<()> {
    todo!("providers add not yet implemented")
}

fn cmd_providers_remove(_root: &Path, _name: &str, _force: bool) -> anyhow::Result<()> {
    todo!("providers remove not yet implemented")
}

fn cmd_providers_status(_root: &Path) -> anyhow::Result<()> {
    todo!("providers status not yet implemented")
}

// ---------------------------------------------------------------------------
// Config subcommands
// ---------------------------------------------------------------------------

/// Return the config path: JOBCARD_CONFIG env var if set, else project config path.
fn resolve_config_path() -> PathBuf {
    if let Ok(p) = std::env::var("JOBCARD_CONFIG") {
        return PathBuf::from(p);
    }
    jobcard_core::config::project_config_path()
}

fn cmd_config_get(config_path: &Path, key: &str) -> anyhow::Result<()> {
    let cfg = if config_path.exists() {
        jobcard_core::config::read_config_file(config_path)?
    } else {
        jobcard_core::Config::default()
    };

    match key {
        "default_provider_chain" => match cfg.default_provider_chain {
            Some(chain) => println!("{}", chain.join(",")),
            None => println!("(unset)"),
        },
        "max_concurrent" => match cfg.max_concurrent {
            Some(v) => println!("{}", v),
            None => println!("(unset)"),
        },
        "cooldown_seconds" => match cfg.cooldown_seconds {
            Some(v) => println!("{}", v),
            None => println!("(unset)"),
        },
        "log_retention_days" => match cfg.log_retention_days {
            Some(v) => println!("{}", v),
            None => println!("(unset)"),
        },
        "default_template" => match cfg.default_template {
            Some(v) => println!("{}", v),
            None => println!("(unset)"),
        },
        _ => anyhow::bail!(
            "unknown config key '{}'. Valid keys: default_provider_chain, \
            max_concurrent, cooldown_seconds, log_retention_days, default_template",
            key
        ),
    }
    Ok(())
}

fn cmd_config_set(config_path: &Path, key: &str, value: &str) -> anyhow::Result<()> {
    let mut cfg = if config_path.exists() {
        jobcard_core::config::read_config_file(config_path)?
    } else {
        jobcard_core::Config::default()
    };

    match key {
        "default_provider_chain" => {
            cfg.default_provider_chain =
                Some(value.split(',').map(|s| s.trim().to_string()).collect());
        }
        "max_concurrent" => {
            cfg.max_concurrent = Some(value.parse::<usize>().with_context(|| {
                format!(
                    "max_concurrent must be a positive integer, got: {}",
                    value
                )
            })?);
        }
        "cooldown_seconds" => {
            cfg.cooldown_seconds = Some(value.parse::<u64>().with_context(|| {
                format!(
                    "cooldown_seconds must be a non-negative integer, got: {}",
                    value
                )
            })?);
        }
        "log_retention_days" => {
            cfg.log_retention_days = Some(value.parse::<u64>().with_context(|| {
                format!(
                    "log_retention_days must be a non-negative integer, got: {}",
                    value
                )
            })?);
        }
        "default_template" => {
            cfg.default_template = Some(value.to_string());
        }
        _ => anyhow::bail!(
            "unknown config key '{}'. Valid keys: default_provider_chain, \
            max_concurrent, cooldown_seconds, log_retention_days, default_template",
            key
        ),
    }

    jobcard_core::config::write_config_file(config_path, &cfg)?;
    Ok(())
}
