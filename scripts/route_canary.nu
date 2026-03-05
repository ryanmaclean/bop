#!/usr/bin/env nu
# route_canary.nu — route pending cards to blue/green lanes with canary support

def run_tests [] {
  use std/assert

  # Test deterministic_bucket: known input produces consistent output
  let bucket1 = (deterministic_bucket "test-card-1")
  let bucket2 = (deterministic_bucket "test-card-1")
  assert equal $bucket1 $bucket2 "deterministic_bucket is deterministic for same input"

  # Test bucket is always in [0, 100)
  for key in ["a" "b" "foo" "bar-baz" "card-123" "🂠-roadmap" "z" ""] {
    let b = (deterministic_bucket $key)
    assert ($b >= 0) $"bucket for '($key)' >= 0, got ($b)"
    assert ($b < 100) $"bucket for '($key)' < 100, got ($b)"
  }

  # Test different inputs produce different buckets (probabilistic but reliable for these)
  let ba = (deterministic_bucket "alpha")
  let bb = (deterministic_bucket "beta")
  # Not guaranteed different, but extremely unlikely to collide for these
  # Just verify they return valid numbers
  assert ($ba >= 0 and $ba < 100) "alpha bucket in range"
  assert ($bb >= 0 and $bb < 100) "beta bucket in range"

  # Test route_lane with overrides
  let lane_blue = (route_lane "card-1" "" "blue-only" [] 50)
  assert equal $lane_blue "blue" "blue-only override routes to blue"

  let lane_green = (route_lane "card-1" "" "green-only" [] 50)
  assert equal $lane_green "green" "green-only override routes to green"

  # Test route_lane with canary team
  let lane_dual = (route_lane "card-1" "teamA" "" ["teamA" "teamB"] 50)
  assert equal $lane_dual "dual" "canary team routes to dual"

  # Test route_lane balanced (no override, no canary)
  let lane_balanced = (route_lane "card-1" "teamX" "" ["teamA"] 50)
  assert ($lane_balanced == "blue" or $lane_balanced == "green") "balanced routes to blue or green"

  # Test team extraction from JSON-like data
  let meta_json = '{"id": "test", "team": "platform"}'
  let parsed = ($meta_json | from json)
  let team = ($parsed | get -o team | default "")
  assert equal $team "platform" "team extracted from JSON"

  # Test empty team from JSON
  let meta_no_team = '{"id": "test"}'
  let parsed2 = ($meta_no_team | from json)
  let team2 = ($parsed2 | get -o team | default "")
  assert equal $team2 "" "missing team defaults to empty string"

  print "PASS: route_canary.nu"
}

def main [
  --test                 # Run internal self-tests
  --green-pct: int = 20  # Percentage of traffic routed to green lane
] {
  if $test {
    run_tests
    return
  }
  let root = ($env.FILE_PWD | path dirname)
  let source_dir = ($root | path join ".cards")
  let blue_dir = ($root | path join ".cards-blue")
  let green_dir = ($root | path join ".cards-green")
  let canary_teams_file = ($root | path join ".cards" "canary-teams.txt")
  let route_override_file = ($root | path join ".cards" "route.override")

  let pending_dir = ($source_dir | path join "pending")
  if not ($pending_dir | path exists) {
    print -e $"No pending source dir at ($pending_dir)"
    exit 1
  }

  mkdir ($blue_dir | path join "pending") ($green_dir | path join "pending")

  # Read route override
  mut route_override = ""
  if ($route_override_file | path exists) {
    let raw = (open --raw $route_override_file | lines | first | default "")
    # Strip comments and trim
    $route_override = ($raw | split column "#" | get column1.0 | str trim)
  }

  # Read canary teams
  mut canary_teams: list<string> = []
  if ($canary_teams_file | path exists) {
    let lines = (open --raw $canary_teams_file | lines)
    for line in $lines {
      let cleaned = ($line | split column "#" | get column1.0 | str trim)
      if ($cleaned | is-not-empty) {
        $canary_teams = ($canary_teams | append $cleaned)
      }
    }
  }

  # Get pending cards
  let cards = (glob ($pending_dir | path join "*.bop") | where ($it | path type) == "dir")

  mut moved = 0
  mut dual_count = 0

  for card in $cards {
    let base = ($card | path basename)
    let id = ($base | str replace ".bop" "")

    # Read team from meta.json if present
    mut team = ""
    let meta_path = ($card | path join "meta.json")
    if ($meta_path | path exists) {
      let meta = (open $meta_path)
      $team = ($meta | get -o team | default "")
    }

    let lane = (route_lane $id $team $route_override $canary_teams $green_pct)
    let result = (move_or_copy $lane $card $base $blue_dir $green_dir)

    if $result {
      if $lane == "dual" {
        print $"($id) -> blue+green \(canary\)"
        $dual_count = $dual_count + 1
      } else {
        print $"($id) -> ($lane)"
      }
      $moved = $moved + 1
    }
  }

  let override_label = if ($route_override | is-empty) { "balanced" } else { $route_override }
  print $"Routed ($moved) pending cards \(green=($green_pct)% deterministic, canary dual cards=($dual_count), override='($override_label)'\)."
}

