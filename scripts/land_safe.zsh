#!/usr/bin/env zsh
set -euo pipefail

ROOT=${0:A:h:h}
TARGET_BRANCH="main"
SOURCE_BRANCH=""
PUSH_REMOTE=""
SKIP_CHECKS=0

usage() {
  cat <<'USAGE'
Usage:
  scripts/land_safe.zsh [--target <branch>] [--source <branch>] [--push <remote>] [--skip-checks]

Defaults:
  --target main
  --source current branch

Behavior:
  1) Enforces clean git + clean jj working state.
  2) Runs gate checks: make check + bop policy check --staged.
  3) Fast-forwards target branch to source (ff-only).
  4) Optionally pushes target branch to remote.

Examples:
  scripts/land_safe.zsh --target release/v0.3.0-factory
  scripts/land_safe.zsh --target main --push origin
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target)
      TARGET_BRANCH="${2:-}"
      shift 2
      ;;
    --source)
      SOURCE_BRANCH="${2:-}"
      shift 2
      ;;
    --push)
      PUSH_REMOTE="${2:-}"
      shift 2
      ;;
    --skip-checks)
      SKIP_CHECKS=1
      shift
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      echo "Unknown arg: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

cd "$ROOT"

if [[ -z "$SOURCE_BRANCH" ]]; then
  SOURCE_BRANCH="$(git branch --show-current)"
fi

if [[ -z "$SOURCE_BRANCH" || -z "$TARGET_BRANCH" ]]; then
  echo "source/target branch must be non-empty" >&2
  exit 2
fi

HAS_JJ=1
if ! command -v jj >/dev/null 2>&1; then
  HAS_JJ=0
fi

if [[ -n "$(git status --porcelain)" ]]; then
  echo "Refusing to land: git working tree is dirty." >&2
  git status --short >&2
  exit 1
fi

if [[ $HAS_JJ -eq 1 ]]; then
  if jj status --no-pager >/dev/null 2>&1; then
    jj_status="$(jj status --no-pager 2>/dev/null || true)"
    if [[ "$jj_status" != *"The working copy is clean"* ]]; then
      echo "Refusing to land: jj working copy is not clean." >&2
      echo "$jj_status" >&2
      exit 1
    fi
  else
    echo "Note: jj is installed but this repo is not initialized for jj; using git-only safety checks." >&2
  fi
fi

if ! git show-ref --verify --quiet "refs/heads/${SOURCE_BRANCH}"; then
  echo "Unknown source branch: ${SOURCE_BRANCH}" >&2
  exit 1
fi
if ! git show-ref --verify --quiet "refs/heads/${TARGET_BRANCH}"; then
  echo "Unknown target branch: ${TARGET_BRANCH}" >&2
  exit 1
fi

if [[ "$SOURCE_BRANCH" == "$TARGET_BRANCH" ]]; then
  echo "Source and target are the same branch (${SOURCE_BRANCH}); nothing to do." >&2
  exit 1
fi

checked_out_elsewhere=0
while IFS= read -r line; do
  if [[ "$line" == "branch refs/heads/${TARGET_BRANCH}" ]]; then
    checked_out_elsewhere=1
    break
  fi
done < <(git worktree list --porcelain)

if [[ $checked_out_elsewhere -eq 1 ]]; then
  echo "Refusing to move ${TARGET_BRANCH}: it is currently checked out in a worktree." >&2
  exit 1
fi

if [[ $SKIP_CHECKS -eq 0 ]]; then
  BOP_BIN="${BOP_BIN:-$ROOT/target/debug/bop}"
  if [[ ! -x "$BOP_BIN" ]]; then
    BOP_BIN="$ROOT/target/debug/jc"
  fi
  if [[ ! -x "$BOP_BIN" ]]; then
    echo "Missing bop/jc binary. Run: cargo build" >&2
    exit 1
  fi

  echo "Running gates..."
  make check
  "$BOP_BIN" policy check --staged
fi

if ! git merge-base --is-ancestor "$TARGET_BRANCH" "$SOURCE_BRANCH"; then
  echo "Refusing non-FF landing: ${TARGET_BRANCH} is not an ancestor of ${SOURCE_BRANCH}." >&2
  exit 1
fi

git branch -f "$TARGET_BRANCH" "$SOURCE_BRANCH"
echo "Fast-forwarded ${TARGET_BRANCH} -> ${SOURCE_BRANCH}"

if [[ -n "$PUSH_REMOTE" ]]; then
  git push "$PUSH_REMOTE" "${TARGET_BRANCH}:${TARGET_BRANCH}"
  echo "Pushed ${TARGET_BRANCH} to ${PUSH_REMOTE}"
fi

echo "Done."
