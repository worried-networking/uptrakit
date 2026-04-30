# MCP Server — Design

## Problem

There is no machine-accessible interface for AI agents (Claude Desktop, Cursor, etc.) to query
controller state or trigger actions. The REST API serves human-driven UIs and scripts, not
AI-agent tool-call patterns. An MCP server closes this gap.

## Approach

Embed an MCP server as a module inside `uptrakit-web-api` (`src/mcp/`). Mount it as a
Streamable HTTP (SSE) endpoint at `/mcp` on the existing Axum HTTPS server. Use the `rmcp`
1.x crate for protocol handling. Gate the entire feature behind a `mcp` Cargo feature
(default-on; disable with `--no-default-features`).

The module is intentionally kept inside `web-api` for MVP because `AppState` is defined there
and the compile-time isolation benefit of a separate crate is not achievable until `AppState` is
extracted into a lighter `web-api-core` crate. Promotion to `crates/ui/mcp/` is a mechanical
refactor deferred to after that extraction.

## Architecture

```text
Axum HTTPS server (existing, port 9443)
├── /api/...          existing REST API
├── /mcp              new — rmcp Streamable HTTP (SSE)
└── /                 SPA fallback
```

`build_mcp_router()` is defined in `uptrakit-web-api` and returns a `Router<Arc<AppState>>`.
It is merged externally in `controller-runtime/src/server.rs`, before middleware layers are
applied, so all middleware (rate limiting, real-IP, security headers, request logging) covers
both routers:

```rust
let router = uptrakit_web_api::build_router(app_state.clone())
    .merge(uptrakit_web_api::mcp::build_mcp_router(app_state.clone()));
// .layer(...) calls follow here, applying to the merged router
```

## Module Structure

```text
crates/ui/web-api/src/mcp/
├── mod.rs          build_mcp_router() → Router<Arc<AppState>>
├── auth.rs         auth validation + Permission::AccessMcp check at HTTP layer
├── terminal.rs     vt100 rendering (width=220) + plain-text output
└── tools/
    ├── mod.rs
    ├── user.rs     get_current_user
    ├── history.rs  list_update_history, get_update_history_detail
    └── update.rs   trigger_update
```

## Authentication

The `/mcp` endpoint uses the same `Authorization: Bearer` header as the REST API. Token
dispatch logic in the existing `require_auth` middleware handles both token types:

- `upk_...` prefix → opaque API token → DB lookup via `ApiTokenService::verify_token()` →
  resolves `(user_id, token_id)`
- No `upk_` prefix → JWT access token → stateless validation

Both produce an `AuthenticatedUser` struct injected into Axum request extensions.

### Auth flow into rmcp tool context

rmcp mounts `StreamableHttpService` (a Tower service) via `Router::nest_service("/mcp", ...)`.
The `/mcp` sub-router does NOT apply the global `require_auth` middleware — auth is handled
entirely by a dedicated Tower layer in `mcp/auth.rs`. Applying both would re-validate the
token twice per request with no benefit and creates a divergence risk if the two paths have
subtly different validation logic or token revocation behavior. The structural reason this
works: `require_auth` is applied via `.route_layer()` on `auth_routes` inside `build_router()`
before any merge — Axum's `.route_layer()` scopes middleware to those specific routes only,
not to subsequently merged routes.

The MCP auth Tower layer must emit the same audit events that `require_auth` emits:
`AUTH_API_TOKEN_AUTHENTICATE` (success and failure) / `AUTH_JWT_AUTHENTICATE`. Without this,
every MCP auth attempt — including failed ones from invalid or probing tokens — is invisible
in the audit log. The `emit_api_token_auth_audit` / `emit_jwt_auth_audit` functions in
`require_auth.rs` must be reachable from `mcp/auth.rs` (move to a shared module or
make crate-internal).

The global audit and request-log middleware (added in `server.rs`) run on all requests
including `/mcp`. Both must tolerate absent `AuthenticatedUser` extensions gracefully —
for MCP requests, `AuthenticatedUser` is not set (only `McpRequestContext` is). Verify
neither panics nor emits error-level logs on `None`.

