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

### Significant: routes directly import DB entities

**Location:** Route handlers throughout `src/routes/`

Route handlers directly import and query SeaORM entity models:

```rust
use uptrakit_shared_db::entity::prelude::*;
use uptrakit_shared_db::entity::provider_config;
```

For example, `routes/provider_configs.rs` constructs `provider_config::ActiveModel` directly in
request handlers, and `routes/hosts.rs` imports `host`, `service`, and `service_host` entities
for multi-table queries.

**Impact:** Database schema changes directly break API handlers. There is no data access layer
to absorb schema evolution. This makes it harder for external developers to understand the API
layer without also understanding the database schema.

### Significant: AppState exposes raw DatabaseConnection

**Location:** `src/lib.rs:157-163`

```rust
pub struct AppState {
    pub db: DatabaseConnection,
    // ...
}
```

Any handler with access to `AppState` can execute arbitrary database queries. There is no
query scoping, no repository pattern, and no abstraction boundary between the API layer and the
database.

**Impact:** Multi-tenancy isolation relies on manual tenant ID filtering in every query. A missed
filter could leak data across tenants. External developers extending the API must understand the
full entity schema and tenant isolation patterns.

### Minor: provider registry used directly in handlers

**Location:** `src/routes/provider_configs.rs`

The `ProviderRegistry` is called directly in route handlers for `validate_config()`,
`mask_config_secrets_str()`, and `restore_config_secrets_str()`. It is not injected via `AppState`.

**Impact:** Acceptable for current architecture but limits future flexibility.

### Extensibility recommendation: data access layer

Consider introducing a repository or data-access pattern:

```rust
pub trait ProviderConfigRepository: Send + Sync {
    async fn get(&self, tenant_id: Uuid, id: Uuid) -> Result<ProviderConfigResponse>;
    async fn create(&self, tenant_id: Uuid, req: CreateProviderConfigRequest) -> Result<ProviderConfigResponse>;
    // ...
}
```

This would:

1. Decouple route handlers from the database schema.
2. Centralize tenant isolation logic.
3. Make it easier for external developers to understand the API without studying SeaORM entities.
4. Enable testing routes with mock repositories.

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
- **Well-designed AppState** (`lib.rs:156-206`).
  Clean separation of public CA snapshot (clonable, debuggable) from private
  key store (non-Clone, non-Debug, behind `RwLock`).
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
