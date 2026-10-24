use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Attestation {
    #[serde(default, alias = "runId")]
    pub run_id: String,
    #[serde(default, alias = "exitCode")]
    pub exit_code: i32,
    #[serde(default, alias = "toolCalls")]
    pub tool_calls: u64,
    #[serde(default)]
    pub model: String,
    #[serde(default, alias = "modifiedFiles")]
    pub modified_files: Vec<String>,
    #[serde(default, alias = "inputCommit")]
    pub input_commit: String,
    #[serde(default, alias = "outputCommit")]
    pub output_commit: String,
    #[serde(default, alias = "recordHash", alias = "hash")]
    pub record_hash: String,
}

impl Attestation {
    pub fn verify(&self) -> bool {
        let Some(expected_hash) = self.compute_record_hash() else {
            return false;
        };
        !self.record_hash.trim().is_empty() && self.record_hash.eq_ignore_ascii_case(&expected_hash)
    }

    pub fn compute_record_hash(&self) -> Option<String> {
        let mut unsigned = self.clone();
        unsigned.record_hash.clear();
        serde_json::to_vec(&unsigned)
            .ok()
            .map(|bytes| blake3::hash(&bytes).to_hex().to_string())
    }

    pub fn summary_for_prompt(&self, verified: bool) -> String {
        let header = if verified {
            "## Previous run attestation"
        } else {
            "## Previous run attestation (TAMPERED — do not trust)"
        };
        let exit_status = if self.exit_code == 0 {
            "passed"
        } else {
            "failed"
        };
        let modified_files = if self.modified_files.is_empty() {
            "(none)".to_string()
        } else {
            self.modified_files.join(", ")
        };
        let output_commit =
            if self.input_commit == self.output_commit && !self.output_commit.trim().is_empty() {
                format!("{} (unchanged — agent made no commits)", self.output_commit)
            } else {
                fallback_value(&self.output_commit).to_string()
            };
        let hash_status = if verified {
            "verified ✓"
        } else {
            "verification failed"
        };

        format!(
            "{header}\n- Run ID: {}\n- Exit code: {} ({exit_status})\n- Tool calls: {}\n- Model: {}\n- Modified files: {modified_files}\n- Input commit: {}\n- Output commit: {output_commit}\n- Record hash: {} ({hash_status})",
            fallback_value(&self.run_id),
            self.exit_code,
            self.tool_calls,
            fallback_value(&self.model),
            fallback_value(&self.input_commit),
            fallback_value(&self.record_hash),
        )
    }
}

fn fallback_value(value: &str) -> &str {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "unknown"
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::Attestation;

    #[test]
    fn verify_passes_when_record_hash_matches() {
        let mut attestation = Attestation {
            run_id: "r1".to_string(),
            exit_code: 0,
            tool_calls: 2,
            model: "mock".to_string(),
            modified_files: vec!["src/lib.rs".to_string()],
            input_commit: "abc".to_string(),
            output_commit: "def".to_string(),
            record_hash: String::new(),
        };
        attestation.record_hash = attestation.compute_record_hash().unwrap();
        assert!(attestation.verify());
    }

    #[test]
    fn verify_fails_when_record_hash_is_tampered() {
        let attestation = Attestation {
            run_id: "r1".to_string(),
            exit_code: 1,
            tool_calls: 3,
            model: "mock".to_string(),
            modified_files: vec!["src/main.rs".to_string()],
            input_commit: "abc".to_string(),
            output_commit: "abc".to_string(),
            record_hash: "deadbeef".to_string(),
        };
        assert!(!attestation.verify());
    }
}
