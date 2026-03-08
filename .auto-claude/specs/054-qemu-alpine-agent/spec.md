# Spec 054 — QEMU segment 2: Alpine cloud-init + shell agent in VM

## Overview

TRIZ P10 (Preliminary action): pre-prepare the VM image so cold-start is fast.
Spec 050 proved the pipeline end-to-end with a no-op VM. This spec (segment 2)
makes the VM actually run the bop prompt as a shell script and write output to
the 9P-mounted card directory.

## What changes from segment 1

### 1. Cloud-init user-data

`adapters/qemu.nu` now writes a real `user-data` cloud-init script:
```yaml
#cloud-config
runcmd:
  - sh /card/prompt.md > /card/output/result.md 2>&1
  - echo $? > /card/logs/vm_exit_code
  - poweroff -f
```

The card bundle is mounted at `/card` via 9P (already working from spec 050).

### 2. Base image build script

`scripts/build-qemu-base.nu` downloads Alpine Linux virt ISO, boots it
headlessly, installs cloud-init, converts to qcow2, saves to `~/.bop/qemu-base.qcow2`.

```nushell
# Usage:
nu scripts/build-qemu-base.nu          # build for current arch
nu scripts/build-qemu-base.nu --arch x86_64
```

This is a one-time setup step. `bop doctor` checks for the image and prints
the build command if missing.

### 3. Exit code passthrough

After QEMU exits, `adapters/qemu.nu` reads `/card/logs/vm_exit_code` and uses
it as the adapter exit code. This preserves the exit 75 (rate-limit) signal
if the agent inside the VM returns it.

### 4. Timeout via QEMU `-rtc`

Set `-rtc base=localtime` and a QEMU monitor `system_powerdown` after timeout
seconds. Alternatively, use `timeout <N> qemu-system-*` in the shell wrapper.

## Acceptance Criteria

- [ ] `nu adapters/qemu.nu <workdir> <prompt_file> <stdout> <stderr>` boots VM,
  runs prompt.md as shell script, writes output to `output/result.md`, exits 0
- [ ] Exit code from VM shell is passed through (exit 75 → adapter exits 75)
- [ ] `scripts/build-qemu-base.nu` downloads and builds the Alpine base image
- [ ] `bop doctor` reports missing base image with build command hint
- [ ] Timeout (default 300s) kills VM and returns exit 75
- [ ] `nu adapters/qemu.nu --test` still passes without booting VM
- [ ] `docs/qemu-setup.md` updated with new build script instructions
- [ ] `cargo test` passes (no Rust changes required)

## Files

- `adapters/qemu.nu` — update to write cloud-init + read exit code
- `scripts/build-qemu-base.nu` — new: Alpine base image builder
- `docs/qemu-setup.md` — update setup guide
- `crates/bop-cli/src/doctor.rs` — add QEMU base image check
