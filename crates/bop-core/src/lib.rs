pub mod cardchars;
pub mod config;
pub mod lineage;
pub mod realtime;
pub mod worktree;

pub use config::{load_config, Config};

use anyhow::Context as _;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum BopError {
    #[error("invalid bop: {0}")]
    Invalid(String),
}

/// Configuration for the .cards/ directory, stored in .cards/config.json
/// All fields are Option so partial configs can be merged.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct CardsConfig {
    /// Zellij session name for live attach (e.g. "bop" or "pmr").
    /// Set by `bop init` when run inside a Zellij session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zellij_session: Option<String>,
}

/// Parse JSON bytes into CardsConfig, returning a clear error on bad schema.
pub fn parse_cards_config(json: &str) -> anyhow::Result<CardsConfig> {
    if json.trim().is_empty() {
        return Ok(CardsConfig::default());
    }
    serde_json::from_str(json).context(
        "malformed .cards/config.json: expected schema with optional fields: \
        zellij_session",
    )
}

/// Return the cards config path: <cwd>/.cards/config.json
pub fn cards_config_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".cards")
        .join("config.json")
}

/// Read config from .cards/config.json (used by `bop init`).
pub fn read_cards_config_file(path: &Path) -> anyhow::Result<CardsConfig> {
    let json = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read .cards config: {}", path.display()))?;
    parse_cards_config(&json)
        .with_context(|| format!("invalid .cards config at {}", path.display()))
}

