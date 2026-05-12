# MCP OAuth — Plan D: Resource Server Rewrite + CIMD Fetcher (Phase 3 + Phase 4)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rewrite the MCP auth layer in `uptrakit-mcp` so it prefix-dispatches between the existing `upk_*` API-token
path and the new OAuth JWT path; serve the Protected Resource Metadata document at both well-known paths; declare
`ToolAuth` metadata next to each MCP handler and gate tool invocations on scope ∩ Permission; build the SSRF-safe CIMD
fetcher with parser hardening (raw bytes persisted, never invalidate on parse failure, versioned parser); detect
material CIMD metadata changes and force re-consent.

**Architecture:** New `crates/ui/mcp/src/oauth/` module hosting the JWT verifier wrapper, the PRM endpoint handler, the
`ToolAuth` declarations, and the scope-check helper. `McpAuthLayer` in `crates/ui/mcp/src/auth.rs` gains a
prefix-dispatch path that routes JWT tokens to the verifier and opaque `upk_*` tokens through the existing path. PRM
document is served at both `/.well-known/oauth-protected-resource` and `/.well-known/oauth-protected-resource/mcp`. CIMD
fetching lives in `crates/ui/web-api/src/oauth/cimd.rs` (Plan B already scaffolded `oauth/mod.rs`) and uses the existing
`SsrfSafeResolver`.

**Tech Stack:** rmcp (existing transport) + axum + tower + sea-orm + `jsonwebtoken` v10 (verifier-side only) + `reqwest`
with `SsrfSafeResolver` + `serde_json::Value` for two-pass CIMD parsing + `sha2` for content hashing.

**Spec:** `docs/superpowers/specs/2026-05-12-mcp-oauth-authorization-design.md` (commit `b7ee4a852`).

**Status:** Draft → Ready for review.

---

## Prerequisites

- **Plan A** (foundation: types, entities) merged.
- **Plan B** (AS routes) merged. This plan reuses the `McpOAuthJwtVerifier` from Plan B's `oauth/jwt.rs` by promoting it
  to a shared crate or duplicating the verifier in `uptrakit-mcp`. **Strategy:** move `McpOAuthJwtVerifier` from
  `crates/ui/web-api/src/oauth/jwt.rs` into `crates/shared/web-api-types/src/oauth/jwt_verifier.rs` (verifier only — the
  signer stays in web-api). This avoids `uptrakit-mcp → uptrakit-web-api` dependency.

## Snapshot binding

- "Plugin HTTP clients: set .connect_timeout(10s) + .timeout(60s); add .dns_resolver(SsrfSafeResolver) for user URLs" —
  CIMD fetcher
- "BEGIN IMMEDIATE for SQLite read-then-write transactions" — CIMD cache upsert + material-change detection
- "Use parking_lot::Mutex (never std::sync::Mutex or tokio::sync::Mutex) in async code" — in-memory CIMD content-hash
  cache (if any)
- "inject wall-clock time via Arc<dyn Fn() -> OffsetDateTime + Send + Sync>" — CIMD fetcher freshness window
- "All HTTP request types in uptrakit-web-api-types implement Validate" — CIMD doc parser hardening (two-pass: `Value` →
  typed struct)
- "preserve error context with rootcause::Report" — `McpResourceServerError` + `impl_report_conversion!`
- "Never use unwrap()/expect()/panic!() in production" — strict
- "semantic audit emission (AuditEntry + AuditEmitter)" — `MCP_OAUTH_AUTHENTICATE`, `OAUTH_CIMD_PARSE_FAILED`,
  `OAUTH_CLIENT_FIRST_USE`, `OAUTH_CLIENT_METADATA_REFRESHED`, `OAUTH_CLIENT_METADATA_CHANGED_MATERIALLY`

## File Structure

**New / modified in `uptrakit-mcp`:**

- `crates/ui/mcp/src/oauth/mod.rs` — module entry.
- `crates/ui/mcp/src/oauth/verifier.rs` — wraps `McpOAuthJwtVerifier` from web-api-types; loads signing-secret-derived
  verifier at boot.
