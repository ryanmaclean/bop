# bop pause / resume / retry-transient — Sample Command Outputs

This document demonstrates the output of the three new commands added to bop.

## 1. `bop pause` — Stop running adapters

### Example: Pausing running cards

```bash
$ bop pause ""
⏸  paused: test-api-timeout (adapter PID 12345 stopped)
⏸  paused: fix-networking-bug (adapter PID 12378 stopped)
⏸  paused: add-auth-feature (adapter PID 12401 stopped)
```

When cards are paused, they are moved from `running/` back to `pending/` with a `paused_at` timestamp in their `meta.json`.

### Example: No running cards

```bash
$ bop pause ""
bop pause: nothing running.
```

### Example: Race condition handling

If a card exits voluntarily with code 75 (rate-limit) during the pause operation, the dispatcher handles the move and pause skips it:

```bash
$ bop pause ""
⏸  paused: test-timeout (adapter PID 12345 stopped)
⚠  warning: card feature-x already moved by dispatcher (skipped)
⏸  paused: fix-bug-y (adapter PID 12401 stopped)
```

---

## 2. `bop resume` — Re-queue paused cards

### Example: Resuming paused cards

```bash
$ bop resume ""
▶  queued for dispatch: test-api-timeout
▶  queued for dispatch: fix-networking-bug
▶  queued for dispatch: add-auth-feature
```

The `paused_at` field is cleared from `meta.json`, allowing the dispatcher to pick up these cards again.

### Example: No paused cards

```bash
$ bop resume ""
bop resume: no paused cards in pending/.
```

---

## 3. `bop retry-transient` — Retry transient failures

### Example: Retrying specific card

```bash
$ bop retry-transient api-integration-test
↩  retry: api-integration-test (reason: rate limit exceeded)
```

### Example: Scanning all failed cards (default mode)

```bash
$ bop retry-transient ""
↩  retry: network-fetch-test (reason: 503 Service Unavailable)
↩  retry: external-api-call (reason: connection refused)
⚠  skipped: build-failed-card (reason: compilation error — not transient)
↩  retry: dns-lookup-test (reason: name resolution failed)
⚠  skipped: test-assertion-fail (reason: assertion failed: expected 2, got 3 — not transient)
```

Cards identified as transient failures are moved from `failed/` back to `pending/` with their `retry_count` incremented and `failure_reason` cleared.

### Example: Retry all failed cards (with `--all` flag)

```bash
$ bop retry-transient "" --all
↩  retry: network-fetch-test (retrying despite reason: service unavailable)
↩  retry: external-api-call (retrying despite reason: connection refused)
↩  retry: build-failed-card (retrying despite reason: compilation error)
↩  retry: dns-lookup-test (retrying despite reason: name resolution failed)
↩  retry: test-assertion-fail (retrying despite reason: assertion failed)
```

The `--all` flag bypasses the transient check and retries every card in `failed/` regardless of the failure reason.

### Example: No failed cards

```bash
$ bop retry-transient ""
bop retry-transient: no failed cards found.
```

---

## Transient Failure Patterns

Cards are considered transient failures if their `meta.json` contains:
- `exit_code: 75` (rate-limit exit code), OR
- `failure_reason` or last line of `logs/stderr` matches any of:
  - `rate limit`
  - `429`
  - `503`
  - `timeout`
  - `connection refused`
  - `network`
  - `ECONNRESET`
  - `EX_TEMPFAIL`
  - `name resolution failed`
  - `no route to host`
  - `524`

Pattern matching is case-insensitive.

---

## Meta Field Updates

### `paused_at` field
Set by `bop pause` when a card is stopped and returned to `pending/`:
```json
{
  "id": "test-api-timeout",
  "stage": "pending",
  "paused_at": "2026-03-07T08:45:23.456789Z",
  ...
}
```

Cleared by `bop resume` when the card is queued for re-dispatch.

### `exit_code` and `failure_reason` fields
Written by the dispatcher when a card moves to `failed/`:
```json
{
  "id": "network-fetch-test",
  "stage": "failed",
  "exit_code": 1,
  "failure_reason": "503 Service Unavailable",
  "retry_count": 0,
  ...
}
```

Both fields are cleared by `bop retry-transient` when the card is retried, and `retry_count` is incremented.
