# wire-action-log — Implementation Summary

## What was done

Modified `adapters/codex.nu` to capture the JSONL event stream from codex when `--json` is supported, writing it to `logs/actions.jsonl` for attestation and auditability.

## Changes

### `adapters/codex.nu`

**New functions:**
- `json_flag_supported` — checks `codex exec --help` for the `--json` flag; returns bool
- `extract_summary` — parses `logs/actions.jsonl` and returns the `content` of the last `agent_response` event

**`main` changes:**
- Derives `logs_dir` and `actions_log` path from the `stdout_abs` argument
- If `--json` is supported: runs `codex exec --json ...` with stdout redirected to `actions.jsonl`, then extracts the human-readable summary and writes it to `stdout_log`
- If `--json` is not supported: falls back to the original plain-text flow (stdout → stdout_log directly)
- Exit code semantics preserved: exit 0 / exit 75 / other

**New tests (17–20):**
- Test 17: `extract_summary` returns `""` for non-existent file
- Test 18: extracts content from last `agent_response` event
- Test 19: returns last when multiple `agent_response` events present
- Test 20: ignores non-`agent_response` events (user messages, usage lines)

### `crates/bop-cli/src/merge_gate.rs`

Fixed pre-existing `cargo fmt` violation (line wrapping on `std::env::temp_dir().join(...)`).

## Acceptance criteria

- [x] `cargo test` — 111 tests pass
- [x] `cargo clippy -- -D warnings` — clean
- [x] `cargo fmt --check` — clean
- [x] `nu adapters/codex.nu --test` — PASS (20 tests)

## How it works

When a real codex invocation runs with `--json`:
1. JSONL events stream to `logs/actions.jsonl`
2. The adapter parses it to find the last `{"type":"agent_response","content":"..."}` line
3. That content is written to `logs/stdout.log` (human-readable summary)
4. `attestation.rs` (future) can call `model_from_action_log` / `action_count_from_log` on the populated file
