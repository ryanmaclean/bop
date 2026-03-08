pub mod bopdeck;
pub mod claude;
pub mod codex;
pub mod gemini;
pub mod history;
pub mod ollama;
pub mod opencode;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::future::Future;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use bop_core::Meta;

fn default_probe() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterConfig {
    pub command: String,
    #[serde(default)]
    pub rate_limit_exit: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooldown_until_epoch_s: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Extra environment variables injected when spawning this provider's adapter.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Connectivity probe enabled (defaults to true). Set to false to disable for air-gapped deployments.
    #[serde(default = "default_probe", skip_serializing_if = "is_true")]
    pub probe: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProvidersFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_provider: Option<String>,
    #[serde(default)]
    pub providers: BTreeMap<String, AdapterConfig>,
}

pub type ProviderSelection = (
    String,
    String,
    i32,
    BTreeMap<String, String>,
    Option<String>,
);

fn validate_provider(name: &str, p: &AdapterConfig) -> anyhow::Result<()> {
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

pub fn read_providers(cards_dir: &Path) -> anyhow::Result<ProvidersFile> {
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

pub fn write_providers(cards_dir: &Path, pf: &ProvidersFile) -> anyhow::Result<()> {
    for (name, provider) in &pf.providers {
        validate_provider(name, provider)?;
    }
    let bytes = serde_json::to_vec_pretty(pf)?;

    let target = providers_path(cards_dir);
    let tmp = target.with_extension("json.tmp");

    fs::write(&tmp, &bytes)?;
    fs::rename(&tmp, &target)?;

    Ok(())
}

pub fn seed_providers(cards_dir: &Path) -> anyhow::Result<()> {
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
        AdapterConfig {
            command: "adapters/mock.nu".to_string(),
            rate_limit_exit: 75,
            cooldown_until_epoch_s: None,
            model: None,
            env: Default::default(),
            probe: true,
        },
    );
    pf.providers.insert(
        "mock2".to_string(),
        AdapterConfig {
            command: "adapters/mock.nu".to_string(),
            rate_limit_exit: 75,
            cooldown_until_epoch_s: None,
            model: None,
            env: Default::default(),
            probe: true,
        },
    );
    write_providers(cards_dir, &pf)?;
    Ok(())
}

pub fn ensure_mock_provider_command(cards_dir: &Path, adapter: &str) -> anyhow::Result<()> {
    let mut pf = read_providers(cards_dir)?;
    if let Some(p) = pf.providers.get_mut("mock") {
        p.command = adapter.to_string();
    }
    write_providers(cards_dir, &pf)?;
    Ok(())
}

pub fn rotate_provider_chain(meta: &mut Meta) {
    if meta.provider_chain.len() <= 1 {
        return;
    }
    let first = meta.provider_chain.remove(0);
    meta.provider_chain.push(first);
}

pub fn select_provider(
    cards_dir: &Path,
    meta: Option<&mut Meta>,
    stage: &str,
) -> anyhow::Result<Option<ProviderSelection>> {
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

    let mut fallback: Option<ProviderSelection> = None;
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
                    fallback = Some((
                        name,
                        p.command.clone(),
                        p.rate_limit_exit,
                        p.env.clone(),
                        p.model.clone(),
                    ));
                }
                continue;
            }
        }

        return Ok(Some((
            name,
            p.command.clone(),
            p.rate_limit_exit,
            p.env.clone(),
            p.model.clone(),
        )));
    }

    Ok(fallback)
}

