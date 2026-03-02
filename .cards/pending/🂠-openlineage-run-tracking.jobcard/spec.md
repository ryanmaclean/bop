# OpenLineage-style run tracking in .jobcard

## Goal

Every `.jobcard/meta.json` should record which model ran which stage, when, and what
happened — an OpenLineage-inspired `runs` array. This gives full provenance of a card
as it moves through the system, enabling auditing and comparison across models.

## Data model

Add to `Meta` in `crates/jobcard-core/src/lib.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct RunRecord {
    pub run_id: String,          // uuid4 short (first 8 chars)
    pub stage: String,           // "implement", "qa", etc.
    pub provider: String,        // "claude", "ollama", "mock"
    pub model: String,           // "qwen3-coder:480b-cloud", "claude-sonnet-4-6", etc.
    pub adapter: String,         // "adapters/ollama-local.zsh"
    pub started_at: String,      // ISO-8601 UTC
    pub ended_at: Option<String>,
    pub outcome: String,         // "success" | "rate_limited" | "failed" | "timeout" | "running"
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
    pub duration_s: Option<u64>,
    pub note: Option<String>,    // short freeform, e.g. "retry 2"
}
```

Add to `Meta`:
```rust
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub runs: Vec<RunRecord>,
```

Use `#[serde(default)]` on all `RunRecord` fields so old cards deserialize cleanly.

## Dispatcher changes (`crates/jc/src/main.rs`)

In the dispatcher loop, before spawning the adapter:
1. Generate a `run_id` = first 8 chars of a uuid (use `uuid::Uuid::new_v4().to_string()[..8]`)
2. Create `RunRecord { run_id, stage, provider, model, adapter, started_at: now, outcome: "running".into(), .. }`
3. Push to `meta.runs` and write meta to disk
4. After adapter exits:
   - Set `ended_at`, `duration_s`, `outcome` ("success" / "rate_limited" / "failed" / "timeout")
   - If `logs/ollama-stats.json` exists, read `prompt_tokens`, `completion_tokens`
   - If `logs/stdout.log` is valid JSON with `total_cost_usd`, read `cost_usd` and `prompt_tokens` etc.
   - Write final `meta.runs` back to disk

## Model detection

The `model` field in `RunRecord` should come from:
- For claude adapter: parse `logs/stdout.log` JSON field `modelUsage` keys (e.g. `claude-sonnet-4-6`)
- For ollama adapter: read `logs/ollama-stats.json` field `model`
- Fallback: use the provider name from `provider_chain`

Add a helper `fn detect_model_from_logs(card_dir: &Path) -> Option<String>` that tries
both log files.

## Dependencies

Add `uuid` crate to `crates/jc/Cargo.toml`:
```toml
uuid = { version = "1", features = ["v4"] }
```

## `bop inspect` output

In `cmd_inspect`, after the Cost line, print:

```
Runs  3 attempts
  #1  2026-03-01T20:45Z  ollama/qwen3-coder:480b-cloud  implement  rate_limited  (45s)
  #2  2026-03-01T20:50Z  claude/claude-sonnet-4-6        implement  success       (180s, $0.73)
```

## Acceptance Criteria
- `cargo build`
- `cargo clippy -- -D warnings`
- `grep -q 'pub struct RunRecord' crates/jobcard-core/src/lib.rs`
- `grep -q 'pub runs:' crates/jobcard-core/src/lib.rs`
- `./target/debug/bop inspect short-cli-flags 2>&1 | grep -qi 'run'`
- `jj log -r 'main..@-' | grep -q .`
