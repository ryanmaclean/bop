#!/usr/bin/env zsh
set -euo pipefail
setopt NULL_GLOB

ROOT=${0:A:h:h}
BOP="${ROOT}/target/debug/bop"

if [[ $# -lt 1 ]]; then
  print "Usage: bop_bop.zsh <goal description>" >&2
  exit 1
fi

goal="$*"

# Slugify goal → card id
id="${goal:l}"           # lowercase
id="${id// /-}"          # spaces → hyphens
id="${id//[^a-z0-9-]/}" # strip non-url chars
id="${id:0:40}"          # max 40 chars
id="bop-${id}"

session="bop-${id}"

print "▶ Creating card: ${id}"
"${BOP}" new implement "${id}" 2>/dev/null || true

# Locate the card
card_dir=""
for state in pending running done; do
  candidate="${ROOT}/.cards/${state}/${id}.jobcard"
  [[ -d "$candidate" ]] && card_dir="$candidate" && break
done

if [[ -z "$card_dir" ]]; then
  print "ERROR: card not found after creation" >&2
  exit 1
fi

# Write goal into spec.md
print "# ${goal}\n\nCreated by bop_bop.zsh." >> "${card_dir}/spec.md"

# Write zellij_session into meta.json (use python3 for safe JSON merge)
python3 - "${card_dir}/meta.json" "${session}" <<'PY'
import json, sys
path, session = sys.argv[1], sys.argv[2]
meta = json.loads(open(path).read())
meta["zellij_session"] = session
meta["zellij_pane"] = "1"
open(path, "w").write(json.dumps(meta, indent=2, ensure_ascii=False))
print(f"  zellij_session: {session}")
PY

# Budget-aware agent routing
# Cost order: ollama (free) → opencode ($) → codex ($) → claude ($$)
PROVIDER="mock"
ADAPTER="${ROOT}/adapters/mock.zsh"

if command -v ollama &>/dev/null; then
  PROVIDER="ollama"; ADAPTER="${ROOT}/adapters/ollama-local.zsh"
fi
if command -v opencode &>/dev/null; then
  PROVIDER="opencode"; ADAPTER="${ROOT}/adapters/opencode.zsh"
fi
if command -v codex &>/dev/null; then
  PROVIDER="codex"; ADAPTER="${ROOT}/adapters/codex.zsh"
fi
if command -v claude &>/dev/null; then
  PROVIDER="claude"; ADAPTER="${ROOT}/adapters/claude.zsh"
fi

print "  provider: ${PROVIDER}"
print "  session:  ${session}"
print ""
print "▶ bop://card/${id}/session"
print ""

# Create JJ workspace if jj is available and repo is initialized
if command -v jj &>/dev/null && jj root --repository "${ROOT}" &>/dev/null 2>&1; then
  worktree_dir="${ROOT}/.worktrees/${id}"
  jj workspace add "${worktree_dir}" 2>/dev/null || true
  python3 - "${card_dir}/meta.json" "${id}" "${worktree_dir}" <<'PY'
import json, sys
path, branch, ws = sys.argv[1], sys.argv[2], sys.argv[3]
meta = json.loads(open(path).read())
meta["worktree_branch"] = f"job/{branch}"
meta["workspace_path"] = ws
open(path, "w").write(json.dumps(meta, indent=2, ensure_ascii=False))
PY
fi

# Launch or attach resumable zellij session with dispatcher inside
if command -v zellij &>/dev/null; then
  zellij attach "${session}" 2>/dev/null || \
    zellij -s "${session}" -- \
      "${BOP}" dispatcher \
        --adapter "${ADAPTER}" \
        --once
else
  # No zellij: run dispatcher inline
  "${BOP}" dispatcher --adapter "${ADAPTER}" --once
fi