- `crates/ui/mcp/src/oauth/prm.rs` — PRM endpoint handler (axum route).
- `crates/ui/mcp/src/oauth/tool_auth.rs` — `ToolAuth` struct + `require_scope` helper.
- `crates/ui/mcp/src/auth.rs` — modified to prefix-dispatch between API token and OAuth.
- `crates/ui/mcp/src/lib.rs` — mount PRM route in `build_mcp_router`.
- `crates/ui/mcp/src/state.rs` — extend `McpState` with `oauth_verifier: Arc<McpOAuthJwtVerifier>` +
  `canonical: Arc<CanonicalUrlConfig>` + `oauth_enabled: bool`.
- `crates/ui/mcp/src/tools/history.rs`, `update.rs`, `user.rs` — declare `ToolAuth` constants and call `require_scope`
  before existing Permission check.

**New / modified in `uptrakit-web-api`:**

- `crates/ui/web-api/src/oauth/cimd.rs` — fetcher + parser + material-change detection.
- `crates/ui/web-api/src/oauth/cimd_parser/mod.rs` — versioned parser dispatch.
- `crates/ui/web-api/src/oauth/cimd_parser/v0_draft00.rs` — current spec-revision parser.
- `crates/ui/web-api/src/routes/oauth/authorize.rs` — integrate CIMD fetch when `client_id` is HTTPS URL.

**New shared verifier:**

- `crates/shared/web-api-types/src/oauth/jwt_verifier.rs` — `McpOAuthJwtVerifier` moved here (in Task 1).

---

## Tasks

### Task 1: Move JWT verifier to shared crate

**Files:**

- Move: `crates/ui/web-api/src/oauth/jwt.rs` verifier → `crates/shared/web-api-types/src/oauth/jwt_verifier.rs`
- Modify: `crates/ui/web-api/src/oauth/jwt.rs` — re-export from shared crate; keep signer local
- Modify: `crates/shared/web-api-types/src/oauth/mod.rs` — pub mod + re-export

- [ ] **Step 1: Cut verifier code**

Move the `McpOAuthJwtVerifier` impl + `JwtError` to `crates/shared/web-api-types/src/oauth/jwt_verifier.rs`. Signer
stays in web-api because only the AS mints tokens.

- [ ] **Step 2: Add jsonwebtoken to web-api-types**

If not already a workspace dep, add `jsonwebtoken = { workspace = true }` to `crates/shared/web-api-types/Cargo.toml`.

- [ ] **Step 3: Compile-check both crates**

Run: `cargo check -p uptrakit-web-api-types -p uptrakit-web-api --all-features`

- [ ] **Step 4: Re-run Plan B's jwt tests**

Run: `cargo test -p uptrakit-web-api oauth::jwt && cargo test -p uptrakit-web-api-types oauth::jwt_verifier`

- [ ] **Step 5: Commit**

```bash
git commit -m "refactor(web-api-types): move JWT verifier to shared crate

uptrakit-mcp will reuse McpOAuthJwtVerifier without depending on
uptrakit-web-api. Signer stays in web-api — only the AS mints tokens."
```

### Task 2: Extend McpState with OAuth fields

**Files:**

- Modify: `crates/ui/mcp/src/state.rs`
- Modify: `crates/ui/mcp/Cargo.toml` (add `uptrakit-web-api-types` already a dep)

- [ ] **Step 1: Add fields**

```rust
use std::sync::Arc;
use uptrakit_web_api_types::oauth::{CanonicalUrlConfig, McpOAuthJwtVerifier};

#[non_exhaustive]
#[derive(Clone)]
pub struct McpState {
    // ... existing fields ...
    pub oauth_enabled: bool,
    pub oauth_verifier: Option<Arc<McpOAuthJwtVerifier>>,
    pub oauth_canonical: Option<Arc<CanonicalUrlConfig>>,
}
```

`CanonicalUrlConfig` lives in `uptrakit_web_api_types::oauth` (delivered by Plan A Task 10 Step 4 — the promotion that
lets `uptrakit-mcp` consume the config without depending on `uptrakit-web-api`). `McpState` holds the full config so the
PRM endpoint can advertise `authorization_servers` and the JWT verifier can build the accepted-audience set from
`accepted_resources`.

- [ ] **Step 2: Update controller startup wiring**

When `oauth.mcp_enabled = true`, populate the fields. When false, leave `oauth_verifier = None`.

