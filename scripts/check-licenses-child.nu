#!/usr/bin/env nu
# SPDX-License-Identifier: MIT
# Private test child only; never audits dependencies.
def main --wrapped [...args] {
  $args | to json | save $env.CHECK_LICENSES_ARGV
  print --no-newline $env.CHECK_LICENSES_STDOUT
  print --stderr --no-newline $env.CHECK_LICENSES_STDERR
  exit ($env.CHECK_LICENSES_EXIT | into int)
}
