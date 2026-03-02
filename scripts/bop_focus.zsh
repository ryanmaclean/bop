#!/usr/bin/env zsh
# bop_focus.zsh [--auto] [--test] <card-id>
#
# Focus a card across all 7 panes in the bop Zellij layout.
#
# Modes:
#   bop_focus.zsh <id>          — print paste commands for each pane
#   bop_focus.zsh --auto <id>   — sweep all panes and type commands in (Zellij only)
#   bop_focus.zsh --test        — identify each pane by writing its name into it
#
# Pane order in bop.kdl (depth-first, layout traversal):
#   1. board     (skip — auto-refreshing watch loop)
#   2. spec      ← SPEC: what to build
#   3. qa        ← QA:   did it work?
#   4. stdout    ← STDOUT: agent live output
#   5. stderr    ← STDERR: agent errors
#   6. inspector ← INSPECTOR: bop inspect metadata
#   7. shell     ← current pane (you run this from here)
#
# If traversal order differs on your setup, tune:
#   BOP_PANE_SKIP=1    (panes to skip before spec; default 1 for board)
#   BOP_PANE_COUNT=7   (total pane count including shell; default 7)
#
# Alt+arrows move focus between panes manually.

set -euo pipefail
setopt NULL_GLOB

ROOT=${0:A:h:h}
BOP="${ROOT}/target/debug/bop"

# ── Args ──────────────────────────────────────────────────────────────────────
AUTO=false
TEST=false
ID=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --auto) AUTO=true; shift ;;
    --test) TEST=true; shift ;;
    -*) print "Unknown flag: $1" >&2; exit 1 ;;
    *) ID="$1"; shift ;;
  esac
done

if [[ "$TEST" == false && -z "$ID" ]]; then
  print "Usage: bop_focus.zsh [--auto] [--test] <card-id>" >&2
  exit 1
fi

# ── Zellij pane automation ────────────────────────────────────────────────────
# Configurable hop counts (override with env vars if your layout order differs)
: "${BOP_PANE_SKIP:=1}"    # panes before spec (board = 1)
: "${BOP_PANE_COUNT:=7}"   # total panes in layout

# Send a command to the currently focused pane, then execute it (Enter)
_send() {
  local cmd="$1"
  zellij action write-chars "$cmd"
  zellij action write 13    # carriage return
}

# Interrupt anything running in the focused pane, then send a fresh command
_replace() {
  local cmd="$1"
  zellij action write 3     # Ctrl+C — interrupt tail -f / watch
  sleep 0.05
  zellij action write-chars "$cmd"
  zellij action write 13
}

# Advance focus by N panes
_hop() {
  local n="${1:-1}"
  local i
  for i in $(seq 1 "$n"); do
    zellij action focus-next-pane
    sleep 0.05
  done
}

# ── Test mode: write pane number into each pane ───────────────────────────────
if [[ "$TEST" == true ]]; then
  if [[ -z "${ZELLIJ:-}" ]]; then
    print "ERROR: --test requires running inside a Zellij session" >&2
    exit 1
  fi
  print "▶ Testing pane traversal from shell (pane 7)..."
  print "  Watch for 'PANE N' appearing in each pane."
  print ""
  # Start from shell (pane 7), cycle all 6 other panes
  for i in 1 2 3 4 5 6; do
    zellij action focus-next-pane
    sleep 0.1
    _send "echo 'PANE ${i} of 6'"
  done
  # Return to shell
  zellij action focus-next-pane
  print "Done. Expected: board=1, spec=2, qa=3, stdout=4, stderr=5, inspector=6"
  print "If order differs, set BOP_PANE_SKIP / reorder layout"
  exit 0
fi

# ── Locate the card ───────────────────────────────────────────────────────────
card_dir=""
for state in running pending done merged failed; do
  candidate="${ROOT}/.cards/${state}/${id}.jobcard"
  [[ -d "$candidate" ]] && card_dir="$candidate" && break
