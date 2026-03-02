---
name: bop-mcp-graphiti-memory
description: Use when bop stages need cross-session memory for discoveries, attempts, and constraints.
---

# Bop MCP Graphiti Memory

## Mission

Persist and retrieve high-value context across cards and stages.

## Stage Fit

- plan: required in Auto-Claude planner mapping.
- implement: required in Auto-Claude coder mapping.
- qa: required in Auto-Claude QA mapping.

## Bop Mapping

1. Resolve stage via `bop inspect <id>`.
2. Load prior context before execution.
3. Record new findings and constraints.
4. Continue with `bop dispatcher --once`.
