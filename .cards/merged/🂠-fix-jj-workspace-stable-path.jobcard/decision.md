# Decision: Stable jj workspace path

## Summary
Fix `prepare_workspace` Jj arm to use a stable path at `<repo_root>/.workspaces/<ws_name>/`
instead of `card_dir/workspace`, mirroring the GitGt fix (ef9446a).

## Rationale
When the card bundle renames (`running/ → done/ → merged/`), the workspace path recorded
in `meta.json` becomes invalid. The stable path outside the card bundle survives these
renames and keeps the jj workspace registered and accessible.

## Decision
Approved — this is a bug fix with no behavioral changes beyond path stability.
