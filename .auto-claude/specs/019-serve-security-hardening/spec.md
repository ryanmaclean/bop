# bop serve: security hardening

## Context

`crates/bop-cli/src/serve.rs` (added in Wave 6) has several security issues
identified by the ideation agent:

- `auth != bearer` is a direct string compare — vulnerable to timing attacks
- No request body size limit — DoS via huge JSON payload
- Card ID validation rejects `/` and `\` but not URL-encoded variants (`%2F`, `%5C`,
  `..`, null bytes) — path traversal risk
- `BOP_SERVE_TOKEN` is optional — server runs unauthenticated by default
- No rate limiting — endpoint can be flooded
- No security response headers

## What to do

1. **Constant-time token comparison** (`serve.rs`):
   - Replace `auth != bearer` with `subtle::ConstantTimeEq` or a manual
     constant-time byte comparison loop. Add `subtle = "2"` to `Cargo.toml`.

2. **Mandatory auth**: if `BOP_SERVE_TOKEN` is not set, log a warning and generate
   a random token at startup (print it once to stderr: `BOP serve token: <token>`).
   Never run unauthenticated.

3. **Request body size limit**: add a `ContentLengthLimit` or `axum::extract::DefaultBodyLimit`
   middleware capping requests at 64 KiB.

4. **Card ID sanitization**: reject IDs containing `..`, `%`, null bytes, or any
   character outside `[a-zA-Z0-9_-]`. Return 400 with a clear message.

5. **Security response headers**: add middleware that injects:
   - `X-Content-Type-Options: nosniff`
   - `X-Frame-Options: DENY`
   - `Cache-Control: no-store`

6. **Rate limiting**: add `tower_governor` or a simple `Arc<Mutex<HashMap<IpAddr, Instant>>>`
   allowing max 10 requests/minute per IP. Return 429 on excess.

7. Run `make check` — must pass with no clippy warnings.

8. Write `output/result.md` summarising each fix.

## Acceptance

- Token comparison is constant-time
- Server refuses to start without a token (generates one if unset)
- Body > 64 KiB returns 429/413
- Card IDs with `..` or `%2F` return 400
- Security headers present on all responses
- `make check` passes
- `output/result.md` exists
