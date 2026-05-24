# MCP `get_current_user` Integration Tests

**Date:** 2026-05-24
**Status:** Spec approved

## Problem

`get_current_user` is the primary MCP tool exercised after auth. No in-process test covers the
full pipeline: `McpAuthLayer` → `StreamableHttpService` (stateful SSE) → `McpRequestContext`
extension injection → `FromContextPart` deserialization → DB query. The existing Docker e2e test
(`oauth_end_to_end_mcp_rs_round_trip`) is broken — it sends `tools/call` without an `initialize`
handshake, which in stateful mode returns HTTP 422, not 200.

## Scope

Two deliverables:

1. **New in-process integration tests** in `crates/ui/mcp/tests/get_current_user_mcp.rs`
2. **Fix** to `crates/core/integration-tests/tests/oauth_end_to_end.rs`

## Architecture

### Why in-process + real TCP, not `tower::oneshot`

`StreamableHttpService` spawns internal tasks; it is not a simple request-response tower service.
`oneshot` cannot drive it. Tests bind `127.0.0.1:0` and use `axum::serve` + `reqwest`, exactly as
rmcp's own test suite does.

### Why stateful SSE (production config)

`StreamableHttpServerConfig::default()` sets `stateful_mode = true`. Tests use this unchanged so
they exercise the same code path production uses, including session management.

---

## Test harness

### `McpTestApp` (in the test file)

```rust
McpTestApp {
    addr: SocketAddr,            // bound address for reqwest
    db: DatabaseConnection,      // for fixture queries
    tenant_id: Uuid,
    cancel: CancellationToken,   // shuts down axum::serve on Drop
}
```

`McpTestApp::new()` sequence:

1. `Database::connect("sqlite::memory:")` → `run_migrations(&db)`
2. Insert default tenant row (`tenant::ActiveModel`)
3. Call `uptrakit_crypto::enable_plaintext_mode()` — required for any DB path that touches
   `EncryptedString` (audit log, token hash)
4. Build `McpState::new(...)` with:
   - `DbState::new(db.clone())`
   - `AuthState::new(JwtManager::from_secret(...), DeviceFlowStore::new(db.clone()), RateLimitStore::new(db.clone()), Arc::new(TokenDenylist::new()))`
   - `settings`: `Settings::new(RegistrationSettings { mode: RegistrationMode::Open, token_hash: None, require_token_for_oidc: false }, 168)`,
     then `settings.set_sans(vec!["127.0.0.1".to_string()]).await` so `build_allowed_hosts` emits a
     non-empty list and rmcp host-header validation is exercised (not bypassed). The portless entry
     `"127.0.0.1"` matches `Host: 127.0.0.1:<any-port>` per rmcp matching rules.
   - `default_tenant_id: tenant_id`
   - `controller_id: Uuid::nil()`
   - Audit emitter: `let audit_db_backend: Arc<dyn AuditLogBackend> = Arc::new(DatabaseBackend::new(db.clone()));`
     then `AuditEmitter::with_backends(AuditLogDispatcher::new(Arc::clone(&audit_db_backend)), Arc::clone(&audit_db_backend), Arc::new(NoopBackend))`
   - `shutdown_token: CancellationToken::new()`
   - `NoopUpdateDispatcher` (tool not called by `get_current_user`)
   - `oauth_enabled: false`, `oauth_verifier: None`, `oauth_canonical: None`
5. `TcpListener::bind("127.0.0.1:0")` → record `addr`
6. `build_mcp_router(state)` → `axum::serve` in `tokio::spawn` with `ct.child_token()`

`Drop` impl: `cancel.cancel()`.

No `#[tokio::test(start_paused = true)]` — no tokio time APIs in this test.

### Fixture helpers (module-private)

```rust
/// Insert active user; return user_id.
async fn insert_user(db, tenant_id, email) -> Uuid

/// Link user to a built-in role that grants AccessMcp (viewer; seeded by migrations).
async fn link_user_to_access_mcp_role(db, tenant_id, user_id)

/// Create upk_ API token; return plaintext token.
async fn create_api_token(db, user_id) -> String
```

`link_user_to_access_mcp_role` queries for a built-in role whose permissions include `access_mcp`
(the `viewer`, `operator`, or `software_manager` roles seeded by migrations all qualify). The
`owner` role was removed in migration `m20260310_000002_granular_permissions`; do not reference it.
Inserts `user_role` linking the user to the found role.

---

## Protocol helpers

### `McpSession`

```rust
struct McpSession {
    client: reqwest::Client,
    base: String,          // "http://127.0.0.1:<port>/mcp"
    session_id: String,
}
```

`McpSession::initialize(addr, bearer) -> McpSession`:

1. POST to `/mcp`:
   - `Content-Type: application/json`
   - `Accept: application/json, text/event-stream`
   - `Authorization: Bearer <bearer>`
   - Body: `{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}`
2. Assert HTTP 200
3. Extract `Mcp-Session-Id` response header → `session_id`
4. POST `notifications/initialized` with `Mcp-Session-Id` header → assert HTTP 202

`McpSession::call_tool(name, args) -> serde_json::Value`:

