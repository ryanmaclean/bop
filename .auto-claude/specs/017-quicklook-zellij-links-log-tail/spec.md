# Quick Look: Zellij live links + live log tail

## Context

`macos/bop/PreviewViewController.swift` already has `zellijSession` and `zellijPane`
fields in `BopCardMeta` but they're never populated and never rendered as live links.
Zellij 0.43+ ships a web client at `http://127.0.0.1:8082/<session-name>` — a real
bookmarkable URL. Cards in `running/` have stdout/stderr at `logs/stdout` and `logs/stderr`
but the Quick Look preview doesn't show them.

## Goals

1. **Dispatcher populates Zellij fields**: when spawning an adapter, write
   `zellij_session` (current `$ZELLIJ_SESSION_NAME` or `"pmr"` fallback) and
   `zellij_pane` (pane name like `"bop-<card-id>"`) into `meta.json`.

2. **Quick Look renders a live Zellij link**: if `zellijSession` is set, show a
   tappable button/link in the card that opens
   `http://127.0.0.1:8082/<session>` in the default browser via
   `NSWorkspace.shared.open(url)`. Use the Zellij logo glyph or a terminal icon.

3. **Live log tail panel**: if the card is in `running/` state and `logs/stdout`
   exists, show the last 30 lines in a monospace scroll view. Auto-refresh every
   2 seconds using a `Timer` (FSEvents not available in sandbox). Show a "live"
   pulsing indicator while running, static when done/failed.

4. **"Open full log" button**: opens the log file in the system default viewer
   (`NSWorkspace.shared.open`) or runs `tail -f logs/stdout` in a new Zellij pane.

## What to do

1. Read `crates/bop-cli/src/dispatcher.rs` — find where adapters are spawned.
   Write `zellij_session` and `zellij_pane` to `meta.json` at spawn time using
   `write_meta`. Use `std::env::var("ZELLIJ_SESSION_NAME").unwrap_or("pmr")`.

2. Read `macos/bop/PreviewViewController.swift` — find where `zellijSession` is
   read. Add a `ZellijLinkButton` SwiftUI view that:
   - Shows only when `zellijSession` is non-nil
   - Renders as a styled pill: `⬡ Open in Zellij · pmr`
   - Calls `NSWorkspace.shared.open(URL(string: "http://127.0.0.1:8082/\(session)")!)`
     on tap (use `Button` + `NSWorkspace` — Quick Look `.appex` can open URLs)

3. Add a `LiveLogView` SwiftUI component:
   - Reads `cardURL.appendingPathComponent("logs/stdout")` as string
   - Shows last 30 lines in a `ScrollView` > `Text` with `.font(.system(.caption, design: .monospaced))`
   - `TimelineView(.periodic(from: .now, by: 2))` or `Timer.scheduledTimer` for refresh
   - Pulsing green dot when state == "running", grey when done

4. Wire `LiveLogView` into the main card view below the stage timeline. Only show
   if log file exists and has content.

5. Run `xcodebuild -project macos/macos.xcodeproj -scheme JobCardHost -configuration Debug build`
   to verify it compiles.

6. Write `output/result.md` documenting what was added.

## Acceptance

- Dispatcher writes `zellij_session` to `meta.json` on card spawn
- Quick Look card shows Zellij link button when session is set
- Quick Look card shows live log tail (last 30 lines) for running cards
- `xcodebuild` succeeds with no errors
- `output/result.md` exists
