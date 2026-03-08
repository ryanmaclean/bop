#!/usr/bin/env nu
# qemu.nu — segment-2 VM adapter (cloud-init shell runner)
#
# Usage:
#   qemu.nu <workdir> <prompt_file> <stdout_log> <stderr_log> [timeout_seconds]
#
# Exit codes:
#   0   success
#   75  timeout / transient failure (EX_TEMPFAIL)
#   127 missing dependency (qemu binary or base image)
#   1+ other failure

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
        # Dispatcher may pass memory-out path as arg5; default timeout is 300s.
        300
    }
}

def detect_arch []: nothing -> string {
    let host_arch = ($nu.os-info.arch | str downcase)
    if ($host_arch == "aarch64") or ($host_arch == "arm64") {
        "aarch64"
    } else {
        "x86_64"
    }
}

def machine_args [arch: string]: nothing -> list<string> {
    if $arch == "aarch64" {
        ["-machine" "virt,accel=hvf" "-cpu" "host"]
    } else if $nu.os-info.name == "macos" {
        ["-machine" "q35,accel=hvf" "-cpu" "host"]
    } else {
        ["-machine" "q35,accel=tcg"]
    }
}

def write_cloud_init [seed_dir: string]: nothing -> nothing {
    let user_data = [
        "#cloud-config"
        "runcmd:"
        "  - mkdir -p /card/output /card/logs"
        "  - sh /card/prompt.md > /card/output/result.md 2>&1"
        "  - echo $? > /card/logs/vm_exit_code"
        "  - poweroff -f"
    ] | str join "\n"

    let meta_data = [
        "instance-id: bop-card"
        "local-hostname: bop-card"
    ] | str join "\n"

    let user_data_path = [$seed_dir "user-data"] | path join
    let meta_data_path = [$seed_dir "meta-data"] | path join

    $user_data | save --force $user_data_path
    $meta_data | save --force $meta_data_path
}

def read_vm_exit_code [path: string]: nothing -> record<found: bool, code: int> {
    if not ($path | path exists) {
        return {found: false, code: 0}
    }

    let parsed = try {
        (open --raw $path | str trim | into int)
    } catch {
        null
    }

    if ($parsed | is-empty) {
        {found: false, code: 0}
    } else {
        {found: true, code: $parsed}
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
    let arch = (detect_arch)
    let qemu_bin = $"qemu-system-($arch)"

    if $test {
        print "qemu adapter: ok"
        if not (command_exists $qemu_bin) {
            print $"qemu adapter: note: ($qemu_bin) not found in PATH (test mode does not require qemu)"
        }
        return
    }

    if ($workdir | is-empty) or ($prompt_file | is-empty) or ($stdout_log | is-empty) or ($stderr_log | is-empty) {
        print -e "error: usage: adapters/qemu.nu <workdir> <prompt_file> <stdout_log> <stderr_log> [timeout_seconds]"
        exit 1
    }

    let orig_dir = (pwd)
    let workdir_abs = (resolve_abs $workdir $orig_dir | path expand)
    let prompt_abs = (resolve_abs $prompt_file $orig_dir | path expand)
    let stdout_abs = (resolve_abs $stdout_log $orig_dir | path expand)
    let stderr_abs = (resolve_abs $stderr_log $orig_dir | path expand)
    let timeout_seconds = (parse_timeout $timeout_or_memory)
    let vm_exit_code_path = [$workdir_abs "logs" "vm_exit_code"] | path join

    let stdout_dir = ($stdout_abs | path dirname)
    let stderr_dir = ($stderr_abs | path dirname)
    if not ($stdout_dir | path exists) { mkdir $stdout_dir }
    if not ($stderr_dir | path exists) { mkdir $stderr_dir }

    if not ($workdir_abs | path exists) {
        let msg = $"error: workdir does not exist: ($workdir_abs)"
        append_log_line $stderr_abs $msg
        print -e $msg
        exit 1
    }

    if not ($prompt_abs | path exists) {
        let msg = $"error: prompt file does not exist: ($prompt_abs)"
        append_log_line $stderr_abs $msg
        print -e $msg
        exit 1
    }

    if not (command_exists $qemu_bin) {
        let msg = [
            $"error: QEMU binary not found in PATH: ($qemu_bin)"
            "Install QEMU and ensure the matching qemu-system-* binary is available."
            "Run `nu adapters/qemu.nu --test` to verify adapter setup."
        ] | str join "\n"
        append_log_line $stderr_abs $msg
        print -e $msg
        exit 127
    }

    let base_image = ("~/.bop/qemu-base.qcow2" | path expand)
    if not ($base_image | path exists) {
        let msg = [
            $"error: missing QEMU base image: ($base_image)"
            "Build it once with: nu scripts/build-qemu-base.nu"
            "See docs/qemu-setup.md for details."
        ] | str join "\n"
        append_log_line $stderr_abs $msg
        print -e $msg
        exit 127
    }

    if ($vm_exit_code_path | path exists) {
        ^rm -f $vm_exit_code_path
    }

    let seed_dir = ((^mktemp -d /tmp/bop-qemu-seed.XXXXXX) | str trim)
    write_cloud_init $seed_dir

    let qemu_args = ((machine_args $arch) ++ [
        "-rtc" "base=localtime"
        "-m" "512M"
        "-drive" $"file=($base_image),if=virtio,format=qcow2"
        # Directory-backed FAT seed drive with cloud-init user-data/meta-data.
        "-drive" $"if=virtio,format=raw,file=fat:rw:($seed_dir)"
        "-virtfs" $"local,path=($workdir_abs),mount_tag=card,security_model=passthrough,id=card"
        "-serial" "stdio"
        "-no-reboot"
        "-nographic"
    ])

    let run = (^perl -e 'alarm(shift); exec @ARGV or die $!' -- ($timeout_seconds | into string) $qemu_bin ...$qemu_args | complete)
    $run.stdout | save --force $stdout_abs
    $run.stderr | save --force $stderr_abs
    let raw_rc = $run.exit_code
    let rc = if $raw_rc < 0 { 128 + (0 - $raw_rc) } else { $raw_rc }

    ^rm -rf $seed_dir

    if $rc == 142 {
        append_log_line $stderr_abs $"qemu adapter: timeout after ($timeout_seconds)s"
        exit 75
    }

    let vm_exit = (read_vm_exit_code $vm_exit_code_path)
    if $vm_exit.found {
        exit $vm_exit.code
    }

    let missing_msg = $"qemu adapter: vm exit code not found at ($vm_exit_code_path)"
    append_log_line $stderr_abs $missing_msg

    if $rc == 0 {
        # Guest powered off, but did not record exit code; treat as failure.
        exit 1
    }

    exit $rc
}
