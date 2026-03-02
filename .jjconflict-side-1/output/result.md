# bop index command — implementation complete

## Summary

Added `bop index` CLI command and `{{codebase_index}}` template substitution.

## Changes

### `crates/jc/src/main.rs`
- Added `Command::Index { print: bool }` variant to the `Command` enum
- Added match arm in `main()` routing to `cmd_index`
- Implemented `cmd_index(cards_root, print_flag)` which:
  - Canonicalizes `cards_root` to get the repo root path
  - Reads `Cargo.toml` to extract workspace members (precise `members = [...]` block parser)
  - For each crate: reads first 60 lines of `src/lib.rs` or `src/main.rs` to extract public API items
  - Emits 5 sections: Header, Top-level layout, Key files, Crate map, Common patterns
  - Writes to `.cards/CODEBASE.md` (default) or prints to stdout (`--print`)

### `crates/jobcard-core/src/lib.rs`
- Added `codebase_index: String` field to `PromptContext`
- Updated `PromptContext::from_files` to load `.cards/CODEBASE.md` while walking ancestors (alongside `system_context.md`)
- Added `{{codebase_index}}` substitution in `render_prompt`
- Updated both test `PromptContext` struct literals to include `codebase_index: String::new()`

## Acceptance criteria results

- `cargo build` ✓
- `cargo clippy -- -D warnings` ✓
- `./target/debug/bop index --print | grep -q 'jobcard-core'` ✓
- `jj log -r 'main..@-' | grep -q .` ✓

## Usage

```
bop index              # write .cards/CODEBASE.md
bop index --print      # print to stdout
```

The generated `CODEBASE.md` is ~2000 words covering: top-level directory layout,
key file descriptions, per-crate public API summary (from first 60 source lines),
and common patterns agents need to know.

Agents can reference the index via `{{codebase_index}}` in prompt templates, or
the dispatcher will auto-include it when `PromptContext::from_files` finds
`.cards/CODEBASE.md`.
