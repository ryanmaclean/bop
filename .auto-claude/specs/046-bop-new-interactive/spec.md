# Spec 046 — bop new: interactive card creation TUI

## Overview

`bop new <template> <id>` creates a card non-interactively. When called with
no arguments (just `bop new`), this spec adds an interactive TUI wizard that
guides the user through card creation step by step.

## Wizard flow

```
bop new
```

Step 1 — Template selection (list with arrow keys):
```
Select template:
  ▶ implement   — Standard implement/QA card
    cheap       — Single-stage, low-cost
    ideation    — Brainstorm + spec only
    roadmap     — Multi-phase planning
    qa-only     — QA pass on existing work
    mr-fix      — Fix a merge-request failure
```

Step 2 — Card ID:
```
Card ID: team-arch/my-feature█
(hint: use team-arch/, team-cli/, team-quality/, team-platform/ prefixes)
```

Step 3 — Spec text (optional):
```
Paste spec (Ctrl+D to finish, Enter for blank spec):
>
```

Step 4 — Provider chain:
```
Provider chain (comma-separated, or Enter for default):
Default: [codex, claude, ollama-local]
> █
```

Step 5 — Confirmation:
```
Create card?
  template: implement
  id:       team-arch/my-feature
  provider: codex, claude, ollama-local
  spec:     (empty — edit output/spec.md after creation)

[Y]es  [N]o  [E]dit
```

## Implementation

Use `ratatui` (already in deps) for the selection UI. For simple text input,
use `crossterm`'s event loop (already in deps via ratatui). Reuse the event
loop pattern from `bop ui` input.rs.

If `stdin` is not a TTY (piped input), fall back to the existing non-interactive
`bop new <template> <id>` behaviour.

## Acceptance Criteria

- [ ] `bop new` with no args launches interactive wizard
- [ ] Template list shows all templates from `.cards/templates/`
- [ ] Card is created in `.cards/pending/` after confirmation
- [ ] `bop new implement my-id` (non-interactive) still works unchanged
- [ ] Stdin pipe (`echo "y" | bop new`) uses non-interactive path
- [ ] `cargo clippy -- -D warnings` clean
- [ ] `cargo test` passes

## Files to modify

- `crates/bop-cli/src/cards.rs` — add interactive wizard entry point
- `crates/bop-cli/src/main.rs` — detect no-args and call wizard
- `crates/bop-cli/src/ui/wizard.rs` — new file: interactive card wizard
- `crates/bop-cli/src/ui/mod.rs` — pub mod wizard
