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
    workdir: string,
    prompt_file: string,
    stdout_log: string,
    stderr_log: string,
    _memory_out?: string,  # memory output file path; read via JOBCARD_MEMORY_OUT env if needed
] {
    # Resolve relative paths before cd
    let orig_dir = (pwd)
    let prompt_abs = if ($prompt_file | str starts-with "/") { $prompt_file } else { $"($orig_dir)/($prompt_file)" }
    let stdout_abs = if ($stdout_log | str starts-with "/") { $stdout_log } else { $"($orig_dir)/($stdout_log)" }
    let stderr_abs = if ($stderr_log | str starts-with "/") { $stderr_log } else { $"($orig_dir)/($stderr_log)" }

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
