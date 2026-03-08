# Spec 037 — merge-gate launchd install

## Overview

`bop factory install` currently installs only the dispatcher launchd agent
(`sh.bop.dispatcher`). The merge-gate service (`sh.bop.merge-gate`) has a template
at `install/macos/sh.bop.merge-gate.plist` but is never installed automatically.

`bop factory status` shows `merge-gate sh.bop.merge-gate: □ not installed`.

This spec makes `bop factory install` also install and bootstrap the merge-gate agent.

## What to implement

### `factory.rs` — extend `cmd_factory_install_macos`

`FACTORY_LABELS` already includes `("sh.bop.merge-gate", "merge-gate")`. The
`generate_plist` function already handles `subcommand == "merge-gate"` by setting
`watch_subdir = "done"`.

The gap: `cmd_factory_install_macos` (line ~324) currently only installs the dispatcher.
Extend it to iterate `FACTORY_LABELS` and install both:

```rust
for (label, subcommand) in FACTORY_LABELS {
    let plist = generate_plist(label, subcommand, repo_root);
    let path = plist_path(label);
    fs::write(&path, plist)?;
    // bootstrap via launchctl
    let _ = StdCommand::new("launchctl")
        .args(["bootstrap", &format!("gui/{}", get_uid()), path.to_str().unwrap()])
        .status();
}
```

### `factory.rs` — `cmd_factory_install_linux`

Similarly extend for systemd: create `.service` + `.path` units for merge-gate
alongside the existing dispatcher units. Use `done/` as the `PathChanged` directory.

### `cmd_factory_uninstall`

Already iterates `FACTORY_LABELS` — should work for merge-gate once it's installed.
Verify: run `bop factory uninstall` after install and confirm both agents are removed.

### `bop factory status` — merge-gate entry

After install, `bop factory status` must show:
```
merge-gate sh.bop.merge-gate: ● running (pid XXXX)
  stdout: /tmp/bop-merge-gate.log
  stderr: /tmp/bop-merge-gate.err
```

## WatchPaths for merge-gate

`generate_plist` with `subcommand = "merge-gate"` already sets `watch_subdir = "done"`,
producing paths like:
```
/Users/studio/bop/.cards/done
/Users/studio/bop/.cards/team-arch/done
...
```

This is correct — launchd fires `bop merge-gate` whenever any card appears in `done/`.

## Acceptance Criteria

- [ ] `bop factory install` installs both `sh.bop.dispatcher` and `sh.bop.merge-gate`
- [ ] `bop factory status` shows merge-gate as ● active or ● running after install
- [ ] `bop factory uninstall` removes both agents cleanly
- [ ] `bop factory stop` stops both; `bop factory start` restarts both
- [ ] `cargo test` passes; `cargo clippy -- -D warnings` clean
- [ ] On Linux: both `.service` + `.path` units created for merge-gate

## Files to modify

- `crates/bop-cli/src/factory.rs` — extend install/uninstall to cover merge-gate