**JWT tokens:** the auth layer should reject JWT tokens (no `upk_` prefix) with a descriptive
error: "MCP requires an API token (`upk_...`); JWT access tokens expire and are not suitable
for persistent MCP connections." This prevents a confusing works-then-breaks failure mode.

The integration pattern: `mcp/auth.rs` implements a Tower layer wrapping
`StreamableHttpService` that:

1. Reads the `Authorization` header directly from the HTTP request
2. Validates the token via `ApiTokenService` (same logic as `require_auth`)
3. Checks `Permission::AccessMcp` — returns 401/403 before rmcp handles the request
4. Inserts a resolved `McpRequestContext { user_id, token_id, tenant_id, permissions }` into
   `http::request::Extensions`. `McpRequestContext` must derive `Clone + Send + Sync + 'static`
   — required by rmcp's `Extension<T>` extractor.

Inside rmcp tool handlers, the context is extracted via rmcp's own `Extension<http::request::Parts>`
(import: `rmcp::handler::server::tool::Extension`, not `axum::extract::Extension`) — rmcp
injects the original HTTP request parts as an extension on each tool-call invocation, making
the full `Extensions` map (including `McpRequestContext`) available per call via
`parts.extensions.get::<McpRequestContext>()`. The `McpRequestContext` must not be stored on
the handler struct, since the struct lives for the entire session (one instance per
`initialize`) while the context is per-request.

**`allowed_hosts` configuration:** `StreamableHttpServerConfig::default()` only allows
`localhost` / `127.0.0.1` / `::1`. For any remote deployment this produces 403 on every
request. `build_mcp_router()` must construct `StreamableHttpServerConfig` with `allowed_hosts`
populated from `state.settings.sans()`. SANs do not include ports but HTTP `Host` headers
do (e.g., `controller.example.com:9443`), causing exact-match 403 failures on all non-localhost
deployments. Step 3 of the commit sequence must resolve this before merging: write a unit test
calling rmcp's comparison with a `Host: hostname:port` header against a bare-hostname
`allowed_hosts` entry. If it fails (expected), implement port-stripping in `build_mcp_router()`
— strip port from each SAN entry and also add the `hostname:port` form — before populating
`allowed_hosts`. Wildcard SANs (`*.example.com`) also require explicit expansion; document
any unresolved wildcard limitation explicitly.

**User setup:** create an API token in the UI, paste it into Claude Desktop / Cursor config as
`Authorization: Bearer upk_...`.

## Permissions

A new `Permission::AccessMcp` variant is added to the `Permission` enum. The DB migration
grants it only to roles that already hold `ViewSoftware` or `TriggerUpdates` — roles that
have meaningful coverage of at least one MCP tool. Roles with neither permission have no MCP
tools available and should not receive `AccessMcp` by default.

**Operator-role coherence:** the `operator` role has `TriggerUpdates` but not `ViewSoftware`.
A user assigned only this role (without `viewer`) receives `AccessMcp` and can call
`trigger_update` but gets 403 on all history tools — they can trigger updates but cannot
observe outcomes via MCP. In practice `AccessPreset::Operator` always bundles `viewer` +
`operator`, so this edge case only affects users assigned the raw role directly. The migration
must add a comment documenting this partial-access scenario. Considered acceptable for MVP;
adding `list_software_items` and `list_hosts` tools at `ViewSoftware` level would close the
gap cleanly.

### Permission enum prerequisite

`Permission` is stored in the database and loaded at runtime. Two code paths load permissions:

- **API token path:** `require_auth.rs` calls `.parse::<Permission>().ok()` — exhaustive
  `FromStr`, silently drops unknown variants
- **JWT path:** claims deserialized via `Deserialize`

Both paths have the same hazard: an old binary encountering a new unknown variant (e.g.,
`access_mcp` added in a newer build) silently drops it → user missing the permission → `403`.

Before adding `AccessMcp`, `Permission` is migrated to the `Other(String)` wire-safe
catch-all pattern (canonical implementation: `crates/shared/wire/src/lib.rs`):

