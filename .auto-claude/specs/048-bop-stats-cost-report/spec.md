# Spec 048 — bop stats: historical cost + token report

## Overview

TRIZ P35 (Parameter changes): runtime-configurable detail level for cost data.
`bop stats` reads all cards across all state dirs and produces a cost/token
summary with configurable grouping.

## Commands

```sh
bop stats               # summary: today, this week, all-time
bop stats --by provider # group by provider
bop stats --by day      # group by calendar day
bop stats --json        # machine-readable output
bop stats --card <id>   # single card detail
```

## Data source

Read `meta.json` for every card in `.cards/{done,merged,failed}/`.
Fields used: `cost_usd`, `tokens_used`, `provider`, `started_at`, `finished_at`.

Cards without cost data (cost_usd = null / 0) are counted but not included
in cost totals — shown as "N cards (no cost data)".

## Output format (default)

```
bop stats
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  Cost summary
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  Today          12 cards    $1.24   84K tokens
  This week      47 cards    $6.82   412K tokens
  All time       183 cards   $23.40  1.8M tokens

  By provider (all time)
  codex          91 cards    $14.20  820K tokens
  claude         72 cards    $8.10   910K tokens
  ollama          8 cards    $0.00   —
  gemini         12 cards    $1.10   70K tokens
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  Avg cost/card: $0.13
  Avg tokens:    9,836
```

## JSON output

```json
{
  "summary": {
    "today": {"cards": 12, "cost_usd": 1.24, "tokens": 84000},
    "week":  {"cards": 47, "cost_usd": 6.82, "tokens": 412000},
    "all":   {"cards": 183, "cost_usd": 23.40, "tokens": 1800000}
  },
  "by_provider": [
    {"provider": "codex", "cards": 91, "cost_usd": 14.20, "tokens": 820000}
  ]
}
```

## Acceptance Criteria

- [ ] `bop stats` shows today / week / all-time totals
- [ ] `--by provider` groups by provider name
- [ ] `--by day` shows last 7 days as rows
- [ ] `--json` emits valid JSON (parseable by `jq`)
- [ ] `--card <id>` shows single card cost + token detail
- [ ] Cards with no cost data are counted but excluded from cost totals
- [ ] `cargo clippy -- -D warnings` clean
- [ ] `cargo test` passes (unit test with mock meta.json data)

## Files

- `crates/bop-cli/src/stats.rs` — new module
- `crates/bop-cli/src/main.rs` — wire `bop stats` subcommand
