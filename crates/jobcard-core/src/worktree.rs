use std::path::Path;
use anyhow::{Context, Result};

/// Create a git worktree at `wt_path` on branch `branch_name`.
/// If the branch already exists, attaches the worktree to it.
pub fn create_worktree(git_root: &Path, wt_path: &Path, branch_name: &str) -> Result<()> {
    // Try creating a new branch with the worktree
    let result = std::process::Command::new("git")
        .args(["worktree", "add", "-b", branch_name])
        .arg(wt_path)
        .current_dir(git_root)
        .output()
        .context("failed to run git worktree add -b")?;

    if result.status.success() {
        return Ok(());
    }

    // Branch already exists — attach worktree to existing branch
    let result = std::process::Command::new("git")
        .args(["worktree", "add"])
        .arg(wt_path)
        .arg(branch_name)
        .current_dir(git_root)
        .output()
        .context("failed to run git worktree add")?;

    if result.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&result.stderr);
    anyhow::bail!("git worktree add failed: {}", stderr);
}

/// Stage all changes in the worktree and commit with a standard message.
/// Uses `--allow-empty` so cards with only log output still produce a commit.
pub fn commit_worktree(wt_path: &Path, card_id: &str) -> Result<()> {
    let env_vars = [
        ("GIT_AUTHOR_NAME", "jobcard-agent"),
        ("GIT_AUTHOR_EMAIL", "agent@jobcard.local"),
        ("GIT_COMMITTER_NAME", "jobcard-agent"),
        ("GIT_COMMITTER_EMAIL", "agent@jobcard.local"),
    ];

    // Stage all changes
    let result = std::process::Command::new("git")
        .args(["add", "-A"])
        .current_dir(wt_path)
        .envs(env_vars)
        .output()
        .context("failed to run git add -A")?;

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        anyhow::bail!("git add -A failed: {}", stderr);
    }

    // Commit with standard message
    let message = format!("feat(jobcard): complete {}", card_id);
    let result = std::process::Command::new("git")
        .args(["commit", "--allow-empty", "-m", &message])
        .current_dir(wt_path)
        .envs(env_vars)
        .output()
        .context("failed to run git commit")?;

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        anyhow::bail!("git commit failed: {}", stderr);
    }

    Ok(())
}

/// Merge `branch_name` into the current HEAD of `git_root`.
/// Returns `true` if merge succeeded, `false` if there's a conflict.
pub fn merge_card_branch(git_root: &Path, branch_name: &str) -> Result<bool> {
    let message = format!("Merge {} via merge-gate", branch_name);
    let out = std::process::Command::new("git")
        .args(["merge", "--no-ff", branch_name, "-m", &message])
        .current_dir(git_root)
        .output()
        .context("failed to run git merge")?;

    Ok(out.status.success())
}

/// Prune and remove the worktree at `wt_path`.
pub fn remove_worktree(git_root: &Path, wt_path: &Path) -> Result<()> {
    let result = std::process::Command::new("git")
        .args(["worktree", "remove", "--force"])
        .arg(wt_path)
        .current_dir(git_root)
        .output()
        .context("failed to run git worktree remove")?;

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        anyhow::bail!("git worktree remove failed: {}", stderr);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_git_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();

        // git init -b main
        let out = std::process::Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(path)
            .output()
            .expect("git init failed");
        assert!(out.status.success(), "git init: {}", String::from_utf8_lossy(&out.stderr));

        // Configure user identity
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.local"])
            .current_dir(path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(path)
            .output()
            .unwrap();

        // Initial empty commit so HEAD exists
        let out = std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(path)
            .output()
            .expect("initial commit failed");
        assert!(out.status.success(), "init commit: {}", String::from_utf8_lossy(&out.stderr));

        dir
    }

    #[test]
    fn test_create_and_remove_worktree() {
        let repo = make_git_repo();
        let repo_path = repo.path();

        let wt_dir = tempfile::tempdir().unwrap();
        let wt_path = wt_dir.path().join("my-worktree");

        // create_worktree should create the directory
        create_worktree(repo_path, &wt_path, "feature/test-wt")
            .expect("create_worktree failed");
        assert!(wt_path.exists(), "worktree directory should exist after creation");

        // remove_worktree should delete it
        remove_worktree(repo_path, &wt_path)
            .expect("remove_worktree failed");
        assert!(!wt_path.exists(), "worktree directory should be gone after removal");
    }

    #[test]
    fn test_commit_worktree_changes() {
        let repo = make_git_repo();
        let repo_path = repo.path();

        let wt_dir = tempfile::tempdir().unwrap();
        let wt_path = wt_dir.path().join("commit-wt");

        let branch = "feature/commit-test";
        create_worktree(repo_path, &wt_path, branch)
            .expect("create_worktree failed");

        // Write a file inside the worktree
        fs::write(wt_path.join("hello.txt"), b"hello from worktree\n")
            .expect("write file failed");

        // Commit from inside the worktree
        commit_worktree(&wt_path, "CARD-42")
            .expect("commit_worktree failed");

        // Verify the branch appears in `git branch` output
        let out = std::process::Command::new("git")
            .args(["branch"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        let branches = String::from_utf8_lossy(&out.stdout);
        assert!(
            branches.contains("feature/commit-test"),
            "branch should appear after commit; got: {}",
            branches
        );
    }

    #[test]
    fn test_merge_card_branch() {
        let repo = make_git_repo();
        let repo_path = repo.path();

        let wt_dir = tempfile::tempdir().unwrap();
        let wt_path = wt_dir.path().join("merge-wt");

        let branch = "feature/merge-test";
        create_worktree(repo_path, &wt_path, branch)
            .expect("create_worktree failed");

        // Write and commit a file in the worktree
        fs::write(wt_path.join("merged.txt"), b"merged content\n")
            .expect("write file failed");
        commit_worktree(&wt_path, "CARD-99")
            .expect("commit_worktree failed");

        // Remove the worktree before merging (not strictly required, but clean)
        remove_worktree(repo_path, &wt_path)
            .expect("remove_worktree failed");

        // Merge the branch into main
        let merged = merge_card_branch(repo_path, branch)
            .expect("merge_card_branch failed");
        assert!(merged, "merge should succeed");

        // Verify the file now exists in the main repo working tree
        assert!(
            repo_path.join("merged.txt").exists(),
            "merged.txt should appear in main repo after merge"
        );
    }
}