- Add `Other(String)` variant with `#[strum(disabled)]` — excluded from `EnumIter`, so
  `Permission::iter()` still yields only known variants; `EnumIter` is preserved
- Replace exhaustive `FromStr` **and** `Deserialize` with infallible implementations:
  unknown strings → `Other(s)` instead of `Err` / deserialization error
- `as_str()` and `description()` return types change from `&'static str` to `&str` (lifetime
  bound to `&self`); add `Other(s) => s.as_str()` and `Other(_) => "(unknown permission)"`
  arms respectively
- `Permission` already does not derive `Copy`; no call-site changes needed for that
- `has_permission()` and `contains()` call sites are unaffected — `PartialEq` still works

## Tools

All tools are tenant-scoped. `TenantDb` is constructed using `state.default_tenant_id`
(via the existing `TenantContext` pattern — current single-tenant model; multi-tenancy is
tracked separately).

`AccessMcp` is a gate to the `/mcp` endpoint, not a substitute for action-level permissions.
Each tool enforces its own fine-grained permission check against `McpRequestContext.permissions`
before calling the underlying action — mirroring the `CanX` extractor pattern in the REST
handlers:

| Tool                       | Required permissions              |
|----------------------------|-----------------------------------|
| `get_current_user`         | `AccessMcp` only                  |
| `list_update_history`      | `AccessMcp` + `ViewSoftware`      |
| `get_update_history_detail`| `AccessMcp` + `ViewSoftware`      |
| `trigger_update`           | `AccessMcp` + `TriggerUpdates`    |

### `get_current_user`

Returns identity information about the token owner.

- **Inputs:** none
- **Returns:** `user_id`, `email`, `first_name`, `last_name`, `permissions[]`
- **Source:** `user_id` from resolved `McpRequestContext`; one `User::find_by_id(user_id)` DB
  lookup to fetch `email`, `first_name`, `last_name` (these fields are not on `McpRequestContext`).
  This is a self-profile lookup equivalent to `GET /api/v1/auth/me` — `ManageUsers` is not
  required and must not be added.

### `list_update_history`

Returns a paginated list of update records, without output text.

- **Inputs:** `host_id?` (UUID), `software_item_id?` (UUID), `status?`, `page?`, `per_page?`
- **Returns:** paginated — `id`, `host_name`, `software_item_name`, `from_version`,
  `to_version`, `status`, `actor_name`, `started_at`, `completed_at`, `interactive`,
  `update_category`
- **Output field:** excluded — `list_update_history()` populates `output` on every record;
  the tool clears it (sets to empty string) on each `UpdateHistoryResponse` before returning.
  No new query variant needed.
- **Source:** `web-api-queries::update_history::list_update_history()` directly

### `get_update_history_detail`

Returns full detail for a single update record including rendered terminal output.

- **Inputs:** `update_history_id` (UUID)
- **Returns:** all fields above + `output` (plain text, terminal-rendered)
- **Output processing:** raw bytes fed through `vt100` crate at width=220, extracting the
  final screen state as plain text. This correctly collapses `\r` rewrites, cursor-up/down
  progress bar tricks, and strips all ANSI escape sequences. Width 220 avoids wrapping on
  typical package manager output.
- **Source:** `web-api-queries::update_history::get_update_history()` directly

### `trigger_update`

Triggers a software update for a specific host.

- **Inputs:** `host_id` (UUID), `software_item_id` (UUID), `to_version` (String)
- **Returns:** `update_history_id`, `status`
- **Hardcoded:** `interactive = false` (AI agent cannot interact with a PTY)
- **Omitted:** `release_info` (MVP scope)
- **Source:** constructs `state.mutation_context()`, builds `TriggerUpdateParams`, calls
  `item_actions::trigger_update()`, then if `result.pending_protection_work` is `Some` calls
  `update_orchestrator::spawn_protection_and_dispatch(Arc::clone(&state), *work)` — identical
  to the REST handler path. Omitting the spawn would silently skip pre-update
  snapshot/protection for every MCP-triggered update.
