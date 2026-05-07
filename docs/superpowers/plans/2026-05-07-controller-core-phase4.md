# controller-core Phase 4 — MCP Decoupling + AppState Minimisation + ADR

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `McpState` (replaces `Arc<AppState>` in MCP code), move all MCP-bounded types from `web-api/src/mcp_compat.rs` into the `uptrakit-mcp` crate,
delete `mcp_compat.rs`, drop `uptrakit-web-api` from MCP's Cargo.toml, add `ServerState` and `PluginState` sub-structs to `AppState`, update
`PluginOpsState`/`GlobalProvidersState` `FromRef` impls, and write the ADR.

**Prerequisite:** Phase 3 plan complete and all CI gates passing.

**Phase 1 deviation check:** `AuthState` was left in `web-api/src/app_state.rs` during Phase 1 due to the orphan
rule. Phase 2 Task 2b moves it to controller-core using the `AuthStateSource` trait pattern. Before starting Phase
4, verify this migration is complete:

```bash
grep "pub struct AuthState" crates/ui/controller-core/src/auth/mod.rs
```

Expected: one match. If missing, complete Phase 2 Task 2b before proceeding — `McpState` in Task 1 below
imports `uptrakit_controller_core::auth::AuthState` and will not compile without it.

**Architecture:**

- `McpState` holds only the controller-core types MCP needs (`DbState`, `AuthState`, `Settings`, `default_tenant_id`, `controller_id`, `audit_emitter`,
  `shutdown_token`, `update_dispatcher`).
- MCP-bounded types (`McpRequestContext`, `McpAuthError`, `McpTriggerError`) live in `uptrakit-mcp`, not in web-api.
- `build_mcp_router` takes `McpState` (not `Arc<AppState>`); controller-runtime constructs `McpState` from the fields available in its startup
  context.
- `AppState` gains `ServerState` (pki_path + rustls_config) and `PluginState` (plugin_ops + global_providers) sub-structs. Existing `FromRef` impls
  updated to go through sub-structs.

**Tech Stack:** Same as prior phases; `uptrakit-controller-core` replaces `uptrakit-web-api` in MCP's deps.

**Standards binding:** `McpRequestContext`, `McpAuthError`, `McpTriggerError` carry `#[non_exhaustive]` (spec). `From<&UpdateDispatchError> for
McpTriggerError` wildcard arm uses `tracing::warn!`. `McpTriggerError` is NOT a wire type — no `Other(String)`.

---

## Task 1: Create `McpState` and `McpSettings` in `uptrakit-mcp`

**Files:**

- Create: `crates/ui/mcp/src/state.rs`
- Create: `crates/ui/mcp/src/settings.rs`
- Modify: `crates/ui/mcp/src/lib.rs` (declare modules)
- Modify: `crates/ui/mcp/Cargo.toml` (add controller-core, remove web-api)

- [ ] **Step 1: Add `uptrakit-controller-core` to `crates/ui/mcp/Cargo.toml` (do NOT remove web-api yet)**

```toml
uptrakit-controller-core = { workspace = true }
```

Keep `uptrakit-web-api` for now — it is removed in Task 6 after all usages are migrated.

- [ ] **Step 2: Create `crates/ui/mcp/src/state.rs`**

```rust
use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use uptrakit_audit_log::AuditEmitter;
use uptrakit_controller_core::auth::AuthState;
use uptrakit_controller_core::db::DbState;
use uptrakit_controller_core::settings::Settings;
use uptrakit_controller_core::update::UpdateDispatcher;

/// Focused state struct for the MCP server.
///
/// Contains only the controller-core fields needed by MCP tool handlers and
/// auth middleware. Has no dependency on `uptrakit-web-api` or `axum`.
///
/// At controller startup one `ControllerUpdateDispatcher` `Arc` is cloned into
/// both `AppState` and `McpState`.
///
/// `#[non_exhaustive]`: prevents external struct literal construction and forces
/// exhaustive pattern match sites to add `..`. Callers use `McpState::new(…)`.
/// Note: `::new()` callers must still be updated when fields are added — `#[non_exhaustive]`
/// does not protect constructor call sites. With one current constructor call site
/// (controller-runtime), this is acceptable.
#[non_exhaustive]
#[derive(Clone)]
pub struct McpState {
    pub db: DbState,
    pub auth: AuthState,
    pub settings: Settings,
    pub default_tenant_id: Uuid,
    pub controller_id: Uuid,
    pub audit_emitter: AuditEmitter,
    pub shutdown_token: CancellationToken,
    pub update_dispatcher: Arc<dyn UpdateDispatcher>,
}

impl McpState {
    pub fn new(
        db: DbState,
        auth: AuthState,
        settings: Settings,
        default_tenant_id: Uuid,
        controller_id: Uuid,
        audit_emitter: AuditEmitter,
        shutdown_token: CancellationToken,
        update_dispatcher: Arc<dyn UpdateDispatcher>,
    ) -> Self {
        Self {
            db, auth, settings, default_tenant_id, controller_id,
            audit_emitter, shutdown_token, update_dispatcher,
        }
    }
}
```

- [ ] **Step 3: Create `crates/ui/mcp/src/settings.rs`**

```rust
use std::net::SocketAddr;

