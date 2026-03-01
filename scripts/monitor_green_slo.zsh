#!/usr/bin/env zsh
set -euo pipefail

ROOT="/Users/studio/gtfs"
GREEN_DIR="${ROOT}/.cards-green"
OVERRIDE_FILE="${ROOT}/.cards/route.override"

WINDOW_MINUTES=${WINDOW_MINUTES:-30}
CHECK_INTERVAL_SEC=${CHECK_INTERVAL_SEC:-60}
MAX_FAILURE_RATE=${MAX_FAILURE_RATE:-0.20}
MIN_SAMPLE_SIZE=${MIN_SAMPLE_SIZE:-5}

required_breach_intervals=$(( (WINDOW_MINUTES * 60 + CHECK_INTERVAL_SEC - 1) / CHECK_INTERVAL_SEC ))
consecutive_breach=0

gather_window_metrics() {
  python3 - <<'PY' "$GREEN_DIR" "$WINDOW_MINUTES"
import json
import os
import pathlib
import sys
import time

root = pathlib.Path(sys.argv[1])
window_minutes = int(sys.argv[2])
cutoff = time.time() - (window_minutes * 60)

merged = 0
failed = 0
policy_violations = 0

for state in ("merged", "failed"):
    state_dir = root / state
    if not state_dir.exists():
        continue
    for card in state_dir.glob("*.jobcard"):
        try:
            mtime = card.stat().st_mtime
        except OSError:
            continue
        if mtime < cutoff:
            continue

        if state == "merged":
            merged += 1
        else:
            failed += 1
            meta_path = card / "meta.json"
            try:
                with meta_path.open("r", encoding="utf-8") as fh:
                    meta = json.load(fh)
                if meta.get("failure_reason") == "policy_violation":
                    policy_violations += 1
            except Exception:
                pass

total = merged + failed
failure_rate = (failed / total) if total else 0.0
print(f"{total} {merged} {failed} {policy_violations} {failure_rate:.6f}")
PY
}

while true; do
  read -r total merged failed policy_violations failure_rate <<<"$(gather_window_metrics)"

  is_breach=0
  awk_check=$(python3 - <<'PY' "$total" "$MIN_SAMPLE_SIZE" "$failure_rate" "$MAX_FAILURE_RATE"
import sys

total = int(sys.argv[1])
min_sample = int(sys.argv[2])
failure_rate = float(sys.argv[3])
max_failure_rate = float(sys.argv[4])
print("1" if total >= min_sample and failure_rate > max_failure_rate else "0")
PY
)
  if [[ "$awk_check" == "1" ]]; then
    is_breach=1
    consecutive_breach=$((consecutive_breach + 1))
  else
    consecutive_breach=0
  fi

  ts="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
  echo "[$ts] green-window total=$total merged=$merged failed=$failed policy_violations=$policy_violations failure_rate=$failure_rate breach=$is_breach consecutive=$consecutive_breach/$required_breach_intervals"

  if (( consecutive_breach >= required_breach_intervals )); then
    echo "blue-only" > "$OVERRIDE_FILE"
    echo "[$ts] rollback activated: wrote $OVERRIDE_FILE (blue-only)" >&2
    consecutive_breach=0
  fi

  sleep "$CHECK_INTERVAL_SEC"
done
