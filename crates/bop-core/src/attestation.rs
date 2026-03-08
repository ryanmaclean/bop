//! Forensic provenance chain for bop card runs.
//!
//! Every card run produces an `Attestation` record that cryptographically binds:
//!   - The workspace state before the agent ran (git tree hash)
//!   - The prompt and spec that were given to the agent
//!   - The adapter script that invoked the agent
//!   - The actual model string returned by the provider
//!   - The structured action log (every tool call the agent made)
//!   - The workspace state after the agent ran (git diff + tree hash)
//!   - The full stdout/stderr transcript
//!   - A blake3 hash over the entire record for tamper detection
//!
//! Phase 2 (MICROCLAW): the VM layer extends this with a TPM quote signed by
//! an Attestation Key that is sealed to specific PCR values established by the
//! dub EFI bootloader during measured boot. That quote is unforgeable without
//! compromising the TPM — providing hardware-rooted proof of the exact software
//! stack that ran the agent.
//!
//! Data flow:
//!
//!   dispatcher::run_card()
//!     → capture_input_snapshot()          before spawning adapter
//!     → [adapter runs — agent acts]
//!     → capture_output_snapshot()         after adapter exits
//!     → build_attestation_record()        tie everything together
//!     → write_attestation()               logs/attestation.json
//!
//!   codex.nu (with --json flag)
//!     → codex outputs JSONL events        logs/actions.jsonl
//!
//!   [VM - future]
//!     → zam writes tpm_quote via 9P       logs/vm_attestation.json

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

// ── Attestation record ───────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Attestation {
    /// Schema version — bump when fields are added.
    pub schema: u32,

    /// Card identity.
    pub card_id: String,
    pub run_id: String,
    pub stage: String,

    // ── Input snapshot (before adapter runs) ────────────────────────────────
    /// Git commit hash in the workspace at job start. `None` if the workspace
    /// is not a git repo (e.g., temp dirs in tests).
    pub input_commit: Option<String>,
    /// Blake3 of the rendered prompt.md given to the agent.
    pub input_prompt_hash: String,
    /// Blake3 of spec.md.
    pub input_spec_hash: String,
    /// Blake3 of meta.json at run start (same as the `checksum` field inside
    /// meta.json but recorded here for cross-reference).
    pub input_meta_checksum: String,

    // ── Agent identity ───────────────────────────────────────────────────────
    /// Provider name (e.g., "codex", "claude", "gemini").
    pub provider: String,
    /// Adapter script path (relative to repo root).
    pub adapter: String,
    /// Blake3 of the adapter script file. Proves exactly which version ran.
    pub adapter_hash: String,
    /// Actual model string from the provider (e.g., "o3-2025-04-16").
    /// Parsed from the codex JSONL event stream or API response headers.
    /// `None` if the provider did not report it.
    pub model: Option<String>,

    // ── Output snapshot (after adapter exits) ────────────────────────────────
    /// Git commit hash in the workspace after the adapter exits.
    pub output_commit: Option<String>,
    /// Exit code returned by the adapter.
    pub exit_code: i32,
    /// Blake3 of stdout.log.
    pub stdout_hash: String,
    /// Blake3 of stderr.log.
    pub stderr_hash: String,
    /// Blake3 of logs/actions.jsonl (codex --json event stream).
    /// `None` if the adapter does not produce an action log.
    pub action_log_hash: Option<String>,
    /// Number of tool-call events in actions.jsonl.
    pub action_count: Option<u64>,

    // ── Diff ─────────────────────────────────────────────────────────────────
    /// Blake3 of output/changes.patch (git diff before..after).
    pub diff_hash: Option<String>,
    /// Files added/modified/deleted by the agent. Derived from the diff.
    pub files_modified: Vec<String>,

    // ── Timing ───────────────────────────────────────────────────────────────
    pub started_at: String,
    pub ended_at: String,
    pub duration_s: Option<u64>,

    // ── VM attestation (Phase 2 — MICROCLAW) ─────────────────────────────────
    /// Populated when the agent ran inside a QEMU VM with dub secure boot.
    /// `None` for host-based adapter runs.
    pub vm_attestation: Option<VmAttestation>,

    // ── Tamper seal ──────────────────────────────────────────────────────────
    /// Blake3 of the canonical JSON of this record with `record_hash` set to
    /// `null`. Computed last. Any modification to any other field invalidates
    /// this hash.
    pub record_hash: Option<String>,
}

