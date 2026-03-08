# Spec 058 — QEMU VM pool: pre-warm N VMs (TRIZ P10 Preliminary action)

## Overview

P10 Preliminary action: spin up N idle QEMU VMs before cards arrive so dispatch
is instant — no cold-boot latency per card. The pool maintains a queue of VMs
in `ready` state; the dispatcher leases one per card, injects the card bundle
via 9P hot-plug or snapshot restore, and releases it back after completion.

## Architecture

```
bop factory pool --size 3      # start pool of 3 idle VMs
                               # each VM: Alpine + cloud-init agent waiting
bop dispatcher                 # leases a VM from pool per card
                               # injects card via 9P hot-plug
                               # on card done: restore VM to clean snapshot
```

### Pool manager (`crates/bop-cli/src/pool.rs`)

- `VmPool { size: usize, vms: Vec<VmSlot> }`
- `VmSlot { pid: u32, state: Ready | Leased(card_id) | Booting }`
- Pool state persisted in `.cards/.pool/pool.json`
- Background `tokio::task` monitors VM health (ping every 5s via QEMU monitor socket)
- Dead VMs are replaced automatically

### VM lifecycle
1. **Boot**: `qemu-system-* -snapshot` from `~/.bop/qemu-base.qcow2` (COW — no disk writes persist between cards)
2. **Ready**: VM boots to Alpine shell, waits for `START` signal on virtio-serial
3. **Lease**: dispatcher writes card bundle path to virtio-serial; VM mounts it and runs agent
4. **Release**: VM exits agent, sends `DONE <exit_code>` on virtio-serial; pool resets VM via QEMU snapshot restore (instant)

### `adapters/qemu-pool.nu`

New adapter that leases from the pool instead of booting a fresh VM:
```nushell
def main [workdir prompt_file stdout_log stderr_log] {
    let slot = pool_lease $workdir   # blocks until slot available
    pool_inject $slot $workdir $prompt_file
    pool_wait $slot $stdout_log $stderr_log
    pool_release $slot
}
```

## Commands

```sh
bop factory pool --size 3     # start/resize pool
bop factory pool status       # show slot states + PIDs
bop factory pool stop         # shut down all VMs
```

## Acceptance Criteria

- [ ] `bop factory pool --size 2` starts 2 idle VMs (visible in `bop factory pool status`)
- [ ] `adapters/qemu-pool.nu` leases a slot, runs card, releases
- [ ] Pool auto-replaces a crashed VM within 10s
- [ ] `bop factory pool stop` cleanly shuts down all VMs
- [ ] Pool state survives dispatcher restart (reads `.cards/.pool/pool.json`)
- [ ] `-snapshot` flag ensures no disk state leaks between cards
- [ ] `cargo clippy -- -D warnings` clean
- [ ] `cargo test` passes (unit test pool lease/release logic without real QEMU)

## Files

- `crates/bop-cli/src/pool.rs` — new VM pool manager
- `adapters/qemu-pool.nu` — new pool-aware adapter
- `crates/bop-cli/src/factory.rs` — add `pool` subcommand
- `crates/bop-cli/src/main.rs` — wire factory pool commands
