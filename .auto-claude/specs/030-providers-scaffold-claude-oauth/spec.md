# Spec 030 — bop providers: scaffold + Claude OAuth

## Goal

Add a `bop providers` subcommand that auto-detects installed AI providers and
displays their quota/usage. Phase 1: scaffold the trait + common types, and
implement the Claude Code OAuth provider.

## Background

CodexBar (steipete/CodexBar) reverse-engineered that Claude Code stores OAuth
tokens at `~/.claude/.credentials.json` (or Keychain `Claude Code-credentials`)
and exposes quota via:

```
GET https://api.anthropic.com/api/oauth/usage
Authorization: Bearer <access_token>
anthropic-beta: oauth-2025-04-20
```

Response fields:
- `five_hour` → session window % used
- `seven_day` → weekly window % used
- `seven_day_sonnet` / `seven_day_opus` → model-specific weekly
- `rate_limit_tier` → Max/Pro/Team/Enterprise

## Deliverables

### 1. `crates/bop-cli/src/providers/mod.rs`

Define the `Provider` trait and `ProviderSnapshot` type:

```rust
pub struct ProviderSnapshot {
    pub provider: String,          // "claude", "codex", "gemini", "ollama", "opencode"
    pub display_name: String,
    pub primary_pct: Option<u8>,   // 0-100, None if not quota-based
    pub secondary_pct: Option<u8>,
    pub primary_label: Option<String>,   // "5h", "7d", "requests"
    pub secondary_label: Option<String>,
    pub tokens_used: Option<u64>,
    pub cost_usd: Option<f64>,
    pub reset_at: Option<chrono::DateTime<chrono::Utc>>,
    pub source: String,            // "oauth", "rpc", "pty", "http", "log"
    pub error: Option<String>,
}

#[async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    fn detect(&self) -> bool;   // returns true if credentials/server found on this machine
    async fn fetch(&self) -> anyhow::Result<ProviderSnapshot>;
}
```

### 2. `crates/bop-cli/src/providers/claude.rs`

`ClaudeProvider` implementation:

1. Read `~/.claude/.credentials.json` — parse `access_token`, `expires_at`
2. If `expires_at` is past, attempt token refresh (best-effort, log warning on fail)
3. `GET https://api.anthropic.com/api/oauth/usage` with `Authorization: Bearer <token>`
   and `anthropic-beta: oauth-2025-04-20`
4. Map response:
   - `primary_pct` = `five_hour.percent_used` (0-100)
   - `secondary_pct` = `seven_day.percent_used`
   - `primary_label` = "5h"
   - `secondary_label` = "7d"
   - `reset_at` = `five_hour.reset_at`
   - `source` = "oauth"
5. `detect()` returns true if `~/.claude/.credentials.json` exists OR if
   `~/.claude/.credentials.json` is missing but Keychain item
   `Claude Code-credentials` is present (macOS only, use `security find-generic-password`)

### 3. `crates/bop-cli/src/providers/history.rs`

Append-only history for sparklines:
- File: `~/.bop/provider-history.jsonl`
- Each line: `{"ts": <unix_ms>, "provider": "claude", "primary_pct": 57, "secondary_pct": 38}`
- Write after every successful fetch
- `read_history(provider, n)` returns last N snapshots for sparkline rendering

### 4. `crates/bop-cli/src/cmd/providers.rs`

CLI command wired into `main.rs`:

```
bop providers              # ANSI table, one-shot
bop providers --watch      # refresh every 60s, in-place update
bop providers --json       # JSON array of ProviderSnapshot
bop providers --interval N # override poll interval (seconds)
```

ANSI table format:
```
Provider      Source  5h    7d    Reset
──────────────────────────────────────
Claude Code   oauth   57%   38%   in 2h 39m
```

Use `indicatif` or simple ANSI bar chars (`█░`) for the percentage — consistent
with the style in `bop status --watch`.

### 5. Tests

- `test_claude_snapshot_parse`: unit test parsing a mock JSON response
- `test_detect_missing_creds`: `detect()` returns false when no creds file
- `test_history_roundtrip`: write + read back from temp file

## Acceptance criteria

```bash
cargo build -p bop-cli
cargo test -p bop-cli providers
./target/debug/bop providers --help
./target/debug/bop providers --json | jq '.[0].provider'
```

All pass. `cargo clippy -- -D warnings` clean. `cargo fmt --check` clean.

## Notes

- Do NOT add reqwest as a new dep if `ureq` or the existing HTTP client is
  already used elsewhere — check `Cargo.toml` first
- `~/.claude/.credentials.json` schema: `{"access_token":"...","refresh_token":"...","expires_at":"..."}`
- The `anthropic-beta` header is required; without it the endpoint returns 403
- If credentials are expired and refresh fails, return a snapshot with
  `error: Some("token expired")` rather than propagating the error
