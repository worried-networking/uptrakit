# Extract MCP Server into `uptrakit-mcp` Crate — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move `crates/ui/web-api/src/mcp/` into a new `crates/ui/mcp/` crate (`uptrakit-mcp`), leaving
`web-api` with two thin public bridge functions and no `rmcp`/`vt100`/`schemars` dependencies.

**Architecture:** Two phases. Phase 1 adds `McpRequestContext`, `McpAuthError`/`validate_api_token_for_mcp`,
and `McpTriggerError`/`mcp_trigger_update` to a new `web-api/src/mcp_compat.rs`, then migrates the existing
`mcp/auth.rs` and `mcp/tools/update.rs` to call those bridge functions — all in-place, no structural change.
Phase 2 creates the new crate, moves the files (rewriting `crate::` paths), removes `mcp/` from `web-api`,
and rewires `controller-runtime`.

**Tech Stack:** Rust, rmcp 1.5 (streamable-http-server), sea-orm, axum, tower, rootcause, uptrakit-audit-log.
`rmcp`, `vt100`, `schemars` are already promoted to `[workspace.dependencies]` (done in prior commit).

---

## File Structure

### New files

- `crates/ui/web-api/src/mcp_compat.rs` — `McpRequestContext`, `McpAuthError`,
  `validate_api_token_for_mcp`, `McpTriggerError`, `mcp_trigger_update`
- `crates/ui/mcp/Cargo.toml` — new crate manifest
- `crates/ui/mcp/src/lib.rs` — `build_mcp_router`, `build_config`, `build_allowed_hosts` (from `mcp/mod.rs`)
- `crates/ui/mcp/src/auth.rs` — `McpAuthLayer`, `McpAuthService` (from `mcp/auth.rs`, rewritten)
- `crates/ui/mcp/src/terminal.rs` — `render_terminal_output` (from `mcp/terminal.rs`, unchanged logic)
- `crates/ui/mcp/src/tools/mod.rs` — `McpHandler`, `mcp_error` (from `mcp/tools/mod.rs`, import-updated)
- `crates/ui/mcp/src/tools/history.rs` — (from `mcp/tools/history.rs`, import-updated)
- `crates/ui/mcp/src/tools/user.rs` — (from `mcp/tools/user.rs`, import-updated)
- `crates/ui/mcp/src/tools/update.rs` — (from `mcp/tools/update.rs`, rewritten to call bridge fn)

### Modified files

- `crates/ui/web-api/src/lib.rs` — add `pub mod mcp_compat`, re-export types; later remove mcp feature
- `crates/ui/web-api/src/mcp/auth.rs` — replace `McpRequestContext` def, simplify `call()`
- `crates/ui/web-api/src/mcp/tools/update.rs` — rewrite to call `mcp_trigger_update`
- `crates/ui/web-api/Cargo.toml` — remove `mcp` feature + optional rmcp/vt100/schemars deps
- `Cargo.toml` (workspace root) — add `uptrakit-mcp` to members + workspace deps
- `crates/core/controller-runtime/Cargo.toml` — change mcp feature, add `uptrakit-mcp`
- `crates/core/controller-runtime/src/server.rs` — change call site to `uptrakit_mcp::`

### Deleted (Phase 2)

- `crates/ui/web-api/src/mcp/` — entire directory

---

## Phase 1 — Prepare `web-api` (in-place, no structural change)

---

### Task 1: Create `mcp_compat.rs` with `McpRequestContext`

`McpRequestContext` currently lives in `mcp/auth.rs`. Extract it to an unconditional `pub` module
so the new crate can import it without a circular dependency.

**Files:**

- Create: `crates/ui/web-api/src/mcp_compat.rs`
- Modify: `crates/ui/web-api/src/lib.rs` (two lines)
- Modify: `crates/ui/web-api/src/mcp/auth.rs` (replace definition)

- [ ] **Step 1: Create `mcp_compat.rs`**

