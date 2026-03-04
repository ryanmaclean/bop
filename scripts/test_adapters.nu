#!/usr/bin/env nu
# Verify that all adapter and script files use Nushell (.nu) with correct shebangs.
# Ensures no legacy .zsh or .sh files remain.

def main [] {
  let root = ($env.FILE_PWD | path dirname)
  mut failed = false

  # Check all .nu adapter files have correct shebang
  let adapter_files = (glob $"($root)/adapters/*.nu")
  for f in $adapter_files {
    let first_line = (open --raw $f | lines | first)
    if $first_line != "#!/usr/bin/env nu" {
      print $"FAIL: ($f) has wrong shebang"
      $failed = true
    }
  }

  # Check that no legacy .zsh or .sh adapter/script files remain
  let legacy_patterns = [
    $"($root)/adapters/*.zsh"
    $"($root)/adapters/*.sh"
    $"($root)/scripts/*.zsh"
    $"($root)/scripts/*.sh"
  ]
  for pattern in $legacy_patterns {
    let matches = (glob $pattern)
    for f in $matches {
      print $"FAIL: legacy shell file still exists: ($f)"
      $failed = true
    }
  }

  if $failed {
    exit 1
  } else {
    print "PASS: all adapters and scripts are nushell"
  }
}
