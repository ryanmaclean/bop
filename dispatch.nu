#!/usr/bin/env nu
# dispatch.nu — bop spec dispatcher
#
# Runs Auto-Claude specs in dependency-ordered waves with rate-limit-aware
# pacing. Spawns Zellij panes in the current session.
#
# Usage:
#   nu dispatch.nu plan              # Show full wave plan, no execution
#   nu dispatch.nu run --wave 0      # Execute only Wave 0
#   nu dispatch.nu run               # Execute all pending waves
#   nu dispatch.nu run --yes         # Skip confirmation prompts
#   nu dispatch.nu status            # Show what's done / pending / failed
#   nu dispatch.nu reset --spec 001  # Mark a spec as pending again
#   nu dispatch.nu mark-done 001     # Manually mark complete
#   nu dispatch.nu mark-failed 001   # Manually mark failed
#   nu dispatch.nu roadmap           # Spawn roadmap agent in Zellij pane
#   nu dispatch.nu ideate            # Spawn ideation agent in Zellij pane

const PYTHON      = "/Applications/Auto-Claude.app/Contents/Resources/python/bin/python3"
const SITE_PKGS   = "/Applications/Auto-Claude.app/Contents/Resources/python-site-packages"
const BACKEND     = "/Applications/Auto-Claude.app/Contents/Resources/backend"
const PROJECT_DIR = "/Users/studio/bop"
const STATE_FILE  = "/Users/studio/bop/.auto-claude/dispatch-state.json"
const LOCK_FILE   = "/Users/studio/bop/.auto-claude/dispatch-lock.json"

def zellij_session [] {
  if "ZELLIJ_SESSION_NAME" in $env {
    $env.ZELLIJ_SESSION_NAME
  } else {
    # Not inside Zellij — find the single active session, or error
    let sessions = (^zellij list-sessions --no-formatting 2>/dev/null | lines | where { |l| ($l | str trim) != "" })
    if ($sessions | length) == 1 {
      $sessions | first | str trim
    } else if ($sessions | length) > 1 {
      error make { msg: $"Multiple Zellij sessions active: ($sessions | str join ', '). Set ZELLIJ_SESSION_NAME or run from inside a session." }
    } else {
      error make { msg: "No Zellij session found. Start one with: zellij --session bop" }
    }
  }
}

