use anyhow::Context;
use chrono::{Duration as ChronoDuration, Utc};
use clap::{Parser, Subcommand, ValueEnum};
use jobcard_core::{write_meta, Meta, RunRecord, StageStatus, VcsEngine as CoreVcsEngine};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::process::Command as TokioCommand;

mod doctor;
mod events;
mod factory;
mod icons;
mod index;
mod inspect;
mod list;
mod lock;
mod logs;
mod memory;
mod paths;
mod poker;
mod policy;
mod providers;
mod quicklook;
mod toast;
mod util;

#[derive(Parser, Debug)]
#[command(name = "bop")]
struct Cli {
    #[arg(long, default_value = ".cards")]
    cards_dir: String,

    #[command(subcommand)]
    cmd: Command,
}

// memory::DEFAULT_MEMORY_TTL_SECONDS → memory.rs
// LEASE_HEARTBEAT_INTERVAL, LEASE_STALE_FLOOR, DISPATCHER_LOCK_REL → lock.rs
// RUN_ID_SEQ → util.rs

#[derive(Subcommand, Debug)]
enum Command {
    Init,
    New {
        template: String,
        id: String,
        /// Team for glyph suit assignment (cli, arch, quality, platform).
        /// Auto-detected from card directory if omitted.
        #[arg(long)]
        team: Option<String>,
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
        #[arg(short = 'a', long, default_value = "adapters/mock.zsh")]
        adapter: String,

        #[arg(short = 'w', long)]
        max_workers: Option<usize>,

        #[arg(long, default_value_t = 500)]
        poll_ms: u64,

        #[arg(long)]
        max_retries: Option<u32>,

        #[arg(long, default_value_t = 1000)]
        reap_ms: u64,

        #[arg(long)]
        no_reap: bool,

        #[arg(short = '1', long)]
        once: bool,

        /// Error-rate threshold (0.0–1.0) above which a job with critical alerts
        /// is moved to failed/ instead of done/. Default 1.0 means never fail.
        #[arg(long, default_value_t = 1.0)]
        validation_fail_threshold: f64,

        /// VCS engine used for workspace preparation and publish.
        #[arg(short = 'v', long, value_enum, default_value_t = VcsEngine::GitGt)]
        vcs_engine: VcsEngine,
    },
    MergeGate {
        #[arg(long, default_value_t = 500)]
        poll_ms: u64,

        #[arg(short = '1', long)]
        once: bool,

        /// VCS engine used for finalize/publish flow.
        #[arg(short = 'v', long, value_enum, default_value_t = VcsEngine::GitGt)]
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
    /// List cards with glyphs, stages, and progress.
    List {
        /// Filter: pending, running, done, failed, merged, active (default), all.
        #[arg(long, default_value = "active")]
        state: String,
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
    /// Promote cards from drafts/ to pending/, making them eligible for dispatch.
    Promote {
        /// Card ID, or "all" to promote every draft.
        id: String,
    },
    /// Import cards from a YAML file into drafts/ (or pending/ with --immediate).
    Import {
        /// Path to YAML file with card definitions (same format as cards.yaml).
        source: String,
        /// Import directly to pending/ instead of drafts/.
        #[arg(long)]
        immediate: bool,
    },
    /// Generate a concise codebase map at .cards/CODEBASE.md for agent orientation.
    Index {
        /// Print to stdout instead of writing the file.
        #[arg(long)]
        print: bool,
    },
    /// Quick-create an ideation card from a topic string.
    #[command(alias = "brainstorm", alias = "ideation")]
    Bstorm {
        /// Topic words (joined into the spec & slugified into the card ID).
        topic: Vec<String>,
        /// Team for glyph suit assignment (cli, arch, quality, platform).
        #[arg(long)]
        team: Option<String>,
    },
    /// Show OpenLineage events from .cards/events.jsonl.
    Events {
        /// Filter events by card ID.
        #[arg(long)]
        card: Option<String>,
        /// Output raw JSONL instead of formatted table.
        #[arg(long)]
        json: bool,
        /// Number of recent events to show.
        #[arg(long, default_value = "20")]
        limit: usize,
        /// Health check: verify events.jsonl integrity and print summary.
        #[arg(long)]
        check: bool,
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

// MemoryStore, MemoryEntry, MemoryOutput, MemoryOutputOps, MemoryOutputValue → memory.rs

// RunLease, DispatcherLockOwner, DispatcherLockGuard → lock.rs
// lock_owner_path, lease_path → lock.rs
// host_name, next_run_id, pid_is_alive_sync → util.rs
// acquire_dispatcher_lock, write_run_lease, read_run_lease, lease_is_stale → lock.rs

// find_repo_script → util.rs

// macos_notify → toast.rs
// card_state_from_path, infer_card_id_from_path, write_webloc → quicklook.rs
// sync_card_action_links, render_card_thumbnail, compress_card → quicklook.rs

// ensure_cards_layout, clone_template, cow_copy_file → paths.rs

// Provider, ProvidersFile, providers::ProviderSelection → providers.rs
// ensure_mock_provider_command, validate_provider → providers.rs
// providers_path, read_providers, write_providers, seed_providers → providers.rs

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

// validate_provider, providers_path, read_providers, write_providers, seed_providers → providers.rs (removed)

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
        let lease = lock::read_run_lease(&card_dir);
        let lease_stale = lease
            .as_ref()
            .map(|l| lock::lease_is_stale(l, stale_after_chrono))
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
        quicklook::render_card_thumbnail(&target);
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

    if let Some(lease) = lock::read_run_lease(card_dir) {
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
            card_type: None,
            metadata_source: None,
            metadata_key: None,
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
            depends_on: vec![],
            spawn_to: None,
            policy_result: None,
            timeout_seconds: None,
            retry_count: Some(0),
            failure_reason: None,
            validation_summary: None,
            glyph: None,
            token: None,
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
            runs: vec![],
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
    team_override: Option<&str>,
) -> anyhow::Result<PathBuf> {
    paths::ensure_cards_layout(cards_dir)?;

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

    let card_dir = cards_dir.join("pending").join(format!(
        "{}-{}.jobcard",
        jobcard_core::cardchars::CARD_BACK,
        id
    ));
    if card_dir.exists() {
        anyhow::bail!("card already exists: {}", id);
    }

    paths::clone_template(&template_dir, &card_dir)
        .with_context(|| format!("failed to clone template {}", template))?;

    fs::create_dir_all(card_dir.join("logs"))?;
    fs::create_dir_all(card_dir.join("output"))?;

    let mut meta = jobcard_core::read_meta(&card_dir).unwrap_or_else(|_| Meta {
        id: id.to_string(),
        created: Utc::now(),
        agent_type: None,
        stage: "spec".to_string(),
        card_type: None,
        metadata_source: None,
        metadata_key: None,
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
        depends_on: vec![],
        spawn_to: None,
        policy_result: None,
        timeout_seconds: None,
        retry_count: Some(0),
        failure_reason: None,
        validation_summary: None,
        glyph: None,
        token: None,
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
        runs: vec![],
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
            .unwrap_or_else(|| util::workflow_mode_for_template(template).to_string()),
    );
    meta.step_index = Some(util::current_stage_step_index(&meta));

    // Auto-assign glyph + token if not already set by template
    if meta.glyph.is_none() {
        use jobcard_core::cardchars::{self, Team};
        let team = match team_override {
            Some("cli") => Team::Cli,
            Some("arch") => Team::Arch,
            Some("quality") => Team::Quality,
            Some("platform") => Team::Platform,
            Some(other) => {
                eprintln!("warning: unknown team '{}', defaulting to cli", other);
                Team::Cli
            }
            None => cardchars::team_from_path(&card_dir),
        };
        let used = cardchars::collect_used_glyphs(cards_dir);
        if let Some((glyph, token)) = cardchars::next_glyph(team, &used) {
            meta.glyph = Some(glyph);
            meta.token = Some(token);
        } else {
            eprintln!("warning: suit full for {:?}, no glyph assigned", team);
        }
    }

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

    // Rename card dir from 🂠 placeholder to actual glyph prefix
    if let Some(ref g) = meta.glyph {
        let new_name = format!("{}-{}.jobcard", g, id);
        let new_dir = card_dir.parent().unwrap().join(&new_name);
        if !new_dir.exists() {
            fs::rename(&card_dir, &new_dir)?;
            quicklook::render_card_thumbnail(&new_dir);
            return Ok(new_dir);
        }
    }

    quicklook::render_card_thumbnail(&card_dir);

    Ok(card_dir)
}

// normalize_namespace, sanitize_namespace, memory_store_path → memory.rs
// prune_memory_store, read_memory_store, write_memory_store → memory.rs
// set_memory_entry, format_memory_for_prompt, memory_namespace_from_meta → memory.rs
// parse_memory_output, merge_memory_output → memory.rs

// append_log_line → util.rs
// workflow_mode_for_template → util.rs
// current_stage_step_index → util.rs
// unique_failed_path → util.rs

// quarantine_invalid_pending_card → paths.rs

// policy → policy.rs
// factory → factory.rs
// icons → icons.rs

// doctor → doctor.rs

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
            paths::ensure_cards_layout(&root)?;
            seed_default_templates(&root)?;
            providers::seed_providers(&root)?;
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
        Command::New { template, id, team } => {
            create_card(&root, &template, &id, None, team.as_deref())?;
            Ok(())
        }
        Command::Status { id } => {
            if id.trim().is_empty() {
                return doctor::print_status_summary(&root);
            }

            let card = paths::find_card(&root, &id).context("card not found")?;
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
            let card = paths::find_card(&root, &id).context("card not found")?;
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
        Command::Logs { id, follow } => logs::cmd_logs(&root, &id, follow).await,
        Command::Inspect { id } => inspect::cmd_inspect(&root, &id),
        Command::List { state } => list::list_cards(&root, &state),
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
            PolicyAction::Check { id, staged } => {
                policy::cmd_policy_check(&root, id.as_deref(), staged)
            }
        },
        Command::Doctor => doctor::cmd_doctor(&root),
        Command::GenerateCompletion { shell } => {
            use clap::CommandFactory;
            clap_complete::generate(shell, &mut Cli::command(), "bop", &mut std::io::stdout());
            Ok(())
        }
        Command::Poker { action } => match action {
            PokerAction::Open { id } => poker::cmd_poker_open(&root, &id),
            PokerAction::Submit { id, glyph, name } => {
                poker::cmd_poker_submit(&root, &id, glyph.as_deref(), name.as_deref())
            }
            PokerAction::Reveal { id } => poker::cmd_poker_reveal(&root, &id),
            PokerAction::Status { id } => poker::cmd_poker_status(&root, &id),
            PokerAction::Consensus { id, glyph } => poker::cmd_poker_consensus(&root, &id, &glyph),
        },
        Command::Factory { action } => match action {
            FactoryAction::Install => factory::cmd_factory_install(&root),
            FactoryAction::Start => factory::cmd_factory_start(),
            FactoryAction::Stop => factory::cmd_factory_stop(),
            FactoryAction::Status => factory::cmd_factory_status(),
            FactoryAction::Uninstall => factory::cmd_factory_uninstall(),
        },
        Command::Icons { action } => match action {
            IconsAction::Sync => icons::cmd_icons_sync(&root),
            IconsAction::Watch => icons::cmd_icons_watch(&root),
            IconsAction::Install => icons::cmd_icons_install(&root),
            IconsAction::Uninstall => icons::cmd_icons_uninstall(),
        },
        Command::Promote { id } => cmd_promote(&root, &id),
        Command::Import { source, immediate } => cmd_import(&root, &source, immediate),
        Command::Index { print } => index::cmd_index(&root, print),
        Command::Bstorm { topic, team } => cmd_bstorm(&root, topic, team),
        Command::Events {
            card,
            json,
            limit,
            check,
        } => {
            if check {
                events::cmd_events_check(&root)
            } else {
                events::cmd_events(&root, card.as_deref(), json, limit)
            }
        }
    }
}

// ── poker ─────────────────────────────────────────────────────────────────────

// poker → poker.rs
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
            // Clean up stale worktree from a previous failed dispatch
            if stable_ws.exists() {
                eprintln!("[dispatcher] cleaning stale worktree for {}", card_id);
                let _ = StdCommand::new("git")
                    .args([
                        "worktree",
                        "remove",
                        stable_ws.to_string_lossy().as_ref(),
                        "--force",
                    ])
                    .current_dir(&git_root)
                    .status();
                if git_branch_exists(&git_root, &branch) {
                    let _ = StdCommand::new("git")
                        .args(["branch", "-D", branch.as_str()])
                        .current_dir(&git_root)
                        .status();
                }
                if stable_ws.exists() {
                    let _ = std::fs::remove_dir_all(&stable_ws);
                }
            }
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
            // Stable path outside the card bundle so it survives card state renames
            let workspaces_dir = repo_root.join(".workspaces");
            std::fs::create_dir_all(&workspaces_dir)?;
            let stable_ws = workspaces_dir.join(&ws_name);
            let legacy_ws = card_dir.join("workspace");
            let ws_path = if stable_ws.exists() {
                stable_ws
            } else if legacy_ws.exists() {
                legacy_ws
            } else {
                stable_ws
            };
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
    paths::ensure_cards_layout(cards_dir)?;
    providers::seed_providers(cards_dir)?;
    providers::ensure_mock_provider_command(cards_dir, adapter)?;
    let _dispatcher_lock = lock::acquire_dispatcher_lock(cards_dir)?;

    let pending_dir = cards_dir.join("pending");
    let running_dir = cards_dir.join("running");
    let done_dir = cards_dir.join("done");
    let failed_dir = cards_dir.join("failed");
    let stale_lease_after = std::cmp::max(
        lock::LEASE_STALE_FLOOR,
        Duration::from_millis(reap_ms.saturating_mul(3)),
    );

    let mut last_reap = std::time::Instant::now()
        .checked_sub(Duration::from_millis(reap_ms))
        .unwrap_or_else(std::time::Instant::now);

    let lineage_enabled = jobcard_core::lineage::is_enabled(cards_dir);

    loop {
        let mut lineage_events: Vec<jobcard_core::lineage::RunEvent> = Vec::new();
        let mut record = |meta: &jobcard_core::Meta, from: &str, to: &str| {
            if lineage_enabled {
                let et = jobcard_core::lineage::event_type_for(from, to);
                lineage_events.push(jobcard_core::lineage::build_run_event(et, meta, from, to));
            }
        };

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

        let running_count = fs::read_dir(&running_dir)
            .map(|rd| {
                rd.filter(|e| {
                    e.as_ref()
                        .ok()
                        .and_then(|e| e.path().extension().map(|x| x == "jobcard"))
                        .unwrap_or(false)
                })
                .count()
            })
            .unwrap_or(0);
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
                            let _ = paths::quarantine_invalid_pending_card(
                                &pending_path,
                                &failed_dir,
                                &reason,
                            );
                            continue;
                        }
                    };

                    // ── pre-dispatch gates ──────────────────────────────────
                    // Read meta before moving to running/ so we can skip
                    // cards that aren't ready yet.
                    if let Some(ref pre_meta) = meta {
                        // Gate 1: decision_required — needs human approval first
                        if pre_meta.decision_required {
                            eprintln!("[dispatcher] skipping {} — decision_required", pre_meta.id);
                            continue;
                        }
                        // Gate 2: depends_on — all deps must be in done/ or merged/
                        if !pre_meta.depends_on.is_empty() {
                            let blocked = pre_meta.depends_on.iter().any(|dep_id| {
                                !paths::card_exists_in(cards_dir, "done", dep_id)
                                    && !paths::card_exists_in(cards_dir, "merged", dep_id)
                            });
                            if blocked {
                                eprintln!(
                                    "[dispatcher] skipping {} — unmet depends_on: {:?}",
                                    pre_meta.id, pre_meta.depends_on
                                );
                                continue;
                            }
                        }
                    }

                    let running_path = running_dir.join(&name);
                    // Count before rename so the current card is not included in the tally
                    let active = count_running_cards(cards_dir);
                    if fs::rename(&pending_path, &running_path).is_err() {
                        continue;
                    }
                    if let Some(ref m) = meta {
                        record(m, "pending", "running");
                    }
                    quicklook::render_card_thumbnail(&running_path);

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
                            if let Some(ref m) = meta {
                                record(m, "running", "failed");
                            }
                            quicklook::render_card_thumbnail(&failed_path);
                            continue;
                        }
                    };
                    persist_workspace_meta(&mut meta, &running_path, vcs_engine, ws_info.as_ref());

                    // Assign deterministic zellij session name
                    if let Some(ref mut m) = meta {
                        if m.zellij_session.is_none() {
                            m.zellij_session = Some(format!("bop-{}", card_id));
                            let _ = write_meta(&running_path, m);
                        }
                    }

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

                    let (
                        provider_name,
                        provider_cmd,
                        rate_limit_exit,
                        provider_env,
                        provider_model,
                    ) = match providers::select_provider(cards_dir, meta.as_mut(), &stage)? {
                        Some(v) => v,
                        None => {
                            let pending_path = pending_dir.join(&name);
                            let _ = fs::rename(&running_path, &pending_path);
                            if let Some(ref m) = meta {
                                record(m, "running", "pending");
                            }
                            quicklook::render_card_thumbnail(&pending_path);
                            continue;
                        }
                    };

                    if let Some(ref mut meta) = meta {
                        let _ = write_meta(&running_path, meta);
                    }

                    let (exit_code, mut meta) = run_card(
                        cards_dir,
                        &running_path,
                        &provider_cmd,
                        &provider_name,
                        &provider_env,
                        provider_model.as_deref(),
                        rate_limit_exit,
                    )
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

                            providers::rotate_provider_chain(meta);
                            let _ =
                                providers::set_provider_cooldown(cards_dir, &provider_name, 300);
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
                    if let Some(ref m) = meta {
                        let to_state = if validation_triggered_fail {
                            "failed"
                        } else if exit_code == 0 {
                            "done"
                        } else if is_rate_limited {
                            "pending"
                        } else {
                            "failed"
                        };
                        record(m, "running", to_state);
                    }
                    quicklook::render_card_thumbnail(&target);
                    quicklook::compress_card(&target);
                    toast::macos_notify(&card_id, &target);
                    if exit_code == 0 && !validation_triggered_fail {
                        maybe_advance_stage(cards_dir, &target);
                        spawn_child_cards(cards_dir, &target);
                    }
                }
            }
        }

        // Flush collected lineage events (O(N) — one write per loop iteration)
        if !lineage_events.is_empty() {
            jobcard_core::lineage::flush_events(cards_dir, &lineage_events);
            lineage_events.clear();
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
        token: meta.token.clone(),
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

    // COW-copy spec.md from parent (APFS clone — zero disk cost until modified)
    let spec_src = done_card_dir.join("spec.md");
    if spec_src.exists() {
        if let Err(err) = paths::cow_copy_file(&spec_src, &next_card_dir.join("spec.md")) {
            eprintln!("[stage-advance] failed COW-copying spec.md: {err}");
        }
    }

    // COW-copy prompt.md template from parent
    let prompt_src = done_card_dir.join("prompt.md");
    if prompt_src.exists() {
        if let Err(err) = paths::cow_copy_file(&prompt_src, &next_card_dir.join("prompt.md")) {
            eprintln!("[stage-advance] failed COW-copying prompt.md: {err}");
        }
    }

    // Carry prior stage output: COW-copy done card's output/result.md → next card's output/prior_result.md
    let result_src = done_card_dir.join("output").join("result.md");
    if result_src.exists() {
        if let Err(err) = paths::cow_copy_file(
            &result_src,
            &next_card_dir.join("output").join("prior_result.md"),
        ) {
            eprintln!("[stage-advance] failed COW-copying output/prior_result.md: {err}");
        }
    }

    quicklook::render_card_thumbnail(&next_card_dir);
    eprintln!(
        "[stage-advance] {} ({}) → {} ({})",
        meta.id, meta.stage, next_id, next_stage
    );
}

