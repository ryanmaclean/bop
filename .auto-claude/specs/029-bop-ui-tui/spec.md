# bop ui — full interactive TUI (kanban-first)

## Context

`bop status --watch` (spec 025) does in-place ANSI redraw — no alternate
screen. `bop ui` is the explicit opt-in full TUI: alternate screen, **kanban
columns as primary layout**, keyboard navigation, live log tail, fuzzy filter,
action menu. Think k9s for bop cards, with kanban-tui's column-first UX.

**Prior art synthesized:**
- **ratatui** (codex-rs, gitui, television): double-buffer diff rendering
  built-in — full frame every tick, only changed cells emitted. Do not
  implement a separate sprite-map diff.
- **codex-rs AppEvent pattern**: crossterm + FSEvents notify + timer ticks
  all become one `AppEvent` enum, processed by a single async event loop.
- **codex-rs newline-gated streaming**: buffer log bytes, emit only on `\n`
  to prevent partial ANSI sequence flicker in the live log tail.
- **television / nucleo**: blazing-fast fuzzy filter for `/` mode.
- **lazygit**: action popup via ratatui `Clear` + centered overlay; context-
  sensitive footer keybindings per mode.
- **htop**: provider meter bars in header; saturation coloring.
- **btop**: sparkline `▁▂▃▄▅▆▇█` for throughput history.
- **k9s**: three-zone law (header / body / footer); breadcrumb; `/` filter.
- **kanban-tui / TUI-Kanban**: horizontal columns as primary view; `h/l`
  between columns, `j/k` within; `Shift+H/L` move card to adjacent state;
  WIP limit indicator on column headers.
- **mc (Midnight Commander)**: `Ctrl+O` drops to subshell in card worktree,
  returning to TUI on exit; F-key secondary legend in footer.
- **yazi**: async task priority — render current column first, then prefetch
  neighbours. Multi-select `Tab` accumulates marked cards.

## Dependencies

```toml
[dependencies]
ratatui   = "0.29"          # MIT — double-buffer diff, widget system
crossterm = "0.28"          # MIT — alternate screen, events, mouse
nucleo    = "0.5"           # MIT — helix's fuzzy matcher, <1ms on 10k items
```

## Layout

```
╔═ bop · 14:23:07 · claude ████░░ ok · codex ░░░░░░ 4m · ▁▂▃▅▇ 8/hr ══════╗
╠═ · PENDING (3) ════╦═ ⚙ RUNNING (2/4) ═════╦═ ✓ DONE (8) ╦═ ✗ FAILED (1) ╣
║  · docs-update     ║▶ my-feature  claude 2m ║ ✓ perf-impr ║ ✗ broken-card ║
║  · refactor-api    ║  ████████░░ 67%  Ph 2  ║ ✓ network   ║               ║
║  · add-tests       ║  bugfix-auth codex  43s║ ✓ cleanup   ║               ║
║                    ║  ████░░░░░░ 22%  Ph 1  ║             ║               ║
║                    ║                        ║             ║               ║
╠════════════════════╩════════════════════════╩═════════════╩═══════════════╣
║ [h/l]col [j/k]card [↵]actions [Shift+H/L]move [/]filter [n]new [q]quit  ║
╚══════════════════════════════════════════════════════════════════════════╝
```

Three zones:
- **Header** (2 rows): live clock + provider meter bars + sparkline
- **Body** (terminal height - 4): horizontal kanban columns
- **Footer** (1 row): context-sensitive keybindings

## Implementation

### Entry point

`crates/bop-cli/src/ui.rs` — `pub async fn run_ui(cards_dir: PathBuf) -> anyhow::Result<()>`

Called via `bop ui` subcommand. Enters alternate screen, installs raw mode,
exits cleanly on `q` or Ctrl-C. RAII cleanup via a `TerminalGuard` struct that
calls `crossterm::terminal::disable_raw_mode` + `LeaveAlternateScreen` on drop.

### AppEvent

```rust
enum AppEvent {
    Key(KeyEvent),
    Resize(u16, u16),
    Tick,                          // 250ms timer
    Cards(Vec<CardView>),          // notify watcher sent new card list
    LogLine(String),               // new line from log tail reader
}
```

Background tasks send to `tokio::sync::mpsc::unbounded_channel::<AppEvent>()`.
UI loop: `tokio::select!` on the channel + crossterm event reader. Never block
the render thread.

### App state