- [ ] **Step 3: Compile + test**

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(mcp): extend McpState with OAuth verifier slots

Per spec §6.2. Slots are None until oauth.mcp_enabled flips."
```

### Task 3: ToolAuth declarations + require_scope helper

**Files:**

- Create: `crates/ui/mcp/src/oauth/mod.rs`
- Create: `crates/ui/mcp/src/oauth/tool_auth.rs`

- [ ] **Step 1: Write tests**

```rust
#[test]
fn require_scope_bypasses_for_api_token() {
    let ctx = sample_ctx(McpAuthMethod::ApiToken);
    assert!(require_scope(&ctx, &McpScope::Write).is_ok());
}

#[test]
fn require_scope_passes_when_oauth_carries_scope() {
    let ctx = sample_ctx(McpAuthMethod::OAuth {
        client_id: "x".into(),
        jti: uuid::Uuid::nil(),
        scopes: vec![McpScope::Read, McpScope::Write],
    });
    assert!(require_scope(&ctx, &McpScope::Write).is_ok());
}

#[test]
fn require_scope_rejects_when_oauth_missing_scope() {
    let ctx = sample_ctx(McpAuthMethod::OAuth {
        client_id: "x".into(),
        jti: uuid::Uuid::nil(),
        scopes: vec![McpScope::Read],
    });
    assert!(matches!(
        require_scope(&ctx, &McpScope::Write),
        Err(McpScopeError::Insufficient { required: McpScope::Write })
    ));
}
```

- [ ] **Step 2: Verify failure**

- [ ] **Step 3: Implement**

```rust
// crates/ui/mcp/src/oauth/tool_auth.rs
use uptrakit_controller_core::auth::Permission;
use uptrakit_web_api_types::oauth::McpScope;
use thiserror::Error;

use crate::context::{McpAuthMethod, McpRequestContext};

#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct ToolAuth {
    pub required_scopes: &'static [McpScope],
    pub required_permissions: &'static [Permission],
}

#[non_exhaustive]
#[derive(Debug, Error, PartialEq, Eq)]
pub enum McpScopeError {
    #[error("insufficient scope: required {required:?}")]
    Insufficient { required: McpScope },
}

/// Verify the context carries every scope in `required` (all-of semantics).
///
/// API-token contexts bypass scope checks (no scope concept exists at issuance).
///
/// # Errors
/// Returns `McpScopeError::Insufficient` for OAuth contexts missing any required scope.
pub fn require_scopes(
    ctx: &McpRequestContext,
    required: &[McpScope],
) -> Result<(), McpScopeError> {
    match &ctx.auth_method {
        McpAuthMethod::ApiToken => Ok(()),
        McpAuthMethod::OAuth { scopes, .. } => {
            for r in required {
                if !scopes.contains(r) {
                    return Err(McpScopeError::Insufficient { required: r.clone() });
                }
            }
            Ok(())
        }
    }
}
```

- [ ] **Step 4: Verify pass**

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(mcp): ToolAuth + require_scopes helper

Per spec §8.2 + §8.3. All-of semantics; API tokens bypass scope check."
```

### Task 4: Declare ToolAuth constants and gate each existing tool

**Files:**

- Modify: `crates/ui/mcp/src/tools/history.rs`
- Modify: `crates/ui/mcp/src/tools/update.rs`
- Modify: `crates/ui/mcp/src/tools/user.rs`

- [ ] **Step 1: Add ToolAuth constants**

```rust
// in history.rs
pub(crate) const LIST_UPDATE_HISTORY_AUTH: ToolAuth = ToolAuth {
    required_scopes: &[McpScope::Read],
    required_permissions: &[Permission::ViewSoftware],
};
pub(crate) const GET_UPDATE_HISTORY_DETAIL_AUTH: ToolAuth = ToolAuth {
    required_scopes: &[McpScope::Read],
    required_permissions: &[Permission::ViewSoftware],
};

// in update.rs
pub(crate) const TRIGGER_UPDATE_AUTH: ToolAuth = ToolAuth {
    required_scopes: &[McpScope::Write],
    required_permissions: &[Permission::TriggerUpdates],
};

// in user.rs
pub(crate) const GET_CURRENT_USER_AUTH: ToolAuth = ToolAuth {
    required_scopes: &[McpScope::Read],
    required_permissions: &[],
};
```

