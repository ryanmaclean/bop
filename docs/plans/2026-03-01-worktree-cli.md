# Worktree CLI Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add `jc worktree list|create|clean` subcommands to manage the git worktrees associated with job cards.

**Architecture:** Add a `Worktree` variant to the existing `Command` enum in `crates/jc/src/main.rs`, with a nested `WorktreeAction` sub-enum. All three subcommands operate on the filesystem (scanning `.cards/*/NNNN.jobcard/worktree/` directories) and, where git is available, cross-reference with `git worktree list --porcelain` to surface orphaned linked worktrees.

**Tech Stack:** Rust, Clap 4 (derive), `std::process::Command` for git calls, integration tests using `tempfile` crate.

---

## Context (read before touching code)

- All CLI logic lives in one file: `crates/jc/src/main.rs`
- Job cards live at `.cards/<state>/<id>.jobcard/` (states: `pending`, `running`, `done`, `merged`, `failed`)
- A card's working directory is `<card_dir>/worktree/` if that path exists; the field `worktree_branch` in `meta.json` names the associated git branch (e.g. `job/foo-id`)
- Integration tests live in `crates/jc/tests/` and follow the pattern: build binary via `cargo build`, create `tempfile::tempdir()`, drive the binary with `std::process::Command`
- The workspace `Cargo.toml` defines shared deps; `crates/jc/Cargo.toml` already has `tempfile = "3"` in `[dev-dependencies]`

---

## Task 1: CLI skeleton — add the `Worktree` command

**Files:**
- Modify: `crates/jc/src/main.rs`

No test needed — verify with `cargo build`.

**Step 1: Add `WorktreeAction` enum and `Worktree` variant**

In `main.rs`, add after the existing `enum Command { ... }` definition:

```rust
#[derive(Subcommand, Debug)]
enum WorktreeAction {
    /// List all job card worktrees and flag orphans.
    List,
    /// Create a git worktree for a pending or running job card.
    Create { id: String },
    /// Remove worktrees for done/merged cards or orphaned git worktrees.
    Clean {
        #[arg(long)]
        dry_run: bool,
    },
}
```

Add this variant to `enum Command`:

```rust
    /// Manage git worktrees associated with job cards.
    Worktree {
        #[command(subcommand)]
        action: WorktreeAction,
    },
```

**Step 2: Add stub match arm in `main`**

Inside the `match cli.cmd { ... }` block, add:

```rust
        Command::Worktree { action } => match action {
            WorktreeAction::List => cmd_worktree_list(&root),
            WorktreeAction::Create { id } => cmd_worktree_create(&root, &id),
            WorktreeAction::Clean { dry_run } => cmd_worktree_clean(&root, dry_run),
        },
```

**Step 3: Add three stub functions** (anywhere after the existing helpers):

```rust
fn cmd_worktree_list(_root: &Path) -> anyhow::Result<()> {
    todo!()
}

fn cmd_worktree_create(_root: &Path, _id: &str) -> anyhow::Result<()> {
    todo!()
}

fn cmd_worktree_clean(_root: &Path, _dry_run: bool) -> anyhow::Result<()> {
    todo!()
}
```

**Step 4: Verify the build compiles**

```bash
cargo build -p jc
```
Expected: compiled successfully (todo! panics are fine at compile time).

**Step 5: Commit**

```bash
git add crates/jc/src/main.rs
git commit -m "feat(worktree): add CLI skeleton for worktree subcommands"
```

---

## Task 2: `worktree list` — scan cards and detect orphans

**Files:**
- Create: `crates/jc/tests/worktree_harness.rs`
- Modify: `crates/jc/src/main.rs`

### Step 1: Write the failing test

Create `crates/jc/tests/worktree_harness.rs` with:

