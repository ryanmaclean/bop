# Shell completions (bash / zsh / fish)

## Goal

Add `bop completions <shell>` command that outputs a valid completion script
for bash, zsh, or fish using clap's built-in `generate` facility.

## Context

Card spec: `.cards/team-cli/failed/shell-completions.jobcard/spec.md`.
The CLI uses `clap` with `derive` feature. Add `clap_complete` as a dev/build dep.

## Steps

1. Add `clap_complete` to `crates/bop-cli/Cargo.toml`:
   ```toml
   [dependencies]
   clap_complete = "4"
   ```

2. Add `Completions` variant to `Command` enum:
   ```rust
   Completions {
       #[arg(value_enum)]
       shell: clap_complete::Shell,
   }
   ```

3. Dispatch arm:
   ```rust
   Command::Completions { shell } => {
       let mut cmd = <Cli as CommandFactory>::command();
       generate(shell, &mut cmd, "bop", &mut std::io::stdout());
       Ok(())
   }
   ```

4. Smoke-test:
   ```sh
   ./target/debug/bop completions bash | head -5
   ./target/debug/bop completions zsh  | head -5
   ./target/debug/bop completions fish | head -5
   ```

5. Document install steps in a brief comment in the command's doc string.

6. Run `make check`.

## Acceptance

`bop completions bash|zsh|fish` exits 0 and outputs a non-empty completion script.
`make check` passes.
