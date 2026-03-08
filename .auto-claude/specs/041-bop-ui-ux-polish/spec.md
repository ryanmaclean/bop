# Spec 041 — bop ui: UX polish pass

## Overview

`bop ui` (spec 029) has the 3-pane ratatui layout working but is missing the
high-value UX interactions that were spec'd in the MEMORY.md UX research:
fuzzy filter, Tab multi-select, Ctrl+O subshell, Shift+H/L card moves, and
WIP limit indicators.

## Features to implement

### 1. Fuzzy filter (`/`)

Pressing `/` opens a filter bar at the bottom of the kanban area.
Typed text filters visible cards by `id` substring (case-insensitive).
`Esc` clears filter and restores all cards. `Enter` keeps filter active
and returns focus to card navigation.

### 2. `Shift+H` / `Shift+L` — move card between states

When a card is selected, `Shift+H` moves it left one column (e.g. `done` →
`running`) and `Shift+L` moves it right (`pending` → `running`).
Implementation: `fs::rename` on the card directory — the same atomic move
the dispatcher uses. Only allow moves that are valid state transitions
(see `CardState` in bop-core). Show an error toast if invalid.

### 3. `Ctrl+O` — open card subshell

Opens a new Zellij pane in the card's `worktree/` directory (or card dir
if no worktree). Uses `zellij run --name <card-id> -- $SHELL` with cwd set
to the card directory. Falls back to spawning `$SHELL` in a child process
if not inside Zellij.

### 4. WIP limit on running column

Read `max_workers` from `.cards/.bop/config.json` (default: 3).
When `running/` count == max_workers, render the running column header in
yellow. When count > max_workers, render in red (overloaded).

### 5. Empty column collapse

Columns with zero cards collapse to a 3-char glyph divider:
`│ ∅ │` (or similar). This frees horizontal space for columns with cards.
The `h/l` navigation skips collapsed columns.

## Acceptance Criteria

- [ ] Pressing `/` shows a filter input bar; typing filters card list by id
- [ ] `Esc` clears filter
- [ ] `Shift+H` / `Shift+L` perform `fs::rename` state transitions
- [ ] Invalid moves show a brief error message (2-second toast)
- [ ] `Ctrl+O` opens a shell pane in the card directory
- [ ] Running column header turns yellow at max_workers, red above it
- [ ] Zero-card columns collapse to narrow glyph divider
- [ ] `cargo clippy -- -D warnings` clean
- [ ] `cargo test` passes

## Files to modify

- `crates/bop-cli/src/ui/app.rs` — add filter state, WIP limit check
- `crates/bop-cli/src/ui/input.rs` — Shift+H/L, Ctrl+O, `/` key handlers
- `crates/bop-cli/src/ui/widgets/kanban.rs` — empty column collapse, WIP colors
- `crates/bop-cli/src/ui/widgets/footer.rs` — show filter bar when active
