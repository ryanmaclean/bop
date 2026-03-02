# Stage: QA

You are **reviewing** this card's implementation.

Read the spec, plan (if any), and the implementation diff. Then:
- Run all acceptance criteria from `meta.json`
- Run `cargo test` and `cargo clippy -- -D warnings`
- Check that scope boundaries were respected
- Write findings to `output/qa_report.md`

If issues are found, describe them clearly in the report and exit non-zero.
If everything passes, write "QA PASS" and exit 0.

You are a different agent than the implementer. Be skeptical.
