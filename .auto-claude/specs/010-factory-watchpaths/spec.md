# Fix bop factory: KeepAlive → WatchPaths

## Problem

`crates/bop-cli/src/factory.rs` installs launchd plists with `KeepAlive: true` —
a persistent polling daemon. This violates the project's TRIZ constraint:
**no polling loops; use OS events**.

`crates/bop-cli/src/icons.rs` already has the correct WatchPaths pattern.
Copy it.

## What to change

In `factory.rs`, the generated plist for both dispatcher and merge-gate services:

**Remove:**
```xml
<key>KeepAlive</key>
<true/>
<key>RunAtLoad</key>
<true/>
```

**Add:**
```xml
<!-- Dispatcher: wake when cards appear in pending/ -->
<key>WatchPaths</key>
<array>
  <string>{cards_dir}/pending</string>
  <!-- team dirs are watched recursively by launchd -->
</array>

<!-- Throttle: don't re-trigger faster than once per second -->
<key>ThrottleInterval</key>
<integer>1</integer>

<!-- Exit after --once; launchd re-arms on next event -->
<key>KeepAlive</key>
<false/>
```

For merge-gate, watch `done/` instead of `pending/`.

Also update `bop factory install` to generate team-dir WatchPaths dynamically
(walk `.cards/` for `team-*/pending` and `team-*/done`).

## Key reference

See `crates/bop-cli/src/icons.rs` lines ~152-190 for the correct pattern already
working in this codebase.

## Steps

1. Read `icons.rs` to understand the existing WatchPaths plist generation
2. Refactor `factory.rs` to use the same approach
3. `bop factory install` → check generated plists contain WatchPaths, not KeepAlive
4. Verify: drop a card into `.cards/pending/` manually, confirm dispatcher fires
   within 2s without a running `bop dispatcher --loop`
5. `make check`

## Acceptance

Generated plists contain `WatchPaths`, not `KeepAlive: true`.
`bop factory status` shows services loaded.
`make check` passes.
