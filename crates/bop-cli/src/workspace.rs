use anyhow::Context;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use tokio::process::Command as TokioCommand;

use bop_core::{write_meta, Meta};

use super::VcsEngine;

// ---------- workspace types ----------

#[derive(Debug, Clone)]
pub struct WorkspaceInfo {
    pub name: String,
    pub path: PathBuf,
    pub change_ref: Option<String>,
}

// ---------- changes.json types ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffStats {
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangesManifest {
    pub branch: String,
    pub files_changed: Vec<FileChange>,
    pub stats: DiffStats,
}

/// Capture git diff summary and write `changes.json` into the card directory.
pub async fn write_changes_json(
    card_dir: &Path,
    workdir: &Path,
    branch: &str,
) -> anyhow::Result<()> {
    let name_status = TokioCommand::new("git")
        .args(["diff", "--name-status", "HEAD~1"])
        .current_dir(workdir)
        .output()
        .await
        .context("failed to run git diff --name-status")?;

    let ns_text = String::from_utf8_lossy(&name_status.stdout);
    let mut files: Vec<FileChange> = Vec::new();
    for line in ns_text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, '\t');
        let status_code = parts.next().unwrap_or("").trim();
        let path = parts.next().unwrap_or("").trim();
        if path.is_empty() {
            continue;
        }
        let status = match status_code.chars().next() {
            Some('A') => "added",
            Some('D') => "deleted",
            Some('M') => "modified",
            Some('R') => "renamed",
            Some('C') => "copied",
            _ => "modified",
        };
        files.push(FileChange {
            path: path.to_string(),
            status: status.to_string(),
        });
    }

    let stat_output = TokioCommand::new("git")
        .args(["diff", "--stat", "HEAD~1"])
        .current_dir(workdir)
        .output()
        .await
        .context("failed to run git diff --stat")?;

    let stat_text = String::from_utf8_lossy(&stat_output.stdout);
    let mut insertions: usize = 0;
    let mut deletions: usize = 0;
    if let Some(summary_line) = stat_text.lines().last() {
        for part in summary_line.split(',') {
            let part = part.trim();
            if part.contains("insertion") {
                if let Some(n) = part.split_whitespace().next() {
                    insertions = n.parse().unwrap_or(0);
                }
            } else if part.contains("deletion") {
                if let Some(n) = part.split_whitespace().next() {
                    deletions = n.parse().unwrap_or(0);
                }
            }
        }
    }

    let manifest = ChangesManifest {
        branch: branch.to_string(),
        files_changed: files.clone(),
        stats: DiffStats {
            files_changed: files.len(),
            insertions,
            deletions,
        },
    };

    let json = serde_json::to_string_pretty(&manifest)?;
    fs::write(card_dir.join("changes.json"), json)?;
    Ok(())
}

// ---------- workspace helpers ----------

