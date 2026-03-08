# Atomic write_meta + bop recover

## Context

`write_meta` in `crates/bop-core/src/lib.rs` uses `fs::write()` which truncates
the file then writes. A power loss or crash between truncation and completion leaves
`meta.json` empty or partially written — the card is unreadable. The fix is a
write-to-tmp-then-rename pattern (atomic on POSIX/APFS).

Similarly, `providers.json` writes in `crates/bop-cli/src/providers.rs` should
use the same pattern.

On startup, any card in `running/` with a dead PID (or no PID file) is an orphan
from a previous crash and should be moved back to `pending/`. Currently the orphan
reaper handles this during the dispatch loop, but it doesn't run on startup before
the first poll — a freshly booted machine can have stale `running/` cards for up
to `reap_ms` milliseconds.

## What to do

1. **Fix `write_meta`** (`crates/bop-core/src/lib.rs` line ~625):
   Replace:
   ```rust
   fs::write(meta_path(card_dir), bytes)?;
   ```
   With atomic write:
   ```rust
   let target = meta_path(card_dir);
   let tmp = target.with_extension("json.tmp");
   fs::write(&tmp, bytes)?;
   fs::rename(&tmp, &target)?;
   ```
   This is crash-safe: either the old `meta.json` or the new one exists, never
   a half-written file.

2. **Fix `providers.json` write** in `crates/bop-cli/src/providers.rs`:
   Find all `fs::write(providers_path, ...)` calls and apply the same
   tmp+rename pattern.

3. **Startup orphan scan** (`crates/bop-cli/src/dispatcher.rs`):
   Before entering the dispatch loop, call a `recover_orphans(cards_dir)` function
   that:
   - Reads every card in `running/`
   - Checks if `logs/pid` exists and if that PID is alive (`kill(pid, 0)`)
   - If PID is dead or missing: `fs::rename(running/card, pending/card)` and log a warning
   - If `meta.json` is missing or unparseable (corrupt): write a minimal recovery
     `meta.json` from the directory name (id only, stage="pending") then move to `pending/`

4. **`bop recover` command** (`crates/bop-cli/src/main.rs`):
   Expose `recover_orphans` as `bop recover` — runs the same scan on demand.
   Useful after an unexpected shutdown. Output:
   ```
   bop recover: scanning .cards/running/ for orphaned cards...
   ✓ recovered: my-task → pending/
   ✓ recovered: other-task (corrupt meta.json) → pending/
   2 cards recovered.
   ```
   If nothing to recover: `bop recover: nothing to recover.`

5. Run `make check` — must pass.

6. Write `output/result.md` with before/after and crash scenario analysis.

## Acceptance

- `write_meta` uses tmp+rename (no in-place truncate)
- `providers.json` writes use tmp+rename
- `bop recover` exists and scans `running/` for dead PIDs and corrupt meta
- Dispatcher calls `recover_orphans` before its first poll
- `make check` passes
- `output/result.md` exists
