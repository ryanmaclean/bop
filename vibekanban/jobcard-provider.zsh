#!/usr/bin/env zsh
# JobCard provider for vibekanban-cli.
# Outputs all job cards as a JSON array to stdout.
# Usage: CARDS_DIR=.cards ./vibekanban/jobcard-provider.zsh
setopt NULL_GLOB
set -euo pipefail

CARDS_DIR="${CARDS_DIR:-.cards}"

map_status() {
  case "$1" in
    pending) echo "pending"    ;;
    running) echo "in_progress";;
    done)    echo "review"     ;;
    merged)  echo "done"       ;;
    failed)  echo "blocked"    ;;
    *)       echo "unknown"    ;;
  esac
}

# Build JSON array using python3 (available on macOS, MIT license)
python3 - "$CARDS_DIR" <<'PYEOF'
import json, os, sys, pathlib

cards_dir = pathlib.Path(sys.argv[1])
tasks = []

state_map = {
    "pending": "pending",
    "running": "in_progress",
    "done":    "review",
    "merged":  "done",
    "failed":  "blocked",
}

for team_dir in sorted(cards_dir.iterdir()):
    if not team_dir.is_dir():
        continue
    team = team_dir.name
    for state, status in state_map.items():
        state_dir = team_dir / state
        if not state_dir.is_dir():
            continue
        for card_dir in sorted(state_dir.iterdir()):
            if card_dir.suffix != ".jobcard":
                continue
            meta_file = card_dir / "meta.json"
            if not meta_file.exists():
                continue
            try:
                meta = json.loads(meta_file.read_text())
            except Exception:
                continue
            card_id = meta.get("id", card_dir.stem)
            title   = meta.get("title", meta.get("id", card_dir.stem))
            tasks.append({
                "id":        card_id,
                "title":     title,
                "status":    status,
                "team":      team,
                "stage":     meta.get("stage", ""),
                "meta_path": str(meta_file),
            })

print(json.dumps(tasks, indent=2))
PYEOF
