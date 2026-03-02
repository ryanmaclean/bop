# ql-interactive — Interactive Quick Look Preview

## Goal

Make the Stop and Approve buttons in `macos/gtfs/PreviewViewController.swift`
actually work by bridging QL sandbox → JobCardHost via a custom URL scheme.

## Architecture

```
[QL Preview: Stop button clicked]
    ↓ NSWorkspace.shared.open(URL("jobcard://stop/CARD-ID")!)
[macOS routes jobcard:// to JobCardHost.app]
    ↓ onOpenURL receives "jobcard://stop/CARD-ID"
[JobCardHost runs: Process → jc kill CARD-ID]
```

## Why this works

- `NSWorkspace.shared.open(url:)` is permitted in QL sandboxed extensions
- `JobCardHost.app` does NOT need to be sandboxed (it's a local helper, not App Store)
- Unsandboxed apps can exec subprocesses via `Process()`

## Files to change

### 1. `macos/JobCardHost/JobCardHostApp.swift`

Register URL handling. Parse `jobcard://ACTION/CARD-ID` and run:
- `stop`    → `jc kill CARD-ID`
- `approve` → `jc approve CARD-ID`

Use `Process()` to find `jc` binary at a configurable path (default: `~/.local/bin/jc`
falling back to `./target/debug/jc` relative to the app bundle's parent).

### 2. `macos/JobCardHost/Info.plist`

Add `CFBundleURLTypes` entry:
```xml
<key>CFBundleURLTypes</key>
<array>
  <dict>
    <key>CFBundleURLName</key>
    <string>com.yourorg.jobcard</string>
    <key>CFBundleURLSchemes</key>
    <array><string>jobcard</string></array>
  </dict>
</array>
```

### 3. `macos/JobCardHost/JobCardHost.entitlements`

Remove `com.apple.security.app-sandbox` (set to `false` or remove the key).
JobCardHost is a local helper — it must NOT be sandboxed so it can exec jc.

### 4. `macos/gtfs/PreviewViewController.swift`

The Stop button already has the correct UI. Add tap action:
```swift
.onTapGesture {
    if let id = meta?.id {
        NSWorkspace.shared.open(URL(string: "jobcard://stop/\(id)")!)
    }
}
```

Add Approve button in footer when `meta.decision_required == true`:
```swift
if m.decisionRequired == true {
    HStack(spacing: 4) {
        Image(systemName: "checkmark.circle.fill")
        Text("Approve")
    }
    .foregroundColor(.green)
    // ... pill styling matching Stop button
    .onTapGesture {
        NSWorkspace.shared.open(URL(string: "jobcard://approve/\(m.id)")!)
    }
}
```

Add `decisionRequired` to `JobCardMeta` codable:
```swift
let decisionRequired: Bool?
enum CodingKeys: ... {
    case decisionRequired = "decision_required"
}
```

## jc commands to support

- `jc kill <id>` — send SIGTERM to the agent process in `logs/pid`
  (this may already exist; check `main.rs`)
- `jc approve <id>` — clear `decision_required` flag and unblock the card
  (needs implementing; sets `meta.decision_required = false`, moves to running/ if blocked)

## Out of scope

- QL thumbnail generation (separate card)
- Poker submit from QL (future card)
- Any REST or IPC approach (keep it simple: URL scheme + exec)
