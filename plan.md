# Main Plan (bop): Simple, Bomb-Proof Card Factory

This is the canonical operating plan for `main`.
If you are a new agent, start here before writing code.

---

## 0) Naming Lock (Do Not Re-Litigate)

| Role | Name |
|------|------|
| CLI binary | **`bop`** |
| Quick Look bundle | `sh.bop.ql` |
| Host app bundle | `sh.bop` |
| Card UTI | `sh.bop.card` |
| launchd labels | `sh.bop.dispatcher`, `sh.bop.merge-gate` |

Domain verbs: `bop new`, `bop status`, `bop inspect`, `bop poker`, `bop kill`, `bop approve`.

## 1) Mission

A multi-agent software factory where:
- filesystem cards are the source of truth,
- `bop` is the only writer of state transitions,
- UIs (Finder/Quick Look/Zellij) are read+action surfaces,
- reliability and cost control beat feature sprawl.

Target: Auto-Claude/Gas-Town style manager with explicit stages:
`spec → plan → implement → qa → merged|failed`.

## 2) Non-Negotiables

- No new control-plane database for v1.
- One binary (`bop`) is the canonical command.
- Every transition is a folder move by `bop`, never by UI.
- Every landing passes: `make check` + `bop policy check --staged`.
- No hidden magic: behavior is visible in files.

## 3) 60-Second Start For Any Agent

```zsh
cargo build
./target/debug/bop doctor
./target/debug/bop status
./target/debug/bop inspect <card-id>
make check
```

No card assigned? Pick one from `.cards/pending/` and inspect it first.

## 4) Card Symbol Protocol

Cards carry a `glyph` in `meta.json` — priority+team in 1 token.

| Suit | Team | BMP Token |
|------|------|-----------|
| ♠ Spade | CLI/runtime | ♠ |
| ♥ Heart | Architecture | ♥ |
| ♦ Diamond | QA/reliability | ♦ |
| ♣ Club | Platform | ♣ |

| Rank | Priority |
|------|----------|
| Ace | P1 |
| King/Queen | P2 |
| Jack | P3 |
| 2–10 | P4 |
| Joker | Emergency |

Rule: if glyph is missing, add it before work starts.

## 5) Agent Working Loop

1. Inspect card (`meta.json`, `spec.md`).
2. Work only inside declared scope.
3. Run gates (`make check`).
4. Let dispatcher/merge-gate move card state.

No agent invents a new workflow when this loop already works.

---

## 6) What's Built (Current State)

### By tender-vaughan (factory engine layer)

| Artifact | Location | Purpose |
|----------|----------|---------|
| Stage instruction files | `.cards/stages/{spec,plan,implement,qa}.md` | ~50 tokens each, injected into agent prompts per stage |
| Template: implement | `.cards/templates/implement.jobcard/` | 2-stage (implement→qa), opus→sonnet |
| Template: full | `.cards/templates/full.jobcard/` | 4-stage pipeline, tiered models |
| Template: cheap | `.cards/templates/cheap.jobcard/` | 1-stage, ollama-local only, 600s timeout |
| Template: qa-only | `.cards/templates/qa-only.jobcard/` | Review/audit, no implementation |
| Factory design doc | `docs/plans/2026-03-01-autonomous-factory-design.md` | Full architecture for stage auto-progression |
| land_safe.zsh | `scripts/land_safe.zsh` | JJ/git safe-landing script |
| RunLease + dispatcher lock | `crates/jc/src/main.rs` | Filesystem lease heartbeat, stale lock reclaim |

### By Windsurf (UX + reliability layer)

| Artifact | Location | Purpose |
|----------|----------|---------|
| Zellij Meta fields | `crates/jobcard-core/src/lib.rs` | `zellij_session`, `zellij_pane` on Meta struct |
| 7-pane Zellij layout | `layouts/bop.kdl` | Board/spec/qa/stdout/stderr/inspector/shell |
| bop_focus.zsh | `scripts/bop_focus.zsh` | Pane navigator, `--auto` sweeps all |
| bop_bop.zsh | `scripts/bop_bop.zsh` | Goal→card→zellij session bootstrap |
| Format examples | `examples/{pending,running,done}-feat.jobcard/` | What cards look like at each lifecycle state |
| bop:// URL scheme | `macos/JobCardHost/` | Quick Look "Attach" → Zellij session |
| Quick Look redesign | `macos/bop/PreviewViewController.swift` | Stage pipeline view, full palette |
| Log colorization | `crates/jc/src/main.rs` | Tailspin-style color for `bop logs` |
| Dispatcher harness | `crates/jc/tests/dispatcher_harness.rs` | Integration test scaffold |
| Maintenance scripts | `scripts/` | Thumbnail refresh, cold card compression |

---

## 7) The Bridge (ALL LANDED 2026-03-01)

