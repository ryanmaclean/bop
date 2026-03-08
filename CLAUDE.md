# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Skills

Four skills provide orientation for this repo — invoke before working:
- `bop-system` — what `bop` is, card lifecycle, state machine
- `vibekanban` — Quick Look cards, Zellij plugin, glyph system, SwiftUI card anatomy
- `unicode-glyphs` — playing card encoding, BMP/SMP safety, font recommendations
- `bop-team` — drink your own champagne: every unit of work on bop = a bop card

## System context for dispatched agents

`.cards/system_context.md` is auto-prepended to every agent prompt by
`PromptContext::from_files`. Edit it to orient agents without touching templates.

## Commands

```sh
cargo build                  # Build all crates
cargo test                   # Run all tests
cargo test -p bop-core       # Run tests for a single crate
cargo clippy -- -D warnings  # Lint (warnings treated as errors)
cargo fmt                    # Format code
cargo fmt --check            # Check formatting without changes
make check                   # Run test + lint + fmt check together

# Canonical CLI: bop
RUST_LOG=debug ./target/debug/bop dispatcher --once  # Debug dispatcher
bop gantt                    # ANSI Gantt timeline in terminal (auto-fits pane)
bop gantt --html             # HTML Gantt to .cards/bop-gantt.html
bop gantt -o                 # HTML + open in browser
bop gantt -w 80              # Force 80-column width

# JJ-first collaboration (recommended with multiple live agents)
jj status
jj workspace list
jj log -r '::@'
```

## Architecture

This is a **Rust Cargo workspace** with two active crates:

- **`crates/bop-core`** — shared library: `Meta` struct (card state), `read_meta`/`write_meta`, `render_prompt` (template substitution with `{{spec}}`, `{{plan}}`, etc.), and the `realtime` module (feed validation types + tests).
- **`crates/bop-cli`** — CLI binary (`bop` command). Dispatcher and merge-gate support `--vcs-engine git_gt|jj`; prefer `jj` for active multi-agent sessions.

### Multi-Agent VCS Rule (JJ First)

When multiple agents are active, use JJ as the collaboration layer and treat Git as publish/transport:

1. Every agent works in an isolated workspace (`jj workspace add ...`), never directly on shared `main`.
2. Keep one integrator workspace that is the only place allowed to land to `main`.
3. Land only after gates pass in the integrator workspace:
   - `make check`
   - `./target/debug/bop policy check --staged`
4. Publish to Git only from green JJ changes (`jj git push --all`).
5. If a change is partially renamed or mid-refactor, do not land it; queue it behind a decision card.

### Storage — HARD RULES (do not re-litigate)

**NO SQLite. NO Dolt. NO additional JSON files. NO new databases of any kind.**

The storage model is: filesystem as state machine + JSONL append-only event log per card.
This decision is final. Do not propose alternatives. Do not sneak in new dependencies
(rusqlite, sqlx, dolt, sled, redb, etc.). If you think you need a database, you are wrong —
reach for `logs/events.jsonl` (append-only) or `meta.json` (guarded by checksum + readonly perms).

Rationale (also in `docs/storage-decision.md`):
- `fs::rename` is atomic — that IS the transaction
- COW clone (`cp -c`) works on directory bundles, not database rows
- JSONL O_APPEND is atomic for writes < 4096 bytes — no locking needed
- sha256 checksum in `meta.json` detects corruption; JSONL replay recovers it
- Zero external runtime dependencies

### Card State Machine

The filesystem is the state machine. `.cards/` subdirectories represent states; state transitions are atomic `fs::rename` calls:

```
pending/ → running/ → done/ → merged/
                    ↓
                 failed/
```

Each card is a `<id>.card/` directory bundle containing `meta.json`, `spec.md`, `prompt.md`, `logs/`, `output/`, and optionally `worktree/`.

### Key Data Flows

1. **`bop new <template> <id>`** — COW-clones a template from `.cards/templates/` into `.cards/pending/` using APFS `cp -c` (macOS) or `--reflink=auto` (Linux), then writes `meta.json`.

2. **Dispatcher loop** — polls `pending/`, moves cards to `running/`, selects a provider from `.cards/providers.json` (respecting cooldowns), spawns the adapter Nushell script, writes PID to `logs/pid` and as xattr `sh.bop.agent-pid`, then moves the card to `done/` (exit 0), back to `pending/` (exit 75 = rate-limited), or `failed/`.

3. **Provider failover** — each `Meta` has a `provider_chain: Vec<String>`. On rate-limit (exit 75), the chain rotates (front→back) and a 300s cooldown is set on that provider in `providers.json`. For QA stage, the dispatcher avoids reusing the same provider that ran `implement`.

4. **Merge gate loop** — polls `done/`, runs each `acceptance_criteria` entry as a shell command, then merges using the selected engine (`git_gt` or `jj`) into `main` inside the card's workspace/worktree. Failures go to `failed/` with a reason in `meta.json`.

5. **Orphan reaping** — the dispatcher periodically checks `running/` for cards whose PID (from xattr or `logs/pid`) is no longer alive, returning them to `pending/` or moving to `failed/` after `max_retries`.

### Adapters

**Nushell scripts** in `adapters/` with the calling convention:
```
adapter.nu <workdir> <prompt_file> <stdout_log> <stderr_log> [timeout]
```
Exit code 75 signals rate-limiting (EX_TEMPFAIL). `mock.nu` is the default for testing.

### `realtime` Module

`bop_core::realtime` is a standalone sub-module providing types for feed validation (`FeedConfig`, `FeedRecord`, `FeedMetrics`, `validate_record`, `check_alerts`). It has comprehensive unit tests but is not wired into the dispatcher or CLI yet.
