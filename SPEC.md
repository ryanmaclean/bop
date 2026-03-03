# `.bop` File Format Specification

**Version:** 0.1.0-draft  |  **Encoding:** UTF-8  |  **Line endings:** LF canonical, CRLF accepted

See `bop-design-rationale.md` for historical analysis, portability discussion, and design decisions.

---

## 1. Goals

- Define a self-contained unit of work: task definition, work products, session state, lineage, evidence, version history.
- Human-readable, agent-parseable, architecture-neutral (no binary fields, no endian/word-size assumptions).
- macOS bundle today, plain directory everywhere else.
- Agent determines task state/priority/dependencies by reading ≤1 file, ≤4KB.

---

## 2. Single-File Mode (`.bop`)

A plain text file: RFC 822 headers, blank line, Markdown body.

```
Task-Id: 7b2a91f3-task-0042
Title: Migrate auth service to mTLS
State: doing
Created: 2026-03-01T14:22:00Z
Priority: 5
Assignee: ryan
Tags: infra, security, q2
Depends-On: 7b2a91f3-task-0039
Glyph: 5️⃣
Produces: mtls-config, auth-certs
Consumes: mesh-config

## Description

Migrate the auth service from plaintext gRPC to mTLS.

## Acceptance Criteria

- [ ] Certs provisioned via cert-manager
- [ ] Load test at 2x peak traffic
- [x] Runbook updated

## Agent Log

- 2026-03-01T14:30:00Z [claude] Created from Slack thread #platform-eng/834
```

### 2.1 Header Rules

- Syntax: RFC 822 (`Name: Value`, continuation lines start with whitespace).
- Header names: ASCII-only (U+0020–U+007E), case-insensitive.
- Header values: UTF-8. CJK, RTL, emoji all valid.
- YAML frontmatter (`---` delimited) accepted on read, normalized to RFC 822 on write.

### 2.2 Required Headers

| Header | Description |
|---|---|
| `Task-Id` | Unique identifier (UUID or namespaced ID) |
| `Title` | Human-readable task name (UTF-8) |
| `State` | One of: `inbox`, `todo`, `doing`, `review`, `done`, `blocked`, `cancelled` |
| `Created` | ISO 8601 timestamp |

### 2.3 Reserved Headers

| Header | Description |
|---|---|
| `Priority` | Planning poker scale: 1, 2, 3, 5, 8, 13, 21 |
| `Estimate` | Unitless integer (team defines semantics) |
| `Assignee` | Person or agent identifier |
| `Tags` | Comma-separated labels |
| `Depends-On` | Comma-separated Task-Id references (task→task graph) |
| `Blocked-By` | Comma-separated Task-Id references |
| `Due` | ISO 8601 date |
| `Language` | BCP 47 tag for body content |
| `Content-Encoding` | Body encoding if not UTF-8 |
| `Format-Version` | Spec version (e.g., `0.1.0`) |
| `Board` | Board name this task belongs to |
| `Column` | Kanban column (may differ from State) |
| `Glyph` | Visual representation for Finder/terminal |
| `Agent-Lock` | Advisory lock: agent session ID |
| `Agent-Lock-Expires` | ISO 8601 expiry for advisory lock |
| `Checksum-Body` | `algorithm:hex` hash of body section |
| `Resume-Command` | Shell command to rehydrate working environment |
| `Lineage-Namespace` | OpenLineage namespace (typically board/team) |
| `Lineage-Run-Id` | OpenLineage run ID for this execution |
| `Produces` | Comma-separated dataset names (OpenLineage outputs) |
| `Consumes` | Comma-separated dataset names (OpenLineage inputs) |
| `X-*` | Reserved for local extensions |

### 2.4 Body

Everything after the first blank line. Markdown. Contains description, acceptance criteria, checklists, agent logs. Agents can stop reading at the blank line if they only need headers.

---

## 3. Bundle Mode (`.bop` directory)

The bundle IS the work. Self-contained: task definition, work products, terminal session, evidence, lineage, version history.

### 3.0 macOS UTI registration (required for bundle behavior)

A `.bop` directory is treated as an opaque bundle by Finder only when the `com.apple.bop` UTI is registered on the system. Registration is done by installing the `bop` app or CLI tool, which includes a UTI declaration conforming to `com.apple.package`.

Every `.bop` bundle MUST contain `Info.plist` at its root:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key>
  <string>com.apple.bop.{{task-id}}</string>
  <key>CFBundleName</key>
  <string>{{title}}</string>
  <key>CFBundlePackageType</key>
  <string>BNDL</string>
