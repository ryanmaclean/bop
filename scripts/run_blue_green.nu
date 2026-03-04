#!/usr/bin/env nu
# run_blue_green.nu — start blue/green factory with dual dispatcher + merge-gate pairs

def main [] {
  let root = ($env.FILE_PWD | path dirname)
  let bop = $"($root)/target/debug/bop"
  let blue_dir = $"($root)/.cards-blue"
  let green_dir = $"($root)/.cards-green"
  let adapter_default = $"($root)/adapters/mock.nu"

  if not ($bop | path exists) {
    print -e $"Missing bop binary: ($bop)"
    print -e "Run: cargo build"
    exit 1
  }

  mkdir $blue_dir $green_dir
  ^$bop --cards-dir $blue_dir init o> /dev/null
  ^$bop --cards-dir $green_dir init o> /dev/null

  # Keep templates/providers in sync from the canonical cards root when present
  if ($"($root)/.cards/templates" | path exists) {
    ^rsync -a --delete $"($root)/.cards/templates/" $"($blue_dir)/templates/"
    ^rsync -a --delete $"($root)/.cards/templates/" $"($green_dir)/templates/"
  }
  if ($"($root)/.cards/providers.json" | path exists) {
    cp $"($root)/.cards/providers.json" $"($blue_dir)/providers.json"
    cp $"($root)/.cards/providers.json" $"($green_dir)/providers.json"
  }

  let blue_disp_log = "/tmp/bop-blue-dispatcher.log"
  let green_disp_log = "/tmp/bop-green-dispatcher.log"
  let blue_mg_log = "/tmp/bop-blue-merge-gate.log"
  let green_mg_log = "/tmp/bop-green-merge-gate.log"

  # Kill any existing processes
  do { ^pkill -f $"bop --cards-dir ($blue_dir) dispatcher" } | complete
  do { ^pkill -f $"bop --cards-dir ($green_dir) dispatcher" } | complete
  do { ^pkill -f $"bop --cards-dir ($blue_dir) merge-gate" } | complete
  do { ^pkill -f $"bop --cards-dir ($green_dir) merge-gate" } | complete

  # Start blue dispatcher + merge-gate (git_gt engine)
  ^nohup $bop --cards-dir $blue_dir dispatcher --adapter $adapter_default --vcs-engine git_gt --poll-ms 250 --reap-ms 1000 o>> $blue_disp_log e>&1 &
  ^nohup $bop --cards-dir $blue_dir merge-gate --vcs-engine git_gt --poll-ms 500 o>> $blue_mg_log e>&1 &

  # Start green dispatcher + merge-gate (jj engine)
  ^nohup $bop --cards-dir $green_dir dispatcher --adapter $adapter_default --vcs-engine jj --poll-ms 250 --reap-ms 1000 o>> $green_disp_log e>&1 &
  ^nohup $bop --cards-dir $green_dir merge-gate --vcs-engine jj --poll-ms 500 o>> $green_mg_log e>&1 &

  print "Blue/Green factory started."
  print ""
  print $"Blue cards dir:  ($blue_dir)  \(engine=git_gt\)"
  print $"Green cards dir: ($green_dir) \(engine=jj\)"
  print ""
  print "Logs:"
  print $"  ($blue_disp_log)"
  print $"  ($green_disp_log)"
  print $"  ($blue_mg_log)"
  print $"  ($green_mg_log)"
}