```rust
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .parent().unwrap()
        .to_path_buf()
}

fn build_jc() {
    let status = Command::new("cargo")
        .arg("build")
        .current_dir(repo_root())
        .status()
        .expect("cargo build failed");
    assert!(status.success());
}

fn jc_bin() -> PathBuf {
    repo_root().join("target").join("debug").join("jc")
}

/// Write a minimal meta.json into a card directory.
fn write_meta(card_dir: &Path, id: &str, branch: &str) {
    let meta = format!(
        r#"{{"id":"{id}","created":"2026-03-01T00:00:00Z","stage":"implement","provider_chain":[],"stages":{{}},"acceptance_criteria":[],"worktree_branch":"{branch}"}}"#,
        id = id, branch = branch
    );
    fs::write(card_dir.join("meta.json"), meta).unwrap();
}

#[test]
fn worktree_list_shows_cards_with_worktrees() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    // jc init
    let s = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status().unwrap();
    assert!(s.success());

    // Card in running/ with a worktree/ dir
    let running_card = cards.join("running").join("run-job.jobcard");
    fs::create_dir_all(running_card.join("worktree")).unwrap();
    write_meta(&running_card, "run-job", "job/run-job");

    // Card in done/ with a worktree/ dir
    let done_card = cards.join("done").join("done-job.jobcard");
    fs::create_dir_all(done_card.join("worktree")).unwrap();
    write_meta(&done_card, "done-job", "job/done-job");

    // Card in pending/ WITHOUT a worktree/ dir (should not appear)
    let pending_card = cards.join("pending").join("no-wt.jobcard");
    fs::create_dir_all(&pending_card).unwrap();
    write_meta(&pending_card, "no-wt", "job/no-wt");

    let out = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "worktree", "list"])
        .output().unwrap();
    assert!(out.status.success(), "jc worktree list failed: {}", String::from_utf8_lossy(&out.stderr));

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("run-job"),  "should show run-job:\n{}", stdout);
    assert!(stdout.contains("done-job"), "should show done-job:\n{}", stdout);
    assert!(!stdout.contains("no-wt"),   "should NOT show no-wt (no worktree/):\n{}", stdout);
    assert!(stdout.contains("job/run-job"),  "should show branch:\n{}", stdout);
    assert!(stdout.contains("running"),      "should show state:\n{}", stdout);
    assert!(stdout.contains("done"),         "should show done state:\n{}", stdout);
}

#[test]
fn worktree_list_flags_orphaned_git_worktrees() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    // Init the temp dir as a git repo (needed for git worktree add)
    let s = Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(td.path())
        .status().unwrap();
    assert!(s.success());
    // Initial commit required for git worktree add
    fs::write(td.path().join("README"), "init").unwrap();
    Command::new("git").args(["add", "."]).current_dir(td.path()).status().unwrap();
    Command::new("git")
        .args(["-c", "user.email=a@b", "-c", "user.name=a", "commit", "-m", "init"])
        .current_dir(td.path()).status().unwrap();

    let s = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status().unwrap();
    assert!(s.success());

    // Create an orphaned git worktree: a branch with NO corresponding card
    let orphan_path = td.path().join("orphan-wt");
    Command::new("git")
        .args(["worktree", "add", "-b", "job/orphan", orphan_path.to_str().unwrap()])
        .current_dir(td.path())
        .status().unwrap();

    let out = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "worktree", "list"])
        .output().unwrap();
    assert!(out.status.success());

    let stdout = String::from_utf8_lossy(&out.stdout);
    // The orphan-wt worktree must appear and be flagged
    assert!(
        stdout.contains("orphan") || stdout.contains(orphan_path.to_str().unwrap()),
        "orphaned worktree should appear in list:\n{}", stdout
    );
    assert!(stdout.contains("orphaned"), "orphaned worktrees should be flagged:\n{}", stdout);
}
```

### Step 2: Run the test to verify it fails

```bash
cargo test -p jc --test worktree_harness worktree_list_shows_cards_with_worktrees -- --nocapture
```
Expected: FAIL with "not yet implemented" (todo! panic) or compilation error.

### Step 3: Implement `cmd_worktree_list`

Add two helpers and replace the stub in `main.rs`:

```rust
/// Returns (path, branch) for every worktree known to git (excluding the main worktree).
/// Returns empty vec if git is unavailable or not in a git repo.
fn git_worktree_paths(from_dir: &Path) -> Vec<(PathBuf, String)> {
    let Ok(out) = StdCommand::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(from_dir)
        .output()
    else {
        return vec![];
    };
    if !out.status.success() {
        return vec![];
    }

    let text = String::from_utf8_lossy(&out.stdout);
    let mut result = Vec::new();
    let mut current_path: Option<PathBuf> = None;
    let mut current_branch = String::new();
    let mut is_main = false;

    for line in text.lines() {
        if line.starts_with("worktree ") {
            // flush previous
            if let Some(p) = current_path.take() {
                if !is_main {
                    result.push((p, current_branch.clone()));
                }
            }
            current_path = Some(PathBuf::from(line.trim_start_matches("worktree ")));
            current_branch = String::new();
            is_main = false;
        } else if line.starts_with("branch ") {
            current_branch = line.trim_start_matches("branch refs/heads/").to_string();
        } else if line == "HEAD (detached)" || line.contains("bare") {
            // mark but don't skip the path
        } else if line.is_empty() {
            // separator between worktree blocks — flush
            if let Some(p) = current_path.take() {
                if !is_main {
                    result.push((p, current_branch.clone()));
                }
            }
            is_main = false;
        }
    }
    // flush last block
    if let Some(p) = current_path {
        if !is_main {
            result.push((p, current_branch));
        }
    }
    result
}

fn cmd_worktree_list(root: &Path) -> anyhow::Result<()> {
    let states = ["pending", "running", "done", "merged", "failed"];
    // (worktree_path, id, branch, state)
    let mut card_worktrees: Vec<(PathBuf, String, String, String)> = Vec::new();

    for &state in &states {
        let dir = root.join(state);
        if !dir.exists() { continue; }
        for ent in fs::read_dir(&dir)?.flatten() {
            let card_dir = ent.path();
            if !card_dir.is_dir() { continue; }
            if card_dir.extension().and_then(|s| s.to_str()).unwrap_or("") != "jobcard" { continue; }
            let wt_path = card_dir.join("worktree");
            if !wt_path.exists() { continue; }
            let meta = jobcard_core::read_meta(&card_dir).ok();
            let id = meta.as_ref().map(|m| m.id.clone())
                .unwrap_or_else(|| card_dir.file_stem().and_then(|s| s.to_str()).unwrap_or("?").to_string());
            let branch = meta.as_ref().and_then(|m| m.worktree_branch.clone()).unwrap_or_else(|| "?".to_string());
            card_worktrees.push((wt_path, id, branch, state.to_string()));
        }
    }

    // Print header
    println!("{:<20} {:<30} {:<10} PATH", "ID", "BRANCH", "STATUS");
    for (path, id, branch, state) in &card_worktrees {
        println!("{:<20} {:<30} {:<10} {}", id, branch, state, path.display());
    }

    // Detect orphaned git worktrees
    let git_paths: Vec<PathBuf> = git_worktree_paths(root)
        .into_iter()
        .map(|(p, _)| p)
        .collect();

    let known_paths: std::collections::HashSet<PathBuf> =
        card_worktrees.iter().map(|(p, _, _, _)| p.clone()).collect();

    for gp in git_paths {
        if !known_paths.contains(&gp) {
            println!("{:<20} {:<30} {:<10} {}", "[orphaned]", "?", "orphaned", gp.display());
        }
    }

    Ok(())
}
```

> **Note on the `is_main` flag:** The first block emitted by `git worktree list --porcelain` is always the main worktree. Detect it by checking if the path equals the git root or by checking for the absence of a `branch` line AND presence of `HEAD` without a `branch` prefix. The simplest reliable approach: mark the very first block as main.

Revise `git_worktree_paths` to mark the first block as main:

```rust
fn git_worktree_paths(from_dir: &Path) -> Vec<(PathBuf, String)> {
    let Ok(out) = StdCommand::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(from_dir)
        .output()
    else {
        return vec![];
    };
    if !out.status.success() {
        return vec![];
    }

    let text = String::from_utf8_lossy(&out.stdout);
    let mut result = Vec::new();
    let mut current_path: Option<PathBuf> = None;
    let mut current_branch = String::new();
    let mut block_index: usize = 0;

    for line in text.lines() {
        if line.starts_with("worktree ") {
            if let Some(p) = current_path.take() {
                if block_index > 1 {  // skip index 1 = the main worktree block
                    result.push((p, std::mem::take(&mut current_branch)));
                }
            }
            current_path = Some(PathBuf::from(&line["worktree ".len()..]));
            current_branch = String::new();
            block_index += 1;
        } else if let Some(rest) = line.strip_prefix("branch refs/heads/") {
            current_branch = rest.to_string();
        } else if line.is_empty() {
            if let Some(p) = current_path.take() {
                if block_index > 1 {
                    result.push((p, std::mem::take(&mut current_branch)));
                }
            }
        }
    }
    if let Some(p) = current_path {
        if block_index > 1 {
            result.push((p, current_branch));
        }
    }
    result
}
```

### Step 4: Run the tests to verify they pass

```bash
cargo test -p jc --test worktree_harness -- --nocapture
```
Expected: both `worktree_list_*` tests PASS.

### Step 5: Commit

