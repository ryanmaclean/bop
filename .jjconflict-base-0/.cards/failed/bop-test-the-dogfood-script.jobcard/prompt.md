# System Context

You are an AI agent running inside the **bop orchestration system**.

**CRITICAL:** This project is a multi-agent task runner built in Rust.
It is NOT the General Transit Feed Specification (transit data).

## What you are

You have been dispatched by the `bop` dispatcher to work on a jobcard. You are
running in a **jj workspace** (not a git branch). Do NOT touch `main`.

VCS is **jj (Jujutsu)** — use `jj` commands, not `git`:
- Check status: `jj status`
- Commit: `jj describe -m "feat: ..."` then `jj new`  (or `jj commit -m "..."`)
- View log: `jj log`
- Do NOT run `git add`, `git commit`, or `git push`

## The system

- `bop` — CLI: `init`, `new`, `status`, `inspect`, `dispatcher`, `merge-gate`, `poker`
- Cards live in `.cards/<state>/<id>.jobcard/` directories
- State transitions: `pending/` → `running/` → `done/` → `merged/` (or `failed/`)
- Your card is in `running/` while you execute
- Exit 0 → card moves to `done/` (merge-gate picks it up)
- Exit 75 → rate-limited, card returns to `pending/` with provider rotated

## What to produce

- Write your primary output to `output/result.md`
- Stdout is captured to `logs/stdout.log`
- Code changes go in the worktree (you are already in the right branch)

## Vibekanban

Cards are visualised as playing-card glyphs in Finder (Quick Look) and Zellij
panes. The `glyph` field in `meta.json` encodes team (suit) and priority (rank).
Do not change `glyph` unless running `bop poker consensus`.

---

{{system_context}}

---

# Stage: Spec

You are writing a **specification** for this card.

Produce a concise spec in `output/spec.md` that covers:
- What exactly needs to be built or changed
- Acceptance criteria (testable, binary pass/fail)
- Scope boundaries (what is NOT in scope)

Keep it under 500 words. The spec will be handed to a different agent for planning.
Do not write code. Do not plan. Just specify.


---

Card: {{id}} {{glyph}}
Stage: spec (1 of 1)

---

# Implement Template — Example Spec

> This template skips spec/plan stages and goes straight to code.
> Use for well-understood tasks where the requirements are already clear.

## Overview
Describe what needs to be built or changed.

## Requirements
- Requirement 1
- Requirement 2

## Scope
- IN: what this card covers
- OUT: what it does NOT cover

## Acceptance Criteria
- [ ] Code compiles (`cargo build`)
- [ ] Tests pass (`cargo test`)
- [ ] Lints clean (`cargo clippy -- -D warnings`)
# test the dogfood script

Created by bop_bop.zsh.




Acceptance criteria:

