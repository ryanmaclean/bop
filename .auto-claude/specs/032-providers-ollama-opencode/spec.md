# Spec 032 — bop providers: Ollama + opencode

## Goal

Implement `OllamaProvider` and `OpenCodeProvider` in `crates/bop-cli/src/providers/`,
building on the `Provider` trait from spec 030.

## Ollama Provider

Ollama has two modes: local inference server and cloud API. Both use the same
API surface but different base URLs and authentication.

### Local mode (`http://localhost:11434`)

`detect()`: `GET http://localhost:11434/api/version` succeeds (< 500ms timeout)

`fetch()`:
1. `GET http://localhost:11434/api/ps` — loaded models
   Response: `{"models":[{"name":"...","size_vram":N,"expires_at":"..."}]}`
2. Compute VRAM meter:
   - Sum `size_vram` across all loaded models
   - Get total VRAM via `GET http://localhost:11434/api/ps` (future: `/api/show` for GPU info)
   - For now: `primary_pct = None` (VRAM total not exposed), show loaded model names
3. Return snapshot with:
   - `provider` = "ollama-local"
   - `display_name` = "Ollama (local)"
   - `primary_pct` = None (no quota ceiling)
   - `tokens_used` = None (no per-session total exposed)
   - `source` = "http"
   - Include loaded model names as a custom field in JSON output
   - `reset_at` = earliest `expires_at` among loaded models (when first model unloads)

### Cloud mode (`https://ollama.com`)

`detect()`: `OLLAMA_API_KEY` env var set OR `~/.ollama/config.json` has `api_key`

`fetch()`:
1. `GET https://ollama.com/api/ps` with `Authorization: Bearer <api_key>`
2. Same response shape as local
3. `primary_pct` = None (no quota API; subscription is fair-use only)
4. `source` = "http"
5. `display_name` = "Ollama (cloud)"

### Display

When `primary_pct` is None, show loaded model count + names instead of a bar:
```
Ollama (local)  http    —      —    mistral:latest, llama3:8b
Ollama (cloud)  http    —      —    (idle)
```

## OpenCode Provider

opencode (sst/opencode) runs a local HTTP server on port 4096 with SSE events.

### `detect()`

`GET http://localhost:4096/health` responds with `{"healthy":true,"version":"..."}` (< 500ms)

### `fetch()` — one-shot snapshot

1. `GET http://localhost:4096/session` — list all sessions
2. For the most recently updated session, `GET http://localhost:4096/session/:id`
3. Extract token/cost from session metadata if present
4. Return snapshot:
   - `provider` = "opencode"
   - `display_name` = "opencode"
   - `primary_pct` = None
   - `tokens_used` = total tokens from active session (if exposed)
   - `cost_usd` = cost from session (if exposed)
   - `source` = "http"

### SSE subscription (for `--watch` mode only)

When `bop providers --watch` is running and opencode is detected:
- Subscribe to `GET http://localhost:4096/event` (SSE stream)
- On each `session.*` event, update the in-memory snapshot
- Display live token/cost counters as opencode runs

This is additive — one-shot `bop providers` still uses the REST snapshot.

## Acceptance criteria

```bash
cargo build -p bop-cli
cargo test -p bop-cli providers::ollama
cargo test -p bop-cli providers::opencode
./target/debug/bop providers --json
```

All pass. clippy clean. fmt clean.

## Notes

- Local Ollama: do not fail if `api/ps` returns empty models list — just show "(idle)"
- OpenCode: if port 4096 is closed, `detect()` returns false silently; no error printed
- Use `tokio::time::timeout(Duration::from_millis(500), ...)` for all `detect()` checks
- The SSE subscription should be a best-effort background task; if it drops,
  fall back to polling the REST endpoint every 60s
- Check existing HTTP client in `Cargo.toml` before adding new deps
