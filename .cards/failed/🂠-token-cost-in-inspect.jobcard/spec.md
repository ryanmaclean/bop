# Show token cost and usage in `bop inspect`

## Problem
After a card runs, you can't see how many tokens it used or what it cost without
manually parsing `logs/stdout.log` JSON.

## Change

In `bop inspect <id>`, after showing the card state, add a "Cost" section if
`logs/stdout.log` exists and contains a JSON result with `usage` and `total_cost_usd`.

Parse the fields and display:
```
Cost  $0.73  |  cache_read 623k  cache_create 55k  output 2.7k  |  13 turns
```

## Implementation

In `crates/jc/src/main.rs`, in the `Commands::Inspect` handler:
1. Check if `<card_dir>/logs/stdout.log` exists
2. Try to parse it as JSON: `serde_json::from_str::<serde_json::Value>(&contents)`
3. If successful and `usage` key present, extract:
   - `usage.cache_read_input_tokens`
   - `usage.cache_creation_input_tokens`
   - `usage.output_tokens`
   - `total_cost_usd`
   - `num_turns`
4. Print as a single compact "Cost" line

Use `serde_json` (already a dep). No new dependencies.

## Acceptance Criteria
- `cargo build`
- `cargo clippy -- -D warnings`
- `./target/debug/bop inspect short-cli-flags 2>&1 | grep -qi 'cost'`
- `jj log -r 'main..@-' | grep -q .`
