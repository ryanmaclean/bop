# Ollama end-to-end adapter test

## Goal

Run `scripts/test-real-adapters.nu --adapter ollama` and get a PASS.
Fix `adapters/ollama-local.nu` if the card reaches `done/` but `output/result.md`
is missing (known gap: plain text responses are not written to result.md).

## Prerequisites

- `scripts/test-real-adapters.nu` exists (spec 001 complete)
- Ollama is running: `curl -sf http://localhost:11434/api/tags`
- Model `qwen2.5:7b` is available: `ollama list | grep qwen2.5:7b`

## Known bug to fix

`adapters/ollama-local.nu` only writes `output/result.md` when the model emits
a structured `{"files": [...]}` JSON block. For plain text responses (the
common case), stdout is captured but `output/result.md` is never written.

Fix: after the JSON extraction block (~line 103), add a fallback:

```nushell
    # Fallback: if no structured JSON output, write stdout as result.md
    if not ($"($workdir)/output/result.md" | path exists) {
        mkdir $"($workdir)/output"
        open --raw $stdout_abs | save --force $"($workdir)/output/result.md"
    }
```

## Steps

1. Run the test:
   ```sh
   OLLAMA_MODEL=qwen2.5:7b nu --no-config-file scripts/test-real-adapters.nu --adapter ollama --timeout 180
   ```

2. If card reaches `done/` but `output/result.md` is missing: apply the fallback fix above.

3. If card goes to `failed/`: debug directly:
   ```sh
   td=$(mktemp -d)
   mkdir -p $td/output
   echo "Write the text 'hello from ollama' to output/result.md" > $td/prompt.md
   touch $td/stdout.log $td/stderr.log
   OLLAMA_MODEL=qwen2.5:7b nu --no-config-file adapters/ollama-local.nu \
     $td $td/prompt.md $td/stdout.log $td/stderr.log
   echo "Exit: $?"
   cat $td/stderr.log
   ```

4. Re-run until:
   ```
   ✓ ollama
   1 passed  0 skipped  0 failed
   ```

5. Run `make check` — must pass.

## Acceptance

`nu --no-config-file scripts/test-real-adapters.nu --adapter ollama --timeout 180` exits 0.
`done/<card>.bop/output/result.md` contains non-empty text.
`make check` passes.
