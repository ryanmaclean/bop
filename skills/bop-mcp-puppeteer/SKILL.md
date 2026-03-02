---
name: bop-mcp-puppeteer
description: Use for web UI/browser automation in bop QA stage for non-Electron frontends.
---

# Bop MCP Puppeteer

## Mission

Automate browser QA checks for web frontend behavior.

## Stage Fit

- qa only (optional based on project capability and config).

## Activation

Requires web frontend capability and `PUPPETEER_MCP_ENABLED=true`.

## Bop Mapping

1. Confirm QA stage with `bop inspect <id>`.
2. Run browser validations and store artifacts in `output/`.
3. Run `bop merge-gate --once` when acceptance criteria pass.
