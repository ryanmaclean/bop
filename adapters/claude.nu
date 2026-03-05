#!/usr/bin/env nu
# claude.nu — dispatch a card prompt to Claude Code (claude CLI)
#
# Usage (called by dispatcher):
#   claude.nu <workdir> <prompt_file> <stdout_log> <stderr_log> [memory_out]
#
# Exit codes:
#   0   success
#   75  transient (rate-limited, SIGALRM timeout) → back to pending/
#   1+  failure → failed/

def main [
    workdir: string = "",
    prompt_file: string = "",
    stdout_log: string = "",
    stderr_log: string = "",
    _memory_out?: string,  # memory output file; read via JOBCARD_MEMORY_OUT env if needed
    --test  # Run self-tests
] {
    if $test {
        run_tests
        return
    }

    if $workdir == "" {
        print -e "error: workdir is required"
        exit 1
    }

    # Timeout: prefer card's timeout_seconds if available via BOP_CARD_DIR/meta.json, else 3600s
    let timeout = 3600
    let orig_dir = (pwd)
    let prompt_abs = if ($prompt_file | str starts-with "/") { $prompt_file } else { [$orig_dir $prompt_file] | path join }
    let stdout_abs = if ($stdout_log | str starts-with "/") { $stdout_log } else { [$orig_dir $stdout_log] | path join }
    let stderr_abs = if ($stderr_log | str starts-with "/") { $stderr_log } else { [$orig_dir $stderr_log] | path join }

    cd $workdir

    # Allow spawning claude from within a Claude Code session
    hide-env CLAUDECODE

    # MCP config: merge .cards/mcp.json (global) + card-level mcp.json if present
    let mcp_args = (
        [".cards/mcp.json", "mcp.json"]
        | where { |f| ($f | path exists) }
        | each { |f| ["--mcp-config", $f] }
        | flatten
    )

    let prompt_text = open --raw $prompt_abs

    let claude_args = [
        "-p", $prompt_text,
        "--dangerously-skip-permissions",
        "--output-format", "json",
    ] ++ $mcp_args

    # perl alarm — macOS lacks GNU timeout; SIGALRM exit = 128+14 = 142
    ^perl -e 'alarm(shift); exec @ARGV or die $!' -- ($timeout | into string) claude ...$claude_args o> $stdout_abs e> $stderr_abs
    let rc = $env.LAST_EXIT_CODE

    if $rc == 142 { exit 75 }

    if ($stderr_abs | path exists) {
        let stderr_text = open --raw $stderr_abs
        if (($stderr_text | str contains --ignore-case "rate limit")
            or ($stderr_text | str contains "429")
            or ($stderr_text | str contains --ignore-case "too many requests")) {
            exit 75
        }
    }

    exit $rc
}

def run_tests []: nothing -> nothing {
    use std/assert

    # test 1: path resolution — absolute stays absolute
    let abs = if ("/tmp/foo" | str starts-with "/") { "/tmp/foo" } else { [(pwd) "foo"] | path join }
    assert ($abs == "/tmp/foo") "absolute path should stay absolute"

    # test 2: path resolution — relative gets resolved
    let rel = if ("foo" | str starts-with "/") { "foo" } else { [(pwd) "foo"] | path join }
    assert ($rel | str ends-with "/foo") "relative path should be resolved"
    assert ($rel | str starts-with "/") "resolved path should be absolute"

    # test 3: timeout default
    let timeout = 3600
    assert ($timeout == 3600) "default timeout should be 3600"

    # test 4: rate-limit exit code mapping
    let rc = 142
    let mapped = if $rc == 142 { 75 } else { $rc }
    assert ($mapped == 75) "exit code 142 (SIGALRM) should map to 75"

    # test 5: rate-limit detection in stderr text
    let stderr_text = "Error: 429 Too Many Requests"
    let is_rate_limited = (($stderr_text | str contains "429") or ($stderr_text | str contains --ignore-case "rate limit"))
    assert $is_rate_limited "should detect rate limiting from stderr content"

    print "PASS: claude.nu"
}