# ---------------------------------------------------------------------------
# Spec table
# wave: 0=no-deps direct, 1=sequential, 2=parallel
# cost: 1=trivial 2=small 3=medium 4=complex
# mode: direct=writes to repo (no merge needed), isolated=worktree
# ---------------------------------------------------------------------------
def specs [] {
  [
    # ── Wave 0: create test script, no deps ─────────────────────────────────
    [id,    name,                                    wave, cost, mode    ];
    ["001", "create scripts/test-real-adapters.nu",  0,    3,    "direct"],

    # ── Wave 1: run each adapter e2e — sequential so fixes carry forward ────
    ["002", "ollama end-to-end adapter test",         1,    3,    "direct"],
    ["003", "claude end-to-end adapter test",         1,    4,    "direct"],
    ["004", "codex end-to-end adapter test",          1,    3,    "direct"],

    # ── Wave 2: team-cli P1 features (sequential — each builds on prev) ───────
    ["006", "job-control: retry/kill/logs commands",  2,    4,    "direct"],
    ["007", "bop clean command",                      2,    2,    "direct"],
    ["008", "shell completions (bash/zsh/fish)",      2,    2,    "direct"],

    # ── Wave 3: event-driven merge-gate (launchd/systemd.path, no daemon) ───
    # TRIZ #13 Inversion: OS calls bop on done/ change; bop exits after --once
    ["009", "event-driven merge-gate (launchd/systemd.path)", 3, 3, "direct"],

    # ── Wave 4: foundation hardening — fix before adding more features ───────
    ["010", "fix factory: KeepAlive → WatchPaths",     4, 2, "direct"],
    ["011", "fix CI: replace self-hosted with ubuntu",  4, 2, "direct"],
    ["012", "dogfood: seed bop's own backlog as cards", 4, 2, "direct"],

    # ── Wave 5: VM-first execution — QEMU adapter (dub + zam) ────────────────
    # Target: every card runs in an isolated VM, not on the host
    # dub: /Users/studio/efi — UEFI bootloader + TPM attestation
    # zam: /Users/studio/zam — <5MB unikernel, 9P card reader, UART output
    ["013", "QEMU adapter: VM-per-card execution",     5, 4, "direct"],

    # ── Wave 6: finish loose ends + bop serve smoke test ────────────────────
    # Previously run via codex exec; now via run.py (--direct) so Auto-Claude
    # subtask state is recorded. AC sees the already-committed work and closes out.
    ["014", "make check clean: fix clippy + finish install-hooks Linux",  6, 2, "direct"],
    ["015", "bop serve smoke test: curl POST creates card in pending/",   6, 3, "direct"],
    ["016", "resume spec-003 last subtask: claude e2e cleanup",           6, 2, "direct"],

    # ── Wave 8: laptop resilience — ACID safety + battery/wifi/cell ─────────
    # Fixes: atomic write_meta, crash recovery, pause/resume, sleep/wake,
    # network resilience, transient retry, live status watch.
    # Sequential: 022 (atomics) must land before 023 (pause uses write_meta),
    # 023 before 024 (sleep handler calls pause logic).
    ["022", "atomic write_meta + bop recover",                            8, 3, "direct"],
    ["023", "bop pause / resume / retry-transient",                       8, 3, "direct"],
    ["024", "sleep/wake awareness + network resilience",                  8, 4, "direct"],
    ["025", "bop status --watch (live in-place terminal view)",           8, 3, "direct"],
    ["026", "storage resilience: JSONL WAL + checksum + file perms",     8, 3, "direct"],

    # ── Wave 9: terminal card renderer + AC plan progress ────────────────────
    # 028 first: renderer trait + TermCaps are the surface 027 renders into
    ["028", "terminal card renderer: progressive degradation + demoscene",  9, 4, "direct"],

    ["029", "bop ui: full TUI (ratatui, 3-pane, fuzzy, log tail)",        9, 4, "direct"],

    # ── Wave 10: AC plan progress in Quick Look + CLI ────────────────────────
    # Depends on: 025 (status --watch reloads plan), 026 (ac_spec_id in Meta), 028 (renderer)
    ["027", "AC plan progress: Quick Look Plan tab + CLI block bars",   10, 3, "direct"],

    # ── Wave 11: provider quota monitor + BopDeck feed ───────────────────────
    # Research: Claude/Codex/Gemini OAuth APIs, Ollama /api/ps, opencode SSE.
    # Sequential: 030 (trait + Claude) → 031 (Codex + Gemini) → 032 (Ollama + opencode) → 033 (watch + BopDeck)
    ["030", "providers scaffold + Claude OAuth (5h/7d quota meters)",   11, 3, "direct"],
    ["031", "providers: Codex OAuth+RPC + Gemini OAuth+quota API",      11, 4, "direct"],
    ["032", "providers: Ollama local/cloud + opencode SSE",             11, 3, "codex"],
    ["033", "providers --watch + BopDeck socket feed + history JSONL",  11, 4, "codex"],
    ["034", "bop bridge: session state socket + Claude Code hooks",     11, 4, "codex"],

    # ── Wave 12: codex adapter hardening + job management ────────────────────
    # 035+037+038 parallel (no deps), 036 after 035, 039 after 037, 040 standalone
    ["035", "codex adapter: card rank → reasoning effort + --full-auto",  12, 2, "codex"],
    ["036", "per-card adapter routing in dispatcher",                     12, 3, "codex"],
    ["037", "merge-gate launchd install",                                 12, 2, "codex"],
    ["038", "dispatch.nu: spec cost → codex reasoning effort",            12, 1, "codex"],
    ["039", "factory status tab in bop ui TUI",                          12, 3, "codex"],
    ["040", "codex MCP per-project routing",                             12, 1, "codex"],

    # ── Wave 7: lightweight UX + Zellij live links + security ────────────────
    # Goals: working Zellij deep links in Quick Look, live log tail,
    # color-coded CLI, serve.rs security hardening, event-driven dispatcher.
    ["017", "Quick Look: Zellij live links + live log tail",              7, 4, "direct"],
    ["018", "CLI UX: color-coded states + summary stats + better errors", 7, 2, "direct"],
    ["019", "bop serve: security hardening (timing, auth, rate limit)",   7, 3, "direct"],
    ["020", "event-driven dispatcher: replace polling with notify",       7, 3, "direct"],
    ["021", "bop init: zero-config Zellij onboarding + bop doctor",      7, 2, "direct"],
  ]
}

def cooldown [cost: int] {
  match $cost {
    1 => 20
    2 => 40
    3 => 75
    4 => 120
    _ => 60
  }
}