pub fn set_provider_cooldown(
    cards_dir: &Path,
    provider: &str,
    cooldown_s: i64,
) -> anyhow::Result<()> {
    let mut pf = read_providers(cards_dir)?;
    let now = Utc::now().timestamp();
    if let Some(p) = pf.providers.get_mut(provider) {
        p.cooldown_until_epoch_s = Some(now + cooldown_s);
    }
    write_providers(cards_dir, &pf)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Provider quota/usage types (spec 030)
// ---------------------------------------------------------------------------

/// A point-in-time snapshot of a provider's quota and usage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSnapshot {
    /// Short identifier, e.g. "claude", "codex", "gemini", "ollama".
    pub provider: String,
    /// Human-readable name shown in tables, e.g. "Claude Code".
    pub display_name: String,
    /// Primary usage percentage (0-100), `None` if provider is not quota-based.
    pub primary_pct: Option<u8>,
    /// Secondary usage percentage (0-100).
    pub secondary_pct: Option<u8>,
    /// Label for primary metric, e.g. "5h", "requests".
    pub primary_label: Option<String>,
    /// Label for secondary metric, e.g. "7d".
    pub secondary_label: Option<String>,
    /// Cumulative tokens consumed (if available).
    pub tokens_used: Option<u64>,
    /// Cumulative cost in USD (if available).
    pub cost_usd: Option<f64>,
    /// When the primary quota window resets.
    pub reset_at: Option<DateTime<Utc>>,
    /// How the data was obtained: "oauth", "rpc", "pty", "http", "log".
    pub source: String,
    /// Non-fatal error message (e.g. "token expired").
    pub error: Option<String>,
    /// Currently loaded models for non-quota-based providers (e.g. Ollama).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loaded_models: Option<Vec<String>>,
}

/// Trait implemented by each provider's quota monitor.
///
/// `detect()` is synchronous — it only checks for local credentials or
/// configuration files. `fetch()` is async and may make network calls.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Short identifier matching `ProviderSnapshot::provider`.
    fn name(&self) -> &str;

    /// Returns `true` if credentials or a running server are found locally.
    fn detect(&self) -> bool;

    /// Fetch current quota/usage from the provider. On transient failures,
    /// return a snapshot with `error` set rather than propagating the error.
    async fn fetch(&self) -> anyhow::Result<ProviderSnapshot>;
}

/// Returns all registered provider implementations.
///
/// Add new providers here as they are implemented.
pub fn all_providers() -> Vec<Box<dyn Provider>> {
    vec![
        Box::new(claude::ClaudeProvider::new()),
        Box::new(codex::CodexProvider::new()),
        Box::new(gemini::GeminiProvider::new()),
        Box::new(ollama::OllamaLocalProvider::new()),
        Box::new(ollama::OllamaCloudProvider::new()),
        Box::new(opencode::OpenCodeProvider::new()),
    ]
}

/// Detects which providers are available on this machine by calling
/// `detect()` on every registered provider and keeping only those that
/// return `true`.
pub fn detect_all_providers() -> Vec<Box<dyn Provider>> {
    all_providers().into_iter().filter(|p| p.detect()).collect()
}

/// Runs an async detect probe from synchronous `Provider::detect()`.
///
/// We execute the future on a dedicated thread/runtime so provider detect can
/// use async HTTP with `tokio::time::timeout` without requiring callers to
/// provide an async context.
pub(crate) fn run_detect_async<F>(future: F) -> bool
where
    F: Future<Output = bool> + Send + 'static,
{
    std::thread::spawn(move || {
        let Ok(rt) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return false;
        };
        rt.block_on(future)
    })
    .join()
    .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// cmd_providers — CLI entry point
// ---------------------------------------------------------------------------

use crate::colors::{BOLD, DIM, RESET};

const DEFAULT_PROVIDER_POLL_INTERVAL_S: u64 = 60;
const PROVIDER_POLL_INTERVAL_ENV: &str = "BOP_PROVIDER_POLL_INTERVAL";

/// Format a percentage as a bar with ANSI styling: `▓▓▓▓▓░░░░░ 57%`
fn pct_bar(pct: Option<u8>, width: usize) -> String {
    match pct {
        Some(p) => {
            let filled = ((p as usize) * width + 50) / 100; // round
            let empty = width.saturating_sub(filled);
            let color = if p >= 90 {
                "\x1b[38;5;160m" // red
            } else if p >= 70 {
                "\x1b[38;5;172m" // amber
            } else {
                "\x1b[38;5;71m" // green
            };
            format!(
                "{}{}{}{}  {:>3}%",
                color,
                "█".repeat(filled),
                "░".repeat(empty),
                RESET,
                p
            )
        }
        None => format!("{}{:>width$}{}", DIM, "—", RESET, width = width + 5),
    }
}

