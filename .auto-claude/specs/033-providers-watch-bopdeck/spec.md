# Spec 033 — bop providers: --watch mode + BopDeck socket feed

## Goal

Wire all providers (030–032) into a live `--watch` mode and feed snapshots to
BopDeck's Unix domain socket as OpenLineage events, so the notch HeaderWidget
shows live provider meters.

## `bop providers --watch`

1. Start all detected providers concurrently (tokio tasks)
2. Poll each every `$BOP_PROVIDER_POLL_INTERVAL` seconds (default: 60)
3. Re-render the ANSI table in-place on each update (use `crossterm` or the
   same approach as `bop status --watch`)
4. On each successful snapshot, append to `~/.bop/provider-history.jsonl`
5. On each snapshot, emit an OpenLineage event to the BopDeck socket (see below)
6. Ctrl+C exits cleanly

## BopDeck socket protocol

BopDeck listens at `/tmp/bop-deck-<username>.sock` for newline-delimited
OpenLineage `RunEvent` JSON.

Emit one event per provider per poll cycle:

```json
{
  "eventType": "RUNNING",
  "eventTime": "2026-03-07T20:14:00Z",
  "run": {
    "runId": "<uuid-v4-stable-per-provider>",
    "facets": {}
  },
  "job": {
    "namespace": "bop",
    "name": "provider.<provider_id>"
  },
  "inputs": [],
  "outputs": [],
  "facets": {
    "bop_provider_quota": {
      "_producer": "https://github.com/ryanmaclean/bop",
      "_schemaURL": "bop://facets/provider-quota/v1",
      "provider": "claude",
      "displayName": "Claude Code",
      "primaryPct": 57,
      "secondaryPct": 38,
      "primaryLabel": "5h",
      "secondaryLabel": "7d",
      "resetAt": "2026-03-07T22:53:00Z",
      "tokensUsed": null,
      "costUsd": null,
      "source": "oauth",
      "error": null
    }
  }
}
```

Use a **stable UUID per provider** (derive from provider name as a UUID v5
namespace so the same provider always has the same `runId` — BopDeck uses this
to update rather than create new entries).

## BopDeck socket writer

`crates/bop-cli/src/providers/bopdeck.rs`:

```rust
pub struct BopDeckWriter {
    socket_path: PathBuf,
}

impl BopDeckWriter {
    pub fn new() -> Self { /* /tmp/bop-deck-<username>.sock */ }
    pub fn detect(&self) -> bool { /* socket path exists */ }
    pub async fn emit(&self, snapshot: &ProviderSnapshot) -> anyhow::Result<()> {
        /* connect to Unix socket, write JSON + newline, disconnect */
    }
}
```

Connect fresh on each emit (don't hold a persistent connection — socket may
restart). If connect fails, log at debug level and continue silently.

## `~/.bop/provider-history.jsonl`

Each line:
```json
{"ts":1741382040000,"provider":"claude","primary_pct":57,"secondary_pct":38,"tokens_used":null,"cost_usd":null}
```

`read_sparkline(provider, n)` → returns last N `primary_pct` values as `Vec<Option<u8>>`
for use in BopDeck HeaderWidget sparkline rendering (already wired in spec 029 header).

## Acceptance criteria

```bash
cargo build -p bop-cli
cargo test -p bop-cli providers

# Integration: start watch, verify socket emit
./target/debug/bop providers --watch --interval 5 &
PID=$!
sleep 8
kill $PID
cat ~/.bop/provider-history.jsonl | jq -s 'length > 0'  # true

# Help
./target/debug/bop providers --help
```

All pass. clippy clean. fmt clean.

## Notes

- UUID v5: use namespace `6ba7b810-9dad-11d1-80b4-00c04fd430c8` (DNS namespace)
  with provider name as input: `uuid::Uuid::new_v5(&NAMESPACE_DNS, b"claude")`
- If BopDeck socket is not present, skip silently — never error on missing socket
- `~/.bop/` directory: create with `fs::create_dir_all` if not present
- History file max size: trim to last 10k lines on open if file > 1MB
- The `--watch` in-place rendering: use `\x1b[<N>A` (cursor up N lines) +
  `\x1b[J` (clear to end of screen) pattern, same as `bop status --watch`
