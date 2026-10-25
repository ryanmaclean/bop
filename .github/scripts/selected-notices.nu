#!/usr/bin/env nu
# SOURCE CANDIDATE ONLY. Run inside the pinned FreeBSD VM after cargo-deny.
# This is an inventory gate, not a license-policy override or product approval.

def fail [why: string] {
  error make { msg: $why }
}

def sha256_file [path: string] {
  let result = (^sha256 -q $path | complete)
  if $result.exit_code != 0 { fail $"sha256 failed: ($path)" }
  $result.stdout | str trim
}

def main [
  manifest: string,
  package: string,
  features: string,
  lock: string,
  expected_lock_sha256: string,
  expected_head: string,
  receipt_dir: string
] {
  let target = 'x86_64-unknown-freebsd'
  let lock_hash = (sha256_file $lock)
  if $lock_hash != $expected_lock_sha256 { fail 'Cargo.lock bytes changed' }
  let root = ($lock | path dirname | path expand)
  let head_result = (^git -c $"safe.directory=($root)" -C $root rev-parse HEAD | complete)
  if $head_result.exit_code != 0 or ($head_result.stdout | str trim) != $expected_head {
    fail 'source head changed'
  }
  if ('CARGO_HOME' not-in $env) { fail 'CARGO_HOME must be explicit' }

  let metadata_result = (^cargo metadata --format-version 1 --locked --offline --filter-platform $target --manifest-path $manifest | complete)
  if $metadata_result.exit_code != 0 { fail 'cargo metadata failed' }
  let metadata = ($metadata_result.stdout | from json)
  let lock_packages = ((open --raw $lock) | from toml | get package)
  let feature_args = if $features == '-' { [] } else { ['--features' $features] }
  let tree_result = (^cargo tree --locked --offline --target $target --edges normal --manifest-path $manifest -p $package ...$feature_args --prefix none --format '{p}' | complete)
  if $tree_result.exit_code != 0 { fail 'selected cargo tree failed' }
  let lines = ($tree_result.stdout | lines)
  if ($lines | length) == 0 { fail 'empty selected tree' }

  mkdir $receipt_dir
  let texts_dir = ($receipt_dir | path join 'texts')
  mkdir $texts_dir
  let selected = ($lines | each {|line|
    let parsed = ($line | parse -r '^(?<name>[^ ]+) v(?<version>[^ ]+)(?: .*)?$')
    if ($parsed | length) != 1 { fail $"unrecognized cargo tree line: ($line)" }
    {name: $parsed.0.name, version: $parsed.0.version}
  } | uniq)
  if ($selected | length) == 0 { fail 'empty selected package ID set' }

  mut rows = []
  mut text_rows = []
  mut n = 0
  for selected_id in $selected {
    let matches = ($metadata.packages | where name == $selected_id.name and version == $selected_id.version)
    if ($matches | length) != 1 { fail $"ambiguous or missing metadata ID: ($selected_id.name) ($selected_id.version)" }
    let pkg = $matches.0
    let source = ($pkg.source? | default '')
    if $source == '' {
      let kind = if ($pkg.id in $metadata.workspace_members) { 'first-party-workspace' } else { 'vendored-path-dependency' }
      if $kind == 'vendored-path-dependency' { fail $"unreviewed vendored path dependency: ($pkg.id)" }
      $rows = ($rows | append {id: $pkg.id, source_kind: $kind, archive_sha256: null, direct_legal_texts: null})
      continue
    }
    if $source != 'registry+https://github.com/rust-lang/crates.io-index' {
      fail $"unreviewed registry/git source: ($pkg.id) ($source)"
    }
    let locked = ($lock_packages | where name == $pkg.name and version == $pkg.version and source == $source)
    if ($locked | length) != 1 { fail $"ambiguous or missing lock row: ($pkg.id)" }
    let checksum = ($locked.0.checksum? | default '')
    if $checksum !~ '^[0-9a-f]{64}$' { fail $"missing exact archive checksum: ($pkg.id)" }
    let archive_name = $"($pkg.name)-($pkg.version).crate"
    let archives = (glob $"($env.CARGO_HOME)/registry/cache/*/($archive_name)")
    if ($archives | length) != 1 { fail $"archive absent or ambiguous: ($pkg.id)" }
    let archive = $archives.0
    if (sha256_file $archive) != $checksum { fail $"archive checksum differs from lock: ($pkg.id)" }
    let listing = (^tar -tzf $archive | complete)
    if $listing.exit_code != 0 { fail $"archive listing failed: ($pkg.id)" }
    let prefix = $"($pkg.name)-($pkg.version)/"
    let members = ($listing.stdout | lines)
    if ($members | length) == 0 { fail $"empty archive: ($pkg.id)" }
    mut direct_count = 0
    for member in $members {
      if not ($member | str starts-with $prefix) { fail $"unexpected archive path: ($pkg.id) ($member)" }
      let rel = ($member | str substring ($prefix | str length)..)
      if $rel =~ '(^|/)\.\.(/|$)' or ($rel | str starts-with '/') { fail $"unsafe archive path: ($member)" }
      let leaf = ($rel | path basename)
      if $leaf !~ '(?i)^(license|licence|copying|notice|copyright)([._-].*)?$' { continue }
      if ($rel | str contains '/') == false { $direct_count = $direct_count + 1 }
      let out = ($texts_dir | path join $"($n).raw")
      # Nu redirection keeps tar external; LAST_EXIT_CODE is checked explicitly.
      ^tar -xOzf $archive $member o> $out
      if $env.LAST_EXIT_CODE != 0 { fail $"legal text extraction failed: ($pkg.id) ($member)" }
      let bytes = (^stat -f '%z' $out | str trim | into int)
      if $bytes == 0 { fail $"empty legal text: ($pkg.id) ($member)" }
      $text_rows = ($text_rows | append {id: $pkg.id, member: $member, raw_file: ($out | path basename), sha256: (sha256_file $out), bytes: $bytes, direct: (not ($rel | str contains '/'))})
      $n = $n + 1
    }
    if $direct_count == 0 { fail $"no direct legal text in exact archive: ($pkg.id)" }
    $rows = ($rows | append {id: $pkg.id, source_kind: 'crates-io-registry', archive_sha256: $checksum, direct_legal_texts: $direct_count})
  }
  $rows | sort-by id | to json | save -f ($receipt_dir | path join 'selected-ids.json')
  $text_rows | sort-by id member | to json | save -f ($receipt_dir | path join 'raw-text-index.json')
  $lock_hash | save -f ($receipt_dir | path join 'lock-sha256.txt')
  $expected_head | save -f ($receipt_dir | path join 'source-head.txt')
  'COMPLETE: exact selected IDs, lock archive hashes and raw legal texts inventoried' | save -f ($receipt_dir | path join 'COMPLETE')
}
