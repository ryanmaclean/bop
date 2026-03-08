# bop — Heterogeneous Agent Orchestrator

![CI](https://github.com/ryanmaclean/bop/actions/workflows/ci.yml/badge.svg)

A pluggable job system for parallel AI coding agents. The filesystem IS the state machine: directory bundles (`.bop`) are navigable in Finder with Quick Look previews. No database. `mv` = state transition.

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
```sh
cargo build --release
cp target/release/bop /usr/local/bin/
```

## Quick Start

```sh
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

```sh
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
bop providers                 # Show AI provider quota/usage table (Claude, Codex, Gemini, Ollama…)
bop providers --watch         # Live-refresh quota display every 30s
bop bridge listen             # Start session state bridge daemon (~/.bop/bridge.sock)
bop bridge emit --cli claude --event stage-change --stage in-progress
bop bridge install --target claude  # Hook Claude Code to emit bridge events automatically
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

## Provider Quota (`bop providers`)

`bop providers` auto-detects installed AI CLIs and shows live quota/usage:

```
Provider     Primary  Secondary  Reset        Source
Claude Code  ▓▓▓▓▓░░   ▓▓░░░░░   5h 23m       oauth
Codex        ▓▓▓░░░░   —         —            rpc
Gemini       ▓░░░░░░   ▓▓░░░░░   2d 11h       oauth
Ollama       local     —         —             http
```

Credentials are read from their native locations (`~/.claude/.credentials.json`,
Keychain, `~/.config/opencode/`, etc.) — no configuration needed.

## Session State Bridge (`bop bridge`)

`bop bridge` maps live AI session events to BopDeck's notch display and card
stage history. Cards move through five visible stages:

```
planning → in-progress → human-review → ai-review → done
```

### Install once (Claude Code)

```sh
bop bridge install --target claude
```

This writes hooks into `~/.claude/settings.json`. After that, every Claude Code
session automatically emits `SessionStart`, `ToolStart`, `ToolDone`, and
`SessionEnd` events to the bridge socket — no per-card setup needed.

### How agent sessions use it

Agents running inside bop cards are instructed (via `.cards/system_context.md`)
to call stage-change events at natural transition points:

```sh
# Agent starts implementation:
bop bridge emit --cli claude --event stage-change --stage in-progress

# Agent finishes, awaiting human review:
bop bridge emit --cli claude --event stage-change --stage human-review
```

These calls are **fire-and-forget**: if the bridge daemon isn't running they
exit 0 silently and never block the agent's actual work.

### Multi-CLI coverage

| CLI | Integration method |
|-----|-------------------|
| Claude Code | Hook system (`~/.claude/settings.json`) — automatic after `bop bridge install` |
| opencode | `bop bridge opencode` subscribes to SSE bus (`localhost:4096/event`) |
| Goose | SSE adapter (spec 035) |
| Gemini CLI | ACP ndjson adapter (spec 035) |
| Aider / Crush | Log file tail (spec 035) |
| Ollama | REST poll (spec 035) |

Events are appended to `~/.bop/bridge-events.jsonl` and streamed to BopDeck
via `~/.bop/bridge.sock`.

## BopDeck.app

BopDeck is a macOS companion app built in `macos/`. It has two surfaces:

### Quick Look extension — fully implemented

Press **Space** on any `.card` bundle in Finder to get a rich preview:

- **Overview tab** — glyph, stage pipeline (Spec → Plan → Code → QA with ✓), labels, description, acceptance criteria, created/elapsed time
- **Subtasks tab** — progress bar + checklist from `meta.json`
- **Plan tab** — Auto-Claude `implementation_plan.json` phases and subtasks, collapsible
- **Logs tab** — live log tail (last 30 lines, auto-refreshes every 2s with pulsing indicator)
- **Files tab** — bundle file listing
- **Action buttons** — "Attach Session" (opens Zellij), "Tail Logs", "Stop", "Open Spec" — all via `bop://` deep links

### `bop://` URL scheme — fully implemented

`BopDeck.app` handles `bop://card/<id>/<action>` URLs:

| URL | Action |
|-----|--------|
| `bop://card/<id>/session` | `zellij attach <session>` in Ghostty/Terminal |
| `bop://card/<id>/tail` | `bop logs <id> --follow` |
| `bop://card/<id>/logs` | `bop logs <id>` |
| `bop://card/<id>/stop` | `bop kill <id>` |
| `bop://card/<id>/spec` | Open `spec.md` in default editor |

Card lookup searches all state dirs (`running/`, `pending/`, `done/`, `merged/`, `failed/`) for the card. The Zellij session name is read from `meta.json` `zellij_session` field.

### Main window — stub (planned)

`ContentView.swift` is currently a placeholder. A kanban board view and notch overlay showing live card counts and bridge events are planned but not yet implemented.

Build and deploy:
```sh
xcodebuild -project macos/macos.xcodeproj -scheme JobCardHost -configuration Debug build
# Then copy to /Applications/ and re-register the QL extension:
lsregister -f /Applications/JobCardHost.app
qlmanage -r && qlmanage -r cache
```

## Zellij Integration

bop ships a Zellij layout (`layouts/bop.kdl`) and a WASM status bar plugin showing live card counts per state. Launch with:

```sh
zellij --layout layouts/bop.kdl
```

## Build & Test

```sh
cargo build                  # Build all crates
cargo test                   # Run all tests
cargo clippy -- -D warnings  # Lint
cargo fmt --check            # Format check
make check                   # All three at once
```

## License

[MIT](LICENSE)
