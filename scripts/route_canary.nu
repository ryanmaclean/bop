#!/usr/bin/env nu
# route_canary.nu — route pending cards to blue/green lanes with canary support

def main [
  --green-pct: int = 20  # Percentage of traffic routed to green lane
] {
  let root = ($env.FILE_PWD | path dirname)
  let source_dir = $"($root)/.cards"
  let blue_dir = $"($root)/.cards-blue"
  let green_dir = $"($root)/.cards-green"
  let canary_teams_file = $"($root)/.cards/canary-teams.txt"
  let route_override_file = $"($root)/.cards/route.override"

  if not ($"($source_dir)/pending" | path exists) {
    print -e $"No pending source dir at ($source_dir)/pending"
    exit 1
  }

  mkdir $"($blue_dir)/pending" $"($green_dir)/pending"

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
  let cards = (glob $"($source_dir)/pending/*.bop" | where ($it | path type) == "dir")

  mut moved = 0
  mut dual_count = 0

  for card in $cards {
    let base = ($card | path basename)
    let id = ($base | str replace ".bop" "")

    # Read team from meta.json if present
    mut team = ""
    let meta_path = $"($card)/meta.json"
    if ($meta_path | path exists) {
      try {
        let meta = (open $meta_path)
        $team = ($meta | get -o team | default "")
      }
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
  if $nu.os-info.name == "macos" {
    # Try APFS clone copy first
    let ditto_result = (do { ^ditto --clone $src $"($dst_dir)/($base)" } | complete)
    if $ditto_result.exit_code == 0 { return true }

    let cp_clone = (do { ^cp -cR $src $"($dst_dir)/" } | complete)
    if $cp_clone.exit_code == 0 { return true }

    print -e $"copy_card: APFS clone copy failed for ($src) -> ($dst_dir)"
    return false
  }

  # Linux: try reflink, fallback to regular copy
  let reflink = (do { ^cp --reflink=auto -r $src $"($dst_dir)/" } | complete)
  if $reflink.exit_code == 0 { return true }

  cp -r $src $"($dst_dir)/"
  true
}

def move_or_copy [
  lane: string
  card: string
  base: string
  blue_dir: string
  green_dir: string
]: nothing -> bool {
  match $lane {
    "blue" => {
      if ($"($blue_dir)/pending/($base)" | path exists) {
        print -e $"skip ($base): already exists in blue"
        return false
      }
      mv $card $"($blue_dir)/pending/"
    }
    "green" => {
      if ($"($green_dir)/pending/($base)" | path exists) {
        print -e $"skip ($base): already exists in green"
        return false
      }
      mv $card $"($green_dir)/pending/"
    }
    "dual" => {
      if ($"($blue_dir)/pending/($base)" | path exists) or ($"($green_dir)/pending/($base)" | path exists) {
        print -e $"skip ($base): already exists in blue or green"
        return false
      }
      if not (copy_card $card $"($blue_dir)/pending") { return false }
      if not (copy_card $card $"($green_dir)/pending") { return false }
      rm -rf $card
    }
    _ => {
      print -e $"invalid lane: ($lane)"
      return false
    }
  }
  true
}
