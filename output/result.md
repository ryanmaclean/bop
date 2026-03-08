# AC Implementation Plan Progress in Quick Look + CLI — Results

## Summary

Surfaced Auto-Claude's per-phase/subtask progress into bop's two visual layers:
a compact inline progress bar in `bop list` (CLI), and a full "Plan" tab in the
macOS Quick Look card preview. The only new data-model field is
`ac_spec_id: Option<String>` on `Meta` — everything else is derived at render
time from the existing `implementation_plan.json` file.

**Implementation stats:**
- 1 new module: `crates/bop-cli/src/acplan.rs` (860 lines, 36 tests)
- 6 phases, 13 subtasks — all completed
- 5 services touched: bop-core, bop-cli, macOS Quick Look, dispatch.nu, output
- 953 tests pass (`make check` clean: test + clippy + fmt)
- Xcode build succeeds for Quick Look extension

---

## 1. CLI — `bop list` with AC Progress Bar

When a card has `ac_spec_id` set in its `meta.json`, `bop list` renders an
extra progress line below the card row showing a 16-cell block bar, N/T subtask
count, current phase name, and a half-circle completion glyph.

### ASCII Screenshot (Full renderer, 256-color terminal)

```
░▒▓ RUNNING (2) ▓▒░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
▌ ▶ 027-ac-progress-quicklook-cli  claude  6m14s  92%
▌   ████████████████░░░░  12/13   Verification + Output  ◕
▌ ▶ 029-bop-ui                     codex   2m30s  50%
▌   ████████░░░░░░░░  10/20   Phase 4: Widgets  ◔

░▒▓ PENDING (1) ▓▒░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
▌ · 030-release-pipeline

░▒▓ DONE (1) ▓▒░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
▌ ✓ 022-atomic-write-meta  claude  4m02s
▌   ████████████████████  7/7   Phase 2: CLI + Docs  ●

░▒▓ 5 total │ 67% success │ avg 4m15s ▓▒░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
```

### Progress line format

```
▌   ████████████░░░░  6/7   Phase 2: CLI + Docs  ◑
     ^^^^^^^^^^^^^^^^ ^^^^^  ^^^^^^^^^^^^^^^^^^^^  ^
     16-cell bar      N/T    current phase name    half-circle
```

- **Block bar**: 16 characters wide, `█` (U+2588) for completed fraction,
  `░` (U+2591) for remainder
- **N/T count**: completed subtasks / total subtasks across all phases
- **Phase name**: name of the first phase with incomplete subtasks (truncated
  to 24 characters)
- **Half-circle glyph**: encodes current-phase fraction using BMP-safe
  characters:

  | Fraction   | Glyph | Codepoint |
  |-----------|-------|-----------|
  | 0%         | `○`   | U+25CB    |
  | 1–33%      | `◔`   | U+25D4    |
  | 34–66%     | `◑`   | U+25D1    |
  | 67–99%     | `◕`   | U+25D5    |
  | 100%       | `●`   | U+25CF    |

When `ac_spec_id` is absent or the plan file cannot be found, the progress line
falls back to the existing moon-quarter glyph and percentage display — no
behavior change for non-AC cards.

### Watch mode

`bop list --watch` now additionally watches `.auto-claude/specs/` for changes
to `implementation_plan.json` files. When an AC agent commits subtask
progress, the plan data is reloaded and the display updates without requiring
a card state change.

---

## 2. Quick Look — Plan Tab

### Tab appearance

The Plan tab appears in the Quick Look preview tab bar only when the card has
`ac_spec_id` set and the corresponding `implementation_plan.json` loads
successfully. The tab label shows the subtask count: **Plan (12/13)**.

### Tab bar with Plan tab

```
  Overview    Subtasks (5)    Stages    Plan (12/13)    Spec    Logs
  ────────    ────────────    ──────    ────────────    ────    ────
                                       ^^^^^^^^^^^^
                                       new tab
```

### Plan tab layout

```
  12 of 13 subtasks complete                              92%
  ██████████████████████████████████████████████████░░░░░░

  ◉  Core Data Model — ac_spec_id field                  1/1
     ✓  Add ac_spec_id field to Meta struct

  ◉  Plan Loader Module                                  3/3
     ✓  Create acplan.rs with AcPlan types and parser
     ✓  Add find_git_root and resolve_spec_dir
     ✓  Add plan_summary and enrich_card_view

  ◉  CLI Rendering — bop list + status --watch           4/4
     ✓  Add ac_subtasks_done/total to CardView
     ✓  Update render_progress with N/T + half-circle
     ✓  Wire plan enrichment into collect_card_views
     ✓  Update list_cards_watch to reload plans

  ◉  Quick Look — Plan Tab                               3/3
     ✓  Add acSpecId, AcPlan structs, plan CardTab
     ✓  Git root discovery + plan loading
     ✓  Plan tab rendering with SF Symbol icons

  ◉  dispatch.nu — Card Linkage                          1/1
     ✓  link_card_to_spec in run_spec

  ◔  Verification + Output                               1/2
     ✓  Run make check
     ↻  Create output/result.md
```

### SF Symbol icons

**Phase status** — rendered with `Image(systemName:)`:

| State       | SF Symbol                    | Color                  |
|------------|------------------------------|------------------------|
| All done    | `circle.fill`                | Green (`.stageActive`) |
| Partial     | `circle.lefthalf.filled`     | Amber (`.pillOrange`)  |
| Not started | `circle`                     | Dim (`.textMuted`)     |

**Subtask status** — rendered per-row:

