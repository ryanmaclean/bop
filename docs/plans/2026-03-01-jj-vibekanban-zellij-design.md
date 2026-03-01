# JobCard: jj + vibekanban + Zellij Integration Design

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:writing-plans then superpowers:subagent-driven-development to implement this design task-by-task.

**Goal:** Replace git worktrees with jj workspaces for per-card isolation, replace the ratatui TUI with vibekanban-cli as the primary UI (reading `.cards/` directly), and add full zellij integration with adaptive pane layout and a WASM status bar plugin.

**Architecture:** The `jc` dispatcher initializes a jj repo on startup, creates a `jj workspace` per card before dispatch, and the merge gate squashes changes then pushes stacked PRs via `jj git push`. vibekanban polls `.cards/` directly via filesystem. Zellij gets a native layout file, per-card pane hooks with adaptive tier logic (1–5 panes / 6–20 grouped / 21+ aggregated), and a Rust/WASM status bar plugin.

**License constraint:** All dependencies must be MIT, BSD, or Apache-2.0. Shell scripts use zsh (MIT). No bash, no fish (both GPL).

---

## Component Map

```
jj repo (.jj/)
  └── workspaces/
        └── .cards/team-xxx/running/card-abc/workspace/   ← per-card jj workspace

jc dispatcher (Rust)
  ├── jj workspace add <card>/workspace                   ← before adapter
  └── merge gate:
        ├── jj squash --from <workspace-change>           ← fold changes
        ├── jj workspace forget <workspace>               ← clean up
        ├── jj git push --change @                        ← push branch
        └── gh pr create --base <parent> --stack          ← stacked PR

UI:
  vibekanban-cli  ← polls .cards/ directly (no REST)
  zellij          ← layout + WASM plugin + per-card panes

Scripts:
  adapters/*.zsh         ← all rewritten from bash to zsh
  scripts/launch_teams.zsh
  scripts/dashboard.zsh
  zellij/jobcard.kdl     ← native layout
  zellij/plugin/         ← Rust/WASM status bar plugin
```

---

## Section 1: jj VCS Layer

### Decision
**jj-only** (no git worktrees). jj uses git as its storage backend (`.git/` exists internally) but all VCS operations go through `jj` CLI.

### jj Repo Initialization
Dispatcher checks for `.jj/` on startup. If absent, runs:
```zsh
jj git init --colocate   # creates .jj/ alongside existing .git/
jj config set --repo user.name "JobCard"
jj config set --repo user.email "jobcard@local"
```

### Per-Card Workspace Lifecycle
```
1. Dispatcher picks up card from pending/
2. mv card → running/
3. jj workspace add <card_dir>/workspace
   → creates an isolated workspace on a new anonymous change
4. Adapter runs: cd <card_dir>/workspace && <adapter> ...
5. Card completes → card_dir moved to done/
6. Merge gate picks up done/ card:
   a. jj squash --from <workspace-change> --into <main-change>
   b. jj workspace forget <card_dir>/workspace
   c. jj git push --change @   → pushes branch to remote
   d. gh pr create --base <parent-branch> --stack
   e. mv card → merged/
```

### Crash Recovery
If dispatcher restarts and finds `running/` cards:
- Check if `<card_dir>/workspace/` still exists in `jj workspace list`
- If yes: resume normally (jj workspace is persistent)
- If no: restore to `pending/` (jj has no partial state to clean)

### PR Stacking
Cards are squashed in completion order. Each card's jj change sits on top of the previous merged change, forming a natural stack. `jj git push --stack` pushes all un-pushed changes in the stack at once.

### Dependencies
- `jj` CLI (Apache 2.0) — system dependency, called via `std::process::Command`
- `gh` CLI (MIT) — for `gh pr create --stack`

---

## Section 2: vibekanban-cli Integration

### Decision
vibekanban polls `.cards/` directly via filesystem — no REST API layer.

### Card → Task Mapping

| `.cards/` state | vibekanban column |
|-----------------|-------------------|
| `pending/`      | Backlog           |
| `running/`      | In Progress       |
| `done/`         | Review            |
| `merged/`       | Done              |
| `failed/`       | Blocked           |

### JobCard Provider for vibekanban
Contribute a `jobcard` provider to vibekanban (or maintain a fork) that:
1. Reads `<cards-dir>/<team>/<state>/*.jobcard/meta.json` for task data
2. Reads `<card>/logs/stdout.log` for live output
3. Maps vibekanban actions to `jc` CLI commands:
   - Assign agent → `jc providers add`
   - Retry → `jc retry <id>`
   - Kill → `jc kill <id>`
   - View logs → `jc logs <id>`

### TUI Removal
Remove from `crates/jc/Cargo.toml`:
```toml
# REMOVE:
crossterm = "0.29"
ratatui = "0.29"
```
Remove from `main.rs`: `cmd_dashboard`, `draw_dashboard`, `handle_dashboard_key`.

---

## Section 3: Zellij Integration

### Three-Tier Adaptive Layout

Total active card count determines which tier is used:

**Tier 1: 1–5 cards** → one pane per card
```
┌─────────────────────┬────────────────────┐
│  ▶ card-abc (cli)   │  ▶ card-def (arch) │
│  [live stdout]      │  [live stdout]     │
├─────────────────────┴────────────────────┤
│  status bar: [cli:1▶] [arch:1▶] [q:0▶]  │
└──────────────────────────────────────────┘
```

