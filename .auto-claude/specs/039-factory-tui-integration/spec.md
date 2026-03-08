# Spec 039 — factory status tab in `bop ui` TUI

## Overview

`bop ui` (spec 029) is a ratatui kanban TUI with a 3-pane layout: header, kanban columns,
footer. There is no visibility into the launchd factory services that drive the system.

This spec adds a **Factory tab** (or overlay panel) to `bop ui` that shows:
- Live service status for dispatcher, merge-gate, iconwatcher
- Recent log tail from `/tmp/bop-dispatcher.log` and `/tmp/bop-merge-gate.log`
- Keybinding to start/stop services

## UX design

Toggle with `F2` (or `Tab` to cycle tabs). The factory panel replaces the kanban columns
area when active, showing:

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  FACTORY SERVICES                              [F2] close
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  ● sh.bop.dispatcher    running   pid 24670   [S]top
  □ sh.bop.merge-gate    stopped               [R]un
  ● sh.bop.iconwatcher   active                [S]top
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  DISPATCHER LOG  /tmp/bop-dispatcher.log  (last 20 lines)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  [2026-03-08 22:00:01] scanning pending/ — 0 cards
  [2026-03-08 22:00:16] scanning pending/ — 1 card
  [2026-03-08 22:00:16] dispatching providers-ollama.card → codex
  ...
```

Keybindings in factory panel:
- `j/k` — move selection between services
- `s` — stop selected service (`bop factory stop`)
- `r` — start/restart selected service (`bop factory start`)
- `l` — toggle log pane between dispatcher/merge-gate logs
- `F2` or `Esc` — return to kanban view

## Implementation

### New file: `crates/bop-cli/src/ui/factory_tab.rs`

Implement `FactoryTabWidget` as a ratatui `Widget`. It:
1. Reads service status by calling `factory::factory_status_one` logic (or shelling out
   to `launchctl list sh.bop.dispatcher` via `std::process::Command`)
2. Reads the last N lines of `/tmp/bop-dispatcher.log` using `std::fs::File` + seek to end
3. Renders three zones: service rows, separator, log tail

Refresh on a 2-second tick (same as the existing header sparkline refresh).

### `crates/bop-cli/src/ui/app.rs`

Add `AppTab` enum variant `Factory`. Handle `F2` keypress to toggle. When tab is `Factory`,
render `FactoryTabWidget` in the main content area instead of the kanban columns.

### Log reading

Use `std::io::BufReader` + `seek(SeekFrom::End(-8192))` to efficiently tail logs without
reading the entire file. Parse last N newline-delimited lines.

## Acceptance Criteria

- [ ] `bop ui` compiles and starts without errors
- [ ] Pressing `F2` switches to factory panel showing 3 service rows
- [ ] Service status reflects actual launchctl state (not hardcoded)
- [ ] Log tail shows last 20 lines of `/tmp/bop-dispatcher.log`
- [ ] Pressing `Esc` or `F2` again returns to kanban view
- [ ] `cargo clippy -- -D warnings` clean
- [ ] `cargo test` passes

## Files to create/modify

- `crates/bop-cli/src/ui/factory_tab.rs` — new widget
- `crates/bop-cli/src/ui/app.rs` — add AppTab::Factory, F2 handler
- `crates/bop-cli/src/ui/mod.rs` — pub mod factory_tab
