# job-control: bop retry / kill / logs commands

## Goal

Implement three missing CLI commands in `crates/bop-cli/src/main.rs`:

- `bop retry <id>` — move a failed card back to pending/, reset failure fields
- `bop kill <id>` — SIGTERM the agent PID from `logs/pid` or xattr, move card to failed/
- `bop logs <id> [--follow]` — stream stdout/stderr logs from the card's `logs/` dir

## Context

Card spec is at `.cards/team-cli/failed/job-control-retry.jobcard/spec.md`.
The card is in `failed/` — it previously failed. This spec supersedes it.

State dirs: `.cards/<team>/<state>/<id>.jobcard/`
Meta file: `meta.json` with fields `failure_reason`, `retry_count`, `agent_pid`
Log files: `logs/stdout.log`, `logs/stderr.log`, `logs/pid`

## Steps

1. Add `Retry`, `Kill`, `Logs` variants to the `Command` enum in `crates/bop-cli/src/main.rs`.

2. Implement `bop retry <id>`:
   - Search all team dirs + root for `<id>.jobcard` in `failed/`
   - `fs::rename` to `pending/`
   - Increment `retry_count` in meta.json, clear `failure_reason`
   - Print confirmation

3. Implement `bop kill <id>`:
   - Find card in `running/`
   - Read PID from `logs/pid` (fallback: xattr `sh.bop.agent-pid`)
   - Send SIGTERM via `nix::sys::signal` or `libc::kill`
   - Move card to `failed/`, set `failure_reason: "killed"`

4. Implement `bop logs <id> [--follow]`:
   - Find card in any state dir
   - Print `logs/stdout.log` and `logs/stderr.log`
   - With `--follow`: tail both files, interleave output until Ctrl-C

5. Add unit tests for retry (moves card, increments retry_count).

6. Run `make check`.

## Acceptance

```sh
bop retry <id>     # moves failed card to pending
bop kill <id>      # terminates running agent, card → failed
bop logs <id>      # prints stdout+stderr logs
bop logs <id> --follow  # tails live logs
```
`make check` passes.