// ── child-card-pipeline ───────────────────────────────────────────────────────

/// Convert a priority string ("must_have", "should_have", "could_have") to
/// numeric priority (1, 2, 3). Falls back to parsing as integer.
fn priority_from_yaml(value: &serde_yaml::Value) -> i64 {
    if let Some(n) = value.as_i64() {
        return n;
    }
    if let Some(s) = value.as_str() {
        match s.to_lowercase().replace(['-', ' '], "_").as_str() {
            "must" | "must_have" | "critical" => return 1,
            "should" | "should_have" | "important" => return 2,
            "could" | "could_have" | "nice_to_have" => return 3,
            _ => {}
        }
        if let Ok(n) = s.parse::<i64>() {
            return n;
        }
    }
    3
}

/// Build labels array from explicit labels + roadmap-specific fields
/// (phase, complexity, impact).
fn labels_from_yaml(entry: &serde_yaml::Value) -> Vec<serde_json::Value> {
    let mut labels: Vec<serde_json::Value> = Vec::new();

    // Explicit labels array
    if let Some(seq) = entry["labels"].as_sequence() {
        for item in seq {
            if let Some(name) = item.as_str() {
                labels.push(serde_json::json!({"name": name}));
            } else if let Some(name) = item["name"].as_str() {
                labels.push(serde_json::json!({
                    "name": name,
                    "kind": item["kind"].as_str()
                }));
            }
        }
    }

    // Phase → label
    if let Some(phase) = entry["phase"].as_str() {
        labels.push(serde_json::json!({"name": phase, "kind": "phase"}));
    }

    // Complexity → label
    if let Some(complexity) = entry["complexity"].as_str() {
        let display = match complexity.to_lowercase().as_str() {
            "low" => "Low Complexity",
            "medium" => "Medium Complexity",
            "high" => "High Complexity",
            _ => complexity,
        };
        labels.push(serde_json::json!({"name": display, "kind": "complexity"}));
    }

    // Impact → label
    if let Some(impact) = entry["impact"].as_str() {
        let display = match impact.to_lowercase().as_str() {
            "low" => "Low Impact",
            "medium" => "Medium Impact",
            "high" => "High Impact",
            _ => impact,
        };
        labels.push(serde_json::json!({"name": display, "kind": "impact"}));
    }

    labels
}