| State       | SF Symbol                        | Color                  |
|------------|----------------------------------|------------------------|
| Completed   | `checkmark.circle.fill`          | Green (`.stageActive`) |
| In progress | `arrow.triangle.2.circlepath`    | Amber (`.pillOrange`)  |
| Pending     | `circle`                         | Dim (`.textMuted`)     |

### Interaction

- Phase rows are **collapsible** — tap to toggle expand/collapse
- Phase header background uses existing card row style
  (`Color.black.opacity(0.15)` + `RoundedRectangle` border)
- Chevron indicator (`chevron.down` / `chevron.right`) shows expand state
- Plan data is loaded once in `preparePreviewOfFile` — no lazy async needed

### Git root discovery

`findGitRoot(from:)` walks parent directories from the card URL (max 6 levels)
looking for a `.auto-claude` directory. `resolveSpecDir(gitRoot:specId:)` then
searches `.auto-claude/specs/` for a directory matching the `<id>-*` prefix
pattern and returns the `implementation_plan.json` path.

---

## 3. Changes Made

### bop-core (`crates/bop-core/src/lib.rs`)

- Added `ac_spec_id: Option<String>` field to `Meta` struct
- Serde annotation: `#[serde(default, skip_serializing_if = "Option::is_none")]`
- Key serializes as `"ac_spec_id"` in JSON
- Added to `Default` impl as `None`
- 2 new tests: `meta_ac_spec_id_round_trips`, `meta_ac_spec_id_omitted_when_none`

### bop-cli — new `acplan.rs` module (`crates/bop-cli/src/acplan.rs`)

- `AcPlan`, `AcPhase`, `AcSubtask` — serde `Deserialize` structs
- `parse_plan(path)` — JSON parser with extra-field tolerance
- `find_git_root(start_dir)` — walks parents (max 6 levels) for `.git` or
  `.auto-claude` markers
- `resolve_spec_dir(git_root, ac_spec_id)` — globs
  `.auto-claude/specs/<id>-*/implementation_plan.json`
- `half_circle_glyph(frac)` — BMP-safe circle glyphs at spec thresholds
- `plan_summary(plan)` — returns `PlanSummary` with completed/total counts,
  current phase name, and current phase fraction
- `enrich_card_view(view, meta, git_root)` — populates `CardView` fields
  from plan data (silent no-op when no spec)
- 36 unit tests covering all functions, edge cases, and glyph BMP safety

### bop-cli — renderer updates

- `CardView` gained `ac_subtasks_done: Option<usize>` and
  `ac_subtasks_total: Option<usize>` fields
- `CardRenderer::render_progress()` signature extended with `ac_done` and
  `ac_total` parameters
- All 5 renderers updated (dumb, basic, extended, full, truecolor):
  - When AC data present: show `N/T` count + `half_circle_glyph`
  - When absent: existing moon-quarter glyph + percentage (unchanged)
- `kanban.rs` `build_progress_line` updated similarly
- 10+ test helper `card()` functions updated across render/*.rs and ui/*.rs

### bop-cli — list wiring (`crates/bop-cli/src/list.rs`)

- `collect_card_views()` calls `acplan::enrich_card_view()` after `from_meta()`
- `collect_card_groups()` computes git root once via `acplan::find_git_root()`
- `list_cards_watch()` additionally watches `.auto-claude/specs/` directory
- Channel type changed from `()` to `bool` to distinguish plan changes
  (force redraw) from card changes (check `stats_changed`)

### macOS Quick Look (`macos/bop/PreviewViewController.swift`)

- `BopCardMeta`: added `acSpecId: String?` with `CodingKey "ac_spec_id"`
- New Codable structs: `AcPlan`, `AcPhase`, `AcSubtask`
- `CardTab.plan` case — shown only when `acSpecId` is set and plan loads
- `availableTabs` computed property filters plan tab dynamically
- `tabName(for:)` shows `"Plan (N/T)"` count in tab label
- `findGitRoot(from:)` — walks parent dirs (max 6 levels) for `.auto-claude`
- `resolveSpecDir(gitRoot:specId:)` — prefix match under `.auto-claude/specs/`
- `loadAcPlan(cardURL:specId:)` — combines root discovery + spec resolution
- `planTab()` — overall progress bar + per-phase collapsible sections
- `planPhaseSection(_:)` — phase header with SF Symbol icon, N/T count, chevron
- `phaseIconInfo(_:)` / `subtaskIconInfo(_:)` — icon + color helpers
- `@State collapsedPhases: Set<String>` for expand/collapse state

### dispatch.nu

- New `link_card_to_spec` function: searches `.cards/` state dirs (pending,
  running, done, failed, merged) for a card whose `meta.json` id matches the
  spec slug or contains the spec ID
- When found: reads meta, upserts `ac_spec_id` field, writes back
- Called from `run_spec` after `write_approval`
- Non-fatal: prints skip message if no matching card found

---

## 4. Verification

```
$ make check
cargo test:       953 passed (846 bop-cli + 106 bop-core + 1 doc-test)
cargo clippy:     0 warnings
cargo fmt --check: 0 formatting issues

$ xcodebuild ... -scheme JobCardHost build
BUILD SUCCEEDED
```

All acceptance criteria met:
- [x] `Meta` has `ac_spec_id: Option<String>` (serde rename `ac_spec_id`)
- [x] `bop list` renders block bar + phase half-circle for cards with `ac_spec_id`
- [x] `bop status --watch` reloads plan on file changes
- [x] Quick Look shows "Plan" tab when `ac_spec_id` resolves to a readable plan
- [x] Plan tab shows overall block bar + per-phase subtask list with SF Symbol icons
- [x] Git root discovery walks up from card dir (max 6 levels)
- [x] `make check` passes
- [x] `output/result.md` with ASCII screenshot and QL tab description (this file)