use uptrakit_controller_core::settings::Settings;

/// MCP-specific projection of `Settings`.
///
/// Extracted once at startup to avoid threading the full `Settings` into every
/// handler that only needs the listening address or SANs.
pub struct McpSettings {
    pub sans: Vec<String>,
    pub https_addr: SocketAddr,
}

impl From<&Settings> for McpSettings {
    fn from(s: &Settings) -> Self {
        Self {
            sans: s.sans().to_vec(),
            https_addr: s.https_addr(),
        }
    }
}
```

Adjust the `s.sans()` and `s.https_addr()` calls to match the actual `Settings` API found in controller-core — check with:

```bash
grep -n "pub fn sans\|pub fn https_addr" crates/ui/controller-core/src/settings/mod.rs
```

- [ ] **Step 4: Declare new modules in `crates/ui/mcp/src/lib.rs`**

Add at the top of `lib.rs` (before the existing `pub mod auth;` line):

```rust
pub mod settings;
pub mod state;
```

- [ ] **Step 5: Verify compile**

```bash
cargo check --all-features 2>&1 | grep -E "^error" | head -10
```

Expected: no errors (lib.rs still uses `Arc<AppState>` — that's fine until Task 5).

- [ ] **Step 6: Commit**

```bash
git commit --only crates/ui/mcp/src/state.rs crates/ui/mcp/src/settings.rs \
    crates/ui/mcp/src/lib.rs crates/ui/mcp/Cargo.toml \
  -m "feat(mcp): add McpState and McpSettings structs"
```

---

## Task 2: Move MCP-bounded types to `uptrakit-mcp/src/context.rs`

**Files:**

- Create: `crates/ui/mcp/src/context.rs`
- Modify: `crates/ui/mcp/src/lib.rs` (declare context module)

`McpRequestContext`, `McpAuthError`, `McpTriggerError` currently live in `web-api/src/mcp_compat.rs`. They belong in `uptrakit-mcp`.

- [ ] **Step 1: Create `crates/ui/mcp/src/context.rs`**

```rust
use uuid::Uuid;

// Permission is a flat re-export from controller-core::auth — no `permissions` submodule.
use uptrakit_controller_core::auth::Permission;
use uptrakit_controller_core::update::UpdateDispatchError;

/// Per-request auth context injected into MCP request extensions by `McpAuthLayer`.
///
/// `#[non_exhaustive]`: OAuth 2.1 will add fields (scope claims, sub, etc.).
/// External code must use `McpRequestContext::new(…)`.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct McpRequestContext {
    pub user_id: Uuid,
    pub token_id: Uuid,
    pub tenant_id: Uuid,
    pub permissions: Vec<Permission>,
}

impl McpRequestContext {
    pub fn new(
        user_id: Uuid,
        token_id: Uuid,
        tenant_id: Uuid,
        permissions: Vec<Permission>,
    ) -> Self {
        Self {
            user_id,
            token_id,
            tenant_id,
            permissions,
        }
    }

    pub fn has_permission(&self, perm: &Permission) -> bool {
        self.permissions.contains(perm)
    }
}

/// Error variants for MCP authentication.
///
/// `#[non_exhaustive]`: OAuth 2.1 will introduce new rejection cases.
#[non_exhaustive]
#[derive(Debug)]
pub enum McpAuthError {
    MissingCredentials,
    JwtNotAccepted,
    Unauthorized,
    Forbidden,
    Internal,
}

/// Error variants for the MCP update-trigger tool.
///
/// NOT a wire type — converted to MCP tool error responses internally.
/// `#[non_exhaustive]`: future triggers may add variants (rate-limit, quota).
#[non_exhaustive]
#[derive(Debug)]
pub enum McpTriggerError {
    PermissionDenied,
    HostNotFound,
    SoftwareItemNotFound,
    NotConfigured,
    AgentUnavailable,
    AlreadyInProgress,
    Internal,
}

impl From<&UpdateDispatchError> for McpTriggerError {
    fn from(e: &UpdateDispatchError) -> Self {
        match e {
            UpdateDispatchError::HostNotFound => McpTriggerError::HostNotFound,
            UpdateDispatchError::SoftwareItemNotFound => McpTriggerError::SoftwareItemNotFound,
            UpdateDispatchError::UpdateAlreadyActive => McpTriggerError::AlreadyInProgress,
            UpdateDispatchError::NotConfigured => McpTriggerError::NotConfigured,
            UpdateDispatchError::AgentUnavailable => McpTriggerError::AgentUnavailable,
            UpdateDispatchError::Internal => McpTriggerError::Internal,
            _ => {
                tracing::warn!("unhandled UpdateDispatchError variant; mapping to McpTriggerError::Internal");
                McpTriggerError::Internal
            }
        }
    }
}
```

- [ ] **Step 2: Add `pub mod context;` to `crates/ui/mcp/src/lib.rs`**

- [ ] **Step 3: Verify compile**

```bash
cargo check --all-features 2>&1 | grep -E "^error" | head -10
```

- [ ] **Step 4: Commit**

```bash
git commit --only crates/ui/mcp/src/context.rs crates/ui/mcp/src/lib.rs \
  -m "feat(mcp): add McpRequestContext, McpAuthError, McpTriggerError in mcp crate"
