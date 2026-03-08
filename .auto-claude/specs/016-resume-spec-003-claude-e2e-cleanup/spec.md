# Resume spec-003: claude e2e adapter test — run and verify passing

## Context

Spec 003 updated `scripts/test-real-adapters.nu` to make the prompt more explicit.
One subtask is still blocked: actually running the claude adapter test end-to-end
and confirming it exits 0.

## What to do

1. Read `scripts/test-real-adapters.nu` to understand the current prompt template.
2. Run the claude adapter smoke test:
   ```
   nu --no-config-file scripts/test-real-adapters.nu --adapter claude --timeout 120
   ```
3. If it fails:
   - Check stderr for the failure reason
   - If the prompt is not explicit enough about using Write tool, update it
   - Re-run until it passes (max 3 attempts)
4. Run `make check`.
5. Write `output/result.md` confirming the test passed (or summarising what was fixed).

## Acceptance

- `nu --no-config-file scripts/test-real-adapters.nu --adapter claude --timeout 120` exits 0
- `make check` passes
- `output/result.md` exists
