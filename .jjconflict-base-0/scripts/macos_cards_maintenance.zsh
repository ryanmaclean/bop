#!/usr/bin/env zsh
set -euo pipefail
setopt NULL_GLOB

ROOT=${0:A:h:h}
CARDS_ROOT="${ROOT}/.cards"
RENDER_SWIFT="${ROOT}/scripts/render_card_thumbnail.swift"
ICON_SWIFT="${ROOT}/scripts/set_card_icon.swift"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "macos_cards_maintenance: Darwin only" >&2
  exit 1
fi

if [[ ! -f "${RENDER_SWIFT}" ]]; then
  echo "missing renderer: ${RENDER_SWIFT}" >&2
  exit 1
fi

COMPRESS=0
if [[ "${1:-}" == "--compress" ]]; then
  COMPRESS=1
fi

refresh_thumbnail() {
  local card_dir="$1"
  local meta="${card_dir}/meta.json"
  local out="${card_dir}/QuickLook/Thumbnail.png"
  [[ -f "${meta}" ]] || return 0
  swift "${RENDER_SWIFT}" "${meta}" "${out}" >/dev/null 2>&1 || true
}

compress_card_hfs() {
  local card_dir="$1"
  local parent="${card_dir:h}"
  local name="${card_dir:t}"
  local tmp="${parent}/${name}.hfs.tmp"
  local backup="${parent}/${name}.bak.tmp"
  rm -rf "${tmp}" "${backup}"
  ditto --hfsCompression "${card_dir}" "${tmp}" >/dev/null 2>&1 || {
    rm -rf "${tmp}"
    return 0
  }
  mv "${card_dir}" "${backup}"
  if mv "${tmp}" "${card_dir}"; then
    rm -rf "${backup}"
  else
    mv "${backup}" "${card_dir}" || true
    rm -rf "${tmp}"
  fi
}

typeset -a cards
cards=(
  "${CARDS_ROOT}"/done/*.jobcard
  "${CARDS_ROOT}"/failed/*.jobcard
  "${CARDS_ROOT}"/merged/*.jobcard
  "${CARDS_ROOT}"/team-*/done/*.jobcard
  "${CARDS_ROOT}"/team-*/failed/*.jobcard
  "${CARDS_ROOT}"/team-*/merged/*.jobcard
)

for card_dir in "${cards[@]}"; do
  [[ -d "${card_dir}" ]] || continue
  refresh_thumbnail "${card_dir}"
  if [[ "${COMPRESS}" -eq 1 ]]; then
    compress_card_hfs "${card_dir}"
  fi
done

echo "maintenance complete (compress=${COMPRESS})"
