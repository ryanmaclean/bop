# Result: job-token-cost-in-inspect-v2

## What was done

Updated `cmd_inspect` in `crates/jc/src/main.rs` to display a formatted runs
table matching the spec.

### Changes

**File:** `crates/jc/src/main.rs` — `cmd_inspect` function (~line 5246)

Replaced the conditional `if !meta.runs.is_empty()` block with an
unconditional runs table section:

- Header format: `=== runs (N attempts) ===` (always shown, even when 0)
- Columns: `#N  started_at  provider/model  stage  outcome  duration  cost`
- `started_at` truncated to seconds (first 19 chars of ISO-8601)
- `provider/model` combined with `/` separator
- Missing duration or cost displays as `—` (U+2014 em-dash)
- Fixed-width column alignment via format string width specifiers

## Acceptance criteria

- [x] `cargo build` — clean
- [x] `cargo test` — 64 tests passed, 0 failed
- [x] `cargo clippy -- -D warnings` — clean

## Example output

```
=== runs (2 attempts) ===
  #1   2026-03-02T06:58:21   claude/claude-sonnet-4-6   implement     done      92s     $0.61
  #2   2026-03-02T07:01:03   codex/sonnet               qa            failed    12s     $0.04

=== runs (0 attempts) ===
```