**Tier 2: 6–20 cards** → one pane per team (max 5 panes), each showing latest card
```
┌────────────┬────────────┬────────────┐
│  team-cli  │ team-arch  │ team-qual  │
│  3 running │  2 running │  1 running │
│  [latest]  │  [latest]  │  [latest]  │
├────────────┴────────────┴────────────┤
│  status bar: total 6▶ / 4p / 2d     │
└──────────────────────────────────────┘
```

**Tier 3: 21–300 cards** → vibekanban web view in one pane + status bar only
```
┌──────────────────────────────────────┐
│  [vibekanban web UI - full pane]     │
│  localhost:3000 (npx vibe-kanban)    │
├──────────────────────────────────────┤
│  status bar: 47▶ / 23p / 180d       │
└──────────────────────────────────────┘
```

### Virtual Session Support
Dispatcher detects session type at startup:
```zsh
if [[ -n "$ZELLIJ" && -t 1 ]]; then
  ZELLIJ_MODE=interactive
elif [[ -n "$ZELLIJ" ]]; then
  ZELLIJ_MODE=virtual   # headless/piped
else
  ZELLIJ_MODE=none
fi
```
- `interactive`: full pane management (tiers 1–3)
- `virtual`: `zellij action` commands still work; panes created but may not be visible
- `none`: no zellij calls; cards run silently

### Deliverables

**`zellij/jobcard.kdl`** — native layout:
```kdl
layout {
  default_tab_template {
    children
    pane size=1 borderless=true {
      plugin location="file:zellij/plugin/jobcard.wasm"
    }
  }
  tab name="JobCard" {
    pane split_direction="vertical" {
      pane name="dispatcher" command="jc" {
        args "dispatcher" "--cards-dir" ".cards/team-cli"
      }
    }
  }
}
```

**`zellij/plugin/`** — Rust/WASM status bar plugin:
- Reads `.cards/` state via `watch_fs` (zellij plugin API)
- Renders: `[team:N▶] [team:N▶] ... | total: N▶ Np Nd`
- License: MIT (zellij plugin API is MIT, Rust stdlib is MIT/Apache)

**Dispatcher hooks (in `run_dispatcher`):**
```rust
if zellij_mode == ZellijMode::Interactive {
    let tier = compute_tier(active_count);
    match tier {
        Tier::PerCard => zellij_open_pane(card_id, &card_dir),
        Tier::PerTeam => zellij_update_team_pane(team_name),
        Tier::Aggregated => { /* status bar only */ }
    }
}
```

---

## Section 4: zsh Migration + License Audit

### Adapter Scripts (all → zsh)
```
adapters/claude.sh     → adapters/claude.zsh
adapters/codex.sh      → adapters/codex.zsh
adapters/goose.sh      → adapters/goose.zsh
adapters/aider.sh      → adapters/aider.zsh
adapters/opencode.sh   → adapters/opencode.zsh
adapters/ollama-local.sh → adapters/ollama-local.zsh
adapters/mock.sh       → adapters/mock.zsh
```
All shebang lines: `#!/usr/bin/env zsh`

### Utility Scripts
```
scripts/launch_teams.sh  → scripts/launch_teams.zsh
scripts/dashboard.sh     → scripts/dashboard.zsh
```

### Shell Completions
Generate zsh completion: `jc --generate-completion zsh > _jc`
Install to: `/usr/local/share/zsh/site-functions/_jc`

### License Audit Results

| Dependency | License | Status |
|------------|---------|--------|
| axum | MIT | ✓ |
| tokio | MIT | ✓ |
| serde / serde_json / serde_yaml | MIT/Apache | ✓ |
| clap | MIT/Apache | ✓ |
| anyhow | MIT/Apache | ✓ |
| chrono | MIT/Apache | ✓ |
| notify | MIT/Apache | ✓ |
| walkdir | MIT/Unlicense | ✓ |
| dirs | MIT/Apache | ✓ |
| async-stream | MIT | ✓ |
| utoipa | MIT/Apache | ✓ |
| tower / tower-http | MIT | ✓ |
| **ratatui** | MIT | **REMOVE** (replaced by vibekanban) |
| **crossterm** | MIT | **REMOVE** (replaced by vibekanban) |
| jj CLI | Apache 2.0 | ✓ system dep |
| gh CLI | MIT | ✓ system dep |
| vibekanban-cli | Apache 2.0 | ✓ system dep |
| zellij | MIT | ✓ system dep |
| zsh | MIT | ✓ shell |

No violations. No GPL dependencies.

---

## Implementation Order

| Priority | Task | Notes |
|----------|------|-------|
| 1 | zsh migration (adapters + scripts) | Unblocks everything |
| 2 | jj workspace integration in dispatcher | Replaces git worktree calls |
| 3 | jj merge gate (squash + push + PR) | Replaces git merge gate |
| 4 | Remove ratatui/crossterm | Requires jj gate working first |
| 5 | vibekanban provider (`.cards/` polling) | Standalone |
| 6 | Zellij layout + per-card pane hooks | Standalone |
| 7 | Zellij WASM status bar plugin | Standalone |
| 8 | Shell completions (zsh) | Low risk |
| 9 | License audit integration tests | CI gate |
