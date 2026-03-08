# QEMU adapter: VM-per-card execution

## Goal

Create `adapters/qemu.nu` — a bop adapter that runs each card in an isolated
QEMU VM instead of on the host. This is the target execution model replacing
the host-side claude.nu/ollama.nu/codex.nu adapters.

## Architecture

```
adapters/qemu.nu <workdir> <prompt_file> <stdout_log> <stderr_log>
  ↓
qemu-system-x86_64 \
  -kernel <zam-image> \        # zam unikernel from /Users/studio/zam
  -drive format=raw,...        # dub EFI (from /Users/studio/efi) if UEFI mode
  -virtfs local,path=<workdir>,mount_tag=card,... \  # 9P: card bundle
  -serial file:<stdout_log>    # zam writes structured output to UART
  -nographic -no-reboot
  ↓
zam reads card/prompt.md via 9P
zam invokes tool (claude/ollama/codex — configured in card meta.json)
zam writes card/output/result.md via 9P
VM exits: 0=success, 75=rate-limit, other=failure
```

## Prerequisites

- `qemu-system-x86_64` installed (`which qemu-system-x86_64`)
- zam image built: `/Users/studio/zam/target/zam.elf` or similar
- Card bundle at `<workdir>` with `prompt.md` and `output/` directory

## Steps

1. Check prerequisites:
   ```sh
   which qemu-system-x86_64 || brew install qemu
   ls /Users/studio/zam/target/ 2>/dev/null || echo "zam not built yet"
   ```

2. Create `adapters/qemu.nu`:
   ```nushell
   #!/usr/bin/env nu
   def main [
     workdir: path,
     prompt_file: path,
     stdout_log: path,
     stderr_log: path,
   ] {
     # Check zam image exists
     let zam_image = "/Users/studio/zam/target/zam.elf"
     if not ($zam_image | path exists) {
       "zam image not found — run: cd /Users/studio/zam && cargo build --release"
         | save --append $stderr_log
       exit 1
     }

     # 9P virtfs mounts the card bundle into the VM
     let exit_code = (^qemu-system-x86_64
       -kernel $zam_image
       -virtfs $"local,path=($workdir),mount_tag=card,security_model=mapped-xattr"
       -serial $"file:($stdout_log)"
       -append "card_mount=card"
       -m 128M
       -nographic
       -no-reboot
       | complete).exit_code

     exit $exit_code
   }
   ```

3. Add a smoke test — create a minimal card workdir, run the adapter,
   verify it exits cleanly (even if zam isn't built yet, exit 1 is correct):
   ```sh
   td=$(mktemp -d)
   mkdir -p $td/output
   echo "Write hello to output/result.md" > $td/prompt.md
   nu --no-config-file adapters/qemu.nu $td $td/prompt.md /tmp/qemu-out.log /tmp/qemu-err.log
   echo "Exit: $?"
   ```

4. Register `qemu` as a valid provider in `.cards/providers.json`:
   ```json
   { "name": "qemu", "adapter": "adapters/qemu.nu", "cooldown_s": 0 }
   ```

5. Document in `adapters/README.md` (create if absent):
   - The VM-first architecture
   - How 9P mounts the card bundle
   - How to build zam and dub
   - Exit code contract (0/75/other)

6. `make check`

## Acceptance

`adapters/qemu.nu` exists and is executable.
Running it without a built zam image exits 1 with a clear error (not a crash).
`qemu` appears in providers.json.
`make check` passes.

## Note on zam/dub status

zam (`/Users/studio/zam`) is Wave 0-1 in its own dispatch.nu.
dub (`/Users/studio/efi`) is Wave 1 in its own dispatch.nu.
This spec creates the adapter scaffold now; full VM boot works once zam ships.
The adapter fails gracefully until then.
