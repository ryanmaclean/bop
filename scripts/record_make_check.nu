#!/usr/bin/env nu
# Run `make check` and record the result with a timestamp.

def main [] {
  let root = ($env.FILE_PWD | path dirname)
  let log_file = ($root | path join ".cards" "promotion" "make_check_runs.log")
  let audit_file = $"($log_file).audit"
  mkdir ($log_file | path dirname)

  mut status = "pass"
  let result = do { ^make -C $root check } | complete
  if $result.exit_code != 0 {
    $status = "fail"
  }

  let ts = (date now | format date "%Y-%m-%dT%H:%M:%SZ")
  $status | save --append $log_file
  "\n" | save --append $log_file
  $"($ts) ($status)\n" | save --append $audit_file

  print $"Recorded make check result: ($status)"

  if $status != "pass" {
    exit 1
  }
}
