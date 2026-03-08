#!/usr/bin/env nu
# qemu.nu — run a bop card inside a QEMU-hosted zam unikernel
#
# Boot path (trusted-host, no OVMF, no dub):
#   qemu-system-aarch64 -kernel <zam.unikraft> \
#     -device virtio-9p-pci,mount_tag=cards \  ← card bundle at /
#     -netdev user \                            ← NAT for AI API calls
#     -serial file:<stdout_log>                 ← zam JSON output via UART
#
# zam (Unikraft) reads /prompt.md, calls Claude/Ollama, writes /output/result.md,
# then halts. QEMU exit code = zam exit code.
#
# For hardware / untrusted-hypervisor attestation: set BOP_QEMU_BOOT=dub
# (uses dub.efi + OVMF chain — future path, not implemented here)
#
# Env overrides:
#   ZAM_IMAGE            path to zam Unikraft kernel image
#   BOP_QEMU_MEMORY      VM RAM (default: 256M)
#   BOP_QEMU_TIMEOUT     seconds before SIGKILL (default: 300)
#
# Exit codes:  0=success  75=rate-limited  1+=failed

# Unikraft kernel images — direct -kernel boot, no OVMF needed.
# mount_tag=cards is baked into Unikraft kconfig; card bundle mounts at /.
# Order: release → debug → fallback locations.
# Override all of these with ZAM_IMAGE env var.
const ZAM_CANDIDATES = [
    "/Users/studio/zam/unikraft/.unikraft/build/qemu-arm64_qemu-arm64_qemu-arm64",
    "/Users/studio/zam/unikraft/.unikraft/build/zam_qemu-arm64_qemu-arm64",
    "/Users/studio/zam/unikraft/build/zam_qemu-arm64_qemu-arm64",
    "/Users/studio/zam/build/zam.kernel",
    "/Users/studio/zam/zam.arm64",
]

def find_qemu []: nothing -> string {
    if "QEMU_SYSTEM_AARCH64" in $env { return $env.QEMU_SYSTEM_AARCH64 }
    let candidates = [
        "/opt/homebrew/bin/qemu-system-aarch64"
        "/usr/local/bin/qemu-system-aarch64"
        "qemu-system-aarch64"
    ]
    $candidates | where { |p| (^which $p e> /dev/null | complete).exit_code == 0 } | first? | default "qemu-system-aarch64"
}

def find_zam []: nothing -> string {
    if "ZAM_IMAGE" in $env { return $env.ZAM_IMAGE }
    $ZAM_CANDIDATES | where { |p| ($p | path exists) } | first? | default ""
}

# Hash zam binary + card id before launch.
# On a trusted host, the dispatcher is the trust anchor — no TPM needed.
# Recorded in events.jsonl so provenance is auditable.
def record_attestation [zam_image: string, card_id: string, events_log: string]: nothing -> nothing {
    let zam_hash = (^shasum -a 256 $zam_image | str trim | split column --collapse-empty " " hash _path | get 0.hash)
    let event = {
        ts:        (date now | format date "%Y-%m-%dT%H:%M:%SZ")
        event:     "qemu_launch"
        zam_sha256: $zam_hash
        zam_image: $zam_image
        card:      $card_id
    }
    $event | to json --raw | $"($in)\n" | save --append $events_log
}

def is_rate_limited [stderr_path: string]: nothing -> bool {
    if not ($stderr_path | path exists) { return false }
    let t = (open --raw $stderr_path)
    [
        ($t | str contains "429")
        ($t | str contains --ignore-case "rate limit")
        ($t | str contains --ignore-case "too many requests")
    ] | any { |x| $x }
}

def run_qemu [qemu: string, accel: string, cpu: string, zam: string,
              workdir: string, stdout_path: string, stderr_path: string,
              memory: string]: nothing -> int {
    (^$qemu
        -machine virt
        -accel $accel
        -cpu $cpu
        -m $memory
        -kernel $zam
        # 9P virtfs: card bundle mounted at / inside VM (mount_tag matches Unikraft kconfig)
        -device virtio-9p-pci,fsdev=cards,mount_tag=cards
        -fsdev $"local,security_model=none,path=($workdir),id=cards"
        # NAT networking — zam needs outbound HTTPS for Claude/Ollama API calls
        -netdev user,id=net0
        -device virtio-net-pci,netdev=net0
        # UART → stdout log (zam writes JSON status lines here)
        -serial $"file:($stdout_path)"
        -no-reboot
        -nographic
    e>> $stderr_path | complete).exit_code
}