1. POST to `/mcp` with session ID + `Mcp-Protocol-Version: 2025-03-26`
2. Body: `{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"<name>","arguments":<args>}}`
3. Assert HTTP 200
4. Return `extract_sse_result(response_body)`

### `extract_sse_result(body: &str) -> serde_json::Value`

Split body on `"\n\n"`. For each chunk, find the `data:` line. Skip chunks whose data is empty
(priming events). Parse the first non-empty data line as JSON. Panics with a descriptive message
if not found — test failure is expected to be explicit.

---

## Tests

### `api_token_get_current_user_succeeds`

```text
1. app = McpTestApp::new().await
2. user_id = insert_user(&app.db, app.tenant_id, "owner@mcp.test").await
3. link_user_to_access_mcp_role(&app.db, app.tenant_id, user_id).await
4. token = create_api_token(&app.db, user_id).await
5. session = McpSession::initialize(app.addr, &token).await
6. result = session.call_tool("get_current_user", json!({})).await
7. assert result["result"]["content"][0]["text"] parses to GetCurrentUserResult
8. assert email == "owner@mcp.test"
9. assert user_id matches
```

### `oauth_jwt_get_current_user_succeeds`

```text
1. app = McpTestApp::new_with_oauth(SECRET, ISSUER, AUD).await
   (same as new() but oauth_enabled=true, oauth_verifier=Some(...))
2. user_id = insert_user(...).await
3. link_user_to_access_mcp_role(...).await
4. token = mint_test_jwt(user_id, app.tenant_id, SECRET, ISSUER, AUD, "mcp:read")
5. session = McpSession::initialize(app.addr, &token).await
6. result = session.call_tool("get_current_user", json!({})).await
7. assert email matches
```

JWT minting mirrors `oauth_rs_audience_binding.rs`:
`McpAccessTokenClaims::new(iss, sub, aud, client_id, scope, jti, iat, nbf, exp, tenant_id)` + `jsonwebtoken::encode`.
`exp = 9_999_999_999`. `client_id` and `jti` must be non-empty valid UUID strings
(e.g. `Uuid::new_v4().to_string()`). Use `#[expect(clippy::unwrap_used, reason = "...")]`.

### `missing_access_mcp_permission_returns_403`

```text
1. app = McpTestApp::new().await
2. user_id = insert_user(&app.db, app.tenant_id, "unpriv@mcp.test").await
   (no role link → no AccessMcp)
3. token = create_api_token(&app.db, user_id).await
4. POST /mcp with initialize body + Bearer token
5. assert HTTP 403
```

No `McpSession` needed: `McpAuthLayer` returns 403 before the MCP protocol layer processes
anything. Directly inspect the HTTP response status.

---

## Cargo changes — `crates/ui/mcp/Cargo.toml`

```toml
[features]
db-sqlite = ["uptrakit-shared-db/db-sqlite", "sea-orm/sqlx-sqlite"]

[dev-dependencies]
uptrakit-web-api-auth = { workspace = true }
uptrakit-crypto       = { workspace = true, features = ["testing"] }
reqwest               = { workspace = true }
time                  = { workspace = true }
tokio                 = { workspace = true, features = ["rt-multi-thread", "macros"] }
```

Test file top: `#![cfg(all(test, feature = "db-sqlite"))]`

Quality gate: covered by the workspace gate `cargo test --all-features`. Run `cargo test -p uptrakit-mcp --features db-sqlite` for isolated crate verification.

---

## Fix — Docker e2e test (`oauth_end_to_end.rs`)

Current step 9 sends `tools/call` directly without a session. In stateful mode this returns
HTTP 422. Fix replaces step 9 with the correct 3-step flow:

```text
9a. POST initialize → extract Mcp-Session-Id from header
9b. POST notifications/initialized with session ID → assert 202
9c. POST tools/call with session ID → assert 200
9d. Parse SSE body → assert result contains "email" field
```

The assertion changes from `status == 200` (which currently would be 422) to both status check
and body content verification.

---

## Lint conventions

Lint suppression rules (no `#[allow]` permitted):

- `#[tokio::test]`-annotated functions are covered by `allow-unwrap-in-tests` and `allow-panic-in-tests`
  Clippy config settings, so no per-function annotation is needed there.
- Private fixture helpers (`insert_user`, `link_user_to_access_mcp_role`, `create_api_token`) are NOT
  `#[test]`-annotated, so they are NOT covered by those config settings. Each helper that calls
  `.unwrap()` must carry `#[expect(clippy::unwrap_used, reason = "test fixture — panic on setup failure")]`.
- `extract_sse_result` calls `panic!` directly, which also falls outside `allow-panic-in-tests`. It
  must carry `#[expect(clippy::panic, reason = "test helper — explicit failure message on SSE parse error")]`.
- `McpSession::initialize` and `McpSession::call_tool` are also not `#[test]`-annotated; add
  `#[expect(clippy::unwrap_used, reason = "...")]` at method level if they call `.unwrap()`.

---

## Documentation impact

No user-facing docs, API docs, or ADRs affected. This is an internal test addition with no
externally observable behavior change.

## Out of scope

- Testing `trigger_update` tool (requires host/service DB fixtures beyond this scope)
- Testing `get_update_history` tool
- Performance/load testing of MCP server
- Testing SSE stream resume / GET stream endpoint
