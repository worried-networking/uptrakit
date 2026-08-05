# AGENTS — AI Agent Guide for `uptrakit-web-api`

This file scopes AI agent context to the HTTP API crate at `crates/ui/web-api/`. For project-wide
rules, architecture invariants, cross-crate layout, and quality gates, **read the root
[`AGENTS.md`](../../../AGENTS.md) first** — the root file takes precedence on any topic not covered
here. See also the sibling [`frontend/AGENTS.md`](../../../frontend/AGENTS.md) for the SvelteKit
client that consumes this crate's REST/WebSocket surface.

## What this crate is

`uptrakit-web-api` is the HTTP/WebSocket API layer: Axum routes, middleware (security headers,
request ID/logging, IP resolution, rate limiting, proxy header resolution, auth enforcement, audit
logging, permission checks, tenant context), `AppState` and its sub-states, the OpenAPI router, and
the shared test harness. Authentication primitives (JWT/session/OIDC/tokens/permissions) live in the
sibling `uptrakit-web-api-auth` crate; DB query modules and `TenantDb` live in
`uptrakit-web-api-queries` / `uptrakit-shared-db` and are re-exported here.

## Handler orientation

Route handlers live under `src/routes/`, one module (or module tree) per resource. WebSocket message
handlers for the service connection live under `src/routes/service_ws/handler/`: update-lifecycle
handlers are in `updates/` (one message type per file — `started.rs`, `output.rs`, `result.rs`,
`batch.rs`, `stdin.rs`, plus supporting `dispatch.rs`/`replay.rs`/`finalize.rs`/`audit.rs`), and
common (non-update) message handlers are in `messages/` (ping, certificate renewal, host reporting,
version-check results, discovery results, plugin config reports). The two per-connection session
loops are `session_authenticated.rs` (post-cert operational loop) and `session_enrolled.rs`
(pre-cert enrollment loop); per-connection message dispatch runs through `message_processor.rs`
(`MessageProcessor`, `ProcessorResponse`, `ProcessorAction`). If you need to find the handler for a
given wire message, grep the message type name (e.g. `UpdateStarted`, `ReportHosts`) across
`service_ws/handler/` rather than guessing a file name.

Non-WebSocket REST handlers are grouped by resource directly under `src/routes/` (e.g. `hosts.rs`,
`services.rs`, `software_items.rs`, `settings_*.rs`). When a resource module grows large it is split
into a facade (`mod.rs` with `pub use` re-exports) plus focused submodules — the same pattern as
`service_ws/handler/updates/mod.rs` and `messages/mod.rs`. Follow existing splits as precedent rather
than inventing a new layout.

## Sub-state extractors (do not use full `AppState` by default)

Handlers must declare only the focused sub-state(s) they actually need, not
`State<Arc<AppState>>`. The available sub-states — `DbState`, `AuthState`, `CertState`,
`BroadcastState`, `OidcState`, `AuditEmitterState`, `PluginOpsState`, `GlobalProvidersState`,
`AccessState` (action-extractor access to the `AccessEngine`) — are
defined and wired in `src/app_state.rs`; read that file rather than relying on a count here, since
new sub-states get added over time. Most sub-states implement `FromRef<Arc<AppState>>` directly in
`app_state.rs`; `DbState` and `AuthState` instead come from blanket generic
`impl<S: DbStateSource/AuthStateSource> FromRef<Arc<S>>` implementations in `uptrakit-controller-core`
(`crates/ui/controller-core/src/db.rs`, `src/auth/mod.rs`), so `AppState` only needs to implement the
small `*Source` trait.

**Enforced contract:** a single handler signature must never mix a full `State<Arc<AppState>>`
extractor with a sub-state extractor. `ci/verify_handler_state_contract.sh` checks this on every CI
run — if you add a handler that needs full state, look for a narrower sub-state first; if none
exists, add one in `app_state.rs` rather than reaching for `Arc<AppState>`.

## Typed extractors and permissions

Common typed extractors live in `src/extract.rs`: `Validated<T>` (JSON body deserialize +
`Validate::validate()`, returns HTTP 400 on failure — do not call `.validate()` manually after a
plain `Json<T>` extraction), `ClientIp` / `ProxyIp`, `ServiceIdentity`, and `SessionSvc` /
`ApiTokenSvc` (eliminate manual `Service::new()` construction in handlers).

