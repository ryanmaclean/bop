#!/usr/bin/env bash

# Helper script to enforce a timeout on another command
# Usage: ./timeout_wrapper.sh <timeout_seconds> <command> [args...]

TIMEOUT=$1
shift

if [[ $(uname) == "Darwin" ]]; then
    # Use perl or python for timeout on macOS as gtimeout is not always available
    perl -e "alarm $TIMEOUT; exec @ARGV" "$@"
else
    timeout $TIMEOUT "$@"
fi
