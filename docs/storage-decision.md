# bop Storage Architecture Decision

This document explains why `bop` uses a filesystem state machine with JSONL event logs instead of a traditional database.

## Decision Summary

**Storage model:** Filesystem state machine + JSONL append-only event log per card.

- `fs::rename` = atomic state transition
- `logs/events.jsonl` = write-ahead log, O_APPEND atomic for <4096 byte lines
- `meta.json` = materialized view; sha256 checksum field detects corruption
- `read_meta` falls back to JSONL replay on checksum mismatch
- `meta.json` set readonly between transitions; writable only during dispatcher writes

**Alternatives explicitly rejected:** SQLite, Dolt, sled, redb, or any embedded database.

## Design Constraints

`bop` is a local-first, single-machine kanban engine with these requirements:

1. **Human-inspectable state:** Any developer should be able to debug card state with `ls`, `cat`, `grep`.
2. **Tool-agnostic format:** Cards are directory bundles that work with standard Unix tools, git, Quick Look, etc.
3. **Zero mandatory daemon:** Reading and writing cards requires no background service.
4. **Atomic state transitions:** State changes must be all-or-nothing with crash safety.
5. **Portable across POSIX:** Works on macOS, Linux, BSD without platform-specific storage layers.

## Why Filesystem State Machine

### Atomic Transitions via `fs::rename`

POSIX `rename(2)` provides the atomic transaction primitive we need:

```rust
// Atomic state transition
fs::rename(
    ".cards/pending/card123.card",
    ".cards/running/card123.card"
)?;
```

Properties:

- **Atomicity:** The card is either in `pending/` or `running/`, never both, never neither.
- **Durability:** After `rename` returns, the new state is visible to all readers.
- **Crash safety:** If the process crashes mid-transition, the card remains in the source state.
- **No lock files:** POSIX guarantees rename atomicity without external coordination.

This is the same technique used by:

- Email delivery (Maildir format: `tmp/` → `new/` → `cur/`)
- Git object storage (write to temp, rename to final SHA path)
- systemd (atomic unit file replacement)

### State is the Path

The state machine is encoded as directory structure:

```text
.cards/
  pending/
  running/
  done/
  failed/
  merged/
```

Benefits:

- **Self-documenting:** `ls .cards/running/` shows all running cards instantly.
- **Queryable with standard tools:** `find .cards/done -mtime -1` for recent completions.
- **No stale index:** The filesystem IS the index; no sync lag, no cache invalidation.
- **Concurrent-safe:** Multiple processes can observe state without coordination.

### Comparison to Database Transactions

| Requirement | `fs::rename` | SQLite `BEGIN…COMMIT` |
|-------------|--------------|----------------------|
| Atomicity | ✅ Kernel guarantee | ✅ WAL + journal |
| Durability | ✅ Immediate visibility | ⚠️ Requires `fsync` tuning |
| Tool access | ✅ Any file tool | ❌ Requires sqlite3 CLI |
| Cross-process | ✅ No locking needed | ⚠️ Requires `PRAGMA busy_timeout` |
| Crash recovery | ✅ Automatic | ⚠️ WAL replay on reopen |
| Human inspection | ✅ `ls`, `cat` | ❌ Binary format |

For `bop`, the filesystem provides sufficient ACID properties without the complexity of a database engine.

## Why JSONL Event Log

### Append-Only Write-Ahead Log

Each card has `logs/events.jsonl`:

```jsonl
{"ts":"2026-03-07T10:00:00Z","event":"created","by":"bop new"}
{"ts":"2026-03-07T10:01:00Z","event":"transition","from":"pending","to":"running"}
{"ts":"2026-03-07T10:05:00Z","event":"completed","exit_code":0}
```

Properties:

- **Append-only:** New events are written with O_APPEND flag (atomic for <4096 bytes on POSIX).
- **Crash-safe:** Partial writes are detected (incomplete JSON line); replay stops at last valid event.
- **Human-readable:** Any text editor or `tail -f` can observe events.
- **Portable:** JSONL is a standard format with libraries in every language.

### Materialized View in `meta.json`

The dispatcher maintains `meta.json` as a computed view:

```json
{
  "id": "card123",
  "state": "done",
  "created_at": "2026-03-07T10:00:00Z",
  "completed_at": "2026-03-07T10:05:00Z",
  "sha256": "abc123..."
}
```

Checksum field ensures integrity:

1. Dispatcher computes SHA-256 of canonical JSON (excluding checksum field).
2. On read, if checksum mismatches, replay `events.jsonl` to rebuild state.
3. Detects corruption, partial writes, or manual edits without auxiliary validation.

### Why Not SQLite WAL?

SQLite provides robust transactional guarantees, but introduces complexity:

- **Binary format:** Requires sqlite3 tool or library to inspect.
- **Schema migrations:** Adding fields requires `ALTER TABLE` and migration logic.
- **Exclusive locking:** Write transactions block readers (even with WAL mode).
- **fsync tuning:** Durability requires careful `PRAGMA synchronous` settings.
- **Cross-platform quirks:** NFS, FUSE, and some filesystems have locking issues.

For `bop`, these costs outweigh the benefits. We're not building a relational query engine; we're tracking card state transitions.

## Why Not Dolt?

Dolt is a SQL database with Git-like versioning. Compelling for:

- Branch/merge workflows on tabular data.
- Distributed collaboration with conflict resolution.
- SQL query interface.

But `bop` already has Git (or Jujutsu) at the repository level. Adding a second versioning layer:

- **Duplicates functionality:** Card bundles are already in git; Dolt would version the same transitions again.
- **Daemon requirement:** Dolt server or CLI wraps every access.
- **Opaque storage:** `.dolt/` directory is not human-readable like `.cards/`.
- **Schema lock-in:** Changes to card metadata require Dolt schema migrations.

Dolt solves a different problem (versioned SQL queries on tabular datasets). `bop` is a state machine for directory bundles.

## Portability and Zero-Daemon Philosophy

### Cross-Platform Filesystem Semantics

Filesystem operations are portable POSIX:

- `rename(2)` is atomic on Linux, macOS, BSD, WSL.
- JSONL is plain text; works on any platform with `\n` line endings.
- No platform-specific storage layer (e.g., macOS-only Core Data, Windows-only ESE).

### No Background Service Required

Database engines often require:

- **SQLite:** No daemon, but requires library linking and binary schema.
- **PostgreSQL/MySQL:** Daemon, network port, credential management.
- **Dolt:** Daemon or CLI wrapper for all access.

`bop` has zero storage daemon:

- File operations are direct syscalls.
- Any tool can read cards: `cat`, `jq`, `grep`, Quick Look, `nvim`.
- Debugging requires no special client.

This aligns with Unix philosophy: plain text, composable tools, no hidden state.

## When You WOULD Want a Database

This decision is specific to `bop`'s constraints. Consider a database if:

1. **Complex queries:** JOIN multiple entities, aggregate across thousands of cards, full-text search.
2. **Fine-grained concurrency:** Many writers updating the same entity concurrently.
3. **Transactional guarantees across multiple entities:** ACID across card + user + project in one transaction.
4. **Network storage:** Cards stored on NFS, SMB, or distributed filesystem (where `rename` may not be atomic).
5. **Audit trails:** Database-level logging, rollback, point-in-time recovery.

`bop` intentionally trades these advanced features for simplicity and transparency.

## Scaling Considerations

Expected baseline:

- **10,000 cards:** Single directory with 10k entries is fine on modern filesystems.
- **100,000+ cards:** Partition by date/team (e.g., `.cards/2026-03/team-arch/pending/`).

Modern filesystems (APFS, ext4, XFS, Btrfs) handle large directories well:

- APFS: B-tree indexed directories, millions of entries supported.
- ext4: htree indexing, ~10M files per directory.

If `bop` reaches scale where filesystem state machine becomes a bottleneck, that's a good problem—it means wide adoption. At that point, consider:

- **Read-only query layer:** Build a secondary index in SQLite/PostgreSQL from JSONL events (write path stays filesystem).
- **Sharding:** Multiple `bop` instances per team, federated via shared format.
- **Time-series database:** If analytics on event logs become the primary use case.

But don't prematurely optimize. The filesystem state machine works for the 99% use case.

## Related Designs

Other systems that chose filesystem over database for similar reasons:

- **Maildir:** Email storage format (tmp/ → new/ → cur/).
- **Git:** Object store as content-addressed files, not a database.
- **systemd units:** `/etc/systemd/system/*.service` files, not a config database.
- **Kubernetes manifests:** YAML files, not a relational schema.
- **Nix store:** `/nix/store/` as immutable content-addressed filesystem.

These all share:

- Human-readable formats (text files).
- Atomic operations (rename, symlink swap).
- No mandatory daemon (though helpers exist).
- Tool-agnostic access.

`bop` follows this tradition.

## Summary

`bop` uses filesystem state machine + JSONL event log because:

1. **`fs::rename` is the atomic transaction** we need for state transitions.
2. **JSONL append-only log** provides crash-safe event history.
3. **No database daemon** means zero operational complexity.
4. **Human-readable format** makes debugging trivial.
5. **Portable POSIX semantics** work everywhere without platform-specific storage.

We explicitly reject SQLite, Dolt, and other databases because they solve problems `bop` doesn't have (complex queries, fine-grained concurrency, network storage) while introducing complexity that conflicts with our design goals (binary formats, schema migrations, daemon management).

This is a closed decision. If you need database features, build a secondary index on top of the JSONL event log—don't replace the filesystem state machine.