/// TPM-based attestation from the VM layer (Phase 2 — MICROCLAW).
///
/// Produced by zam inside the QEMU VM and written to `logs/vm_attestation.json`
/// via the 9P filesystem mount. The quote is signed by the TPM Attestation Key
/// which is sealed to the PCR values established during dub's measured boot.
/// This makes the quote unforgeable without compromising the physical TPM.
#[derive(Debug, Serialize, Deserialize)]
pub struct VmAttestation {
    /// Base64-encoded TPM2_Quote structure covering `quote_nonce`.
    /// Verifiable with the AK public key from `ak_cert`.
    pub tpm_quote: String,
    /// The nonce included in the quote — set to blake3 of the software-layer
    /// attestation record so the quote binds to the specific run.
    pub quote_nonce: String,
    /// Selected PCR values at the time of quoting.
    pub pcr_values: std::collections::BTreeMap<String, String>,
    /// PEM-encoded AK certificate chain back to the TPM EK.
    pub ak_cert: String,
    /// Blake3 of dub's boot measurement log (EFI_TCG2_EVENT_LOG).
    pub boot_log_hash: String,
    /// Timestamp from inside the VM (monotonic clock offset from VM start).
    pub vm_timestamp_ns: u64,
}

// ── Snapshot helpers ─────────────────────────────────────────────────────────

/// Capture the current HEAD commit hash from a workspace directory.
/// Returns `None` if the path is not a git repo or git is unavailable.
pub fn git_commit(workspace: &Path) -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(workspace)
        .output()
        .ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        None
    }
}

