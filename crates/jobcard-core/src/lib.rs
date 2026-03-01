pub mod config;
pub mod realtime;
pub mod worktree;

pub use config::{load_config, Config};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum JobCardError {
    #[error("invalid jobcard: {0}")]
    Invalid(String),
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Meta {
    pub id: String,
    pub created: DateTime<Utc>,

    /// Unicode playing-card glyph (U+1F0A0–U+1F0FF) used as the card's
    /// unique visual token across all surfaces (QL preview, zellij pane
    /// title, `jc list` output, vibekanban board).
    /// Suit encodes team: ♠=CLI ♥=Arch ♦=Quality ♣=Platform.
    /// Rank encodes priority: Ace=P1, King/Queen=P2, Jack/Knight=P3, 2-10=P4.
    /// Jokers (🃏🂿🃟) = wildcard/emergency. Trump cards = cross-team escalation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glyph: Option<String>,

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

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,

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

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_result: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_count: Option<u32>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_summary: Option<realtime::ValidationSummary>,

    // ── planning poker ────────────────────────────────────────────────────────
    /// "open" | "revealed" | None (no active round)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poker_round: Option<String>,

    /// participant name → playing-card glyph (e.g. "alice" → "🂻")
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub estimates: BTreeMap<String, String>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl Meta {
    pub fn validate(&self) -> Result<(), JobCardError> {
        if self.id.trim().is_empty() {
            return Err(JobCardError::Invalid("meta.id is empty".to_string()));
        }
        if self.stage.trim().is_empty() {
            return Err(JobCardError::Invalid("meta.stage is empty".to_string()));
        }
        Ok(())
    }
}

pub fn meta_path(card_dir: &Path) -> PathBuf {
    card_dir.join("meta.json")
}

pub fn read_meta(card_dir: &Path) -> anyhow::Result<Meta> {
    let bytes = fs::read(meta_path(card_dir))?;
    let meta: Meta = serde_json::from_slice(&bytes)?;
    meta.validate()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    Ok(meta)
}

pub fn write_meta(card_dir: &Path, meta: &Meta) -> anyhow::Result<()> {
    meta.validate()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let bytes = serde_json::to_vec_pretty(meta)?;
    fs::write(meta_path(card_dir), bytes)?;
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

        // Walk up from card_dir to find .cards/system_context.md
        let system_context = card_dir
            .ancestors()
            .find_map(|p| {
                let candidate = p.join("system_context.md");
                fs::read_to_string(&candidate).ok()
            })
            .unwrap_or_default();

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
        };

        let rendered = render_prompt("Memory:\n{{memory}}\n", &ctx);
        assert_eq!(rendered, "Memory:\nk: v\n");
    }
}
