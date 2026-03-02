# bolt.diy Prompt Export

Source repository: [stackblitz-labs/bolt.diy](https://github.com/stackblitz-labs/bolt.diy)

Snapshot metadata is in `source_info.txt`.

## What Was Collected

- Full source snapshots of prompt-bearing files under `source/`
- Prompt index in `catalog.tsv` covering system/discuss/enhancer/template/context/summary/UI prompt surfaces

## Key Prompt Files

- `source/app/lib/common/prompts/prompts.ts`
- `source/app/lib/common/prompts/new-prompt.ts`
- `source/app/lib/common/prompts/optimized.ts`
- `source/app/lib/common/prompts/discuss-prompt.ts`
- `source/app/lib/common/prompt-library.ts`
- `source/app/routes/api.enhancer.ts`
- `source/app/utils/selectStarterTemplate.ts`
- `source/app/lib/.server/llm/select-context.ts`
- `source/app/lib/.server/llm/create-summary.ts`
- `source/app/components/chat/ExamplePrompts.tsx`

## Notes

- `stream-text.ts` is included because it controls runtime prompt routing (`build` uses selected system prompt, `discuss` uses `discussPrompt()`).
- Starter template imports can inject additional instructions from `.bolt/prompt` in template repos; those are external to bolt.diy core and not part of this snapshot.
