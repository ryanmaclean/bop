# Spec 063 — bop costs: budget alerts + spend dashboard

## Status: DONE

## What was implemented

### New module: `crates/bop-cli/src/costs.rs`

- `CostsBy` enum — `Provider` (default) | `Day` | `Card`
- `cmd_costs(root, by, budget_set, json)` — public entry point
- Card cost collection from `meta.json` (same data source as `bop stats`)
- `by_provider()`, `by_day()`, `by_card()` aggregation helpers
- Budget config persisted to `<cards_root>/.bop/config.json` as `{"budget": {"daily_usd": N}}`
  - Merge-safe: reads existing JSON, updates only the `budget.daily_usd` key
- Daily budget check: prints `[warn] Daily budget $N exceeded — $M spent today` when over

### Wired into `crates/bop-cli/src/main.rs`

```
mod costs;
```

New `Command::Costs` variant:
```
bop costs                     # summary by provider (default)
bop costs --by day            # daily breakdown (7-day window)
bop costs --by card           # per-card spend, sorted by cost desc
bop costs --budget 5.00       # persist daily budget, print confirmation
bop costs --json              # machine-readable JSON output
```

## Acceptance criteria

- [x] `bop costs` shows spend table by provider
- [x] `bop costs --by day` shows daily totals (last 7 days)
- [x] `bop costs --budget 5.00` persists and warns when exceeded
- [x] `bop costs --json` emits valid JSON
- [x] `cargo clippy -- -D warnings` clean
- [x] `cargo test` passes (8 new costs tests + all existing tests pass)

## Tests added (all passing)

- `costs_collect_basic` — reads done + merged cards
- `costs_by_provider_groups_correctly` — aggregates by provider
- `costs_by_day_last_seven` — 7-day window bucketing
- `costs_by_card_sorted_by_cost_desc` — card list sorted descending
- `budget_persist_and_read` — round-trip budget value
- `budget_merge_preserves_existing_keys` — doesn't clobber other keys in config
- `costs_json_output_valid` — JSON mode runs without error
- `budget_warning_text_shown_when_exceeded` — exceeded flag logic verified
