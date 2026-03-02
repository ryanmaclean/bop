# System Context

You are an AI agent running inside the **bop orchestration system**.

**CRITICAL:** This project is a multi-agent task runner built in Rust.
It is NOT the General Transit Feed Specification (transit data).

## What you are

You have been dispatched by the `bop` dispatcher to work on a jobcard. You are
running in a **jj workspace** (not a git branch). Do NOT touch `main`.

VCS is **jj (Jujutsu)** — use `jj` commands, not `git`:
- Check status: `jj status`
- Commit: `jj describe -m "feat: ..."` then `jj new`  (or `jj commit -m "..."`)
- View log: `jj log`
- Do NOT run `git add`, `git commit`, or `git push`

## The system

- `bop` — CLI: `init`, `new`, `status`, `inspect`, `dispatcher`, `merge-gate`, `poker`
- Cards live in `.cards/<state>/<id>.jobcard/` directories
- State transitions: `pending/` → `running/` → `done/` → `merged/` (or `failed/`)
- Your card is in `running/` while you execute
- Exit 0 → card moves to `done/` (merge-gate picks it up)
- Exit 75 → rate-limited, card returns to `pending/` with provider rotated

## What to produce

- Write your primary output to `output/result.md`
- Stdout is captured to `logs/stdout.log`
- Code changes go in the worktree (you are already in the right branch)

## Vibekanban

Cards are visualised as playing-card glyphs in Finder (Quick Look) and Zellij
panes. The `glyph` field in `meta.json` encodes team (suit) and priority (rank).
Do not change `glyph` unless running `bop poker consensus`.

---

{{system_context}}

---

# Stage: Implement

You are **implementing** this card.

Read the spec (and plan, if present). Write code in the workspace.

Requirements:
- Work only inside the declared scope (see spec boundaries)
- Edit files using your tools, then build and test
- Run `cargo build` and `cargo test` before finishing
- Write output summary to `output/result.md`
- If tests fail, fix them. Do not leave broken code.

**Commit your work (jj):**
```
jj describe -m "feat: <what you did>"
jj new
```
Or if you prefer a single commit: `jj commit -m "feat: <what you did>"`

Do NOT use `git add` or `git commit` — this repo uses jj.

Exit 0 only when:
1. You have committed at least one change (jj log shows a new commit)
2. The implementation compiles and tests pass
3. Scope is met


---

Card: {{id}} {{glyph}}
Stage: implement (1 of 1)

---

# Fix: jj Workspace Path Goes Stale When Card Bundle Moves

## Background

We already fixed this for the `GitGt` VCS engine (commit ef9446a):
worktrees now land at `<git_root>/.worktrees/<branch-name>/` instead
of inside the card bundle.

The `Jj` arm has the same bug. `prepare_workspace` (crates/jc/src/main.rs)
puts the jj workspace at `card_dir.join("workspace")`:

```rust
// line ~2447
let ws_path = card_dir.join("workspace");   // shared by both arms at top
...
VcsEngine::Jj => {
    let repo_root = find_git_root(cards_dir)...;
    let ws_name = next_workspace_name(card_id);
    jobcard_core::worktree::create_workspace_with_name(&repo_root, &ws_path, &ws_name)?;
```

When the card bundle renames (`running/ → done/`), the jj workspace path
recorded in `.jj/` becomes stale.

## Fix

Mirror the GitGt fix: for the Jj arm, derive a stable path at
`<repo_root>/.workspaces/<ws_name>/` (note: `.workspaces/` not `.worktrees/`
to stay distinct from the git arm).

```rust
VcsEngine::Jj => {
    let repo_root = find_git_root(cards_dir)
        .unwrap_or_else(|| cards_dir.to_path_buf());
    jobcard_core::worktree::ensure_jj_repo(&repo_root)?;
    let ws_name = next_workspace_name(card_id);
    // Stable path outside the card bundle
    let stable_ws = repo_root.join(".workspaces").join(&ws_name);
    let legacy_ws = card_dir.join("workspace");
    let ws_path = if stable_ws.exists() {
        stable_ws
    } else if legacy_ws.exists() {
        legacy_ws
    } else {
        stable_ws
    };
    jobcard_core::worktree::create_workspace_with_name(&repo_root, &ws_path, &ws_name)?;
```

Also ensure `.workspaces/` is in `.gitignore` (check first, add only if missing).

## Steps

1. Edit `crates/jc/src/main.rs` — find the `Jj` arm of `prepare_workspace`
2. Replace the `ws_path` derivation as shown above
3. Check `.gitignore` for `.workspaces/` entry
4. `cargo build && cargo clippy -- -D warnings`
5. Commit: `jj describe -m "fix: stable jj workspace path outside card bundle"` then `jj new`

## Acceptance Criteria

- `cargo build`
- `cargo clippy -- -D warnings`
- `grep -q '.workspaces' crates/jc/src/main.rs`
- `jj log -r 'main..@-' | grep -q .`   (at least one new commit)

## Scope

Touch only `crates/jc/src/main.rs` and `.gitignore`.




Acceptance criteria:
cargo build
cargo clippy -- -D warnings
grep -q '\.workspaces' crates/jc/src/main.rs
jj log -r 'main..@-' | grep -q .
