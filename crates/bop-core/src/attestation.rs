use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Attestation {
    #[serde(default, alias = "id", alias = "runId")]
    pub run_id: String,
    #[serde(default, alias = "status_code", alias = "acceptance_exit_code")]
    pub exit_code: Option<i32>,
    #[serde(default, alias = "tool_call_count", alias = "toolCalls")]
    pub tool_calls: Option<u64>,
    #[serde(default, alias = "model_name")]
    pub model: Option<String>,
    #[serde(default)]
    pub modified_files: Vec<String>,
    #[serde(default, alias = "input_sha", alias = "input_rev")]
    pub input_commit: Option<String>,
    #[serde(default, alias = "output_sha", alias = "output_rev")]
    pub output_commit: Option<String>,
    #[serde(default, alias = "hash")]
    pub record_hash: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Attestation {
    pub fn compute_record_hash(&self) -> anyhow::Result<String> {
        let mut no_hash = self.clone();
        no_hash.record_hash = None;
        let bytes = serde_json::to_vec(&no_hash)?;
        Ok(blake3::hash(&bytes).to_hex().to_string())
    }

    pub fn verify(&self) -> bool {
        let Some(stored) = self
            .record_hash
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return false;
        };

        match self.compute_record_hash() {
            Ok(computed) => computed.eq_ignore_ascii_case(stored),
            Err(_) => false,
        }
    }

    pub fn format_previous_run_summary(&self, verified: bool) -> String {
        let heading = if verified {
            "## Previous run attestation"
        } else {
            "## Previous run attestation (TAMPERED — do not trust)"
        };
        let run_id = if self.run_id.trim().is_empty() {
            "unknown".to_string()
        } else {
            self.run_id.clone()
        };
        let exit_code = match self.exit_code {
            Some(code) => {
                let status = if code == 0 { "success" } else { "failed" };
                format!("{code} ({status})")
            }
            None => "unknown".to_string(),
        };
        let tool_calls = self
            .tool_calls
            .map(|n| n.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let model = self
            .model
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("unknown");
        let modified_files = if self.modified_files.is_empty() {
            "(none)".to_string()
        } else {
            self.modified_files.join(", ")
        };
        let input_commit = self
            .input_commit
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("unknown");
        let output_commit = match (
            self.input_commit.as_deref().map(str::trim),
            self.output_commit.as_deref().map(str::trim),
        ) {
            (_, Some("")) => "unknown".to_string(),
            (Some(input), Some(output)) if !input.is_empty() && input == output => {
                format!("{output} (unchanged — agent made no commits)")
            }
            (_, Some(output)) => output.to_string(),
            _ => "unknown".to_string(),
        };
        let record_hash = match self
            .record_hash
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(hash) if verified => format!("{hash} (verified ✓)"),
            Some(hash) => format!("{hash} (verification failed)"),
            None => "unknown".to_string(),
        };

        format!(
            "{heading}\n- Run ID: {run_id}\n- Exit code: {exit_code}\n- Tool calls: {tool_calls}\n- Model: {model}\n- Modified files: {modified_files}\n- Input commit: {input_commit}\n- Output commit: {output_commit}\n- Record hash: {record_hash}"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::Attestation;

    #[test]
    fn verify_round_trips_with_computed_hash() {
        let mut attestation = Attestation {
            run_id: "abc123".to_string(),
            exit_code: Some(0),
            tool_calls: Some(3),
            model: Some("mock-model".to_string()),
            modified_files: vec!["src/lib.rs".to_string()],
            input_commit: Some("a1b2c3d".to_string()),
            output_commit: Some("a1b2c3e".to_string()),
            record_hash: None,
            ..Default::default()
        };
        attestation.record_hash = Some(attestation.compute_record_hash().unwrap());
        assert!(attestation.verify());
    }

    #[test]
    fn verify_fails_when_hash_missing_or_tampered() {
        let mut attestation = Attestation {
            run_id: "abc123".to_string(),
            exit_code: Some(1),
            tool_calls: Some(7),
            ..Default::default()
        };
        assert!(!attestation.verify());

        attestation.record_hash = Some("deadbeef".to_string());
        assert!(!attestation.verify());
    }
}
