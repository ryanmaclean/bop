# Sleep/wake awareness + network resilience

## Context

On a laptop running bop:
- Closing the lid suspends adapter processes mid-generation; on wake, their API
  connections are broken. They exit non-zero and cards land in `failed/` — but
  the failure is purely environmental (sleep interrupted the connection), not a
  code problem. These should auto-retry.
- No mechanism exists to pause the dispatcher gracefully before sleep or resume
  it after wake.
- Network handoffs (WiFi → cellular) drop active connections the same way.
- Exit code 75 (EX_TEMPFAIL) signals rate-limiting in bop's convention, but
  network errors typically produce exit codes 1 or 2 from the adapter.

## What to do

### 1. macOS sleep/wake notification subscriber (macOS only)

**Critical constraint**: `IORegisterForSystemPower` and the IOKit run loop MUST
run on a **dedicated OS thread** — not a tokio task. Tokio tasks can be starved
under load, and if `IOAllowPowerChange` is not called promptly after
`kIOMessageSystemWillSleep`, macOS will hang the sleep transition for the full
kernel timeout (~30s). An OS thread with a dedicated CFRunLoop is not subject to
tokio scheduling.

Implementation:

```rust
// In crates/bop-cli/src/dispatcher.rs, macOS only
#[cfg(target_os = "macos")]
fn spawn_power_watcher(tx: tokio::sync::watch::Sender<SleepState>) {
    std::thread::spawn(move || {
        // IORegisterForSystemPower + CFRunLoopRun here
        // On kIOMessageSystemWillSleep: tx.send(SleepState::Sleeping), IOAllowPowerChange
        // On kIOMessageSystemHasPoweredOn: tx.send(SleepState::Awake)
    });
}
```

Add a hard-deadline safety: if `pause_all_running` takes longer than 8 seconds,
call `IOAllowPowerChange` anyway and log a warning — never hold the sleep
transition indefinitely.

```toml
[target.'cfg(target_os = "macos")'.dependencies]
core-foundation = "0.9"
```

The dispatcher tokio loop reads the watch channel:
- On `Sleeping`: call `pause_all_running(cards_dir)`, then `IOAllowPowerChange`
- On `Awake`: log "system resumed, re-dispatching paused cards"

On Linux: feature-gate behind `cfg(target_os = "linux")` — subscribe to logind
D-Bus `PrepareForSleep` signal using `zbus`. If `zbus` is not already a
dependency, use a stub that only retries on exit-75 (sleep detection via retry
heuristics is sufficient for the Linux case).

On other platforms: no-op.

### 2. Network error exit code convention

Update adapters to distinguish network errors from logic errors.

In `adapters/claude.nu` and `adapters/codex.nu`: detect network-related error
patterns in stderr and exit 75 instead of 1. Exact match strings (as constants
at top of file):

```nushell
const NETWORK_ERROR_PATTERNS = [
  "connection refused", "network unreachable", "timeout",
  "ECONNRESET", "rate limit", "429", "503", "524",
  "name resolution failed", "no route to host"
]
```

Add a helper function `is_network_error [stderr: string] -> bool` at the top
of each adapter that checks these patterns case-insensitively.

### 3. Transient-failure auto-retry in dispatcher

In `dispatcher.rs`, when a card exits with code 75 OR with code 1/2 AND
`failure_reason` contains a network keyword:
- Move card back to `pending/` (not `failed/`)
- Apply a 30-second cooldown before re-dispatching (shorter than rate-limit 300s)
- Log: `[warn] transient failure on <id>, retrying in 30s`

**retry_count semantics**: `retry_count` is **per-card-per-failure-episode**,
reset to 0 on any successful dispatch. Cap at 3 per episode. After 3 transient
retries, move to `failed/` with `failure_reason` set to the last error. This
prevents infinite loops while allowing provider chain rotation to continue.

Do not conflate `retry_count` with the provider chain index — they are
independent: a card can exhaust its retry budget on provider A and still
fail-over to provider B via the chain.

### 4. Connectivity probe before dispatch

Before spawning an adapter for a cloud provider (claude, codex, opencode), do a
quick connectivity check on a **spawned tokio task** (not inline, to avoid
blocking the dispatch loop):

```rust
async fn provider_reachable(provider: &str) -> bool
```

Probe targets (read from providers.json if present, else these defaults):
- claude → `api.anthropic.com:443`, 2s timeout
- codex → `api.openai.com:443`, 2s timeout
- opencode → `api.openai.com:443`, 2s timeout
- ollama-local → always `true` (local, no probe)

Probe is **opt-out**: add `"probe": false` field to a provider entry in
`providers.json` to disable for air-gapped or offline deployments.

If unreachable: skip this card this poll cycle (leave in `pending/`), try
next card. Log: `[warn] <provider> unreachable, skipping <id>`.

### 5. Run `make check` — must pass.

### 6. Write `output/result.md` documenting the sleep/wake flow and network resilience.

## Acceptance

- IOKit power watcher runs on a dedicated OS thread (not tokio task) with 8s deadline
- IOAllowPowerChange always called, even if pause_all_running fails or times out
- Adapters exit 75 on network errors using defined constant pattern list
- Dispatcher auto-retries transient failures up to 3 per episode; retry_count resets on success
- Connectivity probe is async, opt-out via providers.json `"probe": false`
- ollama-local always bypasses the probe
- `make check` passes
- `output/result.md` exists
