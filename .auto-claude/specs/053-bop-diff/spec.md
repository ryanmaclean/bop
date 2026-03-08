# Spec 053 — bop diff: show card output diff

## Overview

After an agent completes a card, the most common question is: "what did it
actually change?" `bop diff <id>` answers this without leaving the terminal.

## Commands

```sh
bop diff <id>              # show git diff of changes the card produced
bop diff <id> --stat       # summary only (files changed, insertions, deletions)
bop diff <id> --output     # show output/result.md instead of git diff
bop diff <id> --worktree   # cd into the card worktree and open $EDITOR
```

## Implementation

### Finding the diff

1. Read `meta.json` for the card's `worktree` path and `merge_commit` (if merged).
2. If merged: `git diff <merge_commit>^..<merge_commit>` in the repo root.
3. If done (not yet merged): `git diff HEAD` in `card.worktree` path (jj or git).
4. If no worktree: show `output/result.md` content with a note that no code diff is available.

### Output

Pipe through `delta` if available (check PATH), otherwise use raw `git diff`
with `--color=always`. Page with `$PAGER` (or `less -R`) if output > terminal height.

### `--worktree` flag

If the card has a worktree and it still exists on disk:
```
print "cd <worktree_path>"
exec $EDITOR <worktree_path>
```
If worktree is gone (merged and cleaned): print the merge commit hash and exit.

## Card lookup

`bop diff <id>` searches all state dirs (done, merged, failed, running) for
`<id>.bop/`. Partial id match: if only one card matches the prefix, use it.
Multiple matches: list them and exit 1.

## Acceptance Criteria

- [ ] `bop diff <id>` shows git diff for a merged card
- [ ] `bop diff <id>` shows worktree diff for a done (unmerged) card
- [ ] `--stat` shows summary line only
- [ ] `--output` shows output/result.md content
- [ ] Partial id matching works (e.g. `bop diff spec-041` matches `team-arch/spec-041`)
- [ ] Uses `delta` for colour if present, raw diff otherwise
- [ ] Clear error if card id not found
- [ ] `cargo clippy -- -D warnings` clean
- [ ] `cargo test` passes

## Files

- `crates/bop-cli/src/diff.rs` — new module
- `crates/bop-cli/src/main.rs` — wire `bop diff` subcommand
