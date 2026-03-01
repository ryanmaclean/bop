use anyhow::Context;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use clap::{Parser, Subcommand};
use jobcard_core::{write_meta, Meta};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::time::Duration;
use tokio::process::Command as TokioCommand;
use walkdir::WalkDir;

#[derive(Parser, Debug)]
#[command(name = "jc")]
struct Cli {
    #[arg(long, default_value = ".cards")]
    cards_dir: String,

    #[command(subcommand)]
    cmd: Command,
}

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
    },
    Dispatcher {
        #[arg(long, default_value = "adapters/mock.sh")]
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

fn ensure_cards_layout(root: &Path) -> anyhow::Result<()> {
    for dir in [
        "templates",
        "pending",
        "running",
        "done",
        "merged",
        "failed",
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
            println!("[{}] {}", state, meta.id);
            println!("{}", serde_json::to_string_pretty(&meta)?);
            Ok(())
        }
        Command::Validate { id } => {
            let card = find_card(&root, &id).context("card not found")?;
            let _ = jobcard_core::read_meta(&card)?;
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
        } => {
            let effective_max_workers = max_workers.or(cfg.max_concurrent).unwrap_or(1);
            let effective_max_retries = max_retries.unwrap_or(3);
            run_dispatcher(
                &root,
                &adapter,
                effective_max_workers,
                poll_ms,
                effective_max_retries,
                reap_ms,
                no_reap,
                once,
            )
            .await
        }
        Command::MergeGate { poll_ms, once } => run_merge_gate(&root, poll_ms, once).await,
        Command::Retry { id } => cmd_retry(&root, &id),
        Command::Kill { id } => cmd_kill(&root, &id).await,
        Command::Logs { id, follow } => cmd_logs(&root, &id, follow).await,
        Command::Inspect { id } => cmd_inspect(&root, &id),
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

                    if let Some(ref mut meta) = meta {
                        if is_rate_limited {
                            let next = meta.retry_count.unwrap_or(0).saturating_add(1);
                            meta.retry_count = Some(next);

                            rotate_provider_chain(meta);
                            let _ = set_provider_cooldown(cards_dir, &provider_name, 300);
                        }

                        let _ = write_meta(&running_path, meta);
                    }
                    let target = if exit_code == 0 {
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
    _cards_dir: &Path,
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

    // Render prompt template with actual values
    let mut meta = jobcard_core::read_meta(card_dir).ok();
    if let Some(ref m) = meta {
        let ctx = jobcard_core::PromptContext::from_files(card_dir, m)?;
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

/// Returns (path, branch) for every linked worktree known to git (excluding the main worktree).
/// Returns empty vec if git is unavailable or not in a git repo.
fn git_worktree_paths(from_dir: &Path) -> Vec<(PathBuf, String)> {
    let Ok(out) = StdCommand::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(from_dir)
        .output()
    else {
        return vec![];
    };
    if !out.status.success() {
        return vec![];
    }

    let text = String::from_utf8_lossy(&out.stdout);
    let mut result = Vec::new();
    let mut current_path: Option<PathBuf> = None;
    let mut current_branch = String::new();
    let mut block_index: usize = 0;

    for line in text.lines() {
        if line.starts_with("worktree ") {
            if let Some(p) = current_path.take() {
                if block_index > 1 {
                    result.push((p, std::mem::take(&mut current_branch)));
                }
            }
            current_path = line.strip_prefix("worktree ").map(PathBuf::from);
            current_branch = String::new();
            block_index += 1;
        } else if let Some(rest) = line.strip_prefix("branch refs/heads/") {
            current_branch = rest.to_string();
        } else if line.is_empty() {
            if let Some(p) = current_path.take() {
                if block_index > 1 {
                    result.push((p, std::mem::take(&mut current_branch)));
                }
            }
        }
    }
    if let Some(p) = current_path {
        if block_index > 1 {
            result.push((p, current_branch));
        }
    }
    result
}

fn cmd_worktree_list(root: &Path) -> anyhow::Result<()> {
    let states = ["pending", "running", "done", "merged", "failed"];
    let mut card_worktrees: Vec<(PathBuf, String, String, String)> = Vec::new();

    for &state in &states {
        let dir = root.join(state);
        if !dir.exists() {
            continue;
        }
        for ent in fs::read_dir(&dir)?.flatten() {
            let card_dir = ent.path();
            if !card_dir.is_dir() {
                continue;
            }
            if card_dir.extension().and_then(|s| s.to_str()).unwrap_or("") != "jobcard" {
                continue;
            }
            let wt_path = card_dir.join("worktree");
            if !wt_path.exists() {
                continue;
            }
            let meta = jobcard_core::read_meta(&card_dir).ok();
            let id = meta.as_ref().map(|m| m.id.clone()).unwrap_or_else(|| {
                card_dir
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("?")
                    .to_string()
            });
            let branch = meta
                .as_ref()
                .and_then(|m| m.worktree_branch.clone())
                .unwrap_or_else(|| "?".to_string());
            card_worktrees.push((wt_path, id, branch, state.to_string()));
        }
    }

    println!("{:<20} {:<30} {:<10} PATH", "ID", "BRANCH", "STATUS");
    for (path, id, branch, state) in &card_worktrees {
        println!("{:<20} {:<30} {:<10} {}", id, branch, state, path.display());
    }

    let git_wt_list = git_worktree_paths(root);
    let known_paths: std::collections::HashSet<PathBuf> = card_worktrees
        .iter()
        .map(|(p, _, _, _)| p.clone())
        .collect();

    for (gp, _branch) in git_wt_list {
        if !known_paths.contains(&gp) {
            println!(
                "{:<20} {:<30} {:<10} {}",
                "[orphaned]",
                "?",
                "orphaned",
                gp.display()
            );
        }
    }

    Ok(())
}

fn cmd_worktree_create(root: &Path, id: &str) -> anyhow::Result<()> {
    let card = ["pending", "running"]
        .iter()
        .find_map(|&state| {
            let p = root.join(state).join(format!("{}.jobcard", id));
            if p.exists() {
                Some(p)
            } else {
                None
            }
        })
        .with_context(|| format!("card \'{}\' not found in pending/ or running/", id))?;

    let meta = jobcard_core::read_meta(&card)?;
    let branch = meta
        .worktree_branch
        .as_deref()
        .unwrap_or(&format!("job/{}", id))
        .to_string();

    let wt_path = card.join("worktree");
    if wt_path.exists() {
        anyhow::bail!("worktree already exists for card \'{}\'", id);
    }

    let git_root_out = StdCommand::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(root.parent().unwrap_or(root))
        .output()
        .context("failed to run git; is this directory inside a git repo?")?;
    if !git_root_out.status.success() {
        anyhow::bail!("not inside a git repository");
    }
    let git_root = PathBuf::from(String::from_utf8(git_root_out.stdout)?.trim());

    let add = StdCommand::new("git")
        .args(["worktree", "add", "-b", &branch, wt_path.to_str().unwrap()])
        .current_dir(&git_root)
        .output()?;

    if !add.status.success() {
        let add2 = StdCommand::new("git")
            .args(["worktree", "add", wt_path.to_str().unwrap(), &branch])
            .current_dir(&git_root)
            .output()?;
        if !add2.status.success() {
            let err = String::from_utf8_lossy(&add2.stderr);
            anyhow::bail!("git worktree add failed: {}", err);
        }
    }

    println!(
        "created worktree for \'{}\' at {} (branch: {})",
        id,
        wt_path.display(),
        branch
    );
    Ok(())
}

/// Remove a directory: first try `git worktree remove --force`, fall back to `fs::remove_dir_all`.
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

fn cmd_worktree_clean(root: &Path, dry_run: bool) -> anyhow::Result<()> {
    let stale_states = ["done", "merged"];
    let mut to_remove: Vec<PathBuf> = Vec::new();

    for &state in &stale_states {
        let dir = root.join(state);
        if !dir.exists() {
            continue;
        }
        for ent in fs::read_dir(&dir)?.flatten() {
            let card_dir = ent.path();
            if !card_dir.is_dir() {
                continue;
            }
            if card_dir.extension().and_then(|s| s.to_str()).unwrap_or("") != "jobcard" {
                continue;
            }
            let wt = card_dir.join("worktree");
            if wt.exists() {
                to_remove.push(wt);
            }
        }
    }

    let git_root: Option<PathBuf> = StdCommand::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(root.parent().unwrap_or(root))
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| PathBuf::from(s.trim()));

    if let Some(ref gr) = git_root {
        let active_wt_paths: std::collections::HashSet<PathBuf> = ["pending", "running"]
            .iter()
            .flat_map(|&state| {
                let dir = root.join(state);
                fs::read_dir(dir)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter_map(|e| {
                        let p = e.path();
                        if p.extension().and_then(|s| s.to_str()).unwrap_or("") == "jobcard" {
                            Some(p.join("worktree"))
                        } else {
                            None
                        }
                    })
            })
            .collect();

        for (wt_path, _branch) in git_worktree_paths(gr) {
            if !active_wt_paths.contains(&wt_path) && !to_remove.contains(&wt_path) {
                to_remove.push(wt_path);
            }
        }
    }

    if to_remove.is_empty() {
        println!("nothing to clean");
        return Ok(());
    }

    for path in &to_remove {
        if dry_run {
            println!("would remove: {}", path.display());
        } else {
            println!("removing: {}", path.display());
            remove_worktree(path, git_root.as_deref())?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Providers subcommand implementations
// ---------------------------------------------------------------------------

fn cmd_providers_list(root: &Path) -> anyhow::Result<()> {
    let pf = read_providers(root)?;
    let now = chrono::Utc::now().timestamp();
    if pf.providers.is_empty() {
        println!("No providers configured. Run: jc providers add <name> --adapter <script>");
        return Ok(());
    }
    println!("{:<20} {:<30} {:<20} COOLDOWN", "NAME", "ADAPTER", "MODEL");
    for (name, p) in &pf.providers {
        let model = p.model.as_deref().unwrap_or("-");
        let cooldown = match p.cooldown_until_epoch_s {
            Some(until) if until > now => format!("cooldown: {}s", until - now),
            _ => "none".to_string(),
        };
        println!("{:<20} {:<30} {:<20} {}", name, p.command, model, cooldown);
    }
    Ok(())
}

fn cmd_providers_add(
    root: &Path,
    name: &str,
    adapter: &str,
    model: Option<&str>,
) -> anyhow::Result<()> {
    if name.trim().is_empty() {
        anyhow::bail!("provider name cannot be empty");
    }
    if adapter.trim().is_empty() {
        anyhow::bail!("--adapter cannot be empty");
    }

    let mut pf = read_providers(root)?;

    if pf.providers.contains_key(name) {
        anyhow::bail!(
            "provider '{}' already exists. Use 'providers remove' first.",
            name
        );
    }

    let provider = Provider {
        command: adapter.to_string(),
        rate_limit_exit: 75,
        cooldown_until_epoch_s: None,
        model: model.map(str::to_string),
    };
    validate_provider(name, &provider)?;

    pf.providers.insert(name.to_string(), provider);
    write_providers(root, &pf)?;
    println!("Added provider '{}'.", name);
    Ok(())
}

struct ProviderStats {
    total: usize,
    success: usize,
    failed: usize,
}

fn count_active_jobs_for_provider(cards_dir: &Path, provider_name: &str) -> usize {
    let running_dir = cards_dir.join("running");
    let Ok(entries) = std::fs::read_dir(&running_dir) else {
        return 0;
    };
    let mut count = 0;
    for ent in entries.flatten() {
        let card_dir = ent.path();
        if !card_dir.is_dir() {
            continue;
        }
        if card_dir.extension().and_then(|s| s.to_str()).unwrap_or("") != "jobcard" {
            continue;
        }
        if let Ok(meta) = jobcard_core::read_meta(&card_dir) {
            for record in meta.stages.values() {
                if record.provider.as_deref() == Some(provider_name) {
                    count += 1;
                    break;
                }
            }
        }
    }
    count
}

fn compute_provider_stats(cards_dir: &Path) -> std::collections::BTreeMap<String, ProviderStats> {
    let mut stats: std::collections::BTreeMap<String, ProviderStats> = Default::default();
    let state_dirs = ["pending", "running", "done", "merged", "failed"];
    for state in state_dirs {
        let dir = cards_dir.join(state);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for ent in entries.flatten() {
            let card_dir = ent.path();
            if !card_dir.is_dir() {
                continue;
            }
            if card_dir.extension().and_then(|s| s.to_str()).unwrap_or("") != "jobcard" {
                continue;
            }
            let Ok(meta) = jobcard_core::read_meta(&card_dir) else {
                continue;
            };
            for record in meta.stages.values() {
                let Some(prov) = &record.provider else {
                    continue;
                };
                let entry = stats.entry(prov.clone()).or_insert(ProviderStats {
                    total: 0,
                    success: 0,
                    failed: 0,
                });
                entry.total += 1;
                match record.status {
                    jobcard_core::StageStatus::Done => entry.success += 1,
                    jobcard_core::StageStatus::Failed => entry.failed += 1,
                    _ => {}
                }
            }
        }
    }
    stats
}

fn cmd_providers_remove(root: &Path, name: &str, force: bool) -> anyhow::Result<()> {
    let mut pf = read_providers(root)?;
    if !pf.providers.contains_key(name) {
        anyhow::bail!("provider '{}' not found", name);
    }

    let active = count_active_jobs_for_provider(root, name);
    if active > 0 && !force {
        anyhow::bail!(
            "provider '{}' has {} active job(s) in running/. \
             Use --force to remove anyway.",
            name,
            active
        );
    }

    pf.providers.remove(name);
    // Clear default_provider if it was pointing to the removed provider
    if pf.default_provider.as_deref() == Some(name) {
        pf.default_provider = None;
    }
    write_providers(root, &pf)?;
    if active > 0 {
        eprintln!("Warning: removed '{}' with {} active job(s).", name, active);
    }
    println!("Removed provider '{}'.", name);
    Ok(())
}

fn cmd_providers_status(root: &Path) -> anyhow::Result<()> {
    let pf = read_providers(root)?;
    let stats = compute_provider_stats(root);
    let now = chrono::Utc::now().timestamp();

    if pf.providers.is_empty() {
        println!("No providers configured.");
        return Ok(());
    }

    println!(
        "{:<20} {:>6} {:>8} {:>8} COOLDOWN",
        "PROVIDER", "TOTAL", "SUCCESS", "FAILED"
    );
    for (name, p) in &pf.providers {
        let s = stats.get(name);
        let total = s.map(|s| s.total).unwrap_or(0);
        let success = s.map(|s| s.success).unwrap_or(0);
        let failed = s.map(|s| s.failed).unwrap_or(0);
        let cooldown = match p.cooldown_until_epoch_s {
            Some(until) if until > now => format!("{}s", until - now),
            _ => "none".to_string(),
        };
        println!(
            "{:<20} {:>6} {:>8} {:>8} {}",
            name, total, success, failed, cooldown
        );
    }
    Ok(())
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
                format!("max_concurrent must be a positive integer, got: {}", value)
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
