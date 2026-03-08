# Spec 040 — codex MCP per-project routing

## Overview

`~/.codex/config.toml` has an `auto-codex` MCP server entry with a hardcoded
`AC_PROJECT_DIR=/Users/studio/dub`. When Codex runs bop specs via `dispatch.nu`,
the auto-codex MCP provides context from the wrong project (dub, not bop).

This spec adds project-aware MCP env injection to `codex_shell_cmd` and the
`adapters/codex.nu` adapter so Codex always gets the right project context.

## Two surfaces to fix

### 1. `dispatch.nu` — `codex_shell_cmd`

Inject `AC_PROJECT_DIR` before the codex invocation:
```nushell
$"cd ($PROJECT_DIR) && env -u CLAUDECODE AC_PROJECT_DIR=($PROJECT_DIR) codex exec --full-auto ..."
```

This overrides the global `~/.codex/config.toml` MCP env for this invocation because
`~/.codex/config.toml` MCP server env vars are merged with process env, and
process env takes precedence for `AC_PROJECT_DIR`.

### 2. `adapters/codex.nu`

The adapter receives `workdir` as its first argument (the card's workdir, which is
the repo root). Inject `AC_PROJECT_DIR` from the workdir:

```nushell
# Derive project root: workdir is the card's working directory (= repo root)
let project_root = $workdir
^sh -c $"AC_PROJECT_DIR='($project_root)' codex exec --full-auto -c model_reasoning_effort=($effort) - < '($prompt_abs)' > '($stdout_abs)' 2> '($stderr_abs)'; printf '%d' $? > '($rc_file)'"
```

### 3. `~/.codex/config.toml` — document the pattern

Add a comment block (not code — the config is user-owned) to the bop `CLAUDE.md` or
`docs/codex-mcp-setup.md` explaining:
- The `auto-codex` MCP server env `AC_PROJECT_DIR` determines which project's AC specs
  the MCP can access
- Set it per-project by injecting `AC_PROJECT_DIR` in the dispatch command
- The global config value is just the default; per-invocation env overrides it

## Verification

After this change, run a bop spec via `codex exec` and check that the auto-codex MCP
(if queried) returns bop spec context, not dub context. A simple check: look in the
Codex session for any auto-codex tool calls and verify the project path in the response.

If the auto-codex MCP is not installed for bop (no `[mcp_servers.auto-codex]` in
~/.codex/config.toml), the env injection is still correct and harmless for future use.

## Acceptance Criteria

- [ ] `codex_shell_cmd` in `dispatch.nu` injects `AC_PROJECT_DIR=$PROJECT_DIR`
- [ ] `adapters/codex.nu` injects `AC_PROJECT_DIR=$workdir` into the codex invocation
- [ ] `nu adapters/codex.nu --test` passes all existing tests
- [ ] `cargo test` passes (no Rust changes)
- [ ] A `docs/codex-mcp-setup.md` (or addition to CLAUDE.md) documents the pattern

## Files to modify

- `dispatch.nu` — `codex_shell_cmd`: prepend `AC_PROJECT_DIR=($PROJECT_DIR)`
- `adapters/codex.nu` — inject `AC_PROJECT_DIR` in sh -c wrapper
- `CLAUDE.md` or `docs/codex-mcp-setup.md` — document the per-project MCP pattern