# ---------------------------------------------------------------------------
# State helpers
# ---------------------------------------------------------------------------

def load_state [] {
  if ($STATE_FILE | path exists) {
    open $STATE_FILE
  } else {
    {completed: [], failed: [], skipped: []}
  }
}

def save_state [state: record] {
  $state | to json --indent 2 | save --force $STATE_FILE
}

def mark_done [spec_id: string] {
  let s = load_state
  let s2 = $s | update completed ($s.completed | append $spec_id | uniq)
  save_state $s2
}

def mark_failed [spec_id: string] {
  let s = load_state
  let s2 = $s | update failed ($s.failed | append $spec_id | uniq)
  save_state $s2
}

def write_lock [wave: int] {
  let ts = (date now | format date "%Y-%m-%dT%H:%M:%S")
  {pid: "dispatch", wave: $wave, started: $ts, session: (zellij_session)} | to json --indent 2 | save --force $LOCK_FILE
}

def clear_lock [] {
  if ($LOCK_FILE | path exists) { rm $LOCK_FILE }
}

# ---------------------------------------------------------------------------
# Pane helpers
# ---------------------------------------------------------------------------

def spec_shell_cmd [spec_id: string, flag: string] {
  $"PYTHONPATH=($SITE_PKGS) env -u CLAUDECODE ($PYTHON) ($BACKEND)/run.py --spec ($spec_id) --project-dir ($PROJECT_DIR) ($flag) && nu ($PROJECT_DIR)/dispatch.nu mark-done ($spec_id) || nu ($PROJECT_DIR)/dispatch.nu mark-failed ($spec_id)"
}

# Codex CLI dispatch — non-interactive exec using OpenAI OAuth from ~/.codex/auth.json
def codex_shell_cmd [spec_id: string, cost: int] {
  let effort = match $cost {
    1 => "low"
    2 => "medium"
    3 => "high"
    _ => "xhigh"
  }
  let base = $"($PROJECT_DIR)/.auto-claude/specs"
  let spec_dir = (ls $base | where name =~ $"/($spec_id)-" | get name | first)
  $"cd ($PROJECT_DIR) && env -u CLAUDECODE AC_PROJECT_DIR=($PROJECT_DIR) codex exec --full-auto -m gpt-5.3-codex -c model_reasoning_effort=($effort) -c 'mcp_servers.auto-codex.env.AC_PROJECT_DIR=\"($PROJECT_DIR)\"' - < ($spec_dir)/spec.md && /opt/homebrew/bin/nu ($PROJECT_DIR)/dispatch.nu mark-done ($spec_id) || /opt/homebrew/bin/nu ($PROJECT_DIR)/dispatch.nu mark-failed ($spec_id)"
}

def spawn_pane [name: string, cmd: string] {
  print $"  (ansi cyan)↗ spawning pane ($name)(ansi reset)"
  ^zellij --session (zellij_session) action go-to-tab-name "bop"
  sleep 300ms
  ^zellij --session (zellij_session) run --name $name --close-on-exit -- sh -c $cmd
}

def write_approval [spec_id: string] {
  let base = $"($PROJECT_DIR)/.auto-claude/specs"
  let found = (ls $base | where name =~ $"/($spec_id)-" | get name | first)
  let script = $"
import sys, json
from datetime import datetime
from pathlib import Path
sys.path.insert\(0, '($BACKEND)'\)
from review.state import _compute_spec_hash, REVIEW_STATE_FILE
spec_dir = Path\('($found)'\)
h = _compute_spec_hash\(spec_dir\)
state = \{'approved': True, 'approved_by': 'dispatch-operator', 'approved_at': datetime.now\(\).isoformat\(\), 'feedback': [], 'spec_hash': h, 'review_count': 1\}
\(spec_dir / REVIEW_STATE_FILE\).write_text\(json.dumps\(state, indent=2\)\)
print\(f'approved spec_hash=\{h\}'\)
"
  with-env {PYTHONPATH: $SITE_PKGS} { ^$PYTHON -c $script }
}

# ---------------------------------------------------------------------------
# Card ↔ Spec linkage (dogfood path)
# Search .cards/ state dirs for a card matching the spec, write ac_spec_id
# into its meta.json. Non-fatal — just prints a warning if nothing found.
# ---------------------------------------------------------------------------

