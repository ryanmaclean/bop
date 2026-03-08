# Event-driven merge-gate via launchd / systemd.path

## Problem

A polling merge-gate daemon burns cycles whether cards are arriving or not.
At scale (many developers, CI environments, shared build clusters) idle polling
is waste that compounds. The OS already knows when files change.

## TRIZ solution — Principle #13 Inversion + #25 Self-service

Don't make bop watch `done/` → make the OS call `bop merge-gate --once` when
`done/` changes, then exit. Zero idle cost. The binary is purely reactive.

## What to build

### 1. macOS: launchd plist

Create `install/macos/sh.bop.merge-gate.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>sh.bop.merge-gate</string>

  <key>ProgramArguments</key>
  <array>
    <string>/usr/local/bin/bop</string>
    <string>merge-gate</string>
    <string>--once</string>
  </array>

  <!-- Trigger on any change inside .cards/*/done/ -->
  <key>WatchPaths</key>
  <array>
    <string>CARDS_DIR_PLACEHOLDER</string>
  </array>

  <!-- Throttle: don't re-trigger more than once per 2s -->
  <key>ThrottleInterval</key>
  <integer>2</integer>

  <!-- Do not keep alive — exit after --once, launchd re-arms -->
  <key>KeepAlive</key>
  <false/>

  <key>StandardOutPath</key>
  <string>/tmp/bop-merge-gate.log</string>
  <key>StandardErrorPath</key>
  <string>/tmp/bop-merge-gate.err</string>
</dict>
</plist>
```

### 2. `bop install-hooks` command

Add `bop install-hooks [--uninstall]` to `crates/bop-cli/src/main.rs`:

- Detects platform (`cfg!(target_os = "macos")` vs Linux)
- **macOS**: writes the plist to `~/Library/LaunchAgents/sh.bop.merge-gate.plist`
  substituting the real `.cards/` path, then runs `launchctl load`
- **Linux**: writes `~/.config/systemd/user/bop-merge-gate.path` +
  `bop-merge-gate.service`, runs `systemctl --user enable --now bop-merge-gate.path`
- **`--uninstall`**: unloads and removes both files

### 3. Linux: systemd path unit

`install/linux/bop-merge-gate.path`:
```ini
[Unit]
Description=bop merge-gate trigger

[Path]
PathChanged=%h/.cards/done
PathChanged=%h/.cards/team-arch/done
PathChanged=%h/.cards/team-cli/done
PathChanged=%h/.cards/team-quality/done
PathChanged=%h/.cards/team-intelligence/done
PathChanged=%h/.cards/team-platform/done

[Install]
WantedBy=default.target
```

`install/linux/bop-merge-gate.service`:
```ini
[Unit]
Description=bop merge-gate (one-shot)

[Service]
Type=oneshot
ExecStart=/usr/local/bin/bop merge-gate --once
```

## Key properties

| Property | Value |
|----------|-------|
| Idle CPU | 0% (process does not exist when idle) |
| Latency | <100ms (OS delivers FSEvents/inotify within milliseconds) |
| Daemon count | 0 (launchd/systemd are already running) |
| Scales to N devs | Yes — each dev's launchd/systemd handles their own checkout |

## Steps

1. Create `install/macos/` and `install/linux/` directories
2. Write the plist and systemd unit files
3. Add `InstallHooks` variant to `Command` enum in `main.rs`
4. Implement platform detection and file writing in the dispatch arm
5. Test on macOS: `bop install-hooks`, drop a card into `done/`, confirm
   merge-gate fires within 2s without any polling loop running
6. Run `make check`

## Acceptance

`bop install-hooks` exits 0.
A card dropped into `.cards/done/` triggers `bop merge-gate --once` automatically.
No `bop merge-gate --loop` process needed.
`make check` passes.
