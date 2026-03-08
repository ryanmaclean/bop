# Fix CI: make workflows actually run

## Problem

`.github/workflows/ai-coding-job.yml` requires `runs-on: self-hosted` with a bop
daemon — dead without dedicated infrastructure. It runs on every PR label event
and does nothing useful in GHA cloud.

`.github/workflows/repo-health.yml` and `release.yml` — unknown state, may work.

## What to do

### 1. Replace ai-coding-job.yml with a real CI check

The most valuable CI for bop right now is: does `make check` pass on every push?

Replace `ai-coding-job.yml` with `ci.yml`:

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

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
        run: cargo test && cargo clippy -- -D warnings && cargo fmt --check
```

### 2. Audit repo-health.yml and release.yml

- Read each workflow
- If it references `self-hosted` or `bop serve` and can't run in GHA cloud: fix or remove
- If it's valid cloud CI: leave it

### 3. Add a badge to README if one exists

```markdown
![CI](https://github.com/ryanmaclean/bop/actions/workflows/ci.yml/badge.svg)
```

## Steps

1. Delete `.github/workflows/ai-coding-job.yml`
2. Create `.github/workflows/ci.yml` with the content above
3. Read and audit `repo-health.yml` and `release.yml` — fix or note issues
4. `make check` locally to confirm it would pass
5. Commit

## Acceptance

`.github/workflows/ci.yml` exists with `runs-on: ubuntu-latest`.
`ai-coding-job.yml` removed or rewritten to not silently fail.
`make check` passes.
