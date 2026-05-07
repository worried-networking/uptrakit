# controller-core Phase 2 — Auth Extraction + NotificationState Move

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Phase 2a: add `ServiceConnectionRegistry` re-export, extract `authenticate_api_token` and `emit_api_token_auth_audit` with explicit-parameter
signatures into `controller-core`, update all call sites, remove Phase 1 shims. Phase 2b: audit `NotificationState` components for web-api-specific deps,
resolve them (move `AdminEvent` to `uptrakit-wire`, introduce `Arc<dyn NatsPublisher>` abstraction), then move `NotificationState` into `controller-core`.

**Prerequisite:** Phase 1 plan complete and all CI gates passing.

**Architecture:**

- `authenticate_api_token` and `emit_api_token_auth_audit` move from `web-api/middleware/require_auth.rs` to `controller-core/src/auth/api_token.rs` with
  `AppState` replaced by explicit `db`/`default_tenant_id`/`audit_emitter` params.
- `AdminEvent` moves from `uptrakit-web-api-types` to `uptrakit-wire` — this unblocks `EventBroadcaster` moving to controller-core.
- `NatsTransport` stays in web-api; `NotificationService` and `EventBroadcaster` hold `Arc<dyn NatsPublisher>` instead, with `NatsTransport` implementing
  the trait in web-api.

**Tech Stack:** Same as Phase 1. Additional: `uptrakit-notification-delivery` added to controller-core Cargo.toml for `NotificationDispatcher`.

**Standards binding:** New explicit-param signatures avoid `&AppState` threading — consistent with `AgentCertSigner`, `CommandExecutor` patterns in this
codebase. `#[non_exhaustive]` retained on `AuthFailure`. Wildcard match arm with `tracing::warn!` on any `UpdateDispatchError` → `McpTriggerError`
conversions later.

---

## Task 1: Add `connections.rs` re-export to controller-core

**Files:**

- Modify: `crates/ui/controller-core/src/connections.rs`

`ServiceConnectionRegistry` already lives in `crates/ui/service-connections/`. controller-core re-exports it so callers can import from a single place.

- [ ] **Step 1: Write `crates/ui/controller-core/src/connections.rs`**

```rust
pub use uptrakit_service_connections::ServiceConnectionRegistry;
```

- [ ] **Step 2: Verify compile**

```bash
cargo check --all-features 2>&1 | grep -E "^error" | head -10
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git commit --only crates/ui/controller-core/src/connections.rs \
  -m "refactor(controller-core): add ServiceConnectionRegistry re-export"
```

---

## Task 2: Extract `authenticate_api_token` + `emit_api_token_auth_audit`

**Files:**

- Modify: `crates/ui/controller-core/src/auth/api_token.rs`
- Modify: `crates/ui/web-api/src/middleware/require_auth.rs` (update local wrappers to delegate)

Both functions currently live in `require_auth.rs` as `pub(crate)`. After this task they live in controller-core with `pub` visibility and explicit params
instead of `&AppState`.

Current `emit_api_token_auth_audit` signature (require_auth.rs:64):

```rust
pub(crate) fn emit_api_token_auth_audit(state: &AppState, request_id: Option<String>,
    outcome: AuditOutcome, reason_code: &'static str)
```

Current `authenticate_api_token` signature (require_auth.rs:294):

```rust
pub(crate) async fn authenticate_api_token(state: &AppState, token: &str)
    -> Result<(AuthenticatedUser, Uuid), AuthFailure>
```

- [ ] **Step 1: Write `crates/ui/controller-core/src/auth/api_token.rs`**

