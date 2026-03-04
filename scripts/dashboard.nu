#!/usr/bin/env nu
# dashboard.nu — Live status dashboard for all 5 job teams

def count_cards [dir: string]: nothing -> int {
  if not ($dir | path exists) { return 0 }
  glob $"($dir)/*.bop" | where { |p| ($p | path type) == "dir" } | length
}

def main [] {
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
      let cards_dir = $"($root)/.cards/($team.name)"

      let n_pending = (count_cards $"($cards_dir)/pending")
      let n_running = (count_cards $"($cards_dir)/running")
      let n_done    = (count_cards $"($cards_dir)/done")
      let n_merged  = (count_cards $"($cards_dir)/merged")
      let n_failed  = (count_cards $"($cards_dir)/failed")
      let total = 5

      # Progress bar: M=merged, check=done, tri=running, dot=queued, X=failed
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

      print $"║  ($team.name | fill -w 18)  [($bar)] [($team.adapter)]"
      print $"║  p:($n_pending | fill -w 2) r:($n_running | fill -w 2) d:($n_done | fill -w 2) m:($n_merged | fill -w 2) f:($n_failed | fill -w 2)                                 ║"

      let running_dir = $"($cards_dir)/running"
      if ($running_dir | path exists) {
        for card_path in (glob $"($running_dir)/*.bop") {
          if ($card_path | path type) != "dir" { continue }
          let card_name = ($card_path | path basename | str replace ".bop" "")
          let pid_file = $"($card_path)/logs/pid"
          let pid = if ($pid_file | path exists) {
            $" [(open --raw $pid_file | str trim)]"
          } else { "" }
          print $"║  ▶ ($card_name | fill -w 25)($pid | fill -w 32)║"

          let stdout_file = $"($card_path)/logs/stdout.log"
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
      let c = $"($root)/.cards/($team.name)"
      $td  = $td  + (count_cards $"($c)/done")    + (count_cards $"($c)/merged")
      $tr_ = $tr_ + (count_cards $"($c)/running")
      $tp  = $tp  + (count_cards $"($c)/pending")
      $tf  = $tf  + (count_cards $"($c)/failed")
    }
    print $"║  done:($td | fill -w 3)  running:($tr_ | fill -w 3)  pending:($tp | fill -w 3)  failed:($tf | fill -w 3)         ║"
    print "╚══════════════════════════════════════════════════════════════╝"
    print ""

    # Done/merged summary
    for team in $teams {
      for state in [done merged] {
        let state_dir = $"($root)/.cards/($team.name)/($state)"
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
