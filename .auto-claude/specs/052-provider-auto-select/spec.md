# Spec 052 — provider auto-selection (TRIZ P35+P10+P3)

## Overview

The dispatcher currently uses `provider_chain[0]` statically. If that provider
is quota-exhausted, it retries until the cooldown clears — wasting dispatch
slots. This spec adds quota-aware, cost-tier-aware provider selection.

## Algorithm (P3 Local quality — non-uniform logic per tier)

Before spawning an adapter, the dispatcher calls `select_provider(meta, providers_json)`:

```
1. Read current quota snapshot from `bop providers --json` (cached, max 60s stale)
2. Filter provider_chain to candidates where:
   a. Not in cooldown (providers.json cooldown_until < now)
   b. Quota utilization < 90% (from snapshot)
3. Cost-tier routing (P3):
   - meta.cost == 1 (trivial)  → prefer ollama-local first if available
   - meta.cost == 2 (small)    → prefer ollama-local or codex
   - meta.cost >= 3 (medium+)  → prefer codex or claude (skip ollama-local)
4. If candidates empty → wait for first cooldown to expire (existing behaviour)
5. Return selected provider name
```

## Quota snapshot cache

Add `ProviderCache` in `providers/mod.rs`:
- Stores the last `Vec<ProviderSnapshot>` with a timestamp
- `get_cached(max_age: Duration)` returns cached value if fresh enough
- `refresh()` runs `bop providers --json` (or equivalent internal call)
  and updates the cache

The dispatcher calls `get_cached(Duration::from_secs(60))` — at most one
quota refresh per minute across all concurrent dispatches.

## Config

Add optional `[dispatch]` section to `.cards/.bop/config.json`:
```json
{
  "dispatch": {
    "auto_select_provider": true,
    "quota_block_threshold": 0.90,
    "prefer_cheap_provider": "ollama-local"
  }
}
```
Defaults: auto_select=true, quota_block=0.90, prefer_cheap=null (no preference).

## Acceptance Criteria

- [ ] Dispatcher skips a provider at ≥90% quota and tries next in chain
- [ ] Trivial cards (cost=1) prefer `prefer_cheap_provider` when set
- [ ] Quota snapshot is cached — only refreshed once per 60s (not per card)
- [ ] `bop dispatcher --once` logs which provider was selected and why
- [ ] Existing `provider_chain` override still works (auto-select only reorders, never ignores)
- [ ] `cargo clippy -- -D warnings` clean
- [ ] `cargo test` passes (unit test: select_provider with mock snapshots)

## Files

- `crates/bop-cli/src/providers/mod.rs` — add ProviderCache + select_provider fn
- `crates/bop-cli/src/dispatcher.rs` — call select_provider before spawning adapter
- `crates/bop-core/src/config.rs` — add dispatch section to BopConfig
