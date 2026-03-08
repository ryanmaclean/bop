# Terminal Card Renderer — Output Screenshots

All five progressive degradation levels demonstrated with the same sample card data.

**Sample data:**
- **Running:** `my-feature` (claude, 2m14s, 67%, P1), `bugfix-auth` (codex, 43s, 22%, P3)
- **Pending:** `docs-update`, `refactor-api`, `add-tests`
- **Done:** `perf-improvements` (4m02s)
- **Failed:** `broken-card` (exit 1)
- **Stats:** 7 total | 86% success | avg 3m44s

---

## Level 0 — Dumb (TERM=dumb or unset)

No color, no unicode, pure ASCII. Works over serial consoles, `TERM=dumb` pipes,
and CI logs.

```
=== RUNNING (2) ===
[>] my-feature  claude  2m14s  67%
  [=======   ] 67%  Phase 2: Network
[>] bugfix-auth  codex  43s  22%
  [==        ] 22%  Phase 1: Core

=== PENDING (3) ===
[ ] docs-update
[ ] refactor-api
[ ] add-tests

=== DONE (1) ===
[x] perf-improvements  4m02s

=== FAILED (1) ===
[!] broken-card  exit 1

7 total | 86% success | avg 3m44s
```

**Key features:**
- `[>]` running, `[ ]` pending, `[x]` done/merged, `[!]` failed markers
- `[=====     ] 67%` ASCII progress bar (10-cell)
- Phase name annotation after progress bar
- Footer: pipe-separated aggregate stats
- Zero ANSI escape sequences, zero non-ASCII characters

---

## Level 1 — Basic (8-color ANSI, ASCII borders)

Adds ANSI 8-color (30-37) and `+--+` / `|` ASCII box borders.
Shown here with ANSI stripped for readability; actual output has colored markers
and dim provider/elapsed text.

```
+-- RUNNING (2) --+
| [>] my-feature  claude  2m14s  67%
|   [=======   ] 67%  Phase 2: Network
| [>] bugfix-auth  codex  43s  22%
|   [==        ] 22%  Phase 1: Core

+-- PENDING (3) --+
| [ ] docs-update
| [ ] refactor-api
| [ ] add-tests

+-- DONE (1) --+
| [x] perf-improvements  4m02s

+-- FAILED (1) --+
| [!] broken-card  exit 1

+-- 7 total | 86% success | avg 3m44s --+
```

**Key features:**
- `+-- LABEL (N) --+` colored headers (yellow=running, blue=pending, green=done, red=failed)
- `| ` left border on card rows and progress bars
- Colored markers: `[>]` yellow, `[!]` red, `[x]` green
- Provider and elapsed shown in DIM
- `+-- stats --+` footer with DIM styling

---

## Level 2 — Extended (16-color, single-line box drawing)

Uses bright ANSI colors (90-97), `┌─┐│└┘` box-drawing, left-accent stripe `▌`,
and block-element progress bar `████░░░░` (16-cell).

```
┌ RUNNING (2) ──────────────────────────────────────────────────────────────────┐
▌ ▶ my-feature  claude  2m14s  67%
│   ██████████░░░░░░ 67%  ◕ Phase 2: Network
▌ ▶ bugfix-auth  codex  43s  22%
│   ███░░░░░░░░░░░░░ 22%  ◔ Phase 1: Core

┌ PENDING (3) ──────────────────────────────────────────────────────────────────┐
▌ · docs-update
▌ · refactor-api
▌ · add-tests

┌ DONE (1) ─────────────────────────────────────────────────────────────────────┐
▌ ✓ perf-improvements  4m02s

┌ FAILED (1) ───────────────────────────────────────────────────────────────────┐
▌ ✗ broken-card  exit 1

└─ 7 total │ 86% success │ avg 3m44s ───────────────────────────────────────────┘
```

