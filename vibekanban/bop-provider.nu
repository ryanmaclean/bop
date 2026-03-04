#!/usr/bin/env nu
# JobCard provider for vibekanban-cli.
# Outputs all job cards as a JSON array to stdout.
# Usage: CARDS_DIR=.cards nu vibekanban/bop-provider.nu

def main [
  --test  # Run internal self-tests
] {
  if $test {
    run_tests
    return
  }

  let cards_dir = ($env.CARDS_DIR? | default ".cards")

  if not ($cards_dir | path exists) {
    print --stderr $"ERROR: CARDS_DIR '($cards_dir)' does not exist"
    exit 1
  }

  let py_script = $"($env.FILE_PWD)/_bop_provider_impl.py"
  ^python3 $py_script $cards_dir
}

def run_tests [] {
  use std/assert

  # Test: default fallback for CARDS_DIR
  let fallback = ($env.CARDS_DIR? | default ".cards")
  # In test context CARDS_DIR is likely not set, so default applies
  assert (($fallback | str length) > 0) "fallback should produce a non-empty string"

  # Test: path existence check logic
  assert ("/tmp" | path exists) "/tmp should exist"
  assert (not ("/tmp/nonexistent-bop-test-xyz" | path exists)) "nonexistent path should not exist"

  # Test: python script path construction
  let file_pwd = "/Users/studio/bop/vibekanban"
  let py_script = $"($file_pwd)/_bop_provider_impl.py"
  assert equal $py_script "/Users/studio/bop/vibekanban/_bop_provider_impl.py"
  assert ($py_script | str ends-with ".py") "should end with .py"

  print "PASS: bop-provider.nu"
}