pub fn find_git_root(start: &Path) -> Option<PathBuf> {
    let out = StdCommand::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(start)
        .output()
        .ok()?;
    if out.status.success() {
        Some(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
    } else {
        None
    }
}

pub fn sanitize_workspace_component(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "job".to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn next_workspace_name(card_id: &str) -> String {
    let base = sanitize_workspace_component(card_id);
    let ts = Utc::now().timestamp_millis();
    format!("ws-{}-{}", base, ts)
}

pub fn git_branch_exists(repo_root: &Path, branch: &str) -> bool {
    StdCommand::new("git")
        .args(["show-ref", "--verify", &format!("refs/heads/{}", branch)])
        .current_dir(repo_root)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn git_head_ref(repo_root: &Path) -> Option<String> {
    let out = StdCommand::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_root)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn jj_head_ref(workspace_path: &Path) -> Option<String> {
    let out = StdCommand::new("jj")
        .args(["log", "-r", "@", "-T", "change_id.short()"])
        .current_dir(workspace_path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn prepare_workspace(
    vcs_engine: VcsEngine,
    cards_dir: &Path,
    card_dir: &Path,
    card_id: &str,
    meta: &mut Option<Meta>,
) -> anyhow::Result<Option<WorkspaceInfo>> {
    let ws_path = card_dir.join("workspace");
    if ws_path.exists() {
        let existing_name = meta
            .as_ref()
            .and_then(|m| m.workspace_name.clone())
            .unwrap_or_else(|| "workspace".to_string());
        let change_ref = match vcs_engine {
            VcsEngine::GitGt => find_git_root(cards_dir).and_then(|r| git_head_ref(&r)),
            VcsEngine::Jj => jj_head_ref(&ws_path),
        };
        return Ok(Some(WorkspaceInfo {
            name: existing_name,
            path: ws_path,
            change_ref,
        }));
    }

    match vcs_engine {
        VcsEngine::GitGt => {
            let Some(git_root) = find_git_root(cards_dir) else {
                return Ok(None);
            };
            let branch = meta
                .as_ref()
                .and_then(|m| m.worktree_branch.clone())
                .unwrap_or_else(|| format!("job/{}", card_id));
            let ws_name = branch.replace('/', "-");
            let stable_ws = git_root.join(".worktrees").join(&ws_name);
            let legacy_ws = card_dir.join("workspace");
            // Clean up stale worktree from a previous failed dispatch
            if stable_ws.exists() {
                eprintln!("[dispatcher] cleaning stale worktree for {}", card_id);
                let _ = StdCommand::new("git")
                    .args([
                        "worktree",
                        "remove",
                        stable_ws.to_string_lossy().as_ref(),
                        "--force",
                    ])
                    .current_dir(&git_root)
                    .status();
                if git_branch_exists(&git_root, &branch) {
                    let _ = StdCommand::new("git")
                        .args(["branch", "-D", branch.as_str()])
                        .current_dir(&git_root)
                        .status();
                }
                if stable_ws.exists() {
                    let _ = fs::remove_dir_all(&stable_ws);
                }
            }
            let ws_path = if stable_ws.exists() {
                stable_ws
            } else if legacy_ws.exists() {
                legacy_ws
            } else {
                stable_ws
            };
            let status = if git_branch_exists(&git_root, &branch) {
                StdCommand::new("git")
                    .args([
                        "worktree",
                        "add",
                        ws_path.to_string_lossy().as_ref(),
                        branch.as_str(),
                    ])
                    .current_dir(&git_root)
                    .status()
            } else {
                StdCommand::new("git")
                    .args([
                        "worktree",
                        "add",
                        "-b",
                        branch.as_str(),
                        ws_path.to_string_lossy().as_ref(),
                    ])
                    .current_dir(&git_root)
                    .status()
            };

            if !matches!(status, Ok(s) if s.success()) {
                anyhow::bail!("git worktree add failed for card {}", card_id);
            }
            let change_ref = git_head_ref(&ws_path).or_else(|| git_head_ref(&git_root));
            Ok(Some(WorkspaceInfo {
                name: branch,
                path: ws_path,
                change_ref,
            }))
        }
        VcsEngine::Jj => {
            let repo_root = find_git_root(cards_dir).unwrap_or_else(|| cards_dir.to_path_buf());
            bop_core::worktree::ensure_jj_repo(&repo_root)?;
            let ws_name = next_workspace_name(card_id);
            // Stable path outside the card bundle so it survives card state renames
            let workspaces_dir = repo_root.join(".workspaces");
            fs::create_dir_all(&workspaces_dir)?;
            let stable_ws = workspaces_dir.join(&ws_name);
            let legacy_ws = card_dir.join("workspace");
            let ws_path = if stable_ws.exists() {
                stable_ws
            } else if legacy_ws.exists() {
                legacy_ws
            } else {
                stable_ws
            };
            bop_core::worktree::create_workspace_with_name(&repo_root, &ws_path, &ws_name)?;
            let change_ref = jj_head_ref(&ws_path);
            Ok(Some(WorkspaceInfo {
                name: ws_name,
                path: ws_path,
                change_ref,
            }))
        }
    }
}

pub fn persist_workspace_meta(
    meta: &mut Option<Meta>,
    card_dir: &Path,
    vcs_engine: VcsEngine,
    ws: Option<&WorkspaceInfo>,
) {
    if let Some(m) = meta.as_mut() {
        m.vcs_engine = Some(vcs_engine.as_core());
        if let Some(info) = ws {
            m.workspace_name = Some(info.name.clone());
            m.workspace_path = Some(info.path.to_string_lossy().to_string());
            m.change_ref = info.change_ref.clone();
        } else {
            m.workspace_name = None;
            m.workspace_path = None;
            m.change_ref = None;
        }
        let _ = write_meta(card_dir, m);
    }
}

pub fn is_zellij_interactive() -> bool {
    if std::env::var("ZELLIJ").is_err() {
        return false;
    }
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}

pub fn count_running_cards(cards_dir: &Path) -> usize {
    let running = cards_dir.join("running");
    fs::read_dir(&running)
        .map(|d| {
            d.filter_map(Result::ok)
                .filter(|e| {
                    e.path()
                        .extension()
                        .map(|x| x == "bop")
                        .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0)
}

pub fn zellij_open_card_pane(card_id: &str, card_dir: &Path) {
    let log = card_dir.join("logs").join("stdout.log");
    let Some(log_str) = log.to_str() else { return };
    let _ = StdCommand::new("zellij")
        .args([
            "action", "new-pane", "--name", card_id, "--", "tail", "-f", log_str,
        ])
        .output();
}

pub fn remove_worktree(path: &Path, git_root: Option<&Path>) -> anyhow::Result<()> {
    if let Some(root) = git_root {
        let status = StdCommand::new("git")
            .args(["worktree", "remove", "--force", path.to_str().unwrap_or("")])
            .current_dir(root)
            .status();
        if matches!(status, Ok(s) if s.success()) {
            return Ok(());
        }
    }
    fs::remove_dir_all(path).with_context(|| format!("failed to remove {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // ── sanitize_workspace_component ──────────────────────────────────────────

    #[test]
    fn sanitize_replaces_special_chars_with_hyphens() {
        // Spaces and special chars become hyphens, then leading/trailing hyphens are trimmed
        assert_eq!(sanitize_workspace_component("hello world"), "hello-world");
        assert_eq!(sanitize_workspace_component("foo@bar#baz"), "foo-bar-baz");
    }

    #[test]
    fn sanitize_lowercases_input() {
        assert_eq!(sanitize_workspace_component("HelloWorld"), "helloworld");
        assert_eq!(sanitize_workspace_component("FOO-BAR"), "foo-bar");
    }

    #[test]
    fn sanitize_trims_leading_trailing_hyphens() {
        assert_eq!(sanitize_workspace_component("--hello--"), "hello");
        assert_eq!(sanitize_workspace_component("!!!test!!!"), "test");
    }

    #[test]
    fn sanitize_returns_job_for_empty_or_all_special() {
        assert_eq!(sanitize_workspace_component(""), "job");
        assert_eq!(sanitize_workspace_component("!!!"), "job");
        assert_eq!(sanitize_workspace_component("@#$"), "job");
    }

    // ── next_workspace_name ───────────────────────────────────────────────────

    #[test]
    fn next_workspace_name_starts_with_ws() {
        let name = next_workspace_name("my-card");
        assert!(name.starts_with("ws-"), "got: {}", name);
    }

    #[test]
    fn next_workspace_name_contains_sanitized_card_id() {
        let name = next_workspace_name("My Card");
        // sanitized: "my-card"
        assert!(name.contains("my-card"), "got: {}", name);
    }

    #[test]
    fn next_workspace_name_includes_timestamp_suffix() {
        let name = next_workspace_name("test");
        // Format: ws-test-<timestamp>
        let parts: Vec<&str> = name.splitn(3, '-').collect();
        assert!(parts.len() >= 3, "expected ws-<id>-<ts>, got: {}", name);
        // The last part should be a numeric timestamp
        let last_part = name.rsplit('-').next().unwrap();
        assert!(
            last_part.parse::<i64>().is_ok(),
            "timestamp suffix not numeric: {}",
            last_part
        );
    }

    // ── count_running_cards ───────────────────────────────────────────────────

    #[test]
    fn count_running_cards_counts_only_bop_dirs() {
        let td = tempdir().unwrap();
        let cards_dir = td.path();
        let running = cards_dir.join("running");
        fs::create_dir_all(&running).unwrap();

        // Create some .bop dirs and some non-bop entries
        fs::create_dir_all(running.join("card-a.bop")).unwrap();
        fs::create_dir_all(running.join("card-b.bop")).unwrap();
        fs::create_dir_all(running.join("not-a-card")).unwrap();
        fs::write(running.join("file.txt"), "hello").unwrap();

        assert_eq!(count_running_cards(cards_dir), 2);
    }

    #[test]
    fn count_running_cards_returns_0_for_empty_dir() {
        let td = tempdir().unwrap();
        let cards_dir = td.path();
        fs::create_dir_all(cards_dir.join("running")).unwrap();
        assert_eq!(count_running_cards(cards_dir), 0);
    }

    #[test]
    fn count_running_cards_returns_0_for_nonexistent_dir() {
        let td = tempdir().unwrap();
        // cards_dir/running does not exist
        assert_eq!(count_running_cards(td.path()), 0);
    }

    // ── is_zellij_interactive ─────────────────────────────────────────────────

    #[test]
    fn is_zellij_interactive_returns_false_when_unset() {
        // In test environment ZELLIJ is not set
        std::env::remove_var("ZELLIJ");
        assert!(!is_zellij_interactive());
    }

    // ── find_git_root ─────────────────────────────────────────────────────────

    #[test]
    fn find_git_root_returns_none_for_non_git_dir() {
        let td = tempdir().unwrap();
        assert!(find_git_root(td.path()).is_none());
    }

    // ── struct construction ───────────────────────────────────────────────────

    #[test]
    fn workspace_info_construction() {
        let info = WorkspaceInfo {
            name: "ws-test".to_string(),
            path: PathBuf::from("/tmp/ws"),
            change_ref: Some("abc123".to_string()),
        };
        assert_eq!(info.name, "ws-test");
        assert_eq!(info.path, PathBuf::from("/tmp/ws"));
        assert_eq!(info.change_ref, Some("abc123".to_string()));
    }

    #[test]
    fn file_change_construction() {
        let fc = FileChange {
            path: "src/main.rs".to_string(),
            status: "modified".to_string(),
        };
        assert_eq!(fc.path, "src/main.rs");
        assert_eq!(fc.status, "modified");
    }

    #[test]
    fn diff_stats_construction() {
        let ds = DiffStats {
            files_changed: 3,
            insertions: 42,
            deletions: 10,
        };
        assert_eq!(ds.files_changed, 3);
        assert_eq!(ds.insertions, 42);
        assert_eq!(ds.deletions, 10);
    }

    #[test]
    fn changes_manifest_construction() {
        let manifest = ChangesManifest {
            branch: "job/test".to_string(),
            files_changed: vec![FileChange {
                path: "README.md".to_string(),
                status: "added".to_string(),
            }],
            stats: DiffStats {
                files_changed: 1,
                insertions: 5,
                deletions: 0,
            },
        };
        assert_eq!(manifest.branch, "job/test");
        assert_eq!(manifest.files_changed.len(), 1);
        assert_eq!(manifest.stats.files_changed, 1);
    }
}