All gaps between the two agents' work have been implemented. §7a–7f complete.

### 7a) Meta struct factory fields ✅

`stage_chain`, `stage_models`, `stage_providers`, `stage_budgets` added to Meta.
Serde defaults + skip_serializing_if. Two round-trip tests.

Add to `crates/jobcard-core/src/lib.rs` → `Meta`:

```rust
/// Ordered stage pipeline. Example: ["implement", "qa"]
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub stage_chain: Vec<String>,

/// Model tier per stage. Example: {"implement": "opus", "qa": "sonnet"}
#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
pub stage_models: BTreeMap<String, String>,

/// Adapter per stage. Example: {"implement": "claude", "qa": "codex"}
#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
pub stage_providers: BTreeMap<String, String>,

/// Max token budget per stage. Example: {"implement": 32000, "qa": 8000}
#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
pub stage_budgets: BTreeMap<String, u64>,
```

**Effort:** Small. **Files:** `lib.rs` only. **Status:** DONE.

### 7b) render_prompt template variables ✅

All four now substitute:

| Variable | Source |
|----------|--------|
| `{{stage_instructions}}` | `.cards/stages/<stage>.md` |
| `{{stage_index}}` | Position in `stage_chain` (e.g. "2") |
| `{{stage_count}}` | Length of `stage_chain` (e.g. "4") |
| `{{prior_stage_output}}` | Previous card's `output/result.md` |

**Effort:** Small. **Files:** `lib.rs` only. **Status:** DONE.

### 7c) Dispatcher stage auto-progression ✅

`maybe_advance_stage()` in `main.rs`: when a card succeeds and `stage_chain`
has a next stage, creates child card in `pending/` inheriting spec, glyph,
pipeline config, and prior stage output. **Status:** DONE.

### 7d) Format examples updated ✅

`examples/{pending,running,done}-feat.jobcard` now include `stage_chain`,
`stage_models`, `stage_providers`, `stage_budgets`, `token`, `timeout_seconds`.
**Status:** DONE.

### 7e) bop doctor adapter checks ✅

`cmd_doctor` now checks: adapter CLI binaries (claude, codex, ollama, goose,
aider, opencode), stages/ directory, templates/ count, system_context.md, zellij.
**Status:** DONE.

### 7f) bop factory launchd lifecycle ✅

`bop factory install/start/stop/status/uninstall` generates plists dynamically
from repo root with labels `sh.bop.dispatcher`, `sh.bop.merge-gate`. Includes
PATH with `~/.cargo/bin`, correct CARDS_DIR, and log paths at `/tmp/bop-*.log`.

**Effort:** Medium. **Files:** `main.rs`, `launchd/README.md`. **Status:** DONE.

---

## 8) How to Create Your Own Template

Templates live in `.cards/templates/`. Each is a `.jobcard/` directory bundle.

### Step 1: Copy an existing template

```zsh
cp -c -R .cards/templates/implement.jobcard .cards/templates/my-thing.jobcard
```

(`-c` = APFS clone on macOS, zero-cost copy)

### Step 2: Edit meta.json

This is the only file you MUST customize. Annotated reference:

```jsonc
{
  // ── Identity (bop new fills these) ────────────────────
  "id": "REPLACE_ID",
  "created": null,
  "glyph": null,          // playing card, set after poker or manually
  "token": null,           // BMP symbol for terminals/filenames

  // ── Stage Pipeline (the factory recipe) ───────────────
  "stage": "implement",   // starting stage (first in chain)
  "stage_chain": ["implement", "qa"],

  // ── Model Routing (which model per stage) ─────────────
  // Options: "opus", "sonnet", "haiku", "local"
  "stage_models": {
    "implement": "opus",
    "qa": "sonnet"
  },

  // ── Provider Routing (which adapter per stage) ────────
  // Must match a filename in adapters/ (minus .zsh)
  "stage_providers": {
    "implement": "claude",
    "qa": "claude"
  },

  // ── Budget (max tokens per stage) ─────────────────────
  "stage_budgets": {
    "implement": 32000,
    "qa": 8000
  },

  // ── Failover (if primary hits exit 75) ────────────────
  "provider_chain": ["claude", "codex", "ollama-local"],

  // ── Gates (all must exit 0 to merge) ──────────────────
  "acceptance_criteria": [
    "cargo test",
    "cargo clippy -- -D warnings"
  ],

  // ── Limits ────────────────────────────────────────────
  "timeout_seconds": 3600,
  "worktree_branch": "job/REPLACE_ID"
}
```

### Step 3: Edit spec.md

Write what this template is for. `bop new` copies it; the human fills in real requirements.

```markdown
# My Template — Example Spec

> When to use this template and what it does.

## Overview
What needs to be done.

## Acceptance Criteria
- [ ] What "done" looks like
```

