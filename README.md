# bop — Heterogeneous Agent Orchestrator

A pluggable job system for parallel AI coding agents. The filesystem IS the state machine: directory bundles (`.card`) are navigable in Finder with Quick Look previews. No database. `mv` = state transition.

Reliability guards:
- single-dispatcher lock: `.cards/.locks/dispatcher.lock`
- per-run lease heartbeat: `<card>/logs/lease.json`

## Install

**macOS (pre-built):**
Download the latest [release](https://github.com/ryanmaclean/bop/releases) — DMG or PKG installer.

The DMG includes:
- `bop` CLI binary (ARM64)
- `BopDeck.app` with embedded Quick Look extension
- `zellij` terminal multiplexer (MIT, pre-built)
- Zellij layouts and WASM status plugin
- Sample cards

**From source:**
```zsh
cargo build --release
cp target/release/bop /usr/local/bin/
```

## Quick Start

```zsh
bop doctor                       # verify tooling
bop init                         # create .cards/ structure
bop new implement my-feature     # create a card from template
bop status                       # board view across all states
bop dispatcher --once            # process one pending card
bop merge-gate --once            # merge one done card
```

## Architecture

```
Finder / Quick Look / Spotlight       ← FREE (macOS native)
        │ reads bundle state
        ▼
dispatcher (Rust binary)              ← bop dispatcher
        │ fork per job
        ▼
adapters/ (~20 LOC nushell each)      ← claude, codex, goose, aider, ollama, mock
        │ works inside
        ▼
.cards/<state>/<id>.card/             ← APFS COW clones
```

**State machine:** `pending/ → running/ → done/ → merged/` (or `failed/`)

Storage contract: see `docs/format-storage-contract.md`.

## Filesystem Safety

- State transitions are atomic `rename` operations.
- Dispatcher acquires a lock directory; stale lock owners are reclaimed by PID liveness check.
- Running cards carry a lease heartbeat (`logs/lease.json`); reaper treats dead PID **or** stale heartbeat as orphan.
- `bop kill` is idempotent for stale PIDs and still exits the card from `running/`.

## CLI Commands

```zsh
bop new <template> <id>       # Clone template → pending/
bop status                    # Board view across all states
bop inspect <id>              # Show meta/spec/log summary
bop gantt                     # ANSI Gantt timeline of card runs
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

macOS maintenance:
```nu
nu scripts/macos_cards_maintenance.nu            # refresh terminal-state card thumbnails
nu scripts/macos_cards_maintenance.nu --compress # + HFS/APFS compression
```

## Card Structure

```
my-feature.card/
├── meta.json          ← machine-readable state (glyph, stage, provider_chain)
├── spec.md            ← what to build
├── prompt.md          ← agent prompt with {{variables}}
├── logs/              ← stdout.log, stderr.log, pid, lease.json
├── output/            ← qa_report.md
├── QuickLook/         ← Thumbnail.png
└── changes.json       ← git diff summary
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

Nushell scripts in `adapters/` — one per AI provider:

```
adapter.nu <workdir> <prompt_file> <stdout_log> <stderr_log> [timeout]
```

Exit 75 = rate-limited (triggers provider rotation). Available: `claude`, `codex`, `goose`, `aider`, `opencode`, `ollama-local`, `mock`.

Template cloning on macOS prefers `ditto --clone` (APFS COW), with `cp -c` fallback.
Terminal-state card compression uses `ditto --hfsCompression` (macOS only).

## BopDeck.app

The macOS companion app provides:
- **Quick Look** — preview `.card` bundles in Finder (thumbnail + full preview)
- **Notch overlay** — live card status in the menu bar area
- **`bop://` URL scheme** — deep link to cards

The Quick Look extension (`BopDeckQL.appex`) is embedded in `BopDeck.app` and declares the `sh.bop.card` UTI.

## Zellij Integration

bop ships a Zellij layout (`layouts/bop.kdl`) and a WASM status bar plugin showing live card counts per state. Launch with:

```zsh
zellij --layout layouts/bop.kdl
```

## Build & Test

```zsh
cargo build                  # Build all crates
cargo test                   # Run all tests
cargo clippy -- -D warnings  # Lint
cargo fmt --check            # Format check
make check                   # All three at once
```

## License

[MIT](LICENSE)
