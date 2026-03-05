# What a Year of Agent Teams Teaches You

## The fundamental law
**Agents optimize for output, not outcomes.** They produce files, functions, and features — not working systems. The human's job is to define outcomes and constrain the path.

## Naming is architecture (not bikeshedding)
Every name that enters an agent's context window costs tokens AND causes drift. A repo name that collides with an existing domain causes hallucinations. A two-letter CLI name that collides with common tools wastes debugging cycles. A verbose reverse-DNS bundle ID burns 5 tokens of nothing. The right name is 1 token, works as a verb, has no collisions, and survives `grep`. This is why we settled on `bop`, `sh.bop.card`, `sh.bop.ql`.

## The three layers of agent steering
1. **Skills** — tell agents HOW to work (structured, versioned, invoked explicitly)
2. **CLAUDE.md** — constrain behavior (shell target, naming rules, architecture norms)
3. **system_context.md** — fix domain misidentification (prepended to every prompt)

Without all three, agents drift. Skills without CLAUDE.md = right method, wrong scope. CLAUDE.md without system_context = domain hallucination survives.

## The filesystem is the only reliable coordination mechanism
Not databases. Not APIs. Not shared in-process state. `mv` is atomic. `.cards/STATE/ID.bop/` is the state machine. Agents that write JSON to a file and move it are reliable. Agents that call an API are not.

## Scope creep is inevitable — make it cheap to undo
Every session adds REST APIs, dashboards, memory systems. This is not failure; it's physics. The answer is: `policy_check.nu --staged` at commit time + periodic trim cycles. TRIZ #2: extract the essential, delete the rest. Don't fight entropy, plan for it.

## The card is the atom
Not a ticket. Not a PR. Not a commit. The `.bop` bundle is the atomic unit of work. Everything else (branches, PRs, completions) is derived from it. This is the HyperCard insight applied to software factories.

## Token economy is first-class architecture
- 1-token name saves thousands of tokens/session across all agents
- `bop deal feat-auth` is 4 tokens. A verbose `new --template implement feat-auth` form is 8 tokens.  
- Repeat 10,000 times across a year = real money
- The glyph system (🂻 = Jack of Hearts = effort 13pt) encodes priority + team in 1 token

## Parallel agents need seams, not synchronization
When 3 agents wrote to the same functions in team-cli, you got overlap and contradiction. The fix is not synchronization — it's seams. Each agent gets a worktree (isolation), but all agents agree on the schema (coordination). lib.rs Meta struct is the seam.

## Phase gates prevent the #1 failure mode
Agents that jump to `implement` without `spec` → `plan` produce code that solves the wrong problem. The four-stage pipeline (spec → plan → implement → qa) is not ceremony — it's the only way to make agent output predictable.

## The joker rule is a forcing function
🃏 = "I cannot estimate this" = "break it down." One rule, no exceptions, surfaces scope problems before they become code problems.

## The dual-VCS contradiction (TRIZ in practice)
Problem: agents need isolated workspaces AND shared history. TRIZ #1+#13: segment the workspaces (worktrees), invert the push (agents work in branches, never on main). jj makes this reliable: undo is free, squash is clean, workspaces forget themselves.

## launchd is underrated infrastructure
KeepAlive: true + a dispatcher binary = GUPP (Gets Up, Picks up, Processes). The dispatcher crashes; launchd restarts it; it scans `running/` for orphans and reaps them. Zero k8s.

## The misidentification problem is permanent
Every new agent session risks interpreting a repo name as something unrelated -- a transit feed, a credit card system, a music app. system_context.md auto-prepended to every prompt is the only fix. The 150 tokens it costs are worth it.

## What Finder + Quick Look replaces
A web dashboard. Finder already exists. Quick Look already exists. The entire vibekanban/TUI/REST API scope creep exists because the team forgot that macOS already shipped a file browser in 1984. Build for the filesystem; get the UI free.

## The MICROCLAW principle
Small surface area. Sharp precision. Trustworthy at a glance. Every file you don't have is a file that can't cause problems. Every subcommand you don't ship is one that can't be misused by an agent. The minimum viable factory is: dispatcher, merge-gate, a format spec, and shell adapters. Everything else is optional.
