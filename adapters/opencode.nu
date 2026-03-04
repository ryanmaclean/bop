#!/usr/bin/env nu
# opencode.nu — dispatch a card prompt to OpenCode

def main [
    workdir: string = "",
    prompt_file: string = "",
    stdout_log: string = "",
    stderr_log: string = "",
    _memory_out?: string,  # memory output file; read via JOBCARD_MEMORY_OUT env
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

    let orig_dir = (pwd)
    let prompt_abs = if ($prompt_file | str starts-with "/") { $prompt_file } else { $"($orig_dir)/($prompt_file)" }
    let stdout_abs = if ($stdout_log | str starts-with "/") { $stdout_log } else { $"($orig_dir)/($stdout_log)" }
    let stderr_abs = if ($stderr_log | str starts-with "/") { $stderr_log } else { $"($orig_dir)/($stderr_log)" }

    cd $workdir

    let prompt_text = open --raw $prompt_abs

    ^opencode run $prompt_text o> $stdout_abs e> $stderr_abs
    let rc = $env.LAST_EXIT_CODE

    if ($stderr_abs | path exists) {
        let t = open --raw $stderr_abs
        if (($t | str contains --ignore-case "rate limit") or ($t | str contains "429") or ($t | str contains --ignore-case "too many requests")) {
            exit 75
        }
    }

    exit $rc
}

def run_tests [] {
    use std/assert

    # test 1: path resolution — absolute stays absolute
    let abs = if ("/tmp/foo" | str starts-with "/") { "/tmp/foo" } else { $"(pwd)/foo" }
    assert ($abs == "/tmp/foo") "absolute path should stay absolute"

    # test 2: path resolution — relative gets resolved
    let rel = if ("foo" | str starts-with "/") { "foo" } else { $"(pwd)/foo" }
    assert ($rel | str ends-with "/foo") "relative path should be resolved"
    assert ($rel | str starts-with "/") "resolved path should be absolute"

    # test 3: rate-limit detection
    let stderr_text = "too many requests - please wait"
    let is_rate_limited = (($stderr_text | str contains --ignore-case "rate limit") or ($stderr_text | str contains "429") or ($stderr_text | str contains --ignore-case "too many requests"))
    assert $is_rate_limited "should detect rate limiting from stderr"

    # test 4: non-rate-limit error should not trigger
    let normal_err = "command not found"
    let is_normal = not (($normal_err | str contains --ignore-case "rate limit") or ($normal_err | str contains "429") or ($normal_err | str contains --ignore-case "too many requests"))
    assert $is_normal "non-rate-limit errors should not trigger exit 75"

    print "PASS: opencode.nu"
}
