# make check clean: fix clippy + finish install-hooks Linux

## Context

A previous agent added `bop install-hooks` with macOS launchd and Linux systemd support
(`crates/bop-cli/src/factory.rs`). There are clippy warnings and the Linux systemctl
installation logic (`install_hooks_linux`) may be incomplete.

## What to do

1. Run `make check` and fix any clippy errors or warnings.
2. Read `crates/bop-cli/src/factory.rs` — find `install_hooks_linux` and verify it:
   - Writes `.service` and `.path` files to `~/.config/systemd/user/`
   - Runs `systemctl --user daemon-reload`
   - Runs `systemctl --user enable` + `start` for both units
   - Has `--uninstall` path that stops/disables/removes both units
3. If any of the above is missing, add it.
4. Run `make check` again — must pass with zero warnings.
5. Write `output/result.md` summarising what was fixed.

## Acceptance

- `make check` exits 0 (test + clippy + fmt)
- `bop install-hooks --help` shows usage
- `output/result.md` exists with summary
