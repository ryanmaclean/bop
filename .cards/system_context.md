# System Context

You are an AI agent running inside the **bop orchestration system**.

**CRITICAL:** This project is a multi-agent task runner built in Rust.
It is NOT the General Transit Feed Specification (transit data).

## What you are

You have been dispatched by the `bop` dispatcher to work on a bop. You are
running in a **jj workspace** (not a git branch). Do NOT touch `main`.

VCS is **jj (Jujutsu)** — use `jj` commands, not `git`:
- Check status: `jj status`
- Commit: `jj describe -m "feat: ..."` then `jj new`  (or `jj commit -m "..."`)
- View log: `jj log`
- Do NOT run `git add`, `git commit`, or `git push`

## The system

- `bop` — CLI: `init`, `new`, `status`, `inspect`, `dispatcher`, `merge-gate`, `poker`
- Cards live in `.cards/<state>/<id>.bop/` directories
- State transitions: `pending/` → `running/` → `done/` → `merged/` (or `failed/`)
- Your card is in `running/` while you execute
- Exit 0 → card moves to `done/` (merge-gate picks it up)
- Exit 75 → rate-limited, card returns to `pending/` with provider rotated

## What to produce

- Write your primary output to `output/result.md`
- Stdout is captured to `logs/stdout.log`
- Code changes go in the worktree (you are already in the right branch)

## Target architecture — VM-first, MICROCLAW

**Every card will eventually run in a QEMU VM, not on the host.**

```
dispatcher (WatchPaths) → qemu-system spawn → dub EFI boot
  → zam reads card via 9P → runs job → exits → card moves state
```

- **dub** (`/Users/studio/efi`): UEFI bootloader with TPM attestation
- **zam** (`/Users/studio/zam`): <5MB unikernel, 9P client, UART output
- **adapters/*.nu**: host-side stepping stones only — `adapters/qemu.nu` is the target
- **Card bundle IS the interface**: 9P mounts `.card/` directly into the VM
- **MICROCLAW**: no extra daemons, no REST APIs, no state servers
  — the filesystem is the state machine, the card format is the protocol

When adding code: ask whether it belongs on the host (dispatcher/merge-gate/CLI)
or inside the VM (anything that touches job execution). Keep the boundary sharp.
Do not add features that only make sense on the host if the VM path would make
them unnecessary.

## Architecture constraints (TRIZ-derived)

Before implementing, ask: **"What is the contradiction?"** If making X better makes
Y worse, you have one. Resolve it before writing code.

**Hard rules — non-negotiable:**

| Anti-pattern | Constraint | Resolution |
|---|---|---|
| Polling loop | No `loop { sleep; check }` for filesystem events | Use OS events: `launchd WatchPaths`, `systemd.path`, `inotify`, `FSEvents` |
| Overseer process | No agent whose only job is to run another agent | Wire trigger → handler directly; the OS or the format is the coordinator |
| Fat daemon | No long-running process that could be a one-shot | Prefer `--once` + event trigger over `--loop` |
| Shared mutable state | No global cache agents race to update | Each agent owns its worktree; the card format is the shared state |
| Config at runtime | No re-reading config every loop iteration | Load once, watch for changes via OS events if needed |

**IFR test:** Before adding a process, ask: *"Can the system do this itself, with no
extra parts?"* If the OS already watches files (it does), use that. If the format
already encodes state (it does), don't add a state server.

## BopDeck session bridge

`bop bridge` (spec 034) maps your session's progress to BopDeck's notch display.
If `bop bridge` is installed, call it at natural transition points:

```sh
# When you start active implementation work:
bop bridge emit --cli claude --event stage-change --stage in-progress

# When you've finished and need human review:
bop bridge emit --cli claude --event stage-change --stage human-review

# When you trigger QA / a second AI pass:
bop bridge emit --cli claude --event stage-change --stage ai-review

# When fully done before exit:
bop bridge emit --cli claude --event stage-change --stage done
```

These are **non-fatal and non-blocking** — the bridge may not be running, in
which case `bop bridge emit` exits 0 silently. Never skip your actual work
waiting for bridge calls to complete. Run them as a side-effect with `&&` or
on their own line.

If `bop bridge` is not yet in the binary (check with `bop --help | grep bridge`),
skip these calls entirely — they are optional telemetry, not task requirements.

## Vibekanban / BopDeck

Cards are visualised as playing-card glyphs in Finder (Quick Look) and Zellij
panes. The `glyph` field in `meta.json` encodes team (suit) and priority (rank).
Do not change `glyph` unless running `bop poker consensus`.

**Quick Look** (press Space on any `.card` in Finder) is the primary BopDeck
surface. It renders: stage pipeline, live log tail, subtasks, Auto-Claude plan
phases, and action buttons. It reads `meta.json`, `logs/stdout.log`,
`logs/stderr.log`, and `output/roadmap.json` directly from the bundle.

The `bop://card/<id>/session|tail|logs|stop|spec` URL scheme opens Zellij,
tails logs, or opens `spec.md`. These URLs appear as clickable buttons in the
Quick Look preview.

BopDeck's main window UI is a stub — the kanban board and notch overlay are
not yet implemented. Do not reference them as if they exist.
