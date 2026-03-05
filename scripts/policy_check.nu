#!/usr/bin/env nu
# Policy check script — thin shim over `bop policy check`.
# The heavy lifting is now in Rust (crates/bop-cli/src/policy.rs).
# This script is kept for backward-compat with the dispatcher's
# run_policy_script() call path and for the --test self-test flag.

def run_tests [] {
  use std/assert

  # Test 1: bop binary is available
  let bop_exists = ((which bop | length) > 0)
  assert $bop_exists "bop binary must be on PATH"
  print "  test 1 ok: bop binary found"

  print "PASS: policy_check.nu"
}

def main [
  --mode: string = "staged"    # staged or card
  --json                       # Print JSON result
  --cards-dir: string = ".cards"
  --card-dir: string = ""
  --id: string = ""
  --repo-root: string = ""
  --staged                     # Shortcut for --mode staged
  --help (-h)                  # Show usage
  --test                       # Run self-tests
] {
  if $test {
    run_tests
    return
  }

  if $help {
    print "Usage:"
    print "  scripts/policy_check.nu --staged [--cards-dir .cards]"
    print "  scripts/policy_check.nu --mode card --card-dir <path>"
    print ""
    print "Delegates to: bop policy check"
    exit 0
  }

  # Delegate to Rust binary
  let flags = if $json { ["--json"] } else { [] }
  ^bop policy check ...$flags
}
