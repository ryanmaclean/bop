# QEMU Base Image Setup

The QEMU adapter expects this one-time image:

```text
~/.bop/qemu-base.qcow2
```

## 1) Install dependencies

macOS (Homebrew):

```sh
brew install qemu expect
```

## 2) Build the base image once

Run the build script from repo root:

```sh
nu scripts/build-qemu-base.nu
```

Optional explicit architecture:

```sh
nu scripts/build-qemu-base.nu --arch x86_64
nu scripts/build-qemu-base.nu --arch aarch64
```

What the script does:
- Downloads Alpine `virt` ISO for the selected arch.
- Boots it headlessly in QEMU.
- Installs Alpine onto a raw disk and installs/enables `cloud-init`.
- Converts the disk to `~/.bop/qemu-base.qcow2`.

## 3) Verify setup

```sh
nu adapters/qemu.nu --test
bop doctor
```

`bop doctor` checks for `~/.bop/qemu-base.qcow2` and prints a build hint if missing.

## Runtime behavior

- `adapters/qemu.nu` exits `127` if `qemu-system-*` is missing or the base image is missing.
- VM timeout default is `300s`; timeout maps to exit `75`.
- The adapter reads `logs/vm_exit_code` from the card bundle and returns that VM exit code (including `75` passthrough).
