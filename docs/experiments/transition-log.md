# Experiment: immutable transition log + deterministic projection (bop#9)

Status: prototype, shadow mode only. Directory state stays authoritative.

## What was built

- `crates/bop-core/src/translog.rs`: per-card append-only log at
  `<card>.bop/logs/transitions.bin`, canonical v1 record encoding, a pure reducer
  `apply(prev, record) -> view`, crash-boundary recovery, and `verify` against
  the directory state.
- Shadow writes from the dispatcher, the merge gate and `bop retry`, enabled with
  `BOP_TRANSLOG=1`. Every write happens *after* the directory `rename`, so the
  existing state machine behaves the same as before.
- `bop translog show <id>` / `bop translog verify <id>|--all`: JSON output
  (`bop.translog.view.v1`, `bop.translog.verify.v1`,
  `bop.translog.verify_all.v1`). `verify` exits 1 when the log and directory
  diverge or the log has a torn tail.

## Record (canonical v1, 154-byte body, 190-byte frame)

Fixed little-endian layout: magic `BOPT`, version, op, from, to, epoch, tid,
parent_tid, attempt, request_id (16 B), object_id (32 B), content_hash (32 B),
prev_hash (32 B). Frame = `u32 len | body | blake3(body)`. The field set follows
`scaffolds/durable-tid/docs/DESIGN.md` (epoch, tid, request_id, object_id, op,
parent_tid, content_hash). A golden hash test pins the encoding; a layout
change must bump `RECORD_VERSION`.

How the requirements map to the code:

| #9 requirement | Mechanism | Test |
|---|---|---|
| append-only committed records | single `write_all` at the committed offset + `sync_data`; nothing is rewritten except truncating a torn tail | `torn_tail_is_ignored_then_truncated_with_epoch_bump` |
| retry keeps request identity | `request_id` derived once from card id + creation stamp; the reducer rejects any record that changes it | `retry_preserves_request_identity`, harness `dispatcher_shadow_translog_matches_directory_state` |
| path/status is a projection | the log moves with the bundle; `verify` compares the replayed state with the parent directory | `verify_flags_divergence_between_directory_and_log` |
| deterministic reducer | no clocks, no I/O in `apply`; the op is derived from `(from, to)` | `replay_is_deterministic` (byte-identical logs, equal views) |
| replay after crash gives the same state | recovery stops at the first frame that fails length, checksum, decode or chain checks; the next commit truncates it and bumps `epoch` | `torn_tail_*`, `corrupted_middle_frame_*`, `forged_frame_*` |
| shared canonical encoding | durable-tid field set, fixed widths, explicit version | `encoding_is_fixed_width_and_roundtrips` |

## Measurements (one card: create, claim, complete, retry, claim, complete)

| | current `events.jsonl` | transition log |
|---|---|---|
| bytes per transition | ~106 (JSON, varies) | 190 (fixed) |
| transitions captured | 4 of 6 (`bop retry` writes no event, so the done->pending retry is missing) | 6 of 6 |
| identity on each record | none (path + timestamps only) | object_id + request_id |
| order | wall-clock `ts` | `tid` + `parent_tid` + hash chain |
| torn/partial write detection | none (a partial JSON line is dropped silently or breaks parsers) | per-frame checksum + chain |
| lineage reconstruction | partial; depends on timestamps | full, from facts alone |
| attempt count | `meta.retry_count`, a mutable counter that is rewritten | number of `Claim` facts (derived) |

Code size: about 650 non-blank, non-comment lines for the core module (codec,
reducer, recovery, commit, shadow bridge, verify), plus about 120 for the CLI.

## Findings

1. **Recovery is better defined.** A crash between the directory `rename` and
   the log append leaves the log one transition behind. `verify` reports this
   (`dir_state` running, `log_state` pending) and nothing is invented. A
   write-ahead order (commit to the log, then rename) would make the log
   authoritative. That is the next step if this experiment is promoted; it is
   out of scope for shadow mode.
2. **Mutable shadow state that could go away:** `meta.retry_count` (derivable
   from `Claim` count), and the `stage_transition` lines in `events.jsonl`
   (a strict subset of the log, with no identity).
3. **Order source (C5):** `tid` is the log's own sequence. Per #7, a backend
   with a native TID/checkpoint (HAMMER, LFS) should supply it through the
   `commit_transition` interface. Only `commit()` would change; the reducer
   and record layout would not.
4. **Legacy cards** with no log are bootstrapped with a `Create` followed by
   the shortest legal path to their current `from` state. These bridge
   records cannot be told apart from observed transitions, so a later version
   should add an explicit `Import` op.
5. **Single writer:** the dispatcher lock and the merge gate's done-only scope
   give one writer per card in practice. The log takes no lock of its own.

## Follow-ups

- #8: the benchmark workload can reuse `recover_bytes` and `project` as its
  replay oracle ("same ordered facts, same state" across backends).
- #7: put `commit()` behind the backend trait so the filesystem provides the TID.
