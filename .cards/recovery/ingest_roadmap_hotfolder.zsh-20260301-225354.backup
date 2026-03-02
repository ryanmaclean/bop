#!/usr/bin/env zsh
set -euo pipefail
setopt NULL_GLOB

ROOT=${0:A:h:h}
INBOX_DIR="${ROOT}/examples/roadmap-inbox/drop"
CARDS_DIR="${ROOT}/.cards"
TEMPLATE_DIR=""
PROCESSED_DIR=""
FAILED_DIR=""
DRY_RUN=0

usage() {
  cat <<USAGE
usage: ingest_roadmap_hotfolder.zsh [--inbox DIR] [--cards-dir DIR] [--template-dir DIR] [--processed-dir DIR] [--failed-dir DIR] [--dry-run]

Scans a hot folder for roadmap request files and atomically creates roadmap
jobcard bundles in pending/ without requiring interactive CLI usage.
USAGE
}

while (( $# > 0 )); do
  case "$1" in
    --inbox)
      INBOX_DIR="$2"
      shift 2
      ;;
    --cards-dir)
      CARDS_DIR="$2"
      shift 2
      ;;
    --template-dir)
      TEMPLATE_DIR="$2"
      shift 2
      ;;
    --processed-dir)
      PROCESSED_DIR="$2"
      shift 2
      ;;
    --failed-dir)
      FAILED_DIR="$2"
      shift 2
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ -z "${TEMPLATE_DIR}" ]]; then
  TEMPLATE_DIR="${CARDS_DIR}/templates/roadmap.jobcard"
fi
if [[ -z "${PROCESSED_DIR}" ]]; then
  PROCESSED_DIR="${INBOX_DIR:h}/processed"
fi
if [[ -z "${FAILED_DIR}" ]]; then
  FAILED_DIR="${INBOX_DIR:h}/failed"
fi

PENDING_DIR="${CARDS_DIR}/pending"
LOG_DIR="${CARDS_DIR}/logs"
LOG_FILE="${LOG_DIR}/roadmap-hotfolder.log"

mkdir -p "${INBOX_DIR}" "${PROCESSED_DIR}" "${FAILED_DIR}" "${PENDING_DIR}" "${LOG_DIR}"

if [[ ! -d "${TEMPLATE_DIR}" ]]; then
  echo "missing template directory: ${TEMPLATE_DIR}" >&2
  exit 1
fi
if [[ ! -f "${TEMPLATE_DIR}/meta.json" ]]; then
  echo "missing template meta.json in: ${TEMPLATE_DIR}" >&2
  exit 1
fi

log_line() {
  local msg="$1"
  local ts
  ts=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
  print -r -- "${ts} ${msg}" | tee -a "${LOG_FILE}" >/dev/null
}

