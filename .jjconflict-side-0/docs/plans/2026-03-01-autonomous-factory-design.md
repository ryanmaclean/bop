# Autonomous Factory Design

Date: 2026-03-01
Status: Approved

## Problem

bop has the bones of a software factory (dispatcher, merge-gate, adapters, cards)
but cannot run autonomously. Cards don't progress through stages automatically,
agents start with no context about bop, and there's no "press go" button.

## Design

### Context Window Optimization

Every token in the agent prompt earns its slot:

```
[system_context.md]     ~150 tokens  — domain fix, system identity
[stages/<stage>.md]     ~50 tokens   — stage-specific behavior
[card header]           ~20 tokens   — glyph + id + constraints
[spec.md]               variable     — the work
[prior_stage_output]    variable     — output from previous stage
```

The dispatcher builds the prompt by concatenating these layers, writes to
`prompt.md`, and passes it to the adapter. The glyph system compresses
priority+team into 1 token (e.g. 🂡 = P1/CLI).

### Stage Instruction Files

`.cards/stages/` contains one markdown file per stage:

- `spec.md` — produce a specification, be concise
- `plan.md` — given the spec, produce an implementation plan
- `implement.md` — implement the plan, write code, run tests
- `qa.md` — review implementation, run acceptance criteria

Each is ~50 tokens of dense instruction.

### Template Strategy System

Templates encode the factory recipe. Three shipping templates:

**implement** (default, lean):
- stage_chain: [implement, qa]
- Routes implement→opus/claude, qa→sonnet/claude

**full** (complete pipeline):
- stage_chain: [spec, plan, implement, qa]
- Routes spec→haiku/ollama, plan→sonnet/claude, implement→opus/claude, qa→sonnet/codex

**cheap** (all-local, zero cost):
- stage_chain: [implement]
- Routes implement→codellama:7b/ollama

Templates configure three strategy dimensions:
1. `stage_models` — model tier per stage (token economy)
2. `stage_providers` — adapter per stage (provider routing)
3. `stage_budgets` — max_tokens per stage (cost control)

### Stage Auto-Progression

When a card finishes its current stage (exit 0):
1. Dispatcher checks `stage_chain` in meta.json
2. If next stage exists → writes `output/cards.yaml` with child card
3. Child inherits: spec, glyph, constraints + prior stage output
4. If final stage → card goes to done/ for merge-gate

### MCP Config (Layered)

```
.cards/mcp.json           — global MCP servers
<card>/mcp.json           — per-card overrides (optional)
```

Claude adapter merges both and passes --mcp-config. Others ignore it.

### Zellij Pane per Card

Dispatcher spawns adapter in a named Zellij pane:
```
zellij action new-pane --name "bop:${card_id}:${stage}"
```
Pane is scrollable, resumable. Tier system: 0-5 cards = individual panes,
6-20 = team panes, 21+ = status bar only.

### bop doctor — Adapter Health

Checks each adapter's CLI binary availability. Reports which adapters are
functional vs missing. Warns if providers.json references unavailable adapters.

### bop factory — launchd Lifecycle

```
bop factory install   — generate plists, load into launchd
bop factory start     — launchctl start both daemons
bop factory stop      — launchctl stop both
bop factory status    — running? PID? last log lines
bop factory uninstall — unload + remove plists
```

Labels: sh.bop.dispatcher, sh.bop.merge-gate. KeepAlive: true.

### Dispatcher Scans Team Directories

Extends pending scan to include .cards/team-*/pending/ alongside flat
.cards/pending/.

## Implementation Order

1. Stage files + context injection (smallest, highest value)
2. bop doctor adapter checks
3. Template stage_chain/stage_models/stage_providers in dispatcher
4. bop factory launchd management
5. Zellij pane spawning (partially exists)

## Not Included (MICROCLAW)

- No REST API, web UI, new crates, new dependencies, TUI dashboard
- Everything in main.rs + filesystem artifacts
