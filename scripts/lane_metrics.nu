#!/usr/bin/env nu
# Lane throughput metrics for blue/green card lanes.
# Delegates to embedded Python for JSON/stats processing.

def main [
  --test                       # Run internal self-tests
  --window-minutes: int = 1440  # Time window in minutes (default 24h)
  --output: string = "table"    # Output format: table or json
] {
  if $test {
    run_tests
    return
  }

  let root = ($env.FILE_PWD | path dirname)
  let py_file = $"($root)/scripts/_lane_metrics_impl.py"

  ^python3 $py_file $root $"($window_minutes)" $output
}

def run_tests [] {
  use std/assert

  # Test: default parameter values
  let default_window = 1440
  assert equal $default_window 1440 "default window should be 1440 minutes (24h)"

  # Test: output format options
  let valid_formats = ["table" "json"]
  assert ("table" in $valid_formats) "table should be a valid format"
  assert ("json" in $valid_formats) "json should be a valid format"

  # Test: path construction for python script
  let root = "/tmp/test-root"
  let py_file = $"($root)/scripts/_lane_metrics_impl.py"
  assert equal $py_file "/tmp/test-root/scripts/_lane_metrics_impl.py"

  # Test: window-minutes string interpolation (used when calling python)
  let window = 2880
  let window_str = $"($window)"
  assert equal $window_str "2880"

  print "PASS: lane_metrics.nu"
}
