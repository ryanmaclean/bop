#!/usr/bin/env nu
# opencode.nu — dispatch a card prompt to OpenCode

def main [
    workdir: string,
    prompt_file: string,
    stdout_log: string,
    stderr_log: string,
    _memory_out?: string,  # memory output file; read via JOBCARD_MEMORY_OUT env
] {
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