/// List files changed between two git commits in the workspace.
/// Returns an empty vec if either commit is `None` or git fails.
pub fn git_changed_files(
    workspace: &Path,
    before: Option<&str>,
    after: Option<&str>,
) -> Vec<String> {
    let (Some(a), Some(b)) = (before, after) else {
        return vec![];
    };
    if a == b {
        return vec![];
    }
    let out = Command::new("git")
        .args(["diff", "--name-only", a, b])
        .current_dir(workspace)
        .output();
    let Ok(out) = out else {
        return vec![];
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Write the diff between two commits to `output/changes.patch`.
/// Returns the blake3 hash of the written file, or `None` on failure.
pub fn capture_diff(
    workspace: &Path,
    card_dir: &Path,
    before: Option<&str>,
    after: Option<&str>,
) -> Option<String> {
    let (a, b) = (before?, after?);
    if a == b {
        return None;
    }
    let out = Command::new("git")
        .args(["diff", a, b])
        .current_dir(workspace)
        .output()
        .ok()?;
    if out.stdout.is_empty() {
        return None;
    }
    let patch_path = card_dir.join("output").join("changes.patch");
    std::fs::write(&patch_path, &out.stdout).ok()?;
    Some(blake3_file(&patch_path))
}

// ── Hash helpers ─────────────────────────────────────────────────────────────

/// Blake3 hash of a file's contents. Returns the hex string, or
/// `"<missing>"` if the file does not exist or cannot be read.
pub fn blake3_file(path: &Path) -> String {
    match std::fs::read(path) {
        Ok(bytes) => blake3::hash(&bytes).to_hex().to_string(),
        Err(_) => "<missing>".to_string(),
    }
}

/// Parse the model string from a codex JSONL action log.
/// Looks for `{"type":"agent_response","model":"..."}` entries.
pub fn model_from_action_log(actions_path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(actions_path).ok()?;
    for line in content.lines() {
        let v: serde_json::Value = serde_json::from_str(line).ok()?;
        if v.get("type").and_then(|t| t.as_str()) == Some("agent_response") {
            if let Some(model) = v.get("model").and_then(|m| m.as_str()) {
                if !model.is_empty() {
                    return Some(model.to_string());
                }
            }
        }
        // Also check top-level "model" field on any event
        if let Some(model) = v.get("model").and_then(|m| m.as_str()) {
            if !model.is_empty() {
                return Some(model.to_string());
            }
        }
    }
    None
}

/// Count tool-call events in a codex JSONL action log.
pub fn action_count_from_log(actions_path: &Path) -> Option<u64> {
    let content = std::fs::read_to_string(actions_path).ok()?;
    let count = content
        .lines()
        .filter(|l| {
            serde_json::from_str::<serde_json::Value>(l)
                .ok()
                .and_then(|v| {
                    v.get("type")
                        .and_then(|t| t.as_str())
                        .map(|t| t.starts_with("tool_"))
                })
                .unwrap_or(false)
        })
        .count() as u64;
    if count > 0 {
        Some(count)
    } else {
        None
    }
}

// ── Record assembly and persistence ─────────────────────────────────────────

impl Attestation {
    /// Compute and set `record_hash` — blake3 over the record with
    /// `record_hash` temporarily set to `null`, then serialized as canonical
    /// JSON (sorted keys via BTreeMap ordering from serde).
    pub fn seal(&mut self) {
        self.record_hash = None;
        if let Ok(bytes) = serde_json::to_vec(self) {
            self.record_hash = Some(blake3::hash(&bytes).to_hex().to_string());
        }
    }

    /// Write to `card_dir/logs/attestation.json`.
    pub fn write(&self, card_dir: &Path) -> anyhow::Result<()> {
        let path = card_dir.join("logs").join("attestation.json");
        let bytes =
            serde_json::to_vec_pretty(self).context("failed to serialize attestation record")?;
        std::fs::write(&path, bytes).with_context(|| format!("failed to write {}", path.display()))
    }

    /// Read from `card_dir/logs/attestation.json`.
    pub fn read(card_dir: &Path) -> anyhow::Result<Self> {
        let path = card_dir.join("logs").join("attestation.json");
        let bytes =
            std::fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
        serde_json::from_slice(&bytes).context("failed to parse attestation.json")
    }

    /// Verify the record_hash over this record.
    /// Returns `true` if the record is unmodified since sealing.
    pub fn verify(&self) -> bool {
        let stored = match &self.record_hash {
            Some(h) => h.clone(),
            None => return false,
        };
        let mut copy = Self {
            record_hash: None,
            // Clone all other fields via serde round-trip
            ..serde_json::from_str(&serde_json::to_string(self).unwrap_or_default())
                .unwrap_or_default()
        };
        copy.record_hash = None;
        match serde_json::to_vec(&copy) {
            Ok(bytes) => blake3::hash(&bytes).to_hex().to_string() == stored,
            Err(_) => false,
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn attestation_seal_and_verify_roundtrip() {
        let mut a = Attestation {
            schema: 1,
            card_id: "test-card".to_string(),
            run_id: "abc123".to_string(),
            stage: "implement".to_string(),
            input_prompt_hash: "deadbeef".to_string(),
            input_spec_hash: "cafebabe".to_string(),
            input_meta_checksum: "01234567".to_string(),
            provider: "codex".to_string(),
            adapter: "adapters/codex.nu".to_string(),
            adapter_hash: "ffffffff".to_string(),
            model: Some("o3-2025-04-16".to_string()),
            exit_code: 0,
            stdout_hash: "aabbccdd".to_string(),
            stderr_hash: "11223344".to_string(),
            started_at: "2026-03-08T18:00:00Z".to_string(),
            ended_at: "2026-03-08T18:10:00Z".to_string(),
            ..Default::default()
        };
        a.seal();
        assert!(a.record_hash.is_some());
        assert!(a.verify(), "sealed attestation should verify");
    }

    #[test]
    fn attestation_tamper_detection() {
        let mut a = Attestation {
            schema: 1,
            card_id: "test-card".to_string(),
            run_id: "abc123".to_string(),
            exit_code: 0,
            stdout_hash: "aabbccdd".to_string(),
            stderr_hash: "11223344".to_string(),
            input_prompt_hash: "x".to_string(),
            input_spec_hash: "x".to_string(),
            input_meta_checksum: "x".to_string(),
            provider: "codex".to_string(),
            adapter: "adapters/codex.nu".to_string(),
            adapter_hash: "x".to_string(),
            started_at: "2026-03-08T18:00:00Z".to_string(),
            ended_at: "2026-03-08T18:10:00Z".to_string(),
            ..Default::default()
        };
        a.seal();
        // Tamper with a field after sealing
        a.exit_code = 1;
        assert!(!a.verify(), "tampered attestation should not verify");
    }

    #[test]
    fn attestation_write_and_read() {
        let td = tempdir().unwrap();
        std::fs::create_dir_all(td.path().join("logs")).unwrap();
        let mut a = Attestation {
            schema: 1,
            card_id: "write-read-test".to_string(),
            run_id: "r1".to_string(),
            exit_code: 0,
            stdout_hash: "a".to_string(),
            stderr_hash: "b".to_string(),
            input_prompt_hash: "c".to_string(),
            input_spec_hash: "d".to_string(),
            input_meta_checksum: "e".to_string(),
            provider: "mock".to_string(),
            adapter: "adapters/mock.nu".to_string(),
            adapter_hash: "f".to_string(),
            started_at: "2026-03-08T00:00:00Z".to_string(),
            ended_at: "2026-03-08T00:01:00Z".to_string(),
            ..Default::default()
        };
        a.seal();
        a.write(td.path()).unwrap();
        let loaded = Attestation::read(td.path()).unwrap();
        assert!(loaded.verify());
        assert_eq!(loaded.card_id, "write-read-test");
    }

    #[test]
    fn blake3_file_returns_missing_for_nonexistent() {
        let result = blake3_file(std::path::Path::new("/nonexistent/path/file.txt"));
        assert_eq!(result, "<missing>");
    }

    #[test]
    fn model_from_action_log_parses_agent_response() {
        let td = tempdir().unwrap();
        let log = td.path().join("actions.jsonl");
        std::fs::write(
            &log,
            r#"{"type":"tool_call","name":"read_file"}
{"type":"agent_response","model":"o3-2025-04-16","content":"done"}
"#,
        )
        .unwrap();
        assert_eq!(
            model_from_action_log(&log),
            Some("o3-2025-04-16".to_string())
        );
    }
}
