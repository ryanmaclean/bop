# Spec 045 — bop doctor: hardened diagnostics

## Overview

`bop doctor` exists but may only perform basic checks. This spec makes it a
comprehensive self-diagnostic that catches the most common setup problems
before they cause silent failures.

## Checks to add

### Environment checks
- `nu` installed and version ≥ 0.100.0 (`nu --version`)
- `jj` installed (for `--vcs-engine jj`)
- `git` installed
- `codex` CLI installed (for codex adapter)
- `zellij` installed (for `bop ui` / dispatch.nu)
- `cargo` installed (for merge-gate acceptance criteria)

### Filesystem checks
- `.cards/` directory exists (or print "run bop init")
- `.cards/pending/`, `.cards/running/`, `.cards/done/`, `.cards/merged/`, `.cards/failed/` all exist
- `.cards/.locks/` exists
- `.cards/providers.json` exists and is valid JSON
- `.cards/templates/implement.bop/` exists

### Adapter checks
- For each adapter in `adapters/*.nu`: check the file is executable (`-x`)
- Validate `adapters/mock.nu` can be invoked with `--test` flag

### Provider checks
- For each registered provider in `.cards/providers.json`: check credentials exist
  - claude: check `~/.claude/credentials` or keychain entry exists
  - codex: check `~/.codex/auth.json` exists
  - gemini: check `~/.gemini/credentials.json` exists
  - ollama: check `curl -s http://localhost:11434/api/tags` responds

### Config checks
- `.cards/.bop/config.json` is valid JSON (if it exists)
- `max_workers` is a positive integer

## Output format

```
bop doctor
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  Environment
  ✓ nu 0.111.0
  ✓ jj 0.27.0
  ✓ git 2.49.0
  ✗ codex — not found (install: npm i -g @openai/codex)
  ✓ zellij 0.41.2

  Filesystem
  ✓ .cards/ exists
  ✓ state directories (5/5)
  ✗ .cards/providers.json — missing (run: bop init)

  Providers
  ✓ claude credentials found
  ⚠ codex — CLI not installed, skipping credential check
  ✗ ollama — not responding at localhost:11434

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  2 errors, 1 warning
  Run with --fix to auto-repair what's possible
```

## `--fix` flag

Auto-repair where safe:
- Create missing state directories
- Copy default `providers.json` from template if missing

## Acceptance Criteria

- [ ] `bop doctor` runs without panicking
- [ ] All 5 check categories shown (env, fs, adapters, providers, config)
- [ ] `--fix` creates missing state dirs
- [ ] Exit code 0 if no errors, 1 if any error
- [ ] `cargo clippy -- -D warnings` clean
- [ ] `cargo test` passes

## Files to modify

- `crates/bop-cli/src/doctor.rs` — add all new checks
- `crates/bop-cli/src/main.rs` — wire `--fix` flag
