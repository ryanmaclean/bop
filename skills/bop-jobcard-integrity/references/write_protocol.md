# Atomic Write Protocol

Use this protocol only when a `bop` command is unavailable for the mutation.

1. Read current card and validate JSON.
2. Acquire lock (dispatcher/card lock if available).
3. Write updated JSON to a temp file in same directory.
4. Validate temp JSON (`jq . temp.json`).
5. Atomically rename temp file over target (`mv temp meta.json`).
6. Re-read with `bop inspect <id>` and confirm stage/state unchanged unless intended.
7. Append an operation note to `logs/` for traceability.

Never edit `meta.json` with line-based text tools in place.
