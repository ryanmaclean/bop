# Stage: Implement

You are **implementing** this card.

Read the spec (and plan, if present). Write code in the workspace.

Requirements:
- Work only inside the declared scope (see spec boundaries)
- Edit files using your tools, then build and test
- Run `cargo build` and `cargo test` before finishing
- Write output summary to `output/result.md`
- If tests fail, fix them. Do not leave broken code.

**Commit your work (jj):**
```
jj describe -m "feat: <what you did>"
jj new
```
Or if you prefer a single commit: `jj commit -m "feat: <what you did>"`

Do NOT use `git add` or `git commit` — this repo uses jj.

Exit 0 only when:
1. You have committed at least one change (jj log shows a new commit)
2. The implementation compiles and tests pass
3. Scope is met
