# Unused matcher dependency removal

Spec 041 explicitly requires case-insensitive ID substring matching. The actual
`ui/widgets/filter.rs` implements that with standard-library strings; no source
uses nucleo. Removing the direct dependency preserves that implementation and
the post-quantum TLS requirement. The lockfile also drops its unused rayon and
crossbeam-deque/epoch closure, without upgrading remaining packages. Obsolete
nucleo, nucleo-matcher and already-absent option-ext MPL exceptions are removed.

On the existing FreeBSD builder, in `/var/tmp/bop-unused-matcher-0928`, offline
metadata and the locked native normal/build/dev tree succeeded. All three
exception packages are absent from complete root-workspace metadata. The
standalone WASM Zellij plugin has a separate lockfile and is outside this
claim; its option-ext dependency is not removed here. Under the shared native
build lock, the exact unchanged filter source was compiled with `rustc --test`:
all six existing tests passed. This uses only the standard library, not the
full CLI dependency graph. No laptop build or test was run.

This does not close full CLI acceptance. The remaining pre-existing license
allowances/exceptions in `deny.toml` are not approval under the current strict
MIT/BSD/Apache-2.0 requirement. The native graph still contains unresolved
watcher, TLS, ICU/IDNA and hash implementation license paths. Full `make check`,
CLI Clippy, interactive acceptance and actual dispatcher processing remain
unexecuted. The card remains failed at implementation stage; no QA approval,
main integration or ready PR is claimed.

Evidence is saved in the coordinator's `status-2026-09-27` directory:
`bop-unused-matcher-evidence.tar`, `bop-unused-matcher-native.nu` and
`bop-unused-matcher.diff`. The first staging attempt used an incorrect filter
path and failed before any remote execution; the corrected transfer and native
run succeeded.
