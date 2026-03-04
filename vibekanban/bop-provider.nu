#!/usr/bin/env nu
# JobCard provider for vibekanban-cli.
# Outputs all job cards as a JSON array to stdout.
# Usage: CARDS_DIR=.cards nu vibekanban/bop-provider.nu

def main [] {
  let cards_dir = ($env.CARDS_DIR? | default ".cards")

  if not ($cards_dir | path exists) {
    print --stderr $"ERROR: CARDS_DIR '($cards_dir)' does not exist"
    exit 1
  }

  let py_script = $"($env.FILE_PWD)/_bop_provider_impl.py"
  ^python3 $py_script $cards_dir
}
