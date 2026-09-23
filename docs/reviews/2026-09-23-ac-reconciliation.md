# Auto-Claude / dispatch.nu reconciliation — 2026-09-23

Scope: all 60 specs in `.auto-claude/specs/`. For each one: what `dispatch.nu` says,
what Auto-Claude (AC) says, whether its AC branch is merged, and whether the code
at `main` actually meets the spec's acceptance criteria.

Baseline: `main` = `086fec53` (2026-03-21, equal to `origin/main`). The `feat/mlx-adapter`
branch (2026-04-11) is **not** a descendant of `main` (merge-base `ae72b17`, 2026-03-08),
so it was not used as the base.

Method (proof over report): read each `spec.md`; grep the code for the named
files, flags and functions; run `./target/debug/bop <cmd> --help` and read-only
invocations in scratch dirs; run each adapter's `--test` self-check; one full
`cargo test --no-fail-fast` on `main` (**1171 passed, 0 failed, 1 ignored**);
`cargo clippy -- -D warnings` clean; `cargo fmt --check` clean. Four read-only
verifier agents did the per-spec checks and I spot-checked the high-impact claims
(claude.nu crash, adapter routing, merge-gate `--once`, 041 key bindings).

## Headline

- `dispatch-state.json` marks **60/60 completed** (the spec table in `dispatch.nu` has 59
  rows; `005` is completed but has no row). `dispatch.nu` marked a spec done when the
  agent process **exited 0** (`codex exec … && dispatch.nu mark-done`). Nothing checked
  the acceptance criteria.
- Only **26 of 60** specs fully meet their acceptance criteria at `main`.
  **31 are partial**, and **3 are not met at all** (004, 005, 016).
- The 19 specs that AC shows as `queue`/0 subtasks (035–037, 039–041, 045–056, 058) were
  never planned by AC. Their code landed directly on `main` through the "wave" commits
  `c9077c7` (waves 1–12), `d34d048` (waves 13–15) and `7b9013b` (wave 16), written by the
  codex/Claude adapters outside AC. None of them is wholly missing, but **8 of the 19 are
  only partial**: 035, 036, 037, 041, 050, 052, 054, 058.
- There are two serious runtime bugs that `cargo test` misses:
  1. `adapters/claude.nu` crashes under nu 0.115.1. `is_network_error` is declared
     `string -> bool` but is called without piped input. Every claude card that writes
     stderr exits non-zero (`nu adapters/claude.nu --test` fails with
     "no input value was piped in").
  2. The dispatcher picks the adapter from `provider_chain[0]`, but the provider from
     `select_provider`. When the chain is skipped or reordered (cooldown, quota, QA
     avoidance, cost tier), the wrong adapter runs. `ollama` maps to the missing
     `adapters/ollama-local.nu`, so it silently runs `claude.nu`. The `command` field
     in `providers.json` is ignored.

## Truth table

Columns: dispatch = `dispatch-state.json`; AC = Auto-Claude kanban state; branch =
`auto-claude/<spec>` vs `main` (`direct` = AC ran in the main checkout, no branch;
`none` = no AC branch; the code arrived via a wave commit). Verified = acceptance at
`main` before this reconciliation's fixes.

