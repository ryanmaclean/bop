#!/usr/bin/env nu
# codex.nu — dispatch a card prompt to OpenAI Codex CLI

const NETWORK_ERROR_PATTERNS = [
    "connection refused",
    "network unreachable",
    "connection reset",
    "connection timed out",
    "no route to host",
    "host is unreachable",
    "could not resolve host",
    "name resolution failed",
    "failed to connect",
    "network is down",
    "socket error",
    "ssl connection",
    "tls handshake",
    "temporary failure in name resolution"
]

def is_network_error [stderr_text: string]: nothing -> bool {
    $NETWORK_ERROR_PATTERNS | any { |pattern|
        $stderr_text | str contains --ignore-case $pattern
    }
}

def effort_for_priority [p]: nothing -> string {
    match $p {
        1 => { "xhigh" }
        2 => { "high" }
        3 => { "medium" }
        _ => {
            if (($p | describe) == "int") {
                if ($p >= 4 and $p <= 10) {
                    "low"
                } else {
                    "high"
                }
            } else {
                "high"
            }
        }
    }
}

def main [
    workdir: string = "",
    prompt_file: string = "",
    stdout_log: string = "",
    stderr_log: string = "",
    _memory_out?: string,  # memory output file; read via BOP_MEMORY_OUT env
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

    let meta_path = [$workdir "meta.json"] | path join
    let priority = if ($meta_path | path exists) {
        (open $meta_path | get -o priority | default null)
    } else { null }
    let effort = effort_for_priority $priority

    let orig_dir = (pwd)
    let prompt_abs = if ($prompt_file | str starts-with "/") { $prompt_file } else { [$orig_dir $prompt_file] | path join }
    let stdout_abs = if ($stdout_log | str starts-with "/") { $stdout_log } else { [$orig_dir $stdout_log] | path join }
    let stderr_abs = if ($stderr_log | str starts-with "/") { $stderr_log } else { [$orig_dir $stderr_log] | path join }
    let project_root = $workdir

    cd $workdir

    # Use a shell wrapper to reliably capture exit code — Nushell does not set
    # $env.LAST_EXIT_CODE when both o> and e> redirections are active.
    # Pass prompt via stdin (codex exec -) to avoid shell-escaping issues.
    let rc_file = (mktemp)
    ^sh -c $"AC_PROJECT_DIR='($project_root)' codex exec --full-auto -c model_reasoning_effort='($effort)' -c 'mcp_servers.auto-codex.env.AC_PROJECT_DIR=\"($project_root)\"' - < '($prompt_abs)' > '($stdout_abs)' 2> '($stderr_abs)'; printf '%d' $? > '($rc_file)'"
    let rc = (open --raw $rc_file | into int)

    if ($stderr_abs | path exists) {
        let t = open --raw $stderr_abs

        # Check network errors first (using NETWORK_ERROR_PATTERNS)
        if (is_network_error $t) {
            exit 75
        }

        # Then check rate limiting
        # "429" alone matches OTel timestamps (e.g. 21:50:52.429Z); require "429 " context
        if (($t | str contains "429 Too Many") or ($t | str contains --ignore-case "too many requests")) {
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

    # test 3: rate-limit detection
    let stderr_text = "Too Many Requests"
    let is_rate_limited = (($stderr_text | str contains "429 Too Many") or ($stderr_text | str contains --ignore-case "too many requests"))
    assert $is_rate_limited "should detect rate limiting from stderr"

    # test 4: OTel timestamp with .429 should NOT trigger false positive
    let otel_line = "2026-03-05T21:50:52.429348Z  INFO codex_otel::traces"
    let is_false_positive = (($otel_line | str contains "429 Too Many") or ($otel_line | str contains --ignore-case "too many requests"))
    assert (not $is_false_positive) "OTel timestamp .429 should not trigger rate-limit detection"

    # test 5: non-rate-limit error should not trigger
    let normal_err = "compilation failed"
    let is_normal = not (($normal_err | str contains "429 Too Many") or ($normal_err | str contains --ignore-case "too many requests"))
    assert $is_normal "non-rate-limit errors should not trigger exit 75"

    # test 6: network error detection — connection refused
    let net_err1 = "Error: Connection refused by server"
    assert (is_network_error $net_err1) "should detect 'connection refused' as network error"

    # test 7: network error detection — DNS failure
    let net_err2 = "Error: Could not resolve host: api.example.com"
    assert (is_network_error $net_err2) "should detect DNS resolution failure as network error"

    # test 8: network error detection — no error
    let no_err = "Success: Request completed"
    assert (not (is_network_error $no_err)) "should not detect success message as network error"

    # test 9: network error detection — case insensitive
    let net_err3 = "ERROR: NETWORK UNREACHABLE"
    assert (is_network_error $net_err3) "should detect network errors case-insensitively"

    # test 10: effort mapping — P1
    assert ((effort_for_priority 1) == "xhigh") "priority 1 should map to xhigh"

    # test 11: effort mapping — P2
    assert ((effort_for_priority 2) == "high") "priority 2 should map to high"

    # test 12: effort mapping — P3
    assert ((effort_for_priority 3) == "medium") "priority 3 should map to medium"

    # test 13: effort mapping — low band lower bound
    assert ((effort_for_priority 4) == "low") "priority 4 should map to low"

    # test 14: effort mapping — low band upper bound
    assert ((effort_for_priority 10) == "low") "priority 10 should map to low"

    # test 15: effort mapping — null defaults safely to high
    assert ((effort_for_priority null) == "high") "null priority should default to high"

    # test 16: effort mapping — out-of-range defaults to high
    assert ((effort_for_priority (-1)) == "high") "out-of-range priority should default to high"

    print "PASS: codex.nu"
}