```rust
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use uuid::Uuid;

use uptrakit_audit_log::{AuditActionType, AuditActorType, AuditEntry, AuditEmitter, AuditOutcome};
use uptrakit_shared_db::entity::prelude::*;
use uptrakit_shared_db::entity::{permission, role_permission, user_role};
use uptrakit_web_api_auth::auth::api_token::ApiTokenService;
use uptrakit_web_api_auth::auth::permissions::Permission;
use uptrakit_web_api_auth::auth::{AuthError, AuthMethod};

use crate::auth::{AuthFailure, AuthenticatedUser};

/// Emit an audit log entry for an API token authentication attempt.
///
/// Replaces the web-api–local `emit_api_token_auth_audit(state, …)` call with
/// explicit params so this helper can be used from controller-core contexts that
/// do not have access to `AppState`.
pub fn emit_api_token_auth_audit(
    audit_emitter: &AuditEmitter,
    default_tenant_id: Uuid,
    request_id: Option<String>,
    outcome: AuditOutcome,
    reason_code: &'static str,
) {
    let entry = AuditEntry::builder(AuditActionType::AUTH_API_TOKEN_AUTHENTICATE)
        .tenant_scope(default_tenant_id)
        .actor(AuditActorType::ApiToken, None)
        .outcome(outcome)
        .details(serde_json::json!({ "reason_code": reason_code }))
        .request_id_opt(request_id)
        .build();

    if let Ok(entry) = entry {
        audit_emitter.emit_best_effort(entry);
    }
}

/// Authenticate a `upk_`-prefixed API token via DB lookup.
///
/// Returns `(AuthenticatedUser, token_id_uuid)` on success.
/// Callers pass explicit `db` and `default_tenant_id` rather than `&AppState`.
pub async fn authenticate_api_token(
    db: &DatabaseConnection,
    default_tenant_id: Uuid,
    token: &str,
) -> Result<(AuthenticatedUser, Uuid), AuthFailure> {
    let service = ApiTokenService::new(db.clone());

    let (user_id, token_id) = service
        .verify_token(token)
        .await
        .map_err(|error| classify_api_token_verify_error(&error))?;

    let user = User::find_by_id(user_id)
        .one(db)
        .await
        .map_err(|_| AuthFailure::InternalError)?
        .ok_or(AuthFailure::UserNotFound)?;

    if !user.is_active {
        return Err(AuthFailure::UserDeactivated);
    }

    let permissions = get_user_permissions(db, default_tenant_id, user_id)
        .await
        .map_err(|e| {
            tracing::error!(err = %e, user_id = %user_id, "failed to load user permissions");
            AuthFailure::InternalError
        })?;

    Ok((
        AuthenticatedUser {
            user_id,
            auth_method: AuthMethod::ApiToken,
            permissions,
            jti: None,
        },
        token_id,
    ))
}

async fn get_user_permissions(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    user_id: Uuid,
) -> uptrakit_web_api_auth::auth::Result<Vec<Permission>> {
    use rootcause::prelude::*;

    let user_roles = UserRole::find()
        .filter(user_role::Column::TenantId.eq(tenant_id))
        .filter(user_role::Column::UserId.eq(user_id))
        .all(db)
        .await
        .context_to()?;

    let role_ids: Vec<Uuid> = user_roles.iter().map(|ur| ur.role_id).collect();
    if role_ids.is_empty() {
        return Ok(Vec::new());
    }

    let role_permissions = RolePermission::find()
        .filter(role_permission::Column::RoleId.is_in(role_ids))
        .all(db)
        .await
        .context_to()?;

    let permission_ids: Vec<Uuid> = role_permissions
        .iter()
        .map(|rp| rp.permission_id)
        .collect();
    if permission_ids.is_empty() {
        return Ok(Vec::new());
    }

    let permissions = Permission::find()
        .filter(permission::Column::Id.is_in(permission_ids))
        .all(db)
        .await
        .context_to()?;

    Ok(permissions
        .into_iter()
        .filter_map(|p| uptrakit_web_api_auth::auth::permissions::Permission::from_db(&p))
        .collect())
}

fn classify_api_token_verify_error(error: &rootcause::Report<AuthError>) -> AuthFailure {
    match error.current_context() {
        AuthError::ApiTokenNotFound | AuthError::ApiTokenRevoked => AuthFailure::InvalidApiToken,
        _ => AuthFailure::InternalError,
    }
}
```

> **Note:** The `get_user_permissions` implementation above mirrors the one in `require_auth.rs`. Copy the full body from there
> (`require_auth.rs:385–end`) to ensure exactness — the snippet above shows the structure.

- [ ] **Step 2: Update `emit_api_token_auth_audit` in `require_auth.rs` to delegate**

Replace the function body in `require_auth.rs:64–83` so web-api callers still work without touching every call site:

```rust
pub(crate) fn emit_api_token_auth_audit(
    state: &AppState,
    request_id: Option<String>,
    outcome: uptrakit_audit_log::AuditOutcome,
    reason_code: &'static str,
) {
    uptrakit_controller_core::auth::api_token::emit_api_token_auth_audit(
        &state.audit_emitter,
        state.default_tenant_id,
        request_id,
        outcome,
        reason_code,
    );
}
```

- [ ] **Step 3: Update `authenticate_api_token` in `require_auth.rs` to delegate**

Replace the function body of `authenticate_api_token` (require_auth.rs:294–333) to delegate:

```rust
pub(crate) async fn authenticate_api_token(
    state: &AppState,
    token: &str,
) -> std::result::Result<(AuthenticatedUser, uuid::Uuid), AuthFailure> {
    uptrakit_controller_core::auth::api_token::authenticate_api_token(
        state.db(),
        state.default_tenant_id,
        token,
    )
    .await
}
```

Remove the now-redundant `get_user_permissions` definition from `require_auth.rs` (or leave it if used elsewhere — grep first:
`grep -n "get_user_permissions" crates/ui/web-api/src/`).

- [ ] **Step 4: Verify compile**

```bash
cargo check --all-features 2>&1 | grep -E "^error" | head -20
```

Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git commit --only crates/ui/controller-core/src/auth/api_token.rs crates/ui/web-api/src/middleware/require_auth.rs \
  -m "refactor(controller-core): extract authenticate_api_token with explicit params"
```

---

## Task 3: Remove Phase 1 re-export shims

**Files:**

- Modify: `crates/ui/web-api/src/settings.rs` (inline re-export → direct module)
- Modify: `crates/ui/web-api/src/workload_claims.rs` (inline re-export → direct module)

Phase 1 shims replaced module content with `pub use uptrakit_controller_core::…::*`. Now that types are established, confirm all `crate::settings::…`
and `crate::workload_claims::…` paths in web-api import correctly via the glob, then collapse.

- [ ] **Step 1: Verify all settings paths resolve through the shim**

```bash
cargo check --all-features 2>&1 | grep -E "^error.*settings" | head -10
```

Expected: no errors. If errors exist, make the missing items `pub` in controller-core before removing the shim.

- [ ] **Step 2: Verify workload_claims paths resolve**

```bash
cargo check --all-features 2>&1 | grep -E "^error.*workload" | head -10
```

Expected: no errors.

- [ ] **Step 3: Confirm shims are the final form**

The shims (`pub use uptrakit_controller_core::settings::*`) ARE the final form of these files — they are intentional re-exports, not placeholders.
No further change needed to these files. Mark this task complete with no commit needed.

---

## Task 4: Phase 2b pre-step — Move `AdminEvent` to `uptrakit-wire`

**Files:**

- Modify: `crates/shared/wire/src/lib.rs` (add AdminEvent and related types)
- Modify: `crates/ui/web-api-types/src/events.rs` (replace definitions with re-exports)
- Modify: `crates/ui/web-api/src/event_broadcaster.rs` (update import path)
- Modify: `crates/ui/web-api/src/update_orchestrator.rs` (update import path)

`EventBroadcaster` uses `uptrakit_web_api_types::events::AdminEvent`. For controller-core to own `EventBroadcaster`, `AdminEvent` must come from a crate
in controller-core's dep tree. `uptrakit-wire` is in controller-core deps and is the correct home for protocol event types.

- [ ] **Step 1: Find AdminEvent and all related types in web-api-types**

```bash
grep -n "pub.*enum AdminEvent\|pub.*struct.*Event\|AdminEvent" \
  crates/ui/web-api-types/src/events.rs | head -30
```

Note all `pub` types in `events.rs` that are referenced by `EventBroadcaster` or `update_orchestrator.rs`.

- [ ] **Step 2: Copy `AdminEvent` and related event types to `crates/shared/wire/src/`**

Create `crates/shared/wire/src/admin_events.rs` with the moved types. Add to `crates/shared/wire/src/lib.rs`:

```rust
pub mod admin_events;
pub use admin_events::AdminEvent;
```

Ensure the moved types carry `#[non_exhaustive]` and `serde(rename_all = "camelCase")` if they already had it.

