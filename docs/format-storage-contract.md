# bop Format and Storage Contract

This document defines the on-disk contract for `.jobcard` bundles and the storage behavior expected by `bop`.

## Design Principles

- Format-first, not database-first.
- Filesystem-native state machine.
- Human-readable and tool-agnostic bundles.
- Zero mandatory daemon for read/write access.

`bop` is a reference engine for this format. Any tool that can create directories and write JSON/Markdown can participate.

## Card Format Contract

A card is a directory bundle:

```text
<id>.jobcard/
  meta.json
  spec.md
  prompt.md
  logs/
  output/
```

Required semantics:

- `meta.json` is authoritative machine state.
- `spec.md` and `prompt.md` are plain text, editable by humans and agents.
- Runtime artifacts live under `logs/` and `output/`.

## State Machine Contract

The state machine is the path:

```text
pending/ -> running/ -> done/ -> merged/
                    \-> failed/
```

Transitions must be atomic rename/move operations on the same volume.

## Copy/Clone Contract

Template instantiation should prefer copy-on-write clone semantics:

- macOS/APFS: `ditto --clone` (or `cp -c` fallback)
- Linux: reflink when available (`cp --reflink=auto`)

This preserves low overhead for repeated template structure and shared text.

## Compression Contract (macOS)

Terminal-state cards (`done`, `failed`, `merged`) may be compressed in place using:

- `ditto --hfsCompression`

Behavior requirements:

- Transparent: consumers read files normally (`cat`, Quick Look, `bop logs`).
- Idempotent: repeated compression attempts are safe.
- Atomic: write to temp bundle, swap via rename, remove backup on success.
- No clone+compress in one step: compression path should not use `--clone`.

Non-macOS platforms must safely no-op unless explicitly configured otherwise.

## Dedupe Expectations

APFS clone-on-write already gives dedupe-like behavior for cloned templates:

- Shared physical blocks until write divergence.
- No extra dedupe daemon required.

This is a storage implementation detail, not a format guarantee.

## Portability and Interop

The format must remain plain-files portable:

- No mandatory binary index.
- No opaque write-ahead log.
- No compaction/GC cycle required for correctness.

Cross-machine transport can use `git`, `rsync`, archive export/import, or similar file tools.

## Scaling Guidance

Expected baseline:

- Thousands to low tens-of-thousands of cards on local SSD/APFS.

When directory fanout becomes large (for example, 100k+ entries), add partitioning by date/team/state while preserving bundle format and atomic transition rules.

## Non-Goals

Avoid introducing database responsibilities into the format layer:

- Embedded DB requirement.
- Custom binary storage format.
- Mandatory global index service.
- Background compactor as a correctness dependency.

If those are required, that is a separate system on top of this format, not a change to the format itself.
