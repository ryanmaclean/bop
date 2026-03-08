# QA Fix Request

**Status**: REJECTED
**Date**: 2026-03-07T09:35:00+00:00
**QA Session**: 1

## Critical Issues to Fix

### 1. Adapters Do Not Exit 75 on Network Errors

**Problem**: The adapters `claude.nu` and `codex.nu` define network error detection patterns and helper function, but never actually call the helper function to check stderr and exit 75 on network errors.

**Location**:
- `adapters/claude.nu` lines 79-90
- `adapters/codex.nu` lines 57-67

**Required Fix**: Add network error check before rate-limit check in both adapters.

**Current code** (`claude.nu` lines 79-90):
```nushell
if $rc == 142 { exit 75 }

if ($stderr_abs | path exists) {
    let stderr_text = open --raw $stderr_abs
    if (($stderr_text | str contains --ignore-case "rate limit")
        or ($stderr_text | str contains "429")
        or ($stderr_text | str contains --ignore-case "too many requests")) {
        exit 75
    }
}

exit $rc
```

**Fixed code**:
```nushell
if $rc == 142 { exit 75 }

if ($stderr_abs | path exists) {
    let stderr_text = open --raw $stderr_abs

    # Check network errors first (using NETWORK_ERROR_PATTERNS)
    if (is_network_error $stderr_text) {
        exit 75
    }

    # Then check rate limiting
    if (($stderr_text | str contains --ignore-case "rate limit")
        or ($stderr_text | str contains "429")
        or ($stderr_text | str contains --ignore-case "too many requests")) {
        exit 75
    }
}

exit $rc
```

**Apply same fix to**: `adapters/codex.nu` (similar location, same pattern)

**Verification**:
1. Run adapter tests: `nu adapters/claude.nu --test && nu adapters/codex.nu --test`
2. Create a test card with network error in stderr
3. Verify adapter exits with code 75 (not 1 or 2)
4. Verify dispatcher correctly requeues the card to `pending/`

**Acceptance Criterion**: "Adapters exit 75 on network errors using defined constant pattern list"

## Why This Matters

The spec explicitly requires adapters to exit 75 on network errors. The current implementation has the infrastructure in place (NETWORK_ERROR_PATTERNS constant, is_network_error helper function, tests) but doesn't actually USE it in the main exit logic.

Currently, network errors exit with code 1/2, and the dispatcher's fallback `is_network_failure()` function catches them. This works, but does NOT meet the acceptance criteria which specifically requires adapters to exit 75.

## After Fixes

Once fixes are complete:
1. Test both adapters with `--test` flag
2. Run `make check` to verify no regressions
3. Commit with message: "fix: adapters exit 75 on network errors (qa-requested)"
4. QA will automatically re-run

**Estimated fix time**: 5-10 minutes (simple 4-line addition to 2 files)
