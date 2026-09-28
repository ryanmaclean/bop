# Core dependency and identity validation

The filesystem version binding guard rejects foreign card, request and attempt
records before invoking a backend or changing the binding. Attempt-zero
Create/Promote preparation remains valid for the first attempt. Retry identity
is preserved. This scope does not implement a filesystem backend or establish
dispatcher acceptance.

Two dependency changes make the actual core test graph compatible with the
required MIT/BSD/Apache license choices: standard-library home lookup removes
the dirs/option-ext edge, and the explicitly attributed Unicode 17 compatibility
implementation in `vendor/unicode-ident-xid` replaces unicode-ident's separately
licensed data. The latter is a different implementation, not a metadata-only
license change. Its upstream files and notices remain intact. Empty home paths
are rejected before credential/config resolution.

On native FreeBSD amd64, 2026-09-28, after reviewing the selected locked core
dependency graph, these commands succeeded under the shared builder lock:

```text
cargo test --offline --locked -p unicode-ident
cargo test --offline --locked -p bop-core
cargo clippy --offline --locked -p bop-core --all-targets -- -D warnings
cargo clippy --offline --locked -p unicode-ident --all-targets -- -D warnings
cargo fmt --all -- --check
```

Results: three replacement tests, 139 core unit tests and one core doctest;
zero failures. The real foreign-record regression passed. The two explicit
core command gates on the identity card are therefore satisfied. This does
not fabricate or change the card's separate workflow/QA state.

The replacement's exhaustive range-sweep test covers all 1,112,064 Unicode
scalars for both properties. This verifies lookup behavior against the pinned
licensed tables; it is not an independent certification of upstream Unicode
data. Independent source review found no blocking issue in the replacement.

Full CLI/workspace tests and Clippy remain unexecuted because other selected
CLI dependencies have licensing gates. The workspace formatting pass is not
a compilation or runtime pass. Windows home semantics, dispatcher workflows,
and full application acceptance remain separate work.