```rust
// crates/ui/web-api/src/mcp_compat.rs

use uuid::Uuid;

use crate::auth::permissions::Permission;

/// Per-request auth context injected into MCP request extensions by `McpAuthLayer`.
///
/// Tool handlers extract this from request extensions:
/// `parts.extensions.get::<McpRequestContext>()`.
///
/// `#[non_exhaustive]`: OAuth 2.1 will add fields (scope claims, sub, etc.).
/// External code must use `McpRequestContext::new(...)`.
#[derive(Clone, Debug)]
#[non_exhaustive]
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

    /// Returns `true` if the user holds `perm`.
    pub fn has_permission(&self, perm: &Permission) -> bool {
        self.permissions.contains(perm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync<T: Send + Sync + Clone>() {}

    #[test]
    fn mcp_request_context_is_clone_send_sync() {
        assert_send_sync::<McpRequestContext>();
    }
}
```

- [ ] **Step 2: Wire `mcp_compat` into `lib.rs`**

Open `crates/ui/web-api/src/lib.rs`. After the existing `pub mod` declarations (around line 40),
add two lines — one unconditional module declaration, one re-export:

```rust
pub mod mcp_compat;
pub use mcp_compat::McpRequestContext;
```

Place them _before_ the `#[cfg(feature = "mcp")] pub mod mcp;` block so they compile
regardless of the `mcp` feature.

- [ ] **Step 3: Replace `McpRequestContext` in `mcp/auth.rs`**

In `crates/ui/web-api/src/mcp/auth.rs`:

Remove the full `McpRequestContext` struct definition (lines 17–47, including the `impl` block and
the example doc-comment block).

Replace with a single re-export so existing code in `mcp/` keeps compiling with the same path:

```rust
pub use crate::McpRequestContext;
```

Also remove the test `mcp_request_context_is_clone_send_sync` from `auth.rs` — it now lives in
`mcp_compat.rs`.

- [ ] **Step 4: Verify compilation**

```bash
cargo check -p uptrakit-web-api --all-features
```

Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git commit --only crates/ui/web-api/src/mcp_compat.rs \
           crates/ui/web-api/src/lib.rs \
           crates/ui/web-api/src/mcp/auth.rs \
           -m "refactor(mcp): extract McpRequestContext to mcp_compat for crate boundary prep"
```

---

### Task 2: Add `McpAuthError` and `validate_api_token_for_mcp` bridge

This function wraps the three `pub(crate)` items (`authenticate_api_token`,
`emit_api_token_auth_audit`, `AuthFailure`) so `McpAuthLayer` can call a single `pub` function
instead of internals.

**Files:**

- Modify: `crates/ui/web-api/src/mcp_compat.rs`
- Modify: `crates/ui/web-api/src/lib.rs`

- [ ] **Step 1: Add imports and `McpAuthError` to `mcp_compat.rs`**

At the top of `mcp_compat.rs`, add these imports below the existing `use uuid::Uuid`:

```rust
use crate::AppState;
use crate::middleware::require_auth::{AuthFailure, authenticate_api_token, emit_api_token_auth_audit};
```

Then add `McpAuthError` after the `McpRequestContext` impl block:

```rust
/// Error variants for MCP authentication.
///
/// `#[non_exhaustive]`: OAuth 2.1 will introduce new rejection cases (e.g. scope mismatch).
#[derive(Debug)]
#[non_exhaustive]
pub enum McpAuthError {
    /// No `Authorization` header or empty bearer token.
    MissingCredentials,
    /// Token is present but not an `upk_`-prefixed API token (e.g. a JWT).
    JwtNotAccepted,
    /// API token is invalid, expired, or revoked.
    Unauthorized,
    /// User is deactivated or lacks the `AccessMcp` permission.
    Forbidden,
    /// Internal error during validation.
    Internal,
}
```

- [ ] **Step 2: Add `validate_api_token_for_mcp` to `mcp_compat.rs`**

Append after `McpAuthError`:

```rust
/// Validate a bearer token for an MCP request.
///
/// Accepts `None` (missing `Authorization` header) or `Some(token_str)`. Handles the
/// full auth path: missing token, JWT rejection, DB lookup, `AccessMcp` permission check,
/// and audit emission.
///
/// # TODO
///
/// Replace with OAuth 2.1 Resource Server / Authorization Server validation when that
/// feature lands. At that point `McpAuthLayer` drops this import and owns its own
/// validation logic.
pub async fn validate_api_token_for_mcp(
    state: &AppState,
    token: Option<&str>,
) -> Result<McpRequestContext, McpAuthError> {
    let token = match token {
        Some(t) if !t.is_empty() => t,
        _ => {
            emit_api_token_auth_audit(
                state,
                None,
                uptrakit_audit_log::AuditOutcome::Denied,
                "missing_authorization_header",
            );
            return Err(McpAuthError::MissingCredentials);
        }
    };

    if !token.starts_with("upk_") {
        emit_api_token_auth_audit(
            state,
            None,
            uptrakit_audit_log::AuditOutcome::Denied,
            "jwt_not_accepted_for_mcp",
        );
        return Err(McpAuthError::JwtNotAccepted);
    }

    let (auth_user, token_id) = match authenticate_api_token(state, token).await {
        Ok(pair) => pair,
        Err(failure) => {
            if let Some(reason) = failure.api_token_reason_code() {
                emit_api_token_auth_audit(
                    state,
                    None,
                    uptrakit_audit_log::AuditOutcome::Denied,
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
            state,
            None,
            uptrakit_audit_log::AuditOutcome::Denied,
            "missing_access_mcp_permission",
        );
        return Err(McpAuthError::Forbidden);
    }

    emit_api_token_auth_audit(
        state,
        None,
        uptrakit_audit_log::AuditOutcome::Success,
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

- [ ] **Step 3: Export from `lib.rs`**

Extend the existing `pub use mcp_compat::McpRequestContext;` line (added in Task 1) to also
export the new types:

```rust
pub use mcp_compat::{McpAuthError, McpRequestContext, validate_api_token_for_mcp};
```

- [ ] **Step 4: Verify compilation**

```bash
cargo check -p uptrakit-web-api --all-features
```

Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git commit --only crates/ui/web-api/src/mcp_compat.rs \
           crates/ui/web-api/src/lib.rs \
           -m "feat(mcp): add validate_api_token_for_mcp bridge function"
```

---

### Task 3: Add `McpTriggerError` and `mcp_trigger_update` bridge

Wraps `trigger_update` + `spawn_protection_and_dispatch` + `emit_software_update_audit`
(all `pub(crate)`) behind a single `pub` function. The returned `(Uuid, TriggerUpdateStatus)` tuple
uses only workspace-shared types — no circular dependency with the future `uptrakit-mcp` crate.

**Files:**

- Modify: `crates/ui/web-api/src/mcp_compat.rs`
- Modify: `crates/ui/web-api/src/lib.rs`

- [ ] **Step 1: Add imports to `mcp_compat.rs`**

Add these to the top of `mcp_compat.rs` (after existing imports):

```rust
use std::sync::Arc;

use uptrakit_web_api_types::software_items::TriggerUpdateStatus;

use crate::auth::AuthMethod;
use crate::middleware::require_auth::{AuthenticatedApiTokenId, AuthenticatedUser};
use crate::queries::update_triggers::TriggerUpdateParams;
use crate::queries::update_types::ActorType;
use uptrakit_web_api_queries::queries::update_dispatch::TriggerUpdateError;
```

- [ ] **Step 2: Add `McpTriggerError` to `mcp_compat.rs`**

Append after `validate_api_token_for_mcp`:

```rust
/// Error variants for the MCP update-trigger bridge.
///
/// Maps the full `TriggerUpdateError` surface from `uptrakit-web-api-queries` so that
/// MCP tool handlers can produce meaningful protocol-level errors rather than collapsing
/// everything to a generic internal error.
///
/// `#[non_exhaustive]`: future triggers (rate-limit, quota) may add variants.
#[derive(Debug)]
#[non_exhaustive]
pub enum McpTriggerError {
    PermissionDenied,
    HostNotFound,
    SoftwareItemNotFound,
    /// Host exists but lacks assignment, plugin config, or a known plugin type.
    NotConfigured,
    /// Host has no linked agent or agent is not in Approved status.
    AgentUnavailable,
    AlreadyInProgress,
    Internal,
}
```

- [ ] **Step 3: Add `mcp_trigger_update` to `mcp_compat.rs`**

Append after `McpTriggerError`:

```rust
/// Trigger a software update for an MCP tool call.
///
/// Wraps `actions::software_items::trigger_update`, `update_orchestrator::spawn_protection_and_dispatch`,
/// and `routes::software_items::emit_software_update_audit` — all of which are `pub(crate)` in `web-api`.
///
/// Returns `(update_history_id, TriggerUpdateStatus)` using only types from shared workspace crates,
/// so `uptrakit-mcp` can call this without a circular dependency.
pub async fn mcp_trigger_update(
    state: Arc<AppState>,
    ctx: &McpRequestContext,
    host_id: Uuid,
    software_item_id: Uuid,
    to_version: String,
) -> Result<(Uuid, TriggerUpdateStatus), rootcause::Report<McpTriggerError>> {
    let actor_id_str = ctx.token_id.to_string();
    let tenant_db = crate::tenant_db::TenantDb(uptrakit_shared_db::TenantDb::new(
        state.db().clone(),
        ctx.tenant_id,
    ));
    let mut_ctx = state.mutation_context();

    let audit_user = AuthenticatedUser {
        user_id: ctx.user_id,
        auth_method: AuthMethod::ApiToken,
        permissions: ctx.permissions.clone(),
        jti: None,
    };
    let audit_token = AuthenticatedApiTokenId(ctx.token_id);

    let trigger_result = crate::actions::software_items::trigger_update(
        &tenant_db,
        &mut_ctx,
        TriggerUpdateParams {
            tenant_id: ctx.tenant_id,
            item_id: software_item_id,
            host_id,
            to_version: to_version.clone(),
            actor_type: ActorType::ApiToken.as_str(),
            actor_id: &actor_id_str,
            release_info: None,
            interactive: false,
        },
    )
    .await
    .map_err(|err| {
        let (outcome, reason_code) = err.current_context().trigger_audit_classification();
        crate::routes::software_items::emit_software_update_audit(
            &state,
            ctx.tenant_id,
            &audit_user,
            Some(audit_token),
            software_item_id,
            outcome,
            serde_json::json!({
                "host_id": host_id,
                "to_version": to_version,
                "interactive": false,
                "reason_code": reason_code,
            }),
        );
        let mcp_err = match err.current_context() {
            TriggerUpdateError::HostNotFound => McpTriggerError::HostNotFound,
            TriggerUpdateError::SoftwareItemNotFound => McpTriggerError::SoftwareItemNotFound,
            TriggerUpdateError::UpdateAlreadyActive => McpTriggerError::AlreadyInProgress,
            TriggerUpdateError::HostNotAssigned
            | TriggerUpdateError::NoExecuteUpdatePlugin
            | TriggerUpdateError::PluginConfigNotFound
            | TriggerUpdateError::UnknownPluginType(_) => McpTriggerError::NotConfigured,
            TriggerUpdateError::NoAgent | TriggerUpdateError::AgentNotApproved => {
                McpTriggerError::AgentUnavailable
            }
            _ => McpTriggerError::Internal,
        };
        rootcause::report!(mcp_err)
    })?;

    if let Some(work) = trigger_result.pending_protection_work {
        crate::update_orchestrator::spawn_protection_and_dispatch(Arc::clone(&state), *work);
    }

    let status = match trigger_result.initial_status {
        uptrakit_shared_db::entity::update_history::UpdateStatus::Pending => {
            TriggerUpdateStatus::Pending
        }
        uptrakit_shared_db::entity::update_history::UpdateStatus::Failed => {
            TriggerUpdateStatus::Failed
        }
        _ => TriggerUpdateStatus::Queued,
    };

    let audit_outcome = if matches!(status, TriggerUpdateStatus::Failed) {
        uptrakit_audit_log::AuditOutcome::Failed
    } else {
        uptrakit_audit_log::AuditOutcome::Success
    };

    crate::routes::software_items::emit_software_update_audit(
        &state,
        ctx.tenant_id,
        &audit_user,
        Some(audit_token),
        software_item_id,
        audit_outcome,
        serde_json::json!({
            "host_id": host_id,
            "to_version": to_version,
            "interactive": false,
            "update_history_id": trigger_result.update_history_id,
            "dispatch_status": status.to_string(),
        }),
    );

    Ok((trigger_result.update_history_id, status))
}
```

- [ ] **Step 4: Export from `lib.rs`**

Extend the re-export line in `lib.rs`:

```rust
pub use mcp_compat::{
    McpAuthError, McpRequestContext, McpTriggerError,
    mcp_trigger_update, validate_api_token_for_mcp,
};
```

- [ ] **Step 5: Verify compilation**

```bash
cargo check -p uptrakit-web-api --all-features
```

Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git commit --only crates/ui/web-api/src/mcp_compat.rs \
           crates/ui/web-api/src/lib.rs \
           -m "feat(mcp): add mcp_trigger_update bridge function"
```

---

### Task 4: Migrate `mcp/auth.rs` to call the bridge function

Replace all direct `pub(crate)` usage (`authenticate_api_token`, `AuthFailure`,
`emit_api_token_auth_audit`) with a single call to `validate_api_token_for_mcp`.

**Files:**

- Modify: `crates/ui/web-api/src/mcp/auth.rs`

- [ ] **Step 1: Rewrite `mcp/auth.rs`**

Replace the entire file content with:

```rust
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tower::{Layer, Service};

use crate::AppState;
use crate::{McpAuthError, validate_api_token_for_mcp};

pub use crate::McpRequestContext;

// ---------------------------------------------------------------------------
// Tower layer
// ---------------------------------------------------------------------------

/// Tower [`Layer`] that validates API-token credentials before forwarding to
/// the underlying [`StreamableHttpService`].
///
/// Rejects missing tokens, JWT tokens, and invalid API tokens. On success,
/// inserts [`McpRequestContext`] into request extensions for tool handlers.
#[derive(Clone)]
pub struct McpAuthLayer {
    state: Arc<AppState>,
}

impl McpAuthLayer {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }
}

impl<S> Layer<S> for McpAuthLayer {
    type Service = McpAuthService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        McpAuthService {
            inner,
            state: Arc::clone(&self.state),
        }
    }
}

// ---------------------------------------------------------------------------
// Tower service
// ---------------------------------------------------------------------------

/// Tower [`Service`] produced by [`McpAuthLayer`].
#[derive(Clone)]
pub struct McpAuthService<S> {
    inner: S,
    state: Arc<AppState>,
}

impl<S, B> Service<axum::extract::Request<B>> for McpAuthService<S>
where
    S: Service<axum::extract::Request<B>> + Clone + Send + 'static,
    S::Response: IntoResponse,
    S::Error: Into<std::convert::Infallible>,
    S::Future: Send + 'static,
    B: Send + 'static,
{
    type Response = Response;
    type Error = std::convert::Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(|e| e.into())
    }

    fn call(&mut self, mut req: axum::extract::Request<B>) -> Self::Future {
        let state = Arc::clone(&self.state);
        // Standard Tower clone-and-swap so the ready clone is used.
        let mut inner = self.inner.clone();
        std::mem::swap(&mut inner, &mut self.inner);

        Box::pin(async move {
            let token = extract_bearer_token(&req);
            let mcp_ctx =
                match validate_api_token_for_mcp(&state, token.as_deref()).await {
                    Ok(ctx) => ctx,
                    Err(McpAuthError::MissingCredentials) => {
                        return Ok(unauthorized(
                            "Authentication required: provide an API token via \
                             Authorization: Bearer <upk_...>",
                        ));
                    }
                    Err(McpAuthError::JwtNotAccepted) => {
                        return Ok(unauthorized(
                            "JWT tokens are not accepted for MCP access. \
                             Use an API token (upk_...)",
                        ));
                    }
                    Err(McpAuthError::Forbidden) => {
                        return Ok(plain(
                            StatusCode::FORBIDDEN,
                            "User is deactivated or lacks the AccessMcp permission",
                        ));
                    }
                    Err(McpAuthError::Internal) => {
                        return Ok(plain(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "Internal server error",
                        ));
                    }
                    Err(_) => {
                        return Ok(unauthorized("Invalid or revoked API token"));
                    }
                };

            req.extensions_mut().insert(mcp_ctx);
            inner
                .call(req)
                .await
                .map(IntoResponse::into_response)
                .map_err(Into::into)
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn extract_bearer_token<B>(req: &axum::extract::Request<B>) -> Option<String> {
    req.headers()
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::to_owned)
}

fn plain(status: StatusCode, body: &'static str) -> Response {
    axum::http::Response::builder()
        .status(status)
        .header(
            axum::http::header::CONTENT_TYPE,
            "text/plain; charset=utf-8",
        )
        .body(axum::body::Body::from(body))
        .expect("valid response builder arguments")
}

fn unauthorized(body: &'static str) -> Response {
    plain(StatusCode::UNAUTHORIZED, body)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync<T: Send + Sync + Clone>() {}

    #[test]
    fn mcp_auth_layer_types_are_send_sync() {
        assert_send_sync::<McpAuthLayer>();
    }
}
```

- [ ] **Step 2: Verify compilation**

```bash
cargo check -p uptrakit-web-api --all-features
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git commit --only crates/ui/web-api/src/mcp/auth.rs \
           -m "refactor(mcp): migrate McpAuthLayer to validate_api_token_for_mcp bridge"
```

---

### Task 5: Migrate `mcp/tools/update.rs` to call the bridge function

Replaces the direct `pub(crate)` calls (`trigger_update`, `spawn_protection_and_dispatch`,
`emit_software_update_audit`) with a call to `mcp_trigger_update`. The DTO types
(`TriggerUpdateInput`, `TriggerUpdateResult`) stay in this file — they move with it to the new
crate in Phase 2.

**Files:**

- Modify: `crates/ui/web-api/src/mcp/tools/update.rs`

- [ ] **Step 1: Rewrite `update.rs`**

Replace the entire file content with:

```rust
use rmcp::{ErrorData, Json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uptrakit_web_api_types::software_items::TriggerUpdateStatus;
use uuid::Uuid;

use crate::mcp::auth::McpRequestContext;
use crate::mcp::tools::{McpHandler, mcp_error};
use crate::{McpTriggerError, mcp_trigger_update};

// ---------------------------------------------------------------------------
// Input / output types
// ---------------------------------------------------------------------------

/// Input parameters for the `trigger_update` MCP tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TriggerUpdateInput {
    /// UUID of the host to update.
    pub host_id: String,
    /// UUID of the software item to update.
    pub software_item_id: String,
    /// Target version string (e.g. `"1.2.3"`).
    pub to_version: String,
}

/// Result returned by the `trigger_update` MCP tool.
#[derive(Debug, Serialize, JsonSchema)]
pub struct TriggerUpdateResult {
    /// UUID of the created update history record.
    pub update_history_id: String,
    /// Dispatch status: `"pending"`, `"queued"`, or `"failed"`.
    pub status: String,
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

impl McpHandler {
    /// Core logic for `trigger_update`.
    pub(crate) async fn trigger_update_impl(
        &self,
        ctx: McpRequestContext,
        input: TriggerUpdateInput,
    ) -> Result<Json<TriggerUpdateResult>, ErrorData> {
        use crate::auth::permissions::Permission;
        if !ctx.has_permission(&Permission::TriggerUpdates) {
            return Err(ErrorData::invalid_request(
                "permission denied: TriggerUpdates required",
                None,
            ));
        }

        let host_id = input.host_id.parse::<Uuid>().map_err(|_| {
            ErrorData::invalid_params(
                format!("invalid host_id UUID: {}", input.host_id),
                None,
            )
        })?;

        let software_item_id = input.software_item_id.parse::<Uuid>().map_err(|_| {
            ErrorData::invalid_params(
                format!("invalid software_item_id UUID: {}", input.software_item_id),
                None,
            )
        })?;

        let (update_history_id, status) = mcp_trigger_update(
            std::sync::Arc::clone(&self.state),
            &ctx,
            host_id,
            software_item_id,
            input.to_version.clone(),
        )
        .await
        .map_err(|err| mcp_error(format!("trigger_update failed: {err}")))?;

        Ok(Json(TriggerUpdateResult {
            update_history_id: update_history_id.to_string(),
            status: status.to_string(),
        }))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_update_input_type_exists() {
        let input = TriggerUpdateInput {
            host_id: Uuid::nil().to_string(),
            software_item_id: Uuid::nil().to_string(),
            to_version: "1.0.0".to_owned(),
        };
        assert_eq!(input.to_version, "1.0.0");

        let result = TriggerUpdateResult {
            update_history_id: Uuid::nil().to_string(),
            status: "queued".to_owned(),
        };
        let json = serde_json::to_string(&result).expect("serialisation must succeed");
        assert!(json.contains("queued"));
    }

    #[test]
    fn mcp_trigger_error_variants_exist() {
        // Compile-time check that the variants named in the spec are present.
        let _ = McpTriggerError::PermissionDenied;
        let _ = McpTriggerError::HostNotFound;
        let _ = McpTriggerError::SoftwareItemNotFound;
        let _ = McpTriggerError::NotConfigured;
        let _ = McpTriggerError::AgentUnavailable;
        let _ = McpTriggerError::AlreadyInProgress;
        let _ = McpTriggerError::Internal;
    }
}
```

- [ ] **Step 2: Run full Phase 1 gate**

```bash
cargo check --all-features && cargo test --all-features
```

Expected: all checks and tests pass.

- [ ] **Step 3: Commit**

```bash
git commit --only crates/ui/web-api/src/mcp/tools/update.rs \
           -m "refactor(mcp): migrate trigger_update_impl to mcp_trigger_update bridge"
```

---

## Phase 2 — Create crate and move files

---

### Task 6: Scaffold the `uptrakit-mcp` crate

**Files:**

- Create: `crates/ui/mcp/Cargo.toml`
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Create `crates/ui/mcp/Cargo.toml`**

```toml
[package]
name = "uptrakit-mcp"
description = "MCP server for uptrakit — transport, auth layer, and tools"
edition.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
version.workspace = true

[dependencies]
uptrakit-web-api       = { workspace = true }
uptrakit-web-api-types = { workspace = true }
uptrakit-shared-db     = { workspace = true }
rmcp                   = { workspace = true }
vt100                  = { workspace = true }
schemars               = { workspace = true }
sea-orm                = { workspace = true }
axum                   = { workspace = true }
tower                  = { workspace = true }
uuid                   = { workspace = true }
serde                  = { workspace = true }
serde_json             = { workspace = true }
```

Create a placeholder `crates/ui/mcp/src/lib.rs`:

```rust
// placeholder — filled in Task 7
```

- [ ] **Step 2: Register in workspace `Cargo.toml`**

In the root `Cargo.toml`, add `"crates/ui/mcp"` to `[workspace.members]`. Also add to
`[workspace.dependencies]`:

```toml
uptrakit-mcp = { path = "crates/ui/mcp", version = "0.0.1" }
```

- [ ] **Step 3: Verify scaffold compiles**

```bash
cargo check -p uptrakit-mcp
```

Expected: no errors (empty lib).

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml crates/ui/mcp/Cargo.toml crates/ui/mcp/src/lib.rs
git commit --only Cargo.toml crates/ui/mcp/Cargo.toml crates/ui/mcp/src/lib.rs \
           -m "chore: scaffold uptrakit-mcp crate"
```

---

### Task 7: Move files to `uptrakit-mcp`, rewrite imports

Copy each file from `web-api/src/mcp/` to `crates/ui/mcp/src/`, rewriting `crate::` paths per the
import-path table. Files are listed in dependency order (no file depends on a file listed after it).

**Files:**

- Create: `crates/ui/mcp/src/terminal.rs`
- Create: `crates/ui/mcp/src/tools/mod.rs`
- Create: `crates/ui/mcp/src/tools/history.rs`
- Create: `crates/ui/mcp/src/tools/user.rs`
- Create: `crates/ui/mcp/src/tools/update.rs`
- Create: `crates/ui/mcp/src/auth.rs`
- Replace: `crates/ui/mcp/src/lib.rs`

- [ ] **Step 1: Copy `terminal.rs`**

`crates/ui/mcp/src/terminal.rs` — copy verbatim from `web-api/src/mcp/terminal.rs`.
No import changes needed; it only uses `vt100` (a direct dep of the new crate).

Update the doc-comment example path:

```rust
// Change:
// use uptrakit_web_api::mcp::terminal::render_terminal_output;
// To:
// use uptrakit_mcp::terminal::render_terminal_output;
```

- [ ] **Step 2: Create `tools/mod.rs`**

Copy from `web-api/src/mcp/tools/mod.rs`. Apply these import changes:

```rust
// Remove:
use crate::AppState;
use crate::mcp::auth::McpRequestContext;

// Add:
use std::sync::Arc;
use uptrakit_web_api::AppState;
use uptrakit_web_api::McpRequestContext;
```

All other imports (`rmcp::*`) are unchanged (direct deps of new crate).

- [ ] **Step 3: Create `tools/history.rs`**

Copy from `web-api/src/mcp/tools/history.rs`. Apply these import changes:

```rust
// Remove:
use crate::auth::permissions::Permission;
use crate::mcp::auth::McpRequestContext;
use crate::mcp::tools::{McpHandler, mcp_error};
use crate::queries;

// Add:
use uptrakit_web_api::auth::permissions::Permission;
use uptrakit_web_api::McpRequestContext;
use crate::tools::{McpHandler, mcp_error};
use uptrakit_web_api::queries;
```

Also change the one intra-crate terminal call on the line that reads
`crate::mcp::terminal::render_terminal_output`:

```rust
// Remove:
let rendered = crate::mcp::terminal::render_terminal_output(record.output.as_bytes());

// Add:
let rendered = crate::terminal::render_terminal_output(record.output.as_bytes());
```

- [ ] **Step 4: Create `tools/user.rs`**

Copy from `web-api/src/mcp/tools/user.rs`. Apply these import changes:

```rust
// Remove:
use crate::mcp::auth::McpRequestContext;
use crate::mcp::tools::{McpHandler, mcp_error};

// Add:
use uptrakit_web_api::McpRequestContext;
use crate::tools::{McpHandler, mcp_error};
```

In the `#[cfg(test)]` block, change:

```rust
// Remove:
use crate::auth::permissions::Permission;

// Add:
use uptrakit_web_api::auth::permissions::Permission;
```

- [ ] **Step 5: Create `tools/update.rs`**

Copy from `web-api/src/mcp/tools/update.rs` (the Phase-1-migrated version). Apply these
import changes:

```rust
// Remove:
use crate::mcp::auth::McpRequestContext;
use crate::mcp::tools::{McpHandler, mcp_error};
use crate::{McpTriggerError, mcp_trigger_update};

// Add:
use uptrakit_web_api::McpRequestContext;
use uptrakit_web_api::{McpTriggerError, mcp_trigger_update};
use crate::tools::{McpHandler, mcp_error};
```

Also change the permission import inside `trigger_update_impl`:

```rust
// Remove:
use crate::auth::permissions::Permission;

// Add:
use uptrakit_web_api::auth::permissions::Permission;
```

- [ ] **Step 6: Create `auth.rs`**

Copy from `web-api/src/mcp/auth.rs` (the Phase-1-migrated version). Apply these import changes:

```rust
// Remove:
use crate::AppState;
use crate::{McpAuthError, validate_api_token_for_mcp};
pub use crate::McpRequestContext;

// Add:
use uptrakit_web_api::AppState;
use uptrakit_web_api::{McpAuthError, McpRequestContext, validate_api_token_for_mcp};
```

`pub use crate::McpRequestContext;` is no longer needed — `McpRequestContext` is
imported directly above. Remove it.

- [ ] **Step 7: Write `src/lib.rs`**

Replace the placeholder with the full router, copying from `web-api/src/mcp/mod.rs` and
applying import changes:

```rust
use std::sync::Arc;

use axum::Router;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService,
    session::local::LocalSessionManager,
};
use uptrakit_web_api::AppState;

use crate::auth::McpAuthLayer;
use crate::tools::McpHandler;

pub mod auth;
pub mod terminal;
pub mod tools;

/// Mount the MCP Streamable HTTP transport at `/mcp`.
///
/// The returned router has no axum state type parameter (`Router<()>`); the
/// `McpHandler` captures `Arc<AppState>` directly so no `.with_state()` call
/// is needed.
pub fn build_mcp_router(state: Arc<AppState>) -> Router {
    let config = build_config(&state);
    let raw_service = StreamableHttpService::new(
        {
            let state = Arc::clone(&state);
            move || Ok(McpHandler::new(Arc::clone(&state)))
        },
        Arc::new(LocalSessionManager::default()),
        config,
    );

    let auth_layer = McpAuthLayer::new(Arc::clone(&state));
    let service = tower::ServiceBuilder::new()
        .layer(auth_layer)
        .service(raw_service);

    Router::new().nest_service("/mcp", service)
}

fn build_config(state: &AppState) -> StreamableHttpServerConfig {
    let sans = state.settings.sans();
    let allowed_hosts = build_allowed_hosts(&sans);
    StreamableHttpServerConfig::default().with_allowed_hosts(allowed_hosts)
}

fn build_allowed_hosts(sans: &[String]) -> Vec<String> {
    let mut hosts = Vec::with_capacity(sans.len() * 4);
    for san in sans {
        hosts.push(san.clone());
        hosts.push(format!("{san}:9443"));
        hosts.push(format!("{san}:443"));
        hosts.push(format!("{san}:80"));
    }
    hosts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_allowed_hosts_includes_port_variants() {
        let sans = vec!["controller.example.com".to_string()];
        let hosts = build_allowed_hosts(&sans);
        assert!(hosts.contains(&"controller.example.com".to_string()));
        assert!(hosts.contains(&"controller.example.com:9443".to_string()));
        assert!(hosts.contains(&"controller.example.com:443".to_string()));
        assert!(hosts.contains(&"controller.example.com:80".to_string()));
    }

    #[test]
    fn build_allowed_hosts_empty_sans() {
        let hosts = build_allowed_hosts(&[]);
        assert!(hosts.is_empty());
    }
}
```

- [ ] **Step 8: Verify new crate compiles**

```bash
cargo check -p uptrakit-mcp
```

Expected: no errors.

- [ ] **Step 9: Commit**

```bash
git add crates/ui/mcp/src/
git commit --only crates/ui/mcp/src/ \
           -m "feat(uptrakit-mcp): move mcp/ files to new crate, rewrite import paths"
```

---

### Task 8: Remove `mcp/` from `web-api` and clean up Cargo

**Files:**

- Delete: `crates/ui/web-api/src/mcp/` (directory)
- Modify: `crates/ui/web-api/src/lib.rs`
- Modify: `crates/ui/web-api/Cargo.toml`

- [ ] **Step 1: Delete the `mcp/` directory**

```bash
rm -rf crates/ui/web-api/src/mcp
```

- [ ] **Step 2: Update `web-api/src/lib.rs`**

Remove these two lines:

```rust
#[cfg(feature = "mcp")]
pub mod mcp;
```

and:

```rust
#[cfg(feature = "mcp")]
pub use mcp::build_mcp_router;
```

`build_mcp_router` is now in `uptrakit-mcp`. Nothing in `web-api` re-exports it — callers
(`controller-runtime`) will import it directly from `uptrakit_mcp`.

- [ ] **Step 3: Remove `mcp` feature from `web-api/Cargo.toml`**

In `crates/ui/web-api/Cargo.toml`:

Remove the feature definition:

```toml
mcp = ["dep:rmcp", "dep:vt100", "dep:schemars"]
```

Also remove the `optional = true` flag from `rmcp`, `schemars`, and `vt100` since they are no
longer optional in web-api (they move to `uptrakit-mcp`):

```toml
# Remove these three lines entirely from [dependencies]:
rmcp = { workspace = true, optional = true }
schemars = { workspace = true, optional = true }
vt100 = { workspace = true, optional = true }
```

- [ ] **Step 4: Sub-gate — verify before touching `controller-runtime`**

```bash
cargo check --all-features
```

Expected: no errors. If `cargo check` fails here, do not proceed to Task 9 — fix the errors first.

- [ ] **Step 5: Commit**

```bash
git commit --only crates/ui/web-api/src/ \
           crates/ui/web-api/Cargo.toml \
           -m "refactor(web-api): remove mcp/ module, mcp feature, and rmcp/vt100/schemars deps"
```

---

### Task 9: Wire `controller-runtime` to `uptrakit-mcp`

**Files:**

- Modify: `crates/core/controller-runtime/Cargo.toml`
- Modify: `crates/core/controller-runtime/src/server.rs`

- [ ] **Step 1: Update `controller-runtime/Cargo.toml`**

Change the `mcp` feature from:

```toml
mcp = ["uptrakit-web-api/mcp"]
```

to:

```toml
mcp = ["dep:uptrakit-mcp"]
```

Add `uptrakit-mcp` as an optional dependency in `[dependencies]`:

```toml
uptrakit-mcp = { workspace = true, optional = true }
```

- [ ] **Step 2: Update `controller-runtime/src/server.rs`**

Change the `#[cfg(feature = "mcp")]` block from:

```rust
#[cfg(feature = "mcp")]
{
    router = router.merge(uptrakit_web_api::build_mcp_router(Arc::clone(
        &cfg.app_state,
    )));
}
```

to:

```rust
#[cfg(feature = "mcp")]
{
    router = router.merge(uptrakit_mcp::build_mcp_router(Arc::clone(
        &cfg.app_state,
    )));
}
```

- [ ] **Step 3: Run full gate**

```bash
cargo check --all-features && cargo test --all-features
```

Expected: all checks and tests pass.

- [ ] **Step 4: Per-crate isolation check**

```bash
cargo test -p uptrakit-mcp
cargo test -p uptrakit-web-api
```

Expected: both pass independently.

- [ ] **Step 5: Commit**

```bash
git commit --only crates/core/controller-runtime/Cargo.toml \
           crates/core/controller-runtime/src/server.rs \
           -m "feat: wire controller-runtime to uptrakit-mcp crate"
```

---

## Completion Checklist

- [ ] `cargo check --all-features` — green
- [ ] `cargo test --all-features` — green
- [ ] `cargo test -p uptrakit-mcp` — green
- [ ] `cargo test -p uptrakit-web-api` — green (no mcp module, no rmcp dep)
- [ ] `web-api/Cargo.toml` has no `mcp` feature, no `rmcp`/`vt100`/`schemars` deps
- [ ] `crates/ui/web-api/src/mcp/` directory does not exist
- [ ] `crates/ui/mcp/` exists with 8 source files
- [ ] `mcp_compat.rs` in `web-api` exports `McpRequestContext`, `McpAuthError`,
  `McpTriggerError`, `validate_api_token_for_mcp`, `mcp_trigger_update`
