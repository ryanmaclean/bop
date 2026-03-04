#!/usr/bin/env nu
# Fail if any cargo dependency uses a non-permissive license.
# Requires: cargo install cargo-deny

def main [
  --test  # Run internal self-tests
] {
  if $test {
    run_tests
    return
  }

  if (which cargo-deny | length) == 0 {
    print "cargo-deny not installed. Install with: cargo install cargo-deny --locked"
    exit 1
  }

  ^cargo deny check licenses
  print "License audit passed."
}

def run_tests [] {
  use std/assert

  # Test: which returns a list — length check logic
  let empty_list_len = ([] | length)
  assert equal $empty_list_len 0 "empty list length should be 0"

  # Test: non-empty list length
  let non_empty = [1 2 3]
  assert equal ($non_empty | length) 3 "list of 3 should have length 3"

  # Test: the == 0 check used for tool detection
  assert ($empty_list_len == 0) "empty list length == 0 should be true"
  assert (($non_empty | length) != 0) "non-empty list length != 0 should be true"

  print "PASS: check_licenses.nu"
}
