# Spec 038 — dispatch.nu: spec cost → codex reasoning effort + --full-auto

## Overview

`dispatch.nu`'s `codex_shell_cmd` currently uses `--dangerously-bypass-approvals-and-sandbox`
and doesn't pass a reasoning effort level. The global `~/.codex/config.toml` sets
`model_reasoning_effort = "xhigh"` for everything. This wastes quota on trivial specs.

This spec updates `codex_shell_cmd` to:
1. Switch to `--full-auto` (safer sandbox, still non-interactive)
2. Pass `-c model_reasoning_effort=<level>` based on the spec's `cost` field
3. Pass `-m gpt-5.3-codex` explicitly (match dub's `dispatch.codex.nu` pattern)

## Effort mapping

The spec table has a `cost` field (1–4):

| cost | description | reasoning effort |
|---|---|---|
| 1 | trivial | `low` |
| 2 | small | `medium` |
| 3 | medium | `high` |
| 4 | complex | `xhigh` |

## Implementation

### `dispatch.nu` — `codex_shell_cmd`

Change signature to accept cost:
```nushell
def codex_shell_cmd [spec_id: string, cost: int] {
    let effort = match $cost {
        1 => "low"
        2 => "medium"
        3 => "high"
        _ => "xhigh"
    }
    let base = $"($PROJECT_DIR)/.auto-claude/specs"
    let spec_dir = (ls $base | where name =~ $"/($spec_id)-" | get name | first)
    $"cd ($PROJECT_DIR) && env -u CLAUDECODE codex exec --full-auto -m gpt-5.3-codex -c model_reasoning_effort=($effort) - < ($spec_dir)/spec.md && /opt/homebrew/bin/nu ($PROJECT_DIR)/dispatch.nu mark-done ($spec_id) || /opt/homebrew/bin/nu ($PROJECT_DIR)/dispatch.nu mark-failed ($spec_id)"
}
```

### `dispatch.nu` — `run_spec`

Pass `$s.cost` to `codex_shell_cmd`:
```nushell
let cmd = if $mode == "codex" {
    codex_shell_cmd $spec_id $s.cost
} else {
    ...
}
```

## Verification

Run `nu dispatch.nu plan --wave 12` and manually verify the printed command for a cost=1
spec contains `model_reasoning_effort=low` and a cost=4 spec contains `xhigh`.

Alternatively add a `--dry-run` mode that prints the shell command without executing.

## Acceptance Criteria

- [ ] `codex_shell_cmd` uses `--full-auto` (not `--dangerously-bypass-approvals-and-sandbox`)
- [ ] cost=1 spec → `model_reasoning_effort=low` in command
- [ ] cost=4 spec → `model_reasoning_effort=xhigh` in command
- [ ] `nu dispatch.nu plan --wave 12` runs without errors
- [ ] `nu dispatch.nu status` still works

## Files to modify

- `dispatch.nu` — `codex_shell_cmd`, `run_spec`