/// Build subtasks array from explicit subtasks + user_stories.
fn subtasks_from_yaml(entry: &serde_yaml::Value) -> Vec<serde_json::Value> {
    let mut subtasks: Vec<serde_json::Value> = Vec::new();

    // Explicit subtasks
    if let Some(seq) = entry["subtasks"].as_sequence() {
        for item in seq {
            if let Some(title) = item["title"].as_str() {
                subtasks.push(serde_json::json!({
                    "id": item["id"].as_str().unwrap_or(&format!("st-{}", subtasks.len() + 1)),
                    "title": title,
                    "done": item["done"].as_bool().unwrap_or(false)
                }));
            }
        }
    }

    // User stories → subtasks
    if let Some(seq) = entry["user_stories"].as_sequence() {
        for (i, story) in seq.iter().enumerate() {
            if let Some(text) = story.as_str() {
                subtasks.push(serde_json::json!({
                    "id": format!("us-{}", i + 1),
                    "title": text,
                    "done": false
                }));
            }
        }
    }

    subtasks
}

/// Build spec.md content from a YAML card entry.
///
/// Assembles sections: description, rationale, user stories, dependencies.
fn build_spec_md(entry: &serde_yaml::Value, id: &str) -> String {
    let title = entry["title"].as_str().unwrap_or(id);
    let mut spec = format!("# {}\n", title);

    if let Some(desc) = entry["description"].as_str() {
        spec.push_str(&format!("\n{}\n", desc));
    }

    if let Some(rationale) = entry["rationale"].as_str() {
        spec.push_str(&format!("\n## Rationale\n\n{}\n", rationale));
    }

    if let Some(stories) = entry["user_stories"].as_sequence() {
        let items: Vec<&str> = stories.iter().filter_map(|v| v.as_str()).collect();
        if !items.is_empty() {
            spec.push_str("\n## User Stories\n\n");
            for story in &items {
                spec.push_str(&format!("- {}\n", story));
            }
        }
    }

    if let Some(deps) = entry["depends_on"].as_sequence() {
        let items: Vec<&str> = deps.iter().filter_map(|v| v.as_str()).collect();
        if !items.is_empty() {
            spec.push_str("\n## Dependencies\n\n");
            for dep in &items {
                spec.push_str(&format!("- `{}`\n", dep));
            }
        }
    }

    if let Some(criteria) = entry["acceptance_criteria"].as_sequence() {
        let items: Vec<&str> = criteria.iter().filter_map(|v| v.as_str()).collect();
        if !items.is_empty() {
            spec.push_str("\n## Acceptance Criteria\n\n");
            for c in &items {
                spec.push_str(&format!("- [ ] {}\n", c));
            }
        }
    }

    spec
}

