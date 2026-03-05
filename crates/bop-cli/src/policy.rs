use std::path::Path;
use std::process::Command as StdCommand;

use anyhow::Context;

use crate::workspace::find_git_root;

// ---------------------------------------------------------------------------
// Pure Rust policy engine
// ---------------------------------------------------------------------------

const RUNTIME_STATES: &[&str] = &["running", "done", "failed", "merged"];

/// Parse `git diff --numstat` output. Returns (added, deleted, path) tuples.
/// Binary files (shown as "-\t-\tpath") are skipped.
pub fn parse_numstat(raw: &str) -> Vec<(u64, u64, String)> {
    raw.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let a = parts.next()?;
            let d = parts.next()?;
            let p = parts.next()?.to_string();
            let added = a.parse::<u64>().ok()?;
            let deleted = d.parse::<u64>().ok()?;
            Some((added, deleted, p))
        })
        .collect()
}

/// Parse `git diff --name-status --no-renames` output. Returns (status, path) pairs.
/// NOTE: Requires `--no-renames` flag — rename entries (R/C) have 3 tab-separated fields
/// and are not handled correctly without it.
pub fn parse_name_status(raw: &str) -> Vec<(String, String)> {
    raw.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(2, '\t');
            let status = parts.next()?.trim().to_string();
            let path = parts.next()?.trim().to_string();
            if status.is_empty() || path.is_empty() {
                return None;
            }
            Some((status, path))
        })
        .collect()
}

/// Returns true if this path is a runtime card state dir that should not be committed.
pub fn is_runtime_path(path: &str) -> bool {
    let norm = path.replace('\\', "/");
    RUNTIME_STATES
        .iter()
        .any(|state| norm.contains(&format!(".cards/{state}/")))
}

