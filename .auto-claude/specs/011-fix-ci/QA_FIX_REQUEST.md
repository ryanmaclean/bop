# QA Fix Request

**Status**: REJECTED
**Date**: 2026-03-05
**QA Session**: 1

---

## Critical Issues to Fix

### 1. CI Command Missing Clippy Error Flag
**Problem**: The CI runs `cargo clippy` without `-- -D warnings`, so clippy warnings won't fail the build.

**Location**: `.github/workflows/ci.yml:20`

**Current Code**:
```yaml
run: cargo test && cargo clippy
```

**Required Fix**:
```yaml
run: cargo test && cargo clippy -- -D warnings && cargo fmt --check
```

**Verification**: After fixing, introduce a clippy warning and confirm CI fails. Then remove the warning and confirm CI passes.

---

### 2. CI Command Missing Format Check
**Problem**: The CI doesn't run `cargo fmt --check`, so formatting violations won't fail the build.

**Location**: `.github/workflows/ci.yml:20`

**Current Code**:
```yaml
run: cargo test && cargo clippy
```

**Required Fix**:
```yaml
run: cargo test && cargo clippy -- -D warnings && cargo fmt --check
```

**Verification**: After fixing, introduce a formatting violation (e.g., remove a newline) and confirm CI fails. Then fix formatting and confirm CI passes.

---

### 3. Missing Rust Toolchain Components
**Problem**: The rust-toolchain action doesn't explicitly install clippy and rustfmt components.

**Location**: `.github/workflows/ci.yml:16-17`

**Current Code**:
```yaml
- name: Install Rust toolchain
  uses: dtolnay/rust-toolchain@stable
```

**Required Fix**:
```yaml
- name: Install Rust toolchain
  uses: dtolnay/rust-toolchain@stable
  with:
    components: clippy, rustfmt
```

**Verification**: Check workflow logs confirm components are explicitly installed.

---

## Optional Enhancement

### 4. Missing Rust Cache Action (Performance Optimization)
**Problem**: The workflow doesn't cache Rust dependencies, making builds slower.

**Location**: After line 17 in `.github/workflows/ci.yml`

**Required Fix**: Add this step after the rust-toolchain installation:
```yaml
- name: Cache Rust dependencies
  uses: Swatinem/rust-cache@v2
```

**Verification**: Check workflow logs show cache hits on subsequent runs.

---

## Why This Matters

The spec explicitly defined these quality gates:

```yaml
- uses: dtolnay/rust-toolchain@stable
  with:
    components: clippy, rustfmt
- uses: Swatinem/rust-cache@v2
- name: make check
  run: cargo test && cargo clippy -- -D warnings && cargo fmt --check
```

Without the `-- -D warnings` flag and `cargo fmt --check`:
- Code with clippy warnings can be merged (potential bugs, performance issues)
- Code with formatting violations can be merged (inconsistent style)

This weakens the CI's quality guarantees compared to what `make check` provides locally.

---

## After Fixes

Once fixes are complete:

1. **Test locally**: Run `make check` to verify it still passes
2. **Commit**: Use message like:
   ```
   fix(ci): add missing quality gates to CI workflow (qa-requested)

   - Add -- -D warnings to clippy to fail on warnings
   - Add cargo fmt --check to enforce formatting
   - Add components: clippy, rustfmt to rust-toolchain
   - Add rust-cache for faster builds

   Fixes QA rejection from session 1.
   ```
3. **QA will automatically re-run** and validate the fixes

---

## Expected Outcome

After these fixes:
- CI will fail on clippy warnings ✓
- CI will fail on formatting violations ✓
- CI will have all components explicitly installed ✓
- CI will be faster with caching ✓
- Implementation will match the spec ✓
