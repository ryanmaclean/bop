{{spec}}

Project memory:
{{memory}}

Acceptance criteria:
{{acceptance_criteria}}

Work this card as a planning task, not a feature implementation.
Required output:
1. Final workflow plan document at `docs/plans/2026-03-01-macos-file-agent-workflow.md`
2. Any repo-rename hardening needed for active card execution paths
3. Short handoff summary in `output/result.md` with phased next actions

Constraints:
1. Keep filesystem state-machine semantics unchanged (`pending -> running -> done/failed -> merged`)
2. Preserve Quick Look sandbox boundaries; route controls through host app URL handling
3. Prefer repo-relative adapter commands over absolute machine-specific paths
