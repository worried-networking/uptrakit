# AppState Architecture and Sub-State Decomposition

## Overview

`AppState` (`crates/ui/web-api/src/app_state.rs`) is the monolithic Axum router state shared
across all route handlers. It is registered as `Arc<AppState>` on the router.

To avoid coupling every route handler to the full 26-field struct, five focused sub-states are
defined with hand-written `FromRef<Arc<AppState>>` implementations. Route handlers declare only
the sub-states they actually need.

## Focused Sub-States

| Sub-state | Field path on `AppState` | Purpose |
| --- | --- | --- |
| `DbState` | `state.db` | Wraps `DatabaseConnection`; exposes `db()` accessor |
| `AuthState` | `state.auth` | JWT, session, rate-limit, device-flow stores |
| `BroadcastState` | `state.broadcast` | SSE event broadcaster, device-flow broadcaster |
| `CertState` | `state.cert` | CA snapshot, CRL cache, CA key store |
| `OidcState` | `state.oidc` | OIDC provider stores (`#[cfg(feature = "oidc")]`) |

All five implement `FromRef<Arc<AppState>>` (not derived — `Arc<Struct>` is unsupported by the
derive macro). The `OidcState` impl is gated with `#[cfg(feature = "oidc")]`.

## Using Sub-States in Route Handlers

Declare only the sub-state you need:

```rust
// Database access only
pub async fn list_roles(State(db): State<DbState>, ...) -> Response {
    Role::find().all(db.db()).await
}

// Broadcast + tenant DB (tenant-scoped handler)
pub async fn create_host_tag(
    State(broadcast): State<BroadcastState>,
    tenant_db: TenantDb,
    ...
) -> Response {
    broadcast.event_broadcaster.send(tenant_id, event).await;
}

// Multiple sub-states
pub async fn readyz(
    State(db): State<DbState>,
    State(cert): State<CertState>,
) -> impl IntoResponse { ... }
```

**Rule:** A handler signature must **not** mix `State<Arc<AppState>>` with any focused
`State<SubState>`. The CI hard-gate `ci/verify_handler_state_contract.sh` enforces this.

## Service Extractors

Two typed service extractors eliminate manual `Service::new(state.db().clone())` boilerplate
in route handlers. Both are defined in `crates/ui/web-api/src/extract.rs`.

| Extractor | Wraps | Bound |
| --- | --- | --- |
| `SessionSvc` | `SessionService` | `DbState: FromRef<S>` |
| `ApiTokenSvc` | `ApiTokenService` | `DbState: FromRef<S>` |

Both implement `Deref<Target = Service>` so method calls work transparently.

```rust
// Add SessionSvc to handler parameters — no manual construction needed
pub async fn register(
    State(state): State<Arc<AppState>>,
    session_svc: SessionSvc,
    Validated(req): Validated<RegisterRequest>,
) -> impl IntoResponse {
    session_svc.create_refresh_token(user_id, ...).await
}
```

## Service Traits for Controller Embedding

`uptrakit-web-api-auth` defines two `async_trait` traits that wrap the concrete service
implementations behind a `dyn`-safe interface. This allows downstream consumers (controllers,
test stubs) to depend on the trait without pulling in Axum.

- `SessionOps` — 6 methods (create/verify/rotate/revoke refresh tokens, delete sessions,
  cleanup expired)
- `ApiTokenOps` — 4 methods (create/list/revoke tokens, verify token)

Both are re-exported from `uptrakit_web_api_auth::auth::{SessionOps, ApiTokenOps}`.

## DB Access Policy

Every `async fn` in `crates/ui/web-api/src/routes/` is classified in
`crates/ui/web-api/db_access_policy.toml`:

| Classification | Meaning |
| --- | --- |
| `tenant-agnostic` | Uses `State<DbState>`, never `TenantDb` or full `AppState` |
| `tenant-scoped` | Uses `TenantDb`, never `State<DbState>` or full `AppState` |
| `no-db` | No database access at all |
| `full-state` | Uses `State<Arc<AppState>>` (uncovered fields; migration pending) |
| `ignore` | Non-handler helper function; not validated |

The CI script `ci/verify_db_access_policy.py` checks every handler against this policy on
every CI run. To update after adding or changing a handler:

1. Change the handler signature.
2. Update the corresponding entry in `db_access_policy.toml`.
3. Run `python3 ci/verify_db_access_policy.py` — must exit 0.

To seed classifications for a new route file:

```bash
python3 ci/seed_db_access_policy.py   # auto-classifies; review and correct
```

## Exception Inventory (Phase 2 Complete)

The following retain `State<Arc<AppState>>` because they access fields not yet covered by any
focused sub-state (`settings`, `default_tenant_id`, `plugin_ops`, `service_connections`,
`notification_service`, `shutdown_token`, `pki_path`, `extension_registry`, etc.):

- **Middleware** (documented v1 exceptions, not checked by CI scripts):
  - `require_auth`, `audit_log`, `resolve_ip`, `resolve_proxy_headers`
  - `require_auth.rs` retains one `ApiTokenService::new()` call (scoped exception)
- **Group A partial**: `auth.rs`, `device_auth.rs`, `oidc_auth.rs` — extractors added, but
  `State<Arc<AppState>>` kept for uncovered fields
- **Group D routes**: ~34 route files with uncovered fields (see `db_access_policy.toml`)

Phase 3 (AppState privatisation) is blocked until a follow-up RFC covers the remaining fields.

## Related Documents

- [Coding Standards](coding-standards.md) — handler patterns and DB query rules
- [Testing](testing.md) — test harness and integration tests
- [Security — Auth and Authorization](../security/auth-and-authorization.md)
