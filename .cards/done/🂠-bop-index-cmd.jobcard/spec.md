# Add `bop index` command — codebase map for agents

## Problem
Dispatched agents spend 5–10 of their 13–14 turns just discovering which files exist
and where things are. Reading 600K+ tokens of codebase per card costs ~$0.70.

## Solution
Add `bop index` CLI command that writes `.cards/CODEBASE.md` — a concise (~2K token)
map of the repo that agents read first instead of exploring.

## What to generate

`.cards/CODEBASE.md` should include:
1. **Top-level layout** — one-line description of each directory
2. **Key files** — path + one-sentence purpose for the 20 most important files
3. **Crate map** — for each Cargo crate: name, purpose, main public types/fns
4. **Common patterns** — e.g. "state transitions are `fs::rename`, cards are in `.cards/`"

Keep it under 3000 tokens (about 2000 words).

## Implementation

In `crates/jc/src/main.rs`, add a `Commands::Index` variant.

The command:
1. Walks the workspace with `walkdir` (already a dep) or manual fs calls
2. Lists crates from `Cargo.toml` members
3. For each crate, reads `src/lib.rs` or `src/main.rs` header (first 50 lines)
4. Writes `.cards/CODEBASE.md` with the structured output

## `PromptContext` integration
In `crates/jobcard-core/src/lib.rs`, `PromptContext::from_files` should auto-include
`.cards/CODEBASE.md` if it exists — prepended after `system_context.md` and before
stage template. Add a `{{codebase_index}}` substitution key.

## CLI
```
bop index              # generate/refresh .cards/CODEBASE.md
bop index --print      # print to stdout instead of file
```

## Acceptance Criteria
- `cargo build`
- `cargo clippy -- -D warnings`
- `cargo test -p jc 2>&1 | grep -v FAILED | grep -q 'test result'`
- `./target/debug/bop index --print | grep -q 'jobcard-core'`
- `jj log -r 'main..@-' | grep -q .`
