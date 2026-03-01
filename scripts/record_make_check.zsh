#!/usr/bin/env zsh
set -euo pipefail

ROOT="/Users/studio/gtfs"
LOG_FILE="${ROOT}/.cards/promotion/make_check_runs.log"
mkdir -p "$(dirname "$LOG_FILE")"

status="pass"
if ! make -C "$ROOT" check; then
  status="fail"
fi

ts="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
echo "$status" >> "$LOG_FILE"
echo "$ts $status" >> "${LOG_FILE}.audit"

echo "Recorded make check result: $status"

if [[ "$status" != "pass" ]]; then
  exit 1
fi
