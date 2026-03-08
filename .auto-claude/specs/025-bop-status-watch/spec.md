# bop status --watch (live updating terminal view)

## Context

`bop gantt` is rich but static. `bop list` is plain. On a laptop you want a
lightweight live view that shows what's running, what's pending, and what just
finished — updating automatically without redrawing the whole screen. Think
`watch -n1 bop list` but better: in-place update using ANSI cursor control,
no full-screen TUI overhead, works over SSH and in Zellij panes.

## What to do

### 1. `bop status --watch` flag

Add `--watch` / `-w` flag to the existing `bop status` (or `bop list`) command.

When `--watch` is set:
- Print the current status, then enter a loop
- Use `notify` watcher (already added in spec 020) to watch `.cards/` for any
  changes (file create/modify/delete in any state dir)
- On change: move cursor to top of the last output (`\x1b[{N}A` where N = lines
  printed), redraw in-place
- Show a live clock in the header: `bop · 14:23:07 · watching .cards/`
- Ctrl-C exits cleanly (restore cursor, drop watcher to avoid fd leak)

### 2. Live status layout

```
bop · 14:23:07 · watching .cards/
─────────────────────────────────────────
● running  (2)
  ⚙ my-feature          claude    2m 14s  ████████░░░░ 67%
  ⚙ bugfix-auth         codex     0m 43s  ███░░░░░░░░░ 22%

● pending  (3)
  · docs-update
  · refactor-api
  · add-tests

● done  (8) · failed (1) · merged (12)
  ✓ perf-improvements   done      4m 02s
  ✗ broken-card         failed    1m 15s  exit 1

─────────────────────────────────────────
14 total · success rate 92% · avg 3m 44s
```

Rules:
- Progress bar for running cards: read `meta.json` `progress` field (0-100) if
  set, otherwise use elapsed time as a fraction of the adapter timeout
- Elapsed time: `started` field in `meta.json` stages
- Colors: running=amber, done=green, failed=red, pending=blue, merged=dim

### 3. Terminal-width-aware redraw

**Re-query terminal width on every redraw event** — do not cache it. Terminal
resize between events will corrupt the cursor-up line count if N was computed
using a stale width. Use `terminal_size()` or equivalent before each render.

Separator lines (`─────`) expand to current terminal width minus 2.
Truncate card names and provider names to fit the available columns.

### 4. Minimal redraw

Only redraw if the state actually changed (compare card counts per state before
and after the notify event). Avoids flicker on rapid changes.

### 5. `bop status` (without `--watch`) improvements

Ensure `bop status` without `--watch` also shows the summary line and uses
colors (may already be done in spec 018 — verify and integrate if not).

### 6. Run `make check` — must pass.

### 7. Write `output/result.md` with an ASCII screenshot of the watch output.

## Acceptance

- `bop status --watch` / `bop list --watch` updates in-place on `.cards/` changes
- Terminal width re-queried on every redraw (not cached)
- Shows running cards with elapsed time and progress bar
- Ctrl-C exits cleanly, drops the notify watcher (no fd leak)
- No full-screen TUI — works in any terminal width
- `make check` passes
- `output/result.md` with sample output exists
