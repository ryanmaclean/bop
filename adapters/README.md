# bop Adapters

Adapters are the execution layer between the dispatcher and AI coding agents. Each adapter is a ~150 LOC Nushell script that follows a standard calling convention.

## Architecture Overview

```
dispatcher
  ↓ spawn
adapter.nu <workdir> <prompt_file> <stdout_log> <stderr_log> [memory_out]
  ↓ invoke
AI tool (claude, ollama, codex, etc.)
  ↓ writes
workdir/output/result.md
```

**Current state:** Adapters run on the host OS (macOS/Linux), invoking tools like `claude`, `ollama`, or `aider` directly.

**Target state:** Every card runs in an isolated QEMU VM via `adapters/qemu.nu`.

## VM-First Architecture (Target)

The `qemu.nu` adapter is the end-state execution model. Instead of running agents on the host, each card runs in an isolated VM:

```
adapters/qemu.nu <workdir> <prompt_file> <stdout_log> <stderr_log>
  ↓
qemu-system-x86_64 \
  -kernel /Users/studio/zam/target/zam.elf \       # zam unikernel
  -drive format=raw,file=<dub-efi-image> \         # dub UEFI (future)
  -virtfs local,path=<workdir>,mount_tag=card \    # 9P: card bundle
  -serial file:<stdout_log> \                      # UART → structured output
  -m 128M -nographic -no-reboot
  ↓
zam unikernel boots in VM
  → mounts 9P filesystem (card bundle at /mnt/card)
  → reads /mnt/card/prompt.md
  → invokes tool (claude/ollama/codex) based on meta.json
  → writes /mnt/card/output/result.md via 9P
  → outputs JSON to UART (structured logs/metrics)
  → VM exits: 0=success, 75=rate-limit, other=failure
```

### Why VMs?

- **Isolation:** Each card runs in a clean environment; no host-side pollution
- **Reproducibility:** Same image = same execution environment
- **Security:** Sandboxed execution with measured boot (TPM PCR attestation via dub)
- **MICROCLAW:** Card bundle IS the IPC (9P mount), UART IS stdout, no extra servers

### 9P Filesystem

The card bundle is mounted into the VM using QEMU's `virtfs` with the 9P protocol:

```bash
-virtfs local,path=<workdir>,mount_tag=card,security_model=mapped-xattr,readonly=off
```

Inside the VM, zam mounts this as `/mnt/card`:
```
/mnt/card/
├── meta.json         # card metadata (provider, timeout, etc.)
├── prompt.md         # agent instructions
├── spec.md           # what to build
├── output/
│   └── result.md     # agent writes output here (via 9P)
└── workspace/        # git worktree (if applicable)
```

The 9P mount is **read-write**: zam can write `output/result.md` and other artifacts back to the host filesystem atomically.

### Building zam and dub

**zam** (`/Users/studio/zam`): Minimal (~5MB) unikernel that reads bop cards via 9P.
```bash
cd /Users/studio/zam
cargo build --release
# Produces: target/release/zam (or target/zam.elf)
```

**dub** (`/Users/studio/efi`): UEFI bootloader with TPM PCR attestation and A/B rollback.
```bash
cd /Users/studio/efi
cargo build --release --target x86_64-unknown-uefi
# Produces: target/x86_64-unknown-uefi/release/dub.efi
```

**Note:** `qemu.nu` currently boots zam directly via `-kernel`. Full UEFI boot (dub → zam) is a future enhancement.

### Exit Code Contract

All adapters (including `qemu.nu`) follow this contract:

| Exit Code | Meaning | Dispatcher Action |
|-----------|---------|-------------------|
| `0` | Success | Move card to `done/` |
| `75` | Transient failure (rate-limit, timeout) | Move card back to `pending/` (retry) |
| `1+` (not 75) | Permanent failure | Move card to `failed/` |

Exit code `75` is the **rate-limit escape hatch**. Adapters detect:
- HTTP 429 responses
- "rate limit" in stderr
- SIGALRM timeout (exit 142 mapped to 75)

## Current Adapters (Host-Side)

These adapters run AI tools directly on the host. They are **valid** stepping stones; `qemu.nu` replaces them in Wave 5.