- [ ] **Step 2: Add require_scopes call at handler entry**

At the top of each tool handler body, call `require_scopes(&ctx, AUTH.required_scopes)?;` before existing Permission
check. Convert `McpScopeError` to `mcp_error(McpErrorCode::PermissionDenied, ...)` with the 403 insufficient_scope
payload.

- [ ] **Step 3: Update existing tests**

Existing tests construct contexts via `McpAuthMethod::ApiToken`; they continue to pass. Add new tests that construct
`McpAuthMethod::OAuth { ... }` with explicit scope sets and confirm bypass/reject behavior.

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(mcp): declare ToolAuth + gate tool handlers on scope

Per spec §8.2 mapping table. trigger_update requires mcp:write; reads
require mcp:read."
```

### Task 5: Rewrite McpAuthLayer with prefix dispatch

**Files:**

- Modify: `crates/ui/mcp/src/auth.rs`

- [ ] **Step 1: Write tests**

Cover: `Bearer upk_...` succeeds via existing path; `Bearer eyJ...` (JWT shape) routes to OAuth verifier; valid OAuth
JWT produces `McpAuthMethod::OAuth { client_id, jti, scopes }` context; invalid OAuth JWT → 401 with spec-compliant
`WWW-Authenticate`; missing Bearer → 401; unknown Bearer prefix → 401; OAuth disabled + JWT-shaped → 401.

- [ ] **Step 2: Implement prefix dispatch**

```rust
let token = extract_bearer_token(&req);
let mcp_ctx = match token.as_deref() {
    Some(t) if t.starts_with("upk_") => {
        validate_api_token_for_mcp(&state, Some(t)).await
    }
    Some(t) if looks_like_jwt(t) => {
        if let Some(verifier) = state.oauth_verifier.as_ref() {
            validate_oauth_access_token_for_mcp(verifier, &state, t).await
        } else {
            Err(McpAuthError::Unauthorized) // OAuth disabled
        }
    }
    _ => Err(McpAuthError::MissingCredentials),
};
```

`looks_like_jwt(t)` returns `t.matches('.').count() == 2 && t.starts_with("eyJ")`.

`validate_oauth_access_token_for_mcp(verifier, state, token)` calls `verifier.verify(token)`, on success looks up the
user by `claims.sub`, loads their `Permission` set, populates `McpRequestContext` with
`McpAuthMethod::OAuth { client_id, jti, scopes }`. Returns appropriate `McpAuthError` variants on each failure.

401 response carries:

```http
WWW-Authenticate: Bearer realm="mcp",
                         resource_metadata="https://<canonical_host>/.well-known/oauth-protected-resource",
                         scope="mcp:read"
```

- [ ] **Step 3: Verify pass**

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(mcp): prefix-dispatch auth layer for OAuth + API tokens

Per spec §6.1 + §6.3. 401 response carries spec-compliant WWW-Authenticate
pointing at PRM discovery."
```

### Task 6: PRM endpoint at both well-known paths

**Files:**

- Create: `crates/ui/mcp/src/oauth/prm.rs`
- Modify: `crates/ui/mcp/src/lib.rs`

- [ ] **Step 1: Write tests**

Cover: when `oauth_enabled = false` → 404 at both paths; when enabled → 200 with valid JSON; `authorization_servers` is
an array containing the canonical issuer; `scopes_supported` lists `mcp:read` + `mcp:write`; `bearer_methods_supported`
is `["header"]`.

- [ ] **Step 2: Implement**

```rust
#[utoipa::path(
    get,
    path = "/.well-known/oauth-protected-resource",
    responses(
        (status = 200, description = "PRM document", body = ProtectedResourceMetadata),
        (status = 404, description = "OAuth disabled"),
    ),
    tag = "OAuth"
)]
pub async fn get_prm(State(state): State<McpState>) -> Response {
    if !state.oauth_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    let prm = build_prm(&state);
    axum::Json(prm).into_response()
}
```

Same handler bound to both `/.well-known/oauth-protected-resource` and `/.well-known/oauth-protected-resource/mcp` per
spec §6.4.

