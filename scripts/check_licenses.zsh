#!/usr/bin/env zsh
# Fail if any cargo dependency uses a non-permissive license.
# Requires: cargo install cargo-deny
set -euo pipefail

if ! command -v cargo-deny &>/dev/null; then
  echo "cargo-deny not installed. Install with: cargo install cargo-deny --locked"
  exit 1
fi

cargo deny check licenses 2>&1
echo "License audit passed."
