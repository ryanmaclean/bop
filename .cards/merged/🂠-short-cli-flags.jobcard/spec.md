# Short CLI Flags for dispatcher and merge-gate

Double-hyphen long flags are token-heavy and confusing (`--` looks like an
em-dash). Add short aliases for the most-used dispatcher and merge-gate flags.

## Changes (crates/jc/src/main.rs)

For `Command::Dispatcher`, add `short` to these `#[arg(...)]` attributes:

```rust
// adapter: -a
#[arg(short = 'a', long, default_value = "adapters/mock.zsh")]
adapter: String,

// max_workers: -w
#[arg(short = 'w', long)]
max_workers: Option<usize>,

// once: -1
#[arg(short = '1', long)]
once: bool,

// vcs_engine: -v  (both Dispatcher and MergeGate)
#[arg(short = 'v', long, value_enum, default_value_t = VcsEngine::GitGt)]
vcs_engine: VcsEngine,
```

For `Command::MergeGate`, add:
```rust
// once: -1
#[arg(short = '1', long)]
once: bool,

// vcs_engine: -v
#[arg(short = 'v', long, value_enum, default_value_t = VcsEngine::GitGt)]
vcs_engine: VcsEngine,
```

The VcsEngine enum values stay as `git-gt` and `jj` (clap kebab-cases them).
So `-v j` means jj, `-v g` means git-gt.

## Acceptance Criteria

- `cargo build`
- `cargo clippy -- -D warnings`
- `./target/debug/bop dispatcher -v j --help 2>&1 | grep -q vcs`
- `jj log -r 'main..@-' | grep -q .`

## Scope

Touch only `crates/jc/src/main.rs` — just add `short = '...'` to the arg attrs.