```rust
struct App {
    columns: Vec<KanbanColumn>,    // ordered: pending, running, done, failed, merged
    col_focus: usize,              // index into columns
    filter: Option<String>,        // active nucleo query
    mode: Mode,
    log_buf: VecDeque<String>,     // last 200 lines from selected card's logs
    throughput: VecDeque<u8>,      // 8 samples for sparkline
}

struct KanbanColumn {
    state: CardState,              // pending | running | done | failed | merged
    cards: Vec<CardView>,
    list_state: ListState,
    wip_limit: Option<usize>,      // from providers.json max_concurrent for running
    collapsed: bool,               // true when card count == 0
}

enum Mode {
    Normal,
    Filter,       // / active
    ActionPopup,  // ↵ menu open
    Detail,       // card detail overlay
    LogTail,      // full-height log tail overlay
    NewCard,      // n inline creation prompt
    Subshell,     // Ctrl+O — TUI suspended, shell active
}
```

### Column layout

`Layout::horizontal` divides body width equally among non-collapsed columns.
Collapsed columns (0 cards) render as a 3-char narrow divider showing only the
state glyph (`·`, `⚙`, `✓`, `✗`, `~`). Collapsed state toggled automatically
as card counts change.

Each column is a ratatui `List` of card rows inside a `Block` with title:
- Normal: `╔═ · PENDING (3) ════╗` (dim border)
- Focused: amber (`Color::Yellow`) border
- Running column with WIP limit: `⚙ RUNNING (2/4)` — turns amber at ≥75%,
  red at 100% (column header + border, atop saturation coloring)

Card row in column (two lines for running cards, one for others):
```
▶ my-feature    claude    2m 14s
  ████████░░░░ 67%   Phase 2: Network  ◑
```
Running cards show progress; pending/done/failed show one line.

### Navigation

| Key | Action |
|---|---|
| `h` / `l` | Move focus to previous / next column |
| `j` / `k` | Select card up / down within column |
| `H` / `L` (Shift) | Move selected card to adjacent state (left/right) |
| `↵` | Open action popup |
| `d` | Open detail overlay for selected card |
| `F3` / `L` | Open log tail overlay |
| `/` | Activate filter mode |
| `n` | New card inline prompt |
| `!` | Subshell in card worktree (vim `:!` convention; `Ctrl+O` is taken by Zellij) |
| `Tab` | Multi-select: toggle mark on current card |
| `q` / `Ctrl+C` | Quit |

**Shift+H/L card movement**: calls `fs::rename` directly to move card directory
between state dirs (pending ↔ running not allowed from TUI — only done/failed
↔ pending are safe manual moves). Fires a `Cards` event to refresh immediately.

### Detail overlay (lazygit pattern)

Press `d` on selected card → detail panel overlaid on right 50% of body:

```
╭─ my-feature ─────────────────────╮
│ claude · 2m 14s   ████████░░ 67% │
│                                   │
│ Phase 1  Crash Safety  ●●●●●  5/5 │
│ Phase 2  Network       ●○○○   1/4 │
│   ◉ Write result.md  in_progress  │
│   ○ Run make check   pending      │
│                                   │
│ [F3]logs [p]pause [r]retry [Esc]  │
╰───────────────────────────────────╯
```

Uses ratatui `Clear` widget on a centered `Rect` + bordered `Block`. Renders
`CardView` + AC plan progress (spec 027) phase/subtask tree. Scrollable with
`j/k`.

### Action popup (lazygit pattern)

Press `↵` on selected card:

```
╭─ Actions: my-feature ──────────────╮
│ [F3] View logs                     │
│ [z]  Open Zellij pane              │
│ [p]  Pause adapter                 │
│ [r]  Retry                         │
│ [k]  Kill (SIGTERM)                │
│ [i]  Inspect AC plan               │
│ [Esc] Cancel                       │
╰────────────────────────────────────╯
```

Available actions filtered by card state (can't pause a done card, can't retry
a running card).

### Log tail overlay

Full-height overlay replacing the body. Triggered by `F3` or `L`:

```
╔═ logs: my-feature ════════════════════════════════════════════════╗
║ > subtask-1-5: bop recover done                                   ║
║ > Running cargo test -p bop-core                                  ║
║ > test result: ok. 447 passed; 0 failed                           ║
╠═══════════════════════════════════════════════════════════════════╣
║ [↑↓]scroll [f]follow [c]clear [Esc]close                         ║
╚═══════════════════════════════════════════════════════════════════╝
```

**Newline-gated streaming** (codex-rs pattern):
- Open `logs/stdout` and `logs/stderr` of selected card with `O_NONBLOCK`
- Read bytes into a line buffer; only emit complete lines (ending in `\n`)
- Never render a partial line — prevents flickering mid-ANSI-sequence
- On card selection change: close old handles, open new ones, seek to EOF
- Last 200 lines in `VecDeque<String>`. `f` toggles follow-mode (auto-scroll).

### Filter mode (k9s pattern)

`/` activates filter:
- Footer replaced with: `Filter: my-feat█  [Esc]clear  [↵]confirm`
- All columns filtered in real time via nucleo matcher (card id + title)
- Empty columns after filter collapse to narrow dividers
- Matched characters highlighted in card titles (`Color::Yellow + BOLD`)
- Esc clears filter, restores full columns + normal footer

