---
name: bop-stage-mcp-router
description: Use when you need to map Auto-Claude MCP server logic to bop stages, workflows, and commands.
---

# Bop Stage MCP Router

## Purpose

Pick the right MCP server skills for the current bop workflow and execute with deterministic signal and stage mapping.

## Signal Precedence

Resolve workflow using `references/signal_precedence.tsv` in priority order.

## Workflow

1. Resolve state/stage/workflow signals.
2. Resolve workflow mode (`references/workflow_modes.tsv`).
3. Resolve required and optional MCP servers for stage (`references/stage_server_policy.tsv`).
4. Resolve each server to a concrete skill path (`references/server_skill_index.tsv`).
5. Execute stage with bop commands (`references/step_to_bop.tsv`).

## Output Contract

- Report which signal won and why.
- Report stage, required servers used, optional servers used/skipped.
- Include exact bop command(s) run.
- Keep outputs in the card bundle (`spec.md`, `output/`, `logs/`).
