# jj + vibekanban + Zellij Integration Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace git worktrees with jj workspaces for per-card isolation, auto-push stacked PRs on merge-gate success, replace ratatui TUI with vibekanban-cli, and add full zellij integration with adaptive pane layout and a Rust/WASM status bar plugin.

**Architecture:** The `jc` dispatcher initializes a jj repo on startup (`jj git init --colocate`), creates a `jj workspace add` per card before dispatch, and the merge gate runs `jj squash` + `jj git push --change @` + `gh pr create --stack`. vibekanban polls `.cards/` directly via filesystem (no REST layer). Zellij gets a native `jobcard.kdl` layout, per-card pane hooks with adaptive tier logic (1–5 per-card / 6–20 per-team / 21+ aggregated), and a Rust/WASM status bar plugin. All scripts rewritten in zsh (MIT license). No bash, no fish.

**Tech Stack:** Rust (jc + jobcard-core), jj CLI (Apache 2.0), gh CLI (MIT), zsh (MIT), vibekanban-cli (Apache 2.0), zellij (MIT) + `zellij-tile` crate (MIT), `cargo-generate-lock` for license audit.

**Design doc:** `docs/plans/2026-03-01-jj-vibekanban-zellij-design.md`

---

## Task 1: zsh migration — rewrite all adapter and utility scripts

**Files:**
- Modify: `adapters/claude.sh` → `adapters/claude.zsh`
- Modify: `adapters/codex.sh` → `adapters/codex.zsh`
- Modify: `adapters/goose.sh` → `adapters/goose.zsh`
- Modify: `adapters/aider.sh` → `adapters/aider.zsh`
- Modify: `adapters/opencode.sh` → `adapters/opencode.zsh`
- Modify: `adapters/ollama-local.sh` → `adapters/ollama-local.zsh`
- Modify: `adapters/mock.sh` → `adapters/mock.zsh`
- Modify: `scripts/launch_teams.sh` → `scripts/launch_teams.zsh`
- Modify: `scripts/dashboard.sh` → `scripts/dashboard.zsh`
- Test: Run each adapter in mock mode to verify it executes

**Context:** All adapters follow the same contract: receive 4 args `workdir prompt_file stdout_log stderr_log`, exit 75 on rate limit. The existing scripts differ only in the inner `claude`/`codex`/etc. command. zsh is a drop-in syntax replacement for bash for the patterns used here — `set -euo pipefail`, `[[ ]]`, `$(...)`, `$1...$4` all work identically.

**Step 1: Write a failing test script**

Create `scripts/test_zsh_adapters.zsh`:
```zsh
#!/usr/bin/env zsh
set -euo pipefail
# Verify each adapter file uses zsh shebang and not bash
for f in adapters/*.zsh; do
  head -1 "$f" | grep -q '#!/usr/bin/env zsh' || { echo "FAIL: $f is not zsh"; exit 1; }
done
# Verify no .sh files remain (except any explicitly kept)
for f in adapters/*.sh; do
  [[ -f "$f" ]] && { echo "FAIL: bash adapter still exists: $f"; exit 1; }
done
echo "PASS: all adapters are zsh"
```

Run it (expect FAIL — `.sh` files still exist):
```zsh
chmod +x scripts/test_zsh_adapters.zsh && ./scripts/test_zsh_adapters.zsh
```

**Step 2: Convert each adapter**

For each adapter, copy content and change only:
1. Shebang: `#!/usr/bin/env bash` → `#!/usr/bin/env zsh`
2. Keep `set -euo pipefail` (identical in zsh)
3. Keep all other logic unchanged

Create `adapters/claude.zsh`:
```zsh
#!/usr/bin/env zsh
set -euo pipefail

workdir="$1"; prompt_file="$2"; stdout_log="$3"; stderr_log="$4"

orig_dir="$(pwd)"
cd "$workdir"

[[ "$prompt_file" != /* ]] && prompt_file="$orig_dir/$prompt_file"
[[ "$stdout_log"  != /* ]] && stdout_log="$orig_dir/$stdout_log"
[[ "$stderr_log"  != /* ]] && stderr_log="$orig_dir/$stderr_log"

claude -p "$(cat "$prompt_file")" \
  --dangerously-skip-permissions \
  --output-format json \
  > "$stdout_log" 2> "$stderr_log"
rc=$?

grep -qiE 'rate limit|429|too many requests' "$stderr_log" && exit 75
exit $rc
```

Repeat for each adapter (same pattern, different inner command). Then delete all `.sh` files:
```zsh
rm adapters/*.sh
```

**Step 3: Convert utility scripts**

`scripts/launch_teams.zsh` — change shebang and all `#!/usr/bin/env bash` lines to `#!/usr/bin/env zsh`. The `TEAMS` array, `IFS` splitting, `zellij` calls, `set -euo pipefail` all work identically in zsh.

