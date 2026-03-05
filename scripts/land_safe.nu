#!/usr/bin/env nu
# land_safe.nu — safe branch landing with git/jj cleanliness checks and gate enforcement

# Validate that source and target branch names are non-empty and distinct.
# Returns null on success, or an error string on failure.
def validate_branches [source: string, target: string]: nothing -> string {
  if ($source | is-empty) or ($target | is-empty) {
    return "source/target branch must be non-empty"
  }
  if $source == $target {
    return $"Source and target are the same branch \(($source)\); nothing to do."
  }
  ""
}

def run_tests [] {
  use std/assert

  # Test valid branches
  assert equal (validate_branches "feature" "main") ""

  # Test empty source
  assert equal (validate_branches "" "main") "source/target branch must be non-empty"

  # Test empty target
  assert equal (validate_branches "feature" "") "source/target branch must be non-empty"

  # Test both empty
  assert equal (validate_branches "" "") "source/target branch must be non-empty"

  # Test same branch
  let result = (validate_branches "main" "main")
  assert ($result | str contains "same branch") "should reject same branch"

  # Test different valid branches
  assert equal (validate_branches "fix/bug-123" "develop") ""

  print "PASS: land_safe.nu"
}

def main [
  --target: string = "main"    # Target branch to land onto
  --source: string = ""        # Source branch (default: current branch)
  --push: string = ""          # Remote to push target branch to after landing
  --skip-checks                # Skip gate checks (make check + bop policy check)
  --test                       # Run internal self-tests
] {
  if $test {
    run_tests
    return
  }
  let root = ($env.FILE_PWD | path dirname)
  cd $root

  let target_branch = $target
  let source_branch = if ($source | is-empty) {
    ^git branch --show-current | str trim
  } else {
    $source
  }

  let validation_error = (validate_branches $source_branch $target_branch)
  if ($validation_error | is-not-empty) {
    print -e $validation_error
    exit 2
  }

  let has_jj = (which jj | length) > 0

  # Check git working tree is clean
  let porcelain = (^git status --porcelain | str trim)
  if ($porcelain | is-not-empty) {
    print -e "Refusing to land: git working tree is dirty."
    ^git status --short e+o> /dev/null
    print -e (^git status --short)
    exit 1
  }

  # Check jj working copy is clean (if jj is available)
  if $has_jj {
    let jj_ok = do { ^jj status --no-pager } | complete
    if $jj_ok.exit_code == 0 {
      let jj_status = (do { ^jj status --no-pager } | complete).stdout
      if not ($jj_status | str contains "The working copy is clean") {
        print -e "Refusing to land: jj working copy is not clean."
        print -e $jj_status
        exit 1
      }
    } else {
      print -e "Note: jj is installed but this repo is not initialized for jj; using git-only safety checks."
    }
  }

  # Verify branches exist
  let source_ref = (do { ^git show-ref --verify --quiet $"refs/heads/($source_branch)" } | complete)
  if $source_ref.exit_code != 0 {
    print -e $"Unknown source branch: ($source_branch)"
    exit 1
  }
  let target_ref = (do { ^git show-ref --verify --quiet $"refs/heads/($target_branch)" } | complete)
  if $target_ref.exit_code != 0 {
    print -e $"Unknown target branch: ($target_branch)"
    exit 1
  }

  # Check target is not checked out in a worktree
  let worktree_output = (^git worktree list --porcelain)
  let checked_out = ($worktree_output | str contains $"branch refs/heads/($target_branch)")
  if $checked_out {
    print -e $"Refusing to move ($target_branch): it is currently checked out in a worktree."
    exit 1
  }

  # Run gate checks
  if not $skip_checks {
    let bop_bin = if ("BOP_BIN" in $env) {
      $env.BOP_BIN
    } else if (($root | path join "target" "debug" "bop") | path exists) {
      ($root | path join "target" "debug" "bop")
    } else if (($root | path join "target" "debug" "jc") | path exists) {
      ($root | path join "target" "debug" "jc")
    } else {
      print -e "Missing bop/jc binary. Run: cargo build"
      exit 1
    }

    print "Running gates..."
    ^make check
    ^$bop_bin policy check --staged
  }

  # Ensure fast-forward is possible
  let ff_check = (do { ^git merge-base --is-ancestor $target_branch $source_branch } | complete)
  if $ff_check.exit_code != 0 {
    print -e $"Refusing non-FF landing: ($target_branch) is not an ancestor of ($source_branch)."
    exit 1
  }

  ^git branch -f $target_branch $source_branch
  print $"Fast-forwarded ($target_branch) -> ($source_branch)"

  if ($push | is-not-empty) {
    ^git push $push $"($target_branch):($target_branch)"
    print $"Pushed ($target_branch) to ($push)"
  }

  print "Done."
}
