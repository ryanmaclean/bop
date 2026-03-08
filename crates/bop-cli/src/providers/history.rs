use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use super::ProviderSnapshot;

const HISTORY_MAX_BYTES: u64 = 1_048_576; // 1 MiB
const HISTORY_MAX_LINES: usize = 10_000;

/// A single history entry stored as one JSONL line.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HistoryEntry {
    /// Unix timestamp in milliseconds.
    pub ts: i64,
    /// Provider short name, e.g. "claude".
    pub provider: String,
    /// Primary usage percentage (0-100), `None` if unavailable.
    pub primary_pct: Option<u8>,
    /// Secondary usage percentage (0-100), `None` if unavailable.
    pub secondary_pct: Option<u8>,
    /// Cumulative tokens consumed (if available).
    pub tokens_used: Option<u64>,
    /// Cumulative cost in USD (if available).
    pub cost_usd: Option<f64>,
}

/// Return the default history file path: `~/.bop/provider-history.jsonl`.
pub fn history_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".bop").join("provider-history.jsonl"))
}

/// Ensure the history file is ready for append operations.
///
/// Creates `~/.bop/` if needed and trims the file to the last 10k lines
/// when it grows beyond 1 MiB.
pub fn prepare_history(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create history dir: {}", parent.display()))?;
    }
    trim_history_if_needed(path)
}

fn trim_history_if_needed(path: &Path) -> anyhow::Result<()> {
    let metadata = match fs::metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err).with_context(|| format!("cannot stat {}", path.display())),
    };

    if metadata.len() <= HISTORY_MAX_BYTES {
        return Ok(());
    }

    let file = fs::File::open(path)
        .with_context(|| format!("cannot open history file: {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut tail: VecDeque<String> = VecDeque::with_capacity(HISTORY_MAX_LINES);

    for line in reader.lines() {
        let line = line.with_context(|| format!("cannot read line from {}", path.display()))?;
        if tail.len() == HISTORY_MAX_LINES {
            tail.pop_front();
        }
        tail.push_back(line);
    }

    let tmp = path.with_extension("jsonl.tmp");
    {
        let mut out = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp)
            .with_context(|| format!("cannot open temp history file: {}", tmp.display()))?;

        for line in tail {
            out.write_all(line.as_bytes())
                .with_context(|| format!("cannot write temp history file: {}", tmp.display()))?;
            out.write_all(b"\n")
                .with_context(|| format!("cannot write temp history file: {}", tmp.display()))?;
        }
    }

    fs::rename(&tmp, path)
        .with_context(|| format!("cannot replace history file: {}", path.display()))?;
    Ok(())
}

/// Append one JSONL line to the history file at `path`.
///
/// Creates parent directories and the file if they don't exist.
/// Uses `O_APPEND` for atomic appends on POSIX/APFS (<4096 byte lines).
pub fn append_history(path: &Path, snapshot: &ProviderSnapshot) -> anyhow::Result<()> {
    prepare_history(path)?;

    let entry = HistoryEntry {
        ts: chrono::Utc::now().timestamp_millis(),
        provider: snapshot.provider.clone(),
        primary_pct: snapshot.primary_pct,
        secondary_pct: snapshot.secondary_pct,
        tokens_used: snapshot.tokens_used,
        cost_usd: snapshot.cost_usd,
    };

    let mut line = serde_json::to_string(&entry).context("failed to serialize history entry")?;
    line.push('\n');

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("cannot open history file: {}", path.display()))?;
    file.write_all(line.as_bytes())
        .with_context(|| format!("cannot write history entry: {}", path.display()))?;

    Ok(())
}

/// Read the last `n` entries for a given provider from the history file.
///
/// Returns entries in chronological order (oldest first).
/// If the file doesn't exist or is empty, returns an empty vec.
/// Malformed lines are silently skipped.
#[allow(dead_code)] // public API for sparkline rendering (upcoming)
pub fn read_history(path: &Path, provider: &str, n: usize) -> Vec<HistoryEntry> {
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let reader = BufReader::new(file);
    let mut entries: Vec<HistoryEntry> = Vec::new();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }
        let entry: HistoryEntry = match serde_json::from_str(&line) {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.provider == provider {
            entries.push(entry);
        }
    }

    // Keep only the last N entries.
    if entries.len() > n {
        entries.drain(..entries.len() - n);
    }

    entries
}

