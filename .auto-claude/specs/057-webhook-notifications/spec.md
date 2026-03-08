# Spec 057 — webhook notifications (TRIZ P24 Intermediary)

## Overview

P24 Intermediary: external systems (Slack, Discord, CI pipelines) get bop
events via HTTP POST webhooks — no polling, no custom integration per tool.
One outbound POST serves every consumer.

## Configuration

Add to `.cards/.bop/config.json`:
```json
{
  "webhooks": [
    {
      "url": "https://hooks.slack.com/services/...",
      "on": ["done", "failed", "merged"],
      "format": "slack"
    },
    {
      "url": "https://example.com/bop-hook",
      "on": ["done", "failed", "running", "merged"],
      "format": "json"
    }
  ]
}
```

`on` filters which state transitions trigger the webhook.
`format`: `"json"` (generic) or `"slack"` (Slack Block Kit payload).

## Payload formats

### `json` format
```json
{
  "event": "done",
  "card_id": "team-arch/spec-041",
  "state": "done",
  "provider": "codex",
  "cost_usd": 0.18,
  "tokens": 14200,
  "duration_secs": 291,
  "timestamp": "2026-03-08T14:22:18Z"
}
```

### `slack` format (Block Kit)
```json
{
  "text": "✓ team-arch/spec-041 done in 4m 51s ($0.18)",
  "blocks": [
    {"type": "section", "text": {"type": "mrkdwn",
      "text": "✓ *team-arch/spec-041* completed\n>Provider: codex  •  $0.18  •  14,200 tokens"}}
  ]
}
```

## Implementation

- `WebhookClient` in `crates/bop-cli/src/webhook.rs` using `reqwest` (already in deps)
- Fire webhooks asynchronously (tokio::spawn) — never block the dispatcher
- Retry once on network error; log failures to `logs/webhook.log` but do not fail the card
- Read config at startup, cache parsed webhooks in `Dispatcher` struct

## `bop webhook test`

Sends a test payload to all configured webhooks:
```sh
bop webhook test
# Sending test to https://hooks.slack.com/... → 200 OK
# Sending test to https://example.com/bop-hook → 200 OK
```

## Acceptance Criteria

- [ ] Webhooks fire on state transitions matching `on` filter
- [ ] `json` format payload is correct and parseable
- [ ] `slack` format renders in Slack (Block Kit valid)
- [ ] Webhook failures do not fail or block the card
- [ ] `bop webhook test` sends to all configured URLs
- [ ] No webhook config → no webhook calls (zero overhead)
- [ ] `cargo clippy -- -D warnings` clean
- [ ] `cargo test` passes (unit test with mock HTTP server)

## Files

- `crates/bop-cli/src/webhook.rs` — new module
- `crates/bop-cli/src/dispatcher.rs` — fire webhooks on state transitions
- `crates/bop-cli/src/main.rs` — wire `bop webhook test` subcommand
- `crates/bop-core/src/config.rs` — add webhooks to BopConfig
