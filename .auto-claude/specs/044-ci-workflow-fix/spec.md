# Spec 044 — GitHub Actions CI: fix workflow + badge

## Overview

The README has a CI badge:
`![CI](https://github.com/ryanmaclean/bop/actions/workflows/ci.yml/badge.svg)`

The `.github/workflows/ci.yml` workflow may be missing, broken, or using a
self-hosted runner that is no longer available. This spec ensures CI runs
`make check` (cargo test + clippy + fmt) on every push and PR to `main`.

## Requirements

### 1. Workflow file

Create or fix `.github/workflows/ci.yml`:
```yaml
name: CI
on:
  push:
    branches: [main]
  pull_request:
    branches: [main]
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy, rustfmt
      - uses: Swatinem/rust-cache@v2
      - name: make check
        run: make check
```

### 2. Makefile target

Ensure `Makefile` has a `check` target that runs:
```
cargo test
cargo clippy -- -D warnings
cargo fmt --check
```

The existing `make check` should already do this. Verify and fix if broken.

### 3. Platform portability

The CI runs on `ubuntu-latest`. Ensure:
- No macOS-specific code is invoked in tests (launchctl, NSWorkspace, etc.)
- Tests that require macOS-specific syscalls are `#[cfg(target_os = "macos")]`
  gated or use `#[ignore]` on Linux
- `bop factory` commands that call `launchctl` are no-op or return a clear
  error on Linux

## Acceptance Criteria

- [ ] `.github/workflows/ci.yml` exists and is valid YAML
- [ ] Workflow triggers on push to main and on PRs
- [ ] `cargo test` passes on ubuntu-latest (fix any Linux-specific failures)
- [ ] `cargo clippy -- -D warnings` passes on ubuntu-latest
- [ ] `cargo fmt --check` passes
- [ ] The CI badge URL returns a passing badge after next push to main

## Files to create/modify

- `.github/workflows/ci.yml` — workflow definition
- `Makefile` — verify `check` target is correct
- `crates/bop-cli/src/factory.rs` — gate launchctl calls on `#[cfg(target_os = "macos")]`
- Any test files with macOS-only syscalls
