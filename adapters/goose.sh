#!/usr/bin/env bash
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

goose -p "$(cat "$prompt_file")" \
  --dangerously-skip-permissions \
  --output-format json \
  > "$stdout_log" 2> "$stderr_log"
rc=$?

if grep -qiE 'rate limit|429|too many requests' "$stderr_log"; then
  exit 75
fi

exit $rc
