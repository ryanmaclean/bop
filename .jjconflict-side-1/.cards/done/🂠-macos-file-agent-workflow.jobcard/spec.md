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
