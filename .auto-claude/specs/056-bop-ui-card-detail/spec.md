# Spec 056 — bop ui: card detail panel (TRIZ P6 Universality)

## Overview

P6 Universality: `bop ui` becomes the single surface that replaces `bop inspect`,
`bop diff`, `bop replay`, and `bop log` for interactive use. Press `Enter` on any
card in the kanban view to open a full-screen detail panel.

## Detail panel layout

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  team-arch/spec-041  •  done  •  codex  •  4m 51s  •  $0.18
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  [M]eta  [D]iff  [R]eplay  [O]utput  [L]og     Esc close
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  glyph: 🂡   token: ♠A   provider: codex   cost: $0.18
  tokens: 14,200   retries: 1   stage: implement
  worktree: .worktrees/team-arch-spec-041
  merge_commit: abc1234f
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  [content area — switches per tab key M/D/R/O/L]
```

### Tab: Meta (M)
Shows all `meta.json` fields as a two-column key/value table.

### Tab: Diff (D)
Runs `git diff <merge_commit>^..<merge_commit>` inline.
Syntax-highlighted via ratatui `Line` spans (green=add, red=del, default=ctx).
`j/k` to scroll; `G` to jump to end.

### Tab: Replay (R)
Reads `logs/events.jsonl` and renders the same timeline as `bop replay`.
Columns: timestamp, event, state, details.

### Tab: Output (O)
Shows `output/result.md` content with basic markdown rendering
(headers bold, bullets with `•`, code blocks in a contrasting style).

### Tab: Log (L)
Last 200 lines of `logs/stdout.log`. `j/k` scroll; `f` follow mode.

## Keybindings in detail panel
- `M/D/R/O/L` — switch tabs
- `j/k` — scroll content
- `G` — jump to bottom
- `f` — toggle follow mode (Log tab only)
- `Enter` / `Esc` — return to kanban

## Acceptance Criteria

- [ ] `Enter` on any card opens detail panel
- [ ] All 5 tabs render without panic on real card data
- [ ] Diff tab shows syntax-highlighted git diff
- [ ] Replay tab shows events.jsonl timeline
- [ ] `Esc` returns to kanban with cursor on same card
- [ ] Panel works for cards in all states (pending has no diff/replay — shows "not yet available")
- [ ] `cargo clippy -- -D warnings` clean
- [ ] `cargo test` passes

## Files

- `crates/bop-cli/src/ui/detail_panel.rs` — new widget
- `crates/bop-cli/src/ui/app.rs` — Enter handler, AppTab::Detail(card_id)
- `crates/bop-cli/src/ui/input.rs` — tab switching in detail mode
- `crates/bop-cli/src/ui/mod.rs` — pub mod detail_panel
