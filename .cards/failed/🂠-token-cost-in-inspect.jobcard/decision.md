# Decision: Token cost display in bop inspect

## Summary
Adds cost/token parsing to `bop inspect` output by reading `logs/stdout.log` JSON.

## Rationale
Visibility into per-card token cost enables optimization. No behavioral change to
the dispatcher or merge-gate — read-only display enhancement.

## Decision
Approved — useful observability, no risk to core card lifecycle.
