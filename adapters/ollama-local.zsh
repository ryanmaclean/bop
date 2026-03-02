#!/usr/bin/env zsh
# ollama-local.zsh — run a card prompt against a local Ollama model
#
# Usage (called by dispatcher):
#   ollama-local.zsh <workdir> <prompt_file> <stdout_log> <stderr_log>
#
# Exit codes:
#   0   success
#   75  transient (ollama not running, model not loaded) → pending/, rotate provider
#   1   other failure → failed/
#
# Env vars:
#   OLLAMA_MODEL   model to use (default: qwen2.5-coder:7b)
#   OLLAMA_HOST    base URL (default: http://localhost:11434)

set -euo pipefail

workdir="$1"
prompt_file="$2"
stdout_log="$3"
stderr_log="$4"

MODEL="${OLLAMA_MODEL:-qwen2.5-coder:7b}"
HOST="${OLLAMA_HOST:-http://localhost:11434}"

# Make paths absolute before cd
for var in prompt_file stdout_log stderr_log; do
    [[ "${(P)var}" != /* ]] && eval "$var=$(pwd)/${(P)var}"
done

cd "$workdir"

# ── Health check ──────────────────────────────────────────────────────────────
if ! curl -sf "${HOST}/api/tags" >/dev/null 2>&1; then
    print -r "ollama not reachable at ${HOST} — exiting 75 (transient)" >> "$stderr_log"
    exit 75
fi

# ── Model check ───────────────────────────────────────────────────────────────
if ! curl -sf "${HOST}/api/tags" | grep -q "\"${MODEL}\""; then
    print -r "model ${MODEL} not found — pull it with: ollama pull ${MODEL}" >> "$stderr_log"
    exit 75
fi

# ── Run ───────────────────────────────────────────────────────────────────────
print -r "running ${MODEL} on $(wc -c < "$prompt_file") byte prompt" >> "$stderr_log"

ollama run "$MODEL" "$(cat "$prompt_file")" \
    > "$stdout_log" 2>> "$stderr_log"