- [ ] **Step 3: In `crates/ui/web-api-types/src/events.rs`, replace the definitions with re-exports**

```rust
pub use uptrakit_wire::admin_events::*;
pub use uptrakit_wire::AdminEvent;
```

Add `uptrakit-wire = { workspace = true }` to `crates/ui/web-api-types/Cargo.toml` if not already present.

- [ ] **Step 4: Update `event_broadcaster.rs` and `update_orchestrator.rs`**

Both currently import `uptrakit_web_api_types::events::AdminEvent`. After the re-export shim in web-api-types, these imports continue to work unchanged.
Confirm:

```bash
cargo check --all-features 2>&1 | grep -E "^error.*AdminEvent" | head -10
```

Expected: no errors.

- [ ] **Step 5: Update `BroadcastAdminEventPayload` doc comment in wire**

`BroadcastAdminEventPayload` (in `crates/shared/wire/src/payloads.rs`) has a doc comment
referencing `AdminEvent` via its old path `uptrakit_web_api_types::events::AdminEvent`.
After the move, update the doc comment to reference `crate::admin_events::AdminEvent`.

The `event_json: String` field stays intentionally opaque — it was designed this way for
agent-controller wire stability (opaque JSON tolerates AdminEvent schema changes without
requiring agent upgrades). Do NOT change `event_json: String` to `event: AdminEvent`.

```bash
grep -n "uptrakit_web_api_types.*AdminEvent\|AdminEvent" crates/shared/wire/src/payloads.rs
```

Update only the doc comment. Verify compile:

```bash
cargo check --all-features 2>&1 | grep -E "^error" | head -5
```

- [ ] **Step 6: Commit**

```bash
git commit --only crates/shared/wire/ crates/ui/web-api-types/src/events.rs \
  -m "refactor(wire): move AdminEvent from web-api-types to uptrakit-wire"
```

---

## Task 5: Phase 2b pre-step — Introduce `NatsPublisher` trait

**Files:**

- Modify: `crates/ui/controller-core/src/notification.rs` (define NatsPublisher trait)
- Modify: `crates/ui/web-api/src/nats_transport.rs` (implement NatsPublisher)
- Modify: `crates/ui/web-api/src/notification_service.rs` (replace NatsTransport field)
- Modify: `crates/ui/web-api/src/event_broadcaster.rs` (replace NatsTransport field)

`NotificationService` and `EventBroadcaster` both hold `Option<crate::nats_transport::NatsTransport>` behind `#[cfg(feature = "nats")]`. Replacing with
`Arc<dyn NatsPublisher>` removes the web-api-specific type from the data structure.

- [ ] **Step 1: Define `NatsPublisher` trait in `crates/ui/controller-core/src/notification.rs`**

```rust
use std::sync::Arc;

/// Abstraction over NATS publishing used by `NotificationService` and `EventBroadcaster`.
///
/// Allows controller-core types to publish NATS messages without depending on
/// `uptrakit-nats` or `uptrakit-web-api`.
#[async_trait::async_trait]
pub trait NatsPublisher: Send + Sync {
    async fn publish(&self, subject: String, payload: bytes::Bytes);
}
```

Add `bytes = { workspace = true }` to controller-core Cargo.toml if not present. Check workspace deps first: `grep "^bytes" Cargo.toml`.

- [ ] **Step 2: Implement `NatsPublisher` on `NatsTransport` in web-api**

In `crates/ui/web-api/src/nats_transport.rs`, add:

```rust
#[async_trait::async_trait]
impl uptrakit_controller_core::notification::NatsPublisher for NatsTransport {
    async fn publish(&self, subject: String, payload: bytes::Bytes) {
        // delegate to the existing publish method on NatsTransport
        self.publish_bytes(subject, payload).await;
    }
}
```

Adjust method name to match the actual `NatsTransport` API — search with:

