# Cross-project lessons — 2026-09

BOP owns **work identity + filesystem state transitions**. It should stay smaller than Gas Town/Tundra.

## Reuse

- **Moth** already has actors, subagents, file-backed persistence via atomic rename, runlog, and metrics. Reuse or adapt those primitives rather than adding parallel implementations.
- **smolFire** provides the microVM execution substrate; BOP should not become an OS.
- **agent-jail** provides the jail isolation alternative.
- **Genoa** owns image/deployment receipts, not BOP.
- **skills/quota-gate** is the deterministic baseline for provider-routing work before adding Jev/System One.
- **Gas Town** is a useful control: it uses Git/worktrees/Beads/SQLite/mailboxes. BOP should prove how much of that machinery filesystem-native state can remove.

## Storage/versioning

Use the smallest backend contract needed to bind a run UUID to a filesystem-native version identity (HAMMER TID, LFS checkpoint, HAMMER2 snapshot/PFS, FFS snapshot). Do not create another canonical lineage database.

OpenLineage is an export view.

## Agent assignment

Copilot is the normal issue assignee. When Copilot credits are constrained, use `@codex` delegation on the relevant issue/PR.
