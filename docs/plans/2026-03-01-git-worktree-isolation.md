# Git Worktree Isolation + Merge Workflow Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Give each dispatched jobcard its own git worktree so agents work in isolation, then merge via the merge gate rather than writing directly to the main working tree.

**Architecture:** Initialize a git repo in the project root. Before the dispatcher hands a card to an adapter, it creates `git worktree add <card>/worktree jobs/<card-id>`. The adapter runs `cd <worktree>` and does its work there. The merge gate iterates `done/` cards, commits any changes in the worktree, merges the branch into `main`, and moves the card to `merged/`. Crash recovery: if a `running/` card's PID is dead and its worktree branch exists, the reaper just restores it to `pending/` — the branch survives.

**Tech Stack:** Rust (existing dispatcher/merge-gate in `crates/jc/src/main.rs`), `git` CLI (via `std::process::Command`), shell adapters in `adapters/`.

**Research basis:**
- [Gas Town](https://github.com/steveyegge/gastown): git-backed issue state, crash recovery via git persistence
- [auto-claude](https://github.com/AndyMik90/Auto-Claude/blob/develop/CLAUDE.md): worktrees at `.auto-claude/worktrees/tasks/{spec-name}/`, agent runs inside worktree dir
- [ccswarm](https://github.com/nwiizo/ccswarm): git worktree per agent with specialized roles
- [TaskSmith](https://tasksmith.dev/): each task → own worktree → auto-PR on success → discard on failure
- Claude Code built-in: `claude --worktree` flag for isolated runs

---

## Current State Audit

| Item | Status |
|------|--------|
| `git init` | ✗ not a git repo |
| `main.rs` | 2273 lines, builds OK (1 warning) |
| team-cli cards | ✓ 5/5 done — retry/kill/logs/inspect implemented (3 agents overlapped on same cmds) |
| team-arch cards | ✓ 5/5 "done" — but all wrote **plans only** (`docs/plans/`), no code |
| team-quality | 1 pending, 1 running, 3 done |
| team-intelligence | 4 pending, 1 running |
| team-platform | 4 pending, 1 running |
| Worktree isolation | ✗ agents wrote directly to shared `main.rs` |
| Merge gate | ✗ broken (no git, no isolation) |

---

## Phase 1: Initialize Git + Stabilize

### Task 1: Initialize git repo and commit current state

**Files:**
- Create: `.gitignore`
- Run: `git init && git add . && git commit`

**Step 1: Create .gitignore**

```
# Rust build artifacts
/target/

# Card working state (logs, worktrees, etc.)
.cards/*/running/
.cards/*/done/
.cards/*/failed/
.cards/*/merged/

# Auto-generated
*.log
.DS_Store
```

**Step 2: Init and commit**

```bash
cd /Users/studio/gtfs
git init
git add .gitignore
git add Cargo.toml Cargo.lock
git add crates/
git add adapters/
git add scripts/
git add docs/
git add README.md CLAUDE.md
git add .auto-claude/roadmap/
git commit -m "feat: initial commit — jobcard orchestrator with 10 implemented features"
```

Expected: `main` branch with initial commit.

**Step 3: Verify build still passes**

```bash
cargo build 2>&1 | tail -5
```

Expected: `Finished dev profile`.

---

### Task 2: Audit + deduplicate main.rs (team-cli overlap)

Three agents (job-control-retry, job-control-kill, job-control-logs) all implemented the same four commands. The file is 2273 lines. Consolidate to one clean implementation.

**Files:**
- Modify: `crates/jc/src/main.rs`

**Step 1: Identify duplicates**

```bash
grep -n "fn cmd_retry\|fn cmd_kill\|fn cmd_logs\|fn cmd_inspect" crates/jc/src/main.rs
```

Expected: Each function appears exactly once (if agents were polite) or multiple times (collision).

**Step 2: If duplicates exist — keep the last (most refined) version, delete earlier ones**

Keep `cmd_retry` at its last occurrence, delete earlier occurrences. Same for `kill`, `logs`, `inspect`.

**Step 3: Verify tests still pass**

```bash
cargo test 2>&1 | tail -20
```

Expected: All tests pass.

**Step 4: Commit**

```bash
git add crates/jc/src/main.rs
git commit -m "refactor: deduplicate retry/kill/logs/inspect — consolidate 3-agent overlap"
```

---

## Phase 2: Worktree Helpers in jobcard-core

### Task 3: Add `worktree` module to `jobcard-core`

The dispatcher and merge gate both need to create/remove git worktrees. Put the helpers where both can use them.

**Files:**
- Create: `crates/jobcard-core/src/worktree.rs`
- Modify: `crates/jobcard-core/src/lib.rs` (add `pub mod worktree;`)

**Step 1: Write failing test**

In `crates/jobcard-core/src/worktree.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_git_repo() -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(dir.path())
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@test.com")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@test.com")
            .output()
            .unwrap();
        dir
    }

    #[test]
    fn test_create_and_remove_worktree() {
        let repo = make_git_repo();
        let wt_path = repo.path().join("wt-test");
        create_worktree(repo.path(), &wt_path, "jobs/test-card").unwrap();
        assert!(wt_path.exists());
        remove_worktree(repo.path(), &wt_path).unwrap();
        assert!(!wt_path.exists());
    }

    #[test]
    fn test_commit_worktree_changes() {
        let repo = make_git_repo();
        let wt_path = repo.path().join("wt-commit");
        create_worktree(repo.path(), &wt_path, "jobs/commit-card").unwrap();
        std::fs::write(wt_path.join("output.txt"), "agent work").unwrap();
        commit_worktree(&wt_path, "commit-card").unwrap();
        // Branch should exist
        let out = std::process::Command::new("git")
            .args(["branch", "--list", "jobs/commit-card"])
            .current_dir(repo.path())
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).contains("jobs/commit-card"));
    }
}
```

**Step 2: Run to verify fails**

```bash
cargo test -p jobcard-core worktree 2>&1 | tail -10
```

Expected: compile error — module not found.

**Step 3: Implement**

```rust
// crates/jobcard-core/src/worktree.rs
use std::path::Path;
use anyhow::{Context, Result};

/// Create a git worktree at `wt_path` on branch `branch_name`.
/// Branch is created if it doesn't exist.
pub fn create_worktree(git_root: &Path, wt_path: &Path, branch_name: &str) -> Result<()> {
    // Try "add -b <branch>" first; if branch exists, use "add" without -b
    let status = std::process::Command::new("git")
        .args(["worktree", "add", "-b", branch_name,
               &wt_path.to_string_lossy()])
        .current_dir(git_root)
        .status()
        .context("git worktree add")?;

    if !status.success() {
        // Branch may already exist — use existing branch
        std::process::Command::new("git")
            .args(["worktree", "add",
                   &wt_path.to_string_lossy(), branch_name])
            .current_dir(git_root)
            .status()
            .context("git worktree add (existing branch)")?;
    }
    Ok(())
}

/// Stage all changes in the worktree and commit with a standard message.
pub fn commit_worktree(wt_path: &Path, card_id: &str) -> Result<()> {
    let run = |args: &[&str]| -> Result<()> {
        std::process::Command::new("git")
            .args(args)
            .current_dir(wt_path)
            .env("GIT_AUTHOR_NAME", "jobcard-agent")
            .env("GIT_AUTHOR_EMAIL", "agent@jobcard.local")
            .env("GIT_COMMITTER_NAME", "jobcard-agent")
            .env("GIT_COMMITTER_EMAIL", "agent@jobcard.local")
            .status()
            .with_context(|| format!("git {:?}", args))?;
        Ok(())
    };
    run(&["add", "-A"])?;
    // Allow empty commit in case agent produced no file changes (e.g. only logs)
    run(&["commit", "--allow-empty", "-m",
          &format!("feat(jobcard): complete {card_id}")])?;
    Ok(())
}

/// Merge a card's branch into the current HEAD of git_root, then remove the worktree.
/// Returns true if merge succeeded, false on conflict.
pub fn merge_card_branch(git_root: &Path, branch_name: &str) -> Result<bool> {
    let out = std::process::Command::new("git")
        .args(["merge", "--no-ff", branch_name,
               "-m", &format!("Merge {branch_name} via merge-gate")])
        .current_dir(git_root)
        .output()
        .context("git merge")?;
    Ok(out.status.success())
}

/// Prune and remove a worktree directory.
pub fn remove_worktree(git_root: &Path, wt_path: &Path) -> Result<()> {
    std::process::Command::new("git")
        .args(["worktree", "remove", "--force",
               &wt_path.to_string_lossy()])
        .current_dir(git_root)
        .status()
        .context("git worktree remove")?;
    Ok(())
}
```

**Step 4: Add `tempfile` dev-dependency** (if not already in Cargo.toml)

```toml
# crates/jobcard-core/Cargo.toml
[dev-dependencies]
tempfile = "3"
```

**Step 5: Run tests**

```bash
cargo test -p jobcard-core worktree 2>&1 | tail -15
```

Expected: `test result: ok. 2 passed`.

**Step 6: Commit**

```bash
git add crates/jobcard-core/src/worktree.rs crates/jobcard-core/src/lib.rs crates/jobcard-core/Cargo.toml
git commit -m "feat(core): add worktree helpers — create/commit/merge/remove"
```

---

## Phase 3: Dispatcher — Create Worktree Before Dispatch

### Task 4: Create git worktree in dispatcher's worker loop

The dispatcher calls an adapter like:
```
adapter.sh <workdir> <prompt_file> <stdout_log> <stderr_log>
```
Currently `<workdir>` is the project root. Change it to the card's worktree.

**Files:**
- Modify: `crates/jc/src/main.rs` — `run_worker()` / `dispatch_card()` function (currently around line 1400–1600, search for `fn run_worker` or `spawn_worker`)

**Step 1: Locate the dispatch function**

```bash
grep -n "fn.*worker\|adapter.*sh\|spawn\|Command::new" crates/jc/src/main.rs | head -20
```

**Step 2: Add worktree creation before adapter spawn**

In the worker function, after the card is moved to `running/`, add:

```rust
use jobcard_core::worktree::{create_worktree, remove_worktree};

// Determine git root (walk up from cards_dir)
let git_root = find_git_root(&card_path)
    .unwrap_or_else(|| card_path.clone());

let wt_path = card_path.join("worktree");
let branch = format!("jobs/{}", card_id);

if git_root != card_path {
    // We're in a real git repo — create isolated worktree
    create_worktree(&git_root, &wt_path, &branch)
        .context("creating worktree for card")?;
}
```

Add a helper:

```rust
fn find_git_root(start: &Path) -> Option<PathBuf> {
    let out = std::process::Command::new("git")
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
```

**Step 3: Pass worktree path as workdir to adapter**

Where the adapter is currently invoked with the project root, change to:

```rust
let workdir = if wt_path.exists() { &wt_path } else { &git_root };
// Pass workdir as first argument to adapter
```

**Step 4: Test dispatch creates a worktree**

```bash
# Create a test card manually and run dispatcher --once
./target/debug/jc new implement test-wt-card
./target/debug/jc dispatcher --once --max-workers 1
ls .cards/running/test-wt-card.jobcard/worktree/ 2>/dev/null && echo "worktree created"
```

**Step 5: Commit**

```bash
git add crates/jc/src/main.rs
git commit -m "feat(dispatcher): create git worktree per card before adapter dispatch"
```

---

## Phase 4: Merge Gate — Commit + Merge Each Done Card

### Task 5: Implement `merge-gate` to commit + merge card worktrees

The existing `merge-gate` subcommand is a stub. Implement it to:
1. Iterate `done/` cards
2. Commit any uncommitted changes in their worktree
3. `git merge` the card's branch into main
4. On success: move card to `merged/`
5. On conflict: move card to `failed/`, record reason in meta.json

**Files:**
- Modify: `crates/jc/src/main.rs` — find `merge_gate` or `MergeGate` handler (~line 600+)

**Step 1: Locate merge gate**

```bash
grep -n "merge.gate\|merge_gate\|MergeGate" crates/jc/src/main.rs | head -10
```

**Step 2: Write failing integration test**

In `crates/jc/tests/merge_gate_harness.rs` (create new file):

```rust
use std::path::PathBuf;
use tempfile::TempDir;

fn setup_git_repo_with_done_card() -> (TempDir, PathBuf) {
    // init repo, create a done card with a worktree branch, return (tmpdir, cards_dir)
    todo!()
}

#[test]
fn merge_gate_merges_done_card_to_merged() {
    // Run merge gate, verify card moves from done/ to merged/
    todo!()
}

#[test]
fn merge_gate_moves_conflict_card_to_failed() {
    // Set up conflicting changes on main and card branch
    // Run merge gate, verify card moves to failed/
    todo!()
}
```

**Step 3: Implement merge gate logic**

Replace the stub with:

```rust
async fn cmd_merge_gate(root: &Path, poll_ms: u64) -> anyhow::Result<()> {
    use jobcard_core::worktree::{commit_worktree, merge_card_branch, remove_worktree};

    let git_root = find_git_root(root).context("Not in a git repo — merge gate requires git")?;

    loop {
        // Find all team done/ dirs
        for team_dir in root.join(".cards").read_dir()? {
            let team_dir = team_dir?.path();
            let done_dir = team_dir.join("done");
            if !done_dir.exists() { continue; }

            for entry in done_dir.read_dir()? {
                let card_path = entry?.path();
                if !card_path.extension().map_or(false, |e| e == "jobcard") { continue; }

                let card_id = card_path.file_stem().unwrap().to_string_lossy().to_string();
                let wt_path = card_path.join("worktree");
                let branch = format!("jobs/{card_id}");

                // Commit any agent output in the worktree
                if wt_path.exists() {
                    if let Err(e) = commit_worktree(&wt_path, &card_id) {
                        eprintln!("[merge-gate] commit failed for {card_id}: {e}");
                    }

                    // Attempt merge into main
                    let merged = merge_card_branch(&git_root, &branch)?;

                    if merged {
                        // Move card to merged/
                        let merged_dir = team_dir.join("merged");
                        std::fs::create_dir_all(&merged_dir)?;
                        std::fs::rename(&card_path, merged_dir.join(card_path.file_name().unwrap()))?;
                        remove_worktree(&git_root, &wt_path).ok();
                        println!("[merge-gate] ✓ merged {card_id}");
                    } else {
                        // Record conflict, move to failed
                        let failed_dir = team_dir.join("failed");
                        std::fs::create_dir_all(&failed_dir)?;
                        // Update meta.json failure_reason
                        let meta_path = card_path.join("meta.json");
                        if let Ok(mut meta) = read_meta(&meta_path) {
                            meta.failure_reason = Some("merge conflict".to_string());
                            write_meta(&meta_path, &meta)?;
                        }
                        std::fs::rename(&card_path, failed_dir.join(card_path.file_name().unwrap()))?;
                        eprintln!("[merge-gate] ✗ conflict on {card_id} → failed/");
                    }
                }
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(poll_ms)).await;
    }
}
```

**Step 4: Run tests**

```bash
cargo test 2>&1 | tail -20
```

**Step 5: Commit**

```bash
git add crates/jc/src/main.rs crates/jc/tests/merge_gate_harness.rs
git commit -m "feat(merge-gate): commit worktree + git merge done cards, move to merged/ or failed/"
```

---

## Phase 5: Adapter Updates — Run Inside Worktree

### Task 6: Update adapters to `cd` into the worktree

Adapters currently `cd "$workdir"` which is now the worktree path. Verify each adapter correctly handles this.

**Files:**
- Modify: `adapters/claude.sh`, `adapters/codex.sh`, `adapters/opencode.sh`

**Step 1: Verify adapter signature**

All adapters accept: `adapter.sh <workdir> <prompt_file> <stdout_log> <stderr_log>`

Current `claude.sh`:
```bash
workdir="$1"; prompt_file="$2"; stdout_log="$3"; stderr_log="$4"
cd "$workdir" || exit 1
```

This already uses `$workdir`. Since we're now passing the worktree path, no changes needed — **verify only**.

**Step 2: Check `--dangerously-skip-permissions` still works in a worktree**

```bash
# Quick smoke test: create worktree of current repo and run adapter in it
git worktree add /tmp/test-wt-smoke jobs/test-smoke 2>/dev/null || true
bash adapters/mock.sh /tmp/test-wt-smoke /dev/null /tmp/smoke-out /tmp/smoke-err
cat /tmp/smoke-out
git worktree remove --force /tmp/test-wt-smoke
```

Expected: mock adapter runs successfully in the worktree.

**Step 3: Commit (no-op if no changes needed)**

```bash
git commit --allow-empty -m "chore(adapters): verify adapter workdir compatibility with git worktrees"
```

---

## Phase 6: Execute the team-arch Plans

The five team-arch cards produced plans only — they need implementation runs.

### Task 7: Execute plan — `config-file`

**File:** `docs/plans/2026-03-01-global-config-file.md`

**Step 1:** Open a new session in the project root
**Step 2:** Load the plan: `cat docs/plans/2026-03-01-global-config-file.md`
**Step 3:** Use `superpowers:executing-plans` skill to implement task-by-task

### Task 8: Execute plan — `cli-refactoring`

**File:** `docs/plans/2026-03-01-cli-module-refactoring.md`

Split main.rs into modules. Dependent on config-file being done first (config types used by CLI).

### Task 9: Execute plan — `providers-cli`

**File:** `docs/plans/2026-03-01-providers-cli.md`

### Task 10: Execute plan — `worktree-cli`

**File:** `docs/plans/2026-03-01-worktree-cli.md`

This plan implements `jc worktree list/create/clean` — directly related to Phase 2-4 above. **Do this after Task 4 is done.**

---

## Phase 7: Re-run Remaining Cards With Isolation

### Task 11: Reset and re-dispatch remaining cards with worktree isolation

After git init and dispatcher changes, re-run the remaining cards (team-quality, team-intelligence, team-platform) so they work in isolated branches.

```bash
# Kill any still-running dispatchers
pkill -f "jc.*dispatcher" 2>/dev/null || true

# Relaunch dispatchers (now with worktree support)
zsh scripts/launch_teams.sh
```

---

## Verification Checklist

After completing all phases:

- [ ] `git log --oneline | head -20` — shows multiple commits
- [ ] `git branch -a` — shows `jobs/<card-id>` branches for all dispatched cards
- [ ] `cargo build` — passes cleanly
- [ ] `cargo test` — all tests pass
- [ ] `./target/debug/jc dispatcher --once` — creates worktree in `running/<card>/worktree/`
- [ ] `./target/debug/jc merge-gate` — merges `done/` cards, populates `merged/`
- [ ] `./target/debug/jc status` — shows accurate state across all teams

---

## Key Architecture Decisions

| Decision | Rationale |
|----------|-----------|
| Worktree inside card bundle (`<card>/worktree/`) | Crash-recoverable: bundle = all state for a job |
| Branch name = `jobs/<card-id>` | Namespaced, sortable, auto-pruned after merge |
| Commit before merge (allow-empty) | Handles agents that only produce logs, no file changes |
| Merge conflict → `failed/` | Consistent with existing failure handling; human reviews |
| Git root auto-discovered via `git rev-parse` | Works without hardcoded paths; portable |
| `.gitignore` excludes `running/`, `done/`, `failed/` | Card bundle contents (logs, worktrees) are ephemeral |

Sources consulted:
- [Gas Town](https://github.com/steveyegge/gastown)
- [ccswarm](https://github.com/nwiizo/ccswarm)
- [TaskSmith](https://tasksmith.dev/)
- [Claude Code Worktrees](https://claudefa.st/blog/guide/development/worktree-guide)
- [auto-claude CLAUDE.md](https://github.com/AndyMik90/Auto-Claude/blob/develop/CLAUDE.md)
