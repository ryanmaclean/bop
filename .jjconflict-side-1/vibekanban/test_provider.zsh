#!/usr/bin/env zsh
set -euo pipefail
# Create test .cards/ structure
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

mkdir -p "$tmp/team-cli/pending" "$tmp/team-cli/running"
mkdir -p "$tmp/team-cli/pending/card-abc.jobcard"
printf '{"id":"card-abc","title":"Test card","stage":"implement"}' \
  > "$tmp/team-cli/pending/card-abc.jobcard/meta.json"
mkdir -p "$tmp/team-cli/running/card-xyz.jobcard"
printf '{"id":"card-xyz","title":"Running card","stage":"test"}' \
  > "$tmp/team-cli/running/card-xyz.jobcard/meta.json"

# Run provider
SCRIPT_DIR="${0:A:h}"
out=$(CARDS_DIR="$tmp" "$SCRIPT_DIR/jobcard-provider.zsh")
echo "Provider output: $out"

# Validate: must be a JSON array with 2 items
count=$(echo "$out" | python3 -c "import json,sys; d=json.load(sys.stdin); print(len(d))")
[[ "$count" == "2" ]] || { echo "FAIL: expected 2 tasks, got $count"; exit 1; }

id=$(echo "$out" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d[0]['id'])")
card_status=$(echo "$out" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d[0]['status'])")
[[ "$id" == "card-abc" ]] || { echo "FAIL: expected id=card-abc, got $id"; exit 1; }
[[ "$card_status" == "pending" ]] || { echo "FAIL: expected status=pending, got $card_status"; exit 1; }

echo "PASS: provider outputs correct JSON"
