use anyhow::Context;
use bop_core::VcsEngine as CoreVcsEngine;
use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::path::PathBuf;

#[allow(dead_code)]
mod acplan;
mod bridge;
mod cards;
mod colors;
mod diff;
mod dispatcher;
mod doctor;
mod events;
mod export;
mod factory;
mod gantt;
mod icons;
mod index;
mod inspect;
mod list;
mod lock;
mod logs;
mod memory;
mod merge_gate;
mod paths;
mod poker;
mod policy;
mod pool;
mod power;
mod project;
mod providers;
mod quicklook;
mod reaper;
mod render;
mod replay;
mod serve;
mod stats;
mod termcaps;
mod ui;
mod util;
mod watch;
mod webhook;
mod workspace;

#[derive(Parser, Debug)]
#[command(name = "bop")]
struct Cli {
    /// Project root path or registered alias (sets cards root to <project>/.cards).
    #[arg(short = 'p', long, global = true)]
    project: Option<String>,

    /// Legacy explicit cards directory override (hidden; kept for compatibility/tests).
    #[arg(long, hide = true, global = true)]
    cards_dir: Option<String>,

    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    Init,
    New {
        template: Option<String>,
        id: Option<String>,
        /// Team for glyph suit assignment (cli, arch, quality, platform).
        /// Auto-detected from card directory if omitted.
        #[arg(long)]
        team: Option<String>,
    },
    Status {
        #[arg(default_value = "")]
        id: String,
        /// Watch for changes and update status automatically.
        #[arg(short, long)]
        watch: bool,
    },
    Validate {
        id: String,
        /// Run realtime feed validation on the job's output records.
        #[arg(long)]
        realtime: bool,
    },
    Dispatcher {
        #[arg(short = 'a', long, default_value = "adapters/mock.nu")]
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
    /// Retry cards that failed with transient errors (rate limits, timeouts).
    RetryTransient {
        /// Card ID to retry (if omitted with --all, retries all transient failures).
        id: Option<String>,
        /// Retry all cards with transient failures.
        #[arg(long)]
        all: bool,
    },
    /// Pause a running card, preventing it from being dispatched until resumed.
    Pause {
        id: String,
    },
    /// Resume a paused card, allowing it to be dispatched again.
    Resume {
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
    /// Show what a card changed (git diff, output, or open its worktree).
    Diff {
        id: String,
        /// Show one-line summary (files changed/insertions/deletions).
        #[arg(long, conflicts_with_all = ["output", "worktree"])]
        stat: bool,
        /// Show output/result.md instead of git diff.
        #[arg(long, conflicts_with = "worktree")]
        output: bool,
        /// Open the card worktree in $EDITOR.
        #[arg(long, conflicts_with = "output")]
        worktree: bool,
    },
    /// Reconstruct card timeline from logs/events.jsonl.
    Replay {
        /// Card ID to replay.
        id: Option<String>,
        /// Emit machine-readable JSON array.
        #[arg(long)]
        json: bool,
        /// Show only error/retry events.
        #[arg(long)]
        errors: bool,
        /// Show relative timestamps (0s, +1m 43s, ...).
        #[arg(long)]
        relative: bool,
        /// Merge all card event logs from last 24h.
        #[arg(long, conflicts_with = "id")]
        all: bool,
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
        /// Emit newline-delimited JSON (ndjson) instead of ANSI table.
        /// One JSON object per line: Meta fields + "state" + "team".
        /// Pipe to: jq, nu, fzf, rg for filtering and selection.
        #[arg(long)]
        json: bool,
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
    Doctor {
        /// Skip time-consuming checks (e.g. make check).
        #[arg(long)]
        fast: bool,
        /// Auto-repair safe issues (missing state dirs, missing providers.json).
        #[arg(long)]
        fix: bool,
    },
    /// Generate shell completion script.
    Completions {
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
    /// Send test payloads to configured outbound webhooks.
    Webhook {
        #[command(subcommand)]
        action: WebhookAction,
    },
    /// Promote cards from drafts/ to pending/, making them eligible for dispatch.
    Promote {
        /// Card ID, or "all" to promote every draft.
        id: String,
    },
    /// Import cards from a JSON file into drafts/ (or pending/ with --immediate).
    Import {
        /// Path to tarball (`.tar.gz`) or legacy JSON cards file.
        source: String,
        /// Legacy JSON mode only: import directly to pending/ instead of drafts/.
        #[arg(long)]
        immediate: bool,
        /// Overwrite existing card without interactive confirmation.
        #[arg(long)]
        force: bool,
    },
    /// Export a card bundle tarball for sharing/replay/import.
    Export {
        /// Card ID to export.
        id: String,
        /// Output tarball path (default: ./bop-export-<id>-<timestamp>.tar.gz).
        #[arg(long)]
        out: Option<String>,
        /// Exclude logs/ from archive.
        #[arg(long)]
        strip_logs: bool,
        /// Exclude worktree/ from archive (default behavior).
        #[arg(long)]
        strip_worktree: bool,
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
    /// Show a Gantt timeline of card runs (ANSI by default, --html for browser).
    Gantt {
        /// Output HTML file instead of ANSI terminal chart.
        #[arg(long)]
        html: bool,
        /// Open the HTML file in the default browser (implies --html).
        #[arg(long, short = 'o')]
        open: bool,
        /// Override terminal width (auto-detected from pane/terminal).
        #[arg(long, short = 'w')]
        width: Option<usize>,
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
    /// Clean up old cards from done/ and failed/ directories.
    Clean {
        /// Show what would be deleted without actually deleting.
        #[arg(long)]
        dry_run: bool,
        /// Only delete cards older than N days.
        #[arg(long)]
        older_than: Option<u64>,
        /// Target state directory (done, failed, or both).
        #[arg(long)]
        state: Option<String>,
    },
    /// Start an HTTP server that accepts card creation requests via POST /cards/new.
    Serve {
        /// Host to bind (default: 127.0.0.1).
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// Port to listen on (default: 8080).
        #[arg(long, default_value_t = 8080)]
        port: u16,
    },
    /// Install filesystem event hooks for event-driven merge-gate.
    InstallHooks {
        /// Remove hooks instead of installing them.
        #[arg(long)]
        uninstall: bool,
    },
    /// Scan running/ for orphaned cards (dead PIDs, corrupt meta.json) and move them to pending/.
    Recover,
    /// Interactive TUI kanban board for card management.
    Ui,
    /// Show detected AI providers and their quota/usage.
    Providers {
        /// Refresh every --interval seconds (default 60).
        #[arg(short, long)]
        watch: bool,
        /// Emit machine-parseable JSON object instead of ANSI table.
        #[arg(long)]
        json: bool,
        /// Poll interval in seconds for --watch mode.
        /// Falls back to $BOP_PROVIDER_POLL_INTERVAL, then 60.
        #[arg(long)]
        interval: Option<u64>,
    },
    /// Session state bridge — connects AI CLI events to BopDeck.
    Bridge {
        #[command(subcommand)]
        sub: bridge::BridgeSubcommand,
    },
    /// Historical cost/token report across completed cards.
    Stats {
        /// Group rows by provider or by day.
        #[arg(long, value_enum)]
        by: Option<stats::StatsBy>,
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
        /// Show detail for a single card id.
        #[arg(long)]
        card: Option<String>,
    },
    /// Unified live dashboard for running + done cards with log tail.
    Watch {
        /// Aggregate running/done cards across all registered projects.
        #[arg(long)]
        all: bool,
    },
    /// Manage the multi-project registry (~/.bop/projects.json).
    Project {
        #[command(subcommand)]
        action: ProjectAction,
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
    /// Manage a pre-warmed QEMU VM pool for low-latency dispatch.
    Pool {
        /// Start or resize pool to this number of idle VMs.
        #[arg(long)]
        size: Option<usize>,
        /// Optional action: status, stop, monitor, lease, release.
        action: Option<String>,
        /// Card id for internal lease/release actions.
        #[arg(long)]
        card_id: Option<String>,
        /// Lease timeout (seconds) for internal lease action.
        #[arg(long, default_value_t = 300)]
        timeout_s: u64,
        /// Slot id for internal release action.
        #[arg(long)]
        slot: Option<usize>,
        /// Exit code metadata for internal release action.
        #[arg(long, default_value_t = 0)]
        exit_code: i32,
    },
    /// Unload and remove the launchd plist files.
    Uninstall,
}

#[derive(Subcommand, Debug)]
enum IconsAction {
    /// Set icons on every .bop in .cards/ right now (batch).
    Sync,
    /// Watch .cards/ with FSEvents and update icons as cards move (foreground).
    Watch,
    /// Install a launchd WatchPaths agent that runs `bop icons sync` on changes.
    Install,
    /// Unload and remove the icon-watcher launchd agent.
    Uninstall,
}

#[derive(Subcommand, Debug)]
enum WebhookAction {
    /// Send a test payload to every configured webhook endpoint.
    Test,
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
        /// Print JSON result.
        #[arg(long)]
        json: bool,
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

#[derive(Subcommand, Debug)]
enum ProjectAction {
    /// Register a project root for alias-based selection.
    Add {
        /// Path to the project root (contains .cards/).
        path: String,
        /// Optional short alias (e.g. "b", "e", "z").
        #[arg(long)]
        alias: Option<String>,
    },
    /// Show all registered projects.
    List,
    /// Unregister a project by name, alias, or path.
    Remove {
        /// Project name (or alias/path) to remove.
        target: String,
    },
}

fn resolve_config_path(project_root: &Path) -> PathBuf {
    if let Ok(p) = std::env::var("BOP_CONFIG") {
        return PathBuf::from(p);
    }
    project_root.join(".bop").join("config.json")
}

fn project_root_from_cards_root(cards_root: &Path) -> PathBuf {
    if cards_root
        .file_name()
        .is_some_and(|name| name == std::ffi::OsStr::new(".cards"))
    {
        return cards_root
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn load_config_for_cards_root(cards_root: &Path) -> anyhow::Result<bop_core::Config> {
    use bop_core::config::{merge_configs, read_config_file, Config};

    let global = match bop_core::config::global_config_path() {
        Some(path) if path.exists() => read_config_file(&path)
            .with_context(|| format!("global config error: {}", path.display()))?,
        _ => Config::default(),
    };

    let project_path = project_root_from_cards_root(cards_root)
        .join(".bop")
        .join("config.json");
    let project = if project_path.exists() {
        read_config_file(&project_path)
            .with_context(|| format!("project config error: {}", project_path.display()))?
    } else {
        Config::default()
    };

    Ok(merge_configs(global, project))
}

fn resolve_cards_root(
    project_arg: Option<&str>,
    cards_dir_arg: Option<&str>,
) -> anyhow::Result<PathBuf> {
    if let Some(cards_dir) = cards_dir_arg {
        return Ok(PathBuf::from(cards_dir));
    }

    if let Some(project) = project_arg {
        let project_root = project::find_project(project)?;
        return Ok(project_root.join(".cards"));
    }

    Ok(PathBuf::from(".cards"))
}

#[cfg(target_os = "macos")]
fn install_hooks_macos(cards_root: &std::path::Path, uninstall: bool) -> anyhow::Result<()> {
    if uninstall {
        factory::cmd_factory_uninstall()
    } else {
        factory::cmd_factory_install(cards_root)
    }
}

#[cfg(target_os = "linux")]
fn install_hooks_linux(cards_root: &std::path::Path, uninstall: bool) -> anyhow::Result<()> {
    if uninstall {
        factory::cmd_factory_uninstall()
    } else {
        factory::cmd_factory_install(cards_root)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let Cli {
        project: project_arg,
        cards_dir: cards_dir_arg,
        cmd,
    } = cli;

    let cmd = match cmd {
        Command::Project { action } => {
            return match action {
                ProjectAction::Add { path, alias } => {
                    project::cmd_project_add(&path, alias.as_deref())
                }
                ProjectAction::List => project::cmd_project_list(),
                ProjectAction::Remove { target } => project::cmd_project_remove(&target),
            };
        }
        other => other,
    };

    let root = resolve_cards_root(project_arg.as_deref(), cards_dir_arg.as_deref())?;
    let project_root = project_root_from_cards_root(&root);

    // Load merged global+project config (missing files silently skipped)
    let cfg = load_config_for_cards_root(&root).unwrap_or_default();

    match cmd {
        Command::Init => {
            paths::ensure_cards_layout(&root)?;
            cards::seed_default_templates(&root)?;
            providers::seed_providers(&root)?;

            // Create config with sensible defaults if it doesn't exist
            let config_path = resolve_config_path(&project_root);
            if !config_path.exists() {
                let defaults = bop_core::Config {
                    default_provider_chain: Some(vec!["mock".to_string()]),
                    max_concurrent: Some(1),
                    cooldown_seconds: Some(300),
                    log_retention_days: Some(30),
                    default_template: Some("implement".to_string()),
                    dispatch: None,
                    webhooks: None,
                };
                bop_core::config::write_config_file(&config_path, &defaults).with_context(
                    || {
                        format!(
                            "failed to create default config at {}",
                            config_path.display()
                        )
                    },
                )?;
            }

            // Detect Zellij session and write .cards/config.json
            let zellij_session = std::env::var("ZELLIJ_SESSION_NAME").ok();
            let cards_config_path = root.join("config.json");
            let cards_config = bop_core::CardsConfig {
                zellij_session: zellij_session.clone(),
            };
            bop_core::write_cards_config_file(&cards_config_path, &cards_config).with_context(
                || {
                    format!(
                        "failed to write .cards/config.json at {}",
                        cards_config_path.display()
                    )
                },
            )?;

            if let Some(session_name) = zellij_session {
                println!(
                    "{}✓ Zellij session detected: {}{}",
                    colors::state_ansi("done"),
                    session_name,
                    colors::RESET
                );
            } else {
                println!(
                    "{}ℹ  Not running inside Zellij. For live card links, start bop inside a{}",
                    colors::state_ansi("pending"),
                    colors::RESET
                );
                println!("   Zellij session: zellij --session bop");
                println!("   Then re-run: bop init");
            }

            // Print quick-start guide
            println!();
            println!(
                "{}✓ bop workspace ready at .cards/{}",
                colors::state_ansi("done"),
                colors::RESET
            );
            println!();
            println!("Quick start:");
            println!("  bop new agent my-task       # create a card");
            println!("  bop list                    # see all cards");
            println!("  bop dispatcher --once       # run one card");
            println!("  bop gantt                   # timeline view");
            println!();
            println!("Docs: https://github.com/ryanmaclean/bop");

            Ok(())
        }
        Command::New { template, id, team } => {
            let created = cards::cmd_new(
                &root,
                template.as_deref(),
                id.as_deref(),
                team.as_deref(),
                &cfg,
            )?;
            println!(
                "{}✓ Card created → bop dispatch {} to run it{}",
                colors::state_ansi("done"),
                created.id,
                colors::RESET
            );
            Ok(())
        }
        Command::Status { id, watch } => {
            if id.trim().is_empty() {
                if watch {
                    return list::list_cards_watch(&root, "active").await;
                } else {
                    return doctor::print_status_summary(&root);
                }
            }

            let card = paths::require_card(&root, &id)?;
            let state = card
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .unwrap_or("unknown");
            let meta = bop_core::read_meta(&card)?;
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
            let _ = bop_core::read_meta(&card)?;
            if realtime {
                let summary = dispatcher::validate_realtime_output(&card)?;
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
            // Validate adapter exists before running dispatcher
            let adapter_path = if std::path::Path::new(&adapter).is_absolute() {
                PathBuf::from(&adapter)
            } else {
                std::env::current_dir()?.join(&adapter)
            };

            if !adapter_path.exists() {
                anyhow::bail!(
                    "adapter not found: {}\n\nHint: Check available adapters in the adapters/ directory, or use 'ls adapters/*.nu' to list-adapters",
                    adapter
                );
            }

            let effective_max_workers = max_workers.or(cfg.max_concurrent).unwrap_or(1);
            let effective_max_retries = max_retries.unwrap_or(3);
            dispatcher::run_dispatcher(
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
        } => merge_gate::run_merge_gate(&root, poll_ms, once, vcs_engine).await,
        Command::Retry { id } => cards::cmd_retry(&root, &id),
        Command::RetryTransient { id, all } => {
            cards::cmd_retry_transient(&root, id.as_deref(), all)
        }
        Command::Pause { id } => cards::cmd_pause(&root, &id).await,
        Command::Resume { id } => cards::cmd_resume(&root, &id).await,
        Command::Kill { id } => cards::cmd_kill(&root, &id).await,
        Command::Approve { id } => cards::cmd_approve(&root, &id),
        Command::Logs { id, follow } => logs::cmd_logs(&root, &id, follow).await,
        Command::Diff {
            id,
            stat,
            output,
            worktree,
        } => diff::cmd_diff(&root, &id, stat, output, worktree),
        Command::Replay {
            id,
            json,
            errors,
            relative,
            all,
        } => replay::cmd_replay(
            &root,
            id.as_deref(),
            replay::ReplayOptions {
                json,
                errors,
                relative,
                all,
            },
        ),
        Command::Inspect { id } => inspect::cmd_inspect(&root, &id),
        Command::List { state, json } => {
            if json {
                list::list_cards_json(&root, &state, &mut std::io::stdout())
            } else {
                list::list_cards(&root, &state)
            }
        }
        Command::Meta { action } => match action {
            MetaAction::Set {
                id,
                workflow_mode,
                step_index,
                clear_workflow_mode,
                clear_step_index,
            } => cards::cmd_meta_set(
                &root,
                &id,
                workflow_mode.as_deref(),
                step_index,
                clear_workflow_mode,
                clear_step_index,
            ),
        },
        Command::Policy { action } => match action {
            PolicyAction::Check { id, staged, json } => {
                policy::cmd_policy_check(&root, id.as_deref(), staged, json)
            }
        },
        Command::Doctor { fast, fix } => doctor::cmd_doctor(&root, fast, fix),
        Command::Completions { shell } => {
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
            FactoryAction::Pool {
                size,
                action,
                card_id,
                timeout_s,
                slot,
                exit_code,
            } => match action.as_deref() {
                None => {
                    if let Some(pool_size) = size {
                        factory::cmd_factory_pool_size(&root, pool_size)
                    } else {
                        factory::cmd_factory_pool_status(&root)
                    }
                }
                Some("status") => factory::cmd_factory_pool_status(&root),
                Some("stop") => factory::cmd_factory_pool_stop(&root),
                Some("monitor") => factory::cmd_factory_pool_monitor(&root),
                Some("lease") => {
                    let cid = card_id
                        .as_deref()
                        .context("--card-id is required for `bop factory pool lease`")?;
                    factory::cmd_factory_pool_lease(&root, cid, timeout_s)
                }
                Some("release") => {
                    let slot = slot.context("--slot is required for `bop factory pool release`")?;
                    factory::cmd_factory_pool_release(&root, slot, card_id.as_deref(), exit_code)
                }
                Some(other) => anyhow::bail!(
                    "unknown pool action '{}'; expected one of: status, stop, lease, release, monitor",
                    other
                ),
            },
            FactoryAction::Uninstall => factory::cmd_factory_uninstall(),
        },
        Command::Icons { action } => match action {
            IconsAction::Sync => icons::cmd_icons_sync(&root),
            IconsAction::Watch => icons::cmd_icons_watch(&root),
            IconsAction::Install => icons::cmd_icons_install(&root),
            IconsAction::Uninstall => icons::cmd_icons_uninstall(),
        },
        Command::Webhook { action } => match action {
            WebhookAction::Test => webhook::cmd_webhook_test(&root).await,
        },
        Command::Promote { id } => cards::cmd_promote(&root, &id),
        Command::Import {
            source,
            immediate,
            force,
        } => {
            let source_path = PathBuf::from(&source);
            if export::is_tarball_path(&source_path) {
                export::cmd_import_bundle(&root, &source_path, force)
            } else {
                if force {
                    anyhow::bail!("--force is only supported when importing a tarball");
                }
                cards::cmd_import(&root, &source, immediate)
            }
        }
        Command::Export {
            id,
            out,
            strip_logs,
            strip_worktree,
        } => {
            let out_path = out.as_deref().map(PathBuf::from);
            export::cmd_export(&root, &id, out_path.as_deref(), strip_logs, strip_worktree)
        }
        Command::Index { print } => index::cmd_index(&root, print),
        Command::Bstorm { topic, team } => cards::cmd_bstorm(&root, topic, team),
        Command::Gantt { html, open, width } => gantt::cmd_gantt(&root, html || open, open, width),
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
        Command::Clean {
            dry_run,
            older_than,
            state,
        } => cards::cmd_clean(&root, dry_run, older_than, state),
        Command::Serve { host, port } => serve::run_serve(root, &host, port).await,
        Command::InstallHooks { uninstall } => {
            #[cfg(target_os = "macos")]
            {
                install_hooks_macos(&root, uninstall)?;
            }
            #[cfg(target_os = "linux")]
            {
                install_hooks_linux(&root, uninstall)?;
            }
            #[cfg(not(any(target_os = "macos", target_os = "linux")))]
            {
                anyhow::bail!("install-hooks is only supported on macOS and Linux");
            }
            Ok(())
        }
        Command::Recover => {
            let running_dir = root.join("running");
            let pending_dir = root.join("pending");

            eprintln!(
                "{}bop recover: scanning .cards/running/ for orphaned cards...{}",
                colors::state_ansi("pending"),
                colors::RESET
            );

            let recovered = reaper::recover_orphans(&running_dir, &pending_dir).await?;

            if recovered.is_empty() {
                println!(
                    "{}bop recover: nothing to recover.{}",
                    colors::state_ansi("done"),
                    colors::RESET
                );
            } else {
                for card_id in &recovered {
                    println!(
                        "{}✓ recovered: {} → pending/{}",
                        colors::state_ansi("done"),
                        card_id,
                        colors::RESET
                    );
                }
                println!(
                    "{}{} card(s) recovered.{}",
                    colors::state_ansi("done"),
                    recovered.len(),
                    colors::RESET
                );
            }

            Ok(())
        }
        Command::Ui => ui::run_ui(&root).await,
        Command::Providers {
            watch,
            json,
            interval,
        } => providers::cmd_providers(watch, json, interval).await,
        Command::Bridge { sub } => bridge::cmd_bridge(sub).await,
        Command::Stats { by, json, card } => stats::cmd_stats(&root, by, json, card.as_deref()),
        Command::Watch { all } => watch::cmd_watch(&root, all).await,
        Command::Project { .. } => unreachable!("project command handled before cards root resolution"),
    }
}
