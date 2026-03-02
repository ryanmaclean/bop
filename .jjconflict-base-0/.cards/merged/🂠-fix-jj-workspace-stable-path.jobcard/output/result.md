# Fix: jj Workspace Stable Path

## What changed

**`crates/jc/src/main.rs`** — `prepare_workspace`, `VcsEngine::Jj` arm:

Before: used `card_dir.join("workspace")` — a path inside the card bundle that goes stale when the bundle renames (e.g. `running/` → `done/`).

After: derives `repo_root.join(".workspaces").join(&ws_name)` as the stable path, with legacy fallback to `card_dir/workspace` if it already exists. Mirrors the `GitGt` arm which uses `.worktrees/<branch>/`.

**`.gitignore`** — added `.workspaces/` entry alongside `.worktrees/`.

## Commit

`wuqprqqm` — fix: stable jj workspace path outside card bundle

## Acceptance criteria

- `cargo build` — pass
- `cargo clippy -- -D warnings` — pass
- `grep -q '\.workspaces' crates/jc/src/main.rs` — pass
- `jj log -r 'main..@-' | grep -q .` — pass
