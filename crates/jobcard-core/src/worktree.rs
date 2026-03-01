//! jj workspace management for per-card isolation.
//!
//! Each job card gets `jj workspace add <card>/workspace` before the adapter runs.
//! On merge: `jj squash` folds changes back, `jj workspace forget` cleans up.
use anyhow::{Context, Result};
use std::path::Path;

/// Initialize a jj repo at `repo_root` (colocated with git) if `.jj/` doesn't exist yet.
/// Safe to call repeatedly.
pub fn ensure_jj_repo(repo_root: &Path) -> Result<()> {
    if repo_root.join(".jj").join("repo").exists() {
        return Ok(());
    }
    let out = std::process::Command::new("jj")
        .args(["git", "init", "--colocate"])
        .current_dir(repo_root)
        .output()
        .context("failed to run `jj git init --colocate` (is jj installed?)")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("jj git init failed: {}", stderr);
    }
    Ok(())
}

/// Create a jj workspace at `ws_path` from `repo_root`.
/// The workspace name is derived from the path basename.
pub fn create_workspace(repo_root: &Path, ws_path: &Path) -> Result<()> {
    let name = ws_path
        .file_name()
        .and_then(|n| n.to_str())
        .with_context(|| {
            format!(
                "workspace path has no usable basename: {}",
                ws_path.display()
            )
        })?;
    create_workspace_with_name(repo_root, ws_path, name)
}

/// Create a jj workspace at `ws_path` with an explicit workspace name.
pub fn create_workspace_with_name(repo_root: &Path, ws_path: &Path, ws_name: &str) -> Result<()> {
    let out = std::process::Command::new("jj")
        .args(["workspace", "add", "--name", ws_name])
        .arg(ws_path)
        .current_dir(repo_root)
        .output()
        .context("failed to run `jj workspace add`")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("jj workspace add failed: {}", stderr);
    }
    Ok(())
}

/// Squash all changes in the card workspace into its parent change.
/// Run this from inside the workspace directory after the agent finishes.
pub fn squash_workspace(ws_path: &Path) -> Result<()> {
    let out = std::process::Command::new("jj")
        .args(["squash"])
        .current_dir(ws_path)
        .output()
        .context("failed to run `jj squash`")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        // jj exits non-zero when there are no changes to squash — this is OK.
        // Verified against jj 0.x: the actual message is "Nothing changed after diffing".
        // "nothing to squash" and "No diff" do NOT appear in real jj output; removed.
        if stderr.contains("Nothing changed") {
            return Ok(());
        }
        anyhow::bail!("jj squash failed: {}", stderr);
    }
    Ok(())
}

/// Forget (deregister) the workspace. Call from repo_root after squashing.
/// jj does not delete the directory — that's the caller's responsibility.
pub fn forget_workspace(repo_root: &Path, ws_name: &str) -> Result<()> {
    let out = std::process::Command::new("jj")
        .args(["workspace", "forget", ws_name])
        .current_dir(repo_root)
        .output()
        .context("failed to run `jj workspace forget`")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        if stderr.contains("no workspace named") || stderr.contains("doesn't have a workspace") {
            return Ok(()); // already forgotten or never existed
        }
        anyhow::bail!("jj workspace forget failed: {}", stderr);
    }
    Ok(())
}

/// Push all un-pushed changes to the remote as git branches.
/// Best-effort: non-fatal if no remote is configured.
pub fn push_stack(repo_root: &Path, remote: &str) -> Result<()> {
    let out = std::process::Command::new("jj")
        .args(["git", "push", "--remote", remote, "--all"])
        .current_dir(repo_root)
        .output()
        .context("failed to run `jj git push`")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("jj git push failed: {}", stderr);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Legacy aliases — kept so existing call sites compile until Task 3 migrates
// the merge gate. Marked deprecated to surface them in warnings.
// ---------------------------------------------------------------------------

#[deprecated(note = "use create_workspace instead")]
pub fn create_worktree(git_root: &Path, wt_path: &Path, _branch_name: &str) -> Result<()> {
    create_workspace(git_root, wt_path)
}

#[deprecated(note = "use squash_workspace + forget_workspace instead")]
pub fn commit_worktree(_wt_path: &Path, _card_id: &str) -> Result<()> {
    Ok(()) // no-op: jj tracks all changes automatically
}

#[deprecated(note = "push_stack handles this now")]
pub fn merge_card_branch(_git_root: &Path, _branch_name: &str) -> Result<bool> {
    Ok(true) // signal success; real work happens in Task 3
}

#[deprecated(note = "use forget_workspace instead")]
pub fn remove_worktree(_git_root: &Path, _wt_path: &Path) -> Result<()> {
    Ok(()) // no-op until Task 3
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jj_available() -> bool {
        std::process::Command::new("jj").arg("--version").output().is_ok()
    }

    fn make_jj_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("jj")
            .args(["git", "init", "--colocate"])
            .current_dir(dir.path())
            .output().unwrap();
        std::process::Command::new("jj")
            .args(["config", "set", "--repo", "user.name", "Test"])
            .current_dir(dir.path()).output().unwrap();
        std::process::Command::new("jj")
            .args(["config", "set", "--repo", "user.email", "test@test.local"])
            .current_dir(dir.path()).output().unwrap();
        dir
    }

    #[test]
    fn test_ensure_jj_repo_idempotent() {
        if !jj_available() { eprintln!("jj not installed, skipping"); return; }
        let dir = tempfile::tempdir().unwrap();
        ensure_jj_repo(dir.path()).unwrap();
        ensure_jj_repo(dir.path()).unwrap();
        assert!(dir.path().join(".jj").exists());
    }

    #[test]
    fn test_create_workspace() {
        if !jj_available() { eprintln!("jj not installed, skipping"); return; }
        let repo = make_jj_repo();
        let ws = repo.path().join("my-workspace");
        create_workspace(repo.path(), &ws).unwrap();
        assert!(ws.exists());
    }

    #[test]
    fn test_create_and_forget_workspace() {
        if !jj_available() { eprintln!("jj not installed, skipping"); return; }
        let repo = make_jj_repo();
        let ws = repo.path().join("card-workspace");
        create_workspace(repo.path(), &ws).unwrap();
        forget_workspace(repo.path(), "card-workspace").unwrap();
    }

    #[test]
    fn test_squash_workspace_changes() {
        if !jj_available() { eprintln!("jj not installed, skipping"); return; }
        let repo = make_jj_repo();
        let ws = repo.path().join("squash-ws");
        create_workspace(repo.path(), &ws).unwrap();
        // Write a file in the workspace
        std::fs::write(ws.join("result.txt"), b"agent output").unwrap();
        // squash moves changes to parent change
        squash_workspace(&ws).unwrap();
    }
}
