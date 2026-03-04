#!/usr/bin/env nu
# promotion_gate.nu — Promotion gating: checks 4 gates before allowing cutover.

def check_gate1 [check_history_file: string]: nothing -> bool {
  if not ($check_history_file | path exists) { return false }
  let lines = (open --raw $check_history_file | lines | where { |ln| ($ln | str trim) != "" } | each { |ln| $ln | str trim | str downcase })
  let last5 = ($lines | last 5)
  ($last5 | length) == 5 and ($last5 | all { |x| $x == "pass" })
}

def check_gate2 [blue_pvr: float, green_pvr: float]: nothing -> bool {
  $blue_pvr == 0.0 and $green_pvr == 0.0
}

def check_gate3 [green_sr: float, blue_sr: float]: nothing -> bool {
  $green_sr >= $blue_sr
}

def list_incidents [incidents_dir: string]: nothing -> list<string> {
  if not ($incidents_dir | path exists) { return [] }
  if (($incidents_dir | path type) != "dir") { return [] }
  ls $incidents_dir | where type == "file" | get name | each { |p| $p | path basename }
}

def main [
  --test  # Run internal self-tests
] {
  if $test {
    run_tests
    return
  }

  let root = ($env.FILE_PWD | path dirname)
  let window_minutes = 48 * 60
  let check_history_file = $"($root)/.cards/promotion/make_check_runs.log"
  let incidents_dir = $"($root)/.cards/incidents/critical"

  # Get metrics from lane_metrics
  let metrics_json = (^nu $"($root)/scripts/lane_metrics.nu" --window-minutes $window_minutes --output json | from json)

  mut failures = []

  # Gate 1: make check green for 5 consecutive runs.
  let gate1_ok = (check_gate1 $check_history_file)
  if not $gate1_ok {
    $failures = ($failures | append "Gate 1 failed: need 5 consecutive 'pass' entries in .cards/promotion/make_check_runs.log")
  }

  # Gate 2: policy violation rate = 0 for 48h (enforced for both lanes).
  let blue_pvr = ($metrics_json | get -o blue.policy_violation_rate | default 1.0 | into float)
  let green_pvr = ($metrics_json | get -o green.policy_violation_rate | default 1.0 | into float)
  if not (check_gate2 $blue_pvr $green_pvr) {
    $failures = ($failures | append $"Gate 2 failed: policy_violation_rate must be 0.0 \(blue=($blue_pvr), green=($green_pvr)\)")
  }

  # Gate 3: green success rate >= blue success rate over same window.
  let blue_sr = ($metrics_json | get -o blue.success_rate | default 0.0 | into float)
  let green_sr = ($metrics_json | get -o green.success_rate | default 0.0 | into float)
  if not (check_gate3 $green_sr $blue_sr) {
    $failures = ($failures | append $"Gate 3 failed: green success_rate ($green_sr) is below blue ($blue_sr)")
  }

  # Gate 4: no unresolved critical incidents.
  let open_incidents = (list_incidents $incidents_dir)
  if ($open_incidents | length) > 0 {
    let incident_list = ($open_incidents | sort | str join ", ")
    $failures = ($failures | append $"Gate 4 failed: unresolved critical incidents present: ($incident_list)")
  }

  # Report
  print "Promotion gate report (48h window):"
  let g1 = if $gate1_ok { "PASS" } else { "FAIL" }
  let g2 = if (check_gate2 $blue_pvr $green_pvr) { "PASS" } else { "FAIL" }
  let g3 = if (check_gate3 $green_sr $blue_sr) { "PASS" } else { "FAIL" }
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

def run_tests [] {
  use std/assert
  let tmp = (mktemp -d)

  # --- Gate 1 tests: check_gate1 ---
  # Non-existent file -> false
  assert equal (check_gate1 "/tmp/nonexistent-bop-xyz") false "gate1: missing file -> false"

  # File with fewer than 5 entries -> false
  let f1 = $"($tmp)/gate1_short.log"
  "pass\npass\npass\n" | save $f1
  assert equal (check_gate1 $f1) false "gate1: only 3 entries -> false"

  # File with 5 passes -> true
  let f2 = $"($tmp)/gate1_pass.log"
  "pass\npass\npass\npass\npass\n" | save $f2
  assert equal (check_gate1 $f2) true "gate1: 5 passes -> true"

  # File with a fail in last 5 -> false
  let f3 = $"($tmp)/gate1_fail.log"
  "pass\npass\nfail\npass\npass\n" | save $f3
  assert equal (check_gate1 $f3) false "gate1: fail in last 5 -> false"

  # Mixed case -> still passes (normalized to lowercase)
  let f4 = $"($tmp)/gate1_mixed.log"
  "PASS\nPass\npass\nPASS\npass\n" | save $f4
  assert equal (check_gate1 $f4) true "gate1: mixed case passes -> true"

  # More than 5 entries, last 5 are pass -> true
  let f5 = $"($tmp)/gate1_long.log"
  "fail\nfail\npass\npass\npass\npass\npass\n" | save $f5
  assert equal (check_gate1 $f5) true "gate1: old fails, last 5 pass -> true"

  # --- Gate 2 tests: check_gate2 ---
  assert equal (check_gate2 0.0 0.0) true "gate2: both 0.0 -> pass"
  assert equal (check_gate2 0.1 0.0) false "gate2: blue nonzero -> fail"
  assert equal (check_gate2 0.0 0.5) false "gate2: green nonzero -> fail"
  assert equal (check_gate2 1.0 1.0) false "gate2: both 1.0 -> fail"

  # --- Gate 3 tests: check_gate3 ---
  assert equal (check_gate3 0.9 0.8) true "gate3: green > blue -> pass"
  assert equal (check_gate3 0.8 0.8) true "gate3: green == blue -> pass"
  assert equal (check_gate3 0.7 0.8) false "gate3: green < blue -> fail"

  # --- Gate 4 tests: list_incidents ---
  # Non-existent dir -> empty
  assert equal (list_incidents "/tmp/nonexistent-bop-xyz") [] "gate4: missing dir -> empty"

  # Empty dir -> empty
  let inc_dir = $"($tmp)/incidents"
  mkdir $inc_dir
  assert equal (list_incidents $inc_dir) [] "gate4: empty dir -> empty"

  # Dir with files -> lists them
  "outage" | save $"($inc_dir)/inc-001.json"
  "outage" | save $"($inc_dir)/inc-002.json"
  let incidents = (list_incidents $inc_dir)
  assert equal ($incidents | length) 2 "gate4: 2 incident files"
  assert ("inc-001.json" in $incidents) "gate4: should find inc-001.json"
  assert ("inc-002.json" in $incidents) "gate4: should find inc-002.json"

  # Cleanup
  rm -rf $tmp

  print "PASS: promotion_gate.nu"
}
