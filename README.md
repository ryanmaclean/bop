# `.bop` File Format

> A self-contained, human-readable, agent-parseable unit of work — task definition, work products, session state, lineage, evidence, and version history in a single file or directory bundle.

**Version:** 0.1.0-draft  |  **Encoding:** UTF-8  |  **Status:** Draft

---

## What is `.bop`?

`.bop` is a file format for tracking tasks and their associated work. It comes in two modes:

- **Single-file mode** (`.bop`): An RFC 822-style plain text file with structured headers and a Markdown body. Grep-friendly, email-compatible, Obsidian-ready.
- **Bundle mode** (`.boptask` directory): A self-contained directory that holds the task definition, all work products, a Zellij terminal session, observability data, evidence, and version history — all in one place.

Designed to be used by humans and AI agents alike. An agent can determine task state, priority, and dependencies by reading ≤1 file, ≤4KB.

---

## Documentation

| Document | Description |
|---|---|
| [SPEC.md](SPEC.md) | Full format specification (v0.1.0-draft) |
| [bop-design-rationale.md](bop-design-rationale.md) | Historical analysis, portability discussion, and design decisions |

---

## Quick Example

```
Task-Id: 7b2a91f3-task-0042
Title: Migrate auth service to mTLS
State: doing
Created: 2026-03-01T14:22:00Z
Priority: 5
Assignee: ryan
Tags: infra, security, q2

## Description

Migrate the auth service from plaintext gRPC to mTLS.

## Acceptance Criteria

- [ ] Certs provisioned via cert-manager
- [ ] Load test at 2x peak traffic
- [x] Runbook updated
```

---

## Key Features

- **RFC 822 headers** — parseable with `grep`, `awk`, `sed`, any language
- **Markdown body** — human-readable, renders in GitHub, Obsidian, Hugo, Jekyll
- **Atomic state transitions** — POSIX `rename()` for safe concurrent updates
- **Bundled terminal sessions** — Zellij session lives inside the bundle; scrollback is the implicit runbook
- **OpenLineage + OTel** — built-in observability: lineage, baggage propagation, agent traces
- **Secrets-safe** — bundles reference secrets, never store them
- **VCS-native** — jj or git tracks the entire bundle, including work products

---

## States

`inbox` → `todo` → `doing` → `review` → `done`

Also: `blocked`, `cancelled`

---

## Status

This is a **0.1.0-draft** spec. See [SPEC.md § 10](SPEC.md#10-open-questions) for open questions and [bop-design-rationale.md](bop-design-rationale.md) for design decisions.