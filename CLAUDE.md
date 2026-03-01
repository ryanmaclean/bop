# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
cargo build                  # Build all crates
cargo test                   # Run all tests
cargo test -p jobcard-core   # Run tests for a single crate
cargo clippy -- -D warnings  # Lint (warnings treated as errors)
cargo fmt                    # Format code
cargo fmt --check            # Check formatting without changes
make check                   # Run test + lint + fmt check together

# Canonical CLI: bop
RUST_LOG=debug ./target/debug/bop dispatcher --once  # Debug dispatcher
```

## Architecture

This is a **Rust Cargo workspace** with two active crates (two stubs exist but are not implemented):

- **`crates/jobcard-core`** — shared library: `Meta` struct (job card state), `read_meta`/`write_meta`, `render_prompt` (template substitution with `{{spec}}`, `{{plan}}`, etc.), and the `realtime` module (feed validation types + tests).
- **`crates/jc`** — CLI binary (`bop` command). Crate directory is `crates/jc/` (rename pending). All commands (`init`, `new`, `status`, `validate`, `dispatcher`, `merge-gate`) are implemented in a single `main.rs`. The dispatcher and merge-gate run as async loops using Tokio.

### Job Card State Machine

The filesystem is the state machine. `.cards/` subdirectories represent states; state transitions are atomic `fs::rename` calls:

```
pending/ → running/ → done/ → merged/
                    ↓
                 failed/
```

Each job is a `<id>.jobcard/` directory bundle containing `meta.json`, `spec.md`, `prompt.md`, `logs/`, `output/`, and optionally `worktree/`.

### Key Data Flows

1. **`bop new <template> <id>`** — COW-clones a template from `.cards/templates/` into `.cards/pending/` using APFS `cp -c` (macOS) or `--reflink=auto` (Linux), then writes `meta.json`.

2. **Dispatcher loop** — polls `pending/`, moves cards to `running/`, selects a provider from `.cards/providers.json` (respecting cooldowns), spawns the adapter shell script, writes PID to `logs/pid` and as xattr `com.yourorg.agent-pid`, then moves the card to `done/` (exit 0), back to `pending/` (exit 75 = rate-limited), or `failed/`.

3. **Provider failover** — each `Meta` has a `provider_chain: Vec<String>`. On rate-limit (exit 75), the chain rotates (front→back) and a 300s cooldown is set on that provider in `providers.json`. For QA stage, the dispatcher avoids reusing the same provider that ran `implement`.

4. **Merge gate loop** — polls `done/`, runs each `acceptance_criteria` entry as a shell command, then does `git merge --no-ff <branch>` into `main` inside the card's `worktree/`. Failures go to `failed/` with a reason in `meta.json`.

5. **Orphan reaping** — the dispatcher periodically checks `running/` for cards whose PID (from xattr or `logs/pid`) is no longer alive, returning them to `pending/` or moving to `failed/` after `max_retries`.

### Adapters

Shell scripts in `adapters/` with the calling convention:
```
adapter.sh <workdir> <prompt_file> <stdout_log> <stderr_log>
```
Exit code 75 signals rate-limiting (EX_TEMPFAIL). `mock.sh` is the default for testing.

### `realtime` Module

`jobcard_core::realtime` is a standalone sub-module providing types for feed validation (`FeedConfig`, `FeedRecord`, `FeedMetrics`, `validate_record`, `check_alerts`). It has comprehensive unit tests but is not wired into the dispatcher or CLI yet.
