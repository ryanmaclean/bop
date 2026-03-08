# CLI UX: color-coded states, summary stats, better feedback

## Context

`bop list` shows card states as plain text. `bop status` has no aggregate stats.
Error messages don't suggest next steps. The Gantt chart already has a color scheme
(pending=blue, running=amber, done=green, failed=red) — the list view should match.

## What to do

1. **Color-code `bop list`** (`crates/bop-cli/src/list.rs`):
   - Reuse the ANSI color constants already defined in `gantt.rs`
   - pending → blue, running → amber/yellow, done → green, failed → red, merged → dim green
   - Add visual separators between state groups (a thin `─────` line)
   - Show card count per group: `● running (2)` in the group header

2. **Summary stats at bottom of `bop list`** and `bop status`:
   - `14 total · 2 running · 8 done · 1 failed · avg 4m32s`
   - Success rate: `success rate 93%`
   - Read durations from `meta.json` `stages.implement.duration_s` field

3. **Success messages with next-step hints**:
   - After `bop new`: `✓ Card created → bop dispatch <id> to run it`
   - After `bop clean`: `✓ Cleaned N cards → bop list to verify`
   - After `bop kill`: `✓ Killed <id> → it's back in pending/`

4. **Improve error messages**:
   - Card not found: `Card 'xyz' not found. Try: bop list`
   - No cards pending: `Nothing pending. Try: bop new <template> <id>`
   - Adapter not found: `Adapter 'foo.nu' not found. Available: bop list-adapters`

5. Run `make check` — must pass.

6. Write `output/result.md` with before/after examples.

## Acceptance

- `bop list` output is color-coded by state
- `bop list` shows summary stats line at bottom
- State groups have visual separators
- `make check` passes
- `output/result.md` exists
