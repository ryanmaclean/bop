# claude.nu end-to-end adapter test

## Goal

Run `scripts/test-real-adapters.nu --adapter claude` and get a PASS.
A real `claude` CLI invocation must produce `output/result.md` in the card bundle.

## Prerequisites

- `scripts/test-real-adapters.nu` exists (spec 001 complete)
- `claude` CLI installed: `which claude` → `/opt/homebrew/bin/claude`
- Not inside a claude session (the adapter unsets `CLAUDECODE` already)

## How claude.nu works

`adapters/claude.nu` runs:
```
claude -p "<prompt>" --dangerously-skip-permissions --output-format json
```
in `cd $workdir`. Claude writes files directly using its Write tool. The adapter
captures stdout/stderr to logs. `output/result.md` must be created BY the
claude session (via Write tool), not by the adapter itself.

## Steps

1. Run the test:
   ```sh
   nu --no-config-file scripts/test-real-adapters.nu --adapter claude --timeout 120
   ```

2. If card goes to `failed/` or `output/result.md` is missing, debug directly:
   ```sh
   td=$(mktemp -d)
   mkdir -p $td/output
   cat > $td/prompt.md << 'EOF'
   Create the file output/result.md containing exactly the text: hello from claude
   Use the Write tool. Create output/ directory first if needed. Do nothing else.
   EOF
   touch $td/stdout.log $td/stderr.log
   nu --no-config-file adapters/claude.nu $td $td/prompt.md $td/stdout.log $td/stderr.log
   echo "Exit: $?"
   cat $td/stderr.log | head -30
   ls $td/output/
   ```

3. If `output/result.md` not created: the prompt needs to be more explicit.
   Update the prompt in `scripts/test-real-adapters.nu` for claude to say:
   ```
   Use the Write tool to create the file output/result.md.
   The file must contain exactly: hello from claude
   Do not create any other files.
   ```

4. Re-run until:
   ```
   ✓ claude
   1 passed  0 skipped  0 failed
   ```

5. Run `make check`.

## Acceptance

`nu --no-config-file scripts/test-real-adapters.nu --adapter claude --timeout 120` exits 0.
`done/<card>.bop/output/result.md` exists and is non-empty.
`make check` passes.
