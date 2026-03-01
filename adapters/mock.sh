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

if [[ "${MOCK_SLEEP:-}" != "" ]]; then
  sleep "${MOCK_SLEEP}"
fi

{
  echo "mock adapter"
  echo "workdir=$workdir"
  echo "prompt_file=$prompt_file"
  echo "--- prompt ---"
  cat "$prompt_file" || true
  echo "-------------"
} >> "$stdout_log"

{
  echo "mock stderr" 1>&2
  if [[ "${MOCK_STDERR_TEXT:-}" != "" ]]; then
    echo "${MOCK_STDERR_TEXT}" 1>&2
  fi
} 2>> "$stderr_log" || true

if [[ "${MOCK_STDOUT_TEXT:-}" != "" ]]; then
  echo "${MOCK_STDOUT_TEXT}" >> "$stdout_log"
fi

exit "${MOCK_EXIT:-0}"
