# Contributing to bop

## Development Setup

```sh
git clone https://github.com/ryanmaclean/bop.git
cd bop
cargo build
cargo test
```

**Requirements:** Rust 1.75+, Nushell 0.100+, macOS (primary) or Linux.

## Running Tests

```sh
make check           # cargo test + clippy + fmt check
cargo test -p bop-core   # single crate
```

## Adding an Adapter

Adapters live in `adapters/` — one Nushell script per AI provider.

Calling convention:
```
adapter.nu <workdir> <prompt_file> <stdout_log> <stderr_log> [timeout]
```

Exit codes:
- `0` — success (card moves to `done/`)
- `75` — rate-limited (`EX_TEMPFAIL`, triggers provider rotation)
- anything else — failure (card moves to `failed/`)

Copy `adapters/mock.nu` as a starting point.

## Adding a Template

Templates live in `.cards/templates/`. Each template is a `.card/` directory bundle:

```
my-template.card/
├── meta.json     ← stage, provider_chain, acceptance_criteria
├── spec.md       ← what to build (use {{variables}} for substitution)
├── prompt.md     ← agent prompt template
├── logs/
└── output/
```

`bop new my-template some-id` COW-clones the template into `.cards/pending/`.

## Code Style

- `cargo fmt` before committing
- `cargo clippy -- -D warnings` must pass (warnings = errors)
- All shell scripts: `#!/usr/bin/env nu` (Nushell, MIT licensed)
- Python scripts (in `scripts/`): Python 3.9+, dd-trace instrumented via `bop_trace.py`

## Card Symbol Protocol

See the main README for the glyph encoding table. When creating cards programmatically, set both `glyph` (SMP playing card) and `token` (BMP symbol) in `meta.json`.

**Filename rule:** Never use SMP characters (U+10000+) in filenames — shell/find/Rust tooling breaks on 4-byte UTF-8. Use BMP-only `token` for filename suffixes.

## Pull Requests

1. Fork and branch from `main`
2. `make check` must pass
3. Keep commits focused — one logical change per commit
4. Update docs if you change CLI behavior or card format