def route_lane [
  card_id: string
  team: string
  route_override: string
  canary_teams: list<string>
  green_pct: int
]: nothing -> string {
  match $route_override {
    "blue-only" => { return "blue" }
    "green-only" => { return "green" }
    "" | "balanced" => { }
    _ => { print -e $"Unknown route override '($route_override)'; treating as balanced" }
  }

  if ($team | is-not-empty) and ($team in $canary_teams) {
    return "dual"
  }

  let bucket = (deterministic_bucket $card_id)
  if $bucket < $green_pct {
    "green"
  } else {
    "blue"
  }
}

def deterministic_bucket [key: string]: nothing -> int {
  let hash = ($key | hash sha256)
  # Take first 8 hex chars and convert to int mod 100
  let hex8 = ($hash | str substring 0..8)
  # Use nu's built-in to parse hex
  let val = ($"0x($hex8)" | into int)
  $val mod 100
}

def copy_card [src: string, dst_dir: string]: nothing -> bool {
  let base = ($src | path basename)
  let dst_path = ($dst_dir | path join $base)
  if $nu.os-info.name == "macos" {
    # Try APFS clone copy first
    let ditto_result = (do { ^ditto --clone $src $dst_path } | complete)
    if $ditto_result.exit_code == 0 { return true }

    let cp_clone = (do { ^cp -cR $src $dst_dir } | complete)
    if $cp_clone.exit_code == 0 { return true }

    print -e $"copy_card: APFS clone copy failed for ($src) -> ($dst_dir)"
    return false
  }

  # Linux: try reflink, fallback to regular copy
  let reflink = (do { ^cp --reflink=auto -r $src $dst_dir } | complete)
  if $reflink.exit_code == 0 { return true }

  cp -r $src $dst_dir
  true
}

def move_or_copy [
  lane: string
  card: string
  base: string
  blue_dir: string
  green_dir: string
]: nothing -> bool {
  let blue_pending = ($blue_dir | path join "pending")
  let green_pending = ($green_dir | path join "pending")
  match $lane {
    "blue" => {
      if ($blue_pending | path join $base | path exists) {
        print -e $"skip ($base): already exists in blue"
        return false
      }
      mv $card $blue_pending
    }
    "green" => {
      if ($green_pending | path join $base | path exists) {
        print -e $"skip ($base): already exists in green"
        return false
      }
      mv $card $green_pending
    }
    "dual" => {
      if ($blue_pending | path join $base | path exists) or ($green_pending | path join $base | path exists) {
        print -e $"skip ($base): already exists in blue or green"
        return false
      }
      if not (copy_card $card $blue_pending) { return false }
      if not (copy_card $card $green_pending) { return false }
      rm -rf $card
    }
    _ => {
      print -e $"invalid lane: ($lane)"
      return false
    }
  }
  true
}