```bash
grep -n "pub async fn publish" crates/ui/web-api/src/nats_transport.rs
```

- [ ] **Step 3: Update `NotificationService` to use `Arc<dyn NatsPublisher>`**

In `crates/ui/web-api/src/notification_service.rs`, change:

```rust
// Before:
#[cfg(feature = "nats")]
nats: Option<crate::nats_transport::NatsTransport>,

// After:
#[cfg(feature = "nats")]
nats: Option<Arc<dyn uptrakit_controller_core::notification::NatsPublisher>>,
```

Update `with_nats` setter:

```rust
// Before:
pub fn with_nats(mut self, nats: crate::nats_transport::NatsTransport) -> Self {
    self.nats = Some(nats);

// After:
pub fn with_nats(mut self, nats: Arc<dyn uptrakit_controller_core::notification::NatsPublisher>) -> Self {
    self.nats = Some(nats);
```

At the call site that passes a `NatsTransport`, wrap it:

```bash
grep -rn "with_nats(" crates/core/ crates/ui/ | grep -v "\.rs:#"
```

Wrap with `Arc::new(nats_transport)` where needed.

- [ ] **Step 4: Apply the same change to `EventBroadcaster`**

In `crates/ui/web-api/src/event_broadcaster.rs`:

```rust
// Before:
nats: Option<crate::nats_transport::NatsTransport>,

// After:
nats: Option<Arc<dyn uptrakit_controller_core::notification::NatsPublisher>>,
```

Update `with_nats` and any direct `self.nats.publish(…)` calls to use the trait method.

- [ ] **Step 5: Verify compile**

```bash
cargo check --all-features 2>&1 | grep -E "^error" | head -20
```

Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git commit --only crates/ui/controller-core/src/notification.rs \
    crates/ui/web-api/src/nats_transport.rs \
    crates/ui/web-api/src/notification_service.rs \
    crates/ui/web-api/src/event_broadcaster.rs \
  -m "refactor(controller-core): introduce NatsPublisher trait, decouple NatsTransport"
