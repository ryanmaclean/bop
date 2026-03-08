# bop clean command

## Goal

Implement `bop clean` CLI command to remove stale/corrupt cards and temp artifacts.

## Context

Card spec is at `.cards/team-cli/failed/clean-command.jobcard/spec.md`.

## What to clean

- Cards in `failed/` older than N days (default: 7), with `--dry-run` to preview
- Cards with no `meta.json` (corrupt bundles)
- Empty `running/` cards whose PID is no longer alive (orphans)
- `target/` directories inside card bundles (can accumulate from merge-gate bug)

## Steps

1. Add `Clean` variant to `Command` enum:
   ```
   bop clean [--dry-run] [--older-than <days>] [--state <state>]
   ```

2. Implement scan logic:
   - Walk all state dirs across root + team dirs
   - Identify cards matching clean criteria
   - In dry-run: print what would be removed
   - Without dry-run: `fs::remove_dir_all` each target

3. Print summary: `Removed N cards, freed X MB`

4. Add unit tests with tempdir fixtures.

5. Run `make check`.

## Acceptance

`bop clean --dry-run` lists stale/corrupt cards without deleting.
`bop clean` removes them and prints a summary.
`make check` passes.
