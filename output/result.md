# Spec 063 Implementation Result

Implemented `bop costs` with event-log spend aggregation, budget persistence, and JSON output.

## What changed

- Added new module: `crates/bop-cli/src/costs.rs`
  - Scans card `logs/events.jsonl` files across card states.
  - Aggregates spend from cost fields (`cost_usd`, `cost`, `costUsd`, and `usage.total_cost_usd`).
  - Supports grouping modes:
    - default: provider totals for the current week
    - `--by day`: daily totals
    - `--by card`: per-card totals
  - Supports machine output with `--json`.
  - Implements budget config read/write at `.cards/.bop/config.json`:
    - `budget.daily_usd` persisted via `bop costs --budget <amount>`
  - Emits warning when today's spend exceeds configured daily budget:
    - `[warn] Daily budget $<limit> exceeded — $<today> spent today`
  - Includes unit tests for parsing, grouping, config persistence, and log collection.

- Updated CLI wiring in `crates/bop-cli/src/main.rs`
  - Registered `mod costs;`
  - Added new subcommand:
    - `bop costs`
    - `bop costs --by day`
    - `bop costs --by card`
    - `bop costs --budget 5.00`
    - `bop costs --json`
  - Routed to `costs::cmd_costs`.

## Validation

- `cargo build` ✅
- `cargo test` ✅
- `cargo clippy -- -D warnings` ✅

All acceptance checks passed.
