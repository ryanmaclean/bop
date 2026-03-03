# bop

A self-contained unit of work: task definition, work products, terminal session, lineage, evidence, and version history — all in one directory bundle.

## What is a `.bop` file?

A `.bop` file is a macOS directory [bundle](https://developer.apple.com/documentation/foundation/bundle) (opaque in Finder, navigable via "Show Package Contents") that holds everything needed to define, execute, resume, and audit a task:

- **`task.md`** — [RFC 822](https://datatracker.ietf.org/doc/html/rfc822) headers + Markdown body. The task definition.
- **`.bop/`** — Control plane: state, lock, transition log, lineage, OTel baggage.
- **`work/`** — The actual work(space/tree) products from `git`/`jj` respectively (source code, configs, scripts, docs).
- **`session/`** — [Zellij](https://github.com/zellij-org/zellij) terminal session bundled with the task. Resume anywhere.
- **`evidence/`** — Proof the work is correct ([termframe](https://github.com/pamburus/termframe) screenshots, compressed test results, approvals).
- **`output/`** — Final deliverables handed to the next stage or merge gate.
- **`traces/`** — OTel-compatible agent telemetry.

## Quick start

```sh
# Create a task (single-file mode)
cat > my-task.bop <<EOF
Task-Id: $(uuidgen)
Title: My first task
State: inbox
Created: $(date -u +%Y-%m-%dT%H:%M:%SZ)

Description of what needs to be done.
EOF

# Read state (O(1))
grep "^State:" my-task.bop

# Create a bundle (directory mode)
cp -c templates/implement.bop pending/my-task.bop   # macOS APFS clone
bop resume pending/my-task.bop                      # launch Zellij session
```

---

## Documentation

| Document | Description |
|---|---|
| [SPEC.md](SPEC.md) | Full format specification (v0.1.0-draft) |
| [bop-design-rationale.md](bop-design-rationale.md) | Historical analysis, portability discussion, and design decisions |
| [CONTRIBUTING.md](CONTRIBUTING.md) | How to contribute |

---

## Key Features

- **RFC 822 headers** — parseable with `grep`, `awk`, `sed`, any language
- **Markdown body** — human-readable, renders in GitHub, Obsidian, Hugo, Jekyll
- **Atomic state transitions** — POSIX `rename()` for safe concurrent updates
- **Bundled terminal sessions** — Zellij session lives inside the bundle; scrollback is the implicit runbook
- **OpenLineage + OTel** — built-in observability: lineage, baggage propagation, agent traces
- **Secrets-safe** — bundles reference secrets, never store them
- **VCS-native** — jj or git tracks the entire bundle, including work products
- **APFS-native** — zero-disk-cost template cloning on macOS

---

## States

`inbox` → `todo` → `doing` → `review` → `done`

Also: `blocked`, `cancelled`

---

## Status

This is a **0.1.0-draft** spec. See [SPEC.md § 11](SPEC.md#11-open-questions) for open questions and [bop-design-rationale.md](bop-design-rationale.md) for design decisions.

YES! bop was heavily inspired by Steve Yegge's [Beads](https://github.com/steveyegge/beads) ❤️