Authorization uses typed extractors, never inline checks. Route families enforce through
`src/middleware/action.rs`'s `action_extractor!` macro (e.g. `CanReadHosts`), backed by the
`AccessEngine`, and declare a native `security(("oauth2" = ["hosts:read"]), ("developer_token" = []))`
OpenAPI requirement — gated by `ci/verify_action_security_declarations.py` (rules R1–R5). Operations whose
authorization is an OR of alternatives (batch actions, `list_plugin_types`, plugin-type-settings reads)
enforce inline via `authorize_any` and declare one single-scope `oauth2` requirement per alternative, with
**no** action extractor; dynamic surface wrappers declare the authenticated-only form
(`security(("oauth2" = []), ("developer_token" = []))`) plus `extensions(("x-action-dynamic" = json!(true)))`.
The legacy `src/middleware/permission.rs` `permission_extractor!` model (`x-required-permission` extension +
`security(("bearer_token" = []))`) survives only in `users.rs`, `roles.rs`, and `access_presets.rs` until
M1.6a/M1.6b. The sole exception in either model is handlers with a custom auth path (e.g. WebSocket handlers
reading a `?token=` query parameter before the normal extractor chain runs) — these gate inline through the
engine and must carry a `// APPROVED: custom auth path` comment. Full rationale in
[`docs/security/auth-and-authorization.md`](../../../docs/security/auth-and-authorization.md).

## OpenAPI rules

Handlers are only included in the generated OpenAPI spec if registered via `.routes(routes!(...))`
on an `OpenApiRouter` in `src/router.rs` **before** `split_for_parts()` is called — a handler with a
correct `#[utoipa::path]` annotation that is never passed to `routes!()` is silently absent from the
spec and from the generated frontend client. When adding or changing a route, register it in
`router.rs` in the same commit.

Query parameters must be declared via `params(<IntoParamsStruct>)` referencing a struct that derives
`utoipa::IntoParams`, never as an inline hand-maintained parameter list (ADR-0025). This is enforced
by `ci/verify_no_inline_query_params.sh`.

After any route addition, signature change, or contract change, run `./scripts/regen-api.sh` and
commit both `openapi.json` and the regenerated `frontend/src/lib/api/generated/` output in the same
change — do not hand-edit either. See
[`docs/api/http-web-api.md`](../../../docs/api/http-web-api.md) for the endpoint reference.

## Database access policy

`db_access_policy.toml` (crate root) classifies every route handler function as one of
`tenant-agnostic` (uses `State<DbState>` only), `tenant-scoped` (uses `TenantDb` only), `no-db`, or
`full-state` (uses `State<Arc<AppState>>`; migration-pending, avoid adding new entries here), plus
`ignore` for non-handler helper functions. `python3 ci/verify_db_access_policy.py` enforces that every
handler has an entry and that the entry matches the actual extractor used in the signature. Update the
TOML in the same commit as any handler signature change — do not leave it stale.

## Tenant isolation

Use `TenantDb` (defined in `uptrakit-shared-db`, re-exported here) for every tenant-scoped query:
`tenant_db.find::<E>()`, `.find_by_id::<E>(id)`, `.update_many::<E>()`, `.delete_many::<E>()`. Never
call `Entity::find().all(tenant_db.db())` directly on a `TenantScoped` entity — `tenant_db.db()`
carries no tenant filter. For join-table entities that carry no `tenant_id` column (e.g.
`service_host`), use `tenant_db.find_via_tenant_join::<Target, Scoped>(relation)`, which pushes the
tenant filter into the join through the `TenantScoped` side rather than filtering after the fact. Full
rationale in [Coding Standards — Tenant-Safe Database Queries](../../../docs/development/coding-standards.md).

## Testing

New endpoint tests must use the shared `TestApp` harness in `src/test_harness/` (`TestApp` — in-memory
SQLite, migrated schema, seeded tenant, fully wired Axum router; `TestClient` — ergonomic HTTP client
over `tower::oneshot`; fixture helpers such as `register_user`, `insert_service`,
`seed_permissions_for_owner`). Never duplicate a local `test_state()` or `build_test_state()` inline —
extend the harness instead. Integration tests live in `src/integration_tests/` and are gated
`#[cfg(all(test, feature = "db-sqlite"))]`; add new test modules there rather than inline in the route
file for anything that needs a full router + DB. See
[Testing — REST API Integration Tests](../../../docs/development/testing.md#rest-api-integration-tests)
for the shared conventions (time-dependent tests, `EncryptedString` test helpers, FK setup, etc.) that
also apply to tests in this crate.

## Maintaining this file

Keep this file at or under 250 lines (`ci/verify_agents_md_budget.sh` enforces the budget). Do not add
a per-file or per-module inventory table that mirrors the source tree — describe orientation in prose
and tell the reader how to find things (grep targets, entry-point files), since file layout drifts
faster than prose does. Do not hardcode counts of sub-states, extractors, or handler files; point at
the source file that defines them instead. Prefer linking to the canonical docs
(`docs/development/coding-standards.md`, `docs/development/testing.md`,
`docs/security/auth-and-authorization.md`, `docs/api/http-web-api.md`) over copying their content.