/// Create a .jobcard directory from a YAML entry. Used by both
/// `spawn_child_cards()` and `cmd_import()`.
fn create_card_from_yaml(dest_dir: &Path, entry: &serde_yaml::Value) -> Option<String> {
    let id = entry["id"].as_str()?;
    let child_dir = dest_dir.join(format!("{}.jobcard", id));
    if child_dir.exists() {
        return None; // don't overwrite
    }
    let _ = fs::create_dir_all(child_dir.join("logs"));
    let _ = fs::create_dir_all(child_dir.join("output"));

    let labels = labels_from_yaml(entry);
    let subtasks = subtasks_from_yaml(entry);

    let stage = entry["stage"].as_str().unwrap_or("spec");
    let workflow_mode = entry["workflow_mode"].as_str();

    let mut meta = serde_json::json!({
        "id": id,
        "title": entry["title"].as_str().unwrap_or(id),
        "description": entry["description"].as_str().unwrap_or(""),
        "stage": stage,
        "priority": priority_from_yaml(&entry["priority"]),
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

    // Add optional rich fields
    if !labels.is_empty() {
        meta["labels"] = serde_json::Value::Array(labels);
    }
    if !subtasks.is_empty() {
        meta["subtasks"] = serde_json::Value::Array(subtasks);
    }
    if let Some(wm) = workflow_mode {
        meta["workflow_mode"] = serde_json::Value::String(wm.to_string());
    }
    if let Some(deps) = entry["depends_on"].as_sequence() {
        let dep_ids: Vec<serde_json::Value> = deps
            .iter()
            .filter_map(|v| v.as_str().map(|s| serde_json::Value::String(s.to_string())))
            .collect();
        if !dep_ids.is_empty() {
            meta["depends_on"] = serde_json::Value::Array(dep_ids);
        }
    }

    let _ = fs::write(
        child_dir.join("meta.json"),
        serde_json::to_vec_pretty(&meta).unwrap(),
    );

    // Build rich spec.md
    let spec = build_spec_md(entry, id);
    let _ = fs::write(child_dir.join("spec.md"), spec);

    // Write roadmap.json snapshot if features are present
    if let Some(features) = entry["features"].as_sequence() {
        let roadmap_json = serde_json::json!({
            "features": features.iter().map(|f| {
                serde_json::json!({
                    "title": f["title"].as_str().unwrap_or(""),
                    "status": f["status"].as_str().unwrap_or("under_review"),
                    "priority": f["priority"].as_str().unwrap_or(""),
                    "phase": f["phase"].as_str().unwrap_or(""),
                })
            }).collect::<Vec<_>>()
        });
        let _ = fs::write(
            child_dir.join("output/roadmap.json"),
            serde_json::to_vec_pretty(&roadmap_json).unwrap(),
        );
    }

    quicklook::render_card_thumbnail(&child_dir);
    Some(id.to_string())
}

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

    // Read parent meta to determine child destination (pending or drafts).
    let dest = jobcard_core::read_meta(done_card_dir)
        .ok()
        .and_then(|m| m.spawn_to)
        .unwrap_or_else(|| "pending".to_string());
    let dest_dir = cards_dir.join(&dest);
    let _ = fs::create_dir_all(&dest_dir);

    for entry in entries {
        if let Some(id) = create_card_from_yaml(&dest_dir, &entry) {
            eprintln!("[child-cards] created {} in {}/", id, dest);
        }
    }
}

async fn run_card(
    cards_dir: &Path,
    card_dir: &Path,
    adapter: &str,
    provider_name: &str,
    provider_env: &std::collections::BTreeMap<String, String>,
    provider_model: Option<&str>,
    rate_limit_exit: i32,
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
        .map(memory::memory_namespace_from_meta)
        .unwrap_or_else(|| "default".to_string());
    if let Some(ref m) = meta {
        let mut ctx = jobcard_core::PromptContext::from_files(card_dir, m)?;
        match memory::read_memory_store(cards_dir, &memory_namespace) {
            Ok(store) => {
                ctx.memory = memory::format_memory_for_prompt(&store);
            }
            Err(err) => {
                let _ = util::append_log_line(
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
    let started_at_iso = started_at.to_rfc3339();
    let run_id = short_run_id();
    let mut run_idx: Option<usize> = None;
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

        let initial_model = provider_model
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .or_else(|| model_from_provider_env(provider_env))
            .unwrap_or_else(|| provider_name.to_string());
        m.runs.push(RunRecord {
            run_id: run_id.clone(),
            stage: stage.clone(),
            provider: provider_name.to_string(),
            model: initial_model,
            adapter: adapter.to_string(),
            started_at: started_at_iso,
            ended_at: None,
            outcome: "running".to_string(),
            prompt_tokens: None,
            completion_tokens: None,
            cost_usd: None,
            duration_s: None,
            note: None,
        });
        run_idx = Some(m.runs.len().saturating_sub(1));
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
        .envs(provider_env)
        // Card identity — lets any agent orient itself
        .env(
            "BOP_CARD_ID",
            meta.as_ref().map(|m| m.id.as_str()).unwrap_or(""),
        )
        .env("BOP_CARD_DIR", card_dir)
        .env("BOP_CARDS_DIR", cards_dir)
        .env("BOP_STAGE", &stage)
        .env("BOP_PROVIDER", provider_name)
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

    let mut lease = lock::RunLease {
        run_id: util::next_run_id(child.id()),
        pid,
        pid_start_time: started_at,
        started_at,
        heartbeat_at: started_at,
        host: util::host_name(),
    };
    let _ = lock::write_run_lease(card_dir, &lease);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_seconds);
    let mut timed_out = false;
    let status = loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            timed_out = true;
            let _ = child.kill().await;
            break None;
        }
        let remaining = deadline.saturating_duration_since(now);
        let wait_slice = std::cmp::min(lock::LEASE_HEARTBEAT_INTERVAL, remaining);
        match tokio::time::timeout(wait_slice, child.wait()).await {
            Ok(res) => break Some(res?),
            Err(_) => {
                lease.heartbeat_at = Utc::now();
                let _ = lock::write_run_lease(card_dir, &lease);
            }
        }
    };
    let exit_code = if timed_out {
        124
    } else {
        status.and_then(|s| s.code()).unwrap_or(1)
    };

    if let Err(err) = memory::merge_memory_output(cards_dir, &memory_namespace, &memory_out_file) {
        let _ = util::append_log_line(
            &stderr_log,
            &format!(
                "memory merge failed (namespace={}): {}",
                memory_namespace, err
            ),
        );
    }

    let finished_at = Utc::now();
    let duration_s = finished_at
        .signed_duration_since(started_at)
        .num_seconds()
        .try_into()
        .ok();
    let usage = detect_run_usage(card_dir);
    if let Some(ref mut m) = meta {
        let rec = m.stages.entry(stage).or_insert(jobcard_core::StageRecord {
            status: jobcard_core::StageStatus::Pending,
            agent: None,
            provider: None,
            duration_s: None,
            started: None,
            blocked_by: None,
        });
        rec.status = if timed_out {
            jobcard_core::StageStatus::Failed
        } else if exit_code == 0 {
            jobcard_core::StageStatus::Done
        } else if exit_code == rate_limit_exit {
            jobcard_core::StageStatus::Pending
        } else {
            jobcard_core::StageStatus::Failed
        };
        rec.duration_s = duration_s;

        if let Some(idx) = run_idx {
            if let Some(run) = m.runs.get_mut(idx) {
                run.ended_at = Some(finished_at.to_rfc3339());
                run.duration_s = duration_s;
                run.outcome = if timed_out {
                    "timeout".to_string()
                } else if exit_code == 0 {
                    "success".to_string()
                } else if exit_code == rate_limit_exit {
                    "rate_limited".to_string()
                } else {
                    "failed".to_string()
                };
                if run.model.trim().is_empty() {
                    run.model = provider_name.to_string();
                }
                if let Some(run_usage) = usage.as_ref() {
                    if let Some(model) = run_usage.model.as_ref() {
                        run.model = model.clone();
                    }
                    run.prompt_tokens = run_usage.prompt_tokens;
                    run.completion_tokens = run_usage.completion_tokens;
                    run.cost_usd = run_usage.cost_usd;
                } else if let Some(model) = detect_model_from_logs(card_dir) {
                    run.model = model;
                }
                if timed_out {
                    run.note = Some(format!("timeout_seconds={}", timeout_seconds));
                }
            }
        }
    }

    Ok((exit_code, meta))
}

#[derive(Debug, Clone, Default)]
struct RunUsage {
    model: Option<String>,
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    cost_usd: Option<f64>,
}

fn short_run_id() -> String {
    let now = Utc::now()
        .timestamp_nanos_opt()
        .unwrap_or_else(|| Utc::now().timestamp_micros() * 1000) as u64;
    let pid = std::process::id() as u64;
    let seq = util::RUN_ID_SEQ.fetch_add(1, Ordering::Relaxed);
    let mixed = now ^ (pid << 16) ^ seq;
    format!("{:08x}", (mixed & 0xffff_ffff) as u32)
}

fn model_from_provider_env(env: &BTreeMap<String, String>) -> Option<String> {
    for key in ["OLLAMA_MODEL", "ANTHROPIC_MODEL", "OPENAI_MODEL", "MODEL"] {
        if let Some(value) = env.get(key).map(|v| v.trim()).filter(|v| !v.is_empty()) {
            return Some(value.to_string());
        }
    }
    env.iter().find_map(|(k, v)| {
        if k.to_ascii_uppercase().ends_with("_MODEL") {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        None
    })
}

// inspect → inspect.rs
fn parse_usage_from_stdout(stdout_log: &Path) -> Option<RunUsage> {
    let value = inspect::parse_latest_json_line(stdout_log)?;
    let usage = value.get("usage");

    let mut prompt_tokens = usage
        .and_then(|u| u.get("input_tokens"))
        .and_then(|x| x.as_u64());
    let mut completion_tokens = usage
        .and_then(|u| u.get("output_tokens"))
        .and_then(|x| x.as_u64());

    let model = value
        .get("modelUsage")
        .and_then(|m| m.as_object())
        .and_then(|obj| obj.keys().next().cloned())
        .or_else(|| {
            value
                .get("model")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        });

    if prompt_tokens.is_none() || completion_tokens.is_none() {
        if let Some(model_usage) = value.get("modelUsage").and_then(|m| m.as_object()) {
            if let Some((_model_name, stats)) = model_usage.iter().next() {
                if prompt_tokens.is_none() {
                    prompt_tokens = stats.get("inputTokens").and_then(|x| x.as_u64());
                }
                if completion_tokens.is_none() {
                    completion_tokens = stats.get("outputTokens").and_then(|x| x.as_u64());
                }
            }
        }
    }

    Some(RunUsage {
        model,
        prompt_tokens,
        completion_tokens,
        cost_usd: value.get("total_cost_usd").and_then(|x| x.as_f64()),
    })
}

fn parse_usage_from_ollama_stats(stats_log: &Path) -> Option<RunUsage> {
    let content = fs::read_to_string(stats_log).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    Some(RunUsage {
        model: value
            .get("model")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        prompt_tokens: value.get("prompt_tokens").and_then(|x| x.as_u64()),
        completion_tokens: value.get("completion_tokens").and_then(|x| x.as_u64()),
        cost_usd: None,
    })
}

fn detect_run_usage(card_dir: &Path) -> Option<RunUsage> {
    let logs_dir = card_dir.join("logs");
    let mut merged = parse_usage_from_stdout(&logs_dir.join("stdout.log"));

    if let Some(ollama) = parse_usage_from_ollama_stats(&logs_dir.join("ollama-stats.json")) {
        if let Some(current) = merged.as_mut() {
            if current.model.is_none() {
                current.model = ollama.model;
            }
            if current.prompt_tokens.is_none() {
                current.prompt_tokens = ollama.prompt_tokens;
            }
            if current.completion_tokens.is_none() {
                current.completion_tokens = ollama.completion_tokens;
            }
        } else {
            merged = Some(ollama);
        }
    }

    merged
}

fn detect_model_from_logs(card_dir: &Path) -> Option<String> {
    detect_run_usage(card_dir).and_then(|u| u.model)
}

// rotate_provider_chain, select_provider, set_provider_cooldown → providers.rs

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
    paths::ensure_cards_layout(cards_dir)?;

    let done_dir = cards_dir.join("done");
    let merged_dir = cards_dir.join("merged");
    let failed_dir = cards_dir.join("failed");
    let mg_lineage_enabled = jobcard_core::lineage::is_enabled(cards_dir);

    loop {
        let mut mg_lineage_events: Vec<jobcard_core::lineage::RunEvent> = Vec::new();
        let mut mg_record = |meta: &jobcard_core::Meta, from: &str, to: &str| {
            if mg_lineage_enabled {
                let et = jobcard_core::lineage::event_type_for(from, to);
                mg_lineage_events.push(jobcard_core::lineage::build_run_event(et, meta, from, to));
            }
        };

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
                        quicklook::render_card_thumbnail(&failed_path);
                        // No meta available for lineage event here
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

                // Heal stale working copy before running ACs — concurrent jj ops
                // often leave workspaces stale, causing `jj log` to fail.
                if matches!(vcs_engine, VcsEngine::Jj) {
                    let _ = TokioCommand::new("jj")
                        .args(["workspace", "update-stale"])
                        .current_dir(&workdir)
                        .output()
                        .await;
                }

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
                    mg_record(&meta, "done", "failed");
                    quicklook::render_card_thumbnail(&failed_path);
                    continue;
                }

                if let Err(err) = policy::policy_check_card(cards_dir, &card_dir, &meta.id) {
                    meta.failure_reason = Some("policy_violation".to_string());
                    meta.policy_result = Some(format!("failed: {err}"));
                    let _ = fs::write(
                        card_dir.join("output").join("qa_report.md"),
                        format!("policy violation: {err}\n"),
                    );
                    let _ = write_meta(&card_dir, &meta);
                    let failed_path = failed_dir.join(&name);
                    let _ = fs::rename(&card_dir, &failed_path);
                    mg_record(&meta, "done", "failed");
                    quicklook::render_card_thumbnail(&failed_path);
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
                                mg_record(&meta, "done", "failed");
                                quicklook::render_card_thumbnail(&failed_path);
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
                            // push + PR are best-effort — skip gracefully when no remote
                            if vcs_err.is_none() {
                                let _ = jobcard_core::worktree::push_stack(&repo_root, "origin");
                            }
                            if vcs_err.is_none() {
                                let pr_result = StdCommand::new("gh")
                                    .args(["pr", "create", "--fill", "--draft"])
                                    .current_dir(&repo_root)
                                    .output();
                                // gh pr create is best-effort; no remote = skip silently
                                let _ = pr_result;
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
                        mg_record(&meta, "done", "failed");
                        quicklook::render_card_thumbnail(&failed_path);
                        continue;
                    }
                }

                // Best-effort: capture file-change manifest for Quick Look
                let branch = meta.change_ref.clone().unwrap_or_else(|| meta.id.clone());
                let _ = write_changes_json(&card_dir, &workdir, &branch).await;

                let _ = write_meta(&card_dir, &meta);
                let merged_path = merged_dir.join(&name);
                let _ = fs::rename(&card_dir, &merged_path);
                mg_record(&meta, "done", "merged");
                quicklook::compress_card(&merged_path);
                quicklook::render_card_thumbnail(&merged_path);
            }
        }

        // Flush collected lineage events (O(N) — one write per loop iteration)
        if !mg_lineage_events.is_empty() {
            jobcard_core::lineage::flush_events(cards_dir, &mg_lineage_events);
            mg_lineage_events.clear();
        }

        if once {
            break;
        }
        tokio::time::sleep(Duration::from_millis(poll_ms)).await;
    }

    Ok(())
}