```

---

## Task 3: Implement `validate_api_token_for_mcp` in `uptrakit-mcp/src/auth.rs`

**Files:**

- Modify: `crates/ui/mcp/src/auth.rs`

Replace the current `McpAuthLayer` which calls `validate_api_token_for_mcp` from `web-api/mcp_compat.rs` with a self-contained implementation using `controller-core`.

- [ ] **Step 1: Read the current `crates/ui/mcp/src/auth.rs`**

```bash
cat crates/ui/mcp/src/auth.rs
```

Note how `McpAuthLayer` calls `validate_api_token_for_mcp` and where it reads from `AppState`.

- [ ] **Step 2: Write new `validate_api_token_for_mcp` in `crates/ui/mcp/src/auth.rs`**

Add the standalone function (call sites in `McpAuthLayer` will switch from importing from `web-api` to using this local version in Task 5):

```rust
use uptrakit_audit_log::AuditOutcome;
use uptrakit_controller_core::auth::api_token::{
    authenticate_api_token, emit_api_token_auth_audit,
};
// Permission is a flat re-export — no `permissions` submodule in controller-core.
use uptrakit_controller_core::auth::{AuthFailure, Permission};

use crate::context::{McpAuthError, McpRequestContext};
use crate::state::McpState;

/// Validate a bearer token for an MCP request using only `McpState`.
///
/// Accepts `None` (missing header) or `Some(token_str)`. Handles the full
/// auth path: missing token, JWT rejection, DB lookup, `AccessMcp` permission
/// check, and audit emission. Does not require `AppState`.
pub async fn validate_api_token_for_mcp(
    state: &McpState,
    token: Option<&str>,
) -> Result<McpRequestContext, McpAuthError> {
    let token = match token {
        Some(t) if !t.is_empty() => t,
        _ => {
            emit_api_token_auth_audit(
                &state.audit_emitter,
                state.default_tenant_id,
                None,
                AuditOutcome::Denied,
                "missing_authorization_header",
            );
            return Err(McpAuthError::MissingCredentials);
        }
    };

    if !token.starts_with("upk_") {
        emit_api_token_auth_audit(
            &state.audit_emitter,
            state.default_tenant_id,
            None,
            AuditOutcome::Denied,
            "jwt_not_accepted_for_mcp",
        );
        return Err(McpAuthError::JwtNotAccepted);
    }

    let (auth_user, token_id) = match authenticate_api_token(
        state.db.db(),
        state.default_tenant_id,
        token,
    )
    .await
    {
        Ok(pair) => pair,
        Err(failure) => {
            if let Some(reason) = failure.api_token_reason_code() {
                emit_api_token_auth_audit(
                    &state.audit_emitter,
                    state.default_tenant_id,
                    None,
                    AuditOutcome::Denied,
                    reason,
                );
            }
            return Err(match failure {
                AuthFailure::UserDeactivated => McpAuthError::Forbidden,
                AuthFailure::InternalError => McpAuthError::Internal,
                _ => McpAuthError::Unauthorized,
            });
        }
    };

    if !auth_user.has_permission(Permission::AccessMcp) {
        emit_api_token_auth_audit(
            &state.audit_emitter,
            state.default_tenant_id,
            None,
            AuditOutcome::Denied,
            "missing_access_mcp_permission",
        );
        return Err(McpAuthError::Forbidden);
    }

    emit_api_token_auth_audit(
        &state.audit_emitter,
        state.default_tenant_id,
        None,
        AuditOutcome::Success,
        "authenticated",
    );

    Ok(McpRequestContext::new(
        auth_user.user_id,
        token_id,
        state.default_tenant_id,
        auth_user.permissions,
    ))
}
```

- [ ] **Step 3: Verify compile**

```bash
cargo check --all-features 2>&1 | grep -E "^error" | head -10
```

- [ ] **Step 4: Commit**

```bash
git commit --only crates/ui/mcp/src/auth.rs \
  -m "feat(mcp): implement validate_api_token_for_mcp using controller-core"
```

---

## Task 4: Implement `mcp_trigger_update` in `uptrakit-mcp/src/tools/`

**Files:**

- Modify or Create: `crates/ui/mcp/src/tools/update.rs` (check if exists)

- [ ] **Step 1: Check existing tools structure**

```bash
ls crates/ui/mcp/src/tools/
```

If `update.rs` exists, read it. Otherwise create it.

- [ ] **Step 2: Verify `ActorInfo::new` and `UpdateDispatchParams::new` signatures before writing**

```bash
grep -n "pub fn new" crates/ui/controller-core/src/update.rs | head -10
```

Confirm the exact argument count and order for both constructors. The code below assumes a specific
signature — adjust if the actual API differs.

- [ ] **Step 3: Write `mcp_trigger_update` in tools/update.rs (or the appropriate tools file)**

```rust
use uuid::Uuid;

