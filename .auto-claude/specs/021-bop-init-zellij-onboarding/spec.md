# bop init: zero-config Zellij onboarding

## Context

`bop init` creates `.cards/` but doesn't help the user get running. New users
don't know they need Zellij, that the web client needs to be enabled, or what
session name to use. This spec makes `bop init` self-configuring and prints a
clear quick-start.

## What to do

1. **`bop init` Zellij detection** (`crates/bop-cli/src/main.rs` or a new
   `init.rs`):
   - After creating `.cards/`, check `std::env::var("ZELLIJ_SESSION_NAME")`.
   - If inside Zellij: write `{"zellij_session": "<name>"}` to `.cards/config.json`.
     Print: `✓ Zellij session detected: <name>`
   - If NOT inside Zellij: print a hint:
     ```
     ℹ  Not running inside Zellij. For live card links, start bop inside a
        Zellij session: zellij --session bop
        Then re-run: bop init
     ```

2. **Write `zellij_session` to `.cards/config.json`**: define a `CardsConfig`
   struct in `bop-core` with `zellij_session: Option<String>`. Dispatcher reads
   this to populate `meta.json` at spawn time (feeds spec 017).

3. **Quick-start output** at end of `bop init`:
   ```
   ✓ bop workspace ready at .cards/

   Quick start:
     bop new agent my-task       # create a card
     bop list                    # see all cards
     bop dispatcher --once       # run one card
     bop gantt                   # timeline view

   Docs: https://github.com/ryanmaclean/bop
   ```

4. **`bop doctor` subcommand** (new): checks system health and prints a summary:
   - ✓/✗ `nu` on PATH
   - ✓/✗ `claude` CLI available
   - ✓/✗ Running inside Zellij
   - ✓/✗ Zellij web client listening on :8082
   - ✓/✗ `.cards/` exists and is writable
   - ✓/✗ `make check` clean (skip if `--fast`)

5. Run `make check` — must pass.

6. Write `output/result.md` showing sample `bop init` and `bop doctor` output.

## Acceptance

- `bop init` detects Zellij session and writes to `.cards/config.json`
- `bop init` prints a clear quick-start guide
- `bop doctor` exists and checks all 6 conditions
- `make check` passes
- `output/result.md` exists