def link_card_to_spec [spec_id: string] {
  let cards_root = $"($PROJECT_DIR)/.cards"
  if not ($cards_root | path exists) { return }

  # Derive slug from the spec directory name (e.g. "027-ac-progress-quicklook-cli" → "ac-progress-quicklook-cli")
  let base = $"($PROJECT_DIR)/.auto-claude/specs"
  let spec_dirs = (ls $base | where name =~ $"/($spec_id)-" | get name)
  if ($spec_dirs | length) == 0 { return }
  let spec_dir_name = ($spec_dirs | first | path basename)
  let slug = ($spec_dir_name | str replace $"($spec_id)-" "")

  # State dirs to search — flat + team-prefixed
  let state_names = ["pending", "running", "done", "failed", "merged"]
  mut found_meta: string = ""

  # Search flat state dirs: .cards/<state>/*.jobcard/meta.json
  for $st in $state_names {
    if $found_meta != "" { break }
    let state_dir = $"($cards_root)/($st)"
    if not ($state_dir | path exists) { continue }
    let cards = (try { ls $state_dir | where name =~ '\.jobcard$' } catch { [] })
    for $card in $cards {
      let meta_path = $"($card.name)/meta.json"
      if not ($meta_path | path exists) { continue }
      let meta = (try { open $meta_path } catch { null })
      if $meta == null { continue }
      let card_id = ($meta | get -o id | default "")
      let card_title = ($meta | get -o title | default "")
      if $card_id == $slug or ($card_id | str contains $spec_id) or ($card_title | str contains $spec_id) {
        $found_meta = $meta_path
        break
      }
    }
  }

  # Search team-prefixed state dirs: .cards/team-*/<state>/*.jobcard/meta.json
  if $found_meta == "" {
    let team_dirs = (try { ls $cards_root | where name =~ 'team-' | get name } catch { [] })
    for $td in $team_dirs {
      if $found_meta != "" { break }
      for $st in $state_names {
        if $found_meta != "" { break }
        let state_dir = $"($td)/($st)"
        if not ($state_dir | path exists) { continue }
        let cards = (try { ls $state_dir | where name =~ '\.jobcard$' } catch { [] })
        for $card in $cards {
          let meta_path = $"($card.name)/meta.json"
          if not ($meta_path | path exists) { continue }
          let meta = (try { open $meta_path } catch { null })
          if $meta == null { continue }
          let card_id = ($meta | get -o id | default "")
          let card_title = ($meta | get -o title | default "")
          if $card_id == $slug or ($card_id | str contains $spec_id) or ($card_title | str contains $spec_id) {
            $found_meta = $meta_path
            break
          }
        }
      }
    }
  }

  if $found_meta == "" {
    print $"  (ansi light_gray)⊘ no matching card for spec ($spec_id) — skipping ac_spec_id linkage(ansi reset)"
    return
  }

  # Read meta, add/update ac_spec_id, write back
  let meta_path = $found_meta
  try {
    let meta = (open $meta_path)
    let updated = ($meta | upsert ac_spec_id $spec_id)
    $updated | to json --indent 2 | save --force $meta_path
    print $"  (ansi green)⊕ linked ac_spec_id=($spec_id) → ($meta_path | path basename)(ansi reset)"
  } catch {
    print $"  (ansi yellow)⚠ failed to write ac_spec_id to ($meta_path)(ansi reset)"
  }
}

def wait_done [spec_id: string, timeout_min: int = 120] {
  let deadline = (date now) + ($timeout_min * 60sec)
  loop {
    let s = load_state
    if $spec_id in $s.completed { return "done" }
    if $spec_id in $s.failed    { return "failed" }
    if (date now) > $deadline   { return "timeout" }
    sleep 15sec
  }
}

def run_spec [spec_id: string, mode: string, cost: int, dry_run: bool] {
  if $dry_run {
    print $"  would spawn: (ansi yellow)zellij pane bop-($spec_id) — ($mode)(ansi reset)"
    return {ok: true}
  }

  let base = $"($PROJECT_DIR)/.auto-claude/specs"
  let found = (ls $base | where name =~ $"/($spec_id)-" | get name | first)
  let specfile = $"($found)/spec.md"
  print $"\n(ansi green_bold)── ($spec_id) ─────────────────────────────────────────(ansi reset)"
  print (open $specfile | str substring 0..400)
  print "..."

  write_approval $spec_id
  print $"  (ansi green)✓ approved ($spec_id)(ansi reset)"

  link_card_to_spec $spec_id

  let cmd = if $mode == "codex" {
    codex_shell_cmd $spec_id $cost
  } else {
    let flag = if $mode == "direct" { "--direct" } else { "--isolated" }
    spec_shell_cmd $spec_id $flag
  }
  spawn_pane $"bop-($spec_id)" $cmd

  print $"  (ansi light_gray)pane bop-($spec_id) running, polling...(ansi reset)"
  let outcome = wait_done $spec_id
  match $outcome {
    "done"    => {ok: true}
    "failed"  => {ok: false}
    "timeout" => {ok: false}
  }
}

