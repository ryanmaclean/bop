# Main Plan (bop): Simple, Bomb-Proof Card Factory

This is the canonical operating plan for `main`.
If you are a new agent, start here before writing code.

## 0) Naming Lock (Do Not Re-Litigate)

- Product/repo name: `bop`
- Canonical CLI verb: `bop`
- Domain verbs:
  - `bop deal` = create/deal work cards
  - `bop bet` = estimate/prioritize cards (alias for `bop poker`)
- `bop poker open/submit/reveal/consensus` = plan poker flow (LANDED)

Reason: `bop` is short, memorable, and low-token-cost in prompts/logs.

## 1) Mission

Build a low-latency, low-overhead software factory where:
- filesystem cards are the source of truth,
- `bop` is the only writer of state transitions,
- UIs (Finder/Quick Look/vibekanban/zellij) are read and action surfaces,
- reliability and cost control beat feature sprawl.

Target behavior: Auto-Claude/Gas-Town style task manager with explicit stages:
`spec -> plan -> implement -> qa -> merged|failed`.

## 2) Non-Negotiables

- Keep architecture simple: no new control-plane database for v1.
- Keep one binary (`bop`) as the canonical command.
- Every transition is a folder move by `bop`, never by UI direct write.
- Every change must pass both:
  - `make check`
  - `bop policy check --staged` (or `scripts/policy_check.zsh --staged --cards-dir .cards`)
- No hidden magic: behavior must be visible in files.

## 3) 60-Second Start For Any Agent

Run this exact sequence:

```zsh
cargo build
./target/debug/bop doctor
./target/debug/bop status
./target/debug/bop inspect <card-id>
make check
./target/debug/bop policy check --staged
```

If no card is assigned, pick one from `.cards/pending/` and inspect it first.

## 4) Card Symbol Protocol (Mandatory)

Cards must carry a `glyph` in `meta.json` so priority/team are obvious instantly.

Team by suit:
- `spade` = CLI/runtime
- `heart` = architecture/decisions
- `diamond` = QA/reliability
- `club` = platform/integration

Priority by rank:
- `A` = P1
- `K/Q` = P2
- `J/N` = P3
- `10..2` = P4
- joker = incident/emergency

ASCII fallback for unicode-weak terminals:
- `S-A`, `H-A`, `D-K`, `C-7`, `JOKER`

Rule: if glyph is missing, add it before implementation work starts.

## 5) Universal Shortcut (Low-Lift, Works For Any Agent)

Agent working loop:

1. Inspect card and constraints (`meta.json`, `spec.md`, `decision.md` if required).
2. Work only inside declared scope.
3. Run gates (`make check`, policy check).
4. Let dispatcher/merge-gate move card state.

No agent should invent a new workflow when this loop already works.

## 6) Skill Strategy (How We Skill-Up Every Agent)

Use one shared repo skill contract so any model behaves like a task manager.

Required repo skills to add/maintain:
- `ace-of-hearts`: architecture/P1 mode, forces decision record quality.
- `implement-card`: scoped implementation with minimal churn.
- `qa-gatekeeper`: acceptance + policy + failure reason hygiene.
- `release-operator`: blue/green, canary, rollback and promotion gates.

Each skill must enforce:
- read card first,
- respect `policy_scope`,
- update only required files,
- pass local gates before done.

## 7) Codex CLI Integration (Keep It Thin)

Use `adapters/codex.zsh` as the execution bridge.
Do not couple control plane to Codex internals.
Adapter contract remains:

`adapter.sh <workdir> <prompt_file> <stdout_log> <stderr_log>`

Exit `75` means rate-limit and must trigger retry/failover logic.

## 8) MCP Policy (Optional, Not Required For Core)

MCPs are optional accelerators, not dependencies.

Use MCP only when needed for external systems:
- GitHub PR metadata/actions,
- observability and incident integrations,
- optional dashboards.

Do not make card lifecycle depend on MCP availability.
If MCP is down, filesystem workflow must still function.

## 9) User Surface Clarity

Keep user mental model obvious:
- Finder + Quick Look: primary visual board for card state.
- vibekanban-cli: board view over `.cards` with no alternate state store.
- zellij: operator console for dispatcher/merge-gate visibility.

Users should always see card symbols and state quickly without reading long docs.

## 10) Immediate Direction (Simplicity First)

DONE:
- [x] Plan poker (`bop poker open/submit/reveal/consensus`) with glyph-based estimation
- [x] `jc` → `bop` rename across CLI, docs, system_context
- [x] Duplicate file cleanup (templates/, ghost doc refs)

NEXT:
1. Enforce glyph presence/format in policy check.
2. Wire `bop deal` as alias for `bop new` (card-game verb consistency).
3. Prefer fixing reliability gaps over adding features.

## 12) Safe Main Landing While Agents Are Live (JJ-First)

Goal: avoid branch collisions and partial merges while multiple agents are actively writing code.

Operating model:
1. Every agent works in its own JJ workspace. No direct work on shared `main`.
2. One integrator workspace performs all landings to `main`.
3. Git is transport; JJ is coordination.

Fast safe-landing checklist:
1. In integrator workspace, pull latest and rebase/squash candidate changes.
2. Verify tree is clean (`git status --short` must be empty before landing).
3. Run hard gates:
   - `make check`
   - `./target/debug/bop policy check --staged`
4. If either gate fails, do not land. Route card back to `pending/` or `failed/` with reason.
5. Land smallest safe slice first (especially during renames/refactors).
6. Push from JJ to Git only after gates are green.

TRIZ rationale:
- Separate competing changes by workspace (segmentation).
- Use an integrator as intermediary for conflict control.
- Land only with immediate gate feedback, not intuition.

## 11) Definition Of "Good"

The system is good when:
- a new agent can start in under 2 minutes,
- card intent/priority is obvious from glyph + files,
- policy catches out-of-scope churn before merge,
- operational workflow keeps running without heroics.
