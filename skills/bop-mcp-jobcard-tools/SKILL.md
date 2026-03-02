---
name: bop-mcp-jobcard-tools
description: Use when bop execution needs Auto-Claude MCP-style progress/status/discovery tracking.
---

# Bop MCP Jobcard Tools

## Mission

Keep stage progress, discoveries, and QA state synchronized with job execution.

## Core Tool Concepts

- subtask status updates
- build progress reads
- discovery/gotcha recording
- session context reads
- QA status updates

## Stage Fit

- plan: required.
- implement: required.
- qa: required.

## Bop Mapping

1. Inspect stage (`bop inspect <id>`).
2. Update progress while stage runs (`bop dispatcher --once`, `bop logs <id>`).
3. Verify card artifacts reflect current status before merge-gate.
