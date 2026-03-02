# macOS File-Agent Workflow — Planning Card Done

## What was done

1. **Plan document** — `docs/plans/2026-03-01-macos-file-agent-workflow.md` exists
   and is complete. Contains:
   - Architecture diagram (event source → dispatcher → adapter → merge-gate)
   - launchd service topology (`com.yourorg.bop.dispatcher` + `merge-gate`)
   - Quick Look URL scheme routing (`jobcard://stop/<id>`, `jobcard://approve/<id>`)
   - FSEvents primary path + 1s polling fallback
   - SLOs (pickup <100ms, idle CPU <1%, control action median <1s)
   - Phased rollout (4 phases with explicit acceptance gates)

2. **Rename hardening** — audited all active execution paths:
   - `.cards/team-arch/providers.json` — already repo-relative (`adapters/*.zsh`)
   - `.cards/team-cli/providers.json` — already repo-relative
   - `.cards/team-platform/providers.json` — already repo-relative
   - `launchd/com.yourorg.jobcard.merge-gate.plist` — **fixed**: stale
     `/Users/studio/gtfs/.cards` replaced with `REPLACE_WITH_REPO_ROOT/.cards`

3. **Acceptance criteria** — all three pass from workspace root:
   - `test -f docs/plans/...` ✓
   - `rg -n "jobcard://...|FSEvents|launchd|fallback" ...` ✓ (all terms present)
   - `! rg -n "/Users/studio/gtfs/adapters" .cards/team-*/providers.json` ✓ (no matches)

## Phased Next Actions

**Phase 1 — Rename Integrity (complete for active configs)**
- [x] Team provider configs use repo-relative adapter paths
- [x] launchd plists use `REPLACE_WITH_REPO_ROOT` placeholder (not hardcoded paths)
- [ ] `launchd/README.md` install script to substitute placeholder at deploy time

**Phase 2 — Control Surface (next card)**
- Implement `bop kill <id>` and `bop approve <id>` subcommands in `crates/jc/`
- Add URL scheme handling in `JobCardHost.app` (`jobcard://` → shell dispatch)
- QL preview: surface action links only when card is in `running/` state

**Phase 3 — Event Loop (after Phase 2)**
- FSEvents integration in dispatcher (replace/augment polling loop)
- Measure pickup latency on event path vs 1s poll
- Validate idle CPU stays <1% with empty queue

**Phase 4 — Memory Injection (last)**
- `.cards/memory/<namespace>.json` store
- Prompt template `{{memory}}` substitution in `render_prompt`
- TTL + stale-eviction semantics