- **Actor recording:** same logic as the REST handler — API token auth sets
  `actor_type = ActorType::ApiToken`, `actor_id = token_id.to_string()`; JWT auth sets
  `actor_type = ActorType::User`, `actor_id = user_id.to_string()`
- **Audit:** calls `emit_software_update_audit` with the same payload as the REST handler
  (host_id, software_item_id, to_version, update_history_id, dispatch status, actor).
  Omitting this produces a forensic blind spot — MCP-triggered updates would be invisible
  in the audit log.

## Dependencies

Added to `web-api/Cargo.toml` behind `[features] mcp`:

| Crate   | Purpose                                                                      |
|---------|------------------------------------------------------------------------------|
| `rmcp`  | MCP protocol, Streamable HTTP transport (`transport-streamable-http-server`) |
| `vt100` | VT100 terminal emulator for output rendering                                 |

`controller-runtime/Cargo.toml` threads `mcp` → `uptrakit-web-api/mcp`.

## Testing

- **Unit — `terminal.rs`:** `\r` line collapsing, cursor-up progress bar collapse, ANSI
  stripping, multi-byte UTF-8 boundary safety
- **Unit — each tool:** real SQLite in-memory `TenantDb`, assert correct query arguments and
  response shape; verify `list_update_history` returns empty `output` field
- **Unit — auth layer:** valid token with `AccessMcp` passes; valid token without `AccessMcp`
  returns 403; missing/invalid token returns 401
- **Manual end-to-end:** connect Claude Desktop to a local controller instance, exercise all
  four tools, verify tenant isolation holds

## Commit Sequence

1. `Permission`: add `Other(String)` with `#[strum(disabled)]`, replace exhaustive `FromStr`
   and `Deserialize` with infallible implementations returning `Other(s)` for unknown strings
2. `Permission`: add `AccessMcp` variant + DB migration seeding it into roles that hold
   `ViewSoftware` or `TriggerUpdates`; update `Permission::iter().count()` assertion in
   `crates/shared/web-api-types/src/lib.rs` from 33 to 34
3. MCP scaffold: `web-api/src/mcp/mod.rs`, `mcp` feature flag, thread through
   `controller-runtime`, wire `/mcp` merge in `server.rs`, add `rmcp` dep with
   `transport-streamable-http-server` feature; configure `allowed_hosts` from `AppState` SANs
4. MCP auth: `mcp/auth.rs` — HTTP-layer token validation (API tokens only; reject JWT with
   descriptive error), `AccessMcp` check, per-request `McpRequestContext` insertion, audit
   event emission (`AUTH_API_TOKEN_AUTHENTICATE` success/failure); verify global audit and
   request-log middleware tolerate absent `AuthenticatedUser` on MCP requests
5. MCP terminal: `mcp/terminal.rs` — `vt100` rendering, add `vt100` dep
6. MCP tool: `get_current_user`
7. MCP tools: `list_update_history` + `get_update_history_detail`
8. MCP tool: `trigger_update` (including `spawn_protection_and_dispatch` and
   `emit_software_update_audit`)

## Known Gaps (MVP)

- **Rate limiting:** existing rate limiting only covers auth endpoints. `/mcp` has no per-token
  or per-IP rate limit. `trigger_update` can be called in a tight loop by a runaway agent.
  Acceptable for a single trusted operator; must be addressed before multi-user deployments.
- **`to_version` validation:** the tool accepts a free-form string. An AI agent can hallucinate
  a version string that passes dispatch but fails on the host, or worse, resolves to an older
  version and triggers a downgrade. A future `list_available_versions` tool would give the
  agent a valid version set to choose from before calling `trigger_update`.

## Future Work

- **OAuth 2.1 flow** — browser-based "Connect to uptrakit" UX for Claude Desktop. Requires
  `AppState` extraction into `web-api-core` first (authorization server state must not couple
  back into `web-api`). Track as a follow-up after `AppState` decoupling.
- **Promote to `crates/ui/mcp/`** — after `AppState` extraction, promoting this module to a
  separate crate is a mechanical refactor that delivers real compile-time isolation from PKI
  and plugin infrastructure.
- **Additional tools** — host listing, software item inventory, scheduled task management.
