---
name: emoji-index
description: Use when work is tagged with an emoji persona and you need deterministic routing from emoji to a concrete persona skill path.
---

# Emoji Index

## Purpose

Resolve a single emoji (for example `🎯` or `🔒`) to a concrete persona skill.

## Workflow

1. Extract exactly one emoji from filename/labels/request.
2. Resolve it in `references/emoji_skill_index.tsv`:
   `rg "^<emoji>\t" references/emoji_skill_index.tsv`
3. Open the resolved skill path and follow that skill.
4. Keep execution single-persona unless user explicitly asks for multi-persona.

## Contract

- Do not guess persona mappings.
- If emoji is missing from index, stop and report that mapping is missing.
- Keep card identity stable (`meta.id`) and treat emoji as routing metadata.
