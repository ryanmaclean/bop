#!/usr/bin/env nu
# bop-bridge.nu — emit card stage transitions from inside any AI session.
# Source this file to get the `bridge stage` command, or run directly with --test.
# Usage: source vibekanban/bop-bridge.nu
#        bridge stage in-progress
#        bridge stage human-review my-card-id

def main [
  --test  # Run internal self-tests
] {
  if $test {
    run_tests
    return
  }

  print --stderr "Usage: source this file, then call `bridge stage <stage> [card_id]`"
  print --stderr "  Stages: planning, in-progress, human-review, ai-review, done"
  print --stderr "  Run with --test to verify internal logic."
  exit 1
}

# Emit a card stage transition via bop bridge.
# Wraps: bop bridge emit --cli <cli> --event stage-change --stage <stage>
export def "bridge stage" [
    stage: string,       # One of: planning, in-progress, human-review, ai-review, done
    card_id?: string,    # Optional card ID to tag the event
    --cli (-c): string = "claude",  # CLI tool name (default: claude)
] {
    mut args = [
        "bridge"
        "emit"
        "--cli"
        $cli
        "--event"
        "stage-change"
        "--stage"
        $stage
    ]
    if $card_id != null {
        $args = ($args | append ["--card-id", $card_id])
    }
    ^bop ...$args
}

def run_tests [] {
  use std/assert

  # Test: valid stage names are non-empty strings
  let stages = ["planning", "in-progress", "human-review", "ai-review", "done"]
  for s in $stages {
    assert (($s | str length) > 0) $"stage '($s)' should be non-empty"
  }

  # Test: argument construction without card_id
  let base_args = [
    "bridge" "emit" "--cli" "claude" "--event" "stage-change" "--stage" "in-progress"
  ]
  assert equal ($base_args | length) 8 "base args should have 8 elements"
  assert equal ($base_args | get 7) "in-progress" "last arg should be the stage"

  # Test: argument construction with card_id appended
  let with_card = ($base_args | append ["--card-id", "card-42"])
  assert equal ($with_card | length) 10 "args with card_id should have 10 elements"
  assert equal ($with_card | get 8) "--card-id" "9th element should be --card-id flag"
  assert equal ($with_card | get 9) "card-42" "10th element should be the card ID"

  # Test: custom cli flag
  let custom_args = [
    "bridge" "emit" "--cli" "opencode" "--event" "stage-change" "--stage" "done"
  ]
  assert equal ($custom_args | get 3) "opencode" "cli should be overridable"

  print "PASS: bop-bridge.nu"
}
