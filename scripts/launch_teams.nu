#!/usr/bin/env nu
# launch_teams.nu — Launch 5 team dispatchers in a zellij session, one pane each.

def build_dispatcher_cmd [bop: string, root: string, team: record<name: string, adapter_name: string, adapter_path: string>]: nothing -> string {
  $"($bop) --cards-dir ($root)/.cards/($team.name) dispatcher --adapter ($root)/($team.adapter_path) --max-workers 5 --poll-ms 500 --max-retries 3 --reap-ms 2000"
}

def build_log_path [team_name: string]: nothing -> string {
  $"/tmp/bop-($team_name).log"
}

def main [
  --test  # Run internal self-tests
] {
  if $test {
    run_tests
    return
  }

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
    let log_file = (build_log_path $team.name)

    let cmd = (build_dispatcher_cmd $bop $root $team)

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

def run_tests [] {
  use std/assert

  # Test: build_log_path
  let log = (build_log_path "team-cli")
  assert equal $log "/tmp/bop-team-cli.log" "log path for team-cli"

  let log2 = (build_log_path "team-platform")
  assert equal $log2 "/tmp/bop-team-platform.log" "log path for team-platform"

  # Test: build_dispatcher_cmd
  let team = { name: "team-cli", adapter_name: "claude", adapter_path: "adapters/claude.nu" }
  let cmd = (build_dispatcher_cmd "/usr/local/bin/bop" "/home/user/bop" $team)
  assert ($cmd | str contains "--cards-dir /home/user/bop/.cards/team-cli") "cmd should include cards-dir"
  assert ($cmd | str contains "--adapter /home/user/bop/adapters/claude.nu") "cmd should include adapter path"
  assert ($cmd | str contains "--max-workers 5") "cmd should include max-workers"
  assert ($cmd | str contains "--poll-ms 500") "cmd should include poll-ms"
  assert ($cmd | str contains "--max-retries 3") "cmd should include max-retries"
  assert ($cmd | str contains "dispatcher") "cmd should include dispatcher subcommand"

  # Test: team list has 5 entries
  let teams = [
    { name: "team-cli",          adapter_name: "claude",   adapter_path: "adapters/claude.nu" }
    { name: "team-arch",         adapter_name: "claude",   adapter_path: "adapters/claude.nu" }
    { name: "team-quality",      adapter_name: "aider",    adapter_path: "adapters/aider.nu" }
    { name: "team-intelligence", adapter_name: "opencode", adapter_path: "adapters/opencode.nu" }
    { name: "team-platform",     adapter_name: "codex",    adapter_path: "adapters/codex.nu" }
  ]
  assert equal ($teams | length) 5 "should have 5 teams"

  # Test: each team has required fields
  for team in $teams {
    assert ($team.name | str starts-with "team-") "team name should start with team-"
    assert ($team.adapter_path | str ends-with ".nu") "adapter path should end with .nu"
    assert (($team.adapter_name | str length) > 0) "adapter name should be non-empty"
  }

  print "PASS: launch_teams.nu"
}
