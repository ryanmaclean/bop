#!/usr/bin/env zsh
# ollama-local.zsh — run a card prompt against an Ollama model (local or cloud variant)
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
#   OLLAMA_MODEL   model to use (default: qwen3-coder:480b-cloud)
#   OLLAMA_HOST    base URL (default: http://localhost:11434)
#   OLLAMA_TIMEOUT timeout in seconds (default: 600)

set -euo pipefail

workdir="$1"
prompt_file="$2"
stdout_log="$3"
stderr_log="$4"

MODEL="${OLLAMA_MODEL:-qwen3-coder:480b-cloud}"
HOST="${OLLAMA_HOST:-http://localhost:11434}"
TIMEOUT_S="${OLLAMA_TIMEOUT:-600}"

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
# Cloud variants (name:cloud) may not appear in /api/tags — skip check for them
if [[ "$MODEL" != *":cloud" ]]; then
    if ! curl -sf "${HOST}/api/tags" | grep -q "\"${MODEL}\""; then
        print -r "model ${MODEL} not found — pull it with: ollama pull ${MODEL}" >> "$stderr_log"
        exit 75
    fi
fi

# ── Run via HTTP API (gives token stats, works with cloud variants) ───────────
PROMPT_BYTES=$(wc -c < "$prompt_file")
print -r "[ollama] starting ${MODEL} on ${PROMPT_BYTES} byte prompt" >> "$stderr_log"

START_S=$SECONDS

# JSON-encode prompt using python3 (handles quotes, newlines, unicode)
PROMPT_JSON=$(python3 -c "
import json, sys
with open(sys.argv[1]) as f:
    print(json.dumps(f.read()), end='')
" "$prompt_file")

# stats_log lives next to stdout_log in the logs/ dir
STATS_LOG="${stdout_log:h}/ollama-stats.json"

# Build JSON body with python (handles all quoting correctly)
REQUEST_JSON=$(python3 -c "
import json, sys
payload = {
    'model':  sys.argv[1],
    'prompt': open(sys.argv[2]).read(),
    'stream': False,
}
print(json.dumps(payload))
" "$MODEL" "$prompt_file")

RESPONSE=$(curl -s --max-time "$TIMEOUT_S" \
    "${HOST}/api/generate" \
    -H "Content-Type: application/json" \
    -d "$REQUEST_JSON" \
    2>> "$stderr_log")

rc=$?

if [[ $rc -ne 0 ]]; then
    print -r "[ollama] curl failed (rc=$rc) — exiting 75" >> "$stderr_log"
    exit 75
fi

if [[ -z "$RESPONSE" ]]; then
    print -r "[ollama] empty response — model may be unavailable, exiting 75" >> "$stderr_log"
    exit 75
fi

# Check for error in response JSON
if echo "$RESPONSE" | python3 -c "
import json, sys
d = json.load(sys.stdin)
if 'error' in d:
    print('ERROR:', d['error'], file=sys.stderr)
    sys.exit(1)
" 2>> "$stderr_log"; then
    : # no error
else
    print -r "[ollama] model returned error — exiting 75 (transient)" >> "$stderr_log"
    exit 75
fi

# Write the model's text response to stdout_log
python3 -c "
import json, sys
d = json.load(sys.stdin)
print(d.get('response', ''), end='')
" <<< "$RESPONSE" > "$stdout_log"

# Write token stats to ollama-stats.json for dispatcher to read into run record
ELAPSED=$(( SECONDS - START_S ))
python3 -c "
import json, sys, datetime
d = json.load(sys.stdin)
eval_dur = d.get('eval_duration', 0)          # nanoseconds for completion tokens
eval_cnt = d.get('eval_count', 0)
toks_per_s = round(eval_cnt / (eval_dur / 1e9), 1) if eval_dur > 0 else None
stats = {
    'model':             sys.argv[1],
    'provider':          'ollama',
    'prompt_tokens':     d.get('prompt_eval_count', 0),
    'completion_tokens': eval_cnt,
    'toks_per_s':        toks_per_s,
    'total_duration_ns': d.get('total_duration', 0),
    'load_duration_ns':  d.get('load_duration', 0),
    'elapsed_s':         int(sys.argv[2]),
    'done':              d.get('done', False),
    'done_reason':       d.get('done_reason', ''),
    'timestamp':         datetime.datetime.now(datetime.timezone.utc).isoformat().replace('+00:00', 'Z'),
}
print(json.dumps(stats, indent=2))
tps = f\"{toks_per_s} tok/s\" if toks_per_s else \"\"
print(f\"[ollama] done. prompt={stats['prompt_tokens']} completion={eval_cnt} {tps}\", file=sys.stderr)
" "$MODEL" "$ELAPSED" <<< "$RESPONSE" > "$STATS_LOG" 2>> "$stderr_log" \
  || print -r "[ollama] done (stats error)." >> "$stderr_log"
