#!/usr/bin/env nu
# run_blue_green.nu — start blue/green factory with dual dispatcher + merge-gate pairs

# Build adapter path given a root directory
def adapter_path [root: string, name: string]: nothing -> string {
  $root | path join "adapters" $"($name).nu"
}

# Build cards directory path for a color lane
def cards_dir [root: string, color: string]: nothing -> string {
  $root | path join $".cards-($color)"
}

def run_tests [] {
  use std/assert

  # Test adapter path construction
  assert equal (adapter_path "/opt/bop" "mock") "/opt/bop/adapters/mock.nu"
  assert equal (adapter_path "/home/user/bop" "claude") "/home/user/bop/adapters/claude.nu"
  assert equal (adapter_path "/tmp/test" "ollama-local") "/tmp/test/adapters/ollama-local.nu"

  # Test cards directory path construction
  assert equal (cards_dir "/opt/bop" "blue") "/opt/bop/.cards-blue"
  assert equal (cards_dir "/opt/bop" "green") "/opt/bop/.cards-green"

  # Test that blue and green dirs are distinct
  let b = (cards_dir "/root" "blue")
  let g = (cards_dir "/root" "green")
  assert ($b != $g) "blue and green dirs must differ"

  print "PASS: run_blue_green.nu"
}

def main [
  --test  # Run internal self-tests
] {
  if $test {
    run_tests
    return
  }

  let root = ($env.FILE_PWD | path dirname)
  let bop = ($root | path join "target" "debug" "bop")
  let blue_dir = (cards_dir $root "blue")
  let green_dir = (cards_dir $root "green")
  let adapter_default = (adapter_path $root "mock")

  if not ($bop | path exists) {
    print -e $"Missing bop binary: ($bop)"
    print -e "Run: cargo build"
    exit 1
  }

  mkdir $blue_dir $green_dir
  ^$bop --cards-dir $blue_dir init o> /dev/null
  ^$bop --cards-dir $green_dir init o> /dev/null

  # Keep templates/providers in sync from the canonical cards root when present
  let cards_root = ($root | path join ".cards")
  let templates_src = ($cards_root | path join "templates")
  let providers_src = ($cards_root | path join "providers.json")
  if ($templates_src | path exists) {
    ^rsync -a --delete $"($templates_src)/" $"(($blue_dir | path join "templates"))/"
    ^rsync -a --delete $"($templates_src)/" $"(($green_dir | path join "templates"))/"
  }
  if ($providers_src | path exists) {
    cp $providers_src ($blue_dir | path join "providers.json")
    cp $providers_src ($green_dir | path join "providers.json")
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
