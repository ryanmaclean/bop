# Spec 059 — multi-project: bop --project <path> (TRIZ P1 Segmentation)

## Overview

P1 Segmentation: one `bop` install manages multiple repos independently by
making the project root a runtime parameter. Currently `bop` resolves `.cards/`
relative to cwd. This spec adds a global `--project` flag and a project registry.

## Global flag

```sh
bop --project /Users/studio/efi status        # bop for the efi repo
bop --project /Users/studio/zam dispatcher    # zam's dispatcher
bop status                                     # still works: uses cwd
```

`--project <path>` sets the cards root to `<path>/.cards/`. All subsequent
path resolution uses this root. Equivalent to running bop from `<path>` but
without changing the shell's cwd.

## Project registry (`~/.bop/projects.json`)

```json
[
  {"name": "bop",  "path": "/Users/studio/bop",  "alias": "b"},
  {"name": "efi",  "path": "/Users/studio/efi",  "alias": "e"},
  {"name": "zam",  "path": "/Users/studio/zam",  "alias": "z"}
]
```

With registry, use alias instead of path:
```sh
bop -p b status        # same as --project /Users/studio/bop
bop -p e dispatcher    # same as --project /Users/studio/efi
```

## Commands

```sh
bop project add /Users/studio/efi --alias e    # register a project
bop project list                                # show registry
bop project remove efi                          # unregister
```

## `bop watch` multi-project

`bop watch --all` shows running cards across all registered projects,
prefixed with the project name:
```
  ● [bop] team-arch/spec-056    codex    running   1m 04s
  ● [efi] wave-2/spec-034       claude   running   3m 22s
```

## Implementation

- Add `--project` / `-p` to the root `bop` CLI args (before subcommand)
- Thread `cards_dir: PathBuf` through all subcommands (currently many assume `PathBuf::from(".cards")`)
- `find_project(alias_or_path)` resolves alias via registry, falls back to raw path
- Registry stored at `~/.bop/projects.json` (created on first `bop project add`)

## Acceptance Criteria

- [ ] `bop --project /path status` shows cards from that path's `.cards/`
- [ ] `bop -p <alias> status` resolves alias from registry
- [ ] `bop project add/list/remove` manages `~/.bop/projects.json`
- [ ] `bop watch --all` shows cards from all registered projects
- [ ] No `--project` flag → existing behaviour unchanged (cwd-relative)
- [ ] `cargo clippy -- -D warnings` clean
- [ ] `cargo test` passes

## Files

- `crates/bop-cli/src/main.rs` — add --project global flag, thread cards_dir
- `crates/bop-cli/src/project.rs` — new: registry read/write, alias resolution
- `crates/bop-cli/src/watch.rs` — add --all multi-project support
- All subcommand modules: accept `cards_dir: PathBuf` instead of hardcoding
