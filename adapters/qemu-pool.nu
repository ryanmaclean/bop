#!/usr/bin/env nu
# qemu-pool.nu — pooled VM adapter
#
# Usage:
#   qemu-pool.nu <workdir> <prompt_file> <stdout_log> <stderr_log> [timeout_seconds]
#
# Behavior:
#   1) Lease an idle slot from `bop factory pool`
#   2) Run existing qemu adapter for the card workload
#   3) Release/reset the slot back to pool

def command_exists [name: string]: nothing -> bool {
    ((^which $name | complete).exit_code == 0)
}

def resolve_abs [p: string, base: string]: nothing -> string {
    if ($p | str starts-with "/") {
        $p
    } else {
        [$base $p] | path join
    }
}

def append_log_line [path: string, line: string]: nothing -> nothing {
    $"($line)\n" | save --append $path
}

def parse_timeout [arg?: string]: nothing -> int {
    if ($arg | is-empty) {
        return 300
    }

    try {
        $arg | into int
    } catch {
        # Dispatcher arg5 can be memory-out path; default timeout is 300s.
        300
    }
}

def normalize_exit [code: int]: nothing -> int {
    if $code < 0 {
        128 + (0 - $code)
    } else {
        $code
    }
}

def card_id_from_workdir [workdir_abs: string]: nothing -> string {
    let env_card = ($env.BOP_CARD_ID? | default "")
    if not ($env_card | is-empty) {
        return $env_card
    }

    let name = ($workdir_abs | path basename)
    if ($name | str ends-with ".bop") {
        $name | str replace ".bop" ""
    } else {
        $name
    }
}

def pool_lease [cards_dir: string, card_id: string, timeout_s: int, stderr_log: string]: nothing -> record<ok: bool, slot: int> {
    let lease = (^bop --cards-dir $cards_dir factory pool lease --card-id $card_id --timeout-s ($timeout_s | into string) | complete)
    if $lease.exit_code != 0 {
        let msg = if not ($lease.stderr | is-empty) {
            $"pool lease failed: ($lease.stderr | str trim)"
        } else {
            "pool lease failed"
        }
        append_log_line $stderr_log $msg
        return {ok: false, slot: -1}
    }

    let slot = try {
        let parsed = ($lease.stdout | from json)
        ($parsed | get slot)
    } catch {
        null
    }

    if ($slot | is-empty) {
        append_log_line $stderr_log "pool lease returned invalid JSON payload"
        return {ok: false, slot: -1}
    }

    {ok: true, slot: $slot}
}

def pool_inject [_slot: int, _workdir: string, _prompt_file: string]: nothing -> nothing {
    # Current implementation delegates execution to adapters/qemu.nu,
    # which mounts workdir directly for each run.
}

def pool_wait [workdir: string, prompt_file: string, stdout_log: string, stderr_log: string, timeout_s: int]: nothing -> int {
    let run = (^nu adapters/qemu.nu $workdir $prompt_file $stdout_log $stderr_log ($timeout_s | into string) | complete)
    normalize_exit $run.exit_code
}

def pool_release [cards_dir: string, slot: int, card_id: string, exit_code: int, stderr_log: string]: nothing -> nothing {
    let rel = (^bop --cards-dir $cards_dir factory pool release --slot ($slot | into string) --card-id $card_id --exit-code ($exit_code | into string) | complete)
    if $rel.exit_code != 0 {
        let msg = if not ($rel.stderr | is-empty) {
            $"pool release failed for slot ($slot): ($rel.stderr | str trim)"
        } else {
            $"pool release failed for slot ($slot)"
        }
        append_log_line $stderr_log $msg
    }
}

def main [
    workdir: string = "",
    prompt_file: string = "",
    stdout_log: string = "",
    stderr_log: string = "",
    timeout_or_memory?: string,
    --test
] {
    if $test {
        print "qemu-pool adapter: ok"
        if not (command_exists "bop") {
            print "qemu-pool adapter: note: bop not found in PATH"
        }
        if not (command_exists "nu") {
            print "qemu-pool adapter: note: nu not found in PATH"
        }
        return
    }

    if ($workdir | is-empty) or ($prompt_file | is-empty) or ($stdout_log | is-empty) or ($stderr_log | is-empty) {
        print -e "error: usage: adapters/qemu-pool.nu <workdir> <prompt_file> <stdout_log> <stderr_log> [timeout_seconds]"
        exit 1
    }

    if not (command_exists "bop") {
        print -e "error: bop CLI not found in PATH"
        exit 127
    }

    let orig_dir = (pwd)
    let workdir_abs = (resolve_abs $workdir $orig_dir | path expand)
    let prompt_abs = (resolve_abs $prompt_file $orig_dir | path expand)
    let stdout_abs = (resolve_abs $stdout_log $orig_dir | path expand)
    let stderr_abs = (resolve_abs $stderr_log $orig_dir | path expand)
    let timeout_s = (parse_timeout $timeout_or_memory)

    let stdout_dir = ($stdout_abs | path dirname)
    let stderr_dir = ($stderr_abs | path dirname)
    if not ($stdout_dir | path exists) { mkdir $stdout_dir }
    if not ($stderr_dir | path exists) { mkdir $stderr_dir }

    let cards_dir = ($env.BOP_CARDS_DIR? | default ".cards")
    let card_id = (card_id_from_workdir $workdir_abs)

    let lease = (pool_lease $cards_dir $card_id $timeout_s $stderr_abs)
    if not $lease.ok {
        exit 75
    }

    let slot = $lease.slot
    pool_inject $slot $workdir_abs $prompt_abs

    let rc = (pool_wait $workdir_abs $prompt_abs $stdout_abs $stderr_abs $timeout_s)
    pool_release $cards_dir $slot $card_id $rc $stderr_abs

    if $rc == 142 {
        exit 75
    }

    exit $rc
}