/// Format a reset time as a human-readable relative duration.
fn format_reset(reset: Option<&DateTime<Utc>>) -> String {
    match reset {
        Some(t) => {
            let now = Utc::now();
            if *t <= now {
                "now".to_string()
            } else {
                let diff = *t - now;
                let total_mins = diff.num_minutes();
                if total_mins < 1 {
                    format!("in {}s", diff.num_seconds())
                } else if total_mins < 60 {
                    format!("in {}m", total_mins)
                } else {
                    let h = total_mins / 60;
                    let m = total_mins % 60;
                    format!("in {}h {}m", h, m)
                }
            }
        }
        None => format!("{}—{}", DIM, RESET),
    }
}

fn format_non_quota_detail(snap: &ProviderSnapshot) -> Option<String> {
    if let Some(models) = snap.loaded_models.as_ref() {
        if models.is_empty() {
            return Some(format!("{}(idle){}", DIM, RESET));
        }
        return Some(format!("{} loaded: {}", models.len(), models.join(", ")));
    }

    if snap.tokens_used.is_none() && snap.cost_usd.is_none() {
        return None;
    }

    let mut parts: Vec<String> = Vec::new();
    if let Some(tokens) = snap.tokens_used {
        parts.push(format!("{tokens} tok"));
    }
    if let Some(cost) = snap.cost_usd {
        parts.push(format!("${cost:.4}"));
    }
    Some(parts.join("  "))
}

/// Render an ANSI table of provider snapshots.
fn format_providers_table(snapshots: &[ProviderSnapshot]) -> String {
    let mut out = String::new();

    if snapshots.is_empty() {
        out.push_str(&format!("{}No providers detected.{}\n", DIM, RESET));
        out.push('\n');
        out.push_str("Hint: Install Claude Code and authenticate with OAuth to see quota.\n");
        return out;
    }

    // Infer header labels from first snapshot with labels, or use defaults
    let (primary_header, secondary_header) = snapshots
        .iter()
        .find_map(|s| {
            s.primary_label
                .as_ref()
                .zip(s.secondary_label.as_ref())
                .map(|(p, s)| (p.as_str(), s.as_str()))
        })
        .unwrap_or(("5h", "7d"));

    // Header
    out.push_str(&format!(
        "{}Provider         Source    {:<11} {:<11} Reset{}",
        BOLD, primary_header, secondary_header, RESET
    ));
    out.push('\n');
    out.push_str(&format!(
        "{}─────────────────────────────────────────────────────────────{}",
        DIM, RESET
    ));
    out.push('\n');

    for snap in snapshots {
        let name = if snap.error.is_some() {
            format!(
                "{}{:<16}{}",
                "\x1b[38;5;172m", // amber for warning
                snap.display_name,
                RESET
            )
        } else {
            format!("{:<16}", snap.display_name)
        };

        let source = format!("{:<8}", snap.source);
        let primary = pct_bar(snap.primary_pct, 8);
        let secondary = pct_bar(snap.secondary_pct, 8);
        let reset = if snap.primary_pct.is_none() {
            format_non_quota_detail(snap).unwrap_or_else(|| format_reset(snap.reset_at.as_ref()))
        } else {
            format_reset(snap.reset_at.as_ref())
        };

        out.push_str(&format!(
            "{}  {}  {}  {}  {}",
            name, source, primary, secondary, reset
        ));
        out.push('\n');

        if let Some(ref err) = snap.error {
            out.push_str(&format!("  {}⚠  {}{}\n", DIM, err, RESET));
        }
    }

    out
}

fn render_snapshots(snapshots: &[ProviderSnapshot], json: bool) -> anyhow::Result<String> {
    if json {
        let out = serde_json::to_string_pretty(snapshots)?;
        return Ok(format!("{out}\n"));
    }
    Ok(format_providers_table(snapshots))
}