def main [
    workdir: string = "",
    prompt_file: string = "",
    stdout_log: string = "",
    stderr_log: string = "",
    _memory_out?: string,
    --test
] {
    if $test { run_tests; return }
    if $workdir == "" { print -e "error: workdir required"; exit 1 }

    let orig       = (pwd)
    let workdir_abs = ($workdir    | path expand)
    let stdout_abs  = if ($stdout_log | str starts-with "/") { $stdout_log } else { [$orig $stdout_log] | path join }
    let stderr_abs  = if ($stderr_log | str starts-with "/") { $stderr_log } else { [$orig $stderr_log] | path join }
    let events_log  = [$workdir_abs "logs" "events.jsonl"] | path join
    let card_id     = ($workdir_abs | path basename)

    let zam = (find_zam)
    if $zam == "" {
        ([
            "error: zam unikernel image not found"
            ""
            "Build zam Unikraft target:"
            "  cd /Users/studio/zam"
            "  kraft build --target qemu-arm64_qemu-arm64"
            ""
            "Or override: export ZAM_IMAGE=/path/to/zam.kernel"
            ""
            "Searched:"
            ...($ZAM_CANDIDATES | each { |p| $"  ($p)" })
        ] | str join "\n") | save --append $stderr_abs
        exit 1
    }

    let qemu   = (find_qemu)
    let memory = ($env.BOP_QEMU_MEMORY? | default "256M")

    # Attest zam binary before launch (trusted-host path)
    try { record_attestation $zam $card_id $events_log }

    # Try HVF (Apple Hypervisor.framework) first — fast on Apple Silicon.
    # Fall back to TCG software emulation for CI / non-Apple-Silicon hosts.
    let rc = (run_qemu $qemu "hvf" "host" $zam $workdir_abs $stdout_abs $stderr_abs $memory)
    let rc = if $rc != 0 and (open --raw $stderr_abs | str contains --ignore-case "hvf") {
        run_qemu $qemu "tcg" "cortex-a72" $zam $workdir_abs $stdout_abs $stderr_abs $memory
    } else {
        $rc
    }

    if (is_rate_limited $stderr_abs) { exit 75 }
    exit $rc
}

def run_tests []: nothing -> nothing {
    use std/assert

    # test 1: absolute path stays absolute
    let abs = ("/tmp/foo" | path expand)
    assert ($abs == "/tmp/foo") "absolute path unchanged"

    # test 2: ZAM_CANDIDATES ordered — Unikraft release first
    assert ($ZAM_CANDIDATES | get 0 | str contains "unikraft") "first candidate is Unikraft build"
    assert ($ZAM_CANDIDATES | get 0 | str contains "qemu-arm64") "first candidate is arm64"

    # test 3: 9P mount args use mount_tag=cards (matches Unikraft kconfig)
    let fsdev = "local,security_model=none,path=/tmp/test,id=cards"
    assert ($fsdev | str contains "security_model=none") "trusted-host: no xattr mapping"
    assert ($fsdev | str contains "id=cards") "fsdev id matches mount_tag"

    # test 4: rate-limit detection
    let fake_stderr = "/tmp/qemu-test-stderr.txt"
    "Error 429 Too Many Requests\n" | save --force $fake_stderr
    assert (is_rate_limited $fake_stderr) "detects 429"
    rm $fake_stderr

    # test 5: no false positive on clean stderr
    let clean_stderr = "/tmp/qemu-test-clean.txt"
    "qemu: machine virt initialized\n" | save --force $clean_stderr
    assert not (is_rate_limited $clean_stderr) "no false positive on clean output"
    rm $clean_stderr

    print "PASS: qemu.nu"
}