**Key features:**
- `┌─ LABEL ─┐` section headers with `─` fill to terminal width
- `▌` left-accent stripe in state color (bright yellow/blue/green/red)
- `▶ ✓ ✗ ·` unicode state markers
- `████████░░░░░░░░` 16-cell block progress bar
- `◔◑◕●` moon-quarter glyphs for phase progress fraction
- `│` pipe separators in footer with `└─ ... ─┘` box-drawing footer

---

## Level 3 — Full (256-color, demoscene block headers)

256-color ANSI (38;5;NNN), dithered `░▒▓ LABEL ▓▒░` header/footer pattern,
dice glyphs `⚀⚁⚂⚃⚄⚅` for priority P1-P6.

```
░▒▓ RUNNING (2) ▓▒░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
▌ ▶ my-feature  ⚀  claude  2m14s  67%
▌   ██████████░░░░░░ 67%  ◕ Phase 2: Network
▌ ▶ bugfix-auth  ⚂  codex  43s  22%
▌   ███░░░░░░░░░░░░░ 22%  ◔ Phase 1: Core

░▒▓ PENDING (3) ▓▒░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
▌ · docs-update
▌ · refactor-api
▌ · add-tests

░▒▓ DONE (1) ▓▒░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
▌ ✓ perf-improvements  4m02s

░▒▓ FAILED (1) ▓▒░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
▌ ✗ broken-card  exit 1

░▒▓ 7 total │ 86% success │ avg 3m44s ▓▒░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
```

**Key features:**
- `░▒▓ LABEL ▓▒░` dithered fade header — classic BBS/ANSI art style
- `░` fill to terminal width (80 columns shown)
- `⚀⚁⚂⚃⚄⚅` dice glyphs encode priority (P1=⚀ through P6=⚅)
- `▌` left-accent in 256-color state colors (amber 172, green 71, red 160, blue 68)
- BOLD section headers, DIM provider/elapsed/footer
- Same 16-cell progress bar and moon glyphs as Extended

---

## Level 4 — TrueColor (24-bit RGB, double-line box, column layout)

24-bit RGB color via `\x1b[38;2;R;G;Bm`, double-line box chars `╔═╗║╚═╝`,
rounded card separators `╭─╮│╰─╯`, RGB gradient progress bar.

### Single-column (width < 100)

```
╔ RUNNING (2) ═════════════════════════════════════════════════════════════════╗
║ ╭───────────────────────────────────────────────────────────────────────────╮
║ │ ▶ my-feature  ⚀  claude  2m14s  67%
║ │   ██████████░░░░░░ 67%  ◕ Phase 2: Network
║ ╰───────────────────────────────────────────────────────────────────────────╯
║ ╭───────────────────────────────────────────────────────────────────────────╮
║ │ ▶ bugfix-auth  ⚂  codex  43s  22%
║ │   ███░░░░░░░░░░░░░ 22%  ◔ Phase 1: Core
║ ╰───────────────────────────────────────────────────────────────────────────╯

╔ PENDING (3) ═════════════════════════════════════════════════════════════════╗
║ ╭───────────────────────────────────────────────────────────────────────────╮
║ │ · docs-update
║ ╰───────────────────────────────────────────────────────────────────────────╯
║ ╭───────────────────────────────────────────────────────────────────────────╮
║ │ · refactor-api
║ ╰───────────────────────────────────────────────────────────────────────────╯
║ ╭───────────────────────────────────────────────────────────────────────────╮
║ │ · add-tests
║ ╰───────────────────────────────────────────────────────────────────────────╯

╔ DONE (1) ════════════════════════════════════════════════════════════════════╗
║ ╭───────────────────────────────────────────────────────────────────────────╮
║ │ ✓ perf-improvements  4m02s
║ ╰───────────────────────────────────────────────────────────────────────────╯

╔ FAILED (1) ══════════════════════════════════════════════════════════════════╗
║ ╭───────────────────────────────────────────────────────────────────────────╮
║ │ ✗ broken-card  exit 1
║ ╰───────────────────────────────────────────────────────────────────────────╯

╚═ 7 total │ 86% success │ avg 3m44s ═════════════════════════════════════════╝
```