/// Write config to .cards/config.json (used by `bop init`).
pub fn write_cards_config_file(path: &Path, cfg: &CardsConfig) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create .cards config dir: {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(cfg).context("failed to serialize .cards config")?;
    std::fs::write(path, json)
        .with_context(|| format!("cannot write .cards config: {}", path.display()))?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StageStatus {
    Pending,
    Running,
    Done,
    Blocked,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VcsEngine {
    GitGt,
    Jj,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StageRecord {
    pub status: StageStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_s: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_by: Option<String>,
}

/// A label/tag attached to a card (e.g. {"name":"High Impact","kind":"effort"}).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Label {
    pub name: String,
    /// Freeform category: "domain", "effort", "scope", etc.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// A single checklist item inside a card.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Subtask {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub done: bool,
}

/// OpenLineage-style provenance for a single stage execution attempt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct RunRecord {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub run_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stage: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub provider: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub adapter: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_s: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

fn default_meta_version() -> u32 {
    1
}
fn is_version_one(v: &u32) -> bool {
    *v == 1
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Meta {
    pub id: String,
    /// Schema version for forward-compatible migration.
    /// Absent in old cards → deserialized as 1 (current version).
    /// Increment when making breaking changes to Meta layout.
    #[serde(
        default = "default_meta_version",
        skip_serializing_if = "is_version_one"
    )]
    pub meta_version: u32,
    pub created: DateTime<Utc>,

    /// Unicode playing-card glyph (U+1F0A0–U+1F0FF) used as the card's
    /// unique visual token across all surfaces (QL preview, zellij pane
    /// title, `bop list` output, vibekanban board).
    /// Suit encodes team: ♠=CLI ♥=Arch ♦=Quality ♣=Platform.
    /// Rank encodes priority: Ace=P1, King/Queen=P2, Jack/Knight=P3, 2-10=P4.
    /// Jokers (🃏🂿🃟) = wildcard/emergency. Trump cards = cross-team escalation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glyph: Option<String>,

    /// BMP-safe token for terminal, filenames, pane titles.
    /// Suit symbol: ♠♥♦♣ for CLI/Arch/Quality/Platform.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,

    /// Human-readable display title (defaults to `id` when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    /// One-sentence description shown as subtitle in the card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Labels / tags shown as pills (Coding, Performance, High Impact, …).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<Label>,

    /// 0–100 overall completion percentage (updated by agent / merge-gate).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<u8>,

    /// Fine-grained subtasks shown as dots in the card preview.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subtasks: Vec<Subtask>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,

    pub stage: String,

    /// Card behavior/type discriminator.
    /// Example: `roadmap` or `roadmap_feature`.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub card_type: Option<String>,

    /// Relative or absolute path to an external metadata JSON payload.
    /// For roadmap cards this commonly points to `roadmap.json` or
    /// `output/roadmap.json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_source: Option<String>,

    /// Optional key used to select an item from external metadata.
    /// For roadmap feature cards this typically matches `features[].id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_key: Option<String>,

    /// High-level workflow family used for stage routing and agent behavior.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_mode: Option<String>,

    /// 1-based step index inside a workflow plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_index: Option<u32>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,
    /// Cost tier for provider routing:
    /// 1=trivial, 2=small, 3=medium, 4+=complex.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_chain: Vec<String>,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub stages: BTreeMap<String, StageRecord>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acceptance_criteria: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_branch: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_namespace: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vcs_engine: Option<VcsEngine>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_name: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_ref: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub policy_scope: Vec<String>,

    #[serde(default, skip_serializing_if = "is_false")]
    pub decision_required: bool,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_path: Option<String>,

    /// Card IDs this card depends on — dispatcher skips until all are done/merged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,

    /// Where spawn_child_cards() should place children: "pending" (default) or "drafts".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawn_to: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_result: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_count: Option<u32>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_at: Option<DateTime<Utc>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_summary: Option<realtime::ValidationSummary>,

    // ── planning poker ────────────────────────────────────────────────────────
    /// "open" | "revealed" | None (no active round)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poker_round: Option<String>,

    /// participant name → playing-card glyph (e.g. "alice" → "🂻")
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub estimates: BTreeMap<String, String>,

    /// Zellij session name for live attach (e.g. "bop-feat-auth").
    /// Set by bop_bop.nu before dispatch; cleared on merge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zellij_session: Option<String>,

    /// Zellij pane ID within the session (for direct focus).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zellij_pane: Option<String>,

    // ── Auto-Claude linkage ─────────────────────────────────────────────────
    /// Auto-Claude spec ID (e.g. "022") linking this card to an AC
    /// implementation plan. Quick Look and CLI resolve the full path:
    /// `<git_root>/.auto-claude/specs/<ac_spec_id>-*/implementation_plan.json`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ac_spec_id: Option<String>,

    // ── stage pipeline (factory engine) ─────────────────────────────────────
    /// Ordered stage pipeline this card progresses through.
    /// Example: `["implement", "qa"]` or `["spec", "plan", "implement", "qa"]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stage_chain: Vec<String>,

    /// Model tier per stage. Example: `{"implement": "opus", "qa": "sonnet"}`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub stage_models: BTreeMap<String, String>,

    /// Adapter/provider per stage. Example: `{"implement": "claude", "qa": "codex"}`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub stage_providers: BTreeMap<String, String>,

    /// Max token budget per stage. Example: `{"implement": 32000, "qa": 8000}`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub stage_budgets: BTreeMap<String, u64>,

    /// Execution provenance records (OpenLineage-style) for each run attempt.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runs: Vec<RunRecord>,

    /// SHA256 checksum of the canonical JSON serialization (excluding this field).
    /// Used to detect corruption in meta.json; absent checksums trigger JSONL replay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl Default for Meta {
    fn default() -> Self {
        Self {
            id: String::new(),
            meta_version: 1,
            created: DateTime::<Utc>::default(),
            glyph: None,
            token: None,
            title: None,
            description: None,
            labels: Vec::new(),
            progress: None,
            subtasks: Vec::new(),
            agent_type: None,
            stage: String::new(),
            card_type: None,
            metadata_source: None,
            metadata_key: None,
            workflow_mode: None,
            step_index: None,
            priority: None,
            cost: None,
            timeout_seconds: None,
            provider_chain: Vec::new(),
            stages: BTreeMap::new(),
            acceptance_criteria: Vec::new(),
            worktree_branch: None,
            template_namespace: None,
            vcs_engine: None,
            workspace_name: None,
            workspace_path: None,
            change_ref: None,
            policy_scope: Vec::new(),
            decision_required: false,
            decision_path: None,
            depends_on: Vec::new(),
            spawn_to: None,
            policy_result: None,
            retry_count: None,
            failure_reason: None,
            exit_code: None,
            paused_at: None,
            validation_summary: None,
            poker_round: None,
            estimates: BTreeMap::new(),
            zellij_session: None,
            zellij_pane: None,
            ac_spec_id: None,
            stage_chain: Vec::new(),
            stage_models: BTreeMap::new(),
            stage_providers: BTreeMap::new(),
            stage_budgets: BTreeMap::new(),
            runs: Vec::new(),
            checksum: None,
        }
    }
}

impl Meta {
    pub fn validate(&self) -> Result<(), BopError> {
        if self.id.trim().is_empty() {
            return Err(BopError::Invalid("meta.id is empty".to_string()));
        }
        if self.stage.trim().is_empty() {
            return Err(BopError::Invalid("meta.stage is empty".to_string()));
        }
        if let Some(mode) = self.workflow_mode.as_ref() {
            if mode.trim().is_empty() {
                return Err(BopError::Invalid("meta.workflow_mode is empty".to_string()));
            }
        }
        if self.step_index == Some(0) {
            return Err(BopError::Invalid(
                "meta.step_index must be >= 1".to_string(),
            ));
        }
        if self.step_index.is_some() && self.workflow_mode.is_none() {
            return Err(BopError::Invalid(
                "meta.step_index requires meta.workflow_mode".to_string(),
            ));
        }
        Ok(())
    }
}

pub fn meta_path(card_dir: &Path) -> PathBuf {
    card_dir.join("meta.json")
}

fn value_as_non_empty_string(v: Option<&Value>) -> Option<String> {
    v.and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn normalize_roadmap_status(raw: &str) -> Option<String> {
    let key = raw.trim().to_lowercase().replace(['-', ' '], "_");
    match key.as_str() {
        "under_review" | "review" | "underreview" => Some("under_review".to_string()),
        "planned" | "plan" => Some("planned".to_string()),
        "in_progress" | "inprogress" | "active" | "doing" => Some("in_progress".to_string()),
        "done" | "completed" | "complete" => Some("done".to_string()),
        _ => None,
    }
}

fn roadmap_priority_to_rank(raw: &str) -> Option<i64> {
    let key = raw.trim().to_lowercase().replace(['-', ' '], "_");
    match key.as_str() {
        "must" | "must_have" | "critical" => Some(1),
        "should" | "should_have" | "important" => Some(2),
        "could" | "could_have" | "nice_to_have" => Some(3),
        _ => None,
    }
}

fn resolve_metadata_source(card_dir: &Path, meta: &Meta) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(source) = meta.metadata_source.as_ref().map(|s| s.trim()) {
        if !source.is_empty() {
            let p = PathBuf::from(source);
            if p.is_absolute() {
                candidates.push(p);
            } else {
                candidates.push(card_dir.join(p));
            }
        }
    }
    candidates.push(card_dir.join("roadmap.json"));
    candidates.push(card_dir.join("output").join("roadmap.json"));
    candidates.into_iter().find(|p| p.exists())
}

fn hydrate_roadmap_from_json(meta: &mut Meta, json: &Value) {
    if let Some(obj) = json.as_object() {
        if meta.title.is_none() {
            meta.title = value_as_non_empty_string(
                obj.get("project_name")
                    .or_else(|| obj.get("title"))
                    .or_else(|| obj.get("name")),
            );
        }
        if meta.description.is_none() {
            meta.description = value_as_non_empty_string(
                obj.get("vision")
                    .or_else(|| obj.get("summary"))
                    .or_else(|| obj.get("description")),
            );
        }
    }
    if meta.workflow_mode.is_none() {
        meta.workflow_mode = Some("roadmap".to_string());
    }
}

fn hydrate_roadmap_feature_from_json(meta: &mut Meta, json: &Value) {
    let Some(obj) = json.as_object() else {
        return;
    };
    let Some(features) = obj.get("features").and_then(|v| v.as_array()) else {
        return;
    };
    let lookup = meta
        .metadata_key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(meta.id.as_str());
    let Some(feature) = features.iter().find(|f| {
        f.as_object()
            .and_then(|fo| fo.get("id"))
            .and_then(|v| v.as_str())
            .map(|id| id == lookup)
            .unwrap_or(false)
    }) else {
        return;
    };
    let Some(fo) = feature.as_object() else {
        return;
    };

    if meta.title.is_none() {
        meta.title = value_as_non_empty_string(fo.get("title").or_else(|| fo.get("name")));
    }
    if meta.description.is_none() {
        meta.description = value_as_non_empty_string(fo.get("description"));
    }
    if meta.priority.is_none() {
        meta.priority = fo
            .get("priority")
            .and_then(|v| v.as_str())
            .and_then(roadmap_priority_to_rank);
    }
    if meta.acceptance_criteria.is_empty() {
        if let Some(arr) = fo.get("acceptance_criteria").and_then(|v| v.as_array()) {
            meta.acceptance_criteria = arr
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect();
        }
    }
    if meta.labels.is_empty() {
        let mut labels = Vec::new();

        if let Some(priority_raw) = fo.get("priority").and_then(|v| v.as_str()) {
            let p = priority_raw.trim().to_lowercase();
            if !p.is_empty() {
                labels.push(Label {
                    name: p,
                    kind: Some("priority".to_string()),
                });
            }
        }
        if let Some(status_raw) = fo.get("status").and_then(|v| v.as_str()) {
            if let Some(status) = normalize_roadmap_status(status_raw) {
                labels.push(Label {
                    name: status.clone(),
                    kind: Some("status".to_string()),
                });
                if matches!(
                    meta.stage.as_str(),
                    "roadmap" | "roadmap_feature" | "feature"
                ) {
                    meta.stage = status;
                }
            }
        }
        if let Some(phase) =
            value_as_non_empty_string(fo.get("phase").or_else(|| fo.get("phase_id")))
        {
            labels.push(Label {
                name: phase,
                kind: Some("phase".to_string()),
            });
        }
        if !labels.is_empty() {
            meta.labels = labels;
        }
    }
    if meta.workflow_mode.is_none() {
        meta.workflow_mode = Some("roadmap".to_string());
    }
}

fn hydrate_typed_metadata(card_dir: &Path, meta: &mut Meta) {
    let Some(card_type) = meta
        .card_type
        .as_deref()
        .map(str::trim)
        .map(str::to_lowercase)
    else {
        return;
    };
    if card_type.is_empty() {
        return;
    }
    let Some(source_path) = resolve_metadata_source(card_dir, meta) else {
        return;
    };
    let Ok(raw) = fs::read_to_string(source_path) else {
        return;
    };
    let Ok(json) = serde_json::from_str::<Value>(&raw) else {
        return;
    };

    match card_type.as_str() {
        "roadmap" => hydrate_roadmap_from_json(meta, &json),
        "roadmap_feature" | "roadmap-feature" | "roadmap_feature_card" => {
            hydrate_roadmap_feature_from_json(meta, &json)
        }
        _ => {}
    }
}

pub fn read_meta(card_dir: &Path) -> anyhow::Result<Meta> {
    let bytes = fs::read(meta_path(card_dir))?;
    let mut meta: Meta = serde_json::from_slice(&bytes)?;

    // Validate checksum if present
    if let Some(stored_checksum) = meta.checksum.clone() {
        // Set checksum to None and reserialize to compute expected hash
        let mut meta_without_checksum = meta.clone();
        meta_without_checksum.checksum = None;
        let canonical_bytes = serde_json::to_vec(&meta_without_checksum)?;
        let computed_hash = blake3::hash(&canonical_bytes).to_hex().to_string();

        if computed_hash != stored_checksum {
            eprintln!(
                "[warn] meta.json checksum mismatch on {}: expected {}, got {}",
                meta.id, stored_checksum, computed_hash
            );
            return Err(anyhow::anyhow!(
                "meta.json checksum mismatch on {}: stored={}, computed={}",
                meta.id,
                stored_checksum,
                computed_hash
            ));
        }
    }

    hydrate_typed_metadata(card_dir, &mut meta);
    meta.validate()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    Ok(meta)
}

/// Returns true when `card_dir` is inside a `templates/` parent directory.
/// Templates are user-editable config files and should not be locked.
fn is_template_dir(card_dir: &Path) -> bool {
    card_dir.components().any(|c| c.as_os_str() == "templates")
}

/// Clear the macOS user-immutable flag on a file so it can be overwritten.
/// No-op on non-macOS platforms. Called before writing meta.json.
pub fn meta_unprotect(path: &Path) {
    #[cfg(target_os = "macos")]
    {
        // UF_IMMUTABLE = 0x00000002; cleared via fchflags(fd, 0)
        // chflags(2) is the POSIX-ish way; libc::chflags is available on macOS.
        use std::ffi::CString;
        if let Ok(c) = CString::new(path.as_os_str().as_encoded_bytes()) {
            unsafe { libc::chflags(c.as_ptr(), 0) };
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = path;
}

/// Set the macOS user-immutable flag on a file after writing.
/// Prevents accidental manual edits — any write attempt returns EPERM.
/// No-op on non-macOS platforms.
pub fn meta_protect(path: &Path) {
    #[cfg(target_os = "macos")]
    {
        use std::ffi::CString;
        const UF_IMMUTABLE: u32 = 0x00000002;
        if let Ok(c) = CString::new(path.as_os_str().as_encoded_bytes()) {
            unsafe { libc::chflags(c.as_ptr(), UF_IMMUTABLE) };
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = path;
}

/// Remove a card directory, clearing the immutable flag on meta.json first.
/// `fs::remove_dir_all` fails on macOS if any file inside has UF_IMMUTABLE set.
pub fn remove_card_dir(card_dir: &Path) -> std::io::Result<()> {
    meta_unprotect(&meta_path(card_dir));
    std::fs::remove_dir_all(card_dir)
}

pub fn write_meta(card_dir: &Path, meta: &Meta) -> anyhow::Result<()> {
    use std::io::Write;

    meta.validate()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    // Compute checksum: serialize with checksum=None, hash, then set checksum
    let mut meta_with_checksum = meta.clone();
    meta_with_checksum.checksum = None;
    let canonical_bytes = serde_json::to_vec(&meta_with_checksum)?;
    let hash = blake3::hash(&canonical_bytes);
    meta_with_checksum.checksum = Some(hash.to_hex().to_string());

    // Canonical form is compact JSON (eliminates whitespace ambiguity)
    let bytes = serde_json::to_vec(&meta_with_checksum)?;
    let target = meta_path(card_dir);

    // Clear immutable flag before writing so the atomic rename succeeds
    meta_unprotect(&target);

    // Atomic write: temp file + rename
    // Create temp file in same directory to ensure atomic rename on same filesystem
    let temp_dir = target
        .parent()
        .with_context(|| format!("meta.json path has no parent: {}", target.display()))?;
    let mut temp_file = tempfile::Builder::new()
        .prefix(".meta.json.")
        .suffix(".tmp")
        .tempfile_in(temp_dir)
        .with_context(|| format!("failed to create temp file in {}", temp_dir.display()))?;

    temp_file
        .write_all(&bytes)
        .context("failed to write meta.json to temp file")?;

    temp_file
        .persist(&target)
        .map_err(|e| anyhow::anyhow!("failed to persist meta.json: {}", e))?;

    // Lock the file: any process that doesn't go through write_meta gets EPERM.
    // Templates are config files that users customize — skip protection for them.
    if !is_template_dir(card_dir) {
        meta_protect(&target);
    }

    // Best-effort: log the meta_written event to JSONL audit log
    let _ = append_event(
        card_dir,
        &Event {
            ts: Utc::now().to_rfc3339(),
            event: "meta_written".into(),
            stage: None,
            provider: None,
            pid: None,
            exit_code: None,
            from: None,
            to: None,
        },
    );

    Ok(())
}

/// A single event record for the append-only JSONL audit log.
/// All fields are optional to support different event types.
/// Event records MUST stay under 512 bytes when serialized.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Event {
    /// RFC3339 timestamp
    pub ts: String,
    /// Event type: "meta_written", "stage_transition", etc.
    pub event: String,
    /// Optional fields for event context (stage, provider, pid, exit_code, from, to)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

/// Append an event to the card's JSONL audit log at logs/events.jsonl.
/// This is best-effort: failures are returned as Err but should be ignored
/// by callers (use `let _ = append_event(...)`).
///
/// O_APPEND is atomic for writes ≤ PIPE_BUF (4096 bytes on Linux/macOS),
/// so no locking is needed if event records stay under 512 bytes.
pub fn append_event(card_dir: &Path, event: &Event) -> anyhow::Result<()> {
    let logs_dir = card_dir.join("logs");
    fs::create_dir_all(&logs_dir)
        .with_context(|| format!("failed to create logs dir: {}", logs_dir.display()))?;

    let events_path = logs_dir.join("events.jsonl");

    // Serialize to compact JSON (not pretty)
    let mut json_line = serde_json::to_vec(event).context("failed to serialize event")?;

    // Verify size constraint
    if json_line.len() > 512 {
        anyhow::bail!(
            "event record too large ({} bytes, max 512): {}",
            json_line.len(),
            event.event
        );
    }

    // Append newline
    json_line.push(b'\n');

    // Append to file (O_APPEND ensures atomicity for small writes)
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&events_path)
        .with_context(|| format!("failed to open events.jsonl: {}", events_path.display()))?;

    file.write_all(&json_line)
        .with_context(|| format!("failed to write to events.jsonl: {}", events_path.display()))?;

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptContext {
    pub spec: String,
    pub plan: String,
    pub stage: String,
    pub acceptance_criteria: String,
    pub provider: String,
    pub agent: String,
    pub memory: String,
    /// Prepended verbatim before the template. Load from `.cards/system_context.md`.
    pub system_context: String,
    pub worktree_branch: String,
    /// Stage-specific instructions loaded from `.cards/stages/<stage>.md`.
    pub stage_instructions: String,
    /// 1-based index of current stage in the `stage_chain`.
    pub stage_index: String,
    /// Total number of stages in the `stage_chain`.
    pub stage_count: String,
    /// Output from the previous stage (`output/result.md` of prior card).
    pub prior_stage_output: String,
    /// This card's ID — lets prompts reference the card by name.
    pub card_id: String,
    /// Absolute path to the card directory.
    pub card_dir: String,
    /// Concatenated output/result.md from all cards in `depends_on`.
    pub depends_output: String,
    /// Contents of `.cards/CODEBASE.md` if present; empty otherwise.
    pub codebase_index: String,
}

impl PromptContext {
    pub fn from_files(card_dir: &Path, meta: &Meta) -> anyhow::Result<Self> {
        let spec = fs::read_to_string(card_dir.join("spec.md")).unwrap_or_default();
        let plan = fs::read_to_string(card_dir.join("plan.json")).unwrap_or_default();

        let acceptance_criteria = if meta.acceptance_criteria.is_empty() {
            String::new()
        } else {
            meta.acceptance_criteria.join("\n")
        };

        // Walk up from card_dir to find .cards/system_context.md, .cards/CODEBASE.md, and .cards/stages/
        let mut system_context = String::new();
        let mut stage_instructions = String::new();
        let mut codebase_index = String::new();
        for ancestor in card_dir.ancestors() {
            if system_context.is_empty() {
                if let Ok(sc) = fs::read_to_string(ancestor.join("system_context.md")) {
                    system_context = sc;
                }
            }
            if codebase_index.is_empty() {
                if let Ok(ci) = fs::read_to_string(ancestor.join("CODEBASE.md")) {
                    codebase_index = ci;
                }
            }
            if stage_instructions.is_empty() {
                let stage_file = ancestor.join("stages").join(format!("{}.md", meta.stage));
                if let Ok(si) = fs::read_to_string(&stage_file) {
                    stage_instructions = si;
                }
            }
            if !system_context.is_empty()
                && !stage_instructions.is_empty()
                && !codebase_index.is_empty()
            {
                break;
            }
        }

        // Stage index/count from stage_chain
        let (stage_index, stage_count) = if meta.stage_chain.is_empty() {
            ("1".to_string(), "1".to_string())
        } else {
            let idx = meta
                .stage_chain
                .iter()
                .position(|s| s == &meta.stage)
                .map(|i| i + 1)
                .unwrap_or(1);
            (idx.to_string(), meta.stage_chain.len().to_string())
        };

        // Prior stage output: look for output/result.md in card dir
        let prior_stage_output =
            fs::read_to_string(card_dir.join("output").join("prior_result.md")).unwrap_or_default();

        // Dependency output: concatenate output/result.md from all depends_on cards.
        // Walk up from card_dir to find the .cards root, then search done/merged for each dep.
        let depends_output = if meta.depends_on.is_empty() {
            String::new()
        } else {
            let mut parts = Vec::new();
            // card_dir is e.g. .cards/running/my-card.bop — parent.parent = .cards
            if let Some(cards_root) = card_dir.parent().and_then(|p| p.parent()) {
                for dep_id in &meta.depends_on {
                    for state in ["done", "merged"] {
                        let dep_exact = cards_root.join(state).join(format!("{}.bop", dep_id));
                        let result_path = dep_exact.join("output").join("result.md");
                        if result_path.exists() {
                            if let Ok(content) = fs::read_to_string(&result_path) {
                                parts.push(format!("## Output from `{}`\n\n{}", dep_id, content));
                            }
                            break;
                        }
                        // Also try glyph-prefixed dirs
                        let suffix = format!("-{}.bop", dep_id);
                        if let Ok(entries) = fs::read_dir(cards_root.join(state)) {
                            for entry in entries.flatten() {
                                let name = entry.file_name();
                                if name.to_str().map(|n| n.ends_with(&suffix)).unwrap_or(false) {
                                    let rp = entry.path().join("output").join("result.md");
                                    if let Ok(content) = fs::read_to_string(&rp) {
                                        parts.push(format!(
                                            "## Output from `{}`\n\n{}",
                                            dep_id, content
                                        ));
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            parts.join("\n\n---\n\n")
        };

        Ok(Self {
            spec,
            plan,
            stage: meta.stage.clone(),
            acceptance_criteria,
            provider: String::new(),
            agent: meta.agent_type.clone().unwrap_or_default(),
            memory: String::new(),
            system_context,
            worktree_branch: meta.worktree_branch.clone().unwrap_or_default(),
            stage_instructions,
            stage_index,
            stage_count,
            prior_stage_output,
            card_id: meta.id.clone(),
            card_dir: card_dir.to_string_lossy().into_owned(),
            depends_output,
            codebase_index,
        })
    }
}

/// Template renderer supporting {{spec}}, {{plan}}, {{stage}},
/// {{acceptance_criteria}}, {{provider}}, {{agent}}, {{memory}}.
///
/// If `ctx.system_context` is non-empty it is prepended before the template.
/// If the template has no {{...}} markers, it is returned unchanged.
pub fn render_prompt(template: &str, ctx: &PromptContext) -> String {
    let mut out = template.to_string();
    out = out.replace("{{spec}}", &ctx.spec);
    out = out.replace("{{plan}}", &ctx.plan);
    out = out.replace("{{stage}}", &ctx.stage);
    out = out.replace("{{acceptance_criteria}}", &ctx.acceptance_criteria);
    out = out.replace("{{provider}}", &ctx.provider);
    out = out.replace("{{agent}}", &ctx.agent);
    out = out.replace("{{memory}}", &ctx.memory);
    out = out.replace("{{worktree_branch}}", &ctx.worktree_branch);
    out = out.replace("{{stage_instructions}}", &ctx.stage_instructions);
    out = out.replace("{{stage_index}}", &ctx.stage_index);
    out = out.replace("{{stage_count}}", &ctx.stage_count);
    out = out.replace("{{prior_stage_output}}", &ctx.prior_stage_output);
    out = out.replace("{{card_id}}", &ctx.card_id);
    out = out.replace("{{card_dir}}", &ctx.card_dir);
    out = out.replace("{{depends_output}}", &ctx.depends_output);
    out = out.replace("{{codebase_index}}", &ctx.codebase_index);
    if ctx.system_context.is_empty() {
        out
    } else {
        format!("{}\n\n---\n\n{}", ctx.system_context.trim_end(), out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_version_defaults_to_1_on_old_json() {
        // Old card JSON with no meta_version field
        let json = r#"{"id":"test","created":"2026-01-01T00:00:00Z","stage":"spec"}"#;
        let meta: Meta = serde_json::from_str(json).unwrap();
        assert_eq!(meta.meta_version, 1);
    }

    #[test]
    fn meta_version_round_trips() {
        let json = r#"{"id":"t","created":"2026-01-01T00:00:00Z","stage":"spec","meta_version":2}"#;
        let meta: Meta = serde_json::from_str(json).unwrap();
        assert_eq!(meta.meta_version, 2);
        let serialized = serde_json::to_string(&meta).unwrap();
        assert!(serialized.contains("\"meta_version\":2"));
    }

    #[test]
    fn meta_version_1_omitted_from_serialized_json() {
        let json = r#"{"id":"t","created":"2026-01-01T00:00:00Z","stage":"spec"}"#;
        let meta: Meta = serde_json::from_str(json).unwrap();
        assert_eq!(meta.meta_version, 1);
        let serialized = serde_json::to_string(&meta).unwrap();
        assert!(
            !serialized.contains("meta_version"),
            "meta_version:1 must be omitted to keep old card files stable"
        );
    }

    #[test]
    fn render_prompt_replaces_memory_placeholder() {
        let ctx = PromptContext {
            spec: "spec".to_string(),
            plan: "plan".to_string(),
            stage: "implement".to_string(),
            acceptance_criteria: "ac".to_string(),
            provider: "mock".to_string(),
            agent: "agent".to_string(),
            memory: "k: v".to_string(),
            system_context: String::new(),
            worktree_branch: String::new(),
            stage_instructions: String::new(),
            stage_index: "1".to_string(),
            stage_count: "1".to_string(),
            prior_stage_output: String::new(),
            card_id: "test-card".to_string(),
            card_dir: "/tmp/test.bop".to_string(),
            depends_output: String::new(),
            codebase_index: String::new(),
        };

        let rendered = render_prompt("Memory:\n{{memory}}\n", &ctx);
        assert_eq!(rendered, "Memory:\nk: v\n");
    }

    #[test]
    fn render_prompt_replaces_stage_pipeline_vars() {
        let ctx = PromptContext {
            spec: "build auth".to_string(),
            plan: String::new(),
            stage: "qa".to_string(),
            acceptance_criteria: "cargo test".to_string(),
            provider: "claude".to_string(),
            agent: String::new(),
            memory: String::new(),
            system_context: String::new(),
            worktree_branch: "job/feat-auth".to_string(),
            stage_instructions: "You are reviewing code.".to_string(),
            stage_index: "2".to_string(),
            stage_count: "2".to_string(),
            prior_stage_output: "Implemented auth module.".to_string(),
            card_id: "feat-auth-qa".to_string(),
            card_dir: "/tmp/feat-auth-qa.bop".to_string(),
            depends_output: String::new(),
            codebase_index: String::new(),
        };

        let template = "{{stage_instructions}}\nCard stage: {{stage}} ({{stage_index}} of {{stage_count}})\n{{spec}}\n{{prior_stage_output}}";
        let rendered = render_prompt(template, &ctx);
        assert!(rendered.contains("You are reviewing code."));
        assert!(rendered.contains("(2 of 2)"));
        assert!(rendered.contains("build auth"));
        assert!(rendered.contains("Implemented auth module."));
    }

    #[test]
    fn meta_zellij_session_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let m = Meta {
            id: "z1".into(),
            created: chrono::Utc::now(),
            stage: "implement".into(),
            provider_chain: vec![],
            stages: Default::default(),
            acceptance_criteria: vec![],
            zellij_session: Some("bop-z1".into()),
            zellij_pane: Some("3".into()),
            ..Default::default()
        };
        write_meta(dir.path(), &m).unwrap();
        let back = read_meta(dir.path()).unwrap();
        assert_eq!(back.zellij_session.as_deref(), Some("bop-z1"));
        assert_eq!(back.zellij_pane.as_deref(), Some("3"));
    }

    #[test]
    fn meta_ac_spec_id_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let m = Meta {
            id: "ac1".into(),
            created: chrono::Utc::now(),
            stage: "implement".into(),
            ac_spec_id: Some("022".into()),
            ..Default::default()
        };
        write_meta(dir.path(), &m).unwrap();

        // Verify JSON key is 'ac_spec_id'
        let raw = fs::read_to_string(meta_path(dir.path())).unwrap();
        assert!(
            raw.contains("\"ac_spec_id\""),
            "field should serialize as 'ac_spec_id'"
        );

        // Verify round-trip
        let back = read_meta(dir.path()).unwrap();
        assert_eq!(back.ac_spec_id.as_deref(), Some("022"));
    }

    #[test]
    fn meta_ac_spec_id_omitted_when_none() {
        let dir = tempfile::tempdir().unwrap();
        let m = Meta {
            id: "ac-none".into(),
            created: chrono::Utc::now(),
            stage: "implement".into(),
            ..Default::default()
        };
        write_meta(dir.path(), &m).unwrap();

        let raw = fs::read_to_string(meta_path(dir.path())).unwrap();
        assert!(
            !raw.contains("ac_spec_id"),
            "ac_spec_id should be omitted when None"
        );

        // Should still round-trip fine
        let back = read_meta(dir.path()).unwrap();
        assert_eq!(back.ac_spec_id, None);
    }

    #[test]
    fn meta_stage_pipeline_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let mut models = BTreeMap::new();
        models.insert("implement".into(), "opus".into());
        models.insert("qa".into(), "sonnet".into());
        let mut providers = BTreeMap::new();
        providers.insert("implement".into(), "claude".into());
        providers.insert("qa".into(), "codex".into());
        let mut budgets = BTreeMap::new();
        budgets.insert("implement".into(), 32000u64);
        budgets.insert("qa".into(), 8000u64);

        let m = Meta {
            id: "pipe1".into(),
            created: chrono::Utc::now(),
            stage: "implement".into(),
            stage_chain: vec!["implement".into(), "qa".into()],
            stage_models: models.clone(),
            stage_providers: providers.clone(),
            stage_budgets: budgets.clone(),
            ..Default::default()
        };
        write_meta(dir.path(), &m).unwrap();
        let back = read_meta(dir.path()).unwrap();
        assert_eq!(back.stage_chain, vec!["implement", "qa"]);
        assert_eq!(back.stage_models, models);
        assert_eq!(back.stage_providers, providers);
        assert_eq!(back.stage_budgets, budgets);
    }

    #[test]
    fn meta_empty_stage_pipeline_omitted_in_json() {
        let dir = tempfile::tempdir().unwrap();
        let m = Meta {
            id: "no-pipe".into(),
            created: chrono::Utc::now(),
            stage: "implement".into(),
            ..Default::default()
        };
        write_meta(dir.path(), &m).unwrap();
        let raw = fs::read_to_string(meta_path(dir.path())).unwrap();
        // Empty vecs/maps should be skipped by serde
        assert!(!raw.contains("stage_chain"));
        assert!(!raw.contains("stage_models"));
        assert!(!raw.contains("stage_providers"));
        assert!(!raw.contains("stage_budgets"));
        // But should still round-trip fine
        let back = read_meta(dir.path()).unwrap();
        assert!(back.stage_chain.is_empty());
    }

    #[test]
    fn meta_token_field_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let m = Meta {
            id: "tok1".into(),
            created: chrono::Utc::now(),
            stage: "implement".into(),
            glyph: Some("\u{1F0AB}".into()),
            token: Some("\u{2660}".into()),
            ..Default::default()
        };
        write_meta(dir.path(), &m).unwrap();
        let back = read_meta(dir.path()).unwrap();
        assert_eq!(back.token.as_deref(), Some("\u{2660}"));
    }

    #[test]
    fn meta_runs_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let m = Meta {
            id: "run1".into(),
            created: chrono::Utc::now(),
            stage: "implement".into(),
            runs: vec![RunRecord {
                run_id: "abcd1234".into(),
                stage: "implement".into(),
                provider: "claude".into(),
                model: "claude-sonnet-4-6".into(),
                adapter: "adapters/claude.nu".into(),
                started_at: "2026-03-01T20:45:00Z".into(),
                ended_at: Some("2026-03-01T20:48:00Z".into()),
                outcome: "success".into(),
                prompt_tokens: Some(123),
                completion_tokens: Some(456),
                cost_usd: Some(0.73),
                duration_s: Some(180),
                note: Some("retry 1".into()),
            }],
            ..Default::default()
        };
        write_meta(dir.path(), &m).unwrap();
        let raw = fs::read_to_string(meta_path(dir.path())).unwrap();
        assert!(raw.contains("\"runs\""));
        let back = read_meta(dir.path()).unwrap();
        assert_eq!(back.runs.len(), 1);
        assert_eq!(back.runs[0].run_id, "abcd1234");
        assert_eq!(back.runs[0].outcome, "success");
        assert_eq!(back.runs[0].duration_s, Some(180));
    }

    #[test]
    fn meta_workflow_fields_validate_and_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = Meta {
            id: "wf-1".into(),
            created: chrono::Utc::now(),
            stage: "implement".into(),
            workflow_mode: Some("default-feature".into()),
            step_index: Some(1),
            ..Default::default()
        };

        write_meta(dir.path(), &m).unwrap();
        let back = read_meta(dir.path()).unwrap();
        assert_eq!(back.workflow_mode.as_deref(), Some("default-feature"));
        assert_eq!(back.step_index, Some(1));

        m.workflow_mode = Some("   ".into());
        assert!(write_meta(dir.path(), &m).is_err());

        m.workflow_mode = None;
        m.step_index = Some(1);
        assert!(write_meta(dir.path(), &m).is_err());

        m.workflow_mode = Some("default-feature".into());
        m.step_index = Some(0);
        assert!(write_meta(dir.path(), &m).is_err());
    }

    #[test]
    fn meta_card_type_serializes_to_type_key() {
        let dir = tempfile::tempdir().unwrap();
        let m = Meta {
            id: "roadmap-1".into(),
            created: chrono::Utc::now(),
            stage: "roadmap".into(),
            card_type: Some("roadmap".into()),
            metadata_source: Some("roadmap.json".into()),
            ..Default::default()
        };

        write_meta(dir.path(), &m).unwrap();
        let raw = fs::read_to_string(meta_path(dir.path())).unwrap();
        assert!(raw.contains("\"type\""));
        assert!(!raw.contains("\"card_type\""));
    }

    #[test]
    fn parse_valid_cards_config() {
        let json = r#"{"zellij_session": "bop"}"#;
        let cfg = parse_cards_config(json).unwrap();
        assert_eq!(cfg.zellij_session, Some("bop".to_string()));
    }

    #[test]
    fn parse_empty_cards_config() {
        let cfg = parse_cards_config("").unwrap();
        assert_eq!(cfg, CardsConfig::default());
    }

    #[test]
    fn parse_malformed_cards_config_returns_error() {
        let result = parse_cards_config(r#"{"zellij_session": 123}"#);
        assert!(result.is_err(), "expected error for bad schema");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("zellij_session") || msg.contains("invalid") || msg.contains("expected"),
            "error message should hint at the field: {}",
            msg
        );
    }

    #[test]
    fn roundtrip_cards_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".cards").join("config.json");
        let cfg = CardsConfig {
            zellij_session: Some("pmr".to_string()),
        };
        write_cards_config_file(&path, &cfg).unwrap();
        let back = read_cards_config_file(&path).unwrap();
        assert_eq!(back.zellij_session, Some("pmr".to_string()));
    }

    #[test]
    fn cards_config_empty_fields_omitted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".cards").join("config.json");
        let cfg = CardsConfig::default();
        write_cards_config_file(&path, &cfg).unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        // Empty/None fields should be skipped by serde
        assert!(!raw.contains("zellij_session"));
    }

    #[test]
    fn read_meta_hydrates_roadmap_type_from_roadmap_json() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("roadmap.json"),
            r#"{
  "project_name": "Auto-Tundra",
  "vision": "AI orchestration for software teams"
}"#,
        )
        .unwrap();

        let m = Meta {
            id: "roadmap-root".into(),
            created: chrono::Utc::now(),
            stage: "roadmap".into(),
            card_type: Some("roadmap".into()),
            metadata_source: Some("roadmap.json".into()),
            ..Default::default()
        };
        write_meta(dir.path(), &m).unwrap();

        let back = read_meta(dir.path()).unwrap();
        assert_eq!(back.title.as_deref(), Some("Auto-Tundra"));
        assert_eq!(
            back.description.as_deref(),
            Some("AI orchestration for software teams")
        );
        assert_eq!(back.workflow_mode.as_deref(), Some("roadmap"));
    }

    #[test]
    fn read_meta_hydrates_roadmap_feature_type_from_output_json() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("output")).unwrap();
        fs::write(
            dir.path().join("output").join("roadmap.json"),
            r#"{
  "features": [
    {
      "id": "feat-auth",
      "title": "API Authentication & Authorization System",
      "description": "Secure all API endpoints",
      "priority": "must",
      "status": "in progress",
      "phase": "Production Foundation",
      "acceptance_criteria": [
        "All API endpoints enforce authentication by default",
        "Role-based access control is supported"
      ]
    }
  ]
}"#,
        )
        .unwrap();

        let m = Meta {
            id: "job-auth-impl".into(),
            created: chrono::Utc::now(),
            stage: "roadmap_feature".into(),
            card_type: Some("roadmap_feature".into()),
            metadata_key: Some("feat-auth".into()),
            ..Default::default()
        };
        write_meta(dir.path(), &m).unwrap();

        let back = read_meta(dir.path()).unwrap();
        assert_eq!(
            back.title.as_deref(),
            Some("API Authentication & Authorization System")
        );
        assert_eq!(
            back.description.as_deref(),
            Some("Secure all API endpoints")
        );
        assert_eq!(back.priority, Some(1));
        assert_eq!(back.stage, "in_progress");
        assert_eq!(back.workflow_mode.as_deref(), Some("roadmap"));
        assert_eq!(back.acceptance_criteria.len(), 2);

        assert!(back
            .labels
            .iter()
            .any(|l| l.kind.as_deref() == Some("priority") && l.name == "must"));
        assert!(back
            .labels
            .iter()
            .any(|l| l.kind.as_deref() == Some("status") && l.name == "in_progress"));
        assert!(back
            .labels
            .iter()
            .any(|l| l.kind.as_deref() == Some("phase") && l.name == "Production Foundation"));
    }

    #[test]
    fn append_event_creates_logs_dir_and_jsonl_file() {
        let dir = tempfile::tempdir().unwrap();
        let event = Event {
            ts: "2026-03-07T12:00:00Z".into(),
            event: "meta_written".into(),
            stage: Some("running".into()),
            provider: Some("claude".into()),
            pid: Some(12345),
            exit_code: None,
            from: None,
            to: None,
        };

        append_event(dir.path(), &event).unwrap();

        let events_path = dir.path().join("logs").join("events.jsonl");
        assert!(events_path.exists());

        let content = fs::read_to_string(&events_path).unwrap();
        assert!(content.contains("\"event\":\"meta_written\""));
        assert!(content.contains("\"stage\":\"running\""));
        assert!(content.contains("\"pid\":12345"));
        assert!(content.ends_with('\n'));
    }

    #[test]
    fn append_event_appends_multiple_events() {
        let dir = tempfile::tempdir().unwrap();

        let event1 = Event {
            ts: "2026-03-07T12:00:00Z".into(),
            event: "stage_transition".into(),
            stage: None,
            provider: None,
            pid: None,
            exit_code: Some(0),
            from: Some("pending".into()),
            to: Some("running".into()),
        };

        let event2 = Event {
            ts: "2026-03-07T12:05:00Z".into(),
            event: "stage_transition".into(),
            stage: None,
            provider: None,
            pid: None,
            exit_code: Some(0),
            from: Some("running".into()),
            to: Some("done".into()),
        };

        append_event(dir.path(), &event1).unwrap();
        append_event(dir.path(), &event2).unwrap();

        let events_path = dir.path().join("logs").join("events.jsonl");
        let content = fs::read_to_string(&events_path).unwrap();

        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("\"from\":\"pending\""));
        assert!(lines[1].contains("\"from\":\"running\""));
    }

    #[test]
    fn append_event_compact_json_no_pretty_print() {
        let dir = tempfile::tempdir().unwrap();
        let event = Event {
            ts: "2026-03-07T12:00:00Z".into(),
            event: "meta_written".into(),
            stage: Some("qa".into()),
            provider: None,
            pid: None,
            exit_code: None,
            from: None,
            to: None,
        };

        append_event(dir.path(), &event).unwrap();

        let events_path = dir.path().join("logs").join("events.jsonl");
        let content = fs::read_to_string(&events_path).unwrap();

        // Compact JSON should not have newlines inside the object
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 1);
        assert!(!lines[0].contains("\n  "));
    }

    #[test]
    fn append_event_skips_none_fields() {
        let dir = tempfile::tempdir().unwrap();
        let event = Event {
            ts: "2026-03-07T12:00:00Z".into(),
            event: "test_event".into(),
            stage: Some("running".into()),
            provider: None,
            pid: None,
            exit_code: None,
            from: None,
            to: None,
        };

        append_event(dir.path(), &event).unwrap();

        let events_path = dir.path().join("logs").join("events.jsonl");
        let content = fs::read_to_string(&events_path).unwrap();

        // None fields should be omitted from JSON
        assert!(!content.contains("\"provider\""));
        assert!(!content.contains("\"pid\""));
        assert!(!content.contains("\"exit_code\""));
        assert!(content.contains("\"stage\":\"running\""));
    }

    #[test]
    fn append_event_rejects_oversized_records() {
        let dir = tempfile::tempdir().unwrap();

        // Create an event with a very long string that exceeds 512 bytes
        let long_string = "a".repeat(600);
        let event = Event {
            ts: "2026-03-07T12:00:00Z".into(),
            event: long_string,
            stage: None,
            provider: None,
            pid: None,
            exit_code: None,
            from: None,
            to: None,
        };

        let result = append_event(dir.path(), &event);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("too large"));
        assert!(err_msg.contains("512"));
    }

    #[test]
    fn write_meta_appends_event_to_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let card_dir = dir.path();

        let meta = Meta {
            id: "test-write-123".into(),
            created: Utc::now(),
            stage: "pending".into(),
            glyph: Some("🃏".into()),
            title: Some("Test Write Meta".into()),
            ..Default::default()
        };

        // Call write_meta
        write_meta(card_dir, &meta).unwrap();

        // Verify meta.json was created
        let meta_path = card_dir.join("meta.json");
        assert!(meta_path.exists());

        // Verify logs/events.jsonl was created
        let events_path = card_dir.join("logs").join("events.jsonl");
        assert!(
            events_path.exists(),
            "logs/events.jsonl should be created by write_meta"
        );

        // Verify the event contains meta_written
        let content = fs::read_to_string(&events_path).unwrap();
        assert!(content.contains("\"event\":\"meta_written\""));

        // Verify it's valid JSONL (one line, ends with newline)
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 1);
        assert!(content.ends_with('\n'));
    }

    #[test]
    fn write_meta_checksum() {
        let dir = tempfile::tempdir().unwrap();
        let card_dir = dir.path();

        let meta = Meta {
            id: "test-checksum-123".into(),
            created: Utc::now(),
            stage: "pending".into(),
            glyph: Some("🃏".into()),
            title: Some("Test Checksum".into()),
            ..Default::default()
        };

        // Call write_meta
        write_meta(card_dir, &meta).unwrap();

        // Read the written file
        let meta_path = card_dir.join("meta.json");
        let content = fs::read_to_string(&meta_path).unwrap();

        // Verify checksum field exists in the JSON
        assert!(
            content.contains("\"checksum\":"),
            "meta.json should contain checksum field"
        );

        // Parse and verify the checksum is a valid hex string
        let written_meta: Meta = serde_json::from_str(&content).unwrap();
        assert!(
            written_meta.checksum.is_some(),
            "checksum should be set after write_meta"
        );

        let checksum = written_meta.checksum.as_ref().unwrap();
        assert_eq!(
            checksum.len(),
            64,
            "blake3 checksum should be 64 hex characters"
        );
        assert!(
            checksum.chars().all(|c| c.is_ascii_hexdigit()),
            "checksum should be hex-encoded"
        );

        // Verify checksum matches recomputation
        let mut verify_meta = written_meta.clone();
        verify_meta.checksum = None;
        let canonical_bytes = serde_json::to_vec(&verify_meta).unwrap();
        let expected_hash = blake3::hash(&canonical_bytes).to_hex().to_string();

        assert_eq!(
            *checksum, expected_hash,
            "checksum should match recomputation"
        );
    }

    #[test]
    fn read_meta_checksum() {
        let dir = tempfile::tempdir().unwrap();
        let card_dir = dir.path();

        // Create and write a valid meta with checksum
        let meta = Meta {
            id: "test-read-checksum-456".into(),
            created: Utc::now(),
            stage: "pending".into(),
            glyph: Some("🃏".into()),
            title: Some("Test Read Checksum".into()),
            ..Default::default()
        };

        write_meta(card_dir, &meta).unwrap();

        // Read it back - should succeed with valid checksum
        let read_back = read_meta(card_dir);
        assert!(
            read_back.is_ok(),
            "read_meta should succeed with valid checksum"
        );

        // Corrupt the checksum by manually editing meta.json
        let meta_path = card_dir.join("meta.json");
        let content = fs::read_to_string(&meta_path).unwrap();
        let mut corrupted_meta: serde_json::Value = serde_json::from_str(&content).unwrap();

        // Replace checksum with an invalid one
        corrupted_meta["checksum"] = serde_json::Value::String("0".repeat(64));
        // meta.json is immutable after write_meta; unprotect before raw write
        meta_unprotect(&meta_path);
        fs::write(&meta_path, serde_json::to_vec(&corrupted_meta).unwrap()).unwrap();

        // Read should now fail due to checksum mismatch
        let result = read_meta(card_dir);
        assert!(
            result.is_err(),
            "read_meta should fail with invalid checksum"
        );

        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("checksum mismatch"),
            "error message should mention checksum mismatch, got: {}",
            err_msg
        );
    }

    /// After write_meta, the meta.json file must be immutable on macOS.
    /// A raw fs::write attempt must fail with a permission error.
    #[test]
    #[cfg(target_os = "macos")]
    fn meta_json_is_immutable_after_write_meta() {
        let dir = tempfile::tempdir().unwrap();
        let m = Meta {
            id: "imm-1".into(),
            created: chrono::Utc::now(),
            stage: "implement".into(),
            ..Default::default()
        };
        write_meta(dir.path(), &m).unwrap();

        // Direct write to the protected file must fail
        let path = meta_path(dir.path());
        let result = fs::write(&path, b"tampered");
        assert!(
            result.is_err(),
            "direct write to meta.json must be rejected after write_meta (got Ok)"
        );
    }

    /// write_meta must be idempotent: calling it twice succeeds even though the
    /// file is already immutable after the first call.
    #[test]
    fn meta_write_meta_twice_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = Meta {
            id: "imm-2".into(),
            created: chrono::Utc::now(),
            stage: "implement".into(),
            ..Default::default()
        };
        write_meta(dir.path(), &m).unwrap();

        // Update a field and write again — must not fail with EPERM
        m.stage = "qa".into();
        write_meta(dir.path(), &m)
            .expect("second write_meta must succeed (unprotect + write + protect cycle)");

        let back = read_meta(dir.path()).unwrap();
        assert_eq!(back.stage, "qa");
    }

    /// read_meta must succeed even when meta.json has the immutable flag set.
    #[test]
    fn meta_read_meta_works_on_immutable_file() {
        let dir = tempfile::tempdir().unwrap();
        let m = Meta {
            id: "imm-3".into(),
            created: chrono::Utc::now(),
            stage: "implement".into(),
            ..Default::default()
        };
        write_meta(dir.path(), &m).unwrap();

        // read_meta must not fail; the immutable flag is a write-only restriction
        let back = read_meta(dir.path()).expect("read_meta must succeed on immutable meta.json");
        assert_eq!(back.id, "imm-3");
    }
}