```

---

## Task 6: Move `NotificationState` to controller-core

**Files:**

- Modify: `crates/ui/controller-core/src/notification.rs` (add NotificationState + component moves)
- Modify: `crates/ui/web-api/src/app_state.rs` (replace NotificationState definition with re-export)
- Modify: `crates/ui/controller-core/Cargo.toml` (add notification-delivery dep)

After Tasks 4 and 5, `NotificationService`, `NotificationDispatcher`, and `EventBroadcaster` no longer have web-api-specific imports.
They can move to controller-core.

- [ ] **Step 1: Add `uptrakit-notification-delivery` to controller-core Cargo.toml**

`NotificationDispatcher` uses `uptrakit_notification_delivery::NotificationEvent`. Add to deps:

```toml
uptrakit-notification-delivery = { workspace = true }
```

- [ ] **Step 2: Verify `NotificationDispatcher` has no remaining crate:: imports**

```bash
grep -n "crate::" crates/ui/web-api/src/notifications/dispatcher.rs | head -10
```

For each `crate::X` found, identify the source crate and update to the absolute path. Common patterns:

- `crate::queries::…` → `uptrakit_web_api_queries::queries::…`
- `crate::notification_service::…` → will become `crate::notification::…` in controller-core

- [ ] **Step 3: Move `NotificationService` to controller-core/src/notification.rs**

Copy the content of `crates/ui/web-api/src/notification_service.rs` into the notification module. Update all `crate::` imports:

| Old                                                     | New                                                              |
| ------------------------------------------------------- | ---------------------------------------------------------------- |
| `crate::service_connections::ServiceConnectionRegistry` | `uptrakit_service_connections::ServiceConnectionRegistry`        |
| `crate::workload_claims::WorkloadClaimRegistry`         | `crate::workload_claims::WorkloadClaimRegistry` (same crate now) |
| `crate::queries::…`                                     | `uptrakit_web_api_queries::queries::…`                           |
| `crate::queries::update_tracking_states::*`             | `uptrakit_web_api_queries::queries::update_tracking_states::*`   |
| `crate::ServiceNotifier`                                | `uptrakit_web_api_queries::notifier::ServiceNotifier`            |

> **Visibility note:** If `NotificationService` uses a `mutation_context()` method that is currently `pub(crate)`, promote it to `pub` before the move
> — controller-core is a different crate and will fail to compile without it. Check with:
> `grep -n "pub(crate).*mutation_context\|fn mutation_context" crates/ui/web-api/src/notification_service.rs`.

- [ ] **Step 4: Move `EventBroadcaster` to controller-core/src/notification.rs**

Copy the content of `crates/ui/web-api/src/event_broadcaster.rs`. The `AdminEvent` import becomes:

```rust
use uptrakit_wire::AdminEvent;
```

- [ ] **Step 5: Move `NotificationDispatcher` to controller-core/src/notification.rs**

Copy the content of `crates/ui/web-api/src/notifications/dispatcher.rs`. Update `crate::` imports as identified in Step 2.

- [ ] **Step 6: Add `NotificationState` struct to controller-core/src/notification.rs**

```rust
#[derive(Clone)]
pub struct NotificationState {
    pub notification_service: NotificationService,
    pub notification_dispatcher: NotificationDispatcher,
    pub event_broadcaster: EventBroadcaster,
}
```

- [ ] **Step 7: Replace `NotificationState` definition in `crates/ui/web-api/src/app_state.rs`**

```rust
pub use uptrakit_controller_core::notification::NotificationState;
```

Remove the local `pub struct NotificationState { … }` block.

- [ ] **Step 8: Add re-export shims in web-api for moved types**

In `crates/ui/web-api/src/notification_service.rs`:

```rust
pub use uptrakit_controller_core::notification::NotificationService;
```

In `crates/ui/web-api/src/event_broadcaster.rs`:

```rust
pub use uptrakit_controller_core::notification::EventBroadcaster;
```

- [ ] **Step 9: Verify compile**

```bash
cargo check --all-features 2>&1 | grep -E "^error" | head -20
```

Expected: no errors. Fix any remaining `crate::` import mismatches.

- [ ] **Step 10: Commit**

```bash
git commit --only crates/ui/controller-core/ crates/ui/web-api/src/app_state.rs \
    crates/ui/web-api/src/notification_service.rs \
    crates/ui/web-api/src/event_broadcaster.rs \
  -m "refactor(controller-core): move NotificationState, NotificationService, EventBroadcaster"
```

---

## Task 7: Phase 2 CI quality gate

**Files:** None modified — verification only.

- [ ] **Step 1: Format**

```bash
cargo fmt --all && git diff --stat
```

Commit any fmt changes:

```bash
git commit --only crates/ui/controller-core/ crates/ui/web-api/ crates/shared/wire/ \
  -m "chore: apply fmt after Phase 2 moves"
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

---

## Self-Review

**Spec coverage (Phase 2):**

- [x] `connections.rs` re-export (Task 1)
- [x] `authenticate_api_token` extracted with explicit params (Task 2)
- [x] `emit_api_token_auth_audit` extracted with explicit params (Task 2)
- [x] Web-api call sites delegating to controller-core (Task 2 Steps 2–3)
- [x] Phase 1 shims verified as final form (Task 3)
- [x] `AdminEvent` moved to wire (Task 4) — prerequisite for EventBroadcaster move
- [x] `NatsTransport` dependency removed from `NotificationService`/`EventBroadcaster` (Task 5)
- [x] `NotificationState` moved (Task 6)

**Type consistency:** `AuthenticatedUser` returned by `authenticate_api_token` (controller-core) is the same type used by `require_auth.rs` middleware
(imported from controller-core). `AuthFailure` variants match between old and new location — the Phase 1 plan moved the type; this plan moves the function.

**Spec gap addressed:** The spec notes "Before Phase 2, verify that NotificationService, NotificationDispatcher, and EventBroadcaster carry no remaining
uptrakit-web-api-specific imports beyond WorkloadClaimRegistry." Tasks 4 and 5 resolve the two blockers found: `AdminEvent` (web-api-types) and
`NatsTransport` (web-api-specific). `uptrakit-notification-delivery` is added to controller-core Cargo.toml for `NotificationDispatcher`.
