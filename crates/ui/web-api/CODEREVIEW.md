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

## Code Quality Findings

| # | Severity | Category | Location | Description | Suggested fix |
| --- | --- | --- | --- | --- | --- |
| W-7 | Info | Security | `auth/refresh_cookie.rs:16` | `SameSite=Strict; HttpOnly; Secure` cookie attributes are correctly set. CSRF protection is properly handled via `SameSite=Strict`. | None needed. |
| W-8 | Info | Architecture | `lib.rs` | No CORS middleware present. Correct by design since the SvelteKit frontend is served from the same origin. | None needed. |
| W-9 | Info | Testing | `ocsp.rs:515` | `panic!("expected ResponderID::ByKey")` in `#[cfg(test)]` function. Acceptable for test assertions. | None needed. |

## Extensibility Findings

### ~~Significant: routes directly import DB entities~~ RESOLVED

**Location:** Route handlers throughout `src/routes/`

A `src/queries/` module has been introduced with typed `async fn` helpers encapsulating all
SeaORM entity access. Route handlers now delegate CRUD operations to:

- `queries/hosts.rs` — host queries
- `queries/provider_configs.rs` — provider config queries (with `validate_hooks_internal` shared helper)
- `queries/scheduled_tasks.rs` — scheduler queries
- `queries/services.rs` — service queries (approve/reject/deactivate/merge)
- `queries/software_items.rs` — software item queries (with validation helpers)
- `queries/update_history.rs` — update history queries

Handlers retain Axum concerns (extractors, response mapping, AppState side-effects for
notifications and WebSocket connections) but no longer construct `ActiveModel` instances or
import entity modules for CRUD. Dispatch operations (`trigger_update`, `check_versions`,
`check_versions_host`) still query entities directly because they are tightly coupled to
`AppState` and cannot be cleanly separated without injecting the state.

### ~~Significant: AppState exposes raw DatabaseConnection~~ RESOLVED

**Location:** `src/lib.rs`, `src/tenant_db.rs`

This finding has been addressed. The `db` field on `AppState` is now private, accessible only
through a `db()` accessor method. Tenant-scoped route handlers use the `TenantDb` Axum extractor
(defined in `src/tenant_db.rs`) which combines a `DatabaseConnection` with a verified `tenant_id`.
This makes it structurally impossible to access tenant data without first resolving the tenant
context through the extractor machinery.

```rust
// tenant_db.rs
pub struct TenantDb {
    db: DatabaseConnection,   // private — callers use .db()
    pub tenant_id: Uuid,
}

impl FromRequestParts<Arc<AppState>> for TenantDb {
    // Resolves TenantContext, then pairs the connection with tenant_id
}
```

Tenant-agnostic routes (PKI, authentication, WebSocket handlers) continue to use
`State(state)` and call `state.db()`. All handler code passes `cargo check` and
`cargo clippy --deny warnings` cleanly.

### Minor: provider registry used directly in handlers

**Location:** `src/routes/provider_configs.rs`

The `ProviderRegistry` is called directly in route handlers for `validate_config()`,
`mask_config_secrets_str()`, and `restore_config_secrets_str()`. It is not injected via `AppState`.

**Impact:** Acceptable for current architecture but limits future flexibility.

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
- **Complete OpenAPI annotations on all routes.**
  Every public endpoint has `#[utoipa::path]` with request/response schemas,
  security requirements, and tags.
- **Strong test coverage.**
  Auth middleware, JWT, API tokens, password verification, OCSP, device flow,
  reverse proxy header resolution, and router integration all tested.
- **Proper secret redaction.**
  `CaKeyStore::Debug` implementation redacts all key material. `Zeroizing`
  wrappers on private key fields.
- **Well-designed AppState** (`lib.rs`).
  Clean separation of public CA snapshot (clonable, debuggable) from private
  key store (non-Clone, non-Debug, behind `RwLock`). Construction uses a
  type-safe builder (`AppState::builder()...build()`) that returns `Result`,
  so missing fields are caught at startup rather than causing a runtime panic.
- **Feature-gated OIDC** -- `#[cfg(feature = "oidc")]` cleanly gates all OIDC functionality.
- **30+ route modules** organized by domain with consistent patterns.

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
| Mutex `.unwrap()` pattern | Pass | All instances use `.unwrap()` per approved exception |