/// Return the last `n` primary usage values for sparkline rendering.
///
/// Values are returned in chronological order (oldest first).
#[allow(dead_code)] // consumed by BopDeck HeaderWidget integration
pub fn read_sparkline(path: &Path, provider: &str, n: usize) -> Vec<Option<u8>> {
    read_history(path, provider, n)
        .into_iter()
        .map(|entry| entry.primary_pct)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_snapshot(
        provider: &str,
        primary: Option<u8>,
        secondary: Option<u8>,
    ) -> ProviderSnapshot {
        ProviderSnapshot {
            provider: provider.to_string(),
            display_name: provider.to_string(),
            primary_pct: primary,
            secondary_pct: secondary,
            primary_label: None,
            secondary_label: None,
            tokens_used: Some(123),
            cost_usd: Some(0.12),
            reset_at: None,
            source: "test".to_string(),
            error: None,
            loaded_models: None,
        }
    }

    #[test]
    fn test_history_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("provider-history.jsonl");

        // Append several entries for different providers.
        append_history(&path, &mock_snapshot("claude", Some(57), Some(38))).unwrap();
        append_history(&path, &mock_snapshot("codex", Some(10), None)).unwrap();
        append_history(&path, &mock_snapshot("claude", Some(60), Some(40))).unwrap();
        append_history(&path, &mock_snapshot("claude", Some(65), Some(42))).unwrap();

        // Read all claude entries (n=10, only 3 exist).
        let entries = read_history(&path, "claude", 10);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].primary_pct, Some(57));
        assert_eq!(entries[0].tokens_used, Some(123));
        assert_eq!(entries[0].cost_usd, Some(0.12));
        assert_eq!(entries[1].primary_pct, Some(60));
        assert_eq!(entries[2].primary_pct, Some(65));

        // Read last 2 claude entries.
        let entries = read_history(&path, "claude", 2);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].primary_pct, Some(60));
        assert_eq!(entries[1].primary_pct, Some(65));

        // Read codex entries.
        let entries = read_history(&path, "codex", 10);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].primary_pct, Some(10));
        assert_eq!(entries[0].secondary_pct, None);

        // Read entries for unknown provider.
        let entries = read_history(&path, "unknown", 10);
        assert!(entries.is_empty());
    }

    #[test]
    fn read_history_missing_file_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.jsonl");
        let entries = read_history(&path, "claude", 10);
        assert!(entries.is_empty());
    }

    #[test]
    fn read_history_skips_malformed_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("provider-history.jsonl");

        // Write a valid entry, a malformed line, and another valid entry.
        append_history(&path, &mock_snapshot("claude", Some(50), Some(30))).unwrap();
        fs::write(
            &path,
            format!(
                "{}\nnot-valid-json\n{}\n",
                serde_json::to_string(&HistoryEntry {
                    ts: 1000,
                    provider: "claude".to_string(),
                    primary_pct: Some(50),
                    secondary_pct: Some(30),
                    tokens_used: Some(10),
                    cost_usd: Some(0.01),
                })
                .unwrap(),
                serde_json::to_string(&HistoryEntry {
                    ts: 2000,
                    provider: "claude".to_string(),
                    primary_pct: Some(70),
                    secondary_pct: Some(45),
                    tokens_used: Some(11),
                    cost_usd: Some(0.02),
                })
                .unwrap(),
            ),
        )
        .unwrap();

        let entries = read_history(&path, "claude", 10);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].primary_pct, Some(50));
        assert_eq!(entries[0].tokens_used, Some(10));
        assert_eq!(entries[1].primary_pct, Some(70));
        assert_eq!(entries[1].cost_usd, Some(0.02));
    }

    #[test]
    fn history_path_returns_expected_location() {
        if let Some(p) = history_path() {
            assert!(p.ends_with(".bop/provider-history.jsonl"));
        }
        // No home dir is a valid scenario (CI containers), so we don't assert Some.
    }

    #[test]
    fn append_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("deep").join("history.jsonl");
        append_history(&path, &mock_snapshot("claude", Some(42), None)).unwrap();
        assert!(path.exists());
        let entries = read_history(&path, "claude", 10);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].primary_pct, Some(42));
    }

    #[test]
    fn read_sparkline_returns_primary_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("provider-history.jsonl");

        append_history(&path, &mock_snapshot("claude", Some(10), None)).unwrap();
        append_history(&path, &mock_snapshot("claude", Some(20), None)).unwrap();
        append_history(&path, &mock_snapshot("claude", None, None)).unwrap();

        let sparkline = read_sparkline(&path, "claude", 5);
        assert_eq!(sparkline, vec![Some(10), Some(20), None]);
    }

    #[test]
    fn prepare_history_trims_large_file_to_last_10k_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("provider-history.jsonl");

        let mut buf = String::new();
        for i in 0..12_000 {
            let line = serde_json::json!({
                "ts": i,
                "provider": "claude",
                "primary_pct": i % 100,
                "secondary_pct": null,
                "tokens_used": null,
                "cost_usd": null
            })
            .to_string();
            buf.push_str(&line);
            buf.push('\n');
        }
        fs::write(&path, buf).unwrap();
        assert!(fs::metadata(&path).unwrap().len() > HISTORY_MAX_BYTES);

        prepare_history(&path).unwrap();

        let lines = fs::read_to_string(&path).unwrap().lines().count();
        assert_eq!(lines, HISTORY_MAX_LINES);
    }
}
