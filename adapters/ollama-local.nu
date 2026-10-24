#!/usr/bin/env nu
# ollama-local.nu — compatibility shim forwarding to ollama.nu
#
# Keep this file so existing provider configs and dispatcher mappings that
# reference adapters/ollama-local.nu continue to work.

def main [
    workdir: string = "",
    prompt_file: string = "",
    stdout_log: string = "",
    stderr_log: string = "",
    memory_out?: string,
    --test
] {
    let script_dir = if "FILE_PWD" in $env { $env.FILE_PWD } else { (pwd) }
    let target = [$script_dir "ollama.nu"] | path join

    if $test {
        ^nu $target --test
        exit $env.LAST_EXIT_CODE
    }

    if ($memory_out | is-empty) {
        ^nu $target $workdir $prompt_file $stdout_log $stderr_log
    } else {
        ^nu $target $workdir $prompt_file $stdout_log $stderr_log $memory_out
    }

    exit $env.LAST_EXIT_CODE
}