/// Check file/LOC counts against thresholds. Returns violation strings (empty = pass).
pub fn check_thresholds(
    changed_files: usize,
    changed_loc: u64,
    max_files: usize,
    max_loc: u64,
) -> Vec<String> {
    let mut reasons = Vec::new();
    if changed_files > max_files {
        reasons.push(format!(
            "changed_files {} exceeds limit {}",
            changed_files, max_files
        ));
    }
    if changed_loc > max_loc {
        reasons.push(format!(
            "changed_loc {} exceeds limit {}",
            changed_loc, max_loc
        ));
    }
    reasons
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct PolicyResult {
    pub ok: bool,
    pub reasons: Vec<String>,
    pub metrics: PolicyMetrics,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<bool>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct PolicyMetrics {
    pub changed_files: usize,
    pub changed_loc: u64,
}

/// Run the full policy check against the staged diff from git_root.
pub fn run_staged_policy_check(git_root: &Path) -> anyhow::Result<PolicyResult> {
    // Check if this is a git repo
    let check = StdCommand::new("git")
        .args([
            "-C",
            &git_root.to_string_lossy(),
            "rev-parse",
            "--is-inside-work-tree",
        ])
        .output()?;

    if !check.status.success() {
        return Ok(PolicyResult {
            ok: true,
            reasons: vec![format!("skipped: no git repo at {}", git_root.display())],
            metrics: PolicyMetrics {
                changed_files: 0,
                changed_loc: 0,
            },
            skipped: Some(true),
        });
    }

    let run_git = |extra_args: &[&str]| -> anyhow::Result<String> {
        let root_str = git_root.to_str().unwrap_or(".");
        let mut args = vec![
            "-C",
            root_str,
            "-c",
            "core.quotepath=false",
            "diff",
            "--cached",
            "--no-renames",
        ];
        args.extend_from_slice(extra_args);
        let out = StdCommand::new("git").args(&args).output()?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            anyhow::bail!(
                "git diff failed (exit {}): {}",
                out.status.code().unwrap_or(-1),
                stderr.trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    };

    let name_status_raw = run_git(&["--name-status"])?;
    let numstat_raw = run_git(&["--numstat"])?;

    let ns = parse_name_status(&name_status_raw);
    let numstat = parse_numstat(&numstat_raw);

    let changed_files = ns.iter().filter(|(_, p)| !is_runtime_path(p)).count();
    let changed_loc: u64 = numstat
        .iter()
        .filter(|(_, _, p)| !is_runtime_path(p))
        .map(|(a, d, _)| a + d)
        .sum();

    let max_files = 50usize;
    let max_loc = 2000u64;

    let mut reasons = check_thresholds(changed_files, changed_loc, max_files, max_loc);
    reasons = reasons
        .into_iter()
        .map(|r| format!("policy violation: {r}"))
        .collect();

    Ok(PolicyResult {
        ok: reasons.is_empty(),
        reasons,
        metrics: PolicyMetrics {
            changed_files,
            changed_loc,
        },
        skipped: None,
    })
}

// Used in tests to verify the Nu shim calling convention. Not called from
// production code paths since card-mode now returns an explicit error instead
// of delegating to the shim.
#[allow(dead_code)]
pub fn run_policy_script(cwd: &Path, args: &[&str]) -> anyhow::Result<std::process::Output> {
    // Prefer a script relative to the actual git root so the binary works
    // regardless of where it was compiled (avoids stale CARGO_MANIFEST_DIR).
    let git_root_candidate = find_git_root(cwd)
        .map(|r| r.join("scripts").join("policy_check.nu"))
        .unwrap_or_else(|| cwd.join("scripts").join("policy_check.nu"));
    let script_candidates = [
        git_root_candidate,
        cwd.join("scripts").join("policy_check.nu"),
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
    let output = StdCommand::new("nu")
        .arg(script)
        .args(args)
        .current_dir(cwd)
        .output()
        .context("failed to run policy_check.nu")?;
    Ok(output)
}

pub fn cmd_policy_check(
    cards_root: &Path,
    id: Option<&str>,
    _staged: bool,
    json: bool,
) -> anyhow::Result<()> {
    // Card-mode: not yet implemented in the Rust policy engine. The Nu shim
    // (scripts/policy_check.nu) ignores --mode card and silently runs a
    // staged-diff check instead, which is wrong. Fail loudly rather than
    // produce a misleading result.
    if let Some(card_id) = id {
        let card_id = card_id.trim();
        if card_id.is_empty() {
            anyhow::bail!("card id cannot be empty");
        }
        anyhow::bail!(
            "bop policy check --id is not yet implemented in the Rust policy engine. \
             Run: nu scripts/policy_check.nu --mode card --id {} instead.",
            card_id
        );
    }

    // Staged mode: pure Rust implementation.
    let git_root = find_git_root(cards_root)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| cards_root.to_path_buf()));
    let result = run_staged_policy_check(&git_root)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else if result.ok {
        if result.skipped.unwrap_or(false) {
            println!(
                "policy skipped: {}",
                result
                    .reasons
                    .first()
                    .map(|s| s.as_str())
                    .unwrap_or("no git repo")
            );
        } else {
            println!(
                "policy ok — {} files, {} LOC",
                result.metrics.changed_files, result.metrics.changed_loc
            );
        }
    } else {
        for r in &result.reasons {
            eprintln!("{r}");
        }
        std::process::exit(1);
    }
    Ok(())
}

pub fn policy_check_card(
    cards_root: &Path,
    _card_dir: &Path,
    _card_id: &str,
) -> anyhow::Result<()> {
    // Run staged policy check in the repo root. Card-specific meta checks
    // (scope, decision records) are handled by the merge-gate acceptance criteria.
    let git_root = find_git_root(cards_root)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| cards_root.to_path_buf()));
    let result = run_staged_policy_check(&git_root)?;
    if !result.ok {
        let msg = result.reasons.join("; ");
        anyhow::bail!("{}", msg);
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
        // No scripts/policy_check.nu exists — should error
        let result = run_policy_script(td.path(), &["--staged"]);
        assert!(result.is_err());
    }

    #[test]
    fn cmd_policy_check_with_nonexistent_card_id() {
        let td = tempdir().unwrap();
        // Looking up a card that doesn't exist should fail
        let result = cmd_policy_check(td.path(), Some("nonexistent-card-xyz"), false, false);
        assert!(result.is_err());
    }

    #[test]
    fn parse_numstat_parses_standard_output() {
        let raw = "10\t3\tsrc/main.rs\n0\t5\tCargo.toml\n";
        let rows = parse_numstat(raw);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], (10u64, 3u64, "src/main.rs".to_string()));
        assert_eq!(rows[1], (0u64, 5u64, "Cargo.toml".to_string()));
    }

    #[test]
    fn parse_numstat_skips_binary_files() {
        let raw = "10\t3\tsrc/main.rs\n-\t-\tassets/img.png\n";
        let rows = parse_numstat(raw);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].2, "src/main.rs");
    }

    #[test]
    fn is_runtime_path_filters_card_state_dirs() {
        assert!(is_runtime_path(".cards/running/my.bop/meta.json"));
        assert!(is_runtime_path(".cards/done/x.bop/logs/stdout.log"));
        assert!(!is_runtime_path(".cards/templates/implement.bop/meta.json"));
        assert!(!is_runtime_path("src/main.rs"));
    }

    #[test]
    fn is_runtime_path_filters_non_bop_bundles() {
        // Bare id without .bop extension should still be filtered
        assert!(is_runtime_path(".cards/running/some-card/meta.json"));
        // templates are never runtime
        assert!(!is_runtime_path(".cards/templates/implement.bop/meta.json"));
    }

    #[test]
    fn check_thresholds_passes_under_limit() {
        let reasons = check_thresholds(5, 100, 50, 2000);
        assert!(reasons.is_empty());
    }

    #[test]
    fn check_thresholds_fails_over_loc_limit() {
        let reasons = check_thresholds(5, 2500, 50, 2000);
        assert!(!reasons.is_empty());
        assert!(reasons[0].contains("changed_loc"));
    }
}
