#!/usr/bin/env nu
# Installs a launchd QueueDirectories agent that watches a roadmap hot folder
# and creates roadmap bops in pending/.

def build_plist [label: string, inbox_dir: string, script_path: string, cards: string] {
  $'<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>(echo $label)</string>

  <key>QueueDirectories</key>
  <array>
    <string>(echo $inbox_dir)</string>
  </array>

  <key>RunAtLoad</key>
  <true/>

  <key>ProgramArguments</key>
  <array>
    <string>nu</string>
    <string>(echo $script_path)</string>
    <string>--inbox</string>
    <string>(echo $inbox_dir)</string>
    <string>--cards-dir</string>
    <string>(echo $cards)</string>
  </array>

  <key>StandardOutPath</key>
  <string>/tmp/bop-roadmap-inbox.log</string>

  <key>StandardErrorPath</key>
  <string>/tmp/bop-roadmap-inbox.err</string>
</dict>
</plist>'
}

def run_tests [] {
  use std/assert

  let label = "sh.bop.roadmap-inbox"
  let inbox = "/tmp/test-inbox"
  let script = "/usr/local/bin/ingest.nu"
  let cards = "/tmp/test-cards"

  let plist = (build_plist $label $inbox $script $cards)

  # Test: plist contains the Label
  assert ($plist | str contains $"<string>($label)</string>") "plist contains label"

  # Test: plist contains QueueDirectories with inbox path
  assert ($plist | str contains $"<string>($inbox)</string>") "plist contains inbox dir"

  # Test: plist contains ProgramArguments with script path
  assert ($plist | str contains $"<string>($script)</string>") "plist contains script path"

  # Test: plist contains cards-dir argument
  assert ($plist | str contains $"<string>($cards)</string>") "plist contains cards dir"

  # Test: plist has RunAtLoad true
  assert ($plist | str contains "<key>RunAtLoad</key>") "plist has RunAtLoad key"
  assert ($plist | str contains "<true/>") "plist RunAtLoad is true"

  # Test: plist has log paths
  assert ($plist | str contains "/tmp/bop-roadmap-inbox.log") "plist has stdout log path"
  assert ($plist | str contains "/tmp/bop-roadmap-inbox.err") "plist has stderr log path"

  # Test: valid XML declaration
  assert ($plist | str starts-with "<?xml version") "plist starts with XML declaration"

  print "PASS: install_roadmap_hotfolder_launchd.nu"
}

def main [
  --test                 # Run internal self-tests
  --inbox: string        # Hot folder directory to watch
  --cards-dir: string    # Cards root directory
  --uninstall            # Uninstall the launchd agent
  --status               # Check agent status
] {
  if $test {
    run_tests
    return
  }

  let root = ($env.FILE_PWD | path dirname)
  let label = "sh.bop.roadmap-inbox"
  let plist_path = ($env.HOME | path join "Library" "LaunchAgents" $"($label).plist")
  let inbox_dir = ($inbox | default ($root | path join "examples" "roadmap-inbox" "drop"))
  let cards = ($cards_dir | default ($root | path join ".cards"))
  let script_path = ($root | path join "scripts" "ingest_roadmap_hotfolder.nu")
  let uid = (^id -u | str trim)
  let service = $"gui/($uid)/($label)"

  if $status {
    let result = (do { ^launchctl print $service } | complete)
    if $result.exit_code == 0 {
      print $"loaded: ($service)"
      print $"plist:  ($plist_path)"
      return
    }
    print $"not loaded: ($service)"
    exit 1
  }

  if $uninstall {
    do { ^launchctl bootout $service } | complete | ignore
    if ($plist_path | path exists) {
      rm $plist_path
      print $"removed ($plist_path)"
    } else {
      print "not installed"
    }
    return
  }

  if not ($script_path | path exists) {
    print --stderr $"missing ingest script: ($script_path)"
    exit 1
  }

  let inbox_parent = ($inbox_dir | path dirname)
  mkdir ($env.HOME | path join "Library" "LaunchAgents") $inbox_dir ($inbox_parent | path join "processed") ($inbox_parent | path join "failed")

  let plist_content = (build_plist $label $inbox_dir $script_path $cards)

  $plist_content | save --force $plist_path

  do { ^launchctl bootout $service } | complete | ignore
  ^launchctl bootstrap $"gui/($uid)" $plist_path
  do { ^launchctl enable $service } | complete | ignore

  print $"installed ($label)"
  print $"hot folder: ($inbox_dir)"
  print $"cards dir:  ($cards)"
  print "logs:       /tmp/bop-roadmap-inbox.log"
  print "error log:  /tmp/bop-roadmap-inbox.err"
}