// copy_dir_all → util.rs

// ── retry ────────────────────────────────────────────────────────────────────

fn cmd_retry(root: &Path, id: &str) -> anyhow::Result<()> {
    let message = retry_card(root, id)?;
    println!("{}", message);
    Ok(())
}

fn retry_card(root: &Path, id: &str) -> anyhow::Result<String> {
    let card = paths::find_card(root, id).with_context(|| format!("card not found: {}", id))?;
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
    if let Ok(m) = jobcard_core::read_meta(&target) {
        if jobcard_core::lineage::is_enabled(root) {
            let ev = jobcard_core::lineage::build_run_event(
                jobcard_core::lineage::EventType::Other,
                &m,
                state,
                "pending",
            );
            jobcard_core::lineage::flush_events(root, &[ev]);
        }
    }
    quicklook::render_card_thumbnail(&target);
    Ok(format!("retrying: {} -> pending/", id))
}

// ── kill ─────────────────────────────────────────────────────────────────────

async fn cmd_kill(root: &Path, id: &str) -> anyhow::Result<()> {
    let message = kill_card(root, id).await?;
    println!("{}", message);
    Ok(())
}

async fn kill_card(root: &Path, id: &str) -> anyhow::Result<String> {
    let card = paths::find_card(root, id).with_context(|| format!("card not found: {}", id))?;
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
    if let Ok(m) = jobcard_core::read_meta(&card) {
        if jobcard_core::lineage::is_enabled(root) {
            let ev = jobcard_core::lineage::build_run_event(
                jobcard_core::lineage::EventType::Abort,
                &m,
                "running",
                "failed",
            );
            jobcard_core::lineage::flush_events(root, &[ev]);
        }
    }
    fs::rename(&card, &target)
        .with_context(|| format!("failed to move card to failed/: {}", id))?;
    quicklook::render_card_thumbnail(&target);

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

// ── promote / import ─────────────────────────────────────────────────────────

// find_card_in_dir → paths.rs

fn cmd_promote(root: &Path, id: &str) -> anyhow::Result<()> {
    let drafts_dir = root.join("drafts");
    let pending_dir = root.join("pending");
    let _ = fs::create_dir_all(&pending_dir);

    if id == "all" {
        let mut count = 0u32;
        if let Ok(entries) = fs::read_dir(&drafts_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir()
                    && path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.ends_with(".jobcard"))
                {
                    let name = entry.file_name();
                    fs::rename(&path, pending_dir.join(&name))?;
                    eprintln!("promoted: {}", name.to_string_lossy());
                    count += 1;
                }
            }
        }
        if count == 0 {
            anyhow::bail!("no draft cards to promote");
        }
        eprintln!("[promote] moved {} card(s) to pending/", count);
    } else {
        let card = paths::find_card_in_dir(&drafts_dir, id)
            .with_context(|| format!("card '{}' not found in drafts/", id))?;
        let name = card.file_name().unwrap().to_owned();
        fs::rename(&card, pending_dir.join(&name))?;
        eprintln!("promoted: {}", id);
    }
    Ok(())
}

