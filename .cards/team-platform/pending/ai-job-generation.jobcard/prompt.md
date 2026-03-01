You are working inside a Rust Cargo workspace (jobcard-core + jc crates).
The project root is your working directory.

# AI-assisted Job Card Generation

**Phase**: phase-4  **Priority**: wont  **Complexity**: high  **Impact**: medium

## Description
Natural language interface (`jc create --from-description 'Add a dark mode toggle to the settings page'`) that uses an LLM to generate a complete job card (spec.md, template selection, provider chain, acceptance criteria) from a plain-text description.

## Rationale
Lowering the barrier to creating well-formed job cards is valuable long-term but requires significant prompt engineering and could produce inconsistent results in early iterations. Deferred to phase 4 when the core tool is mature and job card schemas are stable.

## User Stories
- As a solo developer, I want to describe a coding task in plain English and have JobCard generate a job card so that I don't have to manually write specs
- As a non-expert user, I want AI-assisted card creation so that I can use JobCard without learning the job card schema

## Acceptance Criteria
- `jc create --from-description '<text>'` generates a complete job card draft
- Generated cards include spec.md, acceptance_criteria, and suggested template
- User is shown the generated card for review before it is written to pending/
- Generation uses the configured default provider

Acceptance criteria:
`jc create --from-description '<text>'` generates a complete job card draft
Generated cards include spec.md, acceptance_criteria, and suggested template
User is shown the generated card for review before it is written to pending/
Generation uses the configured default provider
