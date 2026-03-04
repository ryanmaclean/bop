use std::path::Path;
use std::process::Command as StdCommand;

use anyhow::Context;

use crate::paths;
use crate::workspace::find_git_root;

pub fn run_policy_script(cwd: &Path, args: &[&str]) -> anyhow::Result<std::process::Output> {
    // Prefer a script relative to the actual git root so the binary works
    // regardless of where it was compiled (avoids stale CARGO_MANIFEST_DIR).
    let git_root_candidate = find_git_root(cwd)
        .map(|r| r.join("scripts").join("policy_check.zsh"))
        .unwrap_or_else(|| cwd.join("scripts").join("policy_check.zsh"));
    let script_candidates = [
        git_root_candidate,
        cwd.join("scripts").join("policy_check.zsh"),
    ];
    let script = script_candidates
        .iter()
        .find(|p| p.exists())
        .cloned()
        .with_context(|| {
            format!(
                "policy script missing (checked: {}, {})",
                script_candidates[0].display(),
                script_candidates[1].display()
            )
        })?;
    let output = StdCommand::new("zsh")
        .arg(script)
        .args(args)
        .current_dir(cwd)
        .output()
        .context("failed to run policy_check.zsh")?;
    Ok(output)
}

pub fn cmd_policy_check(cards_root: &Path, id: Option<&str>, staged: bool) -> anyhow::Result<()> {
    let repo_root = find_git_root(cards_root).unwrap_or(std::env::current_dir()?);
    let cards_dir_arg = cards_root.to_string_lossy().to_string();

    let output = if staged || id.is_none() {
        run_policy_script(
            &repo_root,
            &["--staged", "--cards-dir", cards_dir_arg.as_str()],
        )?
    } else {
        let card_id = id.unwrap_or_default().trim();
        if card_id.is_empty() {
            anyhow::bail!("card id cannot be empty");
        }
        let card_dir = paths::find_card(cards_root, card_id).context("card not found")?;
        let card_dir_arg = card_dir.to_string_lossy().to_string();
        run_policy_script(
            &repo_root,
            &[
                "--mode",
                "card",
                "--cards-dir",
                cards_dir_arg.as_str(),
                "--id",
                card_id,
                "--card-dir",
                card_dir_arg.as_str(),
            ],
        )?
    };

    if !output.stdout.is_empty() {
        print!("{}", String::from_utf8_lossy(&output.stdout));
    }
    if !output.stderr.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
    }
    if !output.status.success() {
        anyhow::bail!("policy check failed");
    }
    Ok(())
}

pub fn policy_check_card(cards_root: &Path, card_dir: &Path, card_id: &str) -> anyhow::Result<()> {
    let repo_root = find_git_root(cards_root).unwrap_or(std::env::current_dir()?);
    let cards_dir_arg = cards_root.to_string_lossy().to_string();
    let card_dir_arg = card_dir.to_string_lossy().to_string();
    let output = run_policy_script(
        &repo_root,
        &[
            "--mode",
            "card",
            "--cards-dir",
            cards_dir_arg.as_str(),
            "--id",
            card_id,
            "--card-dir",
            card_dir_arg.as_str(),
        ],
    )?;

    if !output.stdout.is_empty() {
        print!("{}", String::from_utf8_lossy(&output.stdout));
    }
    if !output.stderr.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
    }
    if !output.status.success() {
        anyhow::bail!("policy violation");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn run_policy_script_fails_without_script() {
        let td = tempdir().unwrap();
        // No scripts/policy_check.zsh exists — should error
        let result = run_policy_script(td.path(), &["--staged"]);
        assert!(result.is_err());
    }

    #[test]
    fn cmd_policy_check_with_nonexistent_card_id() {
        let td = tempdir().unwrap();
        // Looking up a card that doesn't exist should fail
        let result = cmd_policy_check(td.path(), Some("nonexistent-card-xyz"), false);
        assert!(result.is_err());
    }
}
