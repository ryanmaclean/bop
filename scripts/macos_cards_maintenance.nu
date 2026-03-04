#!/usr/bin/env nu
# macOS card thumbnail refresh and optional HFS compression.
# Darwin only.

def refresh_thumbnail [render_swift: string, card_dir: string] {
  let meta = $"($card_dir)/meta.json"
  let out = $"($card_dir)/QuickLook/Thumbnail.png"
  if not ($meta | path exists) {
    return
  }
  mkdir $"($card_dir)/QuickLook"
  do { ^swift $render_swift $meta $out } | complete | ignore
}

def compress_card_hfs [card_dir: string] {
  let parent = ($card_dir | path dirname)
  let name = ($card_dir | path basename)
  let tmp = $"($parent)/($name).hfs.tmp"
  let backup = $"($parent)/($name).bak.tmp"
  if ($tmp | path exists) { ^rm -rf $tmp }
  if ($backup | path exists) { ^rm -rf $backup }

  let result = (do { ^ditto --hfsCompression $card_dir $tmp } | complete)
  if $result.exit_code != 0 {
    if ($tmp | path exists) { ^rm -rf $tmp }
    return
  }
  ^mv $card_dir $backup
  let mv_result = (do { ^mv $tmp $card_dir } | complete)
  if $mv_result.exit_code == 0 {
    ^rm -rf $backup
  } else {
    do { ^mv $backup $card_dir } | complete | ignore
    if ($tmp | path exists) { ^rm -rf $tmp }
  }
}

def card_glob_patterns [cards_root: string] {
  [
    $"($cards_root)/done/*.bop"
    $"($cards_root)/failed/*.bop"
    $"($cards_root)/merged/*.bop"
    $"($cards_root)/team-*/done/*.bop"
    $"($cards_root)/team-*/failed/*.bop"
    $"($cards_root)/team-*/merged/*.bop"
  ]
}

def run_tests [] {
  use std/assert

  # Test card_glob_patterns construction
  let patterns = (card_glob_patterns "/tmp/.cards")
  assert equal ($patterns | length) 6 "should produce 6 glob patterns"
  assert equal ($patterns | get 0) "/tmp/.cards/done/*.bop" "first pattern is done"
  assert equal ($patterns | get 1) "/tmp/.cards/failed/*.bop" "second pattern is failed"
  assert equal ($patterns | get 2) "/tmp/.cards/merged/*.bop" "third pattern is merged"
  assert ($patterns | get 3 | str contains "team-*/done") "fourth pattern has team done"
  assert ($patterns | get 4 | str contains "team-*/failed") "fifth pattern has team failed"
  assert ($patterns | get 5 | str contains "team-*/merged") "sixth pattern has team merged"

  # Test all patterns end with *.bop
  for pat in $patterns {
    assert ($pat | str ends-with "*.bop") $"pattern ($pat) ends with *.bop"
  }

  # Test platform detection
  let platform = $nu.os-info.name
  assert ($platform in ["macos" "linux" "windows"]) $"platform ($platform) is recognized"

  # Test QuickLook path construction
  let card_dir = "/tmp/.cards/done/test.bop"
  let ql_dir = $"($card_dir)/QuickLook"
  let thumb = $"($card_dir)/QuickLook/Thumbnail.png"
  assert equal $ql_dir "/tmp/.cards/done/test.bop/QuickLook" "QuickLook dir path"
  assert equal $thumb "/tmp/.cards/done/test.bop/QuickLook/Thumbnail.png" "Thumbnail path"

  # Test meta.json path construction
  let meta = $"($card_dir)/meta.json"
  assert equal $meta "/tmp/.cards/done/test.bop/meta.json" "meta.json path"

  print "PASS: macos_cards_maintenance.nu"
}

def main [
  --test      # Run internal self-tests
  --compress  # Also HFS-compress done/failed/merged cards
] {
  if $test {
    run_tests
    return
  }

  if $nu.os-info.name != "macos" {
    print --stderr "macos_cards_maintenance: Darwin only"
    exit 1
  }

  let root = ($env.FILE_PWD | path dirname)
  let cards_root = $"($root)/.cards"
  let render_swift = $"($root)/scripts/render_card_thumbnail.swift"

  if not ($render_swift | path exists) {
    print --stderr $"missing renderer: ($render_swift)"
    exit 1
  }

  # Collect card directories from done/failed/merged (flat + team layouts)
  mut cards = []
  for pattern in (card_glob_patterns $cards_root) {
    let matches = (glob $pattern)
    $cards = ($cards | append $matches)
  }

  for card_dir in $cards {
    if not ($card_dir | path exists) or (($card_dir | path type) != "dir") {
      continue
    }
    refresh_thumbnail $render_swift $card_dir
    if $compress {
      compress_card_hfs $card_dir
    }
  }

  print $"maintenance complete \(compress=($compress)\)"
}
