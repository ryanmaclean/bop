Implemented attestation context injection for prompt rendering.

## What changed

- Added new `bop_core::attestation` module with:
  - `Attestation` data model
  - `compute_record_hash()` and `verify()` for record integrity checks
  - `format_previous_run_summary()` for human-readable prompt context
- Extended `PromptContext` with:
  - `previous_attestation: Option<String>`
- Updated `PromptContext::from_files()` to:
  - read `logs/attestation.json` when present
  - parse into `Attestation`
  - call `Attestation::verify()`
  - inject a formatted summary (including tamper warning heading when verification fails)
- Updated `render_prompt()` to replace `{{previous_attestation}}` with summary text or empty string.
- Added tests:
  - `from_files_loads_and_formats_previous_attestation`
  - `from_files_without_attestation_sets_none_and_render_replaces_empty`
- Updated prompt templates:
  - `.cards/templates/implement.bop/prompt.md`
  - `.cards/templates/full.bop/prompt.md`
  - both now include `{{previous_attestation}}` in the context header block.
- Updated `adapters/mock.nu`:
  - warns in stderr if `{{previous_attestation}}` remains unresolved.
- Ran formatter; this also applied a formatting-only line wrap in `crates/bop-cli/src/merge_gate.rs`.

## Validation

- `cargo build` ✅
- `cargo test` ✅
- `cargo clippy -- -D warnings` ✅
- `cargo fmt --check` ✅

## Commit status

Unable to run `jj describe` / `jj new` in this sandbox due filesystem permission denial on the repo lock file at:

`/Users/studio/bop/.jj/repo/git_import_export.lock`

Implementation and checks are complete; commit step is blocked by environment permissions.
