# Smoke test: add a hello_bop() function to bop

Add a trivial pub function `hello_bop() -> &'static str` to
`crates/jobcard-core/src/lib.rs` that returns "hello from bop".

Add a unit test `test_hello_bop` that asserts the return value.

Run `cargo test -p jobcard-core` to verify.

Write output/result.md with what you did.
Commit with jj.
