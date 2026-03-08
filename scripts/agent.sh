#!/bin/sh
# bop card agent — BusyBox / POSIX sh compatible
#
# Joins the bop card queue using the atomic mv claim protocol.
# Uses inotifywait (Linux inotify) when available; polls every 2s as fallback.
# Implements the same exit-code contract as the Rust dispatcher:
#   adapter exit 0  → card moved to done/
#   adapter exit 75 → card returned to pending/ (rate-limited, retry)
#   adapter exit *  → card moved to failed/
#
# The mv claim is atomic at the kernel level. N concurrent agents (Rust
# dispatchers, shell agents, Unikraft host scripts) can all race on the
# same pending/ directory — exactly one wins per card.
#
# Usage:
#   sh scripts/agent.sh [cards_dir] [adapter_cmd]
#
#   cards_dir    Path to a .cards/ dir or team subdir. Default: .cards
#   adapter_cmd  Command to run; receives card_dir as $1.
#                Default: sh (expects adapter.sh inside the card dir)
#
# Examples:
#   sh scripts/agent.sh .cards/team-cli/ nu adapters/claude.nu
#   sh scripts/agent.sh .cards/team-arch/
#   sh scripts/agent.sh .cards/ nu adapters/claude.nu &  # two agents race; only one wins per card
#   sh scripts/agent.sh .cards/ nu adapters/claude.nu &

set -eu

CARDS="${1:-.cards}"
ADAPTER="${2:-sh}"

claim_and_run() {
    file="$1"
    src="$CARDS/pending/$file"
    dst="$CARDS/running/$file"

    # Atomic claim — only one agent wins the rename; losers skip silently.
    mv "$src" "$dst" 2>/dev/null || return 0

    # Run adapter with card dir as $1.
    # Subshell: cd into the card so $PWD encodes state+id for the adapter.
    rc=0
    (cd "$dst" && "$ADAPTER" "$dst") || rc=$?

    case $rc in
        0)  mv "$dst" "$CARDS/done/$file"    ;;
        75) mv "$dst" "$CARDS/pending/$file" ;;
        *)  mv "$dst" "$CARDS/failed/$file"  ;;
    esac
}

# Drain any cards already waiting in pending/ before entering watch loop.
for f in "$CARDS/pending/"*.bop; do
    [ -d "$f" ] && claim_and_run "$(basename "$f")"
done

if command -v inotifywait >/dev/null 2>&1; then
    # Event-driven path: Linux inotify via inotifywait (included in BusyBox).
    inotifywait -m -q -e moved_to,create "$CARDS/pending" 2>/dev/null |
    while IFS= read -r line; do
        # inotifywait output: "<dir> <EVENT> <filename>"
        # Extract filename as the last whitespace-delimited token.
        file="${line##* }"
        case "$file" in
            *.bop) claim_and_run "$file" ;;
        esac
    done
else
    # Polling fallback: works on macOS, Unikraft host-side, anywhere without inotify.
    printf 'bop-agent: inotifywait not found, polling every 2s\n' >&2
    while true; do
        found=0
        for f in "$CARDS/pending/"*.bop; do
            if [ -d "$f" ]; then
                found=1
                claim_and_run "$(basename "$f")"
            fi
        done
        # If no cards found in this sweep, sleep before next sweep.
        [ "$found" -eq 0 ] && sleep 2
    done
fi