use uptrakit_controller_core::update::{DispatchOutcome, UpdateDispatchParams};
use uptrakit_web_api_queries::queries::update_types::ActorType;

use crate::context::{McpRequestContext, McpTriggerError};
use crate::state::McpState;

/// Trigger a software update from an MCP tool call.
///
/// Uses `McpState.update_dispatcher` — no `AppState` required.
/// Returns `(update_history_id, DispatchOutcome)` on success.
pub async fn mcp_trigger_update(
    state: &McpState,
    ctx: &McpRequestContext,
    host_id: Uuid,
    software_item_id: Uuid,
    to_version: String,
) -> Result<(Uuid, DispatchOutcome), McpTriggerError> {
    // UpdateDispatchParams is #[non_exhaustive] — must use ::new().
    // ActorInfo is also #[non_exhaustive] — must use ActorInfo::new().
    let params = UpdateDispatchParams::new(
        ctx.tenant_id,
        host_id,
        software_item_id,
        to_version,
        uptrakit_controller_core::update::ActorInfo::new(ActorType::ApiToken, ctx.token_id.to_string()),
        None,
        false,
    );

    state
        .update_dispatcher
        .dispatch(params)
        .await
        .map(|r| (r.update_history_id, r.outcome))
        .map_err(|e| McpTriggerError::from(e.current_context()))
}
```

- [ ] **Step 4: Update tool handler(s) to call the local `mcp_trigger_update`**

Find the MCP tool handler that currently calls `uptrakit_web_api::mcp_compat::mcp_trigger_update`:

```bash
grep -rn "mcp_trigger_update\|mcp_compat" crates/ui/mcp/src/tools/
```

Update each call site to use `crate::tools::update::mcp_trigger_update` with `McpState` (not `Arc<AppState>`). The tool handler will need access to
`McpState` — this is wired in Task 5.

- [ ] **Step 5: Verify compile**

```bash
cargo check --all-features 2>&1 | grep -E "^error" | head -10
```

- [ ] **Step 6: Commit**

```bash
git commit --only crates/ui/mcp/src/tools/ \
  -m "feat(mcp): implement mcp_trigger_update using McpState + UpdateDispatcher"