### Two-column layout (width >= 100)

At width >= 100, `TermCaps.two_column = true` and columns are halved. Each section
box renders at `width / 2` columns. The left column shows RUNNING, right shows
PENDING, with DONE/FAILED stacking below.

```
╔ RUNNING (2) ══════════════════════╗  ╔ PENDING (3) ══════════════════════╗
║ ╭────────────────────────────────╮   ║ ╭────────────────────────────────╮
║ │ ▶ my-feature  ⚀  claude  67%      ║ │ · docs-update
║ │   ██████████░░░░░░ 67%  ◕ Ph2     ║ ╰────────────────────────────────╯
║ ╰────────────────────────────────╯   ║ ╭────────────────────────────────╮
║ ╭────────────────────────────────╮   ║ │ · refactor-api
║ │ ▶ bugfix-auth  ⚂  codex  22%      ║ ╰────────────────────────────────╯
║ │   ███░░░░░░░░░░░░░ 22%  ◔ Ph1     ║ ╭────────────────────────────────╮
║ ╰────────────────────────────────╯   ║ │ · add-tests
╚══════════════════════════════════╝   ║ ╰────────────────────────────────╯
                                       ╚══════════════════════════════════╝
╔ DONE (1) ═════════════════════════╗
║ ╭────────────────────────────────╮  ╔ FAILED (1) ═════════════════════════╗
║ │ ✓ perf-improvements  4m02s       ║ ╭────────────────────────────────╮
║ ╰────────────────────────────────╯  ║ │ ✗ broken-card  exit 1
╚══════════════════════════════════╝  ║ ╰────────────────────────────────╯
                                      ╚══════════════════════════════════╝
╚═ 7 total │ 86% success │ avg 3m44s ════════════════════════════════════╝
```

**Key features:**
- `╔═╗║╚═╝` double-line box for section boundaries
- `╭─╮│╰─╯` rounded single-line for individual card separators within sections
- RGB gradient progress bar: each `█` cell interpolates amber `#B8690F` → green `#1E8A45`
- Two-column layout activates at `width >= 100` via `TermCaps.two_column`
- All glyphs BMP (U+0000-U+FFFF) — no SMP playing-card glyphs by default
- Dice and moon glyphs same as Full level

---

## Progressive Degradation Summary

| Level | ANSI | Borders | Progress Bar | Glyphs | Layout |
|-------|------|---------|--------------|--------|--------|
| Dumb | none | none | `[====    ]` ASCII | `[>][ ][x][!]` | single |
| Basic | 8-color | `+--+\|` ASCII | `[====    ]` ASCII | `[>][ ][x][!]` | single |
| Extended | 16-color bright | `┌─┐│└┘` box | `████░░░░` 16-cell block | `▶·✓✗ ▌` | single |
| Full | 256-color | `░▒▓` dithered | `████░░░░` 16-cell block | `▶·✓✗ ▌ ⚀-⚅ ◔◑◕●` | single |
| TrueColor | 24-bit RGB | `╔═╗║╚╝` double | `████░░░░` RGB gradient | `▶·✓✗ ⚀-⚅ ◔◑◕●` | two-col @ 100+ |

## Test Results

```
$ make check
cargo test     — 643 tests passed, 0 failed
cargo clippy   — 0 warnings (with -D warnings)
cargo fmt      — no formatting issues
```

All acceptance criteria verified:
- TermCaps::detect() classifies all 5 levels correctly (18 unit tests)
- Terminal width re-queried every render (never cached)
- Level 3 headers: `░▒▓ LABEL ▓▒░` fill to terminal width
- Level 3 left-accent `▌` in 256-color state colors
- Level 4 two-column at width >= 100
- Level 4 RGB gradient progress bar (amber -> green)
- All glyphs BMP (U+0000-U+FFFF)
- Graceful fallback: defaults to Extended on unknown TERM
