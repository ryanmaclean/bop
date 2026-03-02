---
name: bop-mcp-electron
description: Use for desktop UI validation in bop QA stage when the target app is Electron.
---

# Bop MCP Electron

## Mission

Automate desktop validation for Electron targets during QA.

## Stage Fit

- qa only (optional based on project capability and config).

## Activation

Requires Electron project capability and `ELECTRON_MCP_ENABLED=true`.

## Bop Mapping

1. Confirm `meta.stage=qa` with `bop inspect <id>`.
2. Run Electron validations and capture evidence in `output/`.
3. Gate merge with `bop merge-gate --once` only after QA evidence is present.
