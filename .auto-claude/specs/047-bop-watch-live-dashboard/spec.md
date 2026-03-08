# Spec 047 — bop watch: unified live dashboard

## Overview

TRIZ P2 (Extraction) + P25 (Self-service): instead of three separate commands
(`bop log`, `bop stats`, `bop attach`), extract a single unified live dashboard
that the system uses to report its own state without the user knowing which
commands to invoke.

`bop watch` is a full-terminal live view that auto-refreshes every 500ms:

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  bop watch  •  3 running  •  1 done  •  $0.42 today
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  ● team-arch/spec-041    codex    running   3m 12s   ↳ [p]ane
  ● team-cli/spec-044     codex    running   1m 04s   ↳ [p]ane
  ● team-quality/spec-045 claude   running   0m 22s   ↳ [p]ane
  ✓ team-arch/spec-040    codex    done      4m 51s   $0.18
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  LOG  team-arch/spec-041  (last 8 lines)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  [14:22:01] Reading crates/bop-cli/src/ui/app.rs
  [14:22:03] Adding fuzzy filter state to App struct
  [14:22:08] Writing input handler for '/' key
  ...
```

## Implementation

Use `crossterm` (already in deps via ratatui) raw mode with a manual render
loop — NOT ratatui widgets, to keep it simple and avoid a second TUI state
machine conflicting with `bop ui`.

### Card row data
Read from `fs::read_dir` on `.cards/running/` and `.cards/done/` (last N).
Parse `meta.json` for id, provider, state, cost. Compute elapsed from
`meta.json.started_at` or `logs/pid` mtime.

### Log tail
Track the selected card (j/k to move). Read last 8 lines of
`logs/stdout.log` via `seek(SeekFrom::End(-4096))`. Re-read on each tick.

### Pane jump
Press `p` on a running card: shell out to
`zellij action switch-mode locked && zellij action focus-pane --name <card-id>`
if inside Zellij. Outside Zellij: print the card directory path.

### Keybindings
- `j/k` — move selection
- `p` — jump to Zellij pane
- `l` — cycle log view between running cards
- `q` / `Esc` — quit

## Acceptance Criteria

- [ ] `bop watch` shows all running cards with elapsed time
- [ ] Log tail updates live every 500ms
- [ ] `j/k` moves card selection, log pane follows
- [ ] `p` prints pane name (or focuses Zellij pane when inside session)
- [ ] `q` exits cleanly (restores terminal)
- [ ] Works with zero running cards (shows empty state message)
- [ ] `cargo clippy -- -D warnings` clean
- [ ] `cargo test` passes

## Files

- `crates/bop-cli/src/watch.rs` — new module
- `crates/bop-cli/src/main.rs` — wire `bop watch` subcommand