```bash
git add crates/jc/src/main.rs crates/jc/tests/worktree_harness.rs
git commit -m "feat(worktree): implement worktree list with orphan detection"
```

---

## Task 3: `worktree create`

**Files:**
- Modify: `crates/jc/tests/worktree_harness.rs` (add test)
- Modify: `crates/jc/src/main.rs` (implement)

### Step 1: Add the failing test (append to `worktree_harness.rs`)

```rust
#[test]
fn worktree_create_makes_git_worktree_for_pending_card() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    // Init git repo with initial commit
    let git = |args: &[&str]| {
        Command::new("git").args(args).current_dir(td.path()).status().unwrap();
    };
    git(&["init", "-b", "main"]);
    fs::write(td.path().join("README"), "init").unwrap();
    git(&["add", "."]);
    git(&["-c", "user.email=a@b", "-c", "user.name=a", "commit", "-m", "init"]);

    // jc init
    let s = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status().unwrap();
    assert!(s.success());

    // Create a pending card manually
    let card_dir = cards.join("pending").join("create-job.jobcard");
    fs::create_dir_all(card_dir.join("logs")).unwrap();
    fs::create_dir_all(card_dir.join("output")).unwrap();
    write_meta(&card_dir, "create-job", "job/create-job");
    fs::write(card_dir.join("spec.md"), "").unwrap();
    fs::write(card_dir.join("prompt.md"), "").unwrap();

    // Run worktree create
    let out = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "worktree", "create", "create-job"])
        .output().unwrap();
    assert!(out.status.success(), "worktree create failed: {}", String::from_utf8_lossy(&out.stderr));

    // worktree/ directory must exist inside the card
    let wt = card_dir.join("worktree");
    assert!(wt.exists(), "worktree/ dir should exist after create");

    // git should know about the new worktree
    let git_out = Command::new("git")
        .args(["worktree", "list"])
        .current_dir(td.path())
        .output().unwrap();
    let git_list = String::from_utf8_lossy(&git_out.stdout);
    assert!(git_list.contains("create-job") || git_list.contains(wt.to_str().unwrap()),
        "git worktree list should show the new worktree:\n{}", git_list);
}

#[test]
fn worktree_create_fails_for_done_card() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    // Init git repo
    let git = |args: &[&str]| {
        Command::new("git").args(args).current_dir(td.path()).status().unwrap();
    };
    git(&["init", "-b", "main"]);
    fs::write(td.path().join("README"), "init").unwrap();
    git(&["add", "."]);
    git(&["-c", "user.email=a@b", "-c", "user.name=a", "commit", "-m", "init"]);

    let s = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status().unwrap();
    assert!(s.success());

    // Card in done/ — create should refuse
    let card_dir = cards.join("done").join("done-job.jobcard");
    fs::create_dir_all(&card_dir).unwrap();
    write_meta(&card_dir, "done-job", "job/done-job");

    let out = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "worktree", "create", "done-job"])
        .output().unwrap();
    assert!(!out.status.success(), "worktree create should fail for done/ card");
}
```

### Step 2: Run to verify failure

```bash
cargo test -p jc --test worktree_harness worktree_create -- --nocapture
```
Expected: FAIL (todo! panic).

### Step 3: Implement `cmd_worktree_create`

Replace the stub:

```rust
fn cmd_worktree_create(root: &Path, id: &str) -> anyhow::Result<()> {
    // Only allow create for pending or running cards
    let card = ["pending", "running"]
        .iter()
        .find_map(|&state| {
            let p = root.join(state).join(format!("{}.jobcard", id));
            if p.exists() { Some(p) } else { None }
        })
        .with_context(|| format!("card '{}' not found in pending/ or running/", id))?;

    let meta = jobcard_core::read_meta(&card)?;
    let branch = meta.worktree_branch.as_deref().unwrap_or(&format!("job/{}", id)).to_string();

    let wt_path = card.join("worktree");
    if wt_path.exists() {
        anyhow::bail!("worktree already exists for card '{}'", id);
    }

    // Find git root (start from root's parent so we find the enclosing repo)
    let git_root_out = StdCommand::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(root.parent().unwrap_or(root))
        .output()
        .context("failed to run git; is this directory inside a git repo?")?;
    if !git_root_out.status.success() {
        anyhow::bail!("not inside a git repository");
    }
    let git_root = PathBuf::from(String::from_utf8(git_root_out.stdout)?.trim());

    // Try: git worktree add -b <branch> <path>
    // If branch already exists, fall back to: git worktree add <path> <branch>
    let add = StdCommand::new("git")
        .args(["worktree", "add", "-b", &branch, wt_path.to_str().unwrap()])
        .current_dir(&git_root)
        .output()?;

    if !add.status.success() {
        // Branch may already exist — try without -b
        let add2 = StdCommand::new("git")
            .args(["worktree", "add", wt_path.to_str().unwrap(), &branch])
            .current_dir(&git_root)
            .output()?;
        if !add2.status.success() {
            let err = String::from_utf8_lossy(&add2.stderr);
            anyhow::bail!("git worktree add failed: {}", err);
        }
    }

    println!("created worktree for '{}' at {} (branch: {})", id, wt_path.display(), branch);
    Ok(())
}
```

