# Stage: Generate

You are generating the executable delivery roadmap.

Produce both files:
- `output/result.md` (human-readable summary)
- `output/roadmap.json` (structured board data)

`output/result.md` must include:
- Ordered milestones/phases
- Scope for each milestone (what ships)
- Dependencies and sequencing constraints
- Risks and mitigation plan per milestone
- Definition of done for each milestone
- Explicit out-of-scope items

`output/roadmap.json` must include:
- `phases`: ordered phase objects (`id`, `name`, optional `goal`)
- `features`: array of features with:
  - `id`
  - `title`
  - `description`
  - `status`: one of `under_review`, `planned`, `in_progress`, `done`
  - `priority`: one of `must`, `should`, `could`
  - `phase`: phase `id`
  - optional `rationale`, `acceptance_criteria`, `dependencies`

Generation defaults:
- Newly generated features should default to `under_review` unless explicitly
  ready for immediate planning.
- Prefer realistic phase assignment over leaving items unphased.

Optimize for handoff quality and execution clarity.
Do not implement code in this stage.
