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
