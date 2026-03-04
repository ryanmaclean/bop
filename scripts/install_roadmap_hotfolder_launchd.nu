#!/usr/bin/env nu
# Installs a launchd QueueDirectories agent that watches a roadmap hot folder
# and creates roadmap bops in pending/.

def main [
  --inbox: string      # Hot folder directory to watch
  --cards-dir: string  # Cards root directory
  --uninstall          # Uninstall the launchd agent
  --status             # Check agent status
] {
  let root = ($env.FILE_PWD | path dirname)
  let label = "sh.bop.roadmap-inbox"
  let plist_path = $"($env.HOME)/Library/LaunchAgents/($label).plist"
  let inbox_dir = ($inbox | default $"($root)/examples/roadmap-inbox/drop")
  let cards = ($cards_dir | default $"($root)/.cards")
  let script_path = $"($root)/scripts/ingest_roadmap_hotfolder.nu"
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
  mkdir $"($env.HOME)/Library/LaunchAgents" $inbox_dir $"($inbox_parent)/processed" $"($inbox_parent)/failed"

  let plist_content = $'<?xml version="1.0" encoding="UTF-8"?>
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