`build_prm(...)` constructs `ProtectedResourceMetadata` from `state.oauth_canonical`. The response body MUST include a
non-standard `x-uptrakit-mcp-auth-spec-revision` field carrying the value of
`uptrakit_web_api_types::oauth::MCP_AUTH_SPEC_REVISION` (declared in Plan A Task 16) per spec §23.1. Extend the
`ProtectedResourceMetadata` builder or use `serde_json::json!` with an extra entry — whichever keeps the typed struct
clean. Add a test asserting the field is present and equals `"2025-11-25"`.

- [ ] **Step 3: Mount in build_mcp_router**

Add the two routes when building the router.

- [ ] **Step 4: Verify pass**

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(mcp): PRM endpoint at root + sub-path

Per spec §6.4 + 2025-11-25 PRM discovery requirements."
```

### Task 7: Integration test — RS rejects token with wrong audience

**Files:**

- Create: `crates/ui/mcp/tests/oauth_rs_audience_binding.rs`

- [ ] **Step 1: Write test**

Mint a JWT with `aud = "https://other.example.com/mcp"`; assert MCP request returns 401 with `WWW-Authenticate`
containing `resource_metadata="..."` and the audit reason is `invalid_audience`.

- [ ] **Step 2: Verify pass**

- [ ] **Step 3: Commit**

```bash
git commit -m "test(mcp): RS rejects tokens with foreign audience

Per spec §6 + §9.1 RFC 8707 audience binding."
```

### Task 8: CIMD fetcher — happy path (fetch + parse + persist raw bytes)

**Files:**

- Create: `crates/ui/web-api/src/oauth/cimd.rs`
- Create: `crates/ui/web-api/src/oauth/cimd_parser/mod.rs`
- Create: `crates/ui/web-api/src/oauth/cimd_parser/v0_draft00.rs`

**Task boundary**: Task 8 covers ONLY first-fetch and update-of-existing-row with successful parse. The parse-failure
branch is Task 9. The material-change re-consent logic is Task 10. The earlier algorithm overview in this task's
pseudocode mentions "On material change" for documentation continuity — implement that branch in Task 10, not here. Task
8's `fetch_and_upsert` returns successfully for any well-formed CIMD document with matching `client_id`; Task 10 adds
the post-upsert side effect of marking consents revalidation_required.

- [ ] **Step 1: Write tests with `wiremock` or equivalent**

Cover happy path: GET an HTTPS URL → returns JSON with `client_id` matching URL, `redirect_uris` + `client_name` present
→ inserts row in `oauth_clients` with `created_via="cimd_cache"`, sets `metadata_content_hash`, `metadata_raw`,
`metadata_cached_at`, `metadata_etag`.

- [ ] **Step 2: Implement fetcher**

```rust
//! CIMD (Client ID Metadata Document) fetcher.

use std::sync::Arc;
use std::time::Duration;
use reqwest::Client;
use uptrakit_shared_types::ssrf::SsrfSafeResolver;

pub struct CimdFetcher {
    client: Client,
    clock: Arc<dyn Fn() -> OffsetDateTime + Send + Sync>,
    db: DatabaseConnection,
}

impl CimdFetcher {
    /// Construct a `CimdFetcher` with SSRF-safe DNS resolution and bounded timeouts.
    ///
    /// # Errors
    /// Returns `reqwest::Error` if the underlying TLS backend cannot be initialized
    /// (rare — typically only when `aws-lc-rs` fails to load).
    pub fn new(
        db: DatabaseConnection,
        clock: Arc<dyn Fn() -> OffsetDateTime + Send + Sync>,
    ) -> Result<Self, reqwest::Error> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .dns_resolver(Arc::new(SsrfSafeResolver::new()))
            .build()?;
        Ok(Self { client, clock, db })
    }

    /// Fetch a CIMD document and upsert the cached row.
    ///
    /// # Errors
    /// Returns OAuthError::InvalidClient if the document's client_id does not match the URL.
    /// Returns OAuthError::Database on DB error.
    /// Never returns InvalidRequest for a parse failure — preserves existing cached row and emits
    /// OAUTH_CIMD_PARSE_FAILED audit instead.
    pub async fn fetch_and_upsert(&self, url: &str) -> Result<oauth_client::Model, rootcause::Report<OAuthError>> {
        // 1. Body cap 64 KB.
        // 2. Compute SHA-256 of bytes.
        // 3. Two-pass parse: serde_json::Value first; CimdParser::v0_draft00::extract second.
        // 4. Validate client_id equals URL exactly.
        // 5. Upsert row with metadata_content_hash, metadata_raw, metadata_cached_at, metadata_etag.
        // 6. On material change: mark consents revalidation_required_at, emit OAUTH_CLIENT_METADATA_CHANGED_MATERIALLY.
        // 7. Emit OAUTH_CLIENT_FIRST_USE or OAUTH_CLIENT_METADATA_REFRESHED.
    }
}
```

Two-pass parsing pattern in `cimd_parser/v0_draft00.rs`: take `&serde_json::Value`, extract typed `CimdDocument`. On any
extraction failure, return `CimdParseError` (separate type from `OAuthError`).

- [ ] **Step 3: Verify pass**

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(oauth): CIMD fetcher with SsrfSafeResolver + 64KB body cap

Per spec §11.3. Two-pass parser; raw bytes persisted for forward compat
with CIMD draft revisions."
```