```

---

## Task 5: Switch `build_mcp_router` to take `McpState`

**Files:**

- Modify: `crates/ui/mcp/src/lib.rs`
- Modify: `crates/core/controller-runtime/src/server.rs`

`build_mcp_router` currently takes `Arc<AppState>`. Change to `McpState` (not wrapped in `Arc` — callers can clone as needed since `McpState: Clone`).

- [ ] **Step 1: Update `McpAuthLayer` to use `McpState`**

In `crates/ui/mcp/src/auth.rs`, change `McpAuthLayer::new` to accept `McpState` instead of `Arc<AppState>`. Update the layer's Service impl to call
`validate_api_token_for_mcp(state, …)` (the local version from Task 3).

Search for current `McpAuthLayer` internals:

```bash
grep -n "AppState\|Arc<AppState>" crates/ui/mcp/src/auth.rs
```

Replace each `Arc<AppState>` usage with `McpState` (cloning from the layer's stored `McpState`).

- [ ] **Step 2: Update `McpHandler` to use `McpState`**

Find how `McpHandler` accesses `AppState`:

```bash
grep -n "AppState\|app_state\|Arc<AppState>" crates/ui/mcp/src/tools/*.rs 2>/dev/null | head -20
```

Change `McpHandler::new` to accept `McpState` instead of `Arc<AppState>`. Update all tool call sites to use `McpState` fields.

- [ ] **Step 3: Rewrite `build_mcp_router` in `crates/ui/mcp/src/lib.rs`**

```rust
use crate::state::McpState;

pub fn build_mcp_router(state: McpState) -> axum::Router {
    let mcp_settings = crate::settings::McpSettings::from(&state.settings);
    let config = build_config(&mcp_settings);
    let raw_service = rmcp::transport::streamable_http_server::StreamableHttpService::new(
        {
            let state = state.clone();
            move || Ok(crate::tools::McpHandler::new(state.clone()))
        },
        std::sync::Arc::new(
            rmcp::transport::streamable_http_server::session::local::LocalSessionManager::default(),
        ),
        config,
    );

    let auth_layer = crate::auth::McpAuthLayer::new(state.clone());
    let service = tower::ServiceBuilder::new()
        .layer(auth_layer)
        .service(raw_service);

    axum::Router::new().nest_service("/mcp", service)
}

fn build_config(settings: &crate::settings::McpSettings) -> rmcp::transport::streamable_http_server::StreamableHttpServerConfig {
    let allowed_hosts = build_allowed_hosts(&settings.sans);
    rmcp::transport::streamable_http_server::StreamableHttpServerConfig::default()
        .with_allowed_hosts(allowed_hosts)
}
```

Remove `use uptrakit_web_api::AppState;` from `lib.rs`.

- [ ] **Step 4a: Add `McpState`-construction accessors to `AppState` in `crates/ui/web-api/src/app_state.rs`**

Most `AppState` fields are `pub(crate)` — controller-runtime (a different crate) cannot access them directly.
First check which fields used in the `McpState::new(…)` call in Step 4b are already public:

```bash
grep -n "pub\s\+auth\|pub\s\+settings\|pub\s\+audit_emitter\|pub\s\+update_dispatcher\|pub\s\+shutdown_token\|pub\s\+controller_id\|pub\s\+default_tenant_id" \
  crates/ui/web-api/src/app_state.rs | head -20
```

For every field that is NOT `pub` (only `pub(crate)`), add a corresponding accessor. Common pattern — add all six
below, removing any that turn out to already be accessible:

```rust
/// Returns a clone of the database state for constructing `McpState`.
pub fn db_state(&self) -> uptrakit_controller_core::db::DbState {
    self.db.clone()
}

/// Returns the auth state for constructing `McpState`.
pub fn auth_state(&self) -> uptrakit_controller_core::auth::AuthState {
    self.auth.clone()
}

/// Returns settings for constructing `McpState`.
pub fn settings(&self) -> uptrakit_controller_core::settings::Settings {
    self.settings.clone()
}

/// Returns the audit emitter for constructing `McpState`.
pub fn audit_emitter(&self) -> uptrakit_audit_log::AuditEmitter {
    self.audit_emitter.clone()
}

/// Returns the update dispatcher for constructing `McpState`.
pub fn update_dispatcher(&self) -> std::sync::Arc<dyn uptrakit_controller_core::update::UpdateDispatcher> {
    std::sync::Arc::clone(&self.update_dispatcher)
}

/// Returns the shutdown token for constructing `McpState`.
pub fn shutdown_token(&self) -> tokio_util::sync::CancellationToken {
    self.shutdown_token.clone()
}
```

For `default_tenant_id` and `controller_id`: if they are already `pub`, use direct field access in Step 4b.
If not, add equivalent accessors. Check with the grep above.

Place these with the other pub accessor methods near the `db()` method
(search: `grep -n "pub fn db(" crates/ui/web-api/src/app_state.rs`).

- [ ] **Step 4b: Update `controller-runtime/src/server.rs` to build `McpState`**

In `crates/core/controller-runtime/src/server.rs`, change the `#[cfg(feature = "mcp")]` block:

```rust
#[cfg(feature = "mcp")]
{
    // McpState is #[non_exhaustive] — must use ::new() (struct literal would fail
    // since controller-runtime is a different crate).
    // All field accesses use the pub accessors added in Step 4a.
    let mcp_state = uptrakit_mcp::state::McpState::new(
        cfg.app_state.db_state(),
        cfg.app_state.auth_state(),
        cfg.app_state.settings(),
        cfg.app_state.default_tenant_id,  // verify pub or add accessor
        cfg.app_state.controller_id,      // verify pub or add accessor
        cfg.app_state.audit_emitter(),
        cfg.app_state.shutdown_token(),
        cfg.app_state.update_dispatcher(),
    );
    router = router.merge(uptrakit_mcp::build_mcp_router(mcp_state));
}
```

- [ ] **Step 5: Verify compile**

```bash
cargo check --all-features 2>&1 | grep -E "^error" | head -20
```

Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git commit --only crates/ui/mcp/src/ crates/core/controller-runtime/src/server.rs \
  -m "feat(mcp): switch build_mcp_router to McpState, remove Arc<AppState>"
```

---

## Task 6: Delete `mcp_compat.rs` and remove web-api dep from MCP

**Files:**

- Delete: `crates/ui/web-api/src/mcp_compat.rs`
- Modify: `crates/ui/web-api/src/lib.rs` (remove mcp_compat module + re-exports)
- Modify: `crates/ui/mcp/Cargo.toml` (remove uptrakit-web-api dep)

- [ ] **Step 1: Verify no remaining usages of `mcp_compat` exports in web-api**

```bash
grep -rn "mcp_compat\|McpRequestContext\|McpAuthError\|McpTriggerError\|mcp_trigger_update\|validate_api_token_for_mcp" \
  crates/ui/web-api/src/ | grep -v "mcp_compat.rs"
```

Expected: no output (all usages should be removed in prior tasks or in web-api tests). If any remain, remove them.

- [ ] **Step 2: Remove `mcp_compat` from `crates/ui/web-api/src/lib.rs`**

Remove these lines:

```rust
pub mod mcp_compat;
pub use mcp_compat::{
    McpAuthError, McpRequestContext, McpTriggerError, mcp_trigger_update,
    validate_api_token_for_mcp,
};
```

- [ ] **Step 3: Delete `crates/ui/web-api/src/mcp_compat.rs`**

```bash
git rm crates/ui/web-api/src/mcp_compat.rs
```

- [ ] **Step 4: Remove `uptrakit-web-api` from `crates/ui/mcp/Cargo.toml`**

Delete the line:

```toml
uptrakit-web-api = { workspace = true }
```

- [ ] **Step 5: Verify compile**

```bash
cargo check --all-features 2>&1 | grep -E "^error" | head -20
```

Expected: no errors.

- [ ] **Step 6: Verify MCP has no remaining web-api dep**

```bash
cargo tree -p uptrakit-mcp | grep uptrakit-web-api
```

Expected: **no output**. This is the primary constraint from the spec.

- [ ] **Step 7: Commit**

```bash
git commit --only crates/ui/web-api/src/lib.rs crates/ui/mcp/Cargo.toml \
    crates/ui/web-api/src/mcp_compat.rs \
  -m "feat(mcp): delete mcp_compat.rs, remove uptrakit-web-api dep from mcp"
```

---

## Task 7: Add `ServerState` and `PluginState` sub-structs to `AppState`

**Files:**

- Modify: `crates/ui/web-api/src/app_state.rs`

`AppState` currently has loose fields `pki_path`, `rustls_config`, `plugin_ops`, `global_providers`. Group them into sub-structs. Route handlers using
`PluginOpsState`/`GlobalProvidersState` extractors are unchanged — only the `FromRef` impls update.

- [ ] **Step 1: Define `ServerState` and `PluginState` in `app_state.rs`**

Add before the `AppState` struct definition:

```rust
/// Grouped TLS server configuration for hot-reload.
///
/// `#[non_exhaustive]`: fields may be added (e.g. OCSP stapling config).
#[non_exhaustive]
#[derive(Clone)]
pub struct ServerState {
    pub pki_path: std::path::PathBuf,
    pub rustls_config: axum_server::tls_rustls::RustlsConfig,
}

impl ServerState {
    pub fn new(
        pki_path: std::path::PathBuf,
        rustls_config: axum_server::tls_rustls::RustlsConfig,
    ) -> Self {
        Self { pki_path, rustls_config }
    }
}

/// Grouped plugin-ops state for plugin configuration and global provider runtimes.
///
/// `#[non_exhaustive]`: fields may be added (e.g. plugin metrics registry).
#[non_exhaustive]
#[derive(Clone)]
pub struct PluginState {
    pub plugin_ops: Arc<dyn uptrakit_plugin_infrastructure_registry::PluginOps>,
    pub global_providers: Arc<crate::global_providers::GlobalProviders>,
}

impl PluginState {
    pub fn new(
        plugin_ops: Arc<dyn uptrakit_plugin_infrastructure_registry::PluginOps>,
        global_providers: Arc<crate::global_providers::GlobalProviders>,
    ) -> Self {
        Self { plugin_ops, global_providers }
    }
}
```

- [ ] **Step 2: Replace loose fields in `AppState` struct**

Change:

```rust
// Before:
pub plugin_ops: Arc<dyn PluginOps>,
pub global_providers: Arc<crate::global_providers::GlobalProviders>,
// ...
pub pki_path: std::path::PathBuf,
pub rustls_config: axum_server::tls_rustls::RustlsConfig,

// After:
pub plugin: PluginState,
pub server: ServerState,
```

- [ ] **Step 3: Update `AppStateBuilder` fields and setters**

In `AppStateBuilder`, replace:

- `plugin_ops: Option<Arc<dyn PluginOps>>` and `global_providers: Option<…>` with individual fields (keep as-is since they're set independently,
  but `build()` now wraps them in `PluginState`)
- `pki_path: Option<PathBuf>` and `rustls_config: Option<RustlsConfig>` similarly

In `build()`, change the assignment:

```rust
// Before:
plugin_ops: self.plugin_ops.unwrap_or_else(|| { … }),
global_providers,
pki_path: self.pki_path.ok_or(AppStateBuildError("pki_path"))?,
rustls_config: self.rustls_config.ok_or(AppStateBuildError("rustls_config"))?,

// After — use ::new() constructors (project convention for #[non_exhaustive] structs):
plugin: PluginState::new(
    self.plugin_ops.unwrap_or_else(|| { … }),
    global_providers,
),
server: ServerState::new(
    self.pki_path.ok_or(AppStateBuildError("pki_path"))?,
    self.rustls_config.ok_or(AppStateBuildError("rustls_config"))?,
),
```

- [ ] **Step 4: Update `PluginOpsState` and `GlobalProvidersState` `FromRef` impls**

Find these impls near the end of `app_state.rs`:

```bash
grep -n "impl FromRef.*PluginOpsState\|impl FromRef.*GlobalProvidersState" crates/ui/web-api/src/app_state.rs
```

Change to go through the new sub-struct:

```rust
// Before:
impl FromRef<Arc<AppState>> for PluginOpsState {
    fn from_ref(state: &Arc<AppState>) -> Self {
        PluginOpsState(state.plugin_ops.clone())
    }
}

impl FromRef<Arc<AppState>> for GlobalProvidersState {
    fn from_ref(state: &Arc<AppState>) -> Self {
        GlobalProvidersState(state.global_providers.clone())
    }
}

// After:
impl FromRef<Arc<AppState>> for PluginOpsState {
    fn from_ref(state: &Arc<AppState>) -> Self {
        PluginOpsState(state.plugin.plugin_ops.clone())
    }
}

impl FromRef<Arc<AppState>> for GlobalProvidersState {
    fn from_ref(state: &Arc<AppState>) -> Self {
        GlobalProvidersState(state.plugin.global_providers.clone())
    }
}
```

- [ ] **Step 5: Update all `state.plugin_ops` and `state.global_providers` field accesses in web-api**

```bash
grep -rn "state\.plugin_ops\|state\.global_providers\|state\.pki_path\|state\.rustls_config" \
  crates/ui/web-api/src/ | grep -v "app_state.rs"
```

For each occurrence:

- `state.plugin_ops` → `state.plugin.plugin_ops`
- `state.global_providers` → `state.plugin.global_providers`
- `state.pki_path` → `state.server.pki_path`
- `state.rustls_config` → `state.server.rustls_config`

Also update the `plugin_ops()` and `global_providers()` accessor methods on `AppState`:

```rust
pub fn plugin_ops(&self) -> Arc<dyn PluginOps> {
    self.plugin.plugin_ops.clone()
}
pub fn global_providers(&self) -> Arc<crate::global_providers::GlobalProviders> {
    self.plugin.global_providers.clone()
}
```

- [ ] **Step 6: Update `controller_update_protection()` and `controller_update_hook()` methods**

These call `self.plugin_ops.controller_update_protection()` — update to `self.plugin.plugin_ops.controller_update_protection()`.

- [ ] **Step 7: Verify compile**

```bash
cargo check --all-features 2>&1 | grep -E "^error" | head -20
```

Expected: no errors. Fix any remaining field accesses.

- [ ] **Step 8: Commit**

```bash
git commit --only crates/ui/web-api/src/ \
  -m "refactor(web-api): group plugin/server fields into PluginState, ServerState sub-structs"
```

---

## Task 8: Write the ADR

**Files:**

- Create: `docs/adr/NNNN-controller-core-boundary.md` (NNNN = next available number)

- [ ] **Step 1: Find the next ADR number**

```bash
ls docs/adr/ | sort | tail -5
```

Note the highest number shown. Use next integer as NNNN in the filename and title.

- [ ] **Step 2: Write `docs/adr/NNNN-controller-core-boundary.md`**

Replace `NNNN` in the filename and in the `#` title below with the actual number from Step 1.

```markdown
# NNNN — Introduce `uptrakit-controller-core` as Business-Logic Boundary

**Date:** 2026-05-07
**Status:** Accepted

## Context

`uptrakit-mcp` depended on `uptrakit-web-api` to access shared state types and
helper functions (`authenticate_api_token`, `mcp_trigger_update`, etc.). This
created a cross-concern dependency that: (a) prevented the MCP server from being
deployed separately from the HTTP server, and (b) would force OAuth 2.1 MCP
authorisation machinery to land in the wrong crate.

The project is preparing to add OAuth 2.1 MCP authorisation (future spec). That
work must land in `uptrakit-mcp`, not in `uptrakit-web-api`. Adding it to mcp
while mcp depends on web-api for core state types would create a tangled
dependency graph that is hard to untangle later.

## Decision

Introduce `uptrakit-controller-core` as a pure business-logic crate with zero
knowledge of `uptrakit-web-api` or `uptrakit-mcp`. Both `web-api` and `mcp`
depend on `controller-core`; neither depends on the other.

The crate boundary is enforced by the absence of `uptrakit-web-api` and
`uptrakit-mcp` path deps in `controller-core/Cargo.toml`. Verified post-Phase 4
by `cargo tree -p uptrakit-mcp | grep uptrakit-web-api` producing no output.

## `crates/ui/` Placement

`controller-core` is pure domain logic yet lives in `crates/ui/` alongside HTTP
and CLI crates. This was chosen over `crates/core/controller-core` for two
reasons:

1. **Co-location with consumers.** Both `web-api` and `mcp` (the primary
   consumers) live in `crates/ui/`. Placing `controller-core` there reduces
   cross-directory import paths and keeps related crates adjacent.

2. **`crates/core/` already has a different signal.** `crates/core/` contains
   the agent runtime, controller runtime, MQTT, and scheduler — background
   services. `controller-core` is not a service; it is shared state and logic.
   Placing it in `crates/core/` would mislead contributors into thinking it
   runs as a service.

Contributors should NOT use the `crates/ui/` directory location as a signal
that `controller-core` has any UI, HTTP, or Axum concerns. The `lib.rs`
invariant doc-comment makes this explicit.

## Alternatives Considered

1. **Keep `mcp → web-api` dep.** Rejected: would force OAuth 2.1 auth machinery
   into `web-api`, bloating it with auth concerns that do not belong there and
   making future MCP standalone deployment harder.

2. **God-struct bundle — expose all types from `web-api` as a flat dep.**
   Rejected: amplifies the coupling problem rather than resolving it.

3. **Place in `crates/core/controller-core`.** Rejected: misleads contributors
   (see placement rationale above). Kept in `crates/ui/` by majority preference.

## Consequences

- `uptrakit-mcp` has zero `uptrakit-web-api` imports (verified by CI).
- OAuth 2.1 MCP auth work lands in `uptrakit-mcp` from day one.
- `AppState` is smaller: grouped into `ServerState` and `PluginState` sub-structs.
- `ControllerUpdateDispatcher` is the single testable production impl of
  `UpdateDispatcher`; tests inject `NoopUpdateDispatcher`.
- All consumers of `authenticate_api_token` pass explicit `db`/`default_tenant_id`
  instead of threading `&AppState` — consistent with `AgentCertSigner` pattern.
```

- [ ] **Step 3: Verify markdown linting**

```bash
npx prettier --write docs/adr/NNNN-controller-core-boundary.md
markdownlint --config .markdownlint.json docs/adr/NNNN-controller-core-boundary.md
```

Expected: no errors.

- [ ] **Step 4: Verify `CONTEXT.md` requires no changes**

Per the spec: "no domain term changes required; this is an architectural boundary, not a new domain concept." Confirm by checking that `CONTEXT.md` has
no stale references to `mcp_compat` or descriptions implying MCP depends on web-api:

```bash
grep -n "mcp_compat\|mcp.*web-api\|web-api.*mcp" CONTEXT.md || echo "no stale refs"
```

Expected: "no stale refs". If any references to the old dependency exist, update them.

- [ ] **Step 5: Commit**

```bash
git commit --only docs/adr/ \
  -m "docs(adr): document controller-core boundary decision (NNNN)"
```

---

## Task 9: Phase 4 CI quality gate + spec constraint verification

**Files:** None modified — verification only.

- [ ] **Step 1: Format**

```bash
cargo fmt --all && git diff --stat
```

- [ ] **Step 2: Full check suite**

```bash
cargo check --no-default-features --features db-sqlite 2>&1 | grep -E "^error"
cargo check --all-features 2>&1 | grep -E "^error"
cargo clippy --all-targets --no-default-features --features db-sqlite 2>&1 | grep -E "^error"
cargo clippy --all-targets --all-features 2>&1 | grep -E "^error"
cargo test --all-features 2>&1 | tail -20
cargo deny check
```

Expected: all clean.

- [ ] **Step 3: Verify the primary spec constraint**

```bash
cargo tree -p uptrakit-mcp | grep uptrakit-web-api
```

**Expected: no output.** If any output appears, the constraint is violated — investigate which feature or dep chain is pulling it in and fix before
marking Phase 4 complete.

- [ ] **Step 4: Commit any residual fixes**

```bash
git commit --only crates/ui/ crates/core/ \
  -m "chore: Phase 4 fmt/clippy cleanup"
```

---

## Self-Review

**Spec coverage:**

- [x] `McpState` struct with zero web-api fields (Task 1)
- [x] `McpSettings` projection (Task 1)
- [x] `McpRequestContext` in mcp crate with `#[non_exhaustive]` (Task 2)
- [x] `McpAuthError` in mcp crate with `#[non_exhaustive]` (Task 2)
- [x] `McpTriggerError` in mcp crate with `#[non_exhaustive]` (Task 2)
- [x] `From<&UpdateDispatchError> for McpTriggerError` wildcard arm + `tracing::warn!` (Task 2)
- [x] `validate_api_token_for_mcp` in `mcp/src/auth.rs` using `McpState` (Task 3)
- [x] `mcp_trigger_update` using `McpState.update_dispatcher` (Task 4)
- [x] `build_mcp_router` takes `McpState` not `Arc<AppState>` (Task 5)
- [x] `mcp_compat.rs` deleted (Task 6)
- [x] `uptrakit-web-api` removed from MCP Cargo.toml (Task 6)
- [x] `cargo tree -p uptrakit-mcp | grep uptrakit-web-api` produces no output (Task 9)
- [x] `ServerState` sub-struct (Task 7)
- [x] `PluginState` sub-struct (Task 7)
- [x] `PluginOpsState` / `GlobalProvidersState` `FromRef` updated (Task 7)
- [x] ADR written (Task 8)
- [x] `controller-core/src/lib.rs` invariant doc-comment (Phase 1, Task 1)

**Type consistency:** `McpRequestContext` uses `Permission` from `uptrakit_controller_core::auth::Permission` (flat re-export — no `permissions`
submodule in controller-core). Verify the re-export exists after Phase 1 Task 4. If not, add it to `controller-core/src/auth/mod.rs`.

**Idiom audit — `McpTriggerError` wildcard arm:** The wildcard arm in `From<&UpdateDispatchError> for McpTriggerError` uses `tracing::warn!` per codebase
convention for `#[non_exhaustive]` enums at external match sites. The `Failed` variant from `DispatchOutcome` in `send_completed` (Phase 3, Task 2) also
has a wildcard with warn. Both are consistent.
