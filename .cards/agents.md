# Agent Interface

How to participate in bop as an agent. No SDK required.

## You are here

When the dispatcher runs your adapter, you are inside a `.jobcard` directory
in `running/` state. Your rendered prompt is in `prompt.md`. Your workspace
(git worktree or card dir) is the working directory.

## Environment variables

The dispatcher sets these before spawning your adapter:

| Variable | Example | What |
|----------|---------|------|
| `BOP_CARD_ID` | `developer-sdk` | Your card's unique identifier |
| `BOP_CARD_DIR` | `/path/.cards/running/developer-sdk.jobcard` | Absolute path to your card |
| `BOP_CARDS_DIR` | `/path/.cards` | Root of all cards |
| `BOP_STAGE` | `implement` | Current stage (spec/plan/implement/qa) |
| `BOP_PROVIDER` | `claude` | Which provider is running you |
| `JOBCARD_MEMORY_NAMESPACE` | `developer-sdk` | Memory store namespace |
| `JOBCARD_MEMORY_OUT` | `/path/.../memory-out.json` | Write learnings here |

## Prompt template variables

Your `prompt.md` template can use `{{var}}` placeholders. The dispatcher
renders them before you see the file:

| Variable | Source | What |
|----------|--------|------|
| `{{spec}}` | `spec.md` | Task specification |
| `{{plan}}` | `plan.json` | Existing plan (if any) |
| `{{card_id}}` | `meta.json` | Your card ID |
| `{{card_dir}}` | runtime | Absolute path to your card |
| `{{stage}}` | `meta.json` | Current stage |
| `{{stage_instructions}}` | `.cards/stages/{stage}.md` | Stage-specific guidance |
| `{{stage_index}}` | `meta.json` | 1-based position in stage chain |
| `{{stage_count}}` | `meta.json` | Total stages |
| `{{acceptance_criteria}}` | `meta.json` | Tests that must pass |
| `{{prior_stage_output}}` | `output/prior_result.md` | What the previous stage produced |
| `{{depends_output}}` | dependency cards | Concatenated `output/result.md` from all `depends_on` cards |
| `{{memory}}` | `.cards/memory/` | Accumulated learnings |
| `{{worktree_branch}}` | `meta.json` | Git branch name |
| `{{provider}}` | runtime | Provider name |
| `{{agent}}` | `meta.json` | Agent type |
| `{{system_context}}` | `.cards/system_context.md` | Shared orientation (prepended) |

## What to produce

| Output | Path | Purpose |
|--------|------|---------|
| Primary result | `output/result.md` | Main deliverable. Downstream cards see this via `{{depends_output}}` |
| Child cards | `output/cards.yaml` | Spawn child cards (dispatched after you finish) |
| Learnings | write to `$JOBCARD_MEMORY_OUT` | Persisted across cards in the same namespace |
| Code changes | in the worktree | Merged by merge-gate after acceptance criteria pass |
| Stdout/stderr | captured automatically | Goes to `logs/stdout.log` and `logs/stderr.log` |

## Exit codes

| Code | Meaning | What happens |
|------|---------|--------------|
| `0` | Success | Card moves to `done/`, merge-gate runs acceptance criteria |
| `75` | Rate limited | Card returns to `pending/`, provider rotated, 5min cooldown |
| Any other | Failure | Card moves to `failed/` with `failure_reason` in meta.json |

## Dependency chain

If your card has `depends_on: ["card-a", "card-b"]` in meta.json:

1. The dispatcher **will not run you** until card-a and card-b are in `done/` or `merged/`
2. Your prompt gets `{{depends_output}}` = concatenated `output/result.md` from both cards
3. You can also read dependency output directly: `$BOP_CARDS_DIR/done/card-a.jobcard/output/result.md`

## Creating cards without the CLI

No SDK needed. A valid card is:

```sh
mkdir -p .cards/pending/my-task.jobcard/{logs,output}
cat > .cards/pending/my-task.jobcard/meta.json << 'EOF'
{
  "id": "my-task",
  "stage": "implement",
  "created": "2026-03-01T00:00:00Z",
  "provider_chain": ["claude"]
}
EOF
cat > .cards/pending/my-task.jobcard/spec.md << 'EOF'
# My Task
Do the thing.
EOF
```

The dispatcher will pick it up on the next poll.

## Filesystem as database

Cards are directories. State transitions are `mv` (atomic `fs::rename`).
No daemon, no connection string, no SDK.

```
.cards/
├── drafts/      ← staging area (dispatcher ignores)
├── pending/     ← ready for dispatch
├── running/     ← currently executing
├── done/        ← succeeded, awaiting merge
├── merged/      ← landed on main
├── failed/      ← errored out
├── templates/   ← card blueprints (COW-cloned on `bop new`)
├── memory/      ← persistent key-value per namespace
└── stages/      ← stage-specific prompt instructions
    ├── spec.md
    ├── plan.md
    ├── implement.md
    └── qa.md
```

## Storage (APFS on macOS)

- **Templates are COW-cloned** — 50 cards from one template = 1× disk usage until modified
- **Terminal-state cards are LZFSE-compressed** — transparent to all readers, 3-5× savings on text
- **State transitions are atomic** — `fs::rename`, journaled, <1ms
- **Finder tags** — every card tagged with `[state, "bop"]` for Smart Folder queries
