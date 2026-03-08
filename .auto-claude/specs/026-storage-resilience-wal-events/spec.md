# Storage resilience: JSONL WAL + checksum

## Context

Real production corruption has been observed from badly-behaved adapters (ollama,
opencode) that write garbage to `meta.json` mid-flight. Spec 022 fixes the primary
crash window with tmp+rename atomicity. This spec adds:

1. An append-only audit log per card (`logs/events.jsonl`) for observability and
   post-mortem debugging — NOT for automated recovery (tmp+rename is the fix)
2. A sha256 checksum in `meta.json` to detect the remaining corruption vector:
   a misbehaving adapter that directly overwrites `meta.json` in the card directory

**What this spec does NOT do (council decision):**
- No automated JSONL replay recovery — that path is undertested and will surface
  as a bug during the exact incident it's meant to prevent. Recovery is `bop recover`
  (spec 022), not automated replay.
- No readonly file permission cycling — operationally hostile (every repair needs
  a chmod), and tmp+rename already prevents the write-window corruption it targets.

## What to do

### 1. JSONL audit log per card (best-effort)

Add `logs/events.jsonl` to every card directory. Append one compact JSON line per
event — O_APPEND is atomic for writes ≤ PIPE_BUF (4096 bytes on Linux/macOS):

```json
{"ts":"2026-03-06T14:23:07Z","event":"meta_written","stage":"running","provider":"claude","pid":12345}
{"ts":"2026-03-06T14:23:09Z","event":"stage_transition","from":"running","to":"done","exit_code":0}
```

Event record size MUST stay under 512 bytes (field names + values). Do not include
full text fields — IDs, stage names, exit codes, timestamps only.

Append with `OpenOptions::append(true).create(true)` — no locking needed.

**Best-effort only**: WAL write failure must never abort a state transition.
Use `let _ = append_event(card_dir, &event);` — log the error at debug level,
continue regardless.

In `write_meta` (`crates/bop-core/src/lib.rs`): after the tmp+rename write (spec 022),
append a `meta_written` event.

In `dispatcher.rs`: append `stage_transition` events at each `fs::rename` call.

### 2. Checksum in `meta.json`

Add a `checksum` field to `Meta`:

```rust
pub checksum: Option<String>,
```

In `write_meta`:
1. Set `meta.checksum = None`
2. Serialize to compact JSON (NOT pretty): `serde_json::to_vec(meta)?`
3. Compute sha256 of those bytes using the `sha2` crate
4. Set `meta.checksum = Some(hex_sha256)`
5. Write the final bytes (via the existing tmp+rename path)

**Canonical form is compact JSON** (`serde_json::to_vec`, not `to_vec_pretty`).
This eliminates whitespace ambiguity between write and verify.

In `read_meta` (`crates/bop-core/src/lib.rs`):
1. Read bytes from disk
2. Deserialize to `Meta`
3. Extract and clone the checksum field, set `parsed.checksum = None`
4. Reserialize with `serde_json::to_vec(&parsed)?`
5. Compute sha256 of those bytes
6. If mismatch: log `[warn] meta.json checksum mismatch on <id>`, return `Err`
7. Caller (dispatcher) handles the `Err` by calling `bop recover` logic

Do NOT attempt automated JSONL replay inside `read_meta`.

Check whether `sha2` and `hex` are already transitive dependencies before adding
them. If not already present, use `blake3` (smaller, faster, already common in
Rust toolchains) instead of `sha2`.

```toml
# Only add if not already transitive:
sha2 = { version = "0.10", optional = false }
hex = "0.4"
# OR if sha2 not present:
blake3 = "1"
```

### 3. Storage format decision document

Write `docs/storage-decision.md` explaining why bop stays on filesystem +
JSONL WAL rather than SQLite or Dolt:

- **Filesystem state machine**: `fs::rename` is atomic; each card directory is
  self-contained and portable; `cp -c` COW cloning works natively
- **SQLite**: ACID transactions but breaks the directory-bundle model; multi-process
  concurrent writes need WAL mode and careful locking; overkill for single-writer cards
- **Dolt**: git history queryability but a full MySQL-compatible server; 4MB binary
  overhead; not appropriate for a CLI tool
- **Decision**: filesystem + JSONL WAL (audit/observability) + sha256 checksum
  (tamper/corruption detection via `read_meta`) + tmp+rename atomicity (spec 022).
  Zero external runtime dependencies beyond the hash crate.

### 4. Run `make check` — must pass.

### 5. Write `output/result.md` documenting the corruption scenarios and mitigations.

## Acceptance

- `logs/events.jsonl` appended to on every meta write and stage transition
- WAL write failure never aborts a state transition (`let _ = append_event(...)`)
- `meta.json` has `checksum` field (sha256/blake3 of compact-JSON content with checksum=None)
- `read_meta` validates checksum and returns `Err` on mismatch (no automated replay)
- `docs/storage-decision.md` exists with format rationale
- `make check` passes
- `output/result.md` exists
