Implemented attestation context injection for prompt rendering.

## What changed
- Added new module `crates/bop-core/src/attestation.rs` with:
  - `Attestation` struct
  - `verify()` integrity check based on deterministic `record_hash`
  - `summary_for_prompt()` formatter for prompt injection (verified/tampered variants)
- Updated `crates/bop-core/src/lib.rs`:
  - Added `pub mod attestation;`
  - Extended `PromptContext` with `previous_attestation: Option<String>`
  - In `PromptContext::from_files`, load `logs/attestation.json` when present, parse attestation, run `verify()`, and set formatted `previous_attestation`
  - In `render_prompt`, replace `{{previous_attestation}}` with summary or empty string
  - Added tests:
    - `prompt_context_from_files_loads_previous_attestation_summary`
    - `prompt_context_from_files_without_attestation_sets_none_and_renders_empty_placeholder`
- Updated templates:
  - `.cards/templates/implement.bop/prompt.md`
  - `.cards/templates/full.bop/prompt.md`
  - Added `{{previous_attestation}}` in the system-context region
- Updated `adapters/mock.nu` self-test prompt fixture to include an attestation block and assert it is preserved in output
- Formatting-only change (rustfmt) in `crates/bop-cli/src/merge_gate.rs`

## Validation
- `cargo fmt --check`: passed
- `cargo build`: passed once before artifact-lock contention
- `cargo test`: ran and showed broad pass output (unit + dispatcher harness), but later invocations encountered long-running cargo lock/session contention in this sandbox
- `cargo clippy -- -D warnings`: blocked by the same long-running cargo lock/session contention

## Environment blockers
- `jj` commands are blocked in this sandbox:
  - `jj status` fails with `Operation not permitted` on `/Users/studio/bop/.jj/repo/git_import_export.lock`
- Because of that `.jj` lock-path permission issue, I could not create the required `jj` commit in this environment.
