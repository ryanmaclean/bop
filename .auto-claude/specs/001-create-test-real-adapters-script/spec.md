# Create scripts/test-real-adapters.nu

## Goal

Write `scripts/test-real-adapters.nu` — a Nushell script that tests each real
adapter (claude, ollama, codex) end-to-end through the dispatcher pipeline.

## What to build

A self-contained Nu script at `scripts/test-real-adapters.nu` that:

1. Accepts `--adapter <name>` (default: `all`) and `--timeout <seconds>` (default: 120)
2. For each adapter to test:
   - Checks prerequisite (tool installed + available): skip with `⊘` if absent
   - Creates an isolated temp dir via `mktemp -d`
   - Writes `providers.json` in the correct format (see below)
   - Creates a `.bop` card in `pending/` with:
     - `meta.json`: `{id, created, stage: "implement", provider_chain: ["<name>"]}`
     - `prompt.md`: trivial instruction to write `output/result.md`
   - Runs `./target/debug/bop --cards-dir <tmpdir> dispatcher --poll-ms 2000 --adapter <path>`
     as a background process
   - Polls until `done/<id>.bop` appears (or timeout)
   - Asserts `done/<id>.bop/output/result.md` exists and is non-empty
   - Prints `✓ <name>` (pass), `⊘ <name> — skipped`, or `✗ <name> — FAILED`
   - Cleans up temp dir
3. Exits 1 if any test failed, 0 otherwise

## providers.json format

```json
{
  "default_provider": "claude",
  "providers": {
    "claude": {
      "command": "/Users/studio/bop/adapters/claude.nu",
      "rate_limit_exit": 75
    }
  }
}
```

## Availability checks

- `claude`:  `(which claude | length) > 0`
- `ollama`:  `(which ollama | length) > 0` AND `curl -sf http://localhost:11434/api/tags` succeeds
- `codex`:   `(which codex | length) > 0` AND `"OPENAI_API_KEY" in $env`

## Prompt for each card

```
Create a file at output/result.md containing exactly the text: hello from <adapter-name>

Use file creation tools. Create the output/ directory first if needed.
Do not write any other files. Do not explain anything.
```

## Important notes

- The bop binary is at `./target/debug/bop` (relative to project root)
- The dispatcher flag is `--cards-dir <dir>` at the TOP LEVEL before `dispatcher`
  e.g.: `bop --cards-dir /tmp/x dispatcher --poll-ms 2000 --adapter adapters/claude.nu`
- The dispatcher needs `providers.json` AND card `provider_chain` to select the adapter
- `spawn_pane` is NOT used here — run dispatcher directly as background process with `^sh -c "... &"`
- Poll for done/ with `path exists` check in a loop, sleep 2s between checks
- Kill the background dispatcher after card reaches done/ or timeout

## Acceptance

```sh
# Script exists and is valid Nu:
nu --no-config-file scripts/test-real-adapters.nu --help

# Dry-run availability check (no real calls):
nu --no-config-file scripts/test-real-adapters.nu --adapter ollama --timeout 1
# Should either skip (ollama not reachable) or attempt once and timeout gracefully
```

Also run `make check` to ensure no Rust regressions.
