#!/usr/bin/env nu
# Lane throughput metrics for blue/green card lanes.
# Delegates to embedded Python for JSON/stats processing.

def main [
  --window-minutes: int = 1440  # Time window in minutes (default 24h)
  --output: string = "table"     # Output format: table or json
] {
  let root = ($env.FILE_PWD | path dirname)
  let py_file = $"($root)/scripts/_lane_metrics_impl.py"

  ^python3 $py_file $root $"($window_minutes)" $output
}
