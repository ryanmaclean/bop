# Terminal card renderer with progressive degradation

## Context

`bop status --watch` and `bop list` show cards as flat text rows. The goal is
a rich, demoscene-inspired ANSI card layout that degrades gracefully across
terminal capabilities — from a dumb SSH pipe to a full truecolor Zellij pane.
vibe-kanban (already integrated via `vibekanban/`) showed that column-based
card layouts are more scannable than flat lists; bop's CLI should match that
quality natively without requiring a separate TUI process.

## Capability detection

Add `crates/bop-cli/src/termcaps.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TermLevel {
    Dumb,       // TERM=dumb or unset; no color, ASCII only
    Basic,      // 8-color ANSI; ASCII borders
    Extended,   // 16-color; box-drawing (single line); unicode blocks
    Full,       // 256-color; block shading ░▒▓; full BMP unicode
    TrueColor,  // 24-bit RGB; double-line box ╔═╗; playing-card tokens
}

pub struct TermCaps {
    pub level: TermLevel,
    pub width: u16,
    pub two_column: bool,   // width >= 100
}

impl TermCaps {
    pub fn detect() -> Self {
        let level = Self::detect_level();
        let width = terminal_size().map(|(w, _)| w.0).unwrap_or(80);
        TermCaps { level, width, two_column: width >= 100 }
    }

    fn detect_level() -> TermLevel {
        // Zellij always supports Full
        if std::env::var("ZELLIJ").is_ok() {
            return TermLevel::Full;
        }
        let colorterm = std::env::var("COLORTERM").unwrap_or_default();
        if colorterm == "truecolor" || colorterm == "24bit" {
            return TermLevel::TrueColor;
        }
        let term = std::env::var("TERM").unwrap_or_default();
        if term.contains("256color") {
            return TermLevel::Full;
        }
        if term.starts_with("xterm") || term.starts_with("screen")
            || term.starts_with("tmux") || term.starts_with("rxvt") {
            return TermLevel::Extended;
        }
        if term == "dumb" || term.is_empty() {
            return TermLevel::Dumb;
        }
        TermLevel::Basic
    }
}
```

`terminal_size` crate (already in Rust ecosystem, MIT, ~2KB). Re-query on
every render — never cache (supports `bop status --watch` resize handling,
spec 025 requirement).

## Card rendering — four layouts

All layouts use the same data from `Meta` + optional `implementation_plan.json`
(spec 027). The renderer selects the layout from `TermCaps.level`.

---

### Level 0 — Dumb (no color, ASCII only)

```
=== RUNNING (2) ===
[>] my-feature     claude   2m14s  67%
[>] bugfix-auth    codex    0m43s  22%

--- PENDING (3) ---
[ ] docs-update
[ ] refactor-api
[ ] add-tests

--- DONE / FAILED ---
[x] perf-improvements   4m02s
[!] broken-card         exit 1

14 total | 92% success | avg 3m44s
```

---

### Level 1 — Basic (8-color, ASCII borders)

```
+-- RUNNING (2) ------------------------------------------+
| [>] my-feature     claude   2m14s  [========  ] 67%     |
| [>] bugfix-auth    codex    0m43s  [===       ] 22%     |
+---------------------------------------------------------+
```

---

### Level 2 — Extended (16-color, single-line box drawing)

Left-accent stripe `▌` (U+258C) per card row, state color:

```
┌─ ⚙ RUNNING ──────────────────────────────────────────┐
│▌ my-feature          claude      2m 14s               │
│  ████████████░░░░ 67%   Phase 2: Network  ◑           │
│                                                        │
│▌ bugfix-auth         codex       0m 43s               │
│  ████░░░░░░░░░░ 22%   Phase 1: Core  ◔                │
└────────────────────────────────────────────────────────┘

┌─ · PENDING ───────┐  ┌─ ✓ DONE · ✗ FAILED ───────────┐
│  · docs-update    │  │  ✓ perf-improvements  4m 02s   │
│  · refactor-api   │  │  ✗ broken-card  exit 1         │
│  · add-tests      │  └────────────────────────────────┘
└───────────────────┘
```

---

### Level 3 — Full (256-color, demoscene block headers)

Section headers use the dithered fade pattern — classic BBS/ANSI art style:

