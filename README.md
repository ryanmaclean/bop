# bop — Heterogeneous Agent Orchestrator

A pluggable job system for parallel AI coding agents. The filesystem IS the state machine: directory bundles (`.jobcard`) are navigable in Finder with Quick Look previews. No database. `mv` = state transition.

**New agents:** read `plan.md` first, then `CLAUDE.md`.

## Quick Start

```bash
cargo build
./target/debug/bop doctor        # verify tooling
./target/debug/bop init           # create .cards/ structure
./target/debug/bop new implement my-feature
./target/debug/bop status
./target/debug/bop dispatcher --once   # process one pending card
./target/debug/bop merge-gate --once   # merge one done card
```

## Architecture

```
Finder / Quick Look / Spotlight       ← FREE (macOS native)
        │ reads bundle state
        ▼
dispatcher (~Rust binary)             ← bop dispatcher
        │ fork per job
        ▼
adapters/ (~20 LOC shell each)        ← claude, codex, goose, aider, ollama, mock
        │ works inside
        ▼
.cards/<state>/<id>.jobcard/          ← APFS COW clones
```

**State machine:** `pending/ → running/ → done/ → merged/` (or `failed/`)

## CLI Commands

```bash
bop new <template> <id>       # Clone template → pending/
bop status                    # Board view across all states
bop inspect <id>              # Show meta/spec/log summary
bop dispatcher [--once]       # Run dispatcher (--once for single pass)
bop merge-gate [--once]       # Run merge gate
bop retry <id>                # Move card back to pending/
bop kill <id>                 # SIGTERM running card → failed/
bop logs <id> [--follow]      # Stream stdout/stderr
bop approve <id>              # Mark done card as merge-ready
bop poker open <id>           # Open estimation round
bop poker submit <id> -n <who> <glyph>   # Submit estimate
bop poker reveal <id>         # Flip all estimates
bop poker consensus <id> <g>  # Lock consensus estimate
bop policy check --staged     # Anti-slop gates on staged changes
bop doctor                    # Verify local tooling
```

## Card Structure

```
my-feature.jobcard/
├── meta.json          ← machine-readable state (glyph, stage, provider_chain)
├── spec.md            ← what to build
├── prompt.md          ← agent prompt with {{variables}}
├── logs/              ← stdout.log, stderr.log, pid
└── output/            ← diff.patch, qa_report.md
```

## Card Symbol Protocol

Cards carry a `glyph` encoding team (suit) and priority (rank):

| Suit | Team | Rank | Priority |
|------|------|------|----------|
| ♠ | CLI/runtime | A | P1 |
| ♥ | Architecture | K/Q | P2 |
| ♦ | QA/reliability | J | P3 |
| ♣ | Platform | 2–10 | P4 |

Joker (🃏) = emergency/breakdown needed. ASCII fallback: `S-A`, `H-K`, `D-7`, `JOKER`.

## Adapters

Shell scripts in `adapters/` — one per AI provider:

```
adapter.sh <workdir> <prompt_file> <stdout_log> <stderr_log>
```

Exit 75 = rate-limited (triggers provider rotation). Available: `claude`, `codex`, `goose`, `aider`, `opencode`, `ollama-local`, `mock`.

## Build & Test

```bash
cargo build                  # Build all crates
cargo test                   # Run all tests
cargo clippy -- -D warnings  # Lint
cargo fmt --check            # Format check
make check                   # All three at once
```

## License

MIT
