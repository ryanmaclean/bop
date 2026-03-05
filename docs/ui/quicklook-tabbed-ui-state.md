# QuickLook Tabbed UI State

## Architecture & Overview
The current QuickLook extension (`macos/bop/PreviewViewController.swift`) is built using SwiftUI embedded in an `NSViewController` via `NSHostingView`. It provides a rich, multi-tab interface for `.bop` bundles.

### Data Model (`meta.json`)
The UI parses `meta.json` from the `.bop` bundle into a `BopMeta` struct. Key fields:
- `id`, `title`, `description`, `stage`, `priority`, `created`
- `labels`: Array of `MetaLabel` (name, kind)
- `progress`: Int percentage
- `subtasks`: Array of `MetaSubtask` (id, title, done)
- `stages`: Dictionary of stages to `MetaStageRecord` (status, agent)
- `glyph`: Emoji/symbol representation
- `acceptanceCriteria`: Array of strings
- `zellijSession`, `zellijPane`: Strings for attaching to terminal

### UI Layout
- **App Background**: `Color.appBg` (Darker outside background)
- **Card Background**: `Color.cardBg` (Inner card)
- **Header**: Glyph, Title, ID pill, current stage, progress text.
- **Progress Bar**: Fills horizontally based on `progress`.
- **Tabs**: 
  - Managed by `enum CardTab: String, CaseIterable { case overview, subtasks, logs, files }`
  - Custom tab bar with underline and `.onHover` pointer cursor.
- **Tab Content**:
  - `Overview`: Labels, Priority, Description, Stage Pipeline, Acceptance Criteria.
  - `Subtasks`: Done/Total count, percentage, list of subtasks with checkmarks.
  - `Logs`: Reads `logs/stdout.log`, displays last 100 lines in a dark terminal-like block.
  - `Files`: Enumerates top-level files in the `.bop` bundle (ignoring `logs`, `output`, `worktree`), displays with system icons.
- **Footer**: Creation time, Stop button (if running).

### Interactivity Requirements
To allow SwiftUI to handle mouse events (like `.onHover` for tabs) inside a QuickLook preview, we use a custom `NSHostingView` subclass that explicitly allows hit testing:
```swift
class EventHostingView<Content: View>: NSHostingView<Content> {
    override func hitTest(_ point: NSPoint) -> NSView? {
        return super.hitTest(point)
    }
}
```

### Distribution & Build Requirements
To distribute a build that others can use:
1. **App Wrapper**: The `BopDeck.app` must be built. It contains the QuickLook extension `.appex`.
2. **Registration**: Users will need to register the app with LaunchServices and pluginkit:
   `/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister -f -R -trusted /Applications/BopDeck.app`
   `pluginkit -e use -i sh.bop.ql`
3. **UTI Definition**: The `Info.plist` defines the `.bop` document type as a package (`com.apple.package`) so macOS treats the directory as a single file.
4. **File Hooks / Interactivity**: 
   - Moving cards around is handled natively by macOS Finder (moving the `.bop` bundle between directories).
   - Planning/Updating is done via editing files inside the bundle (e.g., `meta.json`). QuickLook will render whatever is currently on disk.

## Source Backup
The source is fully backed up in the git history and in `macos/bop/PreviewViewController.swift` as of the latest commit.