```
░▒▓ ⚙ RUNNING (2) ▓▒░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░

▌ ⚀ my-feature          claude      2m 14s
▌   ████████████░░░░ 67%    Phase 2: Network layer   ◑

▌ ⚁ bugfix-auth          codex       0m 43s
▌   ████░░░░░░░░░░ 22%    Phase 1: Core impl         ◔

░▒▓ · PENDING ▓▒░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░

  · docs-update
  · refactor-api
  · add-tests

░▒▓ ✓ DONE · ✗ FAILED ▓▒░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░

  ✓ perf-improvements                              4m 02s
  ✗ broken-card                 failed              exit 1

░▒▓▓▒░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
  14 total · success rate 92% · avg 3m 44s
```

Section header construction:
```
░▒▓ {LABEL} ▓▒░{░ × (width - label_len - 6)}
```

Left-accent `▌` is printed in state color (amber=running, green=done,
red=failed, blue=pending, dim=merged). The dice glyph `⚀⚁⚂⚃⚄⚅` encodes
card priority (P1–P6), BMP-safe.

Progress bar: 16-cell `████░░░░` (filled = `\u{2588}`, empty = `\u{2591}`).

---

### Level 4 — TrueColor (double-line box, column layout)

At `width >= 100`, two-column layout. Below 100, single-column with double box:

```
╔═ ⚙ RUNNING (2) ═══════════════════╗  ╔═ · PENDING (3) ════════════════════╗
║ ⚀ my-feature    claude   2m 14s  ║  ║  · docs-update                     ║
║   ████████░░ 67%  Ph2  ◑         ║  ║  · refactor-api                    ║
╠══════════════════════════════════╣  ║  · add-tests                       ║
║ ⚁ bugfix-auth   codex    0m 43s  ║  ╚════════════════════════════════════╝
║   ████░░░░░░ 22%  Ph1  ◔         ║
╚══════════════════════════════════╝  ╔═ ✓ DONE · ✗ FAILED ════════════════╗
                                      ║ ✓ perf-improvements     4m 02s     ║
                                      ║ ✗ broken-card   failed   exit 1    ║
                                      ╚════════════════════════════════════╝
```

Double-line box chars (all BMP):
- `╔ ═ ╗ ║ ╠ ╣ ╚ ╝` — corners, horizontals, verticals, T-joints
- `╭ ─ ╮ │ ╰ ╯` — rounded single-line for card separators within a box

Progress bar at TrueColor uses RGB gradient fill — amber→green as percent
rises — via `\x1b[38;2;R;G;Bm█\x1b[0m` per cell. Falls back to fixed amber
block at Full/Extended.

---

## Implementation structure

```
crates/bop-cli/src/
  termcaps.rs      — TermLevel detection, TermCaps struct
  render/
    mod.rs         — CardRenderer trait, dispatch by TermLevel
    dumb.rs        — Level 0
    basic.rs       — Level 1
    extended.rs    — Level 2
    full.rs        — Level 3 (demoscene headers)
    truecolor.rs   — Level 4 (double box, RGB gradient)
```

`CardRenderer` trait:

```rust
pub trait CardRenderer {
    fn render_section_header(&self, label: &str, count: usize) -> String;
    fn render_card_row(&self, card: &CardView) -> String;
    fn render_progress(&self, pct: u8, phase: Option<&str>, phase_frac: f32) -> String;
    fn render_footer(&self, stats: &Stats) -> String;
}
```

`CardView` is a flat view struct built from `Meta` + optional AC plan data
(spec 027). The renderer does not read files — the caller assembles `CardView`.

## vibe-kanban provider update

Update `vibekanban/_bop_provider_impl.py` to pass additional fields in each
task object so vibe-kanban's card face gets the same data:

```python
tasks.append({
    "id":           ...,
    "title":        ...,
    "status":       ...,
    "priority":     meta.get("priority", "P4"),
    "provider":     meta.get("provider", ""),
    "elapsed_s":    elapsed_seconds(meta),
    "progress":     meta.get("progress"),
    "ac_spec_id":   meta.get("ac_spec_id"),
})
```

## Acceptance

- `TermCaps::detect()` correctly classifies Dumb / Basic / Extended / Full / TrueColor
- Terminal width re-queried on every render (not cached)
- Level 3 section headers use `░▒▓ LABEL ▓▒░░░` fill to terminal width
- Level 3 left-accent `▌` renders in state color
- Level 4 two-column layout activates at width ≥ 100
- Level 4 progress bar uses RGB gradient (amber→green)
- All glyphs used are BMP (U+0000–U+FFFF); SMP card glyphs only if `glyph` field present and level ≥ Full
- Graceful fallback: if detection fails, defaults to Extended
- `make check` passes
- `output/result.md` with ASCII screenshots of all four levels
