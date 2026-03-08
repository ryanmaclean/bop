# Spec 042 — providers: live Codex + Gemini quota polling

## Overview

`bop providers` (specs 030-032) scaffolded the provider trait and wired Claude
OAuth quota. Codex and Gemini providers may still be returning stub/placeholder
data. This spec makes them return real live quota by hitting the actual APIs.

## Codex CLI quota

Codex CLI stores session quota in its local cache. Query it by:
1. Running `codex --version` to confirm CLI is present
2. Parsing `~/.codex/auth.json` (or equivalent) for the session token
3. Hitting the OpenAI usage API: `GET https://api.openai.com/v1/usage` with
   the session token. OR parsing the local `~/.codex/quota.json` if it exists.
4. If no API available, shell out to `codex usage` or `codex status` and parse
   the human-readable output.

Fallback: return `ProviderSnapshot { utilization: 0.0, loaded_models: None, error: None }`
with a note in `error` if quota is unavailable.

## Gemini CLI quota

Gemini CLI authenticates via `~/.config/google/application_default_credentials.json`
or OAuth in `~/.gemini/`. Query quota by:
1. Parse the refresh token from `~/.gemini/credentials.json`
2. Exchange for access token via `https://oauth2.googleapis.com/token`
3. Call `GET https://generativelanguage.googleapis.com/v1beta/models` to
   confirm auth works (returns model list)
4. For quota: call the Cloud Billing / Quota API or parse from Gemini CLI's
   local state file (`~/.gemini/session.json` or similar)

Fallback: shell out to `gemini --version` and parse any quota output. If
unavailable, return empty snapshot with `error: Some("quota unavailable")`.

## `bop providers --json` output

Ensure the JSON output from `bop providers --json` is machine-parseable:
```json
{
  "providers": [
    {
      "name": "claude",
      "utilization_5h": 0.61,
      "utilization_7d": 0.30,
      "reset_in_secs": 1200,
      "error": null
    },
    {
      "name": "codex",
      "utilization_session": 0.05,
      "reset_in_secs": null,
      "error": null
    }
  ]
}
```

## Acceptance Criteria

- [ ] `bop providers` output shows real (non-zero or confirmed-zero) Codex data
- [ ] `bop providers` output shows real Gemini data or a clear error message
- [ ] `bop providers --json` emits valid JSON parseable by `jq`
- [ ] Claude quota still works (regression check)
- [ ] `cargo clippy -- -D warnings` clean
- [ ] `cargo test` passes

## Files to modify

- `crates/bop-cli/src/providers/codex.rs` — real quota fetch
- `crates/bop-cli/src/providers/gemini.rs` — real quota fetch
- `crates/bop-cli/src/providers/mod.rs` — JSON serialisation of ProviderSnapshot
