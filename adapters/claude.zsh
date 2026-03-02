#!/usr/bin/env zsh
set -euo pipefail

workdir="$1"; prompt_file="$2"; stdout_log="$3"; stderr_log="$4"

# Store original working directory
orig_dir="$(pwd)"

# Change to workdir for execution
cd "$workdir" || exit 1

# Convert paths relative to original directory
if [[ "$prompt_file" != /* ]]; then
    prompt_file="$orig_dir/$prompt_file"
fi
if [[ "$stdout_log" != /* ]]; then
    stdout_log="$orig_dir/$stdout_log"
fi
if [[ "$stderr_log" != /* ]]; then
    stderr_log="$orig_dir/$stderr_log"
fi

# Allow spawning claude from within a Claude Code session
unset CLAUDECODE

# Cap wall-clock time; card timeout_seconds is the authoritative limit but
# this prevents runaway sessions when the dispatcher timeout doesn't fire.
TIMEOUT_S="${5:-600}"

timeout "$TIMEOUT_S" claude -p "$(cat "$prompt_file")" \
  --dangerously-skip-permissions \
  --output-format json \
  > "$stdout_log" 2> "$stderr_log"
rc=$?

# timeout exits 124 on expiry — treat as transient so dispatcher retries
[[ $rc -eq 124 ]] && exit 75

if grep -qiE 'rate limit|429|too many requests' "$stderr_log"; then
  exit 75
fi

exit $rc
