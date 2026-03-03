# `.bop` Design Rationale

This document covers the historical analysis, portability discussion, and design decisions behind the `.bop` file format. For the normative specification, see [SPEC.md](SPEC.md).

---

## 1. Why RFC 822 headers?

**Decision:** Use RFC 822 header syntax (`Name: Value`) rather than YAML, TOML, JSON, or a custom format.

**Rationale:**
- RFC 822 is the format of email, HTTP headers, and countless Unix tools. It is universally understood.
- Parseable with `grep "^State:" task.bop` — no parser library required.
- Continuation lines (lines starting with whitespace) allow multi-line values without quoting rules.
- YAML is accepted on read for compatibility with Obsidian/Hugo/Jekyll, but RFC 822 is the canonical write format.
- JSON and TOML require full parsers and are not line-oriented, making streaming reads harder.
- The blank-line separator between headers and body mirrors HTTP and email exactly — agents can stop reading at the blank line if they only need metadata.

---

## 2. Why two modes (single-file vs bundle)?

**Decision:** Support both a plain `.bop` file and a `.boptask` directory bundle.

**Rationale:**
- Single-file mode is the lowest common denominator: works on any filesystem, in any tool, over email or chat.
- Bundle mode is the full power mode: the task IS its work. Source code, terminal session, evidence, and lineage all live together.
- macOS treats `.boptask` directories as opaque bundles (like `.app`), giving a native file-like experience in Finder.
- On Linux and Windows, `.boptask` is just a directory — no special support needed.
- The two modes share the same header format, so parsers only need one code path for metadata.

---

## 3. Why Zellij?

**Decision:** Bundle Zellij as the default terminal multiplexer, writing session data into the bundle via `ZELLIJ_DATA_DIR`.

