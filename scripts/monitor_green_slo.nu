#!/usr/bin/env nu
# monitor_green_slo.nu — monitor green lane SLO and trigger rollback to blue-only

def main [
  --window-minutes: int = 30         # Observation window in minutes
  --check-interval-sec: int = 60     # Seconds between checks
  --max-failure-rate: float = 0.20   # Maximum allowed failure rate
  --min-sample-size: int = 5         # Minimum samples before evaluating
] {
  let root = ($env.FILE_PWD | path dirname)
  let green_dir = $"($root)/.cards-green"
  let override_file = $"($root)/.cards/route.override"

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

def gather_window_metrics [green_dir: string, window_minutes: int] {
  let cutoff = ((date now) - ($window_minutes * 1min))

  mut merged = 0
  mut failed = 0
  mut policy_violations = 0

  for state in ["merged", "failed"] {
    let state_dir = $"($green_dir)/($state)"
    if not ($state_dir | path exists) { continue }

    let cards = (glob $"($state_dir)/*.bop")
    for card in $cards {
      let mtime = (ls -l $card | get 0.modified)
      if $mtime < $cutoff { continue }

      if $state == "merged" {
        $merged = $merged + 1
      } else {
        $failed = $failed + 1
        let meta_path = $"($card)/meta.json"
        if ($meta_path | path exists) {
          try {
            let meta = (open $meta_path)
            if ($meta | get -o failure_reason | default "") == "policy_violation" {
              $policy_violations = $policy_violations + 1
            }
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
