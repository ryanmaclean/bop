#!/usr/bin/env zsh
# JobCard provider for vibekanban-cli.
# Outputs all job cards as a JSON array to stdout.
# Usage: CARDS_DIR=.cards ./vibekanban/jobcard-provider.zsh
setopt NULL_GLOB
set -euo pipefail

CARDS_DIR="${CARDS_DIR:-.cards}"

[[ -d "$CARDS_DIR" ]] || { print -u2 "ERROR: CARDS_DIR '$CARDS_DIR' does not exist"; exit 1; }

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

STATE_NAMES = set(state_map.keys())

for entry in sorted(cards_dir.iterdir()):
    if not entry.is_dir():
        continue

    if entry.name in STATE_NAMES:
        # Flat layout: .cards/merged/card.jobcard/ (no team level)
        state = entry.name
        status = state_map[state]
        for card_dir in sorted(entry.iterdir()):
            if card_dir.suffix != ".jobcard":
                continue
            meta_file = card_dir / "meta.json"
            if not meta_file.exists():
                continue
            try:
                meta = json.loads(meta_file.read_text(encoding='utf-8', errors='replace'))
            except Exception:
                continue
            tasks.append({
                "id":        meta.get("id", card_dir.stem),
                "title":     meta.get("title", meta.get("id", card_dir.stem)),
                "status":    status,
                "team":      "root",
                "stage":     meta.get("stage", ""),
                "meta_path": str(meta_file),
            })
    else:
        # Team layout: .cards/team-cli/pending/card.jobcard/
        team = entry.name
        for state, status in state_map.items():
            state_dir = entry / state
            if not state_dir.is_dir():
                continue
            for card_dir in sorted(state_dir.iterdir()):
                if card_dir.suffix != ".jobcard":
                    continue
                meta_file = card_dir / "meta.json"
                if not meta_file.exists():
                    continue
                try:
                    meta = json.loads(meta_file.read_text(encoding='utf-8', errors='replace'))
                except Exception:
                    continue
                tasks.append({
                    "id":        meta.get("id", card_dir.stem),
                    "title":     meta.get("title", meta.get("id", card_dir.stem)),
                    "status":    status,
                    "team":      team,
                    "stage":     meta.get("stage", ""),
                    "meta_path": str(meta_file),
                })

print(json.dumps(tasks, indent=2))
PYEOF
