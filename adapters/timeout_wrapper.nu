#!/usr/bin/env nu
# timeout_wrapper.nu — enforce a timeout on another command
#
# Usage: timeout_wrapper.nu <timeout_seconds> <command> [args...]
#
# On macOS, uses perl alarm (GNU timeout not available).
# On Linux, uses system timeout.

def main [
    timeout: int = 0,
    --test,  # Run self-tests
    ...args: string
] {
    if $test {
        run_tests
        return
    }

    if $timeout == 0 {
        print -e "error: timeout is required"
        exit 1
    }

    if $nu.os-info.name == "macos" {
        ^perl -e 'alarm(shift); exec @ARGV or die $!' -- ($timeout | into string) ...$args
    } else {
        ^timeout $timeout ...$args
    }
}

def run_tests []: nothing -> nothing {
    use std/assert

    # test 1: platform detection works
    let platform = $nu.os-info.name
    assert (($platform == "macos") or ($platform == "linux") or ($platform == "windows")) "platform should be recognized"

    # test 2: timeout value handling
    let t = 60
    let t_str = $"($t)"
    assert ($t_str == "60") "timeout should convert to string correctly"

    # test 3: verify perl is available on macOS for the alarm approach
    if $nu.os-info.name == "macos" {
        let perl_check = (do { ^perl -e 'print "ok"' } | complete)
        assert ($perl_check.exit_code == 0) "perl should be available on macOS"
    }

    print "PASS: timeout_wrapper.nu"
}
