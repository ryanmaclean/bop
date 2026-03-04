#!/usr/bin/env nu
# Scans a hot folder for roadmap request files and atomically creates roadmap
# bop bundles in pending/ without requiring interactive CLI usage.

def usage [] {
  print "usage: ingest_roadmap_hotfolder.nu [--inbox DIR] [--cards-dir DIR] [--template-dir DIR] [--processed-dir DIR] [--failed-dir DIR] [--dry-run]"
  print ""
  print "Scans a hot folder for roadmap request files and atomically creates roadmap"
  print "bop bundles in pending/ without requiring interactive CLI usage."
}

def slugify [raw: string] {
  let ascii = ($raw
    | str downcase
    | str replace --all --regex '[^a-z0-9._-]' '-'
    | str replace --all '--' '-'
    | str trim --char '-'
  )
  $ascii
}

def supports_request_file [ext: string] {
  let lower = ($ext | str downcase)
  $lower in ["roadmap" "md" "txt" "json" "yaml" "yml"]
}

def id_exists_anywhere [cards_dir: string, id: string] {
  for state in ["pending" "running" "done" "merged" "failed"] {
    let exact = $"($cards_dir)/($state)/($id).bop"
    if ($exact | path exists) {
      return true
    }
    let matches = (glob $"($cards_dir)/($state)/*-($id).bop")
    if ($matches | length) > 0 {
      return true
    }
  }
  false
}

def clone_template [src: string, dst: string] {
  if $nu.os-info.name == "macos" {
    # Try APFS clone with ditto first
    let result = (do { ^ditto --clone --extattr --acl --qtn --preserveHFSCompression $src $dst } | complete)
    if $result.exit_code == 0 {
      return
    }
    # Try cp -cR (APFS CoW)
    let result2 = (do { ^cp -cR $src $dst } | complete)
    if $result2.exit_code == 0 {
      return
    }
    print --stderr $"clone_template: APFS clone copy failed for ($src) -> ($dst)"
    error make {msg: "clone failed"}
  }

  # Linux: try reflink first, fall back to regular copy
  let result = (do { ^cp --reflink=auto -r $src $dst } | complete)
  if $result.exit_code != 0 {
    ^cp -R $src $dst
  }
}

def write_meta [template_meta: string, target_meta: string, id: string, created: string] {
  let content = (open --raw $template_meta)
  let updated = ($content
    | str replace '"id": "REPLACE_ID"' $'"id": "($id)"'
    | str replace $'"worktree_branch": "job/REPLACE_ID"' $'"worktree_branch": "job/($id)"'
    | str replace '"created": "2026-03-01T00:00:00Z"' $'"created": "($created)"'
  )
  $updated | save --force $target_meta
}

def write_spec_from_request [request_file: string, spec_file: string, id: string, created: string] {
  let request_name = ($request_file | path basename)
  let request_content = (open --raw $request_file)
  let spec = ([
    "# Roadmap Request"
    ""
    $"- ID: ($id)"
    $"- Source: ($request_name)"
    $"- Received \(UTC\): ($created)"
    ""
    "## Request"
    ""
    "```"
    $request_content
    "```"
    ""
    "## Expected Workflow"
    ""
    "- Analyze"
    "- Discover"
    "- Generate"
    "- QA"
  ] | str join "\n")
  $spec | save --force $spec_file
}

def render_thumbnail_if_possible [root: string, card_dir: string] {
  let renderer = $"($root)/scripts/render_card_thumbnail.swift"
  let meta_file = $"($card_dir)/meta.json"
  let ql_dir = $"($card_dir)/QuickLook"
  let out_file = $"($ql_dir)/Thumbnail.png"

  mkdir $ql_dir
  if ($renderer | path exists) and ($meta_file | path exists) {
    do { ^swift $renderer $meta_file $out_file } | complete | ignore
  }
}

def move_request [src: string, dst_dir: string] {
  let stamp = (date now | format date "%Y%m%dT%H%M%SZ")
  let name = ($src | path basename)
  ^mv $src $"($dst_dir)/($stamp)-($name)"
}

def process_request [
  root: string,
  request_file: string,
  template_dir: string,
  pending_dir: string,
  processed_dir: string,
  log_file: string,
  dry_run: bool,
  cards_dir: string,
] {
  let ext = ($request_file | path parse | get extension)
  if not (supports_request_file $ext) {
    let name = ($request_file | path basename)
    log_line $log_file $"skip unsupported file: ($name)"
    return
  }

  let stem = ($request_file | path parse | get stem)
  mut id = (slugify $stem)
  if ($id | is-empty) {
    $id = $"roadmap-(date now | format date '%s')"
  }
  if (id_exists_anywhere $cards_dir $id) {
    $id = $"($id)-(date now | format date '%H%M%S')"
  }

  let created = (date now | format date "%Y-%m-%dT%H:%M:%SZ")
  let card_name = $"🂠-($id).bop"
  let final_card = $"($pending_dir)/($card_name)"
  let temp_card = $"($pending_dir)/.($card_name).tmp.(random int 10000..99999)"

  if ($temp_card | path exists) {
    ^rm -rf $temp_card
  }
  clone_template $template_dir $temp_card

  mkdir $"($temp_card)/logs" $"($temp_card)/output"
  write_meta $"($template_dir)/meta.json" $"($temp_card)/meta.json" $id $created
  write_spec_from_request $request_file $"($temp_card)/spec.md" $id $created
  render_thumbnail_if_possible $root $temp_card

  if $dry_run {
    let name = ($request_file | path basename)
    log_line $log_file $"dry-run: would queue ($card_name) from ($name)"
    ^rm -rf $temp_card
    return
  }

  ^mv $temp_card $final_card
  let name = ($request_file | path basename)
  move_request $request_file $processed_dir
  log_line $log_file $"queued ($card_name) from ($name)"
}

