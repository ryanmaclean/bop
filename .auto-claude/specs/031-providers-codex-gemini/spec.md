# Spec 031 — bop providers: Codex + Gemini

## Goal

Implement `CodexProvider` and `GeminiProvider` in `crates/bop-cli/src/providers/`,
building on the `Provider` trait from spec 030.

## Codex Provider

### Data sources (in priority order)

1. **OAuth API** (preferred)
   - Creds: `~/.codex/auth.json` → `{"access_token":"...","refresh_token":"...","last_refresh":"..."}`
   - Refresh if `last_refresh` > 8 days old: `POST https://auth.openai.com/...` (same flow as CodexBar)
   - Endpoint: `GET https://chatgpt.com/backend-api/wham/usage`
     `Authorization: Bearer <access_token>`
   - Response: `{"session":{"percent_used":N,"reset_at":"..."},"weekly":{"percent_used":N,"reset_at":"..."}}`

2. **JSON-RPC fallback** (if OAuth creds missing/expired)
   - Launch: `codex -s read-only -a untrusted app-server`
   - Handshake over stdin/stdout:
     ```json
     → {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"bop","version":"0.1"}}}
     ← {"result":{"serverInfo":...}}
     → {"jsonrpc":"2.0","id":2,"method":"account/rateLimits/read","params":{}}
     ← {"result":{"windows":[...],"credits":...}}
     ```
   - Parse `windows` array for session + weekly percent used
   - Kill the process after reading; do not keep it alive

3. **PTY fallback** (if RPC fails)
   - Spawn `codex` in a PTY via `tokio::process`
   - Send `/status\n`, wait 3s for output
   - Parse rendered screen: `5h limit: NN%` and `Weekly limit: NN%`

### `detect()`: `~/.codex/auth.json` exists OR `codex` binary on PATH

## Gemini Provider

### Data sources

1. **OAuth quota API** (preferred)
   - Creds: `~/.gemini/oauth_creds.json`
     `{"access_token":"...","refresh_token":"...","expiry_date":N,"id_token":"..."}`
   - Client ID/secret: locate `gemini` binary, find:
     `.../node_modules/@google/gemini-cli-core/dist/src/code_assist/oauth2.js`
     Extract `OAUTH_CLIENT_ID` and `OAUTH_CLIENT_SECRET` via regex
   - Refresh if `expiry_date < now`: `POST https://oauth2.googleapis.com/token`
   - Tier detection: `POST https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist`
     body: `{"metadata":{"ideType":"GEMINI_CLI","pluginType":"GEMINI"}}`
   - Quota: `POST https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuota`
     body: `{"project":"<cloudaicompanionProject from loadCodeAssist or ''>"}`
   - Response: array of `{"remainingFraction":0.62,"resetTime":"...","modelId":"gemini-2.5-pro"}`
   - Map: `primary_pct` = `(1 - min(remainingFraction)) * 100` for Pro models
           `secondary_pct` = same for Flash models
           `primary_label` = "Pro"
           `secondary_label` = "Flash"

2. **PTY fallback**
   - Spawn `gemini` in PTY, send `/stats model\n`, wait 3s
   - Parse output for request counts and limits

### `detect()`: `~/.gemini/oauth_creds.json` exists OR `gemini` binary on PATH

## Acceptance criteria

```bash
cargo build -p bop-cli
cargo test -p bop-cli providers::codex
cargo test -p bop-cli providers::gemini
./target/debug/bop providers --json | jq '[.[] | select(.provider == "codex" or .provider == "gemini")]'
```

All pass. clippy clean. fmt clean.

## Notes

- The `oauth2.js` regex for Gemini: `OAUTH_CLIENT_ID\s*=\s*["']([^"']+)["']`
  and `OAUTH_CLIENT_SECRET\s*=\s*["']([^"']+)["']`
- If `loadCodeAssist` fails (network error, auth error), skip project ID and
  pass empty string to `retrieveUserQuota` — it still returns global quota
- For the Codex RPC: use `tokio::process::Command` with piped stdin/stdout.
  Write JSON lines, read until you get a response with `id: 2`, then kill.
  Timeout: 5 seconds total
- Do not store OAuth secrets; read credentials fresh on each `fetch()` call
