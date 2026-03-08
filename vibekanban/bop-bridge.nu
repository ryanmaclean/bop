#!/usr/bin/env nu
# bop-bridge.nu - emit card stage transitions from inside any AI session

# Usage:
#   source vibekanban/bop-bridge.nu
#   bridge stage in-progress
#   bridge stage human-review my-card-id

export def "bridge stage" [
    stage: string,
    card_id?: string,
    --cli (-c): string = "claude",
] {
    mut args = [
        "bridge"
        "emit"
        "--cli"
        $cli
        "--event"
        "stage-change"
        "--stage"
        $stage
    ]
    if $card_id != null {
        $args = ($args | append ["--card-id", $card_id])
    }
    ^bop ...$args
}
