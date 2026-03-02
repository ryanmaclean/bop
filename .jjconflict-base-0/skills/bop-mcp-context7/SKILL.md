---
name: bop-mcp-context7
description: Use when a bop stage needs external library/framework documentation lookup.
---

# Bop MCP Context7

## Mission

Resolve docs quickly and feed concrete references into the current bop stage.

## Stage Fit

- spec: optional for dependency/API research.
- plan: required in Auto-Claude planner mapping.
- implement: required in Auto-Claude coder mapping.
- qa: required in Auto-Claude QA mapping.

## Bop Mapping

1. Read stage with `bop inspect <id>`.
2. Capture doc-backed decisions in stage outputs.
3. Continue execution via `bop dispatcher --once`.
