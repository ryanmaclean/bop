# QA Fix Request

**Status**: REJECTED
**Date**: 2026-03-05T16:45:00Z
**QA Session**: 1

## Critical Issues to Fix

### 1. Empty Test Prompt in Template

**Problem**: The `write_template` function creates an empty `spec.md` file at line 286, but real adapters need instructions on what to do.

**Location**: `scripts/test-real-adapters.nu:286`

**Required Fix**:
Replace:
```nushell
"" | save --force ($tdir | path join "spec.md")
```

With:
```nushell
let spec_content = "Create a file at output/result.md containing exactly the text: hello from adapter

Use file creation tools. Create the output/ directory first if needed.
Do not write any other files. Do not explain anything."

$spec_content | save --force ($tdir | path join "spec.md")
```

**Verification**: Create a test card, verify spec.md contains the prompt text.

---

### 2. Remove Mock from Default Test List

**Problem**: The spec says "Test real adapters (claude, ollama, codex)" but "mock" is included in the "all" list.

**Location**: `scripts/test-real-adapters.nu:56`

**Required Fix**:
Replace:
```nushell
let adapters_to_test = if $adapter == "all" {
  ["mock", "claude", "ollama", "codex"]
```

With:
```nushell
let adapters_to_test = if $adapter == "all" {
  ["claude", "ollama", "codex"]
```

**Note**: Mock is still used in self-tests, just not in production test runs.

**Verification**: Run `nu --no-config-file scripts/test-real-adapters.nu --help` and verify usage shows correct adapters.

---

### 3. Implement Complete Availability Checks

**Problem**: Current checks only verify command existence. Spec requires:
- ollama: Check if server is running (`curl -sf http://localhost:11434/api/tags`)
- codex: Check if OPENAI_API_KEY is set

**Location**: `scripts/test-real-adapters.nu:166-177`

**Required Fix**:
Replace the entire `is_adapter_available` function with:

```nushell
def is_adapter_available [
  adapter: string  # Adapter name (claude, ollama, or codex)
]: nothing -> bool {
  # Check if command exists
  let cmd_result = (do { ^which $adapter } | complete)
  if $cmd_result.exit_code != 0 {
    return false
  }

  # Additional checks per adapter
  if $adapter == "ollama" {
    # Check if ollama server is running
    let server_result = (do { ^curl -sf http://localhost:11434/api/tags } | complete)
    return ($server_result.exit_code == 0)
  } else if $adapter == "codex" {
    # Check if OPENAI_API_KEY is set
    return ("OPENAI_API_KEY" in $env)
  } else {
    # claude or other adapters: just check command existence
    return true
  }
}
```

**Verification**:
- Test with ollama installed but server not running → should return false
- Test with codex installed but no OPENAI_API_KEY → should return false
- Test with claude installed → should return true

---

### 4. Fix Self-Tests for Mock Adapter

**Problem**: Self-test at line 581 expects mock adapter to create `output/result.md`, but mock adapter doesn't create files.

**Location**: `scripts/test-real-adapters.nu:579-583`

**Required Fix**:
After line 577 (where card reaches done state), manually create the expected output:

```nushell
  # Verify card is actually in done directory
  let done_card = (find_card_in $test_setup.cards_dir "done" "test-dispatcher-1")
  assert ($done_card | path exists) "card should exist in done directory"

  # Mock adapter doesn't create output/result.md, so create it manually for testing
  mkdir ($done_card | path join "output")
  "test output from mock adapter" | save ($done_card | path join "output" "result.md")

  # Test: assert_result with successful card
  print "Testing assert_result..."
  let result = (assert_result $test_setup.cards_dir "test-dispatcher-1")
  assert $result.success "assert_result should succeed for completed card with output"
  assert ($result.message | str contains "bytes") "result message should contain byte count"
```

**Verification**: Run self-tests with `nu --no-config-file scripts/test-real-adapters.nu --test` and verify all assertions pass.

---

## After Fixes

Once all four fixes are complete:

1. **Commit changes**:
   ```bash
   git add scripts/test-real-adapters.nu
   git commit -m "fix: address QA issues in test-real-adapters.nu (qa-requested)

   - Add test prompt content to template spec.md
   - Remove mock from default adapter list (real adapters only)
   - Implement complete availability checks (ollama server + codex API key)
   - Fix self-tests to work with mock adapter limitations"
   ```

2. **Verify acceptance criteria** (from spec lines 68-79):
   ```bash
   # Test 1: Help flag works
   nu --no-config-file scripts/test-real-adapters.nu --help

   # Test 2: Self-tests pass
   nu --no-config-file scripts/test-real-adapters.nu --test

   # Test 3: Can invoke with adapter flag
   nu --no-config-file scripts/test-real-adapters.nu --adapter ollama --timeout 1

   # Test 4: No Rust regressions
   make check
   ```

3. **QA will automatically re-run** after the commit and verify all issues are resolved.

---

## Summary

All four issues are in the same file (`scripts/test-real-adapters.nu`) and are straightforward to fix:
- Issue 1: Add prompt text (5 lines)
- Issue 2: Remove "mock" from list (1 line)
- Issue 3: Enhance availability function (15 lines)
- Issue 4: Add output file creation in test (2 lines)

**Estimated fix time**: 15-20 minutes

The code structure and patterns are excellent. These are logic bugs that prevent the script from working, not architectural issues.
