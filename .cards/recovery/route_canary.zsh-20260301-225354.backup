#!/usr/bin/env zsh
set -euo pipefail

ROOT=${0:A:h:h}
SOURCE_DIR="${ROOT}/.cards"
BLUE_DIR="${ROOT}/.cards-blue"
GREEN_DIR="${ROOT}/.cards-green"
GREEN_PCT=${GREEN_PCT:-20}
CANARY_TEAMS_FILE="${ROOT}/.cards/canary-teams.txt"
ROUTE_OVERRIDE_FILE="${ROOT}/.cards/route.override"

if [[ ! -d "$SOURCE_DIR/pending" ]]; then
  echo "No pending source dir at $SOURCE_DIR/pending" >&2
  exit 1
fi

mkdir -p "$BLUE_DIR/pending" "$GREEN_DIR/pending"

trim() {
  local s="$1"
  s="${s##[[:space:]]#}"
  s="${s%%[[:space:]]#}"
  print -r -- "$s"
}

route_override=""
if [[ -f "$ROUTE_OVERRIDE_FILE" ]]; then
  route_override="$(head -n 1 "$ROUTE_OVERRIDE_FILE" || true)"
  route_override="$(trim "${route_override%%#*}")"
fi

canary_teams=()
if [[ -f "$CANARY_TEAMS_FILE" ]]; then
  while IFS= read -r line; do
    line="$(trim "${line%%#*}")"
    [[ -z "$line" ]] && continue
    canary_teams+=("$line")
  done < "$CANARY_TEAMS_FILE"
fi

is_canary_team() {
  local team="$1"
  for t in "${canary_teams[@]:-}"; do
    if [[ -n "$team" && "$team" == "$t" ]]; then
      return 0
    fi
  done
  return 1
}

deterministic_bucket() {
  local key="$1"
  python3 - <<'PY' "$key"
import hashlib
import sys
h = hashlib.sha256(sys.argv[1].encode()).hexdigest()
print(int(h[:8], 16) % 100)
PY
}

route_lane() {
  local card_id="$1"
  local team="${2:-}"

  case "$route_override" in
    blue-only)
      echo "blue"
      return
      ;;
    green-only)
      echo "green"
      return
      ;;
    ""|balanced)
      ;;
    *)
      echo "Unknown route override '$route_override'; treating as balanced" >&2
      ;;
  esac

  if is_canary_team "$team"; then
    echo "dual"
    return
  fi

  local bucket
  bucket="$(deterministic_bucket "$card_id")"
  if (( bucket < GREEN_PCT )); then
    echo "green"
  else
    echo "blue"
  fi
}

copy_card() {
  local src="$1"
  local dst_dir="$2"

  if [[ "$(uname -s)" == "Darwin" ]]; then
    if ditto --clone "$src" "$dst_dir/${src:t}" >/dev/null 2>&1; then
      return 0
    fi
    if cp -cR "$src" "$dst_dir/" >/dev/null 2>&1; then
      return 0
    fi
    echo "copy_card: APFS clone copy failed for $src -> $dst_dir" >&2
    return 1
  fi

  if cp --reflink=auto -r "$src" "$dst_dir/" >/dev/null 2>&1; then
    return 0
  fi
  cp -R "$src" "$dst_dir/"
}

move_or_copy() {
  local lane="$1"
  local card="$2"
  local base
  base="$(basename "$card")"

  case "$lane" in
    blue)
      if [[ -e "$BLUE_DIR/pending/$base" ]]; then
        echo "skip $base: already exists in blue" >&2
        return 1
      fi
      mv "$card" "$BLUE_DIR/pending/"
      ;;
    green)
      if [[ -e "$GREEN_DIR/pending/$base" ]]; then
        echo "skip $base: already exists in green" >&2
        return 1
      fi
      mv "$card" "$GREEN_DIR/pending/"
      ;;
    dual)
      if [[ -e "$BLUE_DIR/pending/$base" || -e "$GREEN_DIR/pending/$base" ]]; then
        echo "skip $base: already exists in blue or green" >&2
        return 1
      fi
      copy_card "$card" "$BLUE_DIR/pending"
      copy_card "$card" "$GREEN_DIR/pending"
      rm -rf "$card"
      ;;
    *)
      echo "invalid lane: $lane" >&2
      return 1
      ;;
  esac

  return 0
}

moved=0
dual=0
for card in "$SOURCE_DIR"/pending/*.jobcard; do
  [[ -d "$card" ]] || continue
  id="$(basename "$card" .jobcard)"

  team=""
  if [[ -f "$card/meta.json" ]]; then
    team="$(python3 - <<'PY' "$card/meta.json"
import json
import sys
p = sys.argv[1]
try:
    with open(p, 'r', encoding='utf-8') as fh:
        meta = json.load(fh)
    print(meta.get('team') or '')
except Exception:
    print('')
PY
)"
  fi

  lane="$(route_lane "$id" "$team")"
  if move_or_copy "$lane" "$card"; then
    if [[ "$lane" == "dual" ]]; then
      echo "$id -> blue+green (canary)"
      dual=$((dual + 1))
    else
      echo "$id -> $lane"
    fi
    moved=$((moved + 1))
  fi
done

echo "Routed $moved pending cards (green=${GREEN_PCT}% deterministic, canary dual cards=${dual}, override='${route_override:-balanced}')."