### Step 4: prompt.md (usually leave as-is)

The default works for most templates:

```
{{system_context}}
---
{{stage_instructions}}
---
Card: {{id}} {{glyph}}
Stage: {{stage}} ({{stage_index}} of {{stage_count}})
---
{{spec}}
{{prior_stage_output}}
Acceptance criteria:
{{acceptance_criteria}}
```

Only customize if you need extra context injection or a different structure.

### Step 5: Use it

```zsh
bop new my-thing feat-auth
# Edit .cards/pending/feat-auth.jobcard/spec.md
bop dispatcher --once
```

### Shipped Templates

| Template | Stages | Models | Cost | Use When |
|----------|--------|--------|------|----------|
| **implement** | implement → qa | opus → sonnet | $$ | Requirements are clear |
| **full** | spec → plan → implement → qa | sonnet → sonnet → opus → sonnet | $$$ | Complex/ambiguous work |
| **cheap** | implement | ollama-local | free | Small fix, local model |
| **qa-only** | qa | sonnet | $ | Code review, audit |

### Ideas for Custom Templates

| Name | Chain | Why |
|------|-------|-----|
| `hotfix` | `[implement]` claude/opus, 900s | Fast cloud fix, trust gates |
| `research` | `[spec]` sonnet | Spec only, human reviews |
| `duo` | `[implement, qa]` codex→claude | Cheap impl, expensive review |
| `gauntlet` | `[implement, qa, qa]` claude→codex→ollama | Triple-reviewed |

---

## 9) Stage Instruction Files

Live in `.cards/stages/`. One per stage. ~50 tokens each.

| File | Agent's job |
|------|------------|
| `spec.md` | Write a spec under 500 words. No code. |
| `plan.md` | Read spec, produce ordered steps under 800 words. |
| `implement.md` | Write code, run tests, exit 0 when green. |
| `qa.md` | Review as a different agent. Be skeptical. |

**Adding a custom stage:**
1. Create `.cards/stages/security-review.md`
2. Add `"security-review"` to your template's `stage_chain`
3. Add entries in `stage_models`, `stage_providers`, `stage_budgets`

---

## 10) Context Window Architecture

Every token earns its slot:

```
┌─────────────────────────────────────────┐
│ system_context.md        ~150 tokens    │  domain fix
├─────────────────────────────────────────┤
│ stages/<stage>.md        ~50 tokens     │  stage behavior
├─────────────────────────────────────────┤
│ card header              ~20 tokens     │  id + glyph + stage
├─────────────────────────────────────────┤
│ spec.md                  variable       │  the work
├─────────────────────────────────────────┤
│ prior_stage_output       variable       │  previous stage result
├─────────────────────────────────────────┤
│ acceptance_criteria      ~10 tokens     │  shell gates
└─────────────────────────────────────────┘
```

Glyph compresses priority+team into 1 token: 🂡 = Ace of Spades = P1/CLI.

---

## 11) Zellij Integration

| Artifact | What |
|----------|------|
| `layouts/bop.kdl` | 7-pane layout: board, spec, qa, stdout, stderr, inspector, shell |
| `scripts/bop_focus.zsh` | Navigate panes by card id, `--auto` sweeps all |
| `scripts/bop_bop.zsh` | Goal→card→Zellij session bootstrap |
| `bop://` URL scheme | Quick Look "Attach" button opens Zellij session |
| Meta fields | `zellij_session`, `zellij_pane` track running card's pane |

Launch: `zellij --layout layouts/bop.kdl`

---

## 12) Safe Landing (JJ-First)

1. Every agent works in its own workspace. Never touch `main`.
2. One integrator workspace lands to `main`.
3. Gates: `make check` + `bop policy check --staged`.
4. If gates fail → card to `failed/` with reason.
5. Push JJ→Git only after green.

Script: `scripts/land_safe.zsh`

---

## 13) Adapter Contract

```
adapter.zsh <workdir> <prompt_file> <stdout_log> <stderr_log> [timeout]
```

| Exit | Meaning | Action |
|------|---------|--------|
| 0 | Success | → done/ |
| 75 | Rate-limited | → pending/, rotate provider |
| other | Failure | → failed/ |

Adapters: `claude.zsh`, `codex.zsh`, `ollama-local.zsh`, `goose.zsh`, `aider.zsh`, `opencode.zsh`, `mock.zsh`.

---

## 14) MCP Policy

MCPs are optional accelerators, not dependencies.
Card lifecycle must work without MCP.
Use for: GitHub PR actions, observability, dashboards.

---

## 15) Definition of Done

The system is good when:
- A new agent starts in under 2 minutes.
- Card intent is obvious from glyph + spec (no docs needed).
- Users create templates by copying + editing one JSON file.
- Stage auto-progression works end to end.
- `bop factory start` runs unattended.