</dict>
</plist>
```

Without `Info.plist`, macOS will not recognize the directory as a bundle and Finder will show it as a navigable folder.

```
migrate-auth-mtls.bop/
  Info.plist                     # UTI declaration (REQUIRED)
  QuickLook/
    Preview.html                 # Rendered card (generated on state change)
    Thumbnail.png                # 512×512, color-coded by state
  task.bop                       # Task definition (headers + body)
  .bop/                          # Control plane
    state                        # Single line: current state (O(1) reads)
    lock                         # Advisory lock (agent-id + expiry)
    transitions.log              # Append-only state transition log
    config                       # Bundle config (RFC 822 headers)
    lineage.jsonl                # OpenLineage events (append-only)
    baggage                      # OTel-style propagated context
  work/                          # WORK PRODUCTS (the point of the task)
    src/
    scripts/
    docs/
  session/                       # Working environment (Zellij writes here)
    zellij/                      # ZELLIJ_DATA_DIR points here
      config.kdl                 # Per-task Zellij config
      layout.kdl                 # Pane arrangement, startup commands
      session.name               # Session name for attach
      sessions/                  # Zellij native session data
    shell/
      history.nu                 # Curated command history (runbook, nushell)
      env.nu                     # Environment variables (references, not secrets, nushell)
    editor/
      workspace.code-workspace
  output/                        # Final deliverables (patch, QA report, artifact)
    diff.patch
    qa_report.md
  evidence/                      # Proof the work is correct
    screenshots/
    test-results/
    approvals/                   # Can contain nested .bop files
  traces/                        # OTel-compatible agent telemetry (JSONL)
  .jj/                           # Version control (jj preferred, .git/ accepted)
```

### 3.1 `work/` — jj workspace OR git worktree

`work/` is the VCS checkout for this task. It is **either**:
- a **jj workspace** (`jj workspace add work/`), pointing to a named change in the parent repo — preferred, since jj workspaces are lighter than worktrees and support concurrent editing without branch ceremony; or
- a **git worktree** (`git worktree add work/ job/<task-id>`) — used when the parent repo is git-only.

Declared in `.bop/config` via `Work-VCS-Mode: jj-workspace | git-worktree | copy`. Only one mode active per bundle. `bop init` detects the parent repo type and sets this automatically.

### 3.2 `session/` — Zellij lives in the bundle

bop bundles Zellij. The only Zellij patch: respect `ZELLIJ_DATA_DIR` env var. bop's launch wrapper handles everything else:

1. Resolve bundle absolute path
2. Rewrite `cwd` fields in `layout.kdl` to absolute paths
3. Source `session/shell/env.nu`
4. Set `ZELLIJ_DATA_DIR=<bundle>/session/zellij`
5. Exec Zellij

Zellij writes scrollback, pane state, resurrection data directly into the bundle. No copy. No sync. The scrollback is the implicit runbook — what was actually executed, not what was planned.

**`config.kdl`**: Per-task Zellij config. Scrollback limits are Zellij's concern, not bop's.

**`layout.kdl`**: Pane arrangement with startup commands. Declarative blueprint.

**`session.name`**: A reference — the Zellij session runs outside the bundle; the bundle holds the name to reattach. Zellij data (scrollback, pane state) lives in `<bundle>/session/zellij/` via `ZELLIJ_DATA_DIR`. Created on `doing` transition, detached (not killed) on state change.

**`session/shell/history.nu`**: Curated command history (Nushell, MIT-licensed). Filtered by working directory. Complements scrollback (which includes output).

**`session/shell/env.nu`**: Sourceable env vars (Nushell). References only — no secrets (see 3.6).

**Non-Zellij**: `Session-Type` in `.bop/config` declares the type (`zellij`, `tmux`, `screen`, `none`).

**Resume**: `bop resume` runs the five-step launch. On first `doing` transition it writes a per-bundle launchd agent to `~/Library/LaunchAgents/com.apple.bop.<task-id>.plist` (see §10 for the plist template). Falls back to `cd work && $SHELL` if Zellij unavailable.

### 3.3 `evidence/`

Distinct from `work/`: answers "how do we know it's correct" vs "what was produced." Screenshots, test results, approval records. Approvals can be nested `.bop` files.

### 3.4 `.bop/` control plane

**`.bop/state`**: One line, current state. O(1) read. Source of truth is `State:` in `task.bop`; `.bop/state` is the fast-path index. If they disagree, `task.bop` wins.

**`.bop/lock`**: Advisory. Agent-id + ISO 8601 expiry. Real coordination is POSIX `rename()`.

**`.bop/transitions.log`**: Append-only.

```
2026-03-01T14:22:00Z inbox claude-session-abc "Created from Slack thread"
2026-03-02T09:00:00Z doing ryan "Starting work"
```

Format: `ISO8601 new-state actor "reason"`

**`.bop/config`**: Bundle-level settings (RFC 822 headers):

```
VCS: jj
VCS-Auto-Commit: on-state-change
Session-Type: zellij
Work-Root: work
Secrets-Policy: reference-only
Lineage-Collector: file
Trace-Format: otel-jsonl
```

### 3.5 Version control is part of the format

Declared in `.bop/config`. Precedence: jj (`.jj/`) → git (`.git/`) → none (`history/` with timestamped copies). VCS tracks entire bundle. Auto-commit modes: `on-state-change`, `manual`, `on-save`.

### 3.6 Secrets policy

Bundle MUST NOT contain plaintext secrets. `Secrets-Policy` in `.bop/config`:

- `reference-only` (default): env var references, not values
- `keychain`: macOS Keychain / Linux `secret-tool`
- `vault`: HashiCorp Vault paths
- `sops`: Mozilla SOPS encrypted inline

Agent SHOULD stop and report if it cannot resolve secret references.

### 3.7 macOS bundle integration

Finder label colors map to states (convention, not spec):

None=inbox, Blue=todo, Yellow=doing, Orange=review, Green=done, Red=blocked, Gray=cancelled.

---

### 3.8 APFS copy semantics (macOS)

On macOS, bundle creation from a template MUST use APFS copy-on-write cloning. Plain recursive copy (`cp -R`, `rsync`) is forbidden for template instantiation — it defeats the zero-disk-cost property.

**Template instantiation:**
```sh
# REQUIRED: APFS COW clone
cp -c templates/implement.bop pending/feat-auth.bop
# FORBIDDEN: plain recursive copy
# cp -r templates/implement.bop pending/feat-auth.bop  ← DO NOT USE
```

**Bundle archival** — preserve HFS+ compression and resource forks:
```sh
ditto --hfsC source.bop dest.bop
```

**Log compression** — compress completed terminal logs to reclaim space:
```sh
ditto --hfsCompression \
  bundle.bop/session/zellij/ \
  bundle.bop/session/zellij/
