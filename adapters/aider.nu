#!/usr/bin/env nu
# aider.nu — dispatch a card prompt to Aider (cloud or local ollama)
#
# Offline / local mode: set OLLAMA_MODEL env var (e.g. "qwen3-coder:30b").
# The adapter will route to http://localhost:11434 with no network traffic.
# Online mode: no env override; aider uses its own configured cloud provider.

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

    # aider must be installed — exit 75 (transient) if missing so the dispatcher
    # can try another provider rather than permanently failing the card.
    if (which aider | is-empty) {
        "aider not found in PATH — install with: uv tool install aider-chat\n" | save --append $stderr_log
        exit 75
    }

    let orig_dir = (pwd)
    let prompt_abs = if ($prompt_file | str starts-with "/") { $prompt_file } else { [$orig_dir $prompt_file] | path join }
    let stdout_abs = if ($stdout_log | str starts-with "/") { $stdout_log } else { [$orig_dir $stdout_log] | path join }
    let stderr_abs = if ($stderr_log | str starts-with "/") { $stderr_log } else { [$orig_dir $stderr_log] | path join }

    cd $workdir

    let prompt_text = open --raw $prompt_abs

    # Offline mode: OLLAMA_MODEL is set → route to local ollama, no network needed.
    # Online mode: no override → aider uses its own cloud config.
    let ollama_model = if "OLLAMA_MODEL" in $env { $env.OLLAMA_MODEL } else { "" }
    let ollama_host  = if "OLLAMA_HOST" in $env { $env.OLLAMA_HOST } else { "http://localhost:11434" }

    if ($ollama_model | str length) > 0 {
        # Confirm ollama is reachable before committing to this provider.
        let health = (do { ^curl -sf $"($ollama_host)/api/tags" } | complete)
        if $health.exit_code != 0 {
            $"ollama not reachable at ($ollama_host) — exiting 75 (transient)\n" | save --append $stderr_abs
            exit 75
        }

        # aider's openai-compatible shim: point at ollama, no network traffic.
        # --auto-test + --test-cmd makes aider iterate on cargo check failures.
        # --map-tokens 0 disables repo-map (saves context window for the actual task).
        let args = [
            $prompt_text
            "--model" $"ollama/($ollama_model)"
            "--openai-api-base" $"($ollama_host)/v1"
            "--auto-test"
            "--test-cmd" "cargo check 2>&1"
            "--map-tokens" "0"
            "--yes"
            "--no-git"
        ]
        ^aider ...$args o> $stdout_abs e> $stderr_abs
    } else {
        ^aider $prompt_text --yes --no-git o> $stdout_abs e> $stderr_abs
    }
    let rc = $env.LAST_EXIT_CODE

    if ($stderr_abs | path exists) {
        let t = open --raw $stderr_abs
        if (($t | str contains --ignore-case "rate limit") or ($t | str contains "429") or ($t | str contains --ignore-case "too many requests")) {
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
    let stderr_text = "rate limit exceeded"
    let is_rate_limited = (($stderr_text | str contains --ignore-case "rate limit") or ($stderr_text | str contains "429"))
    assert $is_rate_limited "should detect rate limiting from stderr"

    # test 4: non-rate-limit error should not trigger
    let normal_err = "some other error occurred"
    let is_normal = not (($normal_err | str contains --ignore-case "rate limit") or ($normal_err | str contains "429") or ($normal_err | str contains --ignore-case "too many requests"))
    assert $is_normal "non-rate-limit errors should not trigger exit 75"

    # test 5: ollama model flag construction
    let model = "qwen3-coder:30b"
    let flag = $"ollama/($model)"
    assert ($flag == "ollama/qwen3-coder:30b") "ollama model flag should prefix with ollama/"

    # test 6: ollama host default
    let host = if "OLLAMA_HOST" in $env { $env.OLLAMA_HOST } else { "http://localhost:11434" }
    assert ($host | str starts-with "http") "ollama host must be an HTTP URL"

    # test 7: empty OLLAMA_MODEL string means cloud mode (no offline routing)
    let offline = ("" | str length) > 0
    assert (not $offline) "empty OLLAMA_MODEL should not trigger offline mode"

    print "PASS: aider.nu"
}
