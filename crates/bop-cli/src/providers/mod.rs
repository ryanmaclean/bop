pub mod bopdeck;
pub mod claude;
pub mod codex;
pub mod gemini;
pub mod history;
pub mod ollama;
pub mod opencode;

use anyhow::Context;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::future::Future;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSelection {
    pub name: String,
    pub command: String,
    pub rate_limit_exit: i32,
    pub env: BTreeMap<String, String>,
    pub model: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DispatchProviderConfig {
    pub auto_select_provider: bool,
    pub quota_block_threshold: f64,
    pub prefer_cheap_provider: Option<String>,
}

impl Default for DispatchProviderConfig {
    fn default() -> Self {
        Self {
            auto_select_provider: true,
            quota_block_threshold: 0.90,
            prefer_cheap_provider: None,
        }
    }
}

impl DispatchProviderConfig {
    pub fn from_dispatch_config(cfg: Option<&bop_core::config::DispatchConfig>) -> Self {
        let mut out = Self::default();
        if let Some(cfg) = cfg {
            if let Some(auto) = cfg.auto_select_provider {
                out.auto_select_provider = auto;
            }
            if let Some(threshold) = cfg.quota_block_threshold {
                let normalized = if threshold > 1.0 {
                    threshold / 100.0
                } else {
                    threshold
                };
                out.quota_block_threshold = normalized.clamp(0.0, 1.0);
            }
            out.prefer_cheap_provider = cfg
                .prefer_cheap_provider
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
        }
        out
    }
}

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

#[derive(Debug, Clone, Default)]
pub struct ProviderCache {
    snapshots: Vec<ProviderSnapshot>,
    updated_at: Option<Instant>,
}

impl ProviderCache {
    pub fn get_cached(&self, max_age: Duration) -> Option<Vec<ProviderSnapshot>> {
        let updated_at = self.updated_at?;
        if updated_at.elapsed() <= max_age {
            return Some(self.snapshots.clone());
        }
        None
    }

    pub fn refresh(&mut self) -> anyhow::Result<Vec<ProviderSnapshot>> {
        self.refresh_with(refresh_provider_snapshots)
    }

    pub fn get_or_refresh(&mut self, max_age: Duration) -> anyhow::Result<Vec<ProviderSnapshot>> {
        if let Some(cached) = self.get_cached(max_age) {
            return Ok(cached);
        }
        self.refresh()
    }

    fn refresh_with<F>(&mut self, fetch: F) -> anyhow::Result<Vec<ProviderSnapshot>>
    where
        F: FnOnce() -> anyhow::Result<Vec<ProviderSnapshot>>,
    {
        let snapshots = fetch()?;
        self.snapshots = snapshots.clone();
        self.updated_at = Some(Instant::now());
        Ok(snapshots)
    }

    #[cfg(test)]
    fn get_or_refresh_with<F>(
        &mut self,
        max_age: Duration,
        fetch: F,
    ) -> anyhow::Result<Vec<ProviderSnapshot>>
    where
        F: FnOnce() -> anyhow::Result<Vec<ProviderSnapshot>>,
    {
        if let Some(cached) = self.get_cached(max_age) {
            return Ok(cached);
        }
        self.refresh_with(fetch)
    }
}

fn provider_cache() -> &'static Mutex<ProviderCache> {
    static CACHE: OnceLock<Mutex<ProviderCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(ProviderCache::default()))
}

fn run_provider_future<F, T>(future: F) -> anyhow::Result<T>
where
    F: Future<Output = anyhow::Result<T>> + Send + 'static,
    T: Send + 'static,
{
    let handle = std::thread::spawn(move || -> anyhow::Result<T> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        rt.block_on(future)
    });

    handle
        .join()
        .map_err(|_| anyhow::anyhow!("provider quota refresh thread panicked"))?
}

fn refresh_provider_snapshots() -> anyhow::Result<Vec<ProviderSnapshot>> {
    let providers = detect_all_providers();
    if providers.is_empty() {
        return Ok(Vec::new());
    }

    run_provider_future(async move {
        let provider_order: Vec<String> = providers
            .iter()
            .map(|provider| provider.name().to_string())
            .collect();
        let mut snapshots = fetch_provider_snapshots(&providers, None, None).await;
        sort_snapshots(&mut snapshots, &provider_order);
        Ok(snapshots)
    })
}

