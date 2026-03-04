#!/usr/bin/env nu
# bop_focus.nu — Focus a card across all 7 panes in the bop Zellij layout.
#
# Modes:
#   bop_focus.nu <id>          — print paste commands for each pane
#   bop_focus.nu --auto <id>   — sweep all panes and type commands in (Zellij only)
#   bop_focus.nu --test        — identify each pane by writing its name into it
#
# Pane order in bop.kdl (depth-first, layout traversal):
#   1. board     (skip — auto-refreshing watch loop)
#   2. spec      <- SPEC: what to build
#   3. qa        <- QA:   did it work?
#   4. stdout    <- STDOUT: agent live output
#   5. stderr    <- STDERR: agent errors
#   6. inspector <- INSPECTOR: bop inspect metadata
#   7. shell     <- current pane (you run this from here)
#
# If traversal order differs on your setup, tune:
#   BOP_PANE_SKIP=1    (panes to skip before spec; default 1 for board)
#   BOP_PANE_COUNT=7   (total pane count including shell; default 7)
#
# Alt+arrows move focus between panes manually.

def main [
  id?: string        # Card ID to focus
  --auto             # Sweep all panes and type commands in (Zellij only)
  --test             # Identify each pane by writing its name into it
] {
  let root = ($env.FILE_PWD | path dirname)
  let bop = $"($root)/target/debug/bop"

  let pane_skip = if "BOP_PANE_SKIP" in $env { $env.BOP_PANE_SKIP | into int } else { 1 }

  # -- Helper functions implemented as closures --

  # Send a command to focused pane then press Enter
  let send = {|cmd: string|
    ^zellij action write-chars $cmd
    ^zellij action write 13
  }

  # Interrupt current pane then send a command
  let replace_cmd = {|cmd: string|
    ^zellij action write 3   # Ctrl+C
    sleep 50ms
    ^zellij action write-chars $cmd
    ^zellij action write 13
  }

  # Advance focus by N panes
  let hop = {|n: int|
    for _ in 1..($n + 1) {
      ^zellij action focus-next-pane
      sleep 50ms
    }
  }

  # -- Test mode --
  if $test {
    if not ("ZELLIJ" in $env) {
      print -e "ERROR: --test requires running inside a Zellij session"
      exit 1
    }
    print "▶ Testing pane traversal from shell (pane 7)..."
    print "  Watch for 'PANE N' appearing in each pane."
    print ""
    for i in 1..7 {
      ^zellij action focus-next-pane
      sleep 100ms
      do $send $"echo 'PANE ($i) of 6'"
    }
    # Return to shell
    ^zellij action focus-next-pane
    print "Done. Expected: board=1, spec=2, qa=3, stdout=4, stderr=5, inspector=6"
    print "If order differs, set BOP_PANE_SKIP / reorder layout"
    return
  }

  # Require card ID when not in test mode
  if $id == null or $id == "" {
    print -e "Usage: bop_focus.nu [--auto] [--test] <card-id>"
    exit 1
  }

  # -- Locate the card --
  mut card_dir = ""
  for state in [running pending done merged failed] {
    let candidate = $"($root)/.cards/($state)/($id).bop"
    if ($candidate | path exists) {
      $card_dir = $candidate
      break
    }
  }

  if $card_dir == "" {
    print -e $"ERROR: card '($id)' not found in any state"
    do { ^$bop status } | complete | ignore
    exit 1
  }

  let state_name = ($card_dir | path dirname | path basename)
  print $"▶ Focusing: ($id)  \(($state_name)/\)"
  $id | save --force /tmp/.bop_card

  # -- Build per-pane commands --
  let cmd_spec = $"clear; printf '\\033[1;36m── SPEC: ($id) ──\\033[0m\\n'; cat '($card_dir)/spec.md' 2>/dev/null || echo '\(no spec.md\)'"
  let cmd_qa = $"clear; printf '\\033[1;32m── QA: ($id) ──\\033[0m\\n'; cat '($card_dir)/output/qa_report.md' 2>/dev/null || echo '\(pending\)'"
  let cmd_stdout = $"clear; printf '\\033[1;33m── STDOUT: ($id) ──\\033[0m\\n'; tail -f '($card_dir)/logs/stdout.log' 2>/dev/null || echo '\(no stdout yet\)'"
  let cmd_stderr = $"clear; printf '\\033[1;31m── STDERR: ($id) ──\\033[0m\\n'; tail -f '($card_dir)/logs/stderr.log' 2>/dev/null || echo '\(no stderr yet\)'"
  let cmd_inspect = $"clear; watch -n3 '($bop) inspect ($id) 2>/dev/null || echo card not found'"

  # -- Auto mode: sweep panes via zellij action --
  if $auto {
    if not ("ZELLIJ" in $env) {
      print -e "ERROR: --auto requires running inside a Zellij session (bop.kdl layout)"
      print -e "Launch with: zellij --layout layouts/bop.kdl"
      exit 1
    }

    print ""
    print "  Sweeping panes (run --test first if order looks wrong)..."
    print ""

    # Step 1: skip board pane(s)
    do $hop $pane_skip

    # Step 2: spec pane
    ^zellij action focus-next-pane; sleep 100ms
    do $replace_cmd $cmd_spec
    sleep 150ms

    # Step 3: qa pane
    ^zellij action focus-next-pane; sleep 100ms
    do $replace_cmd $cmd_qa
    sleep 150ms

    # Step 4: stdout pane
    ^zellij action focus-next-pane; sleep 100ms
    do $replace_cmd $cmd_stdout
    sleep 150ms

    # Step 5: stderr pane
    ^zellij action focus-next-pane; sleep 100ms
    do $replace_cmd $cmd_stderr
    sleep 150ms

    # Step 6: inspector pane
    ^zellij action focus-next-pane; sleep 100ms
    do $replace_cmd $cmd_inspect
    sleep 150ms

    # Return to shell (pane 7)
    ^zellij action focus-next-pane
    sleep 100ms

    print $"All panes updated for: ($id)"
    print "  SPEC -> QA -> STDOUT -> STDERR -> INSPECTOR"
    print ""
    print "  Alt+arrows to navigate  |  Ctrl+C to stop tail/watch in a pane"
    return
  }

  # -- Manual mode: print paste commands --
  print ""
  print "── Paste into each pane (or run with --auto): ──────────────────────────"
  print ""
  print "  SPEC pane:"
  print $"    ($cmd_spec)"
  print ""
  print "  QA pane:"
  print $"    ($cmd_qa)"
  print ""
  print "  STDOUT pane:"
  print $"    ($cmd_stdout)"
  print ""
  print "  STDERR pane:"
  print $"    ($cmd_stderr)"
  print ""
  print "  INSPECTOR pane:"
  print $"    ($cmd_inspect)"
  print ""
  print "  Alt+arrows to navigate between panes"
  print ""
  do { ^$bop inspect $id } | complete | ignore
}