def log_line [log_file: string, msg: string] {
  let ts = (date now | format date "%Y-%m-%dT%H:%M:%SZ")
  $"($ts) ($msg)\n" | save --append $log_file
}

def run_tests [] {
  use std/assert

  # Test slugify: basic lowercasing and replacement
  assert equal (slugify "Hello World") "hello-world" "slugify basic"
  assert equal (slugify "My--Task") "my-task" "slugify double dash"
  assert equal (slugify "---leading---") "leading" "slugify leading/trailing dashes"
  assert equal (slugify "foo_bar.baz") "foo_bar.baz" "slugify preserves underscores and dots"
  assert equal (slugify "UPPER CASE 123") "upper-case-123" "slugify upper case with numbers"
  assert equal (slugify "") "" "slugify empty string"

  # Test supports_request_file
  assert (supports_request_file "yaml") "yaml is supported"
  assert (supports_request_file "yml") "yml is supported"
  assert (supports_request_file "md") "md is supported"
  assert (supports_request_file "txt") "txt is supported"
  assert (supports_request_file "json") "json is supported"
  assert (supports_request_file "roadmap") "roadmap is supported"
  assert (supports_request_file "YAML") "YAML uppercase is supported"
  assert (not (supports_request_file "exe")) "exe is not supported"
  assert (not (supports_request_file "py")) "py is not supported"

  # Test template path construction
  let cards = "/tmp/test-cards"
  let tmpl = $"($cards)/templates/roadmap.bop"
  assert equal $tmpl "/tmp/test-cards/templates/roadmap.bop" "template path construction"

  # Test pending dir construction
  let pending = $"($cards)/pending"
  assert equal $pending "/tmp/test-cards/pending" "pending dir construction"

  # Test YAML parsing with temp file
  let tmp_dir = (mktemp -d)
  let yaml_file = $"($tmp_dir)/test.yaml"
  "title: Test Roadmap\nitems:\n  - step one\n  - step two\n" | save $yaml_file
  let content = (open --raw $yaml_file)
  assert ($content | str contains "title: Test Roadmap") "YAML file readable"
  assert ($content | str contains "step one") "YAML content has items"
  rm -rf $tmp_dir

  # Test card name construction
  let id = "my-roadmap"
  let card_name = $"🂠-($id).bop"
  assert equal $card_name "🂠-my-roadmap.bop" "card name construction"

  print "PASS: ingest_roadmap_hotfolder.nu"
}

def main [
  --test                    # Run internal self-tests
  --inbox: string           # Hot folder directory to watch
  --cards-dir: string       # Cards root directory
  --template-dir: string    # Template directory path
  --processed-dir: string   # Directory for processed request files
  --failed-dir: string      # Directory for failed request files
  --dry-run                 # Preview without creating cards
] {
  if $test {
    run_tests
    return
  }
  let root = ($env.FILE_PWD | path dirname)
  let inbox_dir = ($inbox | default $"($root)/examples/roadmap-inbox/drop")
  let cards = ($cards_dir | default $"($root)/.cards")
  let inbox_parent = ($inbox_dir | path dirname)
  let tmpl_dir = ($template_dir | default $"($cards)/templates/roadmap.bop")
  let proc_dir = ($processed_dir | default $"($inbox_parent)/processed")
  let fail_dir = ($failed_dir | default $"($inbox_parent)/failed")
  let pending_dir = $"($cards)/pending"
  let log_dir = $"($cards)/logs"
  let log_file = $"($log_dir)/roadmap-hotfolder.log"

  mkdir $inbox_dir $proc_dir $fail_dir $pending_dir $log_dir

  if not ($tmpl_dir | path exists) {
    print --stderr $"missing template directory: ($tmpl_dir)"
    exit 1
  }
  if not ($"($tmpl_dir)/meta.json" | path exists) {
    print --stderr $"missing template meta.json in: ($tmpl_dir)"
    exit 1
  }

  let requests = (glob $"($inbox_dir)/*" | where {|f| ($f | path type) == "file" })
  if ($requests | is-empty) {
    return
  }

  for request_file in $requests {
    let result = (do {
      process_request $root $request_file $tmpl_dir $pending_dir $proc_dir $log_file $dry_run $cards
    } | complete)
    if $result.exit_code != 0 {
      let name = ($request_file | path basename)
      log_line $log_file $"failed to ingest ($name)"
      if ($request_file | path exists) {
        move_request $request_file $fail_dir
      }
    }
  }
}
