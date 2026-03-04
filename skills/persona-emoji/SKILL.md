---
name: persona-emoji
description: Use when a card or request includes a persona emoji (for example `🎯`, `🔒`, or `🚨`) and you need role-specific execution behavior with minimal token overhead.
---

# Persona Emoji

## Purpose

Route one emoji to one persona skill and execute that role's mandate,
deliverables, guardrails, and escalation rule.

## Trigger Signals

- Card filename starts with an emoji prefix: `<emoji>-<slug>.bop`
- `meta.labels` includes `kind=persona` and `persona=<emoji>`
- User explicitly asks for an emoji persona

## Token-Efficient Workflow

1. Extract exactly one emoji from filename/labels/request.
2. Resolve the persona skill via emoji index:
   `rg "^<emoji>\t" ../emoji-index/references/emoji_skill_index.tsv`
3. Open the resolved `skill_path` and follow that skill only.
4. Keep `meta.id` stable and independent from emoji prefix.
5. Fallback only if index is unavailable:
   `rg "^<emoji>\t" references/personas.tsv`

## Output Contract

- Start with one line: `<emoji> <role>: <mission>`.
- Provide three bullets only:
  - `Plan:` concrete next actions.
  - `Checks:` validation and acceptance checks.
  - `Handoff:` what next role/persona needs.
- Enforce guardrails and escalation conditions from the lookup row.

## Multi-Persona Rule

- Default is single-persona execution.
- If user requests multiple personas, pick one lead persona and list the
  supporting personas in `Handoff`, do not blend mandates.

## Lookup File

- Path: `references/personas.tsv`
- One row per emoji; 50 rows total.
- Keep row text concise so lookup stays high-signal and cheap to load.
