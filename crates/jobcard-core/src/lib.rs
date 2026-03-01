pub mod config;
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Meta {
    pub id: String,
    pub created: DateTime<Utc>,

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
    pub retry_count: Option<u32>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
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

        Ok(Self {
            spec,
            plan,
            stage: meta.stage.clone(),
            acceptance_criteria,
            provider: String::new(),
            agent: meta.agent_type.clone().unwrap_or_default(),
            memory: String::new(),
        })
    }
}

/// Very small template renderer supporting {{spec}}, {{plan}}, {{stage}},
/// {{acceptance_criteria}}, {{provider}}, {{agent}}, {{memory}}.
///
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
    out
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
        };

        let rendered = render_prompt("Memory:\n{{memory}}\n", &ctx);
        assert_eq!(rendered, "Memory:\nk: v\n");
    }
}