### Step 4: Run tests

```bash
cargo test -p jc --test worktree_harness worktree_create -- --nocapture
```
Expected: PASS.

### Step 5: Run all worktree tests

```bash
cargo test -p jc --test worktree_harness -- --nocapture
```
Expected: all PASS.

### Step 6: Commit

```bash
git add crates/jc/src/main.rs crates/jc/tests/worktree_harness.rs
git commit -m "feat(worktree): implement worktree create"
```

---

## Task 4: `worktree clean`

**Files:**
- Modify: `crates/jc/tests/worktree_harness.rs` (add tests)
- Modify: `crates/jc/src/main.rs` (implement)

### Step 1: Add the failing tests (append to `worktree_harness.rs`)

```rust
#[test]
fn worktree_clean_dry_run_does_not_remove() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let s = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status().unwrap();
    assert!(s.success());

    // Card in done/ with a worktree/ dir
    let done_card = cards.join("done").join("done-clean.jobcard");
    let wt = done_card.join("worktree");
    fs::create_dir_all(&wt).unwrap();
    write_meta(&done_card, "done-clean", "job/done-clean");

    let out = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "worktree", "clean", "--dry-run"])
        .output().unwrap();
    assert!(out.status.success(), "dry-run failed: {}", String::from_utf8_lossy(&out.stderr));

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("done-clean") || stdout.contains("would remove"),
        "dry-run should preview removal:\n{}", stdout);

    // Worktree must still exist after dry-run
    assert!(wt.exists(), "dry-run must NOT delete the worktree");
}

#[test]
fn worktree_clean_removes_done_and_merged_worktrees() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let s = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status().unwrap();
    assert!(s.success());

    // done/ card with worktree
    let done_card = cards.join("done").join("d1.jobcard");
    let done_wt = done_card.join("worktree");
    fs::create_dir_all(&done_wt).unwrap();
    write_meta(&done_card, "d1", "job/d1");

    // merged/ card with worktree
    let merged_card = cards.join("merged").join("m1.jobcard");
    let merged_wt = merged_card.join("worktree");
    fs::create_dir_all(&merged_wt).unwrap();
    write_meta(&merged_card, "m1", "job/m1");

    // pending/ card with worktree — must NOT be removed
    let pending_card = cards.join("pending").join("p1.jobcard");
    let pending_wt = pending_card.join("worktree");
    fs::create_dir_all(&pending_wt).unwrap();
    write_meta(&pending_card, "p1", "job/p1");

    let out = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "worktree", "clean"])
        .output().unwrap();
    assert!(out.status.success(), "clean failed: {}", String::from_utf8_lossy(&out.stderr));

    assert!(!done_wt.exists(),    "done worktree should be removed");
    assert!(!merged_wt.exists(),  "merged worktree should be removed");
    assert!(pending_wt.exists(),  "pending worktree must NOT be removed");
}

#[test]
fn worktree_clean_removes_orphaned_git_worktrees() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    // Init git repo
    let git = |args: &[&str]| {
        Command::new("git").args(args).current_dir(td.path()).status().unwrap();
    };
    git(&["init", "-b", "main"]);
    fs::write(td.path().join("README"), "init").unwrap();
    git(&["add", "."]);
    git(&["-c", "user.email=a@b", "-c", "user.name=a", "commit", "-m", "init"]);

    let s = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status().unwrap();
    assert!(s.success());

    // Orphaned git worktree: no corresponding job card
    let orphan_path = td.path().join("orphan-wt");
    Command::new("git")
        .args(["worktree", "add", "-b", "job/orphan", orphan_path.to_str().unwrap()])
        .current_dir(td.path())
        .status().unwrap();

    assert!(orphan_path.exists(), "orphan worktree should exist before clean");

    let out = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "worktree", "clean"])
        .output().unwrap();
    assert!(out.status.success(), "clean failed: {}", String::from_utf8_lossy(&out.stderr));

    assert!(!orphan_path.exists(), "orphaned git worktree should be removed");
}
```

