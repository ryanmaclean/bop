# Spec 051 — bop export: card bundle sharing (P6 Universality)

## Overview

TRIZ P6 (Universality): the `.card/` bundle is already a universal format —
filesystem, JSONL, markdown. `bop export` makes it explicitly shareable as a
tarball that anyone can inspect, replay, or import without running bop.

## Commands

```sh
bop export <id>                  # tar card to ./bop-export-<id>.tar.gz
bop export <id> --out <path>     # explicit output path
bop export <id> --strip-logs     # exclude logs/ (smaller, just meta+output)
bop export <id> --strip-worktree # exclude worktree/ (default: already excluded)
bop import <path>                # untar into .cards/done/ for review
```

## Export format

```
bop-export-team-arch-spec-041-20260308T142201.tar.gz
└── team-arch-spec-041/
    ├── meta.json
    ├── spec.md
    ├── prompt.md
    ├── output/
    │   └── result.md
    └── logs/
        ├── stdout.log
        ├── stderr.log
        └── events.jsonl
```

`worktree/` is excluded by default (it's a git worktree — not portable).

## Import

`bop import <tarball>` extracts into `.cards/done/<id>/` so the card is
immediately visible in `bop ui` and inspectable with `bop inspect`.
If a card with the same id already exists, prompt to confirm overwrite
(or `--force` to skip prompt).

## Manifest

Prepend a `MANIFEST.json` to the tarball:
```json
{
  "bop_version": "0.2.0",
  "exported_at": "2026-03-08T14:22:01Z",
  "card_id": "team-arch/spec-041",
  "state": "done",
  "provider": "codex",
  "cost_usd": 0.18,
  "tokens": 12400
}
```

## Acceptance Criteria

- [ ] `bop export <id>` creates a `.tar.gz` in the current directory
- [ ] Tarball contains meta.json, spec.md, output/, logs/ by default
- [ ] `--strip-logs` excludes logs/
- [ ] `bop import <tarball>` extracts to `.cards/done/`
- [ ] `MANIFEST.json` is present in every exported tarball
- [ ] Export fails with clear error if card id does not exist
- [ ] `cargo clippy -- -D warnings` clean
- [ ] `cargo test` passes (unit test: export + re-import round-trip)

## Files

- `crates/bop-cli/src/export.rs` — new module (export + import)
- `crates/bop-cli/src/main.rs` — wire `bop export` and `bop import` subcommands
