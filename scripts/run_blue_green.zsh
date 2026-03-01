#!/usr/bin/env zsh
set -euo pipefail

ROOT=${0:A:h:h}
BOP="${ROOT}/target/debug/bop"
BLUE_DIR="${ROOT}/.cards-blue"
GREEN_DIR="${ROOT}/.cards-green"
ADAPTER_DEFAULT="${ROOT}/adapters/mock.zsh"

if [[ ! -x "" ]]; then
  echo "Missing jc binary: " >&2
  echo "Run: cargo build" >&2
  exit 1
fi

mkdir -p "$BLUE_DIR" "$GREEN_DIR"
"" --cards-dir "$BLUE_DIR" init >/dev/null
"" --cards-dir "$GREEN_DIR" init >/dev/null

# Keep templates/providers in sync from the canonical cards root when present.
if [[ -d "${ROOT}/.cards/templates" ]]; then
  rsync -a --delete "${ROOT}/.cards/templates/" "${BLUE_DIR}/templates/"
  rsync -a --delete "${ROOT}/.cards/templates/" "${GREEN_DIR}/templates/"
fi
if [[ -f "${ROOT}/.cards/providers.json" ]]; then
  cp "${ROOT}/.cards/providers.json" "${BLUE_DIR}/providers.json"
  cp "${ROOT}/.cards/providers.json" "${GREEN_DIR}/providers.json"
fi

BLUE_DISP_LOG="/tmp/bop-blue-dispatcher.log"
GREEN_DISP_LOG="/tmp/bop-green-dispatcher.log"
BLUE_MG_LOG="/tmp/bop-blue-merge-gate.log"
GREEN_MG_LOG="/tmp/bop-green-merge-gate.log"

pkill -f "bop --cards-dir ${BLUE_DIR} dispatcher" >/dev/null 2>&1 || true
pkill -f "bop --cards-dir ${GREEN_DIR} dispatcher" >/dev/null 2>&1 || true
pkill -f "bop --cards-dir ${BLUE_DIR} merge-gate" >/dev/null 2>&1 || true
pkill -f "bop --cards-dir ${GREEN_DIR} merge-gate" >/dev/null 2>&1 || true

nohup "" --cards-dir "$BLUE_DIR" dispatcher \
  --adapter "$ADAPTER_DEFAULT" \
  --vcs-engine git_gt \
  --poll-ms 250 --reap-ms 1000 >>"$BLUE_DISP_LOG" 2>&1 &

nohup "" --cards-dir "$GREEN_DIR" dispatcher \
  --adapter "$ADAPTER_DEFAULT" \
  --vcs-engine jj \
  --poll-ms 250 --reap-ms 1000 >>"$GREEN_DISP_LOG" 2>&1 &

nohup "" --cards-dir "$BLUE_DIR" merge-gate \
  --vcs-engine git_gt \
  --poll-ms 500 >>"$BLUE_MG_LOG" 2>&1 &

nohup "" --cards-dir "$GREEN_DIR" merge-gate \
  --vcs-engine jj \
  --poll-ms 500 >>"$GREEN_MG_LOG" 2>&1 &

cat <<OUT
Blue/Green factory started.

Blue cards dir:  $BLUE_DIR  (engine=git_gt)
Green cards dir: $GREEN_DIR (engine=jj)

Logs:
  $BLUE_DISP_LOG
  $GREEN_DISP_LOG
  $BLUE_MG_LOG
  $GREEN_MG_LOG
OUT
