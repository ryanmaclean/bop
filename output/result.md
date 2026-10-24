# Spec 064 — Gemini Flash adapter implementation

Implemented Gemini Flash adapter integration for bop.

## What changed

- Added `adapters/gemini.nu` with standard adapter calling convention:
  - Signature: `<workdir> <prompt_file> <stdout_log> <stderr_log> [memory_out]`
  - Executes `gemini -m gemini-2.5-flash-preview -p -`
  - Maps Gemini quota/rate-limit style errors to `exit 75`
  - Includes `--test` self-tests for path handling and rate-limit detection

- Updated dispatcher provider adapter resolution in `crates/bop-cli/src/dispatcher.rs`:
  - `gemini-flash` now resolves to `adapters/gemini.nu`
  - Connectivity probe now recognizes `gemini`/`gemini-flash` endpoint

- Updated default provider seeding in `crates/bop-cli/src/providers/mod.rs`:
  - Added `gemini-flash` provider entry when `providers.json` is auto-seeded:
    - `command: adapters/gemini.nu`
    - `rate_limit_exit: 75`
    - `model: gemini-2.5-flash-preview`

## Validation

- `nu --no-config-file adapters/gemini.nu --test` ✅
- `cargo fmt --check` ✅
- `cargo build` ✅
- `cargo test` ✅
- `cargo clippy -- -D warnings` ✅
- `./target/debug/bop dispatcher --adapter adapters/gemini.nu --once` ✅

## Notes

- `bop` CLI binary is not installed on PATH in this environment, so optional `bop bridge emit ...` telemetry calls were skipped.
