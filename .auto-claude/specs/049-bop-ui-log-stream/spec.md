# Spec 049 — bop ui: integrated log stream pane

## Overview

TRIZ P2 (Extraction): extract the log stream into the existing TUI rather than
a separate terminal. `bop ui` already has a 3-zone layout (header, kanban,
footer). This spec adds a toggleable log pane that replaces the kanban zone
when active.

## UX

Press `L` (uppercase) in `bop ui` to toggle the log stream pane.

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  LOG  team-arch/spec-041  •  running  3m 12s     [L] close
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  [14:22:01] Reading crates/bop-cli/src/ui/app.rs
  [14:22:03] Adding fuzzy filter state to App struct
  [14:22:08] Writing input handler for '/' key
  [14:22:15] cargo build...
  [14:22:31]   Compiling bop v0.1.0
  [14:22:44]   Finished dev profile
  [14:22:44] Tests pass
  ...
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

The log pane shows the currently selected card's `logs/stdout.log`, tailing
the last N lines that fit the terminal height. Auto-scrolls to bottom unless
the user has pressed `k` to scroll up (then shows a "↓ following paused"
indicator).

`j/k` in log pane: scroll through log history.
`f` in log pane: resume following (jump to bottom, re-enable auto-scroll).
`L` or `Esc`: return to kanban view.
`Tab`: cycle to next running card's log.

## Implementation

Add `AppTab::Log(card_id)` variant to the existing `AppTab` enum in `app.rs`.
Read log file on each tick using the same `BufReader + seek(SeekFrom::End)`
pattern from `factory_tab.rs`. Store scroll offset as `usize` in `App`.

## Acceptance Criteria

- [ ] `L` toggles log pane for the currently selected card
- [ ] Log tail auto-scrolls to bottom on new lines
- [ ] `k` pauses auto-scroll; `f` resumes
- [ ] `Tab` cycles between running cards' logs
- [ ] `Esc`/`L` returns to kanban view
- [ ] Log pane updates every 500ms tick (same as header sparkline)
- [ ] Empty / missing log file shows "no output yet" message
- [ ] `cargo clippy -- -D warnings` clean
- [ ] `cargo test` passes

## Files

- `crates/bop-cli/src/ui/log_pane.rs` — new widget
- `crates/bop-cli/src/ui/app.rs` — add AppTab::Log, L key handler
- `crates/bop-cli/src/ui/input.rs` — j/k/f/Tab in log pane context
- `crates/bop-cli/src/ui/mod.rs` — pub mod log_pane
