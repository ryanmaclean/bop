#!/usr/bin/env python3
"""JobCard provider implementation. Called by bop-provider.nu."""
import json
import os
import sys
import pathlib
import time

def elapsed_seconds(meta):
    """Compute elapsed seconds from meta: duration_s if present, else started_at to now."""
    if "duration_s" in meta:
        try:
            return float(meta["duration_s"])
        except (TypeError, ValueError):
            pass
    started = meta.get("started_at")
    if started:
        try:
            # ISO-8601 basic parse (Python 3.7+ fromisoformat)
            from datetime import datetime, timezone
            dt = datetime.fromisoformat(started.replace("Z", "+00:00"))
            return round(time.time() - dt.timestamp(), 1)
        except Exception:
            pass
    return None

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
        # Flat layout: .cards/merged/card.bop/ (no team level)
        state = entry.name
        status = state_map[state]
        for card_dir in sorted(entry.iterdir()):
            if card_dir.suffix != ".bop":
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
                "priority":  meta.get("priority", "P4"),
                "provider":  meta.get("provider", ""),
                "elapsed_s": elapsed_seconds(meta),
                "progress":  meta.get("progress"),
                "ac_spec_id": meta.get("ac_spec_id"),
                "meta_path": str(meta_file),
            })
    else:
        # Team layout: .cards/team-cli/pending/card.bop/
        team = entry.name
        for state, status in state_map.items():
            state_dir = entry / state
            if not state_dir.is_dir():
                continue
            for card_dir in sorted(state_dir.iterdir()):
                if card_dir.suffix != ".bop":
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
                    "priority":  meta.get("priority", "P4"),
                    "provider":  meta.get("provider", ""),
                    "elapsed_s": elapsed_seconds(meta),
                    "progress":  meta.get("progress"),
                    "ac_spec_id": meta.get("ac_spec_id"),
                    "meta_path": str(meta_file),
                })

print(json.dumps(tasks, indent=2))
