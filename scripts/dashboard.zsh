#!/usr/bin/env zsh
# Live status dashboard for all 5 job teams
setopt NULL_GLOB   # unmatched globs expand to nothing instead of erroring
ROOT=/Users/studio/gtfs

TEAMS=(
  "team-cli:claude"
  "team-arch:claude"
  "team-quality:claude"
  "team-intelligence:opencode"
  "team-platform:codex"
)

count_cards() { find "$1" -maxdepth 1 -name '*.jobcard' -type d 2>/dev/null | wc -l | tr -d ' '; }

while true; do
  clear
  echo "╔══════════════════════════════════════════════════════════════╗"
  printf  "║  JOBCARD DASHBOARD  %-42s║\n" "$(date '+%H:%M:%S')"
  echo "╠══════════════════════════════════════════════════════════════╣"

  for entry in "${TEAMS[@]}"; do
    IFS=':' read -r team adapter <<< "$entry"
    cards_dir="$ROOT/.cards/$team"

    n_pending=$(count_cards "$cards_dir/pending")
    n_running=$(count_cards "$cards_dir/running")
    n_done=$(count_cards    "$cards_dir/done")
    n_merged=$(count_cards  "$cards_dir/merged")
    n_failed=$(count_cards  "$cards_dir/failed")
    total=5

    # Progress bar: M=merged ✓=done ▶=running ·=queued ✗=failed
    bar=""
    for ((i=0; i<total; i++)); do
      if   (( i < n_merged ));                       then bar+="M"
      elif (( i < n_merged + n_done ));              then bar+="✓"
      elif (( i < n_merged + n_done + n_running ));  then bar+="▶"
      elif (( i < total - n_failed ));               then bar+="·"
      else                                                bar+="✗"
      fi
    done

    printf "║  %-18s [%s] [%s]\n" "$team" "$bar" "$adapter"
    printf "║  p:%-2s r:%-2s d:%-2s m:%-2s f:%-2s%33s║\n" \
      "$n_pending" "$n_running" "$n_done" "$n_merged" "$n_failed" ""

    for card_dir in "$cards_dir/running/"*.jobcard; do
      [ -d "$card_dir" ] || continue
      card_name=$(basename "$card_dir" .jobcard)
      pid=""; pid_file="$card_dir/logs/pid"
      [ -f "$pid_file" ] && pid=" [$(cat "$pid_file")]"
      printf "║  ▶ %-25s%-32s║\n" "$card_name" "$pid"

      stdout="$card_dir/logs/stdout.log"
      if [ -f "$stdout" ] && [ -s "$stdout" ]; then
        last=$(tail -1 "$stdout" 2>/dev/null | tr -d '\r' | sed 's/\x1b\[[0-9;]*m//g' | cut -c1-54)
        printf "║    └ %-56s║\n" "$last"
      fi
    done

    echo "╠══════════════════════════════════════════════════════════════╣"
  done

  td=0; tr_=0; tp=0; tf=0
  for entry in "${TEAMS[@]}"; do
    IFS=':' read -r team _ <<< "$entry"
    c="$ROOT/.cards/$team"
    td=$(( td  + $(count_cards "$c/done")    + $(count_cards "$c/merged") ))
    tr_=$(( tr_ + $(count_cards "$c/running") ))
    tp=$(( tp  + $(count_cards "$c/pending") ))
    tf=$(( tf  + $(count_cards "$c/failed")  ))
  done
  printf "║  done:%-3s  running:%-3s  pending:%-3s  failed:%-3s         ║\n" \
    "$td" "$tr_" "$tp" "$tf"
  echo "╚══════════════════════════════════════════════════════════════╝"
  echo ""

  for entry in "${TEAMS[@]}"; do
    IFS=':' read -r team _ <<< "$entry"
    for state in done merged; do
      for card_dir in "$ROOT/.cards/$team/$state/"*.jobcard; do
        [ -d "$card_dir" ] || continue
        printf "  ✓ %-26s %s\n" "$(basename "$card_dir" .jobcard)" "$team"
      done
    done
  done

  sleep 3
done
