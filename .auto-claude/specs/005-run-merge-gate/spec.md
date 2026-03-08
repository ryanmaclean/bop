# Run merge-gate on done/ cards

## Goal

Run `./target/debug/bop merge-gate --once` repeatedly until all cards in `done/`
(and team subdirs) have been processed to `merged/` or `failed/`.

## Prerequisites

- `cargo build` passes
- Cards exist in `.cards/team-arch/done/` (5 cards) and `.cards/team-quality/done/` (5 cards)

## Steps

1. Build the binary:
   ```sh
   cargo build
   ```

2. Check how many cards are in done/:
   ```sh
   ./target/debug/bop list --state done
   ```

3. Run merge-gate until done/ is empty:
   ```sh
   for i in (seq 1 12) {
     ./target/debug/bop merge-gate --once
     sleep 2sec
   }
   ```

4. Verify cards moved to merged/:
   ```sh
   ./target/debug/bop list --state all
   ```
   Expected: `team-arch/done (0)`, `team-quality/done (0)`,
   and counts in `merged/` increased.

5. Run `make check`.

## Acceptance

`./target/debug/bop list --state done` shows 0 cards in all done/ directories.
`make check` passes.
