# codex.nu end-to-end adapter test

## Goal

Run `scripts/test-real-adapters.nu --adapter codex` and get a PASS or graceful SKIP.
If `OPENAI_API_KEY` is not set, the test skips — that is acceptable.

## Prerequisites

- `scripts/test-real-adapters.nu` exists (spec 001 complete)
- `codex` CLI installed: `which codex` → `/opt/homebrew/bin/codex` (version 0.101.0)
- `OPENAI_API_KEY` env var set (or test skips)

## How codex.nu works

`adapters/codex.nu` runs:
```
codex exec --dangerously-bypass-approvals-and-sandbox \
  -c "sandbox_permissions=[\"disk-full-read-access\",\"disk-write-access\"]" \
  "<prompt>"
```
in `cd $workdir`. Like claude, codex writes files directly using its tools.
`output/result.md` must be created BY the codex session.

## Steps

1. Check API key:
   ```sh
   echo ${OPENAI_API_KEY:0:8}...
   ```
   If empty: the test will skip automatically. Document the skip and exit 0.

2. Run the test:
   ```sh
   nu --no-config-file scripts/test-real-adapters.nu --adapter codex --timeout 120
   ```

3. If `OPENAI_API_KEY` is missing and the test does NOT skip gracefully,
   fix the availability check in `scripts/test-real-adapters.nu`:
   ```nushell
   "codex" => (
       (which codex | length) > 0
       and ("OPENAI_API_KEY" in $env)
   )
   ```

4. If card goes to `failed/` with an auth error: expected — mark as skip.

5. If card goes to `failed/` for another reason, debug:
   ```sh
   td=$(mktemp -d)
   mkdir -p $td/output
   echo "Write hello from codex to output/result.md" > $td/prompt.md
   touch $td/stdout.log $td/stderr.log
   nu --no-config-file adapters/codex.nu $td $td/prompt.md $td/stdout.log $td/stderr.log
   echo "Exit: $?"
   cat $td/stderr.log | head -30
   ```

6. Run full test suite:
   ```sh
   nu --no-config-file scripts/test-real-adapters.nu
   ```
   Expected final output (codex skipped if no key):
   ```
   ✓ claude
   ✓ ollama
   ⊘ codex — skipped (tool not available)
   2 passed  1 skipped  0 failed
   ```

7. Run `make check`.

## Acceptance

`nu --no-config-file scripts/test-real-adapters.nu` exits 0 (pass or skip for codex).
`make check` passes.
All three adapters accounted for in output (pass or skip, no silent omissions).