fn redraw_in_place(output: &str, prev_line_count: &mut usize) {
    if *prev_line_count > 0 {
        print!("\x1b[{}A\x1b[J", *prev_line_count);
    }

    print!("{output}");
    io::stdout().flush().ok();
    *prev_line_count = output.lines().count().max(1);
}

fn resolve_poll_interval(cli_interval: Option<u64>) -> u64 {
    let cli = cli_interval.filter(|v| *v > 0);
    if let Some(interval) = cli {
        return interval;
    }

    std::env::var(PROVIDER_POLL_INTERVAL_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_PROVIDER_POLL_INTERVAL_S)
}

fn error_snapshot(provider_name: &str, error: String) -> ProviderSnapshot {
    ProviderSnapshot {
        provider: provider_name.to_string(),
        display_name: provider_name.to_string(),
        primary_pct: None,
        secondary_pct: None,
        primary_label: None,
        secondary_label: None,
        tokens_used: None,
        cost_usd: None,
        reset_at: None,
        source: "error".to_string(),
        error: Some(error),
        loaded_models: None,
    }
}

async fn fetch_provider_snapshot(
    provider: &dyn Provider,
    history_path: Option<&Path>,
    bopdeck_writer: Option<&bopdeck::BopDeckWriter>,
) -> ProviderSnapshot {
    let snapshot = match provider.fetch().await {
        Ok(snap) => snap,
        Err(err) => error_snapshot(provider.name(), err.to_string()),
    };

    // Successful snapshots are persisted for trend views/sparklines.
    if snapshot.error.is_none() {
        if let Some(path) = history_path {
            let _ = history::append_history(path, &snapshot);
        }
    }

    // BopDeck consumes one OpenLineage event per provider snapshot.
    if let Some(writer) = bopdeck_writer {
        let _ = writer.emit(&snapshot).await;
    }

    snapshot
}

async fn fetch_provider_snapshots(
    providers: &[Box<dyn Provider>],
    history_path: Option<&Path>,
    bopdeck_writer: Option<&bopdeck::BopDeckWriter>,
) -> Vec<ProviderSnapshot> {
    let mut snapshots = Vec::with_capacity(providers.len());
    for provider in providers {
        let snapshot =
            fetch_provider_snapshot(provider.as_ref(), history_path, bopdeck_writer).await;
        snapshots.push(snapshot);
    }
    snapshots
}

fn sort_snapshots(snapshots: &mut [ProviderSnapshot], provider_order: &[String]) {
    snapshots.sort_by_key(|snap| {
        provider_order
            .iter()
            .position(|provider_name| provider_name == &snap.provider)
            .unwrap_or(usize::MAX)
    });
}

fn upsert_snapshot(
    snapshots: &mut Vec<ProviderSnapshot>,
    incoming: ProviderSnapshot,
    provider_order: &[String],
) {
    if let Some(existing) = snapshots
        .iter_mut()
        .find(|snap| snap.provider == incoming.provider)
    {
        *existing = incoming;
    } else {
        snapshots.push(incoming);
    }
    sort_snapshots(snapshots, provider_order);
}

async fn run_provider_watch_task(
    provider: Box<dyn Provider>,
    poll_interval: Duration,
    history_path: Option<PathBuf>,
    bopdeck_writer: bopdeck::BopDeckWriter,
    tx: tokio::sync::mpsc::UnboundedSender<ProviderSnapshot>,
) {
    loop {
        let snapshot = fetch_provider_snapshot(
            provider.as_ref(),
            history_path.as_deref(),
            Some(&bopdeck_writer),
        )
        .await;

        if tx.send(snapshot).is_err() {
            return;
        }

        tokio::time::sleep(poll_interval).await;
    }
}

