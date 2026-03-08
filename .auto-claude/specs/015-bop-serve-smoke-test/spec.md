# bop serve smoke test: curl POST creates card in pending/

## Context

`bop serve` was recently implemented (`crates/bop-cli/src/serve.rs`). It exposes a
`POST /cards/new` endpoint that creates bop cards. The implementation needs a smoke test
that actually starts the server, POSTs to it, and verifies a card appears in `.cards/pending/`.

## What to do

1. Read `crates/bop-cli/src/serve.rs` to understand the current implementation.
2. Add an integration test (or extend existing unit tests) that:
   - Binds the server on a random port (or fixed test port like 18080)
   - POSTs `{"id": "smoke-test-serve", "spec": "# test\nsmoke test spec"}`
   - Asserts HTTP 201 response
   - Asserts `.cards/pending/smoke-test-serve.bop/` directory is created
   - Cleans up after the test
3. Run `cargo test -p bop` to verify the new test passes.
4. Run `make check` — must pass clean.
5. Write `output/result.md` with what was tested and results.

## Acceptance

- `cargo test -p bop` passes including the new serve integration test
- `make check` exits 0
- `output/result.md` exists
