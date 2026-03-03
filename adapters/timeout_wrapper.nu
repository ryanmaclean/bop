#!/usr/bin/env nu
# timeout_wrapper.nu — enforce a timeout on another command
#
# Usage: timeout_wrapper.nu <timeout_seconds> <command> [args...]
#
# On macOS, uses perl alarm (GNU timeout not available).
# On Linux, uses system timeout.

def main [timeout: int, ...args: string] {
    if $nu.os-info.name == "macos" {
        ^perl -e 'alarm(shift); exec @ARGV or die $!' -- $"($timeout)" ...$args
    } else {
        ^timeout $timeout ...$args
    }
}
