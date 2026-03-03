# Contributing to bop

Thank you for your interest in contributing to `bop`! This document covers how to validate your changes before submitting a pull request.

## Prerequisites

- [shellcheck](https://www.shellcheck.net/) — static analysis for shell scripts

```sh
# Install shellcheck
brew install shellcheck   # macOS
apt install shellcheck    # Debian/Ubuntu
```

## Validating shell scripts

All shell scripts in the spec and codebase must pass `shellcheck`. This includes the reference scripts in `SPEC.md`.

```sh
# Check the atomic state transition script (§9 of SPEC.md)
shellcheck -s sh <<'EOF'
#!/bin/sh
TASK_DIR="$1"; NEW_STATE="$2"; ACTOR="$3"; REASON="$4"
printf '%s\n' "$NEW_STATE" > "$TASK_DIR/.bop/state.tmp"
mv "$TASK_DIR/.bop/state.tmp" "$TASK_DIR/.bop/state"
printf '%s %s %s "%s"\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  "$NEW_STATE" "$ACTOR" "$REASON" >> "$TASK_DIR/.bop/transitions.log"
sed -i "s/^State: .*/State: $NEW_STATE/" "$TASK_DIR/task.bop"
EOF
```

## Making changes

1. Fork the repository and create a branch from `main`.
2. Make your changes. Keep diffs focused — one logical change per PR.
3. If you modify any shell scripts (including reference scripts in `SPEC.md`), run `shellcheck` on them.
4. Open a pull request with a clear description of what changed and why.

## Spec changes

When proposing changes to `SPEC.md`:

- Check that all `.bop` bundle directory tree examples are consistent with the spec text.
- Ensure any new shell reference scripts pass `shellcheck -s sh`.
- Update the Open Questions section (§11) if your change resolves or introduces a design question.

## Style

- Line endings: LF (the spec mandates this).
- Encoding: UTF-8.
- Markdown: follow the existing heading and code-fence style in `SPEC.md` and `README.md`.
