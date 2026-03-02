---
name: bop-mcp-linear
description: Use when bop stages need optional Linear issue/project synchronization.
---

# Bop MCP Linear

## Mission

Sync task state with Linear when enabled for the project.

## Stage Fit

- plan: optional.
- implement: optional.
- qa: optional.

## Activation

Requires Linear integration and MCP enablement (`LINEAR_API_KEY`, `LINEAR_MCP_ENABLED`).

## Bop Mapping

1. Read stage via `bop inspect <id>`.
2. Mirror planned/implemented/verified status to Linear.
3. Continue normal card execution (`bop dispatcher --once`, `bop merge-gate --once`).