/// `bop providers` command handler.
///
/// Detects installed providers, fetches their quota/usage, appends to
/// history, and displays the results as an ANSI table or JSON.
pub async fn cmd_providers(watch: bool, json: bool, interval: Option<u64>) -> anyhow::Result<()> {
    let mut providers = detect_all_providers();
    let provider_order: Vec<String> = providers
        .iter()
        .map(|provider| provider.name().to_string())
        .collect();

    let history_path = history::history_path();
    if let Some(path) = history_path.as_deref() {
        history::prepare_history(path)?;
    }
    let bopdeck_writer = bopdeck::BopDeckWriter::new();

    if !watch {
        let mut snapshots =
            fetch_provider_snapshots(&providers, history_path.as_deref(), Some(&bopdeck_writer))
                .await;
        sort_snapshots(&mut snapshots, &provider_order);
        let output = render_snapshots(&snapshots, json)?;
        print!("{output}");
        return Ok(());
    }

    if providers.is_empty() {
        let output = render_snapshots(&[], json)?;
        print!("{output}");
        return Ok(());
    }

    let poll_interval = Duration::from_secs(resolve_poll_interval(interval));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut tasks = Vec::with_capacity(providers.len());

    for provider in providers.drain(..) {
        let tx = tx.clone();
        let history_path = history_path.clone();
        let writer = bopdeck_writer.clone();
        tasks.push(tokio::spawn(run_provider_watch_task(
            provider,
            poll_interval,
            history_path,
            writer,
            tx,
        )));
    }
    drop(tx);

    let mut snapshots = Vec::with_capacity(provider_order.len());
    let mut line_count = 0usize;

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                for task in &tasks {
                    task.abort();
                }
                println!("\nShutting down...");
                return Ok(());
            }
            update = rx.recv() => {
                let Some(snapshot) = update else {
                    return Ok(());
                };
                upsert_snapshot(&mut snapshots, snapshot, &provider_order);
                let frame = render_snapshots(&snapshots, json)?;
                redraw_in_place(&frame, &mut line_count);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bop_core::{Meta, StageRecord, StageStatus};
    use std::sync::Mutex;
    use tempfile::tempdir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn mock_provider(command: &str) -> AdapterConfig {
        AdapterConfig {
            command: command.to_string(),
            rate_limit_exit: 75,
            cooldown_until_epoch_s: None,
            model: None,
            env: Default::default(),
            probe: true,
        }
    }

    #[test]
    fn resolve_poll_interval_prefers_cli_value() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var(PROVIDER_POLL_INTERVAL_ENV, "120");
        assert_eq!(resolve_poll_interval(Some(15)), 15);
        std::env::remove_var(PROVIDER_POLL_INTERVAL_ENV);
    }

    #[test]
    fn resolve_poll_interval_uses_env_when_cli_missing() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var(PROVIDER_POLL_INTERVAL_ENV, "45");
        assert_eq!(resolve_poll_interval(None), 45);
        std::env::remove_var(PROVIDER_POLL_INTERVAL_ENV);
    }

    #[test]
    fn resolve_poll_interval_falls_back_to_default() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var(PROVIDER_POLL_INTERVAL_ENV);
        assert_eq!(
            resolve_poll_interval(None),
            DEFAULT_PROVIDER_POLL_INTERVAL_S
        );
    }

    #[test]
    fn validate_provider_accepts_valid() {
        let p = mock_provider("adapters/mock.nu");
        assert!(validate_provider("mock", &p).is_ok());
    }

    #[test]
    fn validate_provider_rejects_empty_command() {
        let p = mock_provider("");
        assert!(validate_provider("mock", &p).is_err());
    }

    #[test]
    fn validate_provider_rejects_whitespace_command() {
        let p = mock_provider("   ");
        assert!(validate_provider("mock", &p).is_err());
    }

    #[test]
    fn validate_provider_rejects_empty_name() {
        let p = mock_provider("adapters/mock.nu");
        assert!(validate_provider("", &p).is_err());
    }

    #[test]
    fn providers_path_returns_correct_path() {
        let dir = Path::new("/tmp/cards");
        assert_eq!(
            providers_path(dir),
            PathBuf::from("/tmp/cards/providers.json")
        );
    }

    #[test]
    fn read_write_providers_roundtrip() {
        let td = tempdir().unwrap();
        let mut pf = ProvidersFile {
            default_provider: Some("mock".to_string()),
            ..Default::default()
        };
        pf.providers
            .insert("mock".to_string(), mock_provider("adapters/mock.nu"));
        write_providers(td.path(), &pf).unwrap();

        let read_back = read_providers(td.path()).unwrap();
        assert_eq!(read_back.default_provider, Some("mock".to_string()));
        assert!(read_back.providers.contains_key("mock"));
        assert_eq!(read_back.providers["mock"].command, "adapters/mock.nu");
    }

    #[test]
    fn read_providers_returns_default_for_missing_file() {
        let td = tempdir().unwrap();
        let pf = read_providers(td.path()).unwrap();
        assert!(pf.providers.is_empty());
        assert!(pf.default_provider.is_none());
    }

    #[test]
    fn read_providers_rejects_invalid_entry() {
        let td = tempdir().unwrap();
        let json = serde_json::json!({
            "providers": {
                "bad": { "command": "" }
            }
        });
        fs::write(
            td.path().join("providers.json"),
            serde_json::to_vec(&json).unwrap(),
        )
        .unwrap();
        assert!(read_providers(td.path()).is_err());
    }

    #[test]
    fn seed_providers_creates_file() {
        let td = tempdir().unwrap();
        seed_providers(td.path()).unwrap();
        assert!(td.path().join("providers.json").exists());
        let pf = read_providers(td.path()).unwrap();
        assert!(pf.providers.contains_key("mock"));
        assert!(pf.providers.contains_key("mock2"));
        assert_eq!(pf.default_provider, Some("mock".to_string()));
    }

    #[test]
    fn seed_providers_is_idempotent() {
        let td = tempdir().unwrap();
        seed_providers(td.path()).unwrap();
        // Modify the file to detect if seed_providers overwrites
        let mut pf = read_providers(td.path()).unwrap();
        pf.default_provider = Some("changed".to_string());
        write_providers(td.path(), &pf).unwrap();
        seed_providers(td.path()).unwrap();
        // Should not have been overwritten
        let pf2 = read_providers(td.path()).unwrap();
        assert_eq!(pf2.default_provider, Some("changed".to_string()));
    }

    #[test]
    fn ensure_mock_provider_command_updates_mock() {
        let td = tempdir().unwrap();
        seed_providers(td.path()).unwrap();
        ensure_mock_provider_command(td.path(), "adapters/new.nu").unwrap();
        let pf = read_providers(td.path()).unwrap();
        assert_eq!(pf.providers["mock"].command, "adapters/new.nu");
    }

    #[test]
    fn rotate_provider_chain_moves_first_to_last() {
        let mut meta = Meta {
            provider_chain: vec!["a".into(), "b".into(), "c".into()],
            ..Default::default()
        };
        rotate_provider_chain(&mut meta);
        assert_eq!(meta.provider_chain, vec!["b", "c", "a"]);
    }

    #[test]
    fn rotate_provider_chain_noop_on_single() {
        let mut meta = Meta {
            provider_chain: vec!["only".into()],
            ..Default::default()
        };
        rotate_provider_chain(&mut meta);
        assert_eq!(meta.provider_chain, vec!["only"]);
    }

    #[test]
    fn rotate_provider_chain_noop_on_empty() {
        let mut meta = Meta {
            provider_chain: vec![],
            ..Default::default()
        };
        rotate_provider_chain(&mut meta);
        assert!(meta.provider_chain.is_empty());
    }

    #[test]
    fn select_provider_returns_first_available() {
        let td = tempdir().unwrap();
        let mut pf = ProvidersFile::default();
        pf.providers.insert("a".to_string(), mock_provider("cmd_a"));
        pf.providers.insert("b".to_string(), mock_provider("cmd_b"));
        write_providers(td.path(), &pf).unwrap();

        let mut meta = Meta {
            provider_chain: vec!["a".into(), "b".into()],
            ..Default::default()
        };
        let result = select_provider(td.path(), Some(&mut meta), "implement").unwrap();
        assert!(result.is_some());
        let (name, cmd, _, _, _) = result.unwrap();
        assert_eq!(name, "a");
        assert_eq!(cmd, "cmd_a");
    }

    #[test]
    fn select_provider_skips_cooled_down() {
        let td = tempdir().unwrap();
        let future = Utc::now().timestamp() + 9999;
        let mut pf = ProvidersFile::default();
        let mut cooled = mock_provider("cmd_a");
        cooled.cooldown_until_epoch_s = Some(future);
        pf.providers.insert("a".to_string(), cooled);
        pf.providers.insert("b".to_string(), mock_provider("cmd_b"));
        write_providers(td.path(), &pf).unwrap();

        let mut meta = Meta {
            provider_chain: vec!["a".into(), "b".into()],
            ..Default::default()
        };
        let result = select_provider(td.path(), Some(&mut meta), "implement").unwrap();
        let (name, _, _, _, _) = result.unwrap();
        assert_eq!(name, "b");
    }

    #[test]
    fn select_provider_returns_none_when_all_cooled_down() {
        let td = tempdir().unwrap();
        let future = Utc::now().timestamp() + 9999;
        let mut pf = ProvidersFile::default();
        let mut a = mock_provider("cmd_a");
        a.cooldown_until_epoch_s = Some(future);
        let mut b = mock_provider("cmd_b");
        b.cooldown_until_epoch_s = Some(future);
        pf.providers.insert("a".to_string(), a);
        pf.providers.insert("b".to_string(), b);
        write_providers(td.path(), &pf).unwrap();

        let mut meta = Meta {
            provider_chain: vec!["a".into(), "b".into()],
            ..Default::default()
        };
        let result = select_provider(td.path(), Some(&mut meta), "implement").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn select_provider_avoids_qa_provider() {
        let td = tempdir().unwrap();
        let mut pf = ProvidersFile::default();
        pf.providers
            .insert("impl_prov".to_string(), mock_provider("cmd_impl"));
        pf.providers
            .insert("qa_prov".to_string(), mock_provider("cmd_qa"));
        write_providers(td.path(), &pf).unwrap();

        let mut stages = BTreeMap::new();
        stages.insert(
            "implement".to_string(),
            StageRecord {
                status: StageStatus::Done,
                agent: None,
                provider: Some("impl_prov".to_string()),
                duration_s: None,
                started: None,
                blocked_by: None,
            },
        );
        let mut meta = Meta {
            provider_chain: vec!["impl_prov".into(), "qa_prov".into()],
            stages,
            ..Default::default()
        };
        let result = select_provider(td.path(), Some(&mut meta), "qa").unwrap();
        let (name, _, _, _, _) = result.unwrap();
        assert_eq!(name, "qa_prov");
    }

    #[test]
    fn select_provider_falls_back_to_avoided_if_only_option() {
        let td = tempdir().unwrap();
        let mut pf = ProvidersFile::default();
        pf.providers
            .insert("only_prov".to_string(), mock_provider("cmd"));
        write_providers(td.path(), &pf).unwrap();

        let mut stages = BTreeMap::new();
        stages.insert(
            "implement".to_string(),
            StageRecord {
                status: StageStatus::Done,
                agent: None,
                provider: Some("only_prov".to_string()),
                duration_s: None,
                started: None,
                blocked_by: None,
            },
        );
        let mut meta = Meta {
            provider_chain: vec!["only_prov".into()],
            stages,
            ..Default::default()
        };
        let result = select_provider(td.path(), Some(&mut meta), "qa").unwrap();
        let (name, _, _, _, _) = result.unwrap();
        assert_eq!(name, "only_prov");
    }

    #[test]
    fn set_provider_cooldown_sets_epoch() {
        let td = tempdir().unwrap();
        seed_providers(td.path()).unwrap();
        set_provider_cooldown(td.path(), "mock", 300).unwrap();
        let pf = read_providers(td.path()).unwrap();
        let cd = pf.providers["mock"].cooldown_until_epoch_s.unwrap();
        let now = Utc::now().timestamp();
        assert!(cd > now);
        assert!(cd <= now + 301);
    }
}
