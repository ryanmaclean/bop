#!/usr/bin/env nu
# Run `make check` and record the result with a timestamp.

def format_audit_line [status: string]: nothing -> string {
  let ts = (date now | format date "%Y-%m-%dT%H:%M:%SZ")
  $"($ts) ($status)"
}

def main [
  --test  # Run internal self-tests
] {
  if $test {
    run_tests
    return
  }

  let root = ($env.FILE_PWD | path dirname)
  let log_file = ($root | path join ".cards" "promotion" "make_check_runs.log")
  let audit_file = $"($log_file).audit"
  mkdir ($log_file | path dirname)

  mut status = "pass"
  let result = do { ^make -C $root check } | complete
  if $result.exit_code != 0 {
    $status = "fail"
  }

  $status | save --append $log_file
  "\n" | save --append $log_file
  let audit_line = (format_audit_line $status)
  $"($audit_line)\n" | save --append $audit_file

  print $"Recorded make check result: ($status)"

  if $status != "pass" {
    exit 1
  }
}

def run_tests [] {
  use std/assert

  # Test: format_audit_line produces correct format
  let line = (format_audit_line "pass")
  assert ($line | str ends-with " pass") "audit line should end with status"
  # Timestamp format: 2026-03-04T12:00:00Z <status>
  assert ($line =~ '^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z ') "audit line should start with ISO timestamp"

  let fail_line = (format_audit_line "fail")
  assert ($fail_line | str ends-with " fail") "fail audit line should end with 'fail'"

  # Test: status determination logic
  let pass_exit = 0
  let fail_exit = 1
  let status_pass = if $pass_exit != 0 { "fail" } else { "pass" }
  let status_fail = if $fail_exit != 0 { "fail" } else { "pass" }
  assert equal $status_pass "pass"
  assert equal $status_fail "fail"

  # Test: path construction
  let root = "/tmp/test-root"
  let log_file = ($root | path join ".cards" "promotion" "make_check_runs.log")
  assert ($log_file | str contains "promotion") "log path should contain promotion"
  assert ($log_file | str ends-with "make_check_runs.log") "log path should end with filename"

  print "PASS: record_make_check.nu"
}