| spec | dispatch | AC | branch merged? | verified | missing / evidence |
|---|---|---|---|---|---|
| 001 create-test-real-adapters-script | completed | done 7/7 | direct | partial | `scripts/test-real-adapters.nu` exists, but its skip logic (`is_adapter_available`) is only used by `--test`. Missing tools FAIL instead of ⊘ skip. No "N passed/skipped/failed" summary. |
| 002 ollama-e2e-adapter-test | completed | done | direct | partial | The result.md fallback fix is in `adapters/ollama.nu`. No evidence of a passing live run (unverifiable-live). |
| 003 claude-e2e-adapter-test | completed | done | direct | partial | Generic prompt; no passing run recorded. See the claude.nu crash above. |
| 004 codex-e2e-adapter-test | completed | done | direct | **no** | The "graceful skip without OPENAI_API_KEY" check is never called by the run path, so the script exits 1. |
| 005 run-merge-gate | completed | done | direct | **no** | Operational spec. `/Users/studio/bop/.cards` still has 10 cards in `done/` plus 1 in each `team-*/done`. |
| 006 job-control-retry-kill-logs | completed | done | direct | partial | retry/kill/logs exist (20 tests ok). `paths::find_card` ignores `team-*/`. `logs --follow` never picks up a log file created after it starts. |
| 007 bop-clean-command | completed | done | direct | partial | Default age is 30d, not the spec's 7d. Orphan detection never checks the PID, so it can delete a running card with no logs yet. |
| 008 shell-completions | completed | done | direct | yes | bash/zsh/fish generate. |
| 009 event-driven-merge-gate | completed | done | direct | partial | Generated merge-gate plist/unit omits `--once`, so a WatchPaths trigger runs it as a daemon. No ThrottleInterval. `install/` templates unused. |
| 010 factory-watchpaths | completed | done | direct | partial | WatchPaths OK, but no ThrottleInterval, merge-gate lacks `--once`, and plist generation has no unit tests. |
| 011 fix-ci | completed | done | direct | partial | ci.yml is ubuntu + `make check`, but GitHub CI is red on `main` (see 044). |
| 012 dogfood-seed | completed | done | direct | partial | Only 3 of the 7 seed cards exist in live `.cards/pending`. |
| 013 qemu-adapter | completed | done | direct | partial | `qemu.nu` exists but diverged from the spec (cloud-init image, not zam). Exits 127, not 1. README is stale. |
| 014 make-check-clean-install-hooks-linux | completed | done | direct | partial | Linux merge-gate unit is `Type=oneshot` without `--once`, so `systemctl start` never returns. |
| 015 bop-serve-smoke-test | completed | done | direct | yes | `tests/serve_smoke.rs` (5 tests ok). |
| 016 resume-spec-003-claude-e2e-cleanup | completed | done | direct | **no** | The claude e2e run was never passed. `claude.nu --test` fails (crash above). |
| 017 quicklook-zellij-links-log-tail | completed | done | direct | yes | Swift typecheck clean; zellij session/pane written by dispatcher. |
| 018 cli-ux-colors-stats | completed | done | direct | yes | |
| 019 serve-security-hardening | completed | done | direct | yes | Minor: token printed to stdout; IDs accept Unicode. |
| 020 event-driven-dispatcher | completed | done | direct | partial | `--poll-ms` ignored. If the watcher fails, the loop spins hot. |
| 021 bop-init-zellij-onboarding | completed | done | direct | partial | doctor lacks Zellij, :8082 and claude CLI checks. `--fast` is a no-op. |
| 022 atomic-write-meta-crash-recovery | completed | done | direct | yes | Live `bop recover` verified. |
| 023 bop-pause-resume-retry-transient | completed | done | direct | partial | `bop pause <id>` pauses **every** running card. The dispatcher ignores `paused_at`. Pause/resume have no tests. |
| 024 sleep-wake-network-resilience | completed | done | direct | partial | IOKit watcher and `pause_all_running` are stubs. `providers.json` `probe` is never read. |
| 025 bop-status-watch | completed | done | direct | partial | No elapsed/progress for running cards. Stale lines are not cleared. |
| 026 storage-resilience-wal-events | completed | done | direct | yes | Docs say sha256; the code uses blake3. |
| 027 ac-progress-quicklook-cli | completed | done | direct | partial | `dispatch.nu link_card_to_spec` only scans `*.jobcard` (cards are `.bop` now) and writes the locked `meta.json` with `save`. |
| 028 terminal-card-renderer | completed | done | direct | partial | "Two-column" only halves box width. Unknown TERM falls back too low. |
| 029 bop-ui-tui | completed | done | direct | partial | nucleo is unused (substring filter). Sparkline static. F3 unbound. |
| 030 providers-scaffold-claude-oauth | completed | done | direct | partial | `--json` shape changed (`{"providers":[…]}`), so the spec's jq check fails. No token refresh. |
| 031 providers-codex-gemini | completed | done | direct | partial | No token refresh. "PTY" is piped stdin. The jq check fails (shape changed by 042). |
| 032 providers-ollama-opencode | completed | **ai_review / failed 2/6** | branch = main (work landed directly) | partial | opencode SSE `spawn_watch_task` is `#[allow(dead_code)]` and never wired into `--watch`. `--json` drops `loaded_models`. |
| 033 providers-watch-bopdeck | completed | human_review 6/6 | **no** (5 ahead: tests only) | yes | Feature on main; the branch adds upsert/sort/render tests. |
| 034 bop-bridge-session-state | completed | human_review 7/7 | **no** (6 ahead: `bop-bridge.nu --test`) | yes | 14 bridge tests ok; listen/emit roundtrip ok. |
| 035 codex-adapter-rank-effort | completed | queue 0/0 | none | partial | `codex.nu` reads `$workdir/meta.json`, but meta lives in the card dir, so effort always defaults to `high` when a workspace exists. |
| 036 per-card-adapter-routing | completed | queue 0/0 | none | partial | Adapter is not taken from the selected provider. Empty chain runs mock.nu, not the global `--adapter`. Unknown provider requeues forever. `ollama-local.nu`/`gemini.nu` missing. |
| 037 merge-gate-launchd-install | completed | queue 0/0 | none | partial | install/start/stop/uninstall loop both labels, but the merge-gate plist/unit has no `--once` and no `--vcs-engine`. No factory unit tests. Live install is unverifiable (not run). |
| 038 dispatch-cost-effort | completed | **in_progress / planning 0/0** | branch = main | yes | `codex_shell_cmd` gives cost 1 → `low` and cost 4 → `xhigh`, with `--full-auto`. `plan --wave 12`/`status` OK. No dry-run command printer (spec's optional verification aid). |
| 039 factory-tui-integration | completed | queue 0/0 | none | yes | |
| 040 codex-mcp-per-project | completed | queue 0/0 | none | yes | |
| 041 bop-ui-ux-polish | completed | queue 0/0 | none | partial | Shift+L was repurposed to the log pane (049); move-right is `Shift+.`. The filter is a substring filter. |
| 042 providers-live-quota | completed | human_review 6/6 | branch = main (nothing to merge) | yes (code) | Live quota fetch is unverifiable (local creds expired). |
| 043 gantt-html-polish | completed | done 7/7 | **no** (5 ahead / 11 behind) | yes | Features are on main; the branch adds only tests plus scratch files (`HEATMAP_VERIFICATION.md`, `verify_tooltip.sh`, `.cards/test-gantt-responsive.html`). |
| 044 ci-workflow-fix | completed | done 4/4 | yes (PR #4) | partial | GitHub CI red on `main` (086fec5 and the 2026-05-06 run). |
| 045 bop-doctor-hardening | completed | queue 0/0 | none | yes | 5 categories, exit 1 on error, `--fix` test ok. |
| 046 bop-new-interactive | completed | queue 0/0 | none | yes | |
| 047 bop-watch-live-dashboard | completed | queue 0/0 | none | yes | |
| 048 bop-stats-cost-report | completed | queue 0/0 | none | yes | |
| 049 bop-ui-log-stream | completed | queue 0/0 | none | yes | |
| 050 qemu-adapter-skeleton | completed | queue 0/0 | none | partial | Guest never mounts the 9P `/card`, so `vm_exit_code` can't reach the host. |
| 051 bop-export-share | completed | queue 0/0 | none | yes | Live round trip ok. |
| 052 provider-auto-select | completed | queue 0/0 | none | partial | Selection is logged but not acted on (adapter from `provider_chain[0]`; see 036). |
| 053 bop-diff | completed | queue 0/0 | none | yes | |
| 054 qemu-alpine-agent | completed | queue 0/0 | none | partial | No 9P mount in guest. Seed lacks `cidata` label. No aarch64 firmware. |
| 055 bop-replay | completed | queue 0/0 | none | yes | |
| 056 bop-ui-card-detail | completed | queue 0/0 | none | yes | |
| 057 webhook-notifications | completed | done 7/7 | yes (PR #2) | yes | |
| 058 qemu-vm-pool | completed | queue 0/0 | none | partial | `pool_inject` is a no-op, so pooled VMs never run the card. |
| 059 multi-project | completed | human_review 6/6 | **no** (2 ahead: test clippy fixes) | yes | |
| 060 bop-benchmark | completed | **in_progress / coding 5/6** | branch = main (landed via `7b9013b`) | yes | All 9 bullets verified live with mock adapters. The remaining AC subtask was "document verification". Minor: results filename collides within one second; judge parser takes the first `{"scores":…}` (the prompt's example when an adapter echoes it); `$inf/point` shown. |

Totals: **yes 26 · partial 31 · no 3**. All 60 were marked completed.

## Unmerged AC branches

| branch | ahead/behind main | content |
|---|---|---|
| auto-claude/033-providers-watch-bopdeck | 5 / 0 | unit tests for `upsert_snapshot`, `sort_snapshots`, `render_snapshots`; AC plan files |
| auto-claude/034-bop-bridge-session-state | 6 / 0 | `vibekanban/bop-bridge.nu` `--test` self-check; AC plan files |
| auto-claude/042-providers-live-quota | 0 / 0 | nothing (implemented directly on main) |
| auto-claude/059-multi-project | 2 / 0 | clippy fixes in `dispatcher.rs` test code (`--all-targets` lints) |
| auto-claude/043-gantt-html-polish | 5 / 11 | tests + scratch artefacts; not in the human_review set, not merged |
| auto-claude/032, 038, 060 | 0 / 0 | branches still point at `main` |