done

if [[ -z "$card_dir" ]]; then
  print "ERROR: card '${ID}' not found in any state" >&2
  "${BOP}" status 2>/dev/null || true
  exit 1
fi

id="$ID"
print "▶ Focusing: ${id}  (${card_dir:h:t}/)"
print -n "${id}" > /tmp/.bop_card

# ── Build per-pane commands ───────────────────────────────────────────────────
cmd_spec="clear; printf '\\033[1;36m── SPEC: ${id} ──\\033[0m\\n'; cat '${card_dir}/spec.md' 2>/dev/null || echo '(no spec.md)'"
cmd_qa="clear; printf '\\033[1;32m── QA: ${id} ──\\033[0m\\n'; cat '${card_dir}/output/qa_report.md' 2>/dev/null || echo '(pending — criteria: '; cat '${card_dir}/meta.json' 2>/dev/null | python3 -c 'import json,sys; [print(c) for c in json.load(sys.stdin).get(\"acceptance_criteria\",[])]' 2>/dev/null; echo ')'"
cmd_stdout="clear; printf '\\033[1;33m── STDOUT: ${id} ──\\033[0m\\n'; tail -f '${card_dir}/logs/stdout.log' 2>/dev/null || echo '(no stdout yet)'"
cmd_stderr="clear; printf '\\033[1;31m── STDERR: ${id} ──\\033[0m\\n'; tail -f '${card_dir}/logs/stderr.log' 2>/dev/null || echo '(no stderr yet)'"
cmd_inspect="clear; watch -n3 '${BOP} inspect ${id} 2>/dev/null || echo card not found'"

# ── Auto mode: sweep panes via zellij action ──────────────────────────────────
if [[ "$AUTO" == true ]]; then
  if [[ -z "${ZELLIJ:-}" ]]; then
    print "ERROR: --auto requires running inside a Zellij session (bop.kdl layout)" >&2
    print "Launch with: zellij --layout layouts/bop.kdl" >&2
    exit 1
  fi

  print ""
  print "  Sweeping panes (run --test first if order looks wrong)..."
  print ""

  # Step 1: skip board pane(s)
  _hop "$BOP_PANE_SKIP"

  # Step 2: spec pane
  zellij action focus-next-pane; sleep 0.1
  _replace "$cmd_spec"
  sleep 0.15

  # Step 3: qa pane
  zellij action focus-next-pane; sleep 0.1
  _replace "$cmd_qa"
  sleep 0.15

  # Step 4: stdout pane
  zellij action focus-next-pane; sleep 0.1
  _replace "$cmd_stdout"
  sleep 0.15

  # Step 5: stderr pane
  zellij action focus-next-pane; sleep 0.1
  _replace "$cmd_stderr"
  sleep 0.15

  # Step 6: inspector pane
  zellij action focus-next-pane; sleep 0.1
  _replace "$cmd_inspect"
  sleep 0.15

  # Return to shell (pane 7)
  zellij action focus-next-pane
  sleep 0.1

  print "✓ All panes updated for: ${id}"
  print "  SPEC → QA → STDOUT → STDERR → INSPECTOR"
  print ""
  print "  Alt+arrows to navigate  |  Ctrl+C to stop tail/watch in a pane"
  exit 0
fi

# ── Manual mode: print paste commands ────────────────────────────────────────
print ""
print "── Paste into each pane (or run with --auto): ──────────────────────────"
print ""
print "  SPEC pane:"
print "    ${cmd_spec}"
print ""
print "  QA pane:"
print "    ${cmd_qa}"
print ""
print "  STDOUT pane:"
print "    ${cmd_stdout}"
print ""
print "  STDERR pane:"
print "    ${cmd_stderr}"
print ""
print "  INSPECTOR pane:"
print "    ${cmd_inspect}"
print ""
print "  Alt+arrows to navigate between panes"
print ""
"${BOP}" inspect "${id}" 2>/dev/null || true
