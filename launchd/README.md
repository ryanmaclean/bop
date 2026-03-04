# bop Factory — launchd Services

> **Preferred method:** Use `bop factory install` instead of manually copying plists.

## Quick Start

```sh
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

```sh
tail -f /tmp/bop-dispatcher.log   # dispatcher stdout
tail -f /tmp/bop-dispatcher.err   # dispatcher stderr
tail -f /tmp/bop-merge-gate.log   # merge-gate stdout
tail -f /tmp/bop-merge-gate.err   # merge-gate stderr
```

## Optional: Roadmap Hot Folder Trigger

Use a filesystem drop folder to enqueue roadmap cards without typing `bop new`:

```nu
nu scripts/install_roadmap_hotfolder_launchd.nu \
  --inbox (pwd | path join examples/roadmap-inbox/drop) \
  --cards-dir (pwd | path join .cards)
```

Then drop any `.roadmap` / `.md` / `.txt` / `.json` request file into
`examples/roadmap-inbox/drop`. The ingest agent creates `🂠-*.bop` in
`.cards/pending/` using the roadmap template and APFS clone-safe copy rules.

Remove it with:

```nu
nu scripts/install_roadmap_hotfolder_launchd.nu --uninstall
```

## Legacy Plists

The `.plist` files in this directory are **deprecated reference copies**.
Use `bop factory install` which generates correct plists dynamically.