```

**Platform abstraction:**

| Operation | macOS | Linux (Btrfs) | Fallback |
|---|---|---|---|
| Template clone | `cp -c` | `cp --reflink=auto` | `cp -r` (with warning) |
| Bundle archive | `ditto --hfsC` | `tar -czf` | `tar -czf` |
| Log compress | `ditto --hfsCompression` | `btrfs filesystem defragment -czstd` | n/a |

The `bop` Rust implementation enforces this: `clone_bundle()` returns `Err` if `clonefile(2)` is unavailable on macOS, forcing explicit error handling rather than silent degradation.

### 3.9 `output/`

Distinct from `work/` (in-progress) and `evidence/` (proof). `output/` holds the artifact that crosses the boundary — the diff, QA report, or other deliverable handed to the next stage, merge gate, or human reviewer. Written by the agent on task completion; read by the merge gate.

---

## 4. Lineage and Observability

### 4.1 OTel baggage: `.bop/baggage`

RFC 822 headers. Inherited context that propagates through task chains:

```
Trace-Id: abc123def456789
Board: platform-eng
Pipeline: mtls-rollout
Upstream-Task: 7b2a91f3-task-0039
Cost-Center: CC-4200
```

Agent reads baggage on task claim, propagates into OTel spans, copies to downstream tasks.

### 4.2 OpenLineage: `.bop/lineage.jsonl`

Append-only JSONL. OpenLineage START/COMPLETE events with job, run, inputs, outputs. Compatible with Marquez, Datahub, Amundsen.

### 4.3 Agent traces: `traces/*.jsonl`

OTel-compatible JSONL spans. Direct ingest into Datadog, Jaeger, any OTel collector. Carry baggage as span attributes.

### 4.4 How they relate

| Mechanism | Question | Propagation |
|---|---|---|
| Baggage | "What context does this task carry?" | Inherited down |
| Lineage | "What data flows in/out?" | Sideways (task↔dataset) |
| Traces | "What did the agent do?" | Up (into APM) |

---

## 5. Agent Overhead Optimizations

| Operation | Mechanism | Cost |
|---|---|---|
| Read state | `.bop/state` file | O(1), <16 bytes |
| Check lock | `.bop/lock` stat + read | O(1) |
| Scan headers | Read until first blank line | ≤4KB |
| Detect body changes | `Checksum-Body` header | O(1) compare |
| State transition | Write temp + `rename()` | Atomic, POSIX |
| Board scan (N tasks) | N reads of `.bop/state` | O(N), <16 bytes each |
| Dependency graph | `Depends-On` + `Produces`/`Consumes` headers | Header-only scan |

---

## 6. Multilingual Support

- Header keys: ASCII always (protocol, not prose)
- Header values: UTF-8 (`Title: mTLS認証サービスの移行` is valid)
- Body: UTF-8, `Language:` header declares BCP 47 tag
- EBCDIC bridge: ASCII keys transcode mechanically; values are opaque UTF-8 bytes
- Legacy encodings (Shift_JIS, Big5): `Content-Encoding` header declares; agents SHOULD convert to UTF-8 on read
- Future: `Header-Aliases` for localized header names (display convenience, ASCII canonical)

---

## 7. Backward Compatibility

- **grep/awk/sed**: `grep "^State:" *.bop` works
- **Obsidian/Hugo/Jekyll**: YAML frontmatter accepted on read
- **Taskwarrior**: Bridge maps Task-Id↔UUID, State↔status
- **Make**: `Depends-On` maps to prerequisite graph
- **Email**: Structurally identical to RFC 822 message with text/markdown body
- **Extension**: Unknown headers ignored by older parsers, preserved on round-trip

---

## 8. Reference: POSIX Shell Parser

```sh
#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    "") break ;;
    [!\ ]*:*)
      key="${line%%:*}"
      val="${line#*: }"
      printf '%s\t%s\n' "$key" "$val"
      ;;
    [\ ]*)
      printf '\t+%s\n' "${line# }"
      ;;
  esac
done < "$1"
```

## 9. Reference: Atomic State Transition

```sh
#!/bin/sh
TASK_DIR="$1"; NEW_STATE="$2"; ACTOR="$3"; REASON="$4"
printf '%s\n' "$NEW_STATE" > "$TASK_DIR/.bop/state.tmp"
mv "$TASK_DIR/.bop/state.tmp" "$TASK_DIR/.bop/state"
printf '%s %s %s "%s"\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  "$NEW_STATE" "$ACTOR" "$REASON" >> "$TASK_DIR/.bop/transitions.log"
sed -i "s/^State: .*/State: $NEW_STATE/" "$TASK_DIR/task.bop"
```

---

## 10. Reference: launchd Agent Plist Template

`bop resume` writes this plist to `~/Library/LaunchAgents/com.apple.bop.<task-id>.plist` on first `doing` transition.

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>com.apple.bop.{{task-id}}</string>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/local/bin/bop</string>
    <string>resume</string>
    <string>{{bundle-path}}</string>
  </array>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>
  </dict>
  <key>RunAtLoad</key>
  <false/>
  <key>StandardOutPath</key>
  <string>{{bundle-path}}/session/zellij/launchd.log</string>
  <key>StandardErrorPath</key>
  <string>{{bundle-path}}/session/zellij/launchd.err</string>
</dict>
</plist>
```

