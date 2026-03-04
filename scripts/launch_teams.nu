#!/usr/bin/env nu
# launch_teams.nu — Launch 5 team dispatchers in a zellij session, one pane each.

def main [] {
  let root = ($env.FILE_PWD | path dirname)
  let bop = $"($root)/target/debug/bop"

  let teams = [
    { name: "team-cli",          adapter_name: "claude",   adapter_path: "adapters/claude.nu" }
    { name: "team-arch",         adapter_name: "claude",   adapter_path: "adapters/claude.nu" }
    { name: "team-quality",      adapter_name: "aider",    adapter_path: "adapters/aider.nu" }
    { name: "team-intelligence", adapter_name: "opencode", adapter_path: "adapters/opencode.nu" }
    { name: "team-platform",     adapter_name: "codex",    adapter_path: "adapters/codex.nu" }
  ]

  let session = "bop-teams"

  # Kill existing session if present
  do { ^zellij delete-session $session --force } | complete | ignore
  sleep 500ms

  print $"Launching dispatchers in zellij session: ($session)"

  for entry in ($teams | enumerate) {
    let i = $entry.index
    let team = $entry.item
    let cards_dir = $"($root)/.cards/($team.name)"
    let log_file = $"/tmp/bop-($team.name).log"

    let cmd = $"($bop) --cards-dir ($cards_dir) dispatcher --adapter ($root)/($team.adapter_path) --max-workers 5 --poll-ms 500 --max-retries 3 --reap-ms 2000"

    print $"  -> ($team.name) \(($team.adapter_name)\) -> ($cards_dir)"

    if $i == 0 {
      # First pane: create session
      ^zellij --session $session run --name $team.name --floating --close-on-exit -- nu -c $"print $'=== ($team.name) ==='; ($cmd) 2>&1 | tee ($log_file)"
      sleep 500ms
    } else {
      # Subsequent panes: attach to session
      ^zellij --session $session run --name $team.name --direction down --close-on-exit -- nu -c $"print $'=== ($team.name) ==='; ($cmd) 2>&1 | tee ($log_file)"
      sleep 300ms
    }
  }

  print ""
  print "All 5 dispatchers launched."
  print "Watch logs:  tail -f /tmp/bop-team-*.log"
  print "Check status per team:"
  for team in [team-cli team-arch team-quality team-intelligence team-platform] {
    print $"   --cards-dir ($root)/.cards/($team) status"
  }
}
