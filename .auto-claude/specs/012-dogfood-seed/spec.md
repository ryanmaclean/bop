# Dogfood: seed bop's own backlog as bop cards

## Goal

Use bop to manage bop's own development. Create real cards for the known
outstanding work items so the system eats its own cooking.

## Why now

The dispatcher, merge-gate, retry, kill, logs, and clean all work. The system
is ready to manage real work. An empty `.cards/` proves nothing.

## Cards to create

Run `bop new implement <id>` for each, then edit `spec.md` with real content:

### team-arch
- `watchpaths-dispatcher` — extend dispatcher WatchPaths to cover all team-*/pending dirs (follow-on to spec 010)
- `jj-gitignore-audit` — audit all large files that break jj snapshot, add to .gitignore

### team-cli
- `bop-serve` — HTTP endpoint that accepts a spec body and creates a card (enables the ai-coding-job.yml pattern without self-hosted runner)
- `bop-doctor-checks` — expand `bop doctor` to check: nu version, jj version, adapter availability, launchd plist status

### team-quality
- `e2e-factory-test` — test that `bop factory install` + drop card → dispatcher fires → card moves to done/ without any manual daemon management
- `adapter-stress-test` — run 10 cards concurrently through the ollama adapter, verify no races, check orphan reaper handles PID collisions

### team-intelligence
- `model-routing` — route cards to cheap models (haiku) vs capable (sonnet/opus) based on card `cost` field in meta.json

## Steps

1. For each card above, run:
   ```
   bop new implement <id> --team <team>
   ```
   Then write a real spec into `.cards/<team>/pending/<id>.bop/spec.md`

2. Set glyphs via meta.json:
   - team-arch: ♥ suit (hearts), priority from task importance
   - team-cli: ♠ suit (spades)
   - team-quality: ♦ suit (diamonds)
   - team-intelligence: ♣ suit (clubs)

3. Verify `bop list --state pending` shows all seeded cards

4. Do NOT dispatch them — just seed. The human decides dispatch order.

## Acceptance

`bop list --state pending` shows ≥ 7 cards across teams.
Each card has a real `spec.md` (not the template placeholder).
`bop list --json --state pending | jq '.id'` outputs all card IDs.
