#!/usr/bin/env zsh
set -euo pipefail

ROOT="/Users/studio/gtfs"
WINDOW_MINUTES=$((48 * 60))
CHECK_HISTORY_FILE="${ROOT}/.cards/promotion/make_check_runs.log"
INCIDENTS_DIR="${ROOT}/.cards/incidents/critical"

metrics_json="$("${ROOT}/scripts/lane_metrics.zsh" --window-minutes "$WINDOW_MINUTES" --output json)"

python3 - <<'PY' "$metrics_json" "$CHECK_HISTORY_FILE" "$INCIDENTS_DIR"
import json
import pathlib
import sys

metrics = json.loads(sys.argv[1])
history_file = pathlib.Path(sys.argv[2])
incidents_dir = pathlib.Path(sys.argv[3])

failures = []

# Gate 1: make check green for 5 consecutive runs.
gate1_ok = False
if history_file.exists():
    lines = [ln.strip().lower() for ln in history_file.read_text(encoding="utf-8").splitlines() if ln.strip()]
    last5 = lines[-5:]
    gate1_ok = len(last5) == 5 and all(x == "pass" for x in last5)
if not gate1_ok:
    failures.append("Gate 1 failed: need 5 consecutive 'pass' entries in .cards/promotion/make_check_runs.log")

# Gate 2: policy violation rate = 0 for 48h (enforced for both lanes).
blue_pvr = float(metrics.get("blue", {}).get("policy_violation_rate", 1.0))
green_pvr = float(metrics.get("green", {}).get("policy_violation_rate", 1.0))
if blue_pvr != 0.0 or green_pvr != 0.0:
    failures.append(
        f"Gate 2 failed: policy_violation_rate must be 0.0 (blue={blue_pvr:.6f}, green={green_pvr:.6f})"
    )

# Gate 3: green success rate >= blue success rate over same window.
blue_sr = float(metrics.get("blue", {}).get("success_rate", 0.0))
green_sr = float(metrics.get("green", {}).get("success_rate", 0.0))
if green_sr < blue_sr:
    failures.append(
        f"Gate 3 failed: green success_rate {green_sr:.6f} is below blue {blue_sr:.6f}"
    )

# Gate 4: no unresolved critical incidents.
open_incidents = []
if incidents_dir.exists() and incidents_dir.is_dir():
    open_incidents = [p.name for p in incidents_dir.iterdir() if p.is_file()]
if open_incidents:
    failures.append(
        "Gate 4 failed: unresolved critical incidents present: " + ", ".join(sorted(open_incidents))
    )

print("Promotion gate report (48h window):")
print(f"- Gate 1 (5x make check pass): {'PASS' if gate1_ok else 'FAIL'}")
print(f"- Gate 2 (policy violation rate 0): {'PASS' if blue_pvr == 0.0 and green_pvr == 0.0 else 'FAIL'}")
print(f"- Gate 3 (green success >= blue): {'PASS' if green_sr >= blue_sr else 'FAIL'}")
print(f"- Gate 4 (no critical incidents): {'PASS' if not open_incidents else 'FAIL'}")

if failures:
    print("\nFAILED GATES:")
    for item in failures:
        print(f"- {item}")
    sys.exit(1)

print("\nALL GATES PASS: eligible for big-bang cutover to green.")
PY
