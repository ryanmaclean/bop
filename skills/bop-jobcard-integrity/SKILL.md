---
name: bop-jobcard-integrity
description: Use when agents must write to .jobcard bundles safely and consistently without corruption.
---

# Bop Jobcard Integrity

## Mission

Guarantee `.jobcard` writes are deterministic, atomic, and schema-safe.

## Hard Rules

1. Treat the card directory as the state machine; never bypass state folder semantics.
2. Never perform partial in-place writes to `meta.json`.
3. Never rename cards while in `running/`.
4. Prefer `bop` commands over manual filesystem mutation.
5. If manual write is unavoidable, follow the atomic protocol in `references/write_protocol.md`.
6. Set routing fields with `bop meta set <id> --workflow-mode <mode> --step-index <n>`, not ad-hoc edits.
7. Treat `pending/` as quarantine-enforced: invalid `meta.json` is moved to `failed/` by dispatcher.
8. Treat `links.md`, `Logs.webloc`, and `Session.webloc` as system-managed card UI artifacts.
9. On macOS, card/template copies must use APFS clone semantics (`ditto --clone` or `cp -c`); do not add plain `cp -R/-r` fallback for card paths.
10. Terminal-state card refresh/compression must use `ditto --hfsCompression` (transparent, idempotent, no adapters/converters).

## Stage-Scoped Write Surface

Use only stage-allowed files from `references/allowed_writes.tsv`.

## Verification

- `bop inspect <id>` must parse and show valid stage/meta.
- `jq . <meta.json>` must succeed.
- `bop meta set ...` is the canonical mutation path for workflow routing metadata.
- `bop policy check --staged` before merge/integration.