fn cmd_bstorm(root: &Path, topic_words: Vec<String>, team: Option<String>) -> anyhow::Result<()> {
    let topic = topic_words.join(" ");
    if topic.trim().is_empty() {
        anyhow::bail!("topic cannot be empty — usage: bop bstorm <topic words>");
    }

    // Slugify: lowercase, non-alnum→hyphen, collapse runs, trim hyphens, cap at 40.
    let slug: String = topic
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let slug: String = slug
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug = &slug[..slug.len().min(40)];

    let spec_body = format!(
        "# Brainstorm: {topic}\n\n\
         Explore ideas, trade-offs, and approaches for the topic above.\n\
         Produce a structured summary of the best options.\n"
    );

    let card_dir = create_card(root, "ideation", slug, Some(&spec_body), team.as_deref())?;
    let display = card_dir.strip_prefix(root).unwrap_or(&card_dir);
    println!("✓ {}", display.display());
    println!("  edit spec: {}/spec.md", card_dir.display());
    Ok(())
}

// events → events.rs
fn cmd_import(root: &Path, source: &str, immediate: bool) -> anyhow::Result<()> {
    let text = fs::read_to_string(source).with_context(|| format!("cannot read {}", source))?;
    let entries: Vec<serde_yaml::Value> = serde_yaml::from_str(&text)
        .context("invalid YAML — expected a sequence of card definitions")?;

    let dest = if immediate { "pending" } else { "drafts" };
    let dest_dir = root.join(dest);
    fs::create_dir_all(&dest_dir)?;

    let mut count = 0u32;
    for entry in &entries {
        if entry["id"].as_str().is_none() {
            continue;
        }
        match create_card_from_yaml(&dest_dir, entry) {
            Some(id) => {
                eprintln!("[import] created {}", id);
                count += 1;
            }
            None => {
                if let Some(id) = entry["id"].as_str() {
                    eprintln!("[import] skipping {} (already exists)", id);
                }
            }
        }
    }

    eprintln!("[import] created {} card(s) in {}/", count, dest);
    Ok(())
}

fn approve_card(root: &Path, id: &str) -> anyhow::Result<()> {
    let card = paths::find_card(root, id).with_context(|| format!("card not found: {}", id))?;
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
    quicklook::render_card_thumbnail(&card);
    println!("Approved {}", id);
    Ok(())
}

// ── logs ─────────────────────────────────────────────────────────────────────

// logs → logs.rs
// find_card_in_state → paths.rs

// inspect → inspect.rs
// index → index.rs
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

    let card = paths::find_card(root, id).with_context(|| format!("card not found: {}", id))?;
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

// card_exists_in, find_card → paths.rs

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
