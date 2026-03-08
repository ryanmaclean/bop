# Codex MCP Per-Project Routing

`auto-codex` MCP server context is controlled by `AC_PROJECT_DIR`.

## How It Works

- `~/.codex/config.toml` can define a default `AC_PROJECT_DIR` for `[mcp_servers.auto-codex]`.
- Bop can override that default per invocation by setting `AC_PROJECT_DIR` in the process environment for `codex exec`.
- Process environment takes precedence over MCP server env defaults in global config.
- For reliability, Bop also passes `-c mcp_servers.auto-codex.env.AC_PROJECT_DIR="..."` on `codex exec`, which forces the same value in the invocation config.

## Bop Pattern

Use per-project injection so each run resolves MCP context to the current repo:

- `dispatch.nu` sets `AC_PROJECT_DIR=$PROJECT_DIR` in `codex_shell_cmd`.
- `adapters/codex.nu` sets `AC_PROJECT_DIR=$workdir` (card workdir / repo root) before `codex exec`.

This keeps a single global Codex config usable across projects while still routing MCP tools to the correct project at runtime.
