# bop index — implementation complete

## Changes

### `crates/jc/src/main.rs`
- Added `Commands::Index { print: bool }` variant
- Added match arm routing to `cmd_index`
- Implemented `cmd_index(cards_dir, print_stdout)` — derives repo root from `.cards` parent, calls `generate_codebase_index`
- Implemented `generate_codebase_index(repo_root)`:
  - **Top-level layout** — checks 14 known dirs and emits one-line descriptions for those that exist
  - **Key files** — checks 14 important paths and emits path + purpose for those that exist
  - **Crate map** — parses `Cargo.toml` `members = [...]` block, reads `Cargo.toml` of each crate for name, reads first 50 lines of `src/lib.rs` or `src/main.rs` to extract public items
  - **Common patterns** — 7 bullet points describing the system's key conventions

### `crates/jobcard-core/src/lib.rs`
- Added `codebase_index: String` field to `PromptContext`
- `from_files`: walks ancestors to find `.cards/CODEBASE.md` and loads it into `codebase_index`
- `render_prompt`: adds `{{codebase_index}}` substitution
- Updated two test `PromptContext` literals with `codebase_index: String::new()`

## Acceptance criteria

| Check | Result |
|---|---|
| `cargo build` | ✅ |
| `cargo clippy -- -D warnings` | ✅ |
| `./target/debug/bop index --print \| grep -q 'jobcard-core'` | ✅ |
| `jj log -r 'main..@-' \| grep -q .` | ✅ |

## jj commit
`zzrxptlv 57e302a3 feat: add bop index command — generate .cards/CODEBASE.md for agent orientation`
