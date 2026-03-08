# Spec 055 — bop replay: reconstruct card state from JSONL event log

## Overview

Every card accumulates `logs/events.jsonl` — an append-only record of every
state transition, provider attempt, and error. `bop replay <id>` reads this
log and reconstructs the full card history as a timeline, making it easy to
understand why a card failed, retried, or took a long time.

## Output

```
bop replay team-arch/spec-031
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  team-arch/spec-031  •  merged  •  4 events
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  14:00:00  created        pending      cost=4 provider_chain=[codex,claude]
  14:00:01  dispatched     running      provider=codex  pid=12345
  14:01:43  rate-limited   pending      exit=75  cooldown=300s  rotated→claude
  14:06:44  dispatched     running      provider=claude pid=13210
  14:22:18  completed      done         exit=0  tokens=14200  cost=$0.22
  14:22:19  merged         merged       commit=abc1234
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  Total duration: 22m 19s  •  Retries: 1  •  Cost: $0.22
```

## Implementation

### Event types in events.jsonl

Each line is a JSON object with at minimum:
```json
{"ts": "2026-03-08T14:00:00Z", "event": "created", "state": "pending", ...}
```

The `read_meta` / `write_meta` functions already append events. Check what
fields are currently written and ensure the replay parser handles missing
optional fields gracefully.

### `bop replay` command

1. Find card across all state dirs (same lookup as `bop diff`).
2. Open `logs/events.jsonl`, parse line by line.
3. Render as aligned table with relative timestamps if `--relative` flag,
   absolute UTC if default.
4. Summary row at bottom: total duration, retry count, total cost.

### Flags

```sh
bop replay <id>             # full timeline
bop replay <id> --json      # machine-readable event array
bop replay <id> --errors    # show only error/retry events
bop replay <id> --relative  # relative timestamps (0s, +1m43s, ...)
```

### `bop replay --all`

Show a merged replay of all cards in the last 24h, sorted by event timestamp.
Useful for understanding system-wide activity. Equivalent to merging all
`events.jsonl` files and sorting by `ts`.

## Acceptance Criteria

- [ ] `bop replay <id>` renders event timeline for any card with events.jsonl
- [ ] Missing/empty events.jsonl shows "no events recorded" (graceful)
- [ ] `--json` emits valid JSON array
- [ ] `--errors` filters to only error/retry events
- [ ] `--relative` shows relative timestamps
- [ ] `--all` shows last-24h system-wide timeline
- [ ] Summary row shows total duration, retry count, cost
- [ ] `cargo clippy -- -D warnings` clean
- [ ] `cargo test` passes (unit test with mock events.jsonl)

## Files

- `crates/bop-cli/src/replay.rs` — new module
- `crates/bop-cli/src/main.rs` — wire `bop replay` subcommand
