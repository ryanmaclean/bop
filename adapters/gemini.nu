#!/usr/bin/env nu
# gemini.nu — dispatch a card prompt to Gemini CLI (gemini)
#
# Env vars:
#   GEMINI_MODEL  model passed to `gemini -m` (default: gemini-2.5-flash)

def is_rate_limited [text: string]: nothing -> bool {
    (($text | str contains --ignore-case "rate limit")
        or ($text | str contains --ignore-case "too many requests")
        or ($text | str contains --ignore-case "resource_exhausted")
        or ($text | str contains --ignore-case "quota exceeded")
        or ($text | str contains --ignore-case "quota has been exhausted")
        or ($text | str contains "429 Too")
        or ($text | str contains "status: 429")
        or ($text | str contains "\"code\":429"))
}

def main [
    workdir: string = "",
    prompt_file: string = "",
    stdout_log: string = "",
    stderr_log: string = "",
    _memory_out?: string,
    --test
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
    let prompt_abs = if ($prompt_file | str starts-with "/") { $prompt_file } else { [$orig_dir $prompt_file] | path join }
    let stdout_abs = if ($stdout_log | str starts-with "/") { $stdout_log } else { [$orig_dir $stdout_log] | path join }
    let stderr_abs = if ($stderr_log | str starts-with "/") { $stderr_log } else { [$orig_dir $stderr_log] | path join }

    cd $workdir

    let prompt = open --raw $prompt_abs
    let model = if "GEMINI_MODEL" in $env { $env.GEMINI_MODEL } else { "gemini-2.5-flash" }
    if (which gemini | is-empty) {
        "gemini CLI not found in PATH\n" | save --append $stderr_abs
        exit 127
    }

    # `complete` is used because $env.LAST_EXIT_CODE is not reliable when both
    # stdout and stderr are redirected.
    let res = (do { ^gemini -m $model -p $prompt } | complete)
    $res.stdout | save --force $stdout_abs
    $res.stderr | save --force $stderr_abs

    # Only a failed run can be a rate limit; a successful run that merely logs
    # a quota warning must not be retried.
    if $res.exit_code != 0 and (is_rate_limited $res.stderr) {
        exit 75
    }

    exit $res.exit_code
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

    # test 3: rate-limit detection — 429
    assert (is_rate_limited "Error: 429 Too Many Requests") "should detect 429"

    # test 4: rate-limit detection — quota exhausted
    assert (is_rate_limited "RESOURCE_EXHAUSTED: quota has been exhausted") "should detect quota exhaustion"

    # test 4b: a bare "429" inside a timestamp or port is not a rate limit
    assert (not (is_rate_limited "2026-11-16T10:04:29.429Z listening on :8429")) "bare 429 digits should not trigger"

    # test 5: non-rate-limit error should not trigger
    assert (not (is_rate_limited "syntax error in prompt")) "normal errors should not trigger rate-limit"

    print "PASS: gemini.nu"
}
