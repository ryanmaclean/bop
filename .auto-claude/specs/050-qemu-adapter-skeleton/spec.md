# Spec 050 — QEMU adapter skeleton (P1 segmentation: first segment)

## Overview

TRIZ P1 (Segmentation): the QEMU VM-per-card execution path (strategic end
state per MEMORY.md) is too large to ship in one spec. This is segment 1:
a working `adapters/qemu.nu` skeleton that boots a QEMU VM, mounts the card
bundle via 9P virtfs, waits for the VM to exit, and passes through the exit
code. No actual agent runs inside the VM yet — the VM just runs `echo hello`
and exits. That is enough to prove the full pipeline works end-to-end.

## What the adapter does

```
adapters/qemu.nu <workdir> <prompt_file> <stdout_log> <stderr_log>
```

1. Writes a minimal cloud-init `user-data` that runs the prompt file as a
   shell script inside the VM.
2. Boots `qemu-system-aarch64` (Apple Silicon) or `qemu-system-x86_64`
   (Intel) with:
   - `-machine virt,accel=hvf` (macOS Hypervisor.framework)
   - `-cpu host`
   - `-m 512M`
   - `-drive` pointing to a base Alpine Linux image
   - `-virtfs local,path=<workdir>,mount_tag=card,security_model=passthrough,id=card`
   - `-serial stdio` (stdout/stderr captured to logs)
   - `-no-reboot`
3. Waits for QEMU to exit (timeout = adapter arg 5, default 300s).
4. Exit code: pass through QEMU exit code. If QEMU not found: exit 127.
   If timeout: exit 75 (EX_TEMPFAIL = rate-limit signal, causes card retry).

## Base image

Document in `docs/qemu-setup.md` how to get the Alpine base image:
```sh
# Download Alpine virt ISO for your arch
# Convert to qcow2 with 2GB disk
```
This is user-setup, not automated by the adapter. The adapter checks that
`~/.bop/qemu-base.qcow2` exists and exits 127 with a helpful error if not.

## Platform detection

```nushell
let arch = if $nu.os-info.arch == "aarch64" { "aarch64" } else { "x86_64" }
let qemu_bin = $"qemu-system-($arch)"
```

## Acceptance Criteria

- [ ] `adapters/qemu.nu --test` prints "qemu adapter: ok" and exits 0
  (test mode: just checks qemu binary exists, does not boot VM)
- [ ] Adapter exits 127 with helpful message if `~/.bop/qemu-base.qcow2` missing
- [ ] Adapter exits 127 with helpful message if `qemu-system-*` not in PATH
- [ ] `docs/qemu-setup.md` documents how to get the base image
- [ ] On a machine with QEMU: boots VM, mounts 9P, VM exits 0 → adapter exits 0
- [ ] `nu adapters/qemu.nu --test` passes in CI (QEMU not required for --test)
- [ ] `cargo test` passes (no Rust changes)

## Files

- `adapters/qemu.nu` — new adapter
- `docs/qemu-setup.md` — setup guide
