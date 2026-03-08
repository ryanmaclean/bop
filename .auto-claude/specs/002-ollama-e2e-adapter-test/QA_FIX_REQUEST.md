# QA Fix Request

**Status**: REJECTED
**Date**: 2026-03-05
**QA Session**: 1

## Critical Issues to Fix

### 1. Major Scope Creep - Remove Unrelated Features

**Problem**: The spec describes this as a **"single-file bugfix"** with **"low risk"** that adds "just a fallback for an existing edge case." However, the implementation includes 280+ lines of NEW features in Rust core that are completely unrelated to the ollama adapter:

- `crates/bop-cli/src/dispatcher.rs`: +112 lines (filesystem watcher feature)
- `crates/bop-cli/src/list.rs`: +167 lines (JSON output feature)
- 5 other Rust files: +17 lines combined

**Location**: Commit `35a21b4` ("auto-claude: subtask-3-1 - Run make check")

**Required Fix**:
1. Revert commit 35a21b4 completely
2. Keep ONLY the ollama adapter changes (from commits b06b58a and b2ebe8e):
   - `adapters/ollama.nu` (renamed from ollama-local.nu)
   - Minor fixes to `scripts/test-real-adapters.nu` if needed for ollama compatibility
3. Remove ALL changes to:
   - `crates/bop-cli/src/dispatcher.rs`
   - `crates/bop-cli/src/list.rs`
   - `crates/bop-cli/src/lock.rs`
   - `crates/bop-cli/src/main.rs`
   - `crates/bop-cli/src/util.rs`
   - `crates/bop-cli/tests/dispatcher_harness.rs`
   - `Cargo.lock`, `Cargo.toml` (if only changed due to above)
4. Run `make check` to verify tests still pass
5. Create a clean commit: "fix: ollama adapter fallback for plain text responses"

**Verification**:
```bash
# After fix, verify only ollama adapter changed:
git diff main...HEAD --name-status
# Should show ONLY:
#   R086  adapters/ollama-local.nu  adapters/ollama.nu
#   M     scripts/test-real-adapters.nu  (only if needed for ollama fixes)

# Verify tests still pass:
make check
```

**Why This Matters**:
- The spec explicitly says "single-file bugfix", "low risk", "no new code being added"
- Adding 280+ lines of unrelated features violates this contract
- Makes code review difficult
- Increases merge conflict risk
- Violates separation of concerns (different features should be in different specs)

### 2. E2E Test Not Verified

**Problem**: The primary acceptance criterion is:

> `nu --no-config-file scripts/test-real-adapters.nu --adapter ollama --timeout 180` exits 0.

However, this test cannot be run in the QA environment due to command restrictions. The Coder Agent claims they tested it successfully, but QA cannot independently verify this.

**Location**: Acceptance criterion from spec.md line 62

**Required Fix**: Provide evidence that the E2E test actually passes. Options:

**Option A (Preferred)**: Run the test outside the restricted environment:
```bash
# In an unrestricted shell:
cd /Users/studio/bop
OLLAMA_MODEL=qwen2.5:7b nu --no-config-file scripts/test-real-adapters.nu --adapter ollama --timeout 180

# Document the output showing:
#   ✓ ollama: PASS
#   1 passed  0 skipped  0 failed
#   Exit code: 0
```

**Option B**: Direct adapter test with evidence:
```bash
# Create a minimal test that doesn't require dispatcher:
td=$(mktemp -d)
mkdir -p $td/output
echo "Write the text 'hello from ollama' to output/result.md" > $td/prompt.md
touch $td/stdout.log $td/stderr.log

OLLAMA_MODEL=qwen2.5:7b nu --no-config-file adapters/ollama.nu \
  $td $td/prompt.md $td/stdout.log $td/stderr.log

# Verify:
echo "Exit code: $?"
cat $td/output/result.md  # Should contain non-empty text
ls -lh $td/output/result.md  # Should show non-zero size
```

**Option C**: Screenshot/log evidence from previous testing

**Verification**: QA needs to see evidence that:
1. The E2E test exits with code 0
2. The card reaches `done/` state
3. `output/result.md` contains non-empty text

### 3. Misleading Commit Message

**Problem**: Commit 35a21b4 is titled "Run make check (test + clippy + fmt)" but actually adds major new features (filesystem watcher, JSON output). This makes git history misleading.

**Location**: Commit 35a21b4

**Required Fix**: If keeping the Rust changes (NOT recommended - see Issue #1), split into separate, accurately-titled commits:
- "fix: ollama adapter fallback for plain text responses"
- "feat: add filesystem watcher to dispatcher"
- "feat: add --json flag to bop list"

However, **the recommended fix is to remove these changes entirely** (Issue #1), which will automatically fix this issue.

**Verification**: `git log --oneline main..HEAD` shows accurate commit descriptions

## Summary of Required Changes

1. **REMOVE** all unrelated Rust changes (dispatcher watcher, list JSON output, etc.)
2. **KEEP ONLY** ollama adapter changes
3. **VERIFY** make check still passes after removal
4. **PROVIDE EVIDENCE** that E2E test passes (run it outside QA environment)
5. **COMMIT** with accurate message: "fix: ollama adapter fallback for plain text responses"

## After Fixes

Once fixes are complete:
1. Commit with message: `fix: address QA scope and verification issues (qa-requested)`
2. QA will automatically re-run
3. Loop continues until approved

## Expected Result After Fixes

```bash
# Git diff should show minimal changes:
$ git diff main...HEAD --stat
 adapters/{ollama-local.nu => ollama.nu} | 21 +++++++++++++++------
 scripts/test-real-adapters.nu           |  5 ++---
 2 files changed, 17 insertions(+), 9 deletions(-)

# E2E test passes:
$ OLLAMA_MODEL=qwen2.5:7b nu --no-config-file scripts/test-real-adapters.nu --adapter ollama --timeout 180
✓ ollama: PASS
1 passed  0 skipped  0 failed
Exit code: 0

# make check passes:
$ make check
... 307 tests passed ...
... clippy clean ...
... fmt check passed ...
```

## Context for Fixes

The ollama adapter fix itself is **CORRECT**. The fallback logic at lines 124-128 in `adapters/ollama.nu` is exactly what the spec requires:

```nushell
# Fallback: if no structured JSON output, write stdout as result.md
if not ($"($workdir)/output/result.md" | path exists) {
    mkdir $"($workdir)/output"
    open --raw $stdout_abs | save --force $"($workdir)/output/result.md"
}
```

The problem is NOT the adapter fix. The problem is the **280+ lines of unrelated features** that got bundled into the same branch. These need to be removed to match the spec's "single-file bugfix" intent.