### Task 9: CIMD fetcher — parse-failure preserves cached row

**Files:**

- Modify: `crates/ui/web-api/src/oauth/cimd.rs`

- [ ] **Step 1: Write test**

Cover: previously-cached client; refresh returns a malformed document; existing row's parsed fields stay intact;
`metadata_parse_error` + `metadata_parse_error_at` populated; `OAUTH_CIMD_PARSE_FAILED` audit emitted.

- [ ] **Step 2: Implement parse-failure handling**

Per spec §11.3 parser-hardening section: on parse failure, do NOT invalidate the row; do NOT update
`redirect_uris`/`client_name`/etc.; only set `metadata_parse_error` + `metadata_parse_error_at` + `metadata_raw` (so
operators can inspect what was returned).

- [ ] **Step 3: Verify pass**

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(oauth): CIMD parse failures preserve cached row

Per spec §11.3. Forced-outage events avoided when CIMD draft renames fields."
```

### Task 10: CIMD material-change detection forces re-consent

**Files:**

- Modify: `crates/ui/web-api/src/oauth/cimd.rs`

- [ ] **Step 1: Write tests**

Cover: cached row with hash H1; refresh fetches doc with hash H2 (different redirect_uris); detection runs; existing
`oauth_consents` rows for that client have `revalidation_required_at` set; `OAUTH_CLIENT_METADATA_CHANGED_MATERIALLY`
emitted; cosmetic change (only `tos_uri` differs) does NOT set `revalidation_required_at`.

- [ ] **Step 2: Implement**

Per spec §11.3 step 7: compute a normalized hash excluding the cosmetic-fields allowlist (`tos_uri`, `policy_uri`,
`software_version`, `software_id`, plus any field listed in `oauth.cimd_cosmetic_field_allowlist`). Compare normalized
hashes. If different → flag material change.

- [ ] **Step 3: Verify pass**

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(oauth): CIMD material-change detection forces re-consent

Per spec §11.3. Normalized hash excludes cosmetic fields; non-cosmetic
change marks active consents revalidation_required_at."
```

### Task 11: Integrate CIMD into /oauth/authorize

**Files:**

- Modify: `crates/ui/web-api/src/routes/oauth/authorize.rs`

- [ ] **Step 1: Add CIMD resolution path**

When `client_id` starts with `https://`, call `CimdFetcher::fetch_and_upsert` to ensure the row exists / is fresh before
proceeding with redirect_uri validation. Gate on `state.oauth.cimd_enabled` — when false and `client_id` is HTTPS URL,
return `invalid_client`.

- [ ] **Step 2: Update tests**

Add test cases: CIMD-shaped `client_id` triggers fetch; CIMD-disabled rejects URL-shaped `client_id`.

- [ ] **Step 3: Commit**

```bash
git commit -m "feat(oauth): /oauth/authorize integrates CIMD resolution

Per spec §11.3. URL-shaped client_id triggers fetch + upsert when
oauth.cimd_enabled = true; rejected as invalid_client otherwise."
```

### Task 12: CIMD rate-limit middleware on the fetch path

**Files:**

- Modify: `crates/ui/web-api/src/oauth/cimd.rs` (call-side)
- Reference: `EndpointKind::CimdFetch` from Plan B's rate_limit.rs

