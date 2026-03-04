---
name: bop-apfs-tahoe
description: Use when designing or refactoring bop runtime/storage paths to maximize APFS COW/compression and prefer standard Rust crates over custom infrastructure.
---

# Bop APFS Tahoe

## Purpose

Keep bop as a filesystem-native workflow engine (not a database), while
using proven libraries for common runtime concerns.

Target platform: macOS Tahoe and other APFS systems.

## Core Primitive

Treat the system as three primitives:

1. Job: card bundle (`<id>.bop/`) with spec + metadata.
2. Run: one execution attempt with logs, session info, and outcome.
3. Event: append-only lineage record (`events.jsonl` / OpenLineage payloads).

## Library Defaults

Resolve library choices via `references/rust_runtime_crates.tsv`.

- File locking: `fs4` (or `fs2` if already in tree)
- Retries/backoff: `backoff`
- Logging/telemetry: `tracing` + `tracing-subscriber`
- Table/CLI rendering: `tabled` or `comfy-table`
- Process/system inspection: `sysinfo`

Rule: do not implement a bespoke subsystem if a listed crate already covers it.

## APFS Contract

Resolve filesystem operations via `references/apfs_ops.tsv`.

- Creation path (job/template/work item copy):
  - Prefer `ditto --clone`
  - Fallback `cp -c` on macOS
- Terminal-state compaction (`done|failed|merged`):
  - `ditto --hfsCompression`
- Never add plain `cp -R`/`cp -r` for card copy paths on macOS.
- Compression is transparent and idempotent; treat repeated refresh as safe.

## Workflow

1. Classify the change as `job`, `run`, `event`, or `ui`.
2. Pick crates from `rust_runtime_crates.tsv` before writing code.
3. Pick APFS operation from `apfs_ops.tsv`.
4. Enforce invariants:
   - Atomic state transitions via `rename`
   - Lease/lock protection for running work
   - Append-only lineage events
5. Verify with:
   - `cargo build`
   - `./target/debug/bop doctor`
   - `./target/debug/bop policy check --staged`

## Output Contract

- State which primitive was changed (`job/run/event/ui`).
- State which library choice was applied (or why not).
- State which APFS operation was used.
- Confirm no plain recursive copy was introduced on macOS card paths.
