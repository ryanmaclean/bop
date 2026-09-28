# Unicode identifier compatibility implementation

This local, unpublished `unicode-ident` package implements the two free
functions and `(u8, u8, u8)` version constant consumed by the locked Rust
macro dependencies. It is **not** a copy or license relabeling of dtolnay's
unicode-ident implementation or data. The Cargo patch selects this separately
implemented package explicitly. Its name/version satisfy the existing
dependency contract; they do not identify an official upstream release.

`src/tables.rs`, `LICENSE-MIT`, `LICENSE-APACHE`, and `COPYRIGHT` are unchanged
files from unicode-rs/unicode-xid commit
`0aeadace2aa539b1488266c9a17f5c695c5faace`:
https://github.com/unicode-rs/unicode-xid/tree/0aeadace2aa539b1488266c9a17f5c695c5faace
The checked-in Rust tables and lookup code carry their own explicit MIT OR
Apache-2.0 notice. The local wrapper is available under those same alternatives.
There are no third-party production or test dependencies in this package.

The table SHA256 is
`3702df0a7ddaf60f3da80626cd5ec5a985a3b39ec833d08a6e6ad397be6698e6`.
The table covers Unicode **17.0.0**, matching the replaced API's data version.
The published unicode-xid 0.2.6 package instead contains Unicode 16 data and
must not silently replace these pinned files. No ASCII-only approximation,
raw Unicode data import, or generator run is used here.

Tests exercise an independent range sweep against the lookup for all 1,112,064
Unicode scalars, start/continue containment, version typing, and identifier
boundaries including a Unicode 17 Tai Yo letter. These establish lookup/API
behavior; actual consumer compilation and BOP tests are separate gates.

Maintenance: keep all notices, re-review the exact source grants and data
version before updating, verify the complete selected dependency graph and
run this package's tests plus the real consumers. A future unicode-ident API
change requires explicit compatibility review. Binary search may be slower
than upstream unicode-ident's compressed lookup; no performance parity is
claimed. Other repository license/build gates are unaffected.