fn preferred_rank(provider: &str, cost_tier: u8, cheap_provider: Option<&str>) -> u8 {
    let is_cheap = cheap_provider.map(|p| p == provider).unwrap_or(false)
        || provider == "ollama-local"
        || provider == "ollama";

    match cost_tier {
        1 => {
            if is_cheap {
                0
            } else {
                1
            }
        }
        2 => {
            if is_cheap || provider == "codex" {
                0
            } else {
                1
            }
        }
        3.. => {
            if provider == "codex" || provider == "claude" {
                0
            } else if is_cheap {
                2
            } else {
                1
            }
        }
        _ => 1,
    }
}

fn reorder_by_cost_tier(providers: &mut [String], cost_tier: u8, cheap_provider: Option<&str>) {
    providers.sort_by_key(|provider| preferred_rank(provider, cost_tier, cheap_provider));
}

fn quota_utilization(snapshots: &[ProviderSnapshot], provider: &str) -> Option<f64> {
    snapshots.iter().find_map(|snap| {
        if snap.provider != provider {
            return None;
        }

        let mut max_pct: Option<f64> = None;
        if let Some(primary) = snap.primary_pct {
            max_pct = Some(f64::from(primary) / 100.0);
        }
        if let Some(secondary) = snap.secondary_pct {
            let secondary = f64::from(secondary) / 100.0;
            max_pct = Some(max_pct.map(|p| p.max(secondary)).unwrap_or(secondary));
        }
        max_pct
    })
}

fn meta_cost_tier(meta: Option<&Meta>) -> u8 {
    let Some(meta) = meta else {
        return 2;
    };

    if let Some(cost) = meta.cost {
        return cost.max(1);
    }

    meta.priority
        .and_then(|priority| u8::try_from(priority).ok())
        .map(|p| p.max(1))
        .unwrap_or(2)
}

fn build_selection(name: &str, config: &AdapterConfig, reason: String) -> ProviderSelection {
    ProviderSelection {
        name: name.to_string(),
        command: config.command.clone(),
        rate_limit_exit: config.rate_limit_exit,
        env: config.env.clone(),
        model: config.model.clone(),
        reason,
    }
}

fn lock_provider_cache() -> std::sync::MutexGuard<'static, ProviderCache> {
    match provider_cache().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub fn load_dispatch_provider_config(cards_dir: &Path) -> anyhow::Result<DispatchProviderConfig> {
    let base = bop_core::load_config().unwrap_or_default();
    let cards_local_path = cards_dir.join(".bop").join("config.json");
    let merged = if cards_local_path.exists() {
        let local = bop_core::config::read_config_file(&cards_local_path)
            .with_context(|| format!("cards-local config error: {}", cards_local_path.display()))?;
        bop_core::config::merge_configs(base, local)
    } else {
        base
    };

    Ok(DispatchProviderConfig::from_dispatch_config(
        merged.dispatch.as_ref(),
    ))
}

