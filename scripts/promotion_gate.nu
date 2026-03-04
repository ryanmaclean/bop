#!/usr/bin/env nu
# promotion_gate.nu — Promotion gating: checks 4 gates before allowing cutover.

def main [] {
  let root = ($env.FILE_PWD | path dirname)
  let window_minutes = 48 * 60
  let check_history_file = $"($root)/.cards/promotion/make_check_runs.log"
  let incidents_dir = $"($root)/.cards/incidents/critical"

  # Get metrics from lane_metrics
  let metrics_json = (^nu $"($root)/scripts/lane_metrics.nu" --window-minutes $window_minutes --output json | from json)

  mut failures = []

  # Gate 1: make check green for 5 consecutive runs.
  mut gate1_ok = false
  if ($check_history_file | path exists) {
    let lines = (open --raw $check_history_file | lines | where { |ln| ($ln | str trim) != "" } | each { |ln| $ln | str trim | str downcase })
    let last5 = ($lines | last 5)
    if ($last5 | length) == 5 and ($last5 | all { |x| $x == "pass" }) {
      $gate1_ok = true
    }
  }
  if not $gate1_ok {
    $failures = ($failures | append "Gate 1 failed: need 5 consecutive 'pass' entries in .cards/promotion/make_check_runs.log")
  }

  # Gate 2: policy violation rate = 0 for 48h (enforced for both lanes).
  let blue_pvr = ($metrics_json | get -o blue.policy_violation_rate | default 1.0 | into float)
  let green_pvr = ($metrics_json | get -o green.policy_violation_rate | default 1.0 | into float)
  if $blue_pvr != 0.0 or $green_pvr != 0.0 {
    $failures = ($failures | append $"Gate 2 failed: policy_violation_rate must be 0.0 \(blue=($blue_pvr), green=($green_pvr)\)")
  }

  # Gate 3: green success rate >= blue success rate over same window.
  let blue_sr = ($metrics_json | get -o blue.success_rate | default 0.0 | into float)
  let green_sr = ($metrics_json | get -o green.success_rate | default 0.0 | into float)
  if $green_sr < $blue_sr {
    $failures = ($failures | append $"Gate 3 failed: green success_rate ($green_sr) is below blue ($blue_sr)")
  }

  # Gate 4: no unresolved critical incidents.
  mut open_incidents = []
  if ($incidents_dir | path exists) and (($incidents_dir | path type) == "dir") {
    $open_incidents = (ls $incidents_dir | where type == "file" | get name | each { |p| $p | path basename })
  }
  if ($open_incidents | length) > 0 {
    let incident_list = ($open_incidents | sort | str join ", ")
    $failures = ($failures | append $"Gate 4 failed: unresolved critical incidents present: ($incident_list)")
  }

  # Report
  print "Promotion gate report (48h window):"
  let g1 = if $gate1_ok { "PASS" } else { "FAIL" }
  let g2 = if $blue_pvr == 0.0 and $green_pvr == 0.0 { "PASS" } else { "FAIL" }
  let g3 = if $green_sr >= $blue_sr { "PASS" } else { "FAIL" }
  let g4 = if ($open_incidents | length) == 0 { "PASS" } else { "FAIL" }

  print $"- Gate 1 \(5x make check pass\): ($g1)"
  print $"- Gate 2 \(policy violation rate 0\): ($g2)"
  print $"- Gate 3 \(green success >= blue\): ($g3)"
  print $"- Gate 4 \(no critical incidents\): ($g4)"

  if ($failures | length) > 0 {
    print "\nFAILED GATES:"
    for item in $failures {
      print $"- ($item)"
    }
    exit 1
  }

  print "\nALL GATES PASS: eligible for big-bang cutover to green."
}
