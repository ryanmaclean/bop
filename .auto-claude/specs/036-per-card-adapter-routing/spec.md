# Spec 036 — per-card adapter routing in dispatcher

## Overview

The dispatcher currently takes a global `--adapter adapters/claude.nu` flag that is
baked into the launchd plist at install time. Every card uses the same adapter regardless
of what `provider_chain` says in `meta.json`.

This spec makes the dispatcher read `meta.json.provider_chain[0]` and select the adapter
automatically, so dropping a card with `provider_chain: ["codex"]` into `pending/` will
run it with `adapters/codex.nu` without any manual flag changes.

## Adapter resolution

In `crates/bop-cli/src/dispatcher.rs`, after reading `meta.json`, add:

```rust
fn resolve_adapter(meta: &Meta, fallback: &str) -> String {
    let provider = meta.provider_chain.first().map(|s| s.as_str()).unwrap_or("");
    match provider {
        "claude"       => "adapters/claude.nu".to_string(),
        "codex"        => "adapters/codex.nu".to_string(),
        "gemini"       => "adapters/gemini.nu".to_string(),
        "ollama"       => "adapters/ollama-local.nu".to_string(),
        "ollama-local" => "adapters/ollama-local.nu".to_string(),
        "mock"         => "adapters/mock.nu".to_string(),
        "opencode"     => "adapters/opencode.nu".to_string(),
        "goose"        => "adapters/goose.nu".to_string(),
        "aider"        => "adapters/aider.nu".to_string(),
        _              => fallback.to_string(),
    }
}
```

Call `resolve_adapter(&meta, &global_adapter)` instead of using `global_adapter` directly.
The `global_adapter` (from `--adapter` CLI flag) becomes a fallback for cards with empty
or unrecognized provider_chain.

## `bop factory install` plist change

Remove the hardcoded `--adapter adapters/claude.nu` from the dispatcher plist arguments
in `factory.rs`. The adapter is now resolved per-card, so the global flag becomes optional.
Keep it in the plist but set it to `adapters/claude.nu` as explicit fallback (documents intent).

## `bop new` template update

Card templates (`.cards/templates/*.card/meta.json`) should have `provider_chain` set:
- `implement.card` → `["claude"]` (existing behaviour, now explicit)
- Update any template that has empty `provider_chain: []`

## Acceptance Criteria

- [ ] `cargo test` passes
- [ ] `cargo clippy -- -D warnings` clean
- [ ] A card with `provider_chain: ["codex"]` in pending/ is dispatched with `adapters/codex.nu`
- [ ] A card with `provider_chain: ["claude"]` uses `adapters/claude.nu`
- [ ] A card with empty `provider_chain: []` falls back to the global `--adapter` flag
- [ ] A card with an unknown provider (e.g. `["grok"]`) falls back to global `--adapter`
- [ ] Unit test: `resolve_adapter` returns correct adapter for each known provider name
- [ ] `bop factory status` still shows dispatcher as running after reinstall

## Files to modify

- `crates/bop-cli/src/dispatcher.rs` — add `resolve_adapter`, call it per card
- `crates/bop-cli/src/factory.rs` — keep `--adapter` in plist as explicit fallback
- `.cards/templates/implement.card/meta.json` — set `provider_chain: ["claude"]` if empty