### New card inline (n)

`n` opens a minimal prompt in the footer area:
```
New card id: feat-█
```
On `↵`: calls `bop new default <id>` via `Command::new("bop")`, refreshes.
On `Esc`: cancel.

### Subshell (`!` key)

`!` in normal mode (vim `:!` convention) — `Ctrl+O` is Zellij's Session mode
and is intercepted before reaching the app.

Suspend TUI (`LeaveAlternateScreen`, `disable_raw_mode`), spawn `$SHELL` in
the selected card's worktree directory, wait for it to exit, re-enter alternate
screen + raw mode, redraw. Drops user back to exactly where they were. Works
only for cards with a worktree path set in meta. Does NOT open a new Zellij
pane — operates within the existing pane's alternate screen stack.

### Multi-select (yazi pattern)

`Tab` toggles a `marked: bool` on the hovered card. Marked cards display a
`■` prefix. Bulk actions (kill, retry, move) apply to all marked cards.
`Esc` clears all marks.

### Header

```
bop · 14:23:07 · claude ████████░░ ok · codex ░░░░░░░░░░ 4m22s · ▁▂▃▅▇ 8/hr
```

Provider meters (htop style): one `█░` bar per provider from `providers.json`,
colored green (available), amber (busy), red (rate-limited). Sparkline (btop):
last 8 tick samples of cards-completed/hour as `▁▂▃▄▅▆▇█`.

Demoscene live ticker: if terminal width > 120, append scrolling event stream
`  ✓ perf-improvements done 4m ago · ⚙ bugfix-auth running ·  ` advancing
one character per tick. Circular string index into the event log.

### Context-sensitive footer

| Mode | Footer shown |
|---|---|
| Normal | `[h/l]col [j/k]card [↵]actions [Shift+H/L]move [/]filter [n]new [q]quit` |
| Detail | `[j/k]scroll [F3]logs [p]pause [r]retry [Esc]close` |
| LogTail | `[↑↓]scroll [f]follow [c]clear [Esc]close` |
| Filter | `Filter: {query}█  [Esc]clear [↵]confirm` |
| ActionPopup | `[↑↓]select [↵]run [Esc]cancel` |
| NewCard | `New card id: {input}█  [↵]create [Esc]cancel` |

Secondary F-key legend (mc style) shown when terminal height > 30:
```
F3=logs  F4=inspect  F5=pause  F8=kill  F10=quit
```

### Resize handling

`crossterm::event::Event::Resize(w, h)` → `AppEvent::Resize(w, h)` →
re-compute column widths and all `Layout` constraints next render. Ratatui
handles the rest — no manual cursor cleanup needed.

### Startup behaviour

1. Detect `$ZELLIJ` — if set, open UI in current pane
2. Enter alternate screen (`\x1b[?1049h`), enable raw mode
3. Spawn background tasks: notify watcher, 250ms tick timer
4. Read `providers.json` for WIP limits
5. Initial render: all columns with current card counts
6. On exit: `TerminalGuard` drop cleans up — raw mode off, alternate screen
   exit, cursor visible — panics also clean up

### Snapshot tests

Use `insta` crate for regression testing:

```rust
#[test]
fn test_kanban_columns_render() {
    let cols = vec![/* test columns */];
    let rendered = render_to_string(&KanbanWidget { cols });
    insta::assert_snapshot!(rendered);
}
```

Run with `INSTA_UPDATE=new cargo test` to update snapshots.

## Acceptance

- `bop ui` enters alternate screen with horizontal kanban columns as primary layout
- `h/l` navigates between columns; `j/k` navigates within a column
- `Shift+H/L` moves card to adjacent state (fs::rename, safe moves only)
- Empty columns collapse to narrow glyph-only dividers
- Running column header shows WIP limit `⚙ RUNNING (2/4)`, turns red at limit
- Ratatui double-buffer diff used — no manual sprite-map diffing
- Log tail uses newline-gated streaming (no partial-line flicker)
- `/` filter uses nucleo, highlights matched chars, collapses empty columns
- `↵` opens action popup filtered by card state
- `d` opens detail overlay with AC plan progress (spec 027)
- `Ctrl+O` drops to subshell in card worktree, returns to TUI on exit
- `Tab` multi-select marks cards; bulk actions apply to marked set
- `n` creates new card via `bop new default <id>`
- Provider meter bars color-coded: green/amber/red by availability
- Sparkline updates each tick with cards/hour sample
- Demoscene event ticker at width ≥ 120
- Context-sensitive footer changes per mode; F-key bar at height > 30
- Resize handled via crossterm resize event, no corruption
- Snapshot tests with `insta` for kanban layout, detail overlay, header
- `make check` passes
- `output/result.md` with ASCII screenshots of: normal kanban view, detail
  overlay, log tail overlay, filter mode, action popup
