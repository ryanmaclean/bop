#!/usr/bin/env nu
# Fail if any cargo dependency uses a non-permissive license.
# Requires: cargo install cargo-deny

def main [] {
  if (which cargo-deny | length) == 0 {
    print "cargo-deny not installed. Install with: cargo install cargo-deny --locked"
    exit 1
  }

  ^cargo deny check licenses
  print "License audit passed."
}
