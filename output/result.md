# Token Cost in Inspect — Done

## What was done

Added a "Cost" summary line to `bop inspect` output.

After the log tail sections, if `logs/stdout.log` exists and contains a JSON
line with `usage` and `total_cost_usd`, it now prints:

```
Cost  $0.73  |  cache_read 623k  cache_create 55k  output 2.7k  |  13 turns
```

## Implementation

Modified `crates/jc/src/main.rs`:

- In `cmd_inspect`: scans `logs/stdout.log` lines in reverse for valid JSON
  with a `usage` key, extracts `cache_read_input_tokens`,
  `cache_creation_input_tokens`, `output_tokens`, `total_cost_usd`,
  `num_turns` and prints them in the compact Cost line.
- Added helper `fn fmt_tokens(n: u64) -> String` that formats token counts:
  - ≥ 10k → `{n}k` (no decimal)
  - 1k–10k → `{n:.1}k` (one decimal)
  - < 1k → raw number

## Acceptance criteria

- `cargo build` ✓
- `cargo clippy -- -D warnings` ✓
- `./target/debug/bop inspect short-cli-flags 2>&1 | grep -qi 'cost'` ✓
- `jj log -r 'main..@-' | grep -q .` ✓

Committed as `lypsnumm` (feat: show token cost in bop inspect).