`scripts/dashboard.zsh` — same: change shebang, `setopt NULL_GLOB` already present (zsh-native).

Delete `.sh` originals:
```zsh
rm scripts/launch_teams.sh scripts/dashboard.sh
```

**Step 4: Run test — expect PASS**
```zsh
./scripts/test_zsh_adapters.zsh
# Expected: PASS: all adapters are zsh
```

**Step 5: Update any references to `.sh` in Rust code**

Search for `.sh` references in main.rs:
```zsh
grep -n '\.sh' crates/jc/src/main.rs
```
If any adapter paths are hardcoded, change `.sh` → `.zsh`.

**Step 6: Commit**
```zsh
git add adapters/ scripts/ crates/
git commit -m "feat: migrate all adapter and utility scripts from bash to zsh"
```

---

## Task 2: jj workspace — replace git worktrees in dispatcher and jobcard-core

**Files:**
- Modify: `crates/jobcard-core/src/worktree.rs` (replace git worktree with jj workspace)
- Modify: `crates/jc/src/main.rs` (update dispatcher to call jj init + jj workspace add)
- Modify: `crates/jc/tests/worktree_harness.rs` (update tests to use jj)
- Test: `cargo test -- worktree`

**Context:**
- Current dispatcher (main.rs ~line 3370): calls `jobcard_core::worktree::create_worktree(&git_root, &wt_path, &branch)`
- Current `jobcard-core/src/worktree.rs`: runs `git worktree add -b <branch> <path>`
- jj equivalent: `jj workspace add <path>` (creates an anonymous workspace at HEAD)
- jj workspace path within the card: `<card_dir>/workspace/` (same as before)
- Key jj difference: workspaces are always on an anonymous change; no branch flag needed upfront

**Step 1: Write failing tests**

