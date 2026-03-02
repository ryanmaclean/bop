# bop Factory — launchd Services

> **Preferred method:** Use `bop factory install` instead of manually copying plists.

## Quick Start

```zsh
bop factory install   # generates plists from repo root, loads both services
bop factory status    # check if dispatcher + merge-gate are running
bop factory stop      # stop both services
bop factory start     # restart both services
bop factory uninstall # unload + remove plist files
```

## What It Does

`bop factory install` generates two launchd user agents:

| Label | Service |
|-------|---------|
| `sh.bop.dispatcher` | Polls `.cards/pending/`, dispatches agents |
| `sh.bop.merge-gate` | Polls `.cards/done/`, runs acceptance criteria |

Plists are written to `~/Library/LaunchAgents/` with:
- Correct `WorkingDirectory` (auto-detected from repo root)
- Correct `CARDS_DIR` environment variable
- PATH including `~/.cargo/bin` for Rust toolchain
- Log files at `/tmp/bop-dispatcher.{log,err}` and `/tmp/bop-merge-gate.{log,err}`

## Logs

```zsh
tail -f /tmp/bop-dispatcher.log   # dispatcher stdout
tail -f /tmp/bop-dispatcher.err   # dispatcher stderr
tail -f /tmp/bop-merge-gate.log   # merge-gate stdout
tail -f /tmp/bop-merge-gate.err   # merge-gate stderr
```

## Legacy Plists

The `.plist` files in this directory are **deprecated reference copies**.
Use `bop factory install` which generates correct plists dynamically.
