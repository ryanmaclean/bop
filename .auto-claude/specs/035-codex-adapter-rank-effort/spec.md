# Spec 035 — codex adapter: card rank → reasoning effort + --full-auto

## Overview

`adapters/codex.nu` currently uses `--dangerously-bypass-approvals-and-sandbox` and ignores
the card's `meta.json`. This spec makes the adapter:

1. Read `meta.json` from the workdir to determine card priority/rank
2. Map rank → `model_reasoning_effort` so expensive reasoning is reserved for P1 cards
3. Switch from `--dangerously-bypass-approvals-and-sandbox` to `--full-auto` (safer sandbox)

## Priority → reasoning effort mapping

Card rank is stored in `meta.json` as `priority: i64` (set by `bop poker consensus`):

| priority value | rank | effort flag |
|---|---|---|
| 1 | Ace (P1) | `model_reasoning_effort=xhigh` |
| 2 | King/Queen (P2) | `model_reasoning_effort=high` |
| 3 | Jack (P3) | `model_reasoning_effort=medium` |
| 4–10 | 2–10 (P4) | `model_reasoning_effort=low` |
| null/missing | unknown | `model_reasoning_effort=high` (safe default) |

The glyph field (e.g. `🂡` for Ace of Spades) can also be used as a cross-check:
suit/rank is encoded in the SMP Unicode range (U+1F0A0+), but `priority` is the
canonical integer to use here — no need to decode glyphs.

## Implementation

### `adapters/codex.nu`

Add a helper `effort_for_priority [p: int] -> string` that returns the right level.

Read `meta.json` at the start of `main`:
```nushell
let meta_path = [$workdir "meta.json"] | path join
let priority = if ($meta_path | path exists) {
    (open $meta_path | get -o priority | default null)
} else { null }
let effort = effort_for_priority $priority
```

Change the codex invocation from:
```
codex exec --dangerously-bypass-approvals-and-sandbox -
```
to:
```
codex exec --full-auto -c model_reasoning_effort=<effort> -
```

Keep all existing: stdin prompt pass-through, stdout/stderr log capture, rate-limit
detection (exit 75), network error detection (exit 75), self-test suite.

### Self-tests to add

Add to `run_tests`:
- `effort_for_priority 1` → `"xhigh"`
- `effort_for_priority 2` → `"high"`
- `effort_for_priority 3` → `"medium"`
- `effort_for_priority 4` → `"low"`
- `effort_for_priority 10` → `"low"`
- `effort_for_priority null` → `"high"` (safe default)
- `effort_for_priority -1` → `"high"` (out of range → default)

## Acceptance Criteria

- [ ] `nu adapters/codex.nu --test` passes all tests including new effort tests
- [ ] `cargo test` passes (no Rust changes needed, but verify nothing breaks)
- [ ] The adapter no longer contains `--dangerously-bypass-approvals-and-sandbox`
- [ ] A P1 card (priority=1) produces `model_reasoning_effort=xhigh` in invocation
- [ ] A P4 card (priority=4) produces `model_reasoning_effort=low` in invocation
- [ ] A card with no meta.json produces `model_reasoning_effort=high`

## Files to modify

- `adapters/codex.nu` — add `effort_for_priority`, read meta.json, change exec flags