**Rationale:**
- Zellij has a clean data directory model that can be redirected with a single env var (`ZELLIJ_DATA_DIR`).
- Scrollback, pane state, and resurrection data written directly into the bundle — no copy, no sync.
- The scrollback becomes the implicit runbook: what was actually executed, not what was planned.
- tmux and screen are supported via `Session-Type` in `.bop/config`, but require more coordination for data directory isolation.
- The only required Zellij patch is respecting `ZELLIJ_DATA_DIR`. Upstreaming this is preferred over forking (see [SPEC.md § 10.5](SPEC.md#10-decisions)).

---

## 4. Why `work/` and not `attachments/`?

**Decision:** Name the primary work directory `work/`, not `attachments/`, `files/`, or `artifacts/`.

**Rationale:**
- `attachments/` implies passivity — files that accompany a task but aren't the point.
- `work/` makes the intent explicit: this is what the task produces. Source code, configs, scripts, Terraform — all live here.
- Agents modify files in `work/`. VCS tracks everything.
- The name is short, unambiguous, and doesn't collide with common tool conventions.

---

## 5. Why separate `evidence/` from `work/`?

**Decision:** Keep `evidence/` as a distinct directory, separate from `work/`.

**Rationale:**
- `work/` answers "what was produced." `evidence/` answers "how do we know it's correct."
- Screenshots, test results, and approval records are conceptually different from work products.
- Approvals can be nested `.bop` files — a sign-off is itself a task with state.
- Keeping them separate makes it easy for agents to scan just work products or just evidence without filtering.

---

## 6. Why OpenLineage?

**Decision:** Use OpenLineage (JSONL, append-only) for data lineage tracking in `.bop/lineage.jsonl`.

**Rationale:**
- OpenLineage is an open standard with broad ecosystem support: Marquez, Datahub, Amundsen, Airflow, dbt.
- JSONL (newline-delimited JSON) is append-only friendly — no need to parse or rewrite the full file on each event.
- `Produces` and `Consumes` headers in `task.bop` give agents a header-only view of the lineage graph without reading JSONL.
- Compatible with existing data catalog tooling out of the box.

---

## 7. Why OTel for agent traces?

**Decision:** Use OpenTelemetry-compatible JSONL spans in `traces/`.

**Rationale:**
- OTel is the emerging standard for distributed tracing. Direct ingest into Datadog, Jaeger, any OTel collector.
- Baggage (in `.bop/baggage`) propagates `Trace-Id` and other context down through task chains, enabling end-to-end trace correlation across agent handoffs.
- JSONL files in `traces/` are append-only and can be ingested incrementally.
- Separating lineage (what data flowed) from traces (what the agent did) mirrors standard observability practice.

---

## 8. Why `rename()` for state transitions?

**Decision:** Use POSIX `rename()` (via `mv` of a temp file) for atomic state updates to `.bop/state`.

**Rationale:**
- `rename()` is atomic on POSIX filesystems: observers either see the old state or the new state, never a partial write.
- This is the standard pattern for safe file updates in Unix (used by editors, package managers, etc.).
- Advisory locking via `.bop/lock` handles agent coordination at a higher level; `rename()` handles the low-level write safety.
- `.bop/state` is a fast-path index (O(1), <16 bytes). If it disagrees with `State:` in `task.bop`, `task.bop` wins — `state` is a cache, not the source of truth.
- Using `.bop/state` as a symlink was considered (see [SPEC.md § 10.1](SPEC.md#10-decisions)) but rejected: symlink semantics vary across filesystems, and `read` is simpler than `readlink`.

---

## 9. Why jj over git?

**Decision:** Prefer jj (`.jj/`) over git (`.git/`), with git as a fallback.

**Rationale:**
- jj has a cleaner model for working-copy changes and doesn't require staging. Auto-commit on state change is natural.
- jj is compatible with git remotes, so existing infrastructure still works.
- git is fully supported as a fallback — the spec doesn't mandate jj.
- For environments with neither, `history/` with timestamped copies provides a minimal audit trail.
- Large binaries in `work/` remain an open problem for both jj and git (see [SPEC.md § 10.8](SPEC.md#10-decisions)).

---

## 10. Why no binary fields?

**Decision:** No binary fields anywhere in the format. All data is plain text (UTF-8).

**Rationale:**
- Binary fields break grep, diff, and every text-based tool.
- Architecture-neutral: no endian or word-size assumptions.
- Large binaries in `work/` are handled by VCS (jj/git LFS) or left out of VCS via `.bopignore` — they don't enter the format itself.
- `Checksum-Body` is expressed as `algorithm:hex` (e.g., `sha256:abc123...`) — human-readable, not binary.

---

## 11. Why BCP 47 for language tags?

**Decision:** Use BCP 47 (`Language:` header) to declare the language of the task body.

**Rationale:**
- BCP 47 is the IETF standard for language tags, used by HTTP (`Accept-Language`), HTML (`lang=`), and most internationalization libraries.
- Header keys are always ASCII (protocol, not prose). Header values and body are UTF-8 — CJK, RTL, emoji all valid.
- Legacy encodings (Shift_JIS, Big5) are declared via `Content-Encoding` and agents SHOULD convert to UTF-8 on read.
- EBCDIC environments can transcode ASCII header keys mechanically; values are opaque UTF-8 bytes.

---

## 12. Portability Matrix

| Environment | Single-file | Bundle | Notes |
|---|---|---|---|
| macOS | ✅ | ✅ (opaque bundle) | Finder labels map to states |
| Linux | ✅ | ✅ (plain directory) | Full support |
| Windows | ✅ | ✅ (plain directory) | CRLF accepted on read |
| Email | ✅ | ❌ | Single-file is structurally RFC 822 |
| Obsidian | ✅ | ⚠️ | YAML frontmatter accepted on read |
| Taskwarrior | ✅ | ❌ | Bridge maps Task-Id↔UUID, State↔status |
| Make | ✅ | ✅ | `Depends-On` maps to prerequisites |
| grep/awk/sed | ✅ | ✅ | `grep "^State:" task.bop` works |