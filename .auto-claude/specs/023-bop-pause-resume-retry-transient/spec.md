# bop pause / resume / retry-transient

## Context

On a laptop with battery and intermittent connectivity there is no way to:
- Gracefully stop all in-flight work before closing the lid
- Resume exactly where work left off after waking
- Retry cards that failed due to transient errors (network drop, rate limit)
  without retrying cards that failed for code reasons

## What to do

### 1. `bop pause` command

Gracefully stop all running adapters and return their cards to `pending/`.

For each card in `.cards/running/`:

1. Read `logs/pid`. If missing, treat as already-dead (move to pending).
2. Send `SIGTERM` to the adapter process.
3. **Before renaming**, check if the adapter exited voluntarily with code 75
   (rate-limit) during the wait window. If it did, let the normal dispatcher
   exit-code handling apply — do NOT also rename the card. The dispatcher owns
   exit-75 cards; pause does not double-move them.
4. Wait up to 5 seconds for exit. If still alive, send `SIGKILL`.
5. `write_meta` with `stage = "pending"` and `paused_at = <ISO8601 timestamp>`.
6. `fs::rename` the card from `running/` → `pending/`.
7. Print: `⏸  paused: <id> (adapter PID <n> stopped)`

**Race condition**: A concurrent dispatcher instance could pick up the card
between the kill and the rename. Mitigate by checking that the card directory
still exists in `running/` immediately before the rename; if it has already
moved (another process beat us), skip and log a warning.

If nothing is running: `bop pause: nothing running.`

### 2. `bop resume` command

Re-dispatch all paused cards (cards in `pending/` with a `paused_at` field set).

- List all cards in `pending/` whose `meta.json` has `paused_at` set
- Clear `paused_at` from their meta and re-write
- Print: `▶  queued for dispatch: <id>`
- Note: actually dispatching is done by `bop dispatcher` — `bop resume` just
  clears the paused marker so the dispatcher picks them up naturally.

### 3. `bop retry-transient` command

Retry cards in `failed/` that failed due to transient causes, leaving permanent
failures alone.

Transient failure heuristics — check `meta.json` `failure_reason` field OR
last line of `logs/stderr`. Match strings (exact, case-insensitive):

```
"rate limit", "429", "503", "timeout", "connection refused",
"network", "ECONNRESET", "EX_TEMPFAIL", "name resolution failed",
"no route to host", "524"
```

OR: `meta.json` has `exit_code: 75`.

Keep these as a `const TRANSIENT_PATTERNS: &[&str]` slice in the source —
not an ad-hoc string match scattered through the code.

For each matching card:
- Clear `failure_reason`, increment `retry_count`
- `fs::rename` from `failed/` → `pending/`
- Print: `↩  retry: <id> (reason: network timeout)`

Non-transient cards are left in `failed/`:
`⚠  skipped: <id> (reason: build error — not transient)`

Add `--all` flag: `bop retry-transient --all` retries ALL failed cards
regardless of reason.

### 4. Add `exit_code` and `failure_reason` to `Meta`

If not already present, add these fields to the `Meta` struct in `bop-core/src/lib.rs`:

```rust
pub exit_code: Option<i32>,
pub failure_reason: Option<String>,
```

Dispatcher must write `exit_code` when a card transitions to `failed/`.
`failure_reason` should be the last non-empty line of `logs/stderr`, truncated
to 256 bytes.

### 5. Run `make check` — must pass.

### 6. Write `output/result.md` with sample output of all three commands.

## Acceptance

- `bop pause` stops running adapters; does NOT double-move exit-75 voluntary exits
- `bop pause` is safe under concurrent dispatch (checks card still in running/ before rename)
- `bop resume` clears `paused_at` markers
- `bop retry-transient` uses `TRANSIENT_PATTERNS` const, retries only matching failures
- `--all` flag retries everything
- `exit_code` field written to `meta.json` on failure
- `make check` passes
- `output/result.md` exists
