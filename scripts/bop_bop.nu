#!/usr/bin/env nu
# bop_bop.nu — bootstrap script for creating cards
#
# Usage: bop_bop.nu <goal description>

def slugify [text: string]: nothing -> string {
  $text
    | str downcase
    | str replace --all " " "-"
    | str replace --all --regex "[^a-z0-9-]" ""
    | str substring 0..40
}

def run_tests [] {
  use std/assert

  # Test basic slug generation
  assert equal (slugify "Hello World") "hello-world"

  # Test uppercase conversion
  assert equal (slugify "FooBar") "foobar"

  # Test special characters removed
  assert equal (slugify "Fix bug #42!") "fix-bug-42"

  # Test truncation (0..40 is inclusive, yields at most 41 chars)
  let long_input = "this is a very long goal description that exceeds forty characters easily"
  let slug = slugify $long_input
  assert (($slug | str length) <= 41) "slug should be truncated to at most 41 chars"

  # Test spaces become hyphens
  assert equal (slugify "a b c") "a-b-c"

  # Test empty after stripping
  assert equal (slugify "!!!") ""

  # Test card id prefix
  let slug = (slugify "my task")
  let id = $"bop-($slug)"
  assert equal $id "bop-my-task"

  print "PASS: bop_bop.nu"
}

def main [
  --test  # Run internal self-tests
  ...goal: string
] {
  if $test {
    run_tests
    return
  }

  if ($goal | length) == 0 {
    print -e "Usage: bop_bop.nu <goal description>"
    exit 1
  }

  let goal_text = ($goal | str join " ")
  let root = ($env.FILE_PWD | path dirname)
  let bop = ($root | path join "target" "debug" "bop")

  # Slugify goal -> card id
  let id_raw = (slugify $goal_text)
  let id = $"bop-($id_raw)"
  let session = $"bop-($id)"

  print $"▶ Creating card: ($id)"
  do { ^$bop new implement $id } | complete | ignore

  # Locate the card
  mut card_dir = ""
  for state in [pending running done] {
    let candidate = ($root | path join ".cards" $state $"($id).bop")
    if ($candidate | path exists) {
      $card_dir = $candidate
      break
    }
  }

  if $card_dir == "" {
    print -e "ERROR: card not found after creation"
    exit 1
  }

  # Write goal into spec.md
  $"# ($goal_text)\n\nCreated by bop_bop.nu.\n" | save --append ($card_dir | path join "spec.md")

  # Write zellij_session into meta.json
  let meta_path = ($card_dir | path join "meta.json")
  let meta = (open $meta_path | from json | upsert zellij_session $session | upsert zellij_pane "1")
  $meta | to json --indent 2 | save --force $meta_path
  print $"  zellij_session: ($session)"

  # Budget-aware agent routing
  # Cost order: ollama (free) → opencode ($) → codex ($) → claude ($$)
  mut provider = "mock"
  mut adapter = ($root | path join "adapters" "mock.nu")

  if (which ollama | length) > 0 {
    $provider = "ollama"
    $adapter = ($root | path join "adapters" "ollama-local.nu")
  }
  if (which opencode | length) > 0 {
    $provider = "opencode"
    $adapter = ($root | path join "adapters" "opencode.nu")
  }
  if (which codex | length) > 0 {
    $provider = "codex"
    $adapter = ($root | path join "adapters" "codex.nu")
  }
  if (which claude | length) > 0 {
    $provider = "claude"
    $adapter = ($root | path join "adapters" "claude.nu")
  }

  print $"  provider: ($provider)"
  print $"  session:  ($session)"
  print ""
  print $"▶ bop://card/($id)/session"
  print ""

  # Create JJ workspace if jj is available and repo is initialized
  if (which jj | length) > 0 {
    let jj_check = (do { ^jj root --repository $root } | complete)
    if $jj_check.exit_code == 0 {
      let worktree_dir = ($root | path join ".worktrees" $id)
      do { ^jj workspace add $worktree_dir } | complete | ignore

      let meta2 = (open $meta_path | from json
        | upsert worktree_branch $"job/($id)"
        | upsert workspace_path $worktree_dir)
      $meta2 | to json --indent 2 | save --force $meta_path
    }
  }

  # Launch or attach resumable zellij session with dispatcher inside
  if (which zellij | length) > 0 {
    let attach_result = (do { ^zellij attach $session } | complete)
    if $attach_result.exit_code != 0 {
      ^zellij -s $session -- $bop dispatcher --adapter $adapter --once
    }
  } else {
    # No zellij: run dispatcher inline
    ^$bop dispatcher --adapter $adapter --once
  }
}
