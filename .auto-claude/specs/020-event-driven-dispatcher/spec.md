# Event-driven dispatcher: replace polling with file watching

## Context

`dispatcher.rs` and `merge_gate.rs` poll `pending/` and `done/` directories with
`tokio::time::sleep(poll_ms)` — typically every 500ms. This wastes CPU, adds latency,
and conflicts with the `bop factory` WatchPaths design (which is already event-driven
at the launchd level). The `notify` crate provides cross-platform FSEvents/inotify
watching with near-zero overhead.

## What to do

1. Add `notify = { version = "6", features = ["macos_kqueue"] }` to
   `crates/bop-cli/Cargo.toml` (use `macos_fsevent` on Apple Silicon).
   Also add `notify-debouncer-mini = "0.4"` for debouncing rapid writes.

2. **Dispatcher** (`dispatcher.rs`): replace the `loop { sleep(poll_ms) }` poll
   with a `notify::recommended_watcher` watching `.cards/pending/`. On `Create`
   events, wake the dispatcher tokio task via a `tokio::sync::Notify` or channel.
   Keep a 100ms debounce to coalesce rapid card drops.

3. **Merge gate** (`merge_gate.rs`): same pattern watching `.cards/done/`.

4. **Keep `--poll-ms` flag** as a fallback (set `notify` watcher; if it fails,
   fall back to polling and log a warning). Keeps CI/containers working.

5. **Perf bonus**: set release profile `opt-level = "3"` (currently `"s"`) in
   root `Cargo.toml`. Also enable `lto = "thin"` for faster startup.

6. Run `make check` — must pass.

7. Write `output/result.md` with before/after: avg latency from card drop to
   dispatch start (should drop from ~500ms to <50ms).

## Acceptance

- Dispatcher wakes within 100ms of a card appearing in `pending/`
- Merge gate wakes within 100ms of a card appearing in `done/`
- `--poll-ms` fallback still works when notify fails
- Release build uses `opt-level = "3"`
- `make check` passes
- `output/result.md` exists