### Step 2: Run to verify failure

```bash
cargo test -p jc --test worktree_harness worktree_clean -- --nocapture
```
Expected: FAIL (todo! panic).

### Step 3: Implement `cmd_worktree_clean`

Add a helper function and replace the stub:

```rust
/// Remove a directory: first try `git worktree remove --force`, fall back to `fs::remove_dir_all`.
fn remove_worktree(path: &Path, git_root: Option<&Path>) -> anyhow::Result<()> {
    if let Some(root) = git_root {
        let status = StdCommand::new("git")
            .args(["worktree", "remove", "--force", path.to_str().unwrap_or("")])
            .current_dir(root)
            .status();
        if matches!(status, Ok(s) if s.success()) {
            return Ok(());
        }
    }
    // Fallback: plain recursive delete
    fs::remove_dir_all(path).with_context(|| format!("failed to remove {}", path.display()))
}

fn cmd_worktree_clean(root: &Path, dry_run: bool) -> anyhow::Result<()> {
    // Collect worktrees to remove from done/ and merged/
    let stale_states = ["done", "merged"];
    let mut to_remove: Vec<PathBuf> = Vec::new();

    for &state in &stale_states {
        let dir = root.join(state);
        if !dir.exists() { continue; }
        for ent in fs::read_dir(&dir)?.flatten() {
            let card_dir = ent.path();
            if !card_dir.is_dir() { continue; }
            if card_dir.extension().and_then(|s| s.to_str()).unwrap_or("") != "jobcard" { continue; }
            let wt = card_dir.join("worktree");
            if wt.exists() {
                to_remove.push(wt);
            }
        }
    }

    // Find git root (optional — used for orphan detection and clean git state)
    let git_root: Option<PathBuf> = StdCommand::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(root.parent().unwrap_or(root))
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| PathBuf::from(s.trim()));

    // Collect orphaned git worktrees (paths not inside any known pending/running card)
    if let Some(ref gr) = git_root {
        let active_wt_paths: std::collections::HashSet<PathBuf> = ["pending", "running"]
            .iter()
            .flat_map(|&state| {
                let dir = root.join(state);
                fs::read_dir(dir).into_iter().flatten().flatten().filter_map(|e| {
                    let p = e.path();
                    if p.extension().and_then(|s| s.to_str()).unwrap_or("") == "jobcard" {
                        Some(p.join("worktree"))
                    } else {
                        None
                    }
                })
            })
            .collect();

        for (wt_path, _branch) in git_worktree_paths(root) {
            if !active_wt_paths.contains(&wt_path) && !to_remove.contains(&wt_path) {
                to_remove.push(wt_path);
            }
        }
    }

    if to_remove.is_empty() {
        println!("nothing to clean");
        return Ok(());
    }

    for path in &to_remove {
        if dry_run {
            println!("would remove: {}", path.display());
        } else {
            println!("removing: {}", path.display());
            remove_worktree(path, git_root.as_deref())?;
        }
    }

    Ok(())
}
```

### Step 4: Run all worktree tests

```bash
cargo test -p jc --test worktree_harness -- --nocapture
```
Expected: all PASS.

### Step 5: Run full test suite and lints

```bash
make check
```
Expected: all tests pass, no clippy warnings, formatted correctly.

If there are clippy warnings, fix them. Common issues:
- `StdCommand` in `remove_worktree` might shadow an import — check for unused variable warnings
- `is_main` variable in earlier draft was removed — ensure no dead code

### Step 6: Commit

```bash
git add crates/jc/src/main.rs crates/jc/tests/worktree_harness.rs
git commit -m "feat(worktree): implement worktree clean with --dry-run"
```

---

## Verification checklist

After all tasks are complete, verify each acceptance criterion manually:

```bash
# Build release binary
cargo build

# In a git repo with .cards/ set up:
jc --cards-dir .cards worktree list
# → shows table with ID, BRANCH, STATUS, PATH

jc --cards-dir .cards worktree create <some-pending-id>
# → creates worktree/, prints confirmation

jc --cards-dir .cards worktree clean --dry-run
# → prints "would remove: ..." without deleting

jc --cards-dir .cards worktree clean
# → removes done/merged worktrees, prints "removing: ..."
```
