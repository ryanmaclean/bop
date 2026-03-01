#!/usr/bin/env bash
set -euo pipefail

workdir="$1"; prompt_file="$2"; stdout_log="$3"; stderr_log="$4"
orig_dir="$(pwd)"
cd "$workdir" || exit 1

[[ "$prompt_file" != /* ]] && prompt_file="$orig_dir/$prompt_file"
[[ "$stdout_log"  != /* ]] && stdout_log="$orig_dir/$stdout_log"
[[ "$stderr_log"  != /* ]] && stderr_log="$orig_dir/$stderr_log"

codex exec \
  --dangerously-bypass-approvals-and-sandbox \
  -c 'sandbox_permissions=["disk-full-read-access","disk-write-access"]' \
  "$(cat "$prompt_file")" \
  > "$stdout_log" 2> "$stderr_log"
rc=$?

grep -qiE 'rate.?limit|429|too many requests' "$stderr_log" && exit 75
exit $rc
