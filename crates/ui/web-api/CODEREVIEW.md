# Code Review: uptrakit-web-api

## Review metadata

| Field | Value |
| --- | --- |
| Date | 2026-02-17 |
| Scope | Full crate review (`crates/ui/web-api/`) |
| Rating | **Very Good** |
| Reviewer | AI-assisted (Claude Code) |

## Executive summary

The web-api crate is well-structured with excellent middleware layering,
comprehensive authentication, and complete OpenAPI annotations. A few
medium-severity items were found, mostly around silent error fallbacks and a
non-standard lock-poisoning pattern. No critical issues.

## Findings

| # | Severity | Category | Location | Description | Suggested fix |
| --- | --- | --- | --- | --- | --- |
| W-1 | Medium | Safety | `routes/ocsp.rs:42` | `unwrap_or_default()` on `percent_decode_str().decode_utf8()`. If percent-decoded bytes are invalid UTF-8, this silently produces an empty string. Downstream base64 decode then fails and returns a `malformedRequest` OCSP response, so the end behavior is correct, but the error path is unclear and makes debugging harder. | Return the `malformedRequest` response directly on UTF-8 decode failure with a `tracing::debug!` log, instead of falling through to the base64 path. |
| W-2 | Medium | Safety | `ocsp.rs:218` | `response.to_der().unwrap_or_default()` in `build_error_response`. If DER encoding of an error OCSP response fails, the client receives an empty body with `application/ocsp-response` content type. | Return an HTTP 500 status instead of a zero-length OCSP body, or log the DER failure at `error` level so it is observable. |
| W-3 | Medium | Standards | `middleware/rate_limit.rs:121-123` | `.unwrap_or_else(\|poisoned\| poisoned.into_inner())` on `Mutex::lock()`. Per AGENTS.md, the approved exception for mutex locks is `.unwrap()` only, since `panic = "abort"` in the release profile makes lock poisoning impossible. The explicit poisoned-state recovery is non-standard and misleading. | Replace with `.unwrap()` to match the approved exception pattern. |
| W-4 | Low | Code Quality | `middleware/require_auth.rs:100,107,110,133,136,144` | Error messages contain trailing `\n` characters (e.g., `"Invalid or revoked API token\n"`). These newlines end up in JSON response bodies, which is non-standard. | Remove trailing `\n` from all `AuthFailure` string literals. |
| W-5 | Low | Code Quality | `routes/api_tokens.rs:42,79` | `format(&format).unwrap_or_default()` for timestamp formatting. Acceptable per Pattern 16 (display fallback), but a failed format produces an empty string. | Consider `.unwrap_or_else(\|_\| dt.to_string())` for a more informative ISO 8601 fallback. |
| W-6 | Low | Code Quality | `routes/scheduler.rs:31,35,36` | Same timestamp formatting pattern as W-5 with `unwrap_or_default()`. | Same suggestion: use `.to_string()` as fallback instead of empty string. |
| W-7 | Info | Security | `auth/refresh_cookie.rs:16` | `SameSite=Strict; HttpOnly; Secure` cookie attributes are correctly set. CSRF protection is properly handled via `SameSite=Strict` (browser will not attach the cookie on cross-origin requests). No additional CSRF token is needed. | None needed. |
| W-8 | Info | Architecture | `lib.rs` | No CORS middleware present. This is correct by design since the SvelteKit frontend is served from the same origin (same-origin policy applies). | None needed. Document the assumption in the architecture docs if not already present. |
| W-9 | Info | Testing | `ocsp.rs:515` | `panic!("expected ResponderID::ByKey")` appears inside a `#[cfg(test)]` function. This is test-only code and acceptable for assertions. | None needed. |

## Strengths

- **Excellent middleware layering** (`lib.rs:604-618`).
  `request_log` -> `resolve_ip` -> `rate_limit` -> `resolve_proxy_headers` ->
  `require_auth` -> handlers. Each concern cleanly separated.
- **Comprehensive rate limiting with DB-backed store + local fallback**
  (`middleware/rate_limit.rs`, `auth/rate_limit.rs`).
  DB store for cross-instance consistency; local in-memory fallback if DB is
  unavailable.
- **Token denylist with periodic purge.**
  `TokenDenylist` provides immediate JWT revocation on logout/deactivation.
  Periodic purge called from controller (`tasks.rs:128`).
- **Complete OpenAPI annotations on all routes.**
  Every public endpoint has `#[utoipa::path]` with request/response schemas,
  security requirements, and tags.
- **Strong test coverage.**
  Auth middleware, JWT, API tokens, password verification, OCSP, device flow,
  reverse proxy header resolution, and router integration all tested.
- **Proper secret redaction.**
  `CaKeyStore::Debug` implementation redacts all key material. `Zeroizing`
  wrappers on private key fields.
- **Well-designed AppState** (`lib.rs:156-206`).
  Clean separation of public CA snapshot (clonable, debuggable) from private
  key store (non-Clone, non-Debug, behind `RwLock`).

## AGENTS.md compliance checklist

| Rule | Status | Evidence |
| --- | --- | --- |
| No `unsafe` | Pass | No `unsafe` blocks found |
| No `unwrap()`/`panic!()` in production code | Pass | Only in `#[cfg(test)]` modules; mutex `.unwrap()` per approved exception |
| No `#[allow()]` | Pass | No `#[allow()]` attributes found |
| No raw SQL | Pass (approved exception) | Rate limiter uses `Statement::from_sql_and_values()` for parameterized upsert (documented in AGENTS.md) |
| Typed error enums with `thiserror` | Pass | `OcspError`, `AuthError`, `CertSignerError`, etc. |
| `rootcause` context propagation | Pass | `.context()`, `.context_to()`, `bail!()` throughout |
| `impl_report_conversion!` at boundaries | Pass | `ocsp.rs`, `auth/error.rs`, `cert_signer.rs` |
| Secrets never logged | Pass | Error messages sanitized; key material redacted in Debug impls |
| OpenAPI annotations | Pass | All routes annotated with `#[utoipa::path]` |
| Mutex `.unwrap()` pattern | **Finding W-3** | One instance uses `unwrap_or_else` instead of `.unwrap()` |