pub fn select_provider(
    cards_dir: &Path,
    meta: Option<&mut Meta>,
    stage: &str,
    cfg: &DispatchProviderConfig,
) -> anyhow::Result<Option<ProviderSelection>> {
    let pf = read_providers(cards_dir)?;
    let now = Utc::now().timestamp();
    let cost_tier = meta_cost_tier(meta.as_deref());

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

    let quota_snapshots = if cfg.auto_select_provider {
        let mut cache = lock_provider_cache();
        match cache.get_or_refresh(Duration::from_secs(60)) {
            Ok(snapshots) => snapshots,
            Err(err) => {
                eprintln!("[providers] quota refresh failed: {err}");
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    let mut eligible: Vec<String> = Vec::new();
    let mut fallback: Option<String> = None;

    for name in chain {
        let Some(p) = pf.providers.get(&name) else {
            continue;
        };
        if let Some(until) = p.cooldown_until_epoch_s {
            if until > now {
                continue;
            }
        }

        if cfg.auto_select_provider {
            if let Some(utilization) = quota_utilization(&quota_snapshots, &name) {
                if utilization >= cfg.quota_block_threshold {
                    continue;
                }
            }
        }

        if let Some(ref avoid) = avoid_provider {
            if &name == avoid {
                if fallback.is_none() {
                    fallback = Some(name.clone());
                }
                continue;
            }
        }

        eligible.push(name);
    }

    if eligible.is_empty() {
        if let Some(fallback_name) = fallback {
            if let Some(provider) = pf.providers.get(&fallback_name) {
                return Ok(Some(build_selection(
                    &fallback_name,
                    provider,
                    format!(
                        "stage={} fallback to implementation provider because no alternative was eligible",
                        stage
                    ),
                )));
            }
        }
        return Ok(None);
    }

    let mut ordered = eligible.clone();
    if cfg.auto_select_provider {
        reorder_by_cost_tier(
            &mut ordered,
            cost_tier,
            cfg.prefer_cheap_provider.as_deref(),
        );
    }
    let selected_name = ordered.remove(0);
    let Some(provider) = pf.providers.get(&selected_name) else {
        return Ok(None);
    };

    let utilization_note = quota_utilization(&quota_snapshots, &selected_name)
        .map(|u| {
            format!(
                ", quota={:.0}%<{:.0}%",
                u * 100.0,
                cfg.quota_block_threshold * 100.0
            )
        })
        .unwrap_or_default();

    let reason = if cfg.auto_select_provider {
        let reorder_note = if eligible.first() != Some(&selected_name) {
            ", reordered_by_cost_tier=true"
        } else {
            ""
        };
        format!(
            "stage={}, cost_tier={}{}{}, cooldown=ready",
            stage, cost_tier, utilization_note, reorder_note
        )
    } else {
        format!("stage={}, auto_select_provider=false", stage)
    };

    Ok(Some(build_selection(&selected_name, provider, reason)))
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

fn normalize_utilization_label(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut prev_underscore = false;

    for ch in label.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_underscore = false;
        } else if !prev_underscore {
            out.push('_');
            prev_underscore = true;
        }
    }

    out.trim_matches('_').to_string()
}

fn reset_in_secs(reset: Option<&DateTime<Utc>>) -> Option<i64> {
    let t = reset?;
    let now = Utc::now();
    Some((*t - now).num_seconds().max(0))
}

fn snapshot_json_entry(snap: &ProviderSnapshot) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    obj.insert(
        "name".to_string(),
        serde_json::Value::String(snap.provider.clone()),
    );

    if let (Some(label), Some(pct)) = (snap.primary_label.as_deref(), snap.primary_pct) {
        let label_key = normalize_utilization_label(label);
        if !label_key.is_empty() {
            obj.insert(
                format!("utilization_{label_key}"),
                serde_json::json!(f64::from(pct) / 100.0),
            );
        }
    }

    if let (Some(label), Some(pct)) = (snap.secondary_label.as_deref(), snap.secondary_pct) {
        let label_key = normalize_utilization_label(label);
        if !label_key.is_empty() {
            obj.insert(
                format!("utilization_{label_key}"),
                serde_json::json!(f64::from(pct) / 100.0),
            );
        }
    }

    match reset_in_secs(snap.reset_at.as_ref()) {
        Some(secs) => {
            obj.insert("reset_in_secs".to_string(), serde_json::json!(secs));
        }
        None => {
            obj.insert("reset_in_secs".to_string(), serde_json::Value::Null);
        }
    }

    match &snap.error {
        Some(err) => {
            obj.insert("error".to_string(), serde_json::Value::String(err.clone()));
        }
        None => {
            obj.insert("error".to_string(), serde_json::Value::Null);
        }
    }

    serde_json::Value::Object(obj)
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
        let entries: Vec<serde_json::Value> = snapshots.iter().map(snapshot_json_entry).collect();
        let out = serde_json::to_string_pretty(&serde_json::json!({ "providers": entries }))?;
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
    static CACHE_LOCK: Mutex<()> = Mutex::new(());

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

    fn manual_select_cfg() -> DispatchProviderConfig {
        DispatchProviderConfig {
            auto_select_provider: false,
            ..Default::default()
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
        let result = select_provider(
            td.path(),
            Some(&mut meta),
            "implement",
            &manual_select_cfg(),
        )
        .unwrap();
        assert!(result.is_some());
        let selected = result.unwrap();
        assert_eq!(selected.name, "a");
        assert_eq!(selected.command, "cmd_a");
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
        let result = select_provider(
            td.path(),
            Some(&mut meta),
            "implement",
            &manual_select_cfg(),
        )
        .unwrap();
        let selected = result.unwrap();
        assert_eq!(selected.name, "b");
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
        let result = select_provider(
            td.path(),
            Some(&mut meta),
            "implement",
            &manual_select_cfg(),
        )
        .unwrap();
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
        let result =
            select_provider(td.path(), Some(&mut meta), "qa", &manual_select_cfg()).unwrap();
        let selected = result.unwrap();
        assert_eq!(selected.name, "qa_prov");
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
        let result =
            select_provider(td.path(), Some(&mut meta), "qa", &manual_select_cfg()).unwrap();
        let selected = result.unwrap();
        assert_eq!(selected.name, "only_prov");
    }

    #[test]
    fn select_provider_skips_provider_at_or_above_quota_threshold() {
        let _guard = CACHE_LOCK.lock().unwrap();
        let td = tempdir().unwrap();
        let mut pf = ProvidersFile::default();
        pf.providers
            .insert("codex".to_string(), mock_provider("cmd_codex"));
        pf.providers
            .insert("claude".to_string(), mock_provider("cmd_claude"));
        write_providers(td.path(), &pf).unwrap();

        {
            let mut cache = lock_provider_cache();
            let _ = cache.refresh_with(|| {
                Ok(vec![
                    ProviderSnapshot {
                        provider: "codex".into(),
                        display_name: "Codex".into(),
                        primary_pct: Some(95),
                        secondary_pct: None,
                        primary_label: Some("session".into()),
                        secondary_label: None,
                        tokens_used: None,
                        cost_usd: None,
                        reset_at: None,
                        source: "test".into(),
                        error: None,
                        loaded_models: None,
                    },
                    ProviderSnapshot {
                        provider: "claude".into(),
                        display_name: "Claude".into(),
                        primary_pct: Some(20),
                        secondary_pct: None,
                        primary_label: Some("5h".into()),
                        secondary_label: None,
                        tokens_used: None,
                        cost_usd: None,
                        reset_at: None,
                        source: "test".into(),
                        error: None,
                        loaded_models: None,
                    },
                ])
            });
        }

        let mut meta = Meta {
            provider_chain: vec!["codex".into(), "claude".into()],
            ..Default::default()
        };
        let cfg = DispatchProviderConfig {
            auto_select_provider: true,
            quota_block_threshold: 0.90,
            prefer_cheap_provider: None,
        };
        let selected = select_provider(td.path(), Some(&mut meta), "implement", &cfg)
            .unwrap()
            .unwrap();
        assert_eq!(selected.name, "claude");
    }

    #[test]
    fn select_provider_prefers_cheap_provider_for_cost_one_when_configured() {
        let _guard = CACHE_LOCK.lock().unwrap();
        let td = tempdir().unwrap();
        let mut pf = ProvidersFile::default();
        pf.providers
            .insert("codex".to_string(), mock_provider("cmd_codex"));
        pf.providers.insert(
            "ollama-local".to_string(),
            mock_provider("cmd_ollama_local"),
        );
        write_providers(td.path(), &pf).unwrap();

        {
            let mut cache = lock_provider_cache();
            let _ = cache.refresh_with(|| {
                Ok(vec![
                    ProviderSnapshot {
                        provider: "codex".into(),
                        display_name: "Codex".into(),
                        primary_pct: Some(10),
                        secondary_pct: None,
                        primary_label: Some("session".into()),
                        secondary_label: None,
                        tokens_used: None,
                        cost_usd: None,
                        reset_at: None,
                        source: "test".into(),
                        error: None,
                        loaded_models: None,
                    },
                    ProviderSnapshot {
                        provider: "ollama-local".into(),
                        display_name: "Ollama".into(),
                        primary_pct: None,
                        secondary_pct: None,
                        primary_label: None,
                        secondary_label: None,
                        tokens_used: None,
                        cost_usd: None,
                        reset_at: None,
                        source: "test".into(),
                        error: None,
                        loaded_models: Some(vec!["llama3".into()]),
                    },
                ])
            });
        }

        let mut meta = Meta {
            provider_chain: vec!["codex".into(), "ollama-local".into()],
            cost: Some(1),
            ..Default::default()
        };
        let cfg = DispatchProviderConfig {
            auto_select_provider: true,
            quota_block_threshold: 0.90,
            prefer_cheap_provider: Some("ollama-local".into()),
        };
        let selected = select_provider(td.path(), Some(&mut meta), "implement", &cfg)
            .unwrap()
            .unwrap();
        assert_eq!(selected.name, "ollama-local");
    }

    #[test]
    fn select_provider_cost_tier_routing_reorders_but_never_drops_chain_options() {
        let _guard = CACHE_LOCK.lock().unwrap();
        let td = tempdir().unwrap();
        let mut pf = ProvidersFile::default();
        pf.providers.insert(
            "ollama-local".to_string(),
            mock_provider("cmd_ollama_local"),
        );
        write_providers(td.path(), &pf).unwrap();

        {
            let mut cache = lock_provider_cache();
            let _ = cache.refresh_with(|| Ok(Vec::new()));
        }

        let mut meta = Meta {
            provider_chain: vec!["ollama-local".into()],
            cost: Some(3),
            ..Default::default()
        };
        let cfg = DispatchProviderConfig {
            auto_select_provider: true,
            quota_block_threshold: 0.90,
            prefer_cheap_provider: Some("ollama-local".into()),
        };
        let selected = select_provider(td.path(), Some(&mut meta), "implement", &cfg)
            .unwrap()
            .unwrap();
        assert_eq!(selected.name, "ollama-local");
    }

    #[test]
    fn provider_cache_reuses_fresh_snapshot_within_max_age() {
        let mut cache = ProviderCache::default();
        let mut refresh_count = 0usize;

        let first = cache
            .get_or_refresh_with(Duration::from_secs(60), || {
                refresh_count += 1;
                Ok(vec![ProviderSnapshot {
                    provider: "codex".into(),
                    display_name: "Codex".into(),
                    primary_pct: Some(12),
                    secondary_pct: None,
                    primary_label: Some("session".into()),
                    secondary_label: None,
                    tokens_used: None,
                    cost_usd: None,
                    reset_at: None,
                    source: "test".into(),
                    error: None,
                    loaded_models: None,
                }])
            })
            .unwrap();

        let second = cache
            .get_or_refresh_with(Duration::from_secs(60), || {
                refresh_count += 1;
                Ok(Vec::new())
            })
            .unwrap();

        assert_eq!(refresh_count, 1);
        assert_eq!(first.len(), second.len());
        assert_eq!(first[0].provider, second[0].provider);
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

    #[test]
    fn render_snapshots_json_is_wrapped_and_parseable() {
        let snapshots = vec![
            ProviderSnapshot {
                provider: "claude".into(),
                display_name: "Claude Code".into(),
                primary_pct: Some(61),
                secondary_pct: Some(30),
                primary_label: Some("5h".into()),
                secondary_label: Some("7d".into()),
                tokens_used: None,
                cost_usd: None,
                reset_at: Some(Utc::now() + chrono::Duration::minutes(20)),
                source: "oauth".into(),
                error: None,
                loaded_models: None,
            },
            ProviderSnapshot {
                provider: "codex".into(),
                display_name: "Codex CLI".into(),
                primary_pct: Some(5),
                secondary_pct: None,
                primary_label: Some("session".into()),
                secondary_label: Some("weekly".into()),
                tokens_used: None,
                cost_usd: None,
                reset_at: None,
                source: "oauth".into(),
                error: Some("note".into()),
                loaded_models: None,
            },
        ];

        let out = render_snapshots(&snapshots, true).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        let providers = parsed.get("providers").and_then(|v| v.as_array()).unwrap();
        assert_eq!(providers.len(), 2);

        let claude = &providers[0];
        assert_eq!(claude.get("name").and_then(|v| v.as_str()), Some("claude"));
        assert_eq!(
            claude.get("utilization_5h").and_then(|v| v.as_f64()),
            Some(0.61)
        );
        assert_eq!(
            claude.get("utilization_7d").and_then(|v| v.as_f64()),
            Some(0.30)
        );
        assert!(claude.get("reset_in_secs").is_some());
        assert!(claude.get("error").unwrap().is_null());

        let codex = &providers[1];
        assert_eq!(codex.get("name").and_then(|v| v.as_str()), Some("codex"));
        assert_eq!(
            codex.get("utilization_session").and_then(|v| v.as_f64()),
            Some(0.05)
        );
        assert!(codex.get("reset_in_secs").unwrap().is_null());
        assert_eq!(codex.get("error").and_then(|v| v.as_str()), Some("note"));
    }
}
