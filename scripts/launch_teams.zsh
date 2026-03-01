#!/usr/bin/env zsh
# Launch 5 team dispatchers in a zellij session, one pane each.
set -euo pipefail

ROOT=/Users/studio/gtfs
JC=$ROOT/target/debug/jc

TEAMS=(
  "team-cli:claude:adapters/claude.zsh"
  "team-arch:claude:adapters/claude.zsh"
  "team-quality:aider:adapters/aider.zsh"
  "team-intelligence:opencode:adapters/opencode.zsh"
  "team-platform:codex:adapters/codex.zsh"
)

SESSION="jobcard-teams"

# Kill existing session if present
zellij delete-session "$SESSION" --force 2>/dev/null || true
sleep 0.5

echo "Launching dispatchers in zellij session: $SESSION"

for i in {1..${#TEAMS[@]}}; do
  IFS=':' read -r team_name adapter_name adapter_path <<< "${TEAMS[$i]}"
  cards_dir="$ROOT/.cards/$team_name"
  log_file="/tmp/jobcard-$team_name.log"

  CMD="$JC --cards-dir $cards_dir dispatcher \
    --adapter $ROOT/$adapter_path \
    --max-workers 5 \
    --poll-ms 500 \
    --max-retries 3 \
    --reap-ms 2000"

  echo "  → $team_name ($adapter_name) → $cards_dir"

  if [ "$i" -eq 1 ]; then
    # First pane: create session
    zellij --session "$SESSION" run \
      --name "$team_name" \
      --floating \
      --close-on-exit \
      -- bash -c "cd $ROOT && echo '=== $team_name ===' && $CMD 2>&1 | tee $log_file"
    sleep 0.5
  else
    # Subsequent panes: attach to session
    zellij --session "$SESSION" run \
      --name "$team_name" \
      --direction down \
      --close-on-exit \
      -- bash -c "cd $ROOT && echo '=== $team_name ===' && $CMD 2>&1 | tee $log_file"
    sleep 0.3
  fi
done

echo ""
echo "All 5 dispatchers launched."
echo "Watch logs:  tail -f /tmp/jobcard-team-*.log"
echo "Check status per team:"
for team in team-cli team-arch team-quality team-intelligence team-platform; do
  echo "  $JC --cards-dir $ROOT/.cards/$team status"
done