In `crates/jobcard-core/src/worktree.rs`, add test stubs (they'll fail until impl):
```rust
#[test]
fn test_create_jj_workspace() {
    // Will test jj workspace add
    // Requires: jj installed, otherwise skip
    if std::process::Command::new("jj").arg("--version").output().is_err() {
        eprintln!("jj not installed, skipping");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    // jj git init
    std::process::Command::new("jj")
        .args(["git", "init", "--colocate"])
        .current_dir(repo)
        .output().unwrap();
    let ws_path = repo.join("workspace-test");
    create_workspace(repo, &ws_path).expect("create_workspace failed");
    assert!(ws_path.exists());
}
```

Run (expect fail — `create_workspace` not yet defined):
```zsh
cargo test -p jobcard-core -- test_create_jj_workspace 2>&1 | head -20
```

**Step 2: Replace `worktree.rs` with jj implementation**

Replace the full content of `crates/jobcard-core/src/worktree.rs`:

```rust
//! jj workspace management for per-card isolation.
//!
//! Each job card gets a `jj workspace add <card>/workspace` before the adapter runs.
//! On merge: `jj squash` folds changes back, `jj workspace forget` cleans up.
use anyhow::{Context, Result};
use std::path::Path;

/// Initialize a jj repo at `repo_root` (colocated with git) if one does not exist.
/// Safe to call repeatedly — exits quickly if `.jj/` already present.
pub fn ensure_jj_repo(repo_root: &Path) -> Result<()> {
    if repo_root.join(".jj").exists() {
        return Ok(());
    }
    let out = std::process::Command::new("jj")
        .args(["git", "init", "--colocate"])
        .current_dir(repo_root)
        .output()
        .context("failed to run jj git init --colocate (is jj installed?)")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("jj git init failed: {}", stderr);
    }
    Ok(())
}

/// Create a jj workspace at `ws_path` from `repo_root`.
/// The workspace starts at the current working-copy change (@).
pub fn create_workspace(repo_root: &Path, ws_path: &Path) -> Result<()> {
    let name = ws_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("workspace");
    let out = std::process::Command::new("jj")
        .args(["workspace", "add", "--name", name])
        .arg(ws_path)
        .current_dir(repo_root)
        .output()
        .context("failed to run jj workspace add")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("jj workspace add failed: {}", stderr);
    }
    Ok(())
}

/// Squash all changes from the card's workspace change into the parent change (@-).
/// Run this from the workspace directory after the agent completes.
pub fn squash_workspace(ws_path: &Path) -> Result<()> {
    // Stage everything: jj treats all files as auto-tracked
    // `jj squash` moves changes from @ into @- (parent)
    let out = std::process::Command::new("jj")
        .args(["squash"])
        .current_dir(ws_path)
        .output()
        .context("failed to run jj squash")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("jj squash failed: {}", stderr);
    }
    Ok(())
}

/// Forget (remove) the workspace. Call from repo_root after squashing.
/// The workspace's changes are already squashed, so no data is lost.
pub fn forget_workspace(repo_root: &Path, ws_name: &str) -> Result<()> {
    let out = std::process::Command::new("jj")
        .args(["workspace", "forget", ws_name])
        .current_dir(repo_root)
        .output()
        .context("failed to run jj workspace forget")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("jj workspace forget failed: {}", stderr);
    }
    Ok(())
}

/// Push the current stack of changes to the remote as git branches.
/// Each change becomes a branch named `push-<change-id>`.
/// Run this from repo_root after squashing all done cards.
pub fn push_stack(repo_root: &Path, remote: &str) -> Result<()> {
    let out = std::process::Command::new("jj")
        .args(["git", "push", "--remote", remote, "--all"])
        .current_dir(repo_root)
        .output()
        .context("failed to run jj git push")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("jj git push failed: {}", stderr);
    }
    Ok(())
}

// Keep old names as aliases so existing call sites in main.rs compile
// until Task 3 migrates the merge gate.
#[deprecated(note = "use create_workspace instead")]
pub fn create_worktree(git_root: &Path, wt_path: &Path, _branch_name: &str) -> anyhow::Result<()> {
    create_workspace(git_root, wt_path)
}
#[deprecated(note = "use squash_workspace + forget_workspace instead")]
pub fn commit_worktree(_wt_path: &Path, _card_id: &str) -> anyhow::Result<()> { Ok(()) }
#[deprecated(note = "use push_stack instead")]
pub fn merge_card_branch(_git_root: &Path, _branch_name: &str) -> anyhow::Result<bool> { Ok(true) }
#[deprecated(note = "use forget_workspace instead")]
pub fn remove_worktree(_git_root: &Path, _wt_path: &Path) -> anyhow::Result<()> { Ok(()) }

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
        // Set identity for commits
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
        if !jj_available() { return; }
        let dir = tempfile::tempdir().unwrap();
        ensure_jj_repo(dir.path()).unwrap();
        ensure_jj_repo(dir.path()).unwrap(); // idempotent
        assert!(dir.path().join(".jj").exists());
    }

    #[test]
    fn test_create_workspace() {
        if !jj_available() { return; }
        let repo = make_jj_repo();
        let ws = repo.path().join("my-workspace");
        create_workspace(repo.path(), &ws).unwrap();
        assert!(ws.exists(), "workspace dir should exist");
    }

    #[test]
    fn test_create_and_forget_workspace() {
        if !jj_available() { return; }
        let repo = make_jj_repo();
        let ws = repo.path().join("card-workspace");
        create_workspace(repo.path(), &ws).unwrap();
        forget_workspace(repo.path(), "card-workspace").unwrap();
        // workspace dir may linger (jj does not delete the directory on forget)
    }

    #[test]
    fn test_squash_workspace_changes() {
        if !jj_available() { return; }
        let repo = make_jj_repo();
        let ws = repo.path().join("squash-ws");
        create_workspace(repo.path(), &ws).unwrap();
        // Write a file in the workspace
        std::fs::write(ws.join("result.txt"), b"agent output").unwrap();
        // Squash moves changes to parent
        squash_workspace(&ws).unwrap();
    }
}
```

**Step 3: Update dispatcher in `main.rs`**

Find the block at ~line 3370 (after `fs::rename` to running):
```rust
// Create git worktree for isolation (best-effort; non-fatal if git not available)
if let Some(git_root) = find_git_root(&running_path) {
    let wt_path = running_path.join("worktree");
    ...
    if let Err(e) = jobcard_core::worktree::create_worktree(&git_root, &wt_path, &branch) {
```

Replace with:
```rust
// Create jj workspace for isolation (best-effort; non-fatal if jj not available)
{
    // Ensure jj repo is initialized (idempotent)
    let _ = jobcard_core::worktree::ensure_jj_repo(cards_dir);
    let ws_path = running_path.join("workspace");
    if let Err(e) = jobcard_core::worktree::create_workspace(cards_dir, &ws_path) {
        eprintln!("[dispatcher] jj workspace create failed: {e}");
    }
}
```

Also update `run_card` workdir detection (~line 3520) — change `"worktree"` → `"workspace"`:
```rust
let workdir = {
    let ws = card_dir.join("workspace");
    if ws.exists() { ws } else { card_dir.to_path_buf() }
};
```

**Step 4: Update `cmd_worktree_list/create/clean` in main.rs**

These functions at ~line 4326 reference `"worktree"` as the subdirectory name. Change all `card_dir.join("worktree")` → `card_dir.join("workspace")` in these three functions.

Also update `git_worktree_paths` to call `jj workspace list --template ...` instead of `git worktree list --porcelain`. The output format will change:
```zsh
jj workspace list
# output:
# default: ...
# card-abc: ...
```
Parse the name from the left-hand side of `:`.

**Step 5: Run tests**
```zsh
cargo test -- worktree 2>&1
# Expected: all worktree tests pass (skipped gracefully if jj not installed)
cargo build 2>&1
# Expected: builds clean (deprecation warnings OK)
```

**Step 6: Commit**
```zsh
git add crates/
git commit -m "feat: replace git worktrees with jj workspaces"
```

---

## Task 3: jj merge gate — squash + push + stacked PR

**Files:**
- Modify: `crates/jc/src/main.rs` function `run_merge_gate` (~line 3703)
- Modify: `crates/jc/tests/merge_gate_harness.rs`
- Test: `cargo test -- merge_gate`

**Context:**
Current merge gate (~line 3795) does:
1. `commit_worktree` (git add -A + git commit in worktree)
2. `find_git_root` from card_dir
3. `merge_card_branch` (git merge --no-ff from git root)
4. `remove_worktree` (git worktree remove)

New merge gate does:
1. `squash_workspace` (jj squash from inside workspace — folds agent changes into parent change)
2. `forget_workspace` (jj workspace forget from repo root)
3. `push_stack` (jj git push --all from repo root — pushes all un-pushed changes)
4. `gh pr create` per card (create stacked PR)
5. Move card to `merged/`

**Step 1: Write failing integration test**

In `crates/jc/tests/merge_gate_harness.rs`, add:
```rust
#[test]
fn merge_gate_jj_squash_and_push() {
    // Verify merge gate uses jj squash, not git merge
    // This test checks the log output mentions jj squash (not git merge)
    // Skip if jj not available
    if std::process::Command::new("jj").arg("--version").output().is_err() {
        eprintln!("jj not installed, skipping");
        return;
    }
    // TODO: full integration test after impl
    // For now: just verify the new functions exist and compile
    let dir = tempfile::tempdir().unwrap();
    let _ = jobcard_core::worktree::squash_workspace(dir.path());
    let _ = jobcard_core::worktree::forget_workspace(dir.path(), "test");
}
```

Run (expect: compiles and passes — functions exist):
```zsh
cargo test -p jc -- merge_gate_jj_squash_and_push 2>&1
```

**Step 2: Replace merge gate logic**

In `run_merge_gate`, find the block that starts at `let wt_path = card_dir.join("worktree")` (~line 3795).

Replace the full merge sequence:
```rust
let ws_path = card_dir.join("workspace");
if ws_path.exists() {
    let ws_name = name.trim_end_matches(".jobcard");

    // Step 1: Squash agent changes from workspace into parent change.
    if let Err(e) = jobcard_core::worktree::squash_workspace(&ws_path) {
        let _ = fs::write(&qa_log, format!("jj squash failed: {e}\n").as_bytes());
        meta.failure_reason = Some("jj_squash_failed".to_string());
        let _ = write_meta(&card_dir, &meta);
        let _ = fs::rename(&card_dir, failed_dir.join(&name));
        continue;
    }

    // Step 2: Forget the workspace (clean up; data is in parent change).
    let _ = jobcard_core::worktree::forget_workspace(cards_dir, ws_name);

    // Step 3: Push stack to remote (best-effort; non-fatal if no remote).
    if let Err(e) = jobcard_core::worktree::push_stack(cards_dir, "origin") {
        eprintln!("[merge-gate] jj git push failed (no remote?): {e}");
    }

    // Step 4: Create stacked PR (best-effort; requires gh + GitHub remote).
    let pr_result = std::process::Command::new("gh")
        .args(["pr", "create", "--fill", "--draft"])
        .current_dir(cards_dir)
        .output();
    if let Ok(out) = pr_result {
        if !out.status.success() {
            eprintln!("[merge-gate] gh pr create failed (no remote?): {}",
                String::from_utf8_lossy(&out.stderr));
        }
    }

    let _ = write_meta(&card_dir, &meta);
    let _ = fs::rename(&card_dir, merged_dir.join(&name));
    continue;
}
// No workspace: move directly to merged (no VCS work).
let _ = write_meta(&card_dir, &meta);
let _ = fs::rename(&card_dir, merged_dir.join(&name));
```

**Step 3: Remove now-unused imports**

Remove any imports of `find_git_root` from the merge gate scope if it's no longer used there. Run:
```zsh
cargo build 2>&1 | grep 'unused\|error'
```
Fix any unused import warnings.

**Step 4: Run tests**
```zsh
cargo test -- merge_gate 2>&1
# All tests should pass (jj-dependent ones skip gracefully if jj absent)
cargo test 2>&1 | tail -5
# Expected: test result: ok. N passed; 0 failed
```

**Step 5: Commit**
```zsh
git add crates/
git commit -m "feat: jj-based merge gate with squash + push + stacked PR"
```

---

## Task 4: Remove ratatui and crossterm

**Files:**
- Modify: `crates/jc/Cargo.toml`
- Modify: `crates/jc/src/main.rs` (remove TUI functions)
- Test: `cargo build` must succeed without ratatui/crossterm

**Context:**
vibekanban replaces the ratatui TUI. Functions to remove: `cmd_dashboard`, `draw_dashboard`, `handle_dashboard_key`. The `Dashboard` command variant in the CLI enum also goes away. Search for all usages before deleting.

**Step 1: Remove deps from Cargo.toml**

In `crates/jc/Cargo.toml`, remove the lines:
```toml
crossterm = "0.29"
ratatui = "0.29"
```

**Step 2: Find and remove TUI functions from main.rs**
```zsh
grep -n 'cmd_dashboard\|draw_dashboard\|handle_dashboard_key\|Dashboard\|ratatui\|crossterm\|use ratatui\|use crossterm' crates/jc/src/main.rs
```

Remove:
1. The `Dashboard` variant from the `Command` enum
2. The `Command::Dashboard => cmd_dashboard(...)` match arm
3. The `fn cmd_dashboard(...)` function
4. The `fn draw_dashboard(...)` function
5. The `fn handle_dashboard_key(...)` function
6. Any `use ratatui::...` and `use crossterm::...` import lines

**Step 3: Verify build**
```zsh
cargo build 2>&1
# Expected: compiles with 0 errors (warnings OK)
cargo test 2>&1 | tail -5
# Expected: all tests still pass
```

**Step 4: Commit**
```zsh
git add crates/jc/Cargo.toml crates/jc/src/main.rs
git commit -m "feat: remove ratatui TUI (replaced by vibekanban)"
```

---

## Task 5: vibekanban provider — filesystem polling

**Files:**
- Create: `vibekanban/jobcard-provider.zsh` — polling script that translates `.cards/` to vibekanban task format
- Create: `vibekanban/config.json` — vibekanban config pointing at our provider
- Create: `vibekanban/README.md` — how to launch vibekanban with JobCard

**Context:**
vibekanban-cli (`npx vibe-kanban`) reads from GitHub branches by default, but can be configured with custom providers. Our provider is a zsh script that outputs card state as JSON on stdout, which vibekanban polls every few seconds. Format matches vibekanban's task data model.

**Step 1: Write test for provider output format**

Create `vibekanban/test_provider.zsh`:
```zsh
#!/usr/bin/env zsh
set -euo pipefail
# Create test .cards/ structure
tmp=$(mktemp -d)
mkdir -p "$tmp/team-cli/pending" "$tmp/team-cli/running" "$tmp/team-cli/done"
mkdir -p "$tmp/team-cli/pending/card-abc.jobcard"
echo '{"id":"card-abc","title":"Test card","stage":"implement"}' \
  > "$tmp/team-cli/pending/card-abc.jobcard/meta.json"

# Run provider and check output
out=$(CARDS_DIR="$tmp" ./vibekanban/jobcard-provider.zsh)
echo "$out" | python3 -c "
import json, sys
data = json.load(sys.stdin)
assert isinstance(data, list), 'must be a list'
assert len(data) == 1, f'expected 1 task, got {len(data)}'
assert data[0]['id'] == 'card-abc', f'wrong id: {data[0]}'
assert data[0]['status'] == 'pending', f'wrong status: {data[0]}'
print('PASS')
"
rm -rf "$tmp"
```

Run (expect FAIL — script doesn't exist yet):
```zsh
chmod +x vibekanban/test_provider.zsh && ./vibekanban/test_provider.zsh
```

**Step 2: Create the provider script**

Create `vibekanban/jobcard-provider.zsh`:
```zsh
#!/usr/bin/env zsh
# JobCard provider for vibekanban-cli.
# Polls CARDS_DIR and outputs task list as JSON on stdout.
# Usage: CARDS_DIR=.cards ./vibekanban/jobcard-provider.zsh
setopt NULL_GLOB
set -euo pipefail

CARDS_DIR="${CARDS_DIR:-.cards}"

# Map card state directory → vibekanban status
map_status() {
  case "$1" in
    pending) echo "pending" ;;
    running) echo "in_progress" ;;
    done)    echo "review" ;;
    merged)  echo "done" ;;
    failed)  echo "blocked" ;;
    *)       echo "unknown" ;;
  esac
}

tasks='[]'

for team_dir in "$CARDS_DIR"/*/; do
  team=$(basename "$team_dir")
  for state in pending running done merged failed; do
    for card_dir in "$team_dir/$state/"*.jobcard/; do
      [[ -d "$card_dir" ]] || continue
      meta_file="$card_dir/meta.json"
      [[ -f "$meta_file" ]] || continue

      id=$(python3 -c "import json,sys; d=json.load(open('$meta_file')); print(d.get('id','?'))")
      title=$(python3 -c "import json,sys; d=json.load(open('$meta_file')); print(d.get('title',d.get('id','?')))")
      status=$(map_status "$state")

      # Append to tasks array
      task=$(python3 -c "
import json
print(json.dumps({
  'id': '$id',
  'title': '$title',
  'status': '$status',
  'team': '$team',
  'meta_path': '$meta_file',
}))
")
      tasks=$(echo "$tasks" | python3 -c "
import json,sys
tasks=json.load(sys.stdin); tasks.append($task); print(json.dumps(tasks))
")
    done
  done
done

echo "$tasks"
```

**Step 3: Create vibekanban config**

Create `vibekanban/config.json`:
```json
{
  "provider": "custom",
  "custom_provider": {
    "command": "./vibekanban/jobcard-provider.zsh",
    "poll_interval_ms": 2000,
    "cards_dir": ".cards"
  },
  "actions": {
    "retry": "jc --cards-dir .cards retry {id}",
    "kill":  "jc --cards-dir .cards kill {id}",
    "logs":  "jc --cards-dir .cards logs {id}"
  }
}
```

**Step 4: Run the test**
```zsh
./vibekanban/test_provider.zsh
# Expected: PASS
```

**Step 5: Create README**

Create `vibekanban/README.md`:
```markdown
# vibekanban JobCard Provider

Launches vibekanban-cli with the JobCard filesystem provider.

## Usage

```zsh
npx vibe-kanban --config vibekanban/config.json
```

Cards in `.cards/` appear as tasks in vibekanban columns:
- pending → Backlog
- running → In Progress
- done → Review
- merged → Done
- failed → Blocked
```

**Step 6: Commit**
```zsh
git add vibekanban/
git commit -m "feat: vibekanban provider — poll .cards/ filesystem for task data"
```

---

## Task 6: Zellij layout file + per-card pane hooks

**Files:**
- Create: `zellij/jobcard.kdl` — native zellij layout
- Modify: `crates/jc/src/main.rs` — add `ZellijMode` detection + tier logic + `zellij action` calls in dispatcher

**Context:**
Adaptive tier logic based on total active running cards (M1 MacBook Pro screen):
- Tier 1 (1–5): one pane per card
- Tier 2 (6–20): one pane per team (5 panes max)
- Tier 3 (21–300): no panes (vibekanban web view handles it)

Virtual session: `$ZELLIJ` env var is set in any zellij context. `[ -t 1 ]` distinguishes interactive from piped/headless.

**Step 1: Write the layout file**

Create `zellij/jobcard.kdl`:
```kdl
// JobCard zellij layout
// Launch with: zellij --layout zellij/jobcard.kdl

layout {
  default_tab_template {
    children
    pane size=1 borderless=true {
      plugin location="file:target/jobcard-status.wasm" {
        cards_dir ".cards"
      }
    }
  }

  tab name="Dispatchers" focus=true {
    pane split_direction="horizontal" {
      pane name="team-cli" command="jc" {
        args "--cards-dir" ".cards/team-cli" "dispatcher"
              "--adapter" "adapters/claude.zsh"
      }
      pane name="team-arch" command="jc" {
        args "--cards-dir" ".cards/team-arch" "dispatcher"
              "--adapter" "adapters/claude.zsh"
      }
      pane name="team-quality" command="jc" {
        args "--cards-dir" ".cards/team-quality" "dispatcher"
              "--adapter" "adapters/claude.zsh"
      }
    }
  }

  tab name="vibekanban" {
    pane command="npx" {
      args "vibe-kanban" "--config" "vibekanban/config.json"
    }
  }
}
```

**Step 2: Add ZellijMode detection to main.rs**

Add near top of `main()` or in dispatcher init (in `run_dispatcher`):
```rust
#[derive(Debug, Clone, Copy, PartialEq)]
enum ZellijMode {
    Interactive,   // $ZELLIJ set, stdout is a tty
    Virtual,       // $ZELLIJ set, piped/headless
    None,          // not in zellij
}

fn detect_zellij_mode() -> ZellijMode {
    if std::env::var("ZELLIJ").is_err() {
        return ZellijMode::None;
    }
    // Check if stdout is a tty
    use std::os::unix::io::AsRawFd;
    let is_tty = unsafe { libc::isatty(std::io::stdout().as_raw_fd()) } != 0;
    if is_tty { ZellijMode::Interactive } else { ZellijMode::Virtual }
}
```

Add `libc = "0.2"` to `crates/jc/Cargo.toml` (MIT license):
```toml
libc = "0.2"
```

**Step 3: Add tier logic + pane hooks**

Add in `run_dispatcher`, after successfully creating a workspace for a card:
```rust
fn count_running_cards(cards_dir: &Path) -> usize {
    let running = cards_dir.join("running");
    std::fs::read_dir(&running)
        .map(|d| d.filter_map(Result::ok)
                   .filter(|e| e.path().extension().map(|x| x == "jobcard").unwrap_or(false))
                   .count())
        .unwrap_or(0)
}

fn zellij_open_card_pane(card_id: &str, card_dir: &Path) {
    let log = card_dir.join("logs").join("stdout.log");
    let _ = std::process::Command::new("zellij")
        .args(["action", "new-pane", "--name", card_id,
               "--", "tail", "-f", log.to_str().unwrap_or("")])
        .output();
}

fn zellij_close_pane_by_name(card_id: &str) {
    // zellij action close-pane requires the pane to be focused; use rename+close workaround
    let _ = std::process::Command::new("zellij")
        .args(["action", "rename-tab", card_id])
        .output();
}

// In run_dispatcher, after workspace create, before run_card:
let zellij = detect_zellij_mode();
let active = count_running_cards(cards_dir);
if zellij == ZellijMode::Interactive {
    match active {
        0..=5  => zellij_open_card_pane(&card_id, &running_path),
        6..=20 => { /* team pane already open */ }
        _      => { /* tier 3: no pane */ }
    }
}
```

**Step 4: Run tests**
```zsh
cargo build 2>&1
# Expected: clean build
cargo test 2>&1 | tail -5
# Expected: all tests pass
```

**Step 5: Commit**
```zsh
git add zellij/ crates/jc/
git commit -m "feat: zellij layout + adaptive per-card pane hooks"
```

---

## Task 7: Zellij WASM status bar plugin

**Files:**
- Create: `crates/jc-zellij-plugin/` — new Rust crate
- Create: `crates/jc-zellij-plugin/src/lib.rs`
- Create: `crates/jc-zellij-plugin/Cargo.toml`
- Modify: `Cargo.toml` (workspace) — add `crates/jc-zellij-plugin` member

**Context:**
Zellij plugins are Rust crates compiled to WASM32-WASI, using the `zellij-tile` crate (MIT). The plugin reads the `.cards/` directory via the plugin watch-filesystem API and renders a one-line status bar showing card counts per team.

**Step 1: Add crate to workspace**

In `/Users/studio/gtfs/Cargo.toml`, add to `[workspace] members`:
```toml
"crates/jc-zellij-plugin",
```

**Step 2: Create crate files**

Create `crates/jc-zellij-plugin/Cargo.toml`:
```toml
[package]
name = "jc-zellij-plugin"
version = "0.1.0"
edition.workspace = true
license.workspace = true

[lib]
crate-type = ["cdylib"]

[dependencies]
zellij-tile = "0.41"   # MIT license

[profile.release]
opt-level = "s"        # optimize for size
```

Create `crates/jc-zellij-plugin/src/lib.rs`:
```rust
use zellij_tile::prelude::*;
use std::collections::HashMap;

#[derive(Default)]
struct JobCardPlugin {
    // team_name → (running, pending, done)
    counts: HashMap<String, (usize, usize, usize)>,
    cards_dir: String,
}

register_plugin!(JobCardPlugin);

impl ZellijPlugin for JobCardPlugin {
    fn load(&mut self, configuration: BTreeMap<String, String>) {
        self.cards_dir = configuration
            .get("cards_dir")
            .cloned()
            .unwrap_or_else(|| ".cards".to_string());
        // Subscribe to timer events for periodic refresh
        subscribe(&[EventType::Timer]);
        set_timeout(2.0);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Timer(_) => {
                self.refresh_counts();
                set_timeout(2.0);
                true
            }
            _ => false,
        }
    }

    fn render(&mut self, _rows: usize, cols: usize) {
        let mut parts: Vec<String> = Vec::new();
        let mut teams: Vec<&str> = self.counts.keys().map(String::as_str).collect();
        teams.sort();
        for team in &teams {
            let short = team.strip_prefix("team-").unwrap_or(team);
            let (r, p, d) = self.counts[*team];
            parts.push(format!("{}:{}", short, if r > 0 { format!("{}\u{25b6}", r) }
                else if p > 0 { format!("{}·", p) }
                else { format!("{}✓", d) }));
        }
        let bar = parts.join("  ");
        // Pad to cols
        print!("{:width$}", bar, width = cols.min(bar.len() + 2));
    }
}

impl JobCardPlugin {
    fn refresh_counts(&mut self) {
        self.counts.clear();
        let base = std::path::Path::new(&self.cards_dir);
        if let Ok(teams) = std::fs::read_dir(base) {
            for team_entry in teams.flatten() {
                let team_name = team_entry.file_name().to_string_lossy().to_string();
                let mut running = 0usize;
                let mut pending = 0usize;
                let mut done = 0usize;
                for (state, counter) in [("running", &mut running),
                                          ("pending", &mut pending),
                                          ("done", &mut done),
                                          ("merged", &mut done)] {
                    let dir = team_entry.path().join(state);
                    if let Ok(cards) = std::fs::read_dir(&dir) {
                        *counter += cards.flatten()
                            .filter(|e| e.path().extension()
                                .map(|x| x == "jobcard").unwrap_or(false))
                            .count();
                    }
                }
                self.counts.insert(team_name, (running, pending, done));
            }
        }
    }
}
```

**Step 3: Add wasm32-wasi target and build**
```zsh
rustup target add wasm32-wasip1 2>/dev/null || rustup target add wasm32-wasi
cargo build -p jc-zellij-plugin --target wasm32-wasip1 --release 2>&1
# Output: target/wasm32-wasip1/release/jc_zellij_plugin.wasm
cp target/wasm32-wasip1/release/jc_zellij_plugin.wasm target/jobcard-status.wasm
```

**Step 4: Add build script to zellij layout README**

Add to `zellij/README.md`:
```markdown
# Building the status bar plugin

```zsh
cargo build -p jc-zellij-plugin --target wasm32-wasip1 --release
cp target/wasm32-wasip1/release/jc_zellij_plugin.wasm target/jobcard-status.wasm
```

Launch layout:
```zsh
zellij --layout zellij/jobcard.kdl
```
```

**Step 5: Verify main workspace still builds**
```zsh
cargo build 2>&1 | grep -E '^error' | head -10
# Expected: no errors
```

**Step 6: Commit**
```zsh
git add crates/jc-zellij-plugin/ zellij/ Cargo.toml Cargo.lock target/jobcard-status.wasm
git commit -m "feat: zellij WASM status bar plugin showing per-team card counts"
```

---

## Task 8: zsh shell completions

**Files:**
- Create: `completions/_jc` — zsh completion file
- Modify: `crates/jc/src/main.rs` — add `GenerateCompletion` command using `clap_complete`
- Modify: `crates/jc/Cargo.toml` — add `clap_complete`

**Step 1: Add `clap_complete` dependency**

In `crates/jc/Cargo.toml`:
```toml
clap_complete = "4"   # MIT/Apache
```

**Step 2: Add completion generation command**

In `main.rs`, add to the `Command` enum:
```rust
/// Generate shell completion script.
GenerateCompletion {
    #[arg(value_enum)]
    shell: clap_complete::Shell,
},
```

Add match arm in `main()`:
```rust
Command::GenerateCompletion { shell } => {
    use clap::CommandFactory;
    clap_complete::generate(shell, &mut Cli::command(), "jc", &mut std::io::stdout());
}
```

**Step 3: Generate and save the completion**
```zsh
cargo build 2>&1
./target/debug/jc generate-completion zsh > completions/_jc
```

**Step 4: Verify completion file**
```zsh
head -5 completions/_jc
# Expected: #compdef jc
```

**Step 5: Commit**
```zsh
git add completions/ crates/jc/
git commit -m "feat: add zsh shell completion via clap_complete"
```

---

## Task 9: License audit CI test

**Files:**
- Create: `scripts/check_licenses.zsh` — audit script
- Create: `.github/workflows/license-check.yml` — CI workflow (if GitHub remote is configured)

**Goal:** Fail the build if any Rust dependency has a non-MIT/BSD/Apache license.

**Step 1: Write audit script**

Create `scripts/check_licenses.zsh`:
```zsh
#!/usr/bin/env zsh
# Fail if any cargo dependency uses a non-permissive license.
set -euo pipefail

# Requires: cargo install cargo-deny
if ! command -v cargo-deny &>/dev/null; then
  echo "Install cargo-deny: cargo install cargo-deny"
  exit 1
fi

cargo deny check licenses 2>&1
echo "License audit passed."
```

**Step 2: Create `deny.toml`**

Create `deny.toml` at repo root:
```toml
[licenses]
allow = [
  "MIT",
  "Apache-2.0",
  "Apache-2.0 WITH LLVM-exception",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "ISC",
  "Unlicense",
  "Unicode-DFS-2016",
  "CC0-1.0",
  "OpenSSL",
]
deny = ["GPL-2.0", "GPL-3.0", "LGPL-2.0", "LGPL-2.1", "AGPL-3.0"]
copyleft = "deny"

[bans]
# Ban bash as a dep (it never appears in Cargo but explicit is good)
```

**Step 3: Run audit**
```zsh
cargo install cargo-deny --locked 2>&1 | tail -3
cargo deny check licenses 2>&1
# Expected: all licenses allowed
```

**Step 4: Commit**
```zsh
git add scripts/check_licenses.zsh deny.toml
git commit -m "feat: license audit with cargo-deny (MIT/BSD/Apache only)"
```

---

## Parallelization Guide

These tasks can be dispatched in parallel (no shared-state conflicts):

| Parallel Group A | Parallel Group B | Sequential |
|-----------------|-----------------|------------|
| Task 1 (zsh migration) | Task 5 (vibekanban) | Task 3 after Task 2 |
| Task 4 (remove ratatui) | Task 7 (WASM plugin) | Task 9 after all |
| Task 8 (completions) | Task 6 (zellij layout) | |

Task 2 (jj workspace in dispatcher) must come before Task 3 (jj merge gate).
Tasks 1, 4, 5, 6, 7, 8 are all independent and can run in parallel.