| Adapter | Tool | Notes |
|---------|------|-------|
| `claude.nu` | Claude Code CLI | Default provider; supports MCP configs |
| `ollama.nu` | Ollama | Local LLM inference (qwen3-coder, etc.) |
| `codex.nu` | OpenAI Codex | Legacy; requires API key |
| `goose.nu` | Goose | Honc |
| `aider.nu` | Aider | Git-aware pair programmer |
| `opencode.nu` | OpenCode | Experimental |
| `mock.nu` | (none) | Test adapter; echoes prompt to output |
| `qemu.nu` | QEMU + zam | **Target architecture** |

Provider configuration lives in `.cards/providers.json`:
```json
{
  "default_provider": "claude",
  "providers": {
    "claude": { "command": "adapters/claude.nu", "rate_limit_exit": 75 },
    "qemu": { "command": "adapters/qemu.nu", "rate_limit_exit": 75 }
  }
}
```

## Adapter Calling Convention

```nushell
#!/usr/bin/env nu
def main [
    workdir: string,       # Card bundle path (.cards/running/foo.card/)
    prompt_file: string,   # Path to prompt.md (usually <workdir>/prompt.md)
    stdout_log: string,    # Where to write tool stdout
    stderr_log: string,    # Where to write tool stderr
    _memory_out?: string,  # Optional: memory output file (future use)
] {
    # 1. Resolve relative paths (dispatcher may pass relative paths)
    # 2. cd into workdir
    # 3. Invoke AI tool
    # 4. Check for rate-limiting in stderr or exit code
    # 5. Exit with 0 (success), 75 (retry), or 1+ (failure)
}
```

**Path resolution:** Always resolve relative paths to absolute before `cd`:
```nushell
let orig_dir = (pwd)
let workdir_abs = if ($workdir | str starts-with "/") { $workdir } else { [$orig_dir $workdir] | path join }
cd $workdir_abs
```

**Rate-limit detection:**
```nushell
if ($stderr_abs | path exists) {
    let stderr_text = open --raw $stderr_abs
    if (($stderr_text | str contains "429")
        or ($stderr_text | str contains --ignore-case "rate limit")) {
        exit 75
    }
}
```

**Timeout handling (macOS):** Use `perl` alarm for SIGALRM (exit 142 → 75):
```nushell
^perl -e 'alarm(shift); exec @ARGV or die $!' -- 3600 claude -p $prompt o> $stdout e> $stderr
if $env.LAST_EXIT_CODE == 142 { exit 75 }
```

## Creating a New Adapter

1. Copy `adapters/mock.nu` as a template
2. Implement the calling convention (5 args)
3. Resolve paths before `cd $workdir`
4. Invoke your tool, capture stdout/stderr
5. Detect rate-limiting (exit 75 if transient)
6. Add to `.cards/providers.json`
7. Test: `nu --no-config-file adapters/your-adapter.nu <workdir> ...`

## Testing Adapters

All adapters support `--test` flag for self-tests:
```bash
nu --no-config-file adapters/claude.nu --test
nu --no-config-file adapters/qemu.nu --test
```

Integration test:
```bash
td=$(mktemp -d)
mkdir -p $td/output
echo "Write hello to output/result.md" > $td/prompt.md
nu --no-config-file adapters/mock.nu $td $td/prompt.md /tmp/out.log /tmp/err.log
cat $td/output/result.md  # Should contain echoed prompt
```

## Future: Full UEFI Boot Chain

Wave 6+ will enable **dub → zam** boot:
```
qemu-system-x86_64 \
  -drive format=raw,file=<efi-image>,if=none,id=boot \
  -device virtio-blk-pci,drive=boot \
  -bios /usr/share/qemu/OVMF.fd \
  ...
```

dub performs TPM PCR measurements at boot, attestation, and A/B rollback. zam is the payload loaded by dub.

## References

- **MICROCLAW architecture:** Card bundle IS the IPC (9P mount), UART IS stdout
- **zam:** `/Users/studio/zam` — <5MB unikernel, Rust, permissive licenses only
- **dub:** `/Users/studio/efi` — UEFI bootloader, TPM attestation, A/B rollback
- **9P:** Plan 9 filesystem protocol for QEMU virtfs mounts
- **Nushell:** All adapters and scripts are `.nu` (was ZSH/Fish, migrated 2026-03-03)
