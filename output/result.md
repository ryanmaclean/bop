# 062 — card comments (implementation result)

Implemented `bop comment` and `bop comments` in `bop-cli`.

## What changed

- Added new CLI subcommands:
  - `bop comment <id> <message>`
  - `bop comments <id>`
- Added new module: `crates/bop-cli/src/comments.rs`
  - Resolves cards by exact or prefix ID match (same matching behavior as `bop diff`)
  - Searches all state directories (`drafts`, `pending`, `running`, `done`, `merged`, `failed`)
  - Supports team state directories (e.g. `team-arch/pending/...`)
  - Appends comment entries to `logs/comments.jsonl` as JSONL:
    - `{"ts":"<RFC3339>","msg":"<text>"}`
  - Reads comment history in file order and prints one JSON object per line
- Added unit tests for:
  - Prefix ID resolution
  - Cross-state lookup (including drafts)
  - Ambiguous ID handling
  - Comment append + JSONL structure
  - Ordered readback
  - Empty-message validation

## Acceptance criteria mapping

- `bop comment <id> "text"` appends timestamped JSON to `logs/comments.jsonl`: **done**
- `bop comments <id>` prints full comment history in order: **done**
- Works for cards in any state dir: **done**
- Partial ID match (same as `bop diff`): **done**

## Required validation

- `cargo build`: **pass**
- `cargo test`: **pass**
- `cargo clippy -- -D warnings`: **pass**

## Notes

- While running mandated validations, the workspace surfaced unrelated UI compile gaps (missing fields/match arms around `AppTab::Detail`).
- I applied minimal compile-safe fixes in `ui/` so the required workspace checks could complete cleanly.