(`KeepAlive.SuccessfulExit: false` — crash-recovery only, not eternal restart.)

---

## 11. Open Questions

1. **`.bop/state` as symlink?** `readlink` vs `read`. Some filesystems handle symlinks poorly.
2. **Bundle-level checksum?** Merkle tree over full bundle is slow for large `work/` assets.
3. **Board index at scale?** 10K+ tasks: board-level `board.bopindex` vs scan. Cache invalidation tradeoff.
4. **Upstreaming `ZELLIJ_DATA_DIR`?** Submit to Zellij upstream before shipping, or fork and contribute later?
5. **Digital signatures?** `Signature:` header with detached PGP/signify for tamper-evident audit trails.
6. **Bundle-to-bundle references?** `Depends-On` uses Task-Id. Should it also support filesystem paths or content-addresses?
7. **Large binaries in `work/`?** jj/git struggle without LFS. `.bopignore`? Content-addressable `work/blobs/`?
8. **Multi-agent concurrency?** Sub-task locking at `work/` subdirectory level, or out of scope?
9. **`work/` as jj workspace vs git worktree?** jj workspaces are preferred (lighter, no branch required), but git worktrees work. Should `bop` auto-detect or require explicit `Work-VCS-Mode` in `.bop/config`?
10. **launchd plist lifecycle?** Should `bop` install/uninstall the per-bundle plist automatically on `doing`/`done` transitions, or leave plist management to the user?
11. **`ditto --hfsCompression` on active bundles?** Compressing a bundle while an agent is writing logs to it is unsafe. Should compression be gated on `done`/`cancelled` states only?
