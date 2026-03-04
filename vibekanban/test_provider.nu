#!/usr/bin/env nu
# Test for bop-provider.nu — creates a temp .cards/ structure and validates output.

def main [] {
  let tmp = (^mktemp -d | str trim)

  # Cleanup on exit handled by wrapping in try block
  try {
    mkdir $"($tmp)/team-cli/pending/card-abc.bop"
    mkdir $"($tmp)/team-cli/running/card-xyz.bop"
    '{"id":"card-abc","title":"Test card","stage":"implement"}' | save $"($tmp)/team-cli/pending/card-abc.bop/meta.json"
    '{"id":"card-xyz","title":"Running card","stage":"test"}' | save $"($tmp)/team-cli/running/card-xyz.bop/meta.json"

    # Run provider
    let script_dir = $env.FILE_PWD
    $env.CARDS_DIR = $tmp
    let out = (nu $"($script_dir)/bop-provider.nu")
    print $"Provider output: ($out)"

    # Validate: must be a JSON array with 2 items
    let data = ($out | from json)
    let count = ($data | length)
    if $count != 2 {
      print $"FAIL: expected 2 tasks, got ($count)"
      ^rm -rf $tmp
      exit 1
    }

    let id = ($data | get 0.id)
    let card_status = ($data | get 0.status)
    if $id != "card-abc" {
      print $"FAIL: expected id=card-abc, got ($id)"
      ^rm -rf $tmp
      exit 1
    }
    if $card_status != "pending" {
      print $"FAIL: expected status=pending, got ($card_status)"
      ^rm -rf $tmp
      exit 1
    }

    print "PASS: provider outputs correct JSON"
  } catch {
    ^rm -rf $tmp
    exit 1
  }

  ^rm -rf $tmp
}