- [ ] **Step 1: Wire rate-limit check**

Per spec §14.2: 5/min per `(source_ip, metadata_url)` bucket. Apply at fetcher entry, not as tower middleware (the
middleware variant requires HTTP route context; fetcher is invoked server-side from /oauth/authorize).

- [ ] **Step 2: Verify**

- [ ] **Step 3: Commit**

```bash
git commit -m "feat(oauth): CIMD fetch rate-limiting

Per spec §14.2."
```

### Task 13: Update MCP integration tests for prefix-dispatch

**Files:**

- Modify: existing `crates/ui/mcp/src/auth.rs` tests
- Create: `crates/ui/mcp/tests/oauth_prefix_dispatch.rs`

- [ ] **Step 1: Write test**

Cover all six prefix-dispatch outcomes from spec §6.1 + §20 Phase 1.5 master-switch gate:

1. `Bearer upk_...` valid → 200
2. `Bearer eyJ...` valid OAuth → 200 with `McpAuthMethod::OAuth` context
3. `Bearer eyJ...` invalid OAuth → 401 with
   `WWW-Authenticate: Bearer realm="mcp", resource_metadata="...", scope="mcp:read"`
4. `Bearer <garbage>` → 401 with PRM-discovery `WWW-Authenticate`
5. No Authorization header → 401 with PRM-discovery `WWW-Authenticate`
6. `oauth_enabled = false` AND `Bearer eyJ...` → 401 with NO `WWW-Authenticate` advertising OAuth discovery (PRM
   endpoint is 404 when OAuth disabled, so advertising it would lie). Body says "JWT tokens are not accepted for MCP
   access. Use an API token (upk\_...)" — matching the existing pre-OAuth error string for backward compatibility.

- [ ] **Step 2: Verify pass**

- [ ] **Step 3: Commit**

```bash
git commit -m "test(mcp): exhaustive prefix-dispatch behavior

Per spec §6.1."
```

### Task 14: Operator UI integration — RS-side wiring smoke test

**Files:**

- Create: `crates/ui/integration-tests/tests/oauth_end_to_end.rs` (Docker integration test, run with `--ignored`)

- [ ] **Step 1: Write E2E test**

Drive a complete flow: enable OAuth → register client manually via Operator API → simulate `/oauth/authorize` → mint
code → exchange at `/oauth/token` → call MCP tool with returned access token → assert tool succeeds with
`McpAuthMethod::OAuth` audit. Run against a TestApp with `oauth.mcp_enabled = true`.

- [ ] **Step 2: Verify pass**

Run: `cargo test -p uptrakit-integration-tests -- --ignored oauth_end_to_end` Requires Docker per
`docs/development/testing.md`.

- [ ] **Step 3: Commit**

```bash
git commit -m "test(integration): full OAuth + RS round-trip

End-to-end gate per spec §19."
```

### Task 15: Run full quality gates

- [ ] **Step 1: Run gates**

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo deny check
markdownlint --config .markdownlint.json '**/*.md'
```

- [ ] **Step 2: Fix any failures inline (no warnings suppressed)**

- [ ] **Step 3: Commit cleanups if any**

---

## Self-review checklist

- [ ] **Snapshot conformance**: CIMD fetcher uses `connect_timeout(10s)` + `timeout(60s)` + `SsrfSafeResolver::new()`
      (not `permissive()`); two-pass parse via `serde_json::Value` first; clock injected via
      `Arc<dyn Fn() -> OffsetDateTime + Send + Sync>`; SQLite read-then-write uses `BEGIN IMMEDIATE`; audit emission
      semantic, not target-string.
- [ ] **Idiomatic pattern check**: prefix dispatch matches the existing clone-and-swap Service pattern; JWT verifier
      reused from shared crate (no duplicate code); PRM mounted twice at both well-known paths; `ToolAuth` declarations
      co-located with tool handlers (one primary responsibility per file).
- [ ] **Documentation completeness**: utoipa annotations on PRM endpoint (inline acceptance); no doc-file updates here
      (Plan E).
- [ ] **Task atomicity**: each task is a single coherent change with its own commit.
- [ ] **Phase ordering**: requires Plan A + Plan B merged. Plan D can land independently of Plan C and Plan E.
