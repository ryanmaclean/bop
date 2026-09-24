# Watch List

Lore/watch entries for external projects or proposals that intersect with
bop's design space but aren't yet worth active investment. Each entry
records what it is, why it's watched instead of adopted, and the concrete
trigger that would promote it to a design comparison or spec.

Format: one row per entry, newest first. `trigger` should be a checkable
fact (a release, a merged PR, a declared field), not a vague "revisit
later."

| id | subject | area | status | why watch-only | trigger to promote | added |
|---|---|---|---|---|---|---|
| W1 | FJORD (daemonless/fjord, BSD-2-Clause) | jail-services-descriptor | watch-only | compose.yaml + x-fjord extension over podman/ocijail or AppJail for persistent fleet jail services (Gitea, Ergo, Datadog agent); 7 GitHub stars, v0.2.0; its own README admits it "operates in local trusted mode without an integrated authentication barrier" — no declared trust/provenance field, while bop's own `schema/manifest.v1.json` already has one (`min_trust`). Starting a design comparison now is premature given FJORD lacks the one thing AX-first requires. | FJORD declares a trust/provenance field in its manifest schema, or reaches a maturity/adoption level (stars, releases, production users) that changes the cost-benefit of a comparison | 2026-09-23 (decision D22, BSD Weekly addendum) |
