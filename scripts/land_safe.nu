#!/usr/bin/env nu
# land_safe.nu — safe branch landing with git/jj cleanliness checks and gate enforcement

def main [
  --target: string = "main"    # Target branch to land onto
  --source: string = ""        # Source branch (default: current branch)
  --push: string = ""          # Remote to push target branch to after landing
  --skip-checks                # Skip gate checks (make check + bop policy check)
] {
  let root = ($env.FILE_PWD | path dirname)
  cd $root

  let target_branch = $target
  let source_branch = if ($source | is-empty) {
    ^git branch --show-current | str trim
  } else {
    $source
  }

  if ($source_branch | is-empty) or ($target_branch | is-empty) {
    print -e "source/target branch must be non-empty"
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

  if $source_branch == $target_branch {
    print -e $"Source and target are the same branch \(($source_branch)\); nothing to do."
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
    } else if ($"($root)/target/debug/bop" | path exists) {
      $"($root)/target/debug/bop"
    } else if ($"($root)/target/debug/jc" | path exists) {
      $"($root)/target/debug/jc"
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
