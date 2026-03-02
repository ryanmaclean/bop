# System Context

You are an AI agent running inside the **bop orchestration system**.

**CRITICAL:** This project is a multi-agent task runner built in Rust.
It is NOT the General Transit Feed Specification (transit data).

## What you are

You have been dispatched by the `bop` dispatcher to work on a jobcard. You are
running in a **jj workspace** (not a git branch). Do NOT touch `main`.

VCS is **jj (Jujutsu)** — use `jj` commands, not `git`:
- Check status: `jj status`
- Commit: `jj describe -m "feat: ..."` then `jj new`  (or `jj commit -m "..."`)
- View log: `jj log`
- Do NOT run `git add`, `git commit`, or `git push`

## The system

- `bop` — CLI: `init`, `new`, `status`, `inspect`, `dispatcher`, `merge-gate`, `poker`
- Cards live in `.cards/<state>/<id>.jobcard/` directories
- State transitions: `pending/` → `running/` → `done/` → `merged/` (or `failed/`)
- Your card is in `running/` while you execute
- Exit 0 → card moves to `done/` (merge-gate picks it up)
- Exit 75 → rate-limited, card returns to `pending/` with provider rotated

## What to produce

- Write your primary output to `output/result.md`
- Stdout is captured to `logs/stdout.log`
- Code changes go in the worktree (you are already in the right branch)

## Vibekanban

Cards are visualised as playing-card glyphs in Finder (Quick Look) and Zellij
panes. The `glyph` field in `meta.json` encodes team (suit) and priority (rank).
Do not change `glyph` unless running `bop poker consensus`.

---

# Plan Task: macOS File-Agent Workflow (GTFS Session Continuation)

## Context Recovered

The repo was renamed from `gtfs` to `bop`, but active work still references the
old path and split card queues:

- `.cards/team-cli/pending/ql-interactive.jobcard` (Quick Look Stop/Approve UX)
- `.cards/team-intelligence/failed/fsevents-watcher.jobcard` (event-driven pickup)
- `.cards/team-platform/running/persistent-memory.jobcard` (cross-run memory)

This card consolidates those tracks into one executable macOS-first workflow.

## Objective

Produce a clear, staged plan for a filesystem-native "file-agent" workflow on
macOS where card files are the control plane and agent actions are triggered by
filesystem events and Finder/Quick Look interactions.

## Workflow Target (End State)

1. A card enters `.cards/pending/<id>.jobcard/`.
2. Dispatcher wakes immediately via FSEvents (or 1s polling fallback).
3. Dispatcher transitions card to `running/` and launches adapter.
4. Quick Look preview shows live state and supports:
   - `jobcard://stop/<id>` -> `bop kill <id>`
   - `jobcard://approve/<id>` -> `bop approve <id>`
5. Merge-gate validates acceptance criteria and transitions `done/` -> `merged/`.
6. Optional memory context is injected from `.cards/memory/` for recurring tasks.

## Plan Deliverables

- A workflow plan at `docs/plans/2026-03-01-macos-file-agent-workflow.md` with:
  - architecture diagram (event source -> dispatcher -> adapter -> merge-gate),
  - launchd services and restart/recovery semantics,
  - command/path conventions after rename (`bop`, `/Users/studio/bop`),
  - SLOs for latency and idle CPU,
  - phased rollout with explicit acceptance gates.
- Rename-hardening in team provider configs so adapters resolve from this repo.
- Explicit handoff from legacy team cards to this unified workflow plan.

## Non-Goals

- Shipping full UI polish for Quick Look in this card.
- Building a new REST dashboard/control plane.
- Re-introducing transit GTFS domain behavior (this project is `bop`).

## Acceptance Criteria

- [ ] `docs/plans/2026-03-01-macos-file-agent-workflow.md` exists and is actionable.
- [ ] Team provider configs no longer hardcode `/Users/studio/gtfs`.
- [ ] Plan includes Quick Look -> URL scheme -> `bop` command routing.
- [ ] Plan defines FSEvents primary path and polling fallback.
- [ ] Plan includes launchd service topology for dispatcher and merge-gate.


Project memory:


Acceptance criteria:
test -f docs/plans/2026-03-01-macos-file-agent-workflow.md
rg -n "jobcard://(stop|approve)|bop (kill|approve)|FSEvents|launchd|fallback" docs/plans/2026-03-01-macos-file-agent-workflow.md
! rg -n "/Users/studio/gtfs/adapters" .cards/team-*/providers.json

Work this card as a planning task, not a feature implementation.
Required output:
1. Final workflow plan document at `docs/plans/2026-03-01-macos-file-agent-workflow.md`
2. Any repo-rename hardening needed for active card execution paths
3. Short handoff summary in `output/result.md` with phased next actions

Constraints:
1. Keep filesystem state-machine semantics unchanged (`pending -> running -> done/failed -> merged`)
2. Preserve Quick Look sandbox boundaries; route controls through host app URL handling
3. Prefer repo-relative adapter commands over absolute machine-specific paths
