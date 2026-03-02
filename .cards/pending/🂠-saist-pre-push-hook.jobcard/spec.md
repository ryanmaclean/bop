# SAIST pre-push safety gate

## Problem
`bop merge-gate` and manual `jj git push` can accidentally push:
- API keys from `.cards/providers.json` or agent logs
- Screenshot thumbnails (QuickLook/Thumbnail.png) with sensitive UI
- Agent stdout with echoed secrets
- Any sensitive text written by agents to output/result.md

This is catastrophic if pushing to a public remote.

## Solution
Run [datadog/saist](https://github.com/DataDog/saist) as a pre-push check in the
merge-gate BEFORE any `jj git push` call.

saist is a free, offline CLI that detects secrets/PII patterns in text.
Install: `pip install datadog-saist` or `brew install saist` (if available).

## Implementation

In `crates/jobcard-core/src/worktree.rs`, add a `scan_for_secrets` function:

```rust
/// Run datadog-saist on the diff about to be pushed.
/// Returns Err if secrets found, Ok if clean or saist not installed (best-effort).
pub fn scan_for_secrets(repo_root: &Path) -> Result<()> {
    // Get the diff of what would be pushed
    let diff = std::process::Command::new("jj")
        .args(["diff", "--git"])
        .current_dir(repo_root)
        .output();

    let Ok(diff_out) = diff else { return Ok(()); }; // jj not available = skip
    if diff_out.stdout.is_empty() { return Ok(()); }

    // Try saist; if not installed, warn but don't block
    let saist = std::process::Command::new("saist")
        .args(["scan", "--stdin", "--format", "json"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();

    let Ok(mut child) = saist else {
        eprintln!("saist not found — skipping secret scan (install: pip install datadog-saist)");
        return Ok(());
    };

    if let Some(stdin) = child.stdin.take() {
        use std::io::Write;
        let mut stdin = stdin;
        let _ = stdin.write_all(&diff_out.stdout);
    }

    let output = child.wait_with_output()?;
    if !output.status.success() {
        let report = String::from_utf8_lossy(&output.stdout);
        anyhow::bail!("saist found potential secrets in diff:\n{}", report);
    }
    Ok(())
}
```

In `push_stack`, call `scan_for_secrets` before the git push:
```rust
pub fn push_stack(repo_root: &Path, remote: &str) -> Result<()> {
    scan_for_secrets(repo_root)?;
    // ... existing push logic
}
```

## .gitignore additions
Add to `.gitignore`:
```
.cards/*/logs/
.cards/*/output/
.cards/*/QuickLook/
```
These directories should NEVER be pushed — they contain runtime artifacts.

## Acceptance Criteria
- `cargo build`
- `cargo clippy -- -D warnings`
- `grep -q 'scan_for_secrets' crates/jobcard-core/src/worktree.rs`
- `grep -q 'cards.*logs' .gitignore || grep -q 'QuickLook' .gitignore`
- `jj log -r 'main..@-' | grep -q .`
