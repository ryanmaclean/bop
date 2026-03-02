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

## Stage-Scoped Write Surface

Use only stage-allowed files from `references/allowed_writes.tsv`.

## Verification

- `bop inspect <id>` must parse and show valid stage/meta.
- `jq . <meta.json>` must succeed.
- `bop policy check --staged` before merge/integration.
