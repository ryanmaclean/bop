pub mod cardchars;
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
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

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,
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
    pub validation_summary: Option<realtime::ValidationSummary>,

    // ── planning poker ────────────────────────────────────────────────────────
    /// "open" | "revealed" | None (no active round)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poker_round: Option<String>,

    /// participant name → playing-card glyph (e.g. "alice" → "🂻")
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub estimates: BTreeMap<String, String>,

    /// Zellij session name for live attach (e.g. "bop-feat-auth").
    /// Set by bop_bop.zsh before dispatch; cleared on merge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zellij_session: Option<String>,

    /// Zellij pane ID within the session (for direct focus).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zellij_pane: Option<String>,

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
    /// Stage-specific instructions loaded from `.cards/stages/<stage>.md`.
    pub stage_instructions: String,
    /// 1-based index of current stage in the `stage_chain`.
    pub stage_index: String,
    /// Total number of stages in the `stage_chain`.
    pub stage_count: String,
    /// Output from the previous stage (`output/result.md` of prior card).
    pub prior_stage_output: String,
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

        // Walk up from card_dir to find .cards/system_context.md and .cards/stages/
        let mut system_context = String::new();
        let mut stage_instructions = String::new();
        for ancestor in card_dir.ancestors() {
            if system_context.is_empty() {
                if let Ok(sc) = fs::read_to_string(ancestor.join("system_context.md")) {
                    system_context = sc;
                }
            }
            if stage_instructions.is_empty() {
                let stage_file = ancestor.join("stages").join(format!("{}.md", meta.stage));
                if let Ok(si) = fs::read_to_string(&stage_file) {
                    stage_instructions = si;
                }
            }
            if !system_context.is_empty() && !stage_instructions.is_empty() {
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
            stage_instructions: String::new(),
            stage_index: "1".to_string(),
            stage_count: "1".to_string(),
            prior_stage_output: String::new(),
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
}
