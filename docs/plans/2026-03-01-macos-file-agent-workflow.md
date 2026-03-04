# macOS File-Agent Workflow Plan (bop)

## Session Resurrection Snapshot

This plan resumes work from the pre-rename (`gtfs`) session and unifies three
partially-progressed threads:

- `ql-interactive`: Quick Look actions should trigger real job control.
- `fsevents-watcher`: dispatcher should react to filesystem events with low idle CPU.
- `persistent-memory`: agent context should persist across repeated runs.

Repo path has moved to `/Users/studio/bop`; stale `/Users/studio/gtfs` paths
must be treated as migration debt.

## Goal

Ship a macOS-first workflow where files are the source of truth and the agent
control loop is fast, observable, and recoverable:

pending card -> event wake -> running agent -> done -> merge-gate -> merged.

## Architecture

1. Card Ingress
- Input: `.cards/pending/<id>.jobcard/`.
- Contract: bundle contains `meta.json`, `spec.md`, `prompt.md`, `logs/`, `output/`.

2. Dispatcher Activation
- Primary: FSEvents watcher on `.cards/**/pending/`.
- Fallback: polling loop every 1s when watcher is unavailable.
- Invariants:
  - atomic state transition via `fs::rename`,
  - provider cooldown/failover preserved,
  - orphan reaper continues on timer.

3. Agent Execution
- Adapter launched from repo-local paths (`adapters/*.nu`).
- PID recorded in `logs/pid` and xattr for kill/reap.
- Exit handling:
  - `0` -> `done/`,
  - `75` -> back to `pending/` + rotate provider,
  - other -> `failed/` with reason.

4. Finder / Quick Look Control Surface
- QL preview action routes:
  - `jobcard://stop/<id>` -> JobCardHost -> `bop kill <id>`
  - `jobcard://approve/<id>` -> JobCardHost -> `bop approve <id>`
- Extension remains sandboxed.
- Host app handles URL + subprocess execution.

5. Merge Gate
- Executes `acceptance_criteria` shell commands.
- On success, merges in selected VCS engine and writes merge artifacts (including `changes.json` for QL).

6. Memory (Optional)
- Namespace memory under `.cards/memory/<namespace>.json`.
- Prompt context injects `{{memory}}` in future stage templates.

## launchd Topology

- `com.yourorg.bop.dispatcher.plist`
  - ProgramArguments: `.../target/debug/bop dispatcher ...`
  - KeepAlive: true
  - RunAtLoad: true
- `com.yourorg.bop.merge-gate.plist`
  - ProgramArguments: `.../target/debug/bop merge-gate ...`
  - KeepAlive: true
  - RunAtLoad: true

Operational rule: launchd restart + orphan reaping should restore progress after crashes without manual intervention.

## Rename Hardening Rules

1. No absolute `/Users/studio/gtfs/...` adapter commands in active provider configs.
2. Prefer repo-relative adapter paths (`adapters/claude.nu`, etc.).
3. Historical merged/failed cards may keep old paths as immutable audit records.
4. New docs/examples use `bop` (legacy `jc` only where compatibility is explicitly discussed).

## SLOs

- Pickup latency: `<100ms` on event-driven path.
- Idle CPU: `<1%` with no pending cards.
- Recovery: launchd restart returns queue processing without data loss.
- Control action latency (Stop/Approve): median `<1s` from click to state change.

## Phased Execution

Phase 1: Rename Integrity
- Update team provider configs to repo-relative adapters.
- Validate no active provider file references old repo path.

Phase 2: Control Surface
- Complete URL-scheme bridge in Quick Look + JobCardHost.
- Validate `bop kill` / `bop approve` from preview interactions.

Phase 3: Event Loop
- Integrate FSEvents path with polling fallback.
- Measure pickup latency and idle CPU.

Phase 4: Memory Injection
- Add minimal `.cards/memory/` store and prompt injection.
- Define TTL + merge semantics.

## Verification Checklist

- `rg -n "/Users/studio/gtfs/adapters" .cards/team-*/providers.json` returns no matches.
- URL routing is documented and testable for Stop/Approve.
- Event source, fallback behavior, and recovery semantics are explicit.
- Plan provides direct next implementation slices, not abstract principles.
