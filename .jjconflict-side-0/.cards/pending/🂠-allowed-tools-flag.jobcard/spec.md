# Add --allowed-tools flag to claude.zsh adapter

## Problem
Every dispatched claude agent sees the full tool list (web search, playwright, Linear
MCP, etc.) in its system prompt. This bloats context per turn and invites irrelevant
tool use. For code tasks, only shell+file tools are needed.

## Change

In `adapters/claude.zsh`, add `--allowed-tools` to the claude invocation.

The allowed list for code tasks:
```
Bash,Read,Write,Edit,Glob,Grep,LS,TodoWrite,mcp__sequential-thinking__sequentialthinking
```

Make the list configurable via env var `CLAUDE_ALLOWED_TOOLS` with the above as default:
```zsh
ALLOWED_TOOLS="${CLAUDE_ALLOWED_TOOLS:-Bash,Read,Write,Edit,Glob,Grep,LS,TodoWrite,mcp__sequential-thinking__sequentialthinking}"

perl -e 'alarm(shift); exec @ARGV or die $!' -- \
  "$TIMEOUT_S" \
  claude -p "$(cat "$prompt_file")" \
  --dangerously-skip-permissions \
  --output-format json \
  --allowed-tools "$ALLOWED_TOOLS" \
  > "$stdout_log" 2> "$stderr_log"
```

## Acceptance Criteria
- `cargo build` passes (no Rust changes needed)
- `grep -q 'allowed-tools' adapters/claude.zsh`
- `grep -q 'CLAUDE_ALLOWED_TOOLS' adapters/claude.zsh`
- `jj log -r 'main..@-' | grep -q .`
