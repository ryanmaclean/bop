#!/usr/bin/env nu
# mock.nu — test adapter that echoes the prompt without calling any AI model
#
# Usage (called by dispatcher):
#   mock.nu <workdir> <prompt_file> <stdout_log> <stderr_log> [timeout]
#
# Exit codes:
#   MOCK_EXIT env var (default 0)
# Env vars:
#   MOCK_SLEEP          sleep N seconds before responding
#   MOCK_STDOUT_TEXT    extra text to append to stdout_log
#   MOCK_STDERR_TEXT    extra text to append to stderr_log
#   MOCK_EXIT           exit code to use (default 0)

def main [
    workdir: string = "",
    prompt_file: string = "",
    stdout_log: string = "",
    stderr_log: string = "",
    _memory_out?: string,  # memory output file path; read via JOBCARD_MEMORY_OUT env if needed
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

    # Resolve relative paths before cd
    let orig_dir = (pwd)
    let prompt_abs = if ($prompt_file | str starts-with "/") { $prompt_file } else { [$orig_dir $prompt_file] | path join }
    let stdout_abs = if ($stdout_log | str starts-with "/") { $stdout_log } else { [$orig_dir $stdout_log] | path join }
    let stderr_abs = if ($stderr_log | str starts-with "/") { $stderr_log } else { [$orig_dir $stderr_log] | path join }

    cd $workdir

    if "MOCK_SLEEP" in $env {
        ^sleep ($env.MOCK_SLEEP | into int)
    }

    let prompt_text = if ($prompt_abs | path exists) { open --raw $prompt_abs } else { "" }

    [
        "mock adapter"
        $"workdir=($workdir)"
        $"prompt_file=($prompt_abs)"
        "--- prompt ---"
        $prompt_text
        "-------------"
    ] | str join "\n" | save --append $stdout_abs

    "mock stderr\n" | save --append $stderr_abs

    if "MOCK_STDERR_TEXT" in $env {
        $"($env.MOCK_STDERR_TEXT)\n" | save --append $stderr_abs
    }

    if "MOCK_STDOUT_TEXT" in $env {
        $"($env.MOCK_STDOUT_TEXT)\n" | save --append $stdout_abs
    }

    exit (if "MOCK_EXIT" in $env { $env.MOCK_EXIT | into int } else { 0 })
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

    # test 3: full integration — simulate mock adapter logic with temp files
    let tmpdir = (^mktemp -d /tmp/bop-mock-test.XXXXXX)
    let prompt_file = [$tmpdir "prompt.md"] | path join
    let stdout_log = [$tmpdir "stdout.log"] | path join
    let stderr_log = [$tmpdir "stderr.log"] | path join

    "test prompt content" | save $prompt_file
    "" | save $stdout_log
    "" | save $stderr_log

    # Replicate the core mock adapter logic (cannot call main — it exits the process)
    let prompt_text = open --raw $prompt_file
    [
        "mock adapter"
        $"workdir=($tmpdir)"
        $"prompt_file=($prompt_file)"
        "--- prompt ---"
        $prompt_text
        "-------------"
    ] | str join "\n" | save --append $stdout_log
    "mock stderr\n" | save --append $stderr_log

    let stdout_content = open --raw $stdout_log
    let stderr_content = open --raw $stderr_log
    assert ($stdout_content | str contains "mock adapter") "stdout should contain 'mock adapter'"
    assert ($stderr_content | str contains "mock stderr") "stderr should contain 'mock stderr'"
    assert ($stdout_content | str contains "test prompt content") "stdout should contain the prompt text"

    # Cleanup
    ^rm -rf $tmpdir

    print "PASS: mock.nu"
}
