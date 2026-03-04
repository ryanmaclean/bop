#!/usr/bin/env zsh
set -euo pipefail

ROOT=${0:A:h:h}
LABEL="sh.bop.roadmap-inbox"
PLIST_PATH="${HOME}/Library/LaunchAgents/${LABEL}.plist"
INBOX_DIR="${ROOT}/examples/roadmap-inbox/drop"
CARDS_DIR="${ROOT}/.cards"
SCRIPT_PATH="${ROOT}/scripts/ingest_roadmap_hotfolder.zsh"
MODE="install"

usage() {
  cat <<USAGE
usage: install_roadmap_hotfolder_launchd.zsh [--inbox DIR] [--cards-dir DIR] [--uninstall] [--status]

Installs a launchd QueueDirectories agent that watches a roadmap hot folder and
creates roadmap bops in pending/.
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
    --uninstall)
      MODE="uninstall"
      shift
      ;;
    --status)
      MODE="status"
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

uid=$(id -u)
service="gui/${uid}/${LABEL}"

if [[ "${MODE}" == "status" ]]; then
  launchctl print "${service}" >/dev/null 2>&1 && {
    echo "loaded: ${service}"
    echo "plist:  ${PLIST_PATH}"
    exit 0
  }
  echo "not loaded: ${service}"
  exit 1
fi

if [[ "${MODE}" == "uninstall" ]]; then
  launchctl bootout "${service}" >/dev/null 2>&1 || true
  if [[ -f "${PLIST_PATH}" ]]; then
    rm -f "${PLIST_PATH}"
    echo "removed ${PLIST_PATH}"
  else
    echo "not installed"
  fi
  exit 0
fi

if [[ ! -x "${SCRIPT_PATH}" ]]; then
  echo "missing ingest script: ${SCRIPT_PATH}" >&2
  exit 1
fi

mkdir -p "${HOME}/Library/LaunchAgents" "${INBOX_DIR}" "${INBOX_DIR:h}/processed" "${INBOX_DIR:h}/failed"

cat > "${PLIST_PATH}" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>${LABEL}</string>

  <key>QueueDirectories</key>
  <array>
    <string>${INBOX_DIR}</string>
  </array>

  <key>RunAtLoad</key>
  <true/>

  <key>ProgramArguments</key>
  <array>
    <string>/bin/zsh</string>
    <string>${SCRIPT_PATH}</string>
    <string>--inbox</string>
    <string>${INBOX_DIR}</string>
    <string>--cards-dir</string>
    <string>${CARDS_DIR}</string>
  </array>

  <key>StandardOutPath</key>
  <string>/tmp/bop-roadmap-inbox.log</string>

  <key>StandardErrorPath</key>
  <string>/tmp/bop-roadmap-inbox.err</string>
</dict>
</plist>
PLIST

launchctl bootout "${service}" >/dev/null 2>&1 || true
launchctl bootstrap "gui/${uid}" "${PLIST_PATH}"
launchctl enable "${service}" >/dev/null 2>&1 || true

echo "installed ${LABEL}"
echo "hot folder: ${INBOX_DIR}"
echo "cards dir:  ${CARDS_DIR}"
echo "logs:       /tmp/bop-roadmap-inbox.log"
echo "error log:  /tmp/bop-roadmap-inbox.err"
