#!/usr/bin/env zsh
# bop_focus.zsh <card-id>
#
# Focuses a card across all 7 panes in the bop Zellij layout.
# Writes the card ID to /tmp/.bop_card (shared state), then
# sends commands to each named pane via zellij action.
#
# Usage (from the SHELL pane):
#   bop_focus.zsh feat-auth
#   bop_focus.zsh bop-test-the-dogfood-script
#
# Pane navigation shortcuts (no script needed):
#   Alt + h/j/k/l  — move focus between panes
#   Alt + n        — next pane
#   Ctrl + p, e    — enter pane mode, then arrows

set -euo pipefail
setopt NULL_GLOB

ROOT=${0:A:h:h}
BOP="${ROOT}/target/debug/bop"

if [[ $# -lt 1 ]]; then
  print "Usage: bop_focus.zsh <card-id>" >&2
  exit 1
fi

id="$1"

# ── Locate the card ────────────────────────────────────────────────────────────
card_dir=""
for state in running pending done merged failed; do
  candidate="${ROOT}/.cards/${state}/${id}.jobcard"
  [[ -d "$candidate" ]] && card_dir="$candidate" && break
done

if [[ -z "$card_dir" ]]; then
  print "ERROR: card '${id}' not found in any state" >&2
  print "Available cards:" >&2
  "${BOP}" status 2>/dev/null || true
  exit 1
fi

print "▶ Focusing: ${id}  (${card_dir:h:t}/)"

# Shared state — panes that watch this file will auto-update
print -n "${id}" > /tmp/.bop_card

# ── If inside Zellij: populate panes via zellij action ────────────────────────
if [[ -n "${ZELLIJ:-}" ]]; then
  # Helper: send a command string to a pane by navigating to it
  # Zellij 0.40+ supports 'zellij action write-chars'
  # We use 'zellij action focus-next-pane' to cycle through panes.
  # Pane order in bop.kdl (horizontal split, top to bottom):
  #   1. board (auto-refresh, skip)
  #   2. spec
  #   3. qa
  #   4. stdout
  #   5. stderr
  #   6. inspector
  #   7. shell (we are here)
  #
  # Strategy: write commands to /tmp/.bop_focus_<pane>.sh, which each
  # pane's shell sources if it finds the file. Simpler than pane targeting.

  spec_cmd="clear; echo '── SPEC: ${id} ──'; cat '${card_dir}/spec.md' 2>/dev/null || echo '(no spec.md)'"
  qa_cmd="clear; echo '── QA REPORT: ${id} ──'; cat '${card_dir}/output/qa_report.md' 2>/dev/null || echo '(no report yet)'; echo; echo '── ACCEPTANCE CRITERIA ──'; ${BOP} inspect ${id} 2>/dev/null | grep -A5 'acceptance' || true"
  stdout_cmd="clear; echo '── STDOUT: ${id} ──'; ${BOP} logs ${id} 2>/dev/null || cat '${card_dir}/logs/stdout.log' 2>/dev/null || echo '(no logs yet)'"
  stderr_cmd="clear; echo '── STDERR: ${id} ──'; cat '${card_dir}/logs/stderr.log' 2>/dev/null || echo '(no stderr yet)'"
  inspect_cmd="clear; ${BOP} inspect ${id} 2>/dev/null"

  print "${spec_cmd}"    > /tmp/.bop_spec.sh
  print "${qa_cmd}"      > /tmp/.bop_qa.sh
  print "${stdout_cmd}"  > /tmp/.bop_stdout.sh
  print "${stderr_cmd}"  > /tmp/.bop_stderr.sh
  print "${inspect_cmd}" > /tmp/.bop_inspector.sh

  print ""
  print "Commands written to /tmp/.bop_*.sh"
  print "Run these in each pane, or paste the commands below:"
fi

# ── Always: print commands to paste into each pane ────────────────────────────
print ""
print "── Paste into each pane: ───────────────────────────────────────────────"
print ""
print "  SPEC pane:      clear; cat '${card_dir}/spec.md'"
print "  STDOUT pane:    ${BOP} logs ${id} --follow"
print "  STDERR pane:    tail -f '${card_dir}/logs/stderr.log' 2>/dev/null || echo no stderr yet"
print "  QA pane:        watch -n3 \"cat '${card_dir}/output/qa_report.md' 2>/dev/null || echo pending\""
print "  INSPECTOR pane: watch -n3 '${BOP} inspect ${id}'"
print ""
print "  Or: Alt+arrow to move between panes in Zellij"
print ""

# ── Quick local summary ────────────────────────────────────────────────────────
"${BOP}" inspect "${id}" 2>/dev/null || true