# ---------------------------------------------------------------------------
# plan
# ---------------------------------------------------------------------------

def "main plan" [--wave: int = -1] {
  let all = specs
  let filtered = if $wave >= 0 { $all | where wave == $wave } else { $all }
  let state = load_state

  print $"\n(ansi green_bold)── bop Dispatch Plan ──────────────────────────────────(ansi reset)"
  print $"  State file: (ansi light_gray)($STATE_FILE)(ansi reset)"
  print $"  Specs: ($filtered | length) total\n"

  for $w in [0 1 2 3 4 5 6 7 8 9 10 11 12] {
    let wave_specs = $filtered | where wave == $w
    if ($wave_specs | length) == 0 { continue }

    let label = match $w {
      0 => "Wave 0  (no deps, --direct)"
      1 => "Wave 1  (sequential, --direct, each builds on prev)"
      2 => "Wave 2  (sequential, --direct, each builds on prev)"
      3 => "Wave 3  (sequential, --direct)"
      4 => "Wave 4  (parallel ok — independent hardening tasks)"
      5 => "Wave 5  (VM-first: QEMU adapter, dub EFI, zam unikernel)"
      _ => $"Wave ($w)"
    }
    print $"(ansi blue_bold)($label)(ansi reset)"

    for $s in $wave_specs {
      let marker = if ($s.id in $state.completed) {
        $"(ansi green)✓(ansi reset)"
      } else if ($s.id in $state.failed) {
        $"(ansi red)✗(ansi reset)"
      } else {
        $"(ansi light_gray)·(ansi reset)"
      }
      let cost_label = match $s.cost {
        1 => $"(ansi light_gray)trivial(ansi reset)"
        2 => $"(ansi light_gray)small  (ansi reset)"
        3 => $"(ansi yellow)medium (ansi reset)"
        4 => $"(ansi red)complex(ansi reset)"
        _ => "       "
      }
      print $"  ($marker) ($s.id)  ($cost_label)  ($s.name)"
    }
    print ""
  }
}

# ---------------------------------------------------------------------------
# status
# ---------------------------------------------------------------------------

def "main status" [] {
  let state = load_state
  let all = specs

  print $"\n(ansi green_bold)── bop Dispatch Status ─────────────────────────────────(ansi reset)"
  print $"  Completed : (ansi green)($state.completed | length)(ansi reset) / ($all | length)"
  print $"  Failed    : (ansi red)($state.failed | length)(ansi reset)"
  print ""
}

# ---------------------------------------------------------------------------
# run
# ---------------------------------------------------------------------------

def "main run" [
  --wave: int = -1
  --dry-run
  --yes
] {
  let all = specs
  let target_waves = if $wave >= 0 { [$wave] } else { [0 1 2 3 4 5 6 7 8 9 10 11] }

  if not $dry_run {
    main plan --wave $wave
    if not $yes {
      let confirm = (input $"(ansi yellow)Proceed? [y/N] (ansi reset)")
      if $confirm !~ "^[yY]" { print "Aborted."; return }
    }
  }

  for $w in $target_waves {
    let current_state = load_state
    let wave_specs = $all
      | where wave == $w
      | where { |s| not ($s.id in $current_state.completed) and not ($s.id in $current_state.failed) }

    if ($wave_specs | length) == 0 {
      print $"(ansi light_gray)Wave ($w): nothing to do.(ansi reset)"
      continue
    }

    if not $dry_run { write_lock $w }
    print $"\n(ansi blue_bold)━━ Wave ($w) ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━(ansi reset)"

    # All waves sequential for this project (tests must carry fixes forward)
    for $s in $wave_specs {
      print $"\n(ansi attr_bold)[($s.id)] ($s.name)(ansi reset)"
      let res = run_spec $s.id $s.mode $s.cost $dry_run
      if $res.ok {
        mark_done $s.id
        print $"(ansi green)✓ ($s.id) complete.(ansi reset)"
      } else {
        mark_failed $s.id
        print $"(ansi red)✗ ($s.id) failed.(ansi reset)"
        let cont = (input "(ansi yellow)Continue to next spec? [y/N] (ansi reset)")
        if $cont !~ "^[yY]" { break }
      }
      let wait = cooldown $s.cost
      if not $dry_run and $wait > 0 { sleep ($wait * 1sec) }
    }

    if not $dry_run { clear_lock }
    print $"\n(ansi green)Wave ($w) complete.(ansi reset)"
  }

  print $"\n(ansi green_bold)Dispatch complete.(ansi reset)"
  main status
}