slugify() {
  local raw="$1"
  local ascii
  ascii=$(print -r -- "${raw}" | iconv -f UTF-8 -t ASCII//TRANSLIT 2>/dev/null || print -r -- "${raw}")
  ascii=${ascii:l}
  ascii=${ascii//[^a-z0-9._-]/-}
  ascii=${ascii//--/-}
  ascii=${ascii#-}
  ascii=${ascii%-}
  print -r -- "${ascii}"
}

supports_request_file() {
  local ext="$1"
  case "${ext:l}" in
    roadmap|md|txt|json|yaml|yml)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

id_exists_anywhere() {
  local id="$1"
  local state
  for state in pending running done merged failed; do
    if [[ -d "${CARDS_DIR}/${state}/${id}.jobcard" ]]; then
      return 0
    fi
    local matches=("${CARDS_DIR}/${state}"/*-"${id}".jobcard(N))
    if (( ${#matches[@]} > 0 )); then
      return 0
    fi
  done
  return 1
}

clone_template() {
  local src="$1"
  local dst="$2"

  if [[ "$(uname -s)" == "Darwin" ]]; then
    # Preserve APFS clone semantics and metadata (xattrs/ACL/quarantine/compression).
    if ditto --clone --extattr --acl --qtn --preserveHFSCompression "${src}" "${dst}" >/dev/null 2>&1; then
      return 0
    fi
    if cp -cR "${src}" "${dst}" >/dev/null 2>&1; then
      return 0
    fi
    echo "clone_template: APFS clone copy failed for ${src} -> ${dst}" >&2
    return 1
  fi

  if cp --reflink=auto -r "${src}" "${dst}" >/dev/null 2>&1; then
    return 0
  fi
  cp -R "${src}" "${dst}"
}

write_meta() {
  local template_meta="$1"
  local target_meta="$2"
  local id="$3"
  local created="$4"

  local esc_id esc_created
  esc_id=$(print -r -- "${id}" | sed -e 's/[\/&]/\\&/g')
  esc_created=$(print -r -- "${created}" | sed -e 's/[\/&]/\\&/g')

  sed \
    -e "s/\"id\": \"REPLACE_ID\"/\"id\": \"${esc_id}\"/" \
    -e "s/\"worktree_branch\": \"job\\/REPLACE_ID\"/\"worktree_branch\": \"job\\/${esc_id}\"/" \
    -e "s/\"created\": \"2026-03-01T00:00:00Z\"/\"created\": \"${esc_created}\"/" \
    "${template_meta}" > "${target_meta}"
}

write_spec_from_request() {
  local request_file="$1"
  local spec_file="$2"
  local id="$3"
  local created="$4"

  {
    print -r -- "# Roadmap Request"
    print -r -- ""
    print -r -- "- ID: ${id}"
    print -r -- "- Source: ${request_file:t}"
    print -r -- "- Received (UTC): ${created}"
    print -r -- ""
    print -r -- "## Request"
    print -r -- ""
    print -r -- '```'
    cat "${request_file}"
    print -r -- '```'
    print -r -- ""
    print -r -- "## Expected Workflow"
    print -r -- ""
    print -r -- "- Analyze"
    print -r -- "- Discover"
    print -r -- "- Generate"
    print -r -- "- QA"
  } > "${spec_file}"
}

render_thumbnail_if_possible() {
  local card_dir="$1"
  local renderer="${ROOT}/scripts/render_card_thumbnail.swift"
  local meta_file="${card_dir}/meta.json"
  local out_file="${card_dir}/QuickLook/Thumbnail.png"

  mkdir -p "${card_dir}/QuickLook"
  if [[ -f "${renderer}" && -f "${meta_file}" ]]; then
    swift "${renderer}" "${meta_file}" "${out_file}" >/dev/null 2>&1 || true
  fi
}

move_request() {
  local src="$1"
  local dst_dir="$2"
  local stamp
  stamp=$(date -u +"%Y%m%dT%H%M%SZ")
  mv "${src}" "${dst_dir}/${stamp}-${src:t}"
}

process_request() {
  local request_file="$1"
  local ext="${request_file:e}"

  if ! supports_request_file "${ext}"; then
    log_line "skip unsupported file: ${request_file:t}"
    return 0
  fi

  local stem="${request_file:t:r}"
  local id
  id=$(slugify "${stem}")
  if [[ -z "${id}" ]]; then
    id="roadmap-$(date -u +%s)"
  fi
  if id_exists_anywhere "${id}"; then
    id="${id}-$(date -u +%H%M%S)"
  fi

  local created
  created=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

  local card_name="🂠-${id}.jobcard"
  local final_card="${PENDING_DIR}/${card_name}"
  local temp_card="${PENDING_DIR}/.${card_name}.tmp.$$"

  rm -rf "${temp_card}"
  clone_template "${TEMPLATE_DIR}" "${temp_card}"

  mkdir -p "${temp_card}/logs" "${temp_card}/output"
  write_meta "${TEMPLATE_DIR}/meta.json" "${temp_card}/meta.json" "${id}" "${created}"
  write_spec_from_request "${request_file}" "${temp_card}/spec.md" "${id}" "${created}"
  render_thumbnail_if_possible "${temp_card}"

  if (( DRY_RUN == 1 )); then
    log_line "dry-run: would queue ${card_name} from ${request_file:t}"
    rm -rf "${temp_card}"
    return 0
  fi

  mv "${temp_card}" "${final_card}"
  move_request "${request_file}" "${PROCESSED_DIR}"
  log_line "queued ${card_name} from ${request_file:t}"
}

requests=("${INBOX_DIR}"/*(.N))
if (( ${#requests[@]} == 0 )); then
  exit 0
fi

for request_file in "${requests[@]}"; do
  if ! process_request "${request_file}"; then
    log_line "failed to ingest ${request_file:t}"
    [[ -f "${request_file}" ]] && move_request "${request_file}" "${FAILED_DIR}" || true
  fi
done
