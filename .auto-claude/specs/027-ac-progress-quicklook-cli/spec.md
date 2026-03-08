# AC implementation plan progress in Quick Look + CLI

## Context

Auto-Claude's `implementation_plan.json` has per-phase subtask state
(pending / in_progress / completed) that gives a clear picture of how far
along a job is. Neither the Quick Look card preview nor `bop list` surface
this — only raw `progress: Int` from `meta.json` is shown. The goal is to
pipe AC's phase/subtask tree into the card's visual layer with zero extra
tooling: block progress bars in the CLI, and a dedicated "Plan" tab in Quick
Look alongside the existing Subtasks tab.

## Data model

### 1. `ac_spec_id` field in `Meta`

Add to `crates/bop-core/src/lib.rs`:

```rust
pub ac_spec_id: Option<String>,   // e.g. "022"
```

This is the only link needed. Quick Look and CLI resolve the full path:

```
<git_root>/.auto-claude/specs/<ac_spec_id>-*/implementation_plan.json
```

Use a glob search (`read_dir` + prefix match on `<ac_spec_id>-`) to find the
spec dir — do not hardcode the slug.

### 2. How `ac_spec_id` gets written

Two entry points:

**dispatch.nu** — when `write_approval` fires before spawning a pane, also
search `.cards/` (all state dirs) for a card whose `meta.json` `id` matches
the spec slug or whose `title` contains the spec ID. If found, write
`ac_spec_id` into that card's meta. This is the dogfood path where bop uses
itself to track its own development.

**Adapter convention** — if an adapter runs AC internally, it can write
`ac_spec_id` to `meta.json` via `write_meta` before exiting.

For cards with no AC spec, `ac_spec_id` is absent — no change to rendering.

## CLI changes (`crates/bop-cli/src/main.rs` or status command)

### `bop list` / `bop status` inline progress

When a card has `ac_spec_id` set, load `implementation_plan.json` and render
a compact one-liner after the card row:

```
⚙ 022-atomic-write-meta   running   claude   6m 14s
  ████████████░░░░  6/7   Phase 2: CLI + Docs  ◑
```

Format:
- Block bar: 16 chars, `█` per completed subtask fraction, `░` remainder
- `N/T` count: completed subtasks / total subtasks across all phases
- Current phase name (truncated to 24 chars)
- Phase completion half-circle: `○◔◑◕●` (0% 25% 50% 75% 100%)
  - U+25CB, U+25D4, U+25D1, U+25D5, U+25CF — all BMP-safe

Half-circle thresholds (fraction of subtasks in current phase done):
- 0 % → `○`
- 1–33 % → `◔`
- 34–66 % → `◑`
- 67–99 % → `◕`
- 100 % → `●`

Colors: completed subtasks = green, in_progress = amber, pending = dim.
Only shown for cards where `ac_spec_id` resolves to a readable file.

### `bop status --watch` (spec 025)

When the plan view is live, also reload `implementation_plan.json` on each
notify event — it changes as the AC agent commits subtasks.

## Quick Look changes (`macos/bop/PreviewViewController.swift`)

### New tab: "Plan"

Add `plan = "Plan"` to the `CardSection` enum (alongside `subtasks`).
Show the tab only if `ac_spec_id` is set and `implementation_plan.json` loads.

The Plan tab reads `implementation_plan.json` from the git root (walk up from
the card dir via `../` until `.git` or `.auto-claude` is found).

### New Codable structs

```swift
private struct AcPlan: Codable {
    let phases: [AcPhase]
}

private struct AcPhase: Codable {
    let id: String
    let name: String
    let subtasks: [AcSubtask]
}

private struct AcSubtask: Codable {
    let id: String
    let description: String
    let status: String   // "pending" | "in_progress" | "completed"
}
```

### Plan tab rendering

Overall progress bar at top (same style as existing `subtasksTab` header):

```
8 of 12 subtasks complete          67%
████████████░░░░░░░░
```

Then per-phase collapsible rows:

```
◉  Phase 1 — Crash Safety          ●●●●●  5/5
   ✓ Make write_meta atomic
   ✓ Make write_providers atomic
   ✓ Add recover_orphans function
   ✓ Call recover_orphans on startup
   ✓ Add bop recover CLI command

◔  Phase 2 — CLI + Docs            ●○○    1/3
   ✓ Write result.md (in_progress)
   ○ Run make check
   ○ Final review
```

Glyph legend (BMP-safe, existing SF Symbols for Quick Look):
- Phase status icon: `circle.fill` (done), `circle.lefthalf.filled` (partial),
  `circle` (not started) — SF Symbols, no font dep
- Subtask row: `checkmark.circle.fill` (completed, green),
  `arrow.triangle.2.circlepath` (in_progress, amber), `circle` (pending, dim)
  — matches the existing `subtasksTab` icon style

Phase name row background: match existing card row style
(`Color.black.opacity(0.15)` + `RoundedRectangle` border).

### `BopCardMeta` addition

```swift
let acSpecId: String?

enum CodingKeys: String, CodingKey {
    // ... existing keys ...
    case acSpecId = "ac_spec_id"
}
```

Load `AcPlan` lazily when the Plan tab is first shown — do not block the
initial card render. Use a `@State var acPlan: AcPlan? = nil` loaded in
`.task { }` on the tab view.

Git root discovery: walk parent dirs from `cardURL.deletingLastPathComponent()`
until a dir containing `.auto-claude` is found, max 6 levels. Cache the result.

## Acceptance

- `Meta` has `ac_spec_id: Option<String>` field (serde rename `ac_spec_id`)
- `bop list` renders block bar + phase half-circle for cards with `ac_spec_id`
- `bop status --watch` reloads plan on file changes
- Quick Look shows "Plan" tab when `ac_spec_id` resolves to a readable plan
- Plan tab shows overall block bar + per-phase subtask list with SF Symbol icons
- Git root discovery walks up from card dir (max 6 levels)
- `make check` passes
- `output/result.md` with ASCII screenshot of CLI output and description of QL tab
