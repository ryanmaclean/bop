#!/usr/bin/env nu
# monitor_green_slo.nu — monitor green lane SLO and trigger rollback to blue-only

def compute_window_metrics [merged: int, failed: int]: nothing -> record {
  let total = $merged + $failed
  let failure_rate = if $total > 0 { $failed / $total } else { 0.0 }
  {
    total: $total
    merged: $merged
    failed: $failed
    failure_rate: $failure_rate
  }
}

def run_tests [] {
  use std/assert

  # Test compute_window_metrics: all merged, zero failures
  let m1 = (compute_window_metrics 10 0)
  assert equal $m1.total 10 "total is merged + failed"
  assert equal $m1.merged 10 "merged count"
  assert equal $m1.failed 0 "failed count"
  assert equal $m1.failure_rate 0.0 "zero failure rate"

  # Test compute_window_metrics: all failed
  let m2 = (compute_window_metrics 0 5)
  assert equal $m2.total 5 "total with all failed"
  assert equal $m2.failure_rate 1.0 "100% failure rate"

  # Test compute_window_metrics: mixed
  let m3 = (compute_window_metrics 8 2)
  assert equal $m3.total 10 "total mixed"
  assert equal $m3.failure_rate 0.2 "20% failure rate"

  # Test compute_window_metrics: zero total
  let m4 = (compute_window_metrics 0 0)
  assert equal $m4.total 0 "zero total"
  assert equal $m4.failure_rate 0.0 "zero total gives 0.0 rate"

  # Test breach detection logic
  let max_failure_rate = 0.20
  let min_sample_size = 5
  let metrics = (compute_window_metrics 3 2)
  let is_breach = ($metrics.total >= $min_sample_size and $metrics.failure_rate > $max_failure_rate)
  assert equal $is_breach true "5 total, 40% failure rate should breach at 20% threshold"

  # Test no breach when under threshold
  let metrics2 = (compute_window_metrics 9 1)
  let is_breach2 = ($metrics2.total >= $min_sample_size and $metrics2.failure_rate > $max_failure_rate)
  assert equal $is_breach2 false "10% failure rate should not breach at 20% threshold"

  # Test no breach when insufficient samples
  let metrics3 = (compute_window_metrics 1 3)
  let is_breach3 = ($metrics3.total >= $min_sample_size and $metrics3.failure_rate > $max_failure_rate)
  assert equal $is_breach3 false "below min_sample_size should not breach"

  # Test required_breach_intervals calculation
  let window_minutes = 30
  let check_interval_sec = 60
  # Ceiling division: (1800 + 59) / 60 = 30.98... which rounds up to 31 breach intervals
  let raw = (($window_minutes * 60 + $check_interval_sec - 1) / $check_interval_sec)
  assert ($raw > 30) "ceiling division result > 30 for 30min/60s"
  assert ($raw < 32) "ceiling division result < 32 for 30min/60s"

  print "PASS: monitor_green_slo.nu"
}

def main [
  --test                                 # Run internal self-tests
  --window-minutes: int = 30             # Observation window in minutes
  --check-interval-sec: int = 60         # Seconds between checks
  --max-failure-rate: float = 0.20       # Maximum allowed failure rate
  --min-sample-size: int = 5             # Minimum samples before evaluating
] {
  if $test {
    run_tests
    return
  }
  let root = ($env.FILE_PWD | path dirname)
  let green_dir = ($root | path join ".cards-green")
  let override_file = ($root | path join ".cards" "route.override")

  let required_breach_intervals = (($window_minutes * 60 + $check_interval_sec - 1) / $check_interval_sec)
  mut consecutive_breach = 0

  loop {
    let metrics = (gather_window_metrics $green_dir $window_minutes)
    let total = $metrics.total
    let merged = $metrics.merged
    let failed = $metrics.failed
    let policy_violations = $metrics.policy_violations
    let failure_rate = $metrics.failure_rate

    mut is_breach = false
    if $total >= $min_sample_size and $failure_rate > $max_failure_rate {
      $is_breach = true
      $consecutive_breach = $consecutive_breach + 1
    } else {
      $consecutive_breach = 0
    }

    let ts = (date now | format date "%Y-%m-%dT%H:%M:%SZ")
    let breach_int = if $is_breach { 1 } else { 0 }
    print $"[($ts)] green-window total=($total) merged=($merged) failed=($failed) policy_violations=($policy_violations) failure_rate=($failure_rate | math round --precision 6) breach=($breach_int) consecutive=($consecutive_breach)/($required_breach_intervals)"

    if $consecutive_breach >= $required_breach_intervals {
      "blue-only" | save $override_file
      print -e $"[($ts)] rollback activated: wrote ($override_file) \(blue-only\)"
      $consecutive_breach = 0
    }

    sleep ($check_interval_sec * 1sec)
  }
}

def gather_window_metrics [green_dir: string, window_minutes: int]: nothing -> record {
  let cutoff = ((date now) - ($window_minutes * 1min))

  mut merged = 0
  mut failed = 0
  mut policy_violations = 0

  for state in ["merged", "failed"] {
    let state_dir = ($green_dir | path join $state)
    if not ($state_dir | path exists) { continue }

    let cards = (glob ($state_dir | path join "*.bop"))
    for card in $cards {
      let mtime = (ls -l $card | get 0.modified)
      if $mtime < $cutoff { continue }

      if $state == "merged" {
        $merged = $merged + 1
      } else {
        $failed = $failed + 1
        let meta_path = ($card | path join "meta.json")
        if ($meta_path | path exists) {
          let meta = (open $meta_path)
          if ($meta | get -o failure_reason | default "") == "policy_violation" {
            $policy_violations = $policy_violations + 1
          }
        }
      }
    }
  }

  let total = $merged + $failed
  let failure_rate = if $total > 0 { $failed / $total } else { 0.0 }

  {
    total: $total
    merged: $merged
    failed: $failed
    policy_violations: $policy_violations
    failure_rate: $failure_rate
  }
}
