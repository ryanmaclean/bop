# Spec 060 — bop benchmark: compare providers on same spec

## Overview

`bop benchmark` runs the same spec through 2+ providers and produces a
side-by-side comparison of cost, speed, token usage, and output quality.
Useful for calibrating provider choices and validating that cheaper providers
produce acceptable output.

## Command

```sh
bop benchmark <spec-file> --providers codex,claude,ollama-local
bop benchmark <spec-file> --providers codex,claude  --runs 3   # 3 runs each
bop benchmark <spec-file> --providers codex,ollama-local --judge claude
```

## How it works

1. For each provider in `--providers`:
   - COW-clone a fresh `.card/` from a temp template
   - Set `provider_chain: [<provider>]` in meta.json
   - Dispatch the card (using the existing adapter directly, not the full dispatcher)
   - Capture: wall-clock time, exit code, tokens, cost, output/result.md
2. After all runs: render comparison table
3. If `--judge <provider>`: send all outputs to that provider for qualitative scoring

## Output

```
bop benchmark results — my-spec.md
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  Provider       Time      Tokens    Cost     Exit  Score
  ───────────────────────────────────────────────────────
  codex          4m 51s    14,200    $0.18    0     8.2/10
  claude         6m 12s    18,400    $0.31    0     9.1/10
  ollama-local   2m 03s    9,800     $0.00    0     6.4/10
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  Recommendation: codex  (best cost/quality ratio: $0.022/point)
```

Score from `--judge` is parsed from the judge's output (ask it to respond
with a JSON object `{"scores": {"codex": 8.2, "claude": 9.1, ...}}`).

## `bop benchmark --json`

```json
{
  "spec": "my-spec.md",
  "runs": [
    {"provider": "codex", "duration_secs": 291, "tokens": 14200, "cost_usd": 0.18, "exit": 0, "score": 8.2},
    ...
  ],
  "recommendation": "codex"
}
```

## Acceptance Criteria

- [ ] `bop benchmark <spec> --providers codex,claude` runs both and shows table
- [ ] Each provider run is isolated (independent COW card clones)
- [ ] `--judge` sends outputs to judge provider and parses score
- [ ] `--json` emits valid JSON
- [ ] `--runs N` runs each provider N times and averages results
- [ ] Handles provider failure gracefully (marks run as failed, continues others)
- [ ] Results saved to `bop-benchmark-<timestamp>.json` in cwd
- [ ] `cargo clippy -- -D warnings` clean
- [ ] `cargo test` passes

## Files

- `crates/bop-cli/src/benchmark.rs` — new module
- `crates/bop-cli/src/main.rs` — wire `bop benchmark` subcommand
