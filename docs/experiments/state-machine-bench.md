# Common state-machine benchmark workload (bop#8)

`bop translog bench` runs one fixed, seed-derived workload against a scratch
directory on the filesystem under test and prints a single
`ryanlab.bench.v1` record (ryanmaclean/skills `schemas/bench.v1.schema.json`).

```sh
bop translog bench --dir /mnt/under-test/bench --cards 64 --seed 1 \
  --filesystem ffs --runtime microvm --commit "$(git rev-parse --short HEAD)" \
  --out result.json
```

Per card: CREATE, CLAIM (pending to running), APPEND logs, WRITE artifact,
COMPLETE or FAIL (about one card in four fails, chosen from the seed), FSYNC,
CRASH (a torn partial frame is appended to the transition log), RECOVER (the
torn tail must be reported and never treated as committed; the next
transition truncates it and bumps the epoch), QUERY HISTORY and EXPORT LINEAGE.

- The core only uses mkdir, rename, write, fsync and read. It has no
  filesystem-specific code.
- `--version-cmd <exe>` is the backend adapter hook (bop#7). It is called as
  `<exe> <dir>` before and after the run. Its trimmed stdout (HAMMER TID, LFS
  checkpoint, FFS snapshot name, and so on) is reported as the
  `fs_version_before` and `fs_version_after` tags.
- Replay oracle: after the whole tree is written, every log is re-read with
  `translog::recover_bytes` and projected. The result must equal the
  in-memory projection of the committed records and the directory the card
  sits in. `tags.replay_digest` is a blake3 hash over every canonical record
  body and final state. It uses no clocks and no paths, so the same
  `--cards/--seed/--artifact-bytes/--log-lines` must give the same digest on
  every backend. A different digest across backends is a bug.
- The exit status is 1 if any card misses its torn tail or fails replay.
  `metrics.replay_mismatches` and `raw.cards[].replay_ok` show which one.

The standard metrics are `rename_us_p50`, `fsync_us_p50`, `recovery_ms`,
`metadata_bytes_per_run` (committed transition-log bytes per card),
`lineage_reconstruction_ms` and `artifact_bytes`. The extension metrics are
`commit_us_p50/p99`, `transitions`, `torn_tails_detected` and
`replay_mismatches`. Raw per-operation samples are in `raw.samples_us`.
`rss_bytes`, `boot_ms`, `write_amplification` and `history_bytes_per_gib`
are `null`, because they belong to the runtime or backend adapter, not this
workload.
