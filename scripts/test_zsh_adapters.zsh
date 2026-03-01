#!/usr/bin/env zsh
set -euo pipefail
setopt NULL_GLOB
failed=0
for f in /Users/studio/gtfs/adapters/*.zsh; do
  head -1 "$f" | grep -q '#!/usr/bin/env zsh' || { echo "FAIL: $f has wrong shebang"; failed=1; }
done
for f in /Users/studio/gtfs/scripts/launch_teams.zsh /Users/studio/gtfs/scripts/dashboard.zsh; do
  [[ -f "$f" ]] || { echo "FAIL: $f missing"; failed=1; }
  head -1 "$f" | grep -q '#!/usr/bin/env zsh' || { echo "FAIL: $f has wrong shebang"; failed=1; }
done
for f in /Users/studio/gtfs/adapters/*.sh /Users/studio/gtfs/scripts/launch_teams.sh /Users/studio/gtfs/scripts/dashboard.sh; do
  [[ -f "$f" ]] && { echo "FAIL: bash file still exists: $f"; failed=1; }
done 2>/dev/null || true
[[ $failed -eq 0 ]] && echo "PASS: all adapters and scripts are zsh" || exit 1
