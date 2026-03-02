pub mod config;
pub mod realtime;
pub mod worktree;

pub use config::{load_config, Config};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
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
        if let Some(mode) = self.workflow_mode.as_ref() {
            if mode.trim().is_empty() {
                return Err(JobCardError::Invalid(
                    "meta.workflow_mode is empty".to_string(),
                ));
            }
        }
        if self.step_index == Some(0) {
            return Err(JobCardError::Invalid(
                "meta.step_index must be >= 1".to_string(),
            ));
        }
        if self.step_index.is_some() && self.workflow_mode.is_none() {
            return Err(JobCardError::Invalid(
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
    hydrate_typed_metadata(card_dir, &mut meta);
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
}
