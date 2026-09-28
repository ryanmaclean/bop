# Remove the unused CLI matcher dependency

Spec 041 requires a case-insensitive ID substring filter. The existing filter
implements that behavior using standard-library strings; no source uses nucleo.
Remove the unused dependency and its obsolete license exceptions. Preserve
filter behavior and the explicit post-quantum TLS requirement.

Verify the selected dependency metadata and run the existing actual filter
tests on the approved native builder. This is partial dependency remediation:
full CLI license review, `make check`, warning-free Clippy and actual dispatcher
card processing remain required. Do not advance this card to QA/done based on
isolated tests. No dispatcher or service is launched by creating this card.
