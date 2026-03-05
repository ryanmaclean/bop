#!/usr/bin/env nu
# dashboard.nu — Live status dashboard for all 5 job teams

def count_cards [dir: string]: nothing -> int {
  if not ($dir | path exists) { return 0 }
  glob $"($dir)/*.bop" | where { |p| ($p | path type) == "dir" } | length
}

def build_progress_bar [n_merged: int, n_done: int, n_running: int, n_failed: int, total: int]: nothing -> string {
  mut bar = ""
  for i in 0..<$total {
    if $i < $n_merged {
      $bar = $"($bar)M"
    } else if $i < ($n_merged + $n_done) {
      $bar = $"($bar)✓"
    } else if $i < ($n_merged + $n_done + $n_running) {
      $bar = $"($bar)▶"
    } else if $i < ($total - $n_failed) {
      $bar = $"($bar)·"
    } else {
      $bar = $"($bar)✗"
    }
  }
  $bar
}

def main [
  --test  # Run internal self-tests
] {
  if $test {
    run_tests
    return
  }

  let root = ($env.FILE_PWD | path dirname)

  let teams = [
    { name: "team-cli",          adapter: "claude" }
    { name: "team-arch",         adapter: "claude" }
    { name: "team-quality",      adapter: "claude" }
    { name: "team-intelligence", adapter: "opencode" }
    { name: "team-platform",     adapter: "codex" }
  ]

  loop {
    ^clear

    print "╔══════════════════════════════════════════════════════════════╗"
    let ts = (date now | format date "%H:%M:%S")
    print $"║  JOBCARD DASHBOARD  ($ts)                                  ║"
    print "╠══════════════════════════════════════════════════════════════╣"

    for team in $teams {
      let cards_dir = ($root | path join ".cards" $team.name)

      let n_pending = (count_cards ($cards_dir | path join "pending"))
      let n_running = (count_cards ($cards_dir | path join "running"))
      let n_done    = (count_cards ($cards_dir | path join "done"))
      let n_merged  = (count_cards ($cards_dir | path join "merged"))
      let n_failed  = (count_cards ($cards_dir | path join "failed"))
      let total = 5

      let bar = (build_progress_bar $n_merged $n_done $n_running $n_failed $total)

      print $"║  ($team.name | fill -w 18)  [($bar)] [($team.adapter)]"
      print $"║  p:($n_pending | fill -w 2) r:($n_running | fill -w 2) d:($n_done | fill -w 2) m:($n_merged | fill -w 2) f:($n_failed | fill -w 2)                                 ║"

      let running_dir = ($cards_dir | path join "running")
      if ($running_dir | path exists) {
        for card_path in (glob $"($running_dir)/*.bop") {
          if ($card_path | path type) != "dir" { continue }
          let card_name = ($card_path | path basename | str replace ".bop" "")
          let pid_file = ($card_path | path join "logs" "pid")
          let pid = if ($pid_file | path exists) {
            $" [(open --raw $pid_file | str trim)]"
          } else { "" }
          print $"║  ▶ ($card_name | fill -w 25)($pid | fill -w 32)║"

          let stdout_file = ($card_path | path join "logs" "stdout.log")
          if ($stdout_file | path exists) and ((ls $stdout_file | get size | first) > 0b) {
            let last = (open --raw $stdout_file | lines | last | str substring 0..54)
            print $"║    └ ($last | fill -w 56)║"
          }
        }
      }

      print "╠══════════════════════════════════════════════════════════════╣"
    }

    # Totals
    mut td = 0; mut tr_ = 0; mut tp = 0; mut tf = 0
    for team in $teams {
      let c = ($root | path join ".cards" $team.name)
      $td  = $td  + (count_cards ($c | path join "done"))    + (count_cards ($c | path join "merged"))
      $tr_ = $tr_ + (count_cards ($c | path join "running"))
      $tp  = $tp  + (count_cards ($c | path join "pending"))
      $tf  = $tf  + (count_cards ($c | path join "failed"))
    }
    print $"║  done:($td | fill -w 3)  running:($tr_ | fill -w 3)  pending:($tp | fill -w 3)  failed:($tf | fill -w 3)         ║"
    print "╚══════════════════════════════════════════════════════════════╝"
    print ""

    # Done/merged summary
    for team in $teams {
      for state in [done merged] {
        let state_dir = ($root | path join ".cards" $team.name $state)
        if ($state_dir | path exists) {
          for card_path in (glob $"($state_dir)/*.bop") {
            if ($card_path | path type) != "dir" { continue }
            let card_name = ($card_path | path basename | str replace ".bop" "")
            print $"  ✓ ($card_name | fill -w 26) ($team.name)"
          }
        }
      }
    }

    sleep 3sec
  }
}

def run_tests [] {
  use std/assert

  # Test: count_cards on non-existent directory returns 0
  let count = (count_cards "/tmp/nonexistent-dir-bop-test-xyz")
  assert equal $count 0 "count_cards on missing dir should be 0"

  # Test: count_cards on an empty temp directory returns 0
  let tmp = (mktemp -d)
  let count_empty = (count_cards $tmp)
  assert equal $count_empty 0 "count_cards on empty dir should be 0"

  # Test: count_cards counts .bop directories
  let test_dir = ($tmp | path join "cards")
  mkdir $test_dir
  mkdir ($test_dir | path join "card1.bop")
  mkdir ($test_dir | path join "card2.bop")
  "not a dir" | save ($test_dir | path join "card3.bop")
  let count_bop = (count_cards $test_dir)
  assert equal $count_bop 2 "count_cards should count only .bop directories"

  # Test: build_progress_bar produces correct characters
  let bar1 = (build_progress_bar 2 1 1 0 5)
  assert equal $bar1 "MM✓▶·" "bar with 2M 1done 1run 0fail"

  let bar2 = (build_progress_bar 0 0 0 0 5)
  assert equal $bar2 "·····" "bar with all pending"

  let bar3 = (build_progress_bar 5 0 0 0 5)
  assert equal $bar3 "MMMMM" "bar with all merged"

  let bar4 = (build_progress_bar 0 0 0 2 5)
  assert equal $bar4 "···✗✗" "bar with 2 failed"

  let bar5 = (build_progress_bar 1 1 1 1 5)
  assert equal $bar5 "M✓▶·✗" "bar with one of each + one pending"

  # Test: card name extraction logic
  let card_path = "/some/path/running/my-task.bop"
  let card_name = ($card_path | path basename | str replace ".bop" "")
  assert equal $card_name "my-task" "card name should strip .bop suffix"

  # Cleanup
  rm -rf $tmp

  print "PASS: dashboard.nu"
}