# ---------------------------------------------------------------------------
# management commands
# ---------------------------------------------------------------------------

def "main reset" [--spec: string = ""] {
  let s = load_state
  if ($spec | is-empty) {
    save_state {completed: [], failed: [], skipped: []}
    print $"(ansi yellow)All specs reset to pending.(ansi reset)"
  } else {
    let s2 = $s
      | update completed ($s.completed | where { |id| $id != $spec })
      | update failed    ($s.failed    | where { |id| $id != $spec })
    save_state $s2
    print $"(ansi yellow)($spec) reset to pending.(ansi reset)"
  }
}

def "main retry" [] {
  let s = load_state
  let n = ($s.failed | length)
  if $n == 0 { print $"(ansi green)No failed specs.(ansi reset)"; return }
  save_state ($s | update failed [])
  print $"(ansi yellow)($n) failed spec(s) reset to pending.(ansi reset)"
}

def "main mark-done" [spec_id: string] {
  mark_done $spec_id
  print $"(ansi green)✓ ($spec_id) marked done(ansi reset)"
}

def "main mark-failed" [spec_id: string] {
  mark_failed $spec_id
  print $"(ansi red)✗ ($spec_id) marked failed(ansi reset)"
}

# ---------------------------------------------------------------------------
# Roadmap + ideation agents
# ---------------------------------------------------------------------------

def roadmap_cmd [] {
  $"cd ($PROJECT_DIR) && PYTHONPATH=($SITE_PKGS) ($PYTHON) ($BACKEND)/runners/roadmap_runner.py --project ($PROJECT_DIR) --thinking-level high --refresh; exec $env.SHELL"
}

def ideate_cmd [] {
  $"cd ($PROJECT_DIR) && PYTHONPATH=($SITE_PKGS) ($PYTHON) ($BACKEND)/runners/ideation_runner.py --project ($PROJECT_DIR) --types code_improvements,documentation_gaps,security_hardening,performance_optimizations,code_quality --thinking-level high --max-ideas 10 --refresh; exec $env.SHELL"
}

def "main roadmap" [] {
  print $"(ansi blue_bold)↗ Spawning bop-roadmap pane...(ansi reset)"
  ^zellij --session (zellij_session) run --name "bop-roadmap" --close-on-exit -- sh -c (roadmap_cmd)
  print $"(ansi green)✓ Roadmap → ($PROJECT_DIR)/.auto-claude/roadmap/(ansi reset)"
}

def "main ideate" [] {
  print $"(ansi blue_bold)↗ Spawning bop-ideate pane...(ansi reset)"
  ^zellij --session (zellij_session) run --name "bop-ideate" --close-on-exit -- sh -c (ideate_cmd)
  print $"(ansi green)✓ Ideation → ($PROJECT_DIR)/.auto-claude/ideation/(ansi reset)"
}

def "main agents" [] {
  print $"\n(ansi green_bold)━━ bop: launching roadmap + ideation ━━━━━━━━━━━━━━━━━━(ansi reset)"
  ^zellij --session (zellij_session) run --name "bop-roadmap" --close-on-exit -- sh -c (roadmap_cmd)
  ^zellij --session (zellij_session) run --name "bop-ideate"  --close-on-exit -- sh -c (ideate_cmd)
  print $"  roadmap  → ($PROJECT_DIR)/.auto-claude/roadmap/"
  print $"  ideation → ($PROJECT_DIR)/.auto-claude/ideation/"
}

# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------

def main [] {
  print $"(ansi green_bold)bop dispatch.nu(ansi reset)"
  print "Commands: plan | run | status | reset | retry | mark-done | mark-failed"
  print "          roadmap | ideate | agents"
  print ""
  print "Quick start:"
  print "  nu dispatch.nu plan          # see wave plan"
  print "  nu dispatch.nu run --wave 0  # run Wave 0 (spawns Zellij pane bop-001)"
  print "  nu dispatch.nu run --yes     # run all waves"
  print "  nu dispatch.nu status        # check progress"
}
