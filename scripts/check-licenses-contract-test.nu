#!/usr/bin/env nu
# SPDX-License-Identifier: MIT
use std/assert
const TARGET = path self ./check_licenses.nu
const CHILD = path self ./check-licenses-child.nu

def main [] {
  let scratch = (mktemp -d | str trim)
  assert (($scratch | path type) == 'dir')
  let bin = ($scratch | path join bin)
  mkdir $bin
  # Only private test children; Cargo and cargo-deny never execute.
  for name in [cargo cargo-deny] {
    cp $CHILD ($bin | path join $name)
    let mode = (^/bin/chmod 0700 ($bin | path join $name) | complete)
    assert equal $mode.exit_code 0
  }
  let linked = (^/bin/ln -s $nu.current-exe ($bin | path join nu) | complete)
  assert equal $linked.exit_code 0
  mut cases = []
  for row in [
    {name: clean, code: 0, out: '', err: ''}
    {name: success-output, code: 0, out: "audit output\n", err: "warning\n"}
    {name: policy-failure, code: 7, out: "partial\n", err: "denied\n"}
    {name: tool-failure, code: 23, out: '', err: 'tool error'}
  ] {
    let log = ($scratch | path join $'($row.name).argv.json')
    let result = (with-env {PATH: [$bin], CHECK_LICENSES_ARGV: $log, CHECK_LICENSES_EXIT: ($row.code | into string), CHECK_LICENSES_STDOUT: $row.out, CHECK_LICENSES_STDERR: $row.err} {
      ^$nu.current-exe --no-config-file $TARGET | complete
    })
    $result | to json | save ($scratch | path join $'($row.name).result.json')
    assert equal (open $log) [deny check licenses]
    assert equal $result.exit_code $row.code
    assert equal $result.stderr $row.err
    let expected = if $row.code == 0 { $row.out + "License audit passed.\n" } else { $row.out }
    assert equal $result.stdout $expected
    $cases = ($cases | append $row.name)
  }
  rm ($bin | path join cargo-deny)
  let missing_log = ($scratch | path join missing-tool.argv.json)
  assert (not ($missing_log | path exists))
  let absent = (with-env {PATH: [$bin], CHECK_LICENSES_ARGV: $missing_log, CHECK_LICENSES_EXIT: '0', CHECK_LICENSES_STDOUT: 'unexpected child', CHECK_LICENSES_STDERR: ''} { ^$nu.current-exe --no-config-file $TARGET | complete })
  $absent | to json | save ($scratch | path join missing-tool.result.json)
  assert equal $absent.exit_code 1
  assert equal $absent.stdout "cargo-deny not installed. Install with: cargo install cargo-deny --locked\n"
  assert equal $absent.stderr ''
  assert (not ($missing_log | path exists))
  $cases = ($cases | append missing-tool)
  {cases:$cases, count:5, scratch:$scratch, wrapper_only:true, dependency_audit_executed:false} | to json
}
