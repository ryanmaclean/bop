#!/usr/bin/env zsh
set -euo pipefail
ROOT=${0:A:h:h}
setopt NULL_GLOB
failed=0
for f in /adapters/*.zsh; do
  head -1 "$f" | grep -q '#!/usr/bin/env zsh' || { echo "FAIL: $f has wrong shebang"; failed=1; }
done
for f in /scripts/launch_teams.zsh /scripts/dashboard.zsh; do
  [[ -f "$f" ]] || { echo "FAIL: $f missing"; failed=1; }
  head -1 "$f" | grep -q '#!/usr/bin/env zsh' || { echo "FAIL: $f has wrong shebang"; failed=1; }
done
for f in /adapters/*.sh /scripts/launch_teams.sh /scripts/dashboard.sh; do
  [[ -f "$f" ]] && { echo "FAIL: bash file still exists: $f"; failed=1; }
done 2>/dev/null || true
[[ $failed -eq 0 ]] && echo "PASS: all adapters and scripts are zsh" || exit 1
