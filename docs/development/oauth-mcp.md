# MCP OAuth 2.1 — Developer Guide

This guide is for engineers adding MCP tools or modifying the OAuth surface of the MCP server
(`uptrakit-mcp`) or the Authorization Server inside `uptrakit-web-api`.

Related: [ADR 0010](../adr/0010-mcp-oauth-authorization-server-placement.md) ·
[Security guide](../security/oauth-mcp.md) · [Admin guide](../admin/oauth-clients.md) ·
[Spec](../superpowers/specs/2026-05-12-mcp-oauth-authorization-design.md)

## Adding an MCP Tool That Needs OAuth

Every MCP tool declares a `ToolAuth` constant immediately next to its handler in
`crates/ui/mcp/src/tools/`. The auth layer calls `require_scope` followed by the existing
`has_permission` check; both must pass before the handler runs.

### Step 1 — Pick the right scope

| Criterion                                | Scope                           |
| ---------------------------------------- | ------------------------------- |
| Tool reads data only (no state mutation) | `McpScope::Read` (`mcp:read`)   |
| Tool mutates state or triggers an action | `McpScope::Write` (`mcp:write`) |

When in doubt, default to `McpScope::Write`. It is always valid to tighten scope after release; it is
never valid to loosen it silently (see [scope migration policy](#scope-migration-policy) below).

### Step 2 — Declare `ToolAuth`

Follow the `TRIGGER_UPDATE_AUTH` pattern in `crates/ui/mcp/src/tools/update.rs`:

```rust
use uptrakit_mcp::auth::{ToolAuth, McpScope};
use uptrakit_web_api_types::Permission;

pub(crate) const MY_TOOL_AUTH: ToolAuth = ToolAuth {
    required_scopes: &[McpScope::Write],
    required_permissions: &[Permission::TriggerUpdates],
};
```

`ToolAuth.required_scopes` is an all-of slice: every listed scope must be present. Same for
`required_permissions`. An empty `required_permissions` slice means the tool only requires the
`AccessMcp` permission that the auth layer already enforces for every authenticated request.

### Step 3 — Call `require_scope` at the top of the handler

```rust
pub(crate) async fn my_tool_handler(
    ctx: McpRequestContext,
    params: MyToolParams,
) -> Result<MyToolResponse, McpError> {
    require_scope(&ctx, McpScope::Write)?;
    // ... handler body
}
```

`require_scope` returns `Err(McpError::InsufficientScope { required })` for OAuth callers missing the
scope. API-token callers bypass scope checks (no scope concept at issuance) but still pass through the
Permission check in the auth layer.

### v1 Tool Mapping

| Tool                        | Required scopes | Required permissions |
| --------------------------- | --------------- | -------------------- |
| `list_update_history`       | `[Read]`        | `[ViewSoftware]`     |
| `get_update_history_detail` | `[Read]`        | `[ViewSoftware]`     |
| `trigger_update`            | `[Write]`       | `[TriggerUpdates]`   |
| `get_current_user`          | `[Read]`        | `[]`                 |

## Scope Migration Policy

The `mcp:*` namespace is reserved for the uptrakit MCP scope set. The migration policy is:

- **Additive only.** Future granular scopes (e.g., `mcp:fleet:trigger`) refine but never replace the
  coarse pair.
- **`mcp:read` and `mcp:write` retain v1 semantics permanently.** A token issued with `mcp:write`
  today must continue to satisfy any tool that requires `mcp:write` after a granular scope is added.
- **Backward-compatible resolution.** When a tool declares both a coarse and a granular required
  scope, the coarse scope alone satisfies the check on tokens issued before the granular scope existed.
  Granular scopes are "advisory least-privilege" for newly-minted tokens, not a hard authz floor that
  retroactively breaks existing tokens.
- **No silent tightening.** A PR that changes an existing tool from `McpScope::Read` to
  `McpScope::Write` (or adds a new required permission) is a breaking change for existing OAuth
  clients. It requires a deprecation notice and a migration window. File a phase-N spec entry.

## Token Validation Invariants

These invariants are enforced by `validate_oauth_access_token_for_mcp` in `uptrakit-mcp`. Do not
bypass them in tests or in alternate validation paths.

- **Algorithm pinned to HS256.** Tokens with `alg=none`, `RS256`, `ES256`, or any other algorithm are
  rejected before claim examination. The `Validation` instance sets `algorithms: vec![Algorithm::HS256]`.
- **`aud` exact-match against canonical resource URL.** The `aud` claim must equal
  `https://<oauth.canonical_host>/mcp`. Additional accepted hosts in `oauth.accepted_audience_hosts`
  expand the accepted set but do not relax the exact-string comparison rule.
- **`iss` exact-match against canonical issuer.** The `iss` claim must equal
  `https://<oauth.canonical_host>`.
- **Required spec claims.** The `Validation` instance sets `required_spec_claims` to enforce presence
  of `sub`, `aud`, `iss`, `exp`, `iat`, `nbf`, `jti`, `client_id`, `tenant_id` before any other
  check. A token missing any of these is rejected with `McpResourceServerError::MissingRequiredClaim`.
- **No JWT passthrough.** A token that passes signature verification against the Dashboard JWT secret
  is still rejected at the MCP RS because `aud` and the signing secret differ. Cross-rejection is
  double-locked.

## Audit Emission

All OAuth-related audit events in the AS use `AuditActionType::OAUTH_*` constants from
`uptrakit-audit-log`. Emit via `AuditEmitter::emit_event()` (async, best-effort). See
`crates/ui/web-api/src/oauth/services/client.rs` for the canonical pattern.

```rust
// Pattern — async best-effort emit
audit_emitter
    .emit_event(
        AuditEntry::builder()
            .action(AuditActionType::OAUTH_CLIENT_REGISTERED)
            .actor_id(actor_id)
            .details(serde_json::json!({ "client_id": client_id }))
            .build(),
    )
    .await;
```

For RS-side events in `uptrakit-mcp`, use `AuditActionType::MCP_OAUTH_AUTHENTICATE` with a `reason`
field drawn from the typed reason enum. The RS does not perform DB-side revocation checks per request
(JWT validation is stateless v1); revocation propagates at the next access-token mint within the TTL
grace window.

Never emit audit events with `target: "security_audit"`. Use the structured `AuditEntry` builder with
the typed `AuditActionType` constant.

## OAuth boot sequence

`boot_oauth_state` in `crates/ui/web-api/src/oauth/boot.rs` is called during Phase 7d of controller
startup — after `seed_oauth_defaults` and before `validate_configuration` — in
`crates/core/controller-runtime/src/lib.rs`.

`resolve_mcp_enabled(explicit: Option<bool>, canonical_host: Option<&str>) -> bool` determines
whether the OAuth surface is active. The truth table is:

| `explicit` | `canonical_host` | Result  | Reason                                 |
| ---------- | ---------------- | ------- | -------------------------------------- |
| absent     | absent           | `false` | Nothing configured                     |
| absent     | set              | `true`  | Auto-enable                            |
| `false`    | set              | `false` | Operator override wins                 |
| `true`     | absent           | `true`  | Explicit; `CanonicalHostMissing` fires |
| `true`     | set              | `true`  | Normal enabled path                    |

Boot runs a single `BEGIN IMMEDIATE` transaction that reads or generates `oauth.jwt_signing_secret`
(32 random bytes, hex-encoded) and then calls `validate_and_register` in the same transaction. Using
`BEGIN IMMEDIATE` prevents split-brain on rapid restart loops: a second process cannot observe a
partially-written secret between the read and the write.

### Integration test pattern

```rust
api_client.update_oauth_settings("127.0.0.1:<port>").await;
let current_gen = /* read X-Reexec-Generation from GET /healthz */;
api_client.force_reexec().await;
api_client.wait_for_generation(current_gen + 1, Duration::from_secs(30)).await;
```

## Test Patterns

### Clock injection

All OAuth services take `Arc<dyn Fn() -> OffsetDateTime + Send + Sync>` at construction:

```rust
let clock = Arc::new(parking_lot::Mutex::new(OffsetDateTime::now_utc()));
let clock_fn: Arc<dyn Fn() -> OffsetDateTime + Send + Sync> = {
    let c = clock.clone();
    Arc::new(move || *c.lock())
};
// Advance time in a test:
*clock.lock() += Duration::seconds(901); // expire a 15-min token
```

Do not call `tokio::time::sleep` or `std::thread::sleep` in OAuth tests. Do not use
`#[tokio::test(start_paused = true)]` in DB tests (SQLx pool timers fire prematurely — see
`docs/development/testing.md`).

### In-memory SQLite `TestApp`

OAuth integration tests live under `crates/ui/web-api/src/integration_tests/`. See existing examples
for how to spin up a `TestApp` with a migrated in-memory SQLite database. All FK constraints are
enforced (`PRAGMA foreign_keys = ON` set by the migration runner). Always insert all required parent
rows (`oauth_clients` before `oauth_consents`, etc.).

### Multi-step flow tests

For authorization code flow tests:

1. Insert an `oauth_clients` row with the expected `redirect_uri`.
2. Call `GET /oauth/authorize?...` — assert redirect to consent.
3. Call `POST /oauth/consent/{request_id}/approve` — assert redirect with `code=`.
4. Call `POST /oauth/token` with the code and `code_verifier` — assert `access_token`.
5. Use the `access_token` as `Authorization: Bearer eyJ...` against the MCP RS — assert 200.

### Disabling OAuth in tests

To assert that every OAuth surface returns 404 when the master switch is off:

```rust
let mut config = TestConfig::default();
config.oauth.mcp_enabled = false;
let app = TestApp::with_config(config).await;

// Every OAuth well-known and API route returns 404
app.get("/.well-known/oauth-authorization-server").await.assert_status(404);
app.get("/.well-known/oauth-protected-resource").await.assert_status(404);
app.post("/oauth/register").await.assert_status(404);
app.get("/oauth/authorize").await.assert_status(404);
app.post("/oauth/token").await.assert_status(404);
```

JWT-shaped bearer tokens are also rejected (API tokens still work) when `oauth.mcp_enabled = false`.
