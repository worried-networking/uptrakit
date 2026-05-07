# controller-core Phase 3 — UpdateDispatcher Trait + ControllerUpdateDispatcher

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Define `UpdateDispatcher` and `UpdateOutputStream` traits in controller-core, implement `ControllerUpdateDispatcher` (absorbing `trigger_update`,
`spawn_protection_and_dispatch`, and audit helpers from web-api), wire `UpdateOutputBroadcaster` as the `UpdateOutputStream` impl, and switch both `AppState`
and all trigger call sites to `Arc<dyn UpdateDispatcher>`.

**Prerequisite:** Phase 2 plan complete and all CI gates passing.

**Architecture:** `ControllerUpdateDispatcher` is the single production `UpdateDispatcher` impl. It holds the deps it needs (`DatabaseConnection`,
`ServiceConnectionRegistry`, `NotificationState`, `Arc<dyn UpdateOutputStream>`, `Arc<dyn PluginOps>`, `AuditEmitter`). web-api wires
`UpdateOutputBroadcaster` as `dyn UpdateOutputStream` and constructs `ControllerUpdateDispatcher` at startup. Both `AppState` and (later) `McpState`
hold `Arc<dyn UpdateDispatcher>`. Tests inject `NoopUpdateDispatcher`.

**Types defined in controller-core:**

- `UpdateDispatchParams` (`#[non_exhaustive]`, constructor via `::new()`)
- `DispatchOutcome` (`#[non_exhaustive]` enum: `Sent`, `Queued`, `Failed`)
- `UpdateDispatchResult` (`#[non_exhaustive]`)
- `UpdateDispatchError` (`#[non_exhaustive]` enum: domain errors, NOT a wire type)
- `UpdateDispatcher` trait (`#[async_trait]`)
- `UpdateOutputStream` trait (`#[async_trait]`)
- `ControllerUpdateDispatcher` (prod impl)
- `NoopUpdateDispatcher` (test/noop impl)

**Standards binding:** `#[async_trait]` on both traits (consistent with `PluginOps`, `AgentCertSigner`, `CommandExecutor`). `#[non_exhaustive]` on all
structs and enums. `UpdateDispatchError` is NOT a wire type — no `Other(String)`. Wildcard arm with `tracing::warn!` in any external `match` on
`UpdateDispatchError` variants.

---

## Task 1: Define types + traits in `controller-core/src/update/mod.rs`

**Files:**

- Modify: `crates/ui/controller-core/src/update/mod.rs`

- [ ] **Step 1: Write `crates/ui/controller-core/src/update/mod.rs`**

```rust
pub mod controller;

use std::sync::Arc;

use async_trait::async_trait;
use time::OffsetDateTime;
use uuid::Uuid;

use uptrakit_shared_types::OutputStreamType;
use uptrakit_web_api_queries::queries::update_types::ActorType;

/// Groups actor identification for a dispatch request.
///
/// `#[non_exhaustive]`: future auth methods may add fields (e.g. `scope`).
/// External crates must use `ActorInfo::new(…)`.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct ActorInfo {
    pub actor_type: ActorType,
    pub actor_id: String,
}

impl ActorInfo {
    pub fn new(actor_type: ActorType, actor_id: impl Into<String>) -> Self {
        Self { actor_type, actor_id: actor_id.into() }
    }
}

/// Outcome of a dispatch attempt.
///
/// `#[non_exhaustive]`: future outcomes (e.g. `RateLimited`) may be added.
/// External match sites must include a wildcard arm with `tracing::warn!`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchOutcome {
    /// Agent was connected and the dispatch message was delivered.
    Sent,
    /// Record created; agent offline — reconnect recovery will pick it up.
    Queued,
    /// Pre-dispatch validation or protection step failed.
    Failed,
}

/// Result returned by a successful `UpdateDispatcher::dispatch` call.
#[non_exhaustive]
pub struct UpdateDispatchResult {
    pub update_history_id: Uuid,
    pub outcome: DispatchOutcome,
}

/// Domain errors from the update dispatch pipeline.
///
/// NOT a wire type — converted to adapter-specific errors at the HTTP/MCP boundary.
/// `#[non_exhaustive]`: new validation errors may be added. External match sites
/// must include a wildcard arm with `tracing::warn!`.
#[non_exhaustive]
#[derive(Debug)]
pub enum UpdateDispatchError {
    HostNotFound,
    SoftwareItemNotFound,
    UpdateAlreadyActive,
    NotConfigured,
    AgentUnavailable,
    Internal,
}

impl std::fmt::Display for UpdateDispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HostNotFound => write!(f, "host not found"),
            Self::SoftwareItemNotFound => write!(f, "software item not found"),
            Self::UpdateAlreadyActive => write!(f, "update already active for this host"),
            Self::NotConfigured => write!(f, "host not configured for updates"),
            Self::AgentUnavailable => write!(f, "no approved agent linked to host"),
            Self::Internal => write!(f, "internal error"),
            _ => write!(f, "unknown dispatch error"),
        }
    }
}

impl std::error::Error for UpdateDispatchError {}

/// Parameters for triggering a software update via `UpdateDispatcher::dispatch`.
///
/// `#[non_exhaustive]`: new fields (e.g. `force`, `dry_run`) may be added.
/// External crates must construct via `UpdateDispatchParams::new(…)`.
#[non_exhaustive]
pub struct UpdateDispatchParams {
    pub tenant_id: Uuid,
    pub host_id: Uuid,
    pub software_item_id: Uuid,
    pub to_version: String,
    // ActorInfo groups actor_type + actor_id — keeps new() at 7 params (clippy limit).
    pub actor: ActorInfo,
    /// Serialised release metadata; `None` if caller has no release context.
    pub release_info: Option<serde_json::Value>,
    pub interactive: bool,
}

impl UpdateDispatchParams {
    // 7 params — within clippy default limit; no suppression needed.
    pub fn new(
        tenant_id: Uuid,
        host_id: Uuid,
        software_item_id: Uuid,
        to_version: String,
        actor: ActorInfo,
        release_info: Option<serde_json::Value>,
        interactive: bool,
    ) -> Self {
        Self {
            tenant_id,
            host_id,
            software_item_id,
            to_version,
            actor,
            release_info,
            interactive,
        }
    }
}

/// Abstraction over the SSE update-output broadcaster.
///
/// `ControllerUpdateDispatcher` calls this to stream protection/dispatch output
/// without knowing about Axum or SSE. `web-api` provides the concrete impl via
/// `UpdateOutputBroadcaster`.
#[async_trait]
pub trait UpdateOutputStream: Send + Sync {
    async fn create_channel(&self, update_id: Uuid);
    async fn send_line(
        &self,
        update_id: Uuid,
        line_id: Uuid,
        text: String,
        stream: OutputStreamType,
        ts: OffsetDateTime,
    );
    async fn send_completed(
        &self,
        update_id: Uuid,
        outcome: DispatchOutcome,
        error: Option<String>,
    );
}

/// Dispatches software update requests through the protection/agent pipeline.
///
/// Implemented by `ControllerUpdateDispatcher` (production) and
/// `NoopUpdateDispatcher` (tests). Both `AppState` and `McpState` hold
/// `Arc<dyn UpdateDispatcher>`.
#[async_trait]
pub trait UpdateDispatcher: Send + Sync {
    async fn dispatch(
        &self,
        params: UpdateDispatchParams,
    ) -> Result<UpdateDispatchResult, rootcause::Report<UpdateDispatchError>>;
}

/// No-op dispatcher for tests that do not exercise update dispatch.
pub struct NoopUpdateDispatcher;

#[async_trait]
impl UpdateDispatcher for NoopUpdateDispatcher {
    async fn dispatch(
        &self,
        _params: UpdateDispatchParams,
    ) -> Result<UpdateDispatchResult, rootcause::Report<UpdateDispatchError>> {
        use rootcause::report;
        Err(report!(UpdateDispatchError::Internal))
    }
}
```

- [ ] **Step 2: Verify compile**

```bash
cargo check --all-features 2>&1 | grep -E "^error" | head -10
```

Expected: no errors (controller.rs is still empty, that is fine).

- [ ] **Step 3: Commit**

```bash
git commit --only crates/ui/controller-core/src/update/mod.rs \
  -m "feat(controller-core): define UpdateDispatcher, UpdateOutputStream traits + types"
```

---

## Task 2: Implement `UpdateOutputStream` on `UpdateOutputBroadcaster` in web-api

**Files:**

- Modify: `crates/ui/web-api/src/update_output_broadcaster.rs`

`UpdateOutputBroadcaster` already has `create_channel`, `send_line`, `send_completed` methods. Wrap them in the trait impl. The `send_completed` current
signature takes a `String` status — the new trait sends a typed `DispatchOutcome` and the impl converts it.

- [ ] **Step 1: Check current `UpdateOutputBroadcaster::send_completed` signature**

```bash
grep -n "pub async fn send_completed\|pub fn send_completed" \
  crates/ui/web-api/src/update_output_broadcaster.rs
```

Note the exact parameters. If it takes `status: String`, the impl converts `DispatchOutcome` to a string.

- [ ] **Step 2: Add trait impl to `crates/ui/web-api/src/update_output_broadcaster.rs`**

```rust
use uptrakit_controller_core::update::{DispatchOutcome, UpdateOutputStream};

#[async_trait::async_trait]
impl UpdateOutputStream for UpdateOutputBroadcaster {
    async fn create_channel(&self, update_id: uuid::Uuid) {
        self.create_channel(update_id).await;
    }

    async fn send_line(
        &self,
        update_id: uuid::Uuid,
        line_id: uuid::Uuid,
        text: String,
        stream: uptrakit_shared_types::OutputStreamType,
        ts: time::OffsetDateTime,
    ) {
        self.send_line(update_id, line_id, text, stream, ts).await;
    }

    async fn send_completed(
        &self,
        update_id: uuid::Uuid,
        outcome: DispatchOutcome,
        error: Option<String>,
    ) {
        let status = match outcome {
            DispatchOutcome::Sent | DispatchOutcome::Queued => "completed".to_string(),
            DispatchOutcome::Failed => "failed".to_string(),
            _ => {
                tracing::warn!("unhandled DispatchOutcome variant in send_completed");
                "failed".to_string()
            }
        };
        self.send_completed(update_id, status, error).await;
    }
}
```

Adjust the inner method calls if the actual broadcaster API differs — use the exact method names found in Step 1.

- [ ] **Step 3: Verify compile**

```bash
cargo check --all-features 2>&1 | grep -E "^error" | head -10
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git commit --only crates/ui/web-api/src/update_output_broadcaster.rs \
  -m "feat(web-api): implement UpdateOutputStream for UpdateOutputBroadcaster"
```

---

## Task 3: Implement `ControllerUpdateDispatcher`

**Files:**

- Modify: `crates/ui/controller-core/src/update/controller.rs`

Absorbs the logic from:

- `web-api/src/actions/software_items.rs` → `trigger_update` action
- `web-api/src/update_orchestrator.rs` → `spawn_protection_and_dispatch` / `run_protection_and_dispatch`
- `web-api/src/routes/software_items/mod.rs` → `emit_software_update_audit`

`spawn_protection_and_dispatch` is no longer a separate `pub(crate)` function — it is inlined into `dispatch`.

- [ ] **Step 1: Read the source functions in full before writing**

```bash
grep -n "pub.*fn trigger_update\|pub.*fn spawn_protection" \
  crates/ui/web-api/src/actions/software_items.rs \
  crates/ui/web-api/src/update_orchestrator.rs
```

Read `crates/ui/web-api/src/actions/software_items.rs` (the `trigger_update` function) and `crates/ui/web-api/src/update_orchestrator.rs` (the full
file, already read in prior analysis). Note all DB query functions called.

- [ ] **Step 2: Write `crates/ui/controller-core/src/update/controller.rs`**

```rust
use std::sync::Arc;

use async_trait::async_trait;
use sea_orm::DatabaseConnection;
use uuid::Uuid;

use uptrakit_audit_log::AuditEmitter;
use uptrakit_plugin_infrastructure_registry::PluginOps;
use uptrakit_service_connections::ServiceConnectionRegistry;
use uptrakit_web_api_queries::queries::update_dispatch::{
    DispatchUpdateParams, PreUpdateProtectionOutcome, dispatch_update_to_agent,
    fail_before_agent_dispatch, insert_protection_output_line, prepare_pre_update_protection,
    set_inprogress_for_orchestrator,
};
use uptrakit_web_api_queries::queries::update_triggers::TriggerUpdateParams;

use crate::notification::NotificationState;
use crate::update::{
    DispatchOutcome, UpdateDispatchError, UpdateDispatchParams, UpdateDispatchResult,
    UpdateDispatcher, UpdateOutputStream,
};

pub struct ControllerUpdateDispatcher {
    db: DatabaseConnection,
    service_connections: ServiceConnectionRegistry,
    notification: NotificationState,
    output_stream: Arc<dyn UpdateOutputStream>,
    plugin_ops: Arc<dyn PluginOps>,
    audit_emitter: AuditEmitter,
}

impl ControllerUpdateDispatcher {
    pub fn new(
        db: DatabaseConnection,
        service_connections: ServiceConnectionRegistry,
        notification: NotificationState,
        output_stream: Arc<dyn UpdateOutputStream>,
        plugin_ops: Arc<dyn PluginOps>,
        audit_emitter: AuditEmitter,
    ) -> Self {
        Self {
            db,
            service_connections,
            notification,
            output_stream,
            plugin_ops,
            audit_emitter,
        }
    }
}

#[async_trait]
impl UpdateDispatcher for ControllerUpdateDispatcher {
    async fn dispatch(
        &self,
        params: UpdateDispatchParams,
    ) -> Result<UpdateDispatchResult, rootcause::Report<UpdateDispatchError>> {
        use rootcause::{bail, report};
        use uptrakit_web_api_queries::queries::update_dispatch::TriggerUpdateError;

        let tenant_db = uptrakit_shared_db::TenantDb::new(
            self.db.clone(),
            params.tenant_id,
        );

        // 1. Trigger: validate + create update_history record.
        //    trigger_update_for_host takes &DatabaseConnection, not &TenantDb.
        let trigger_result = uptrakit_web_api_queries::queries::update_triggers::trigger_update_for_host(
            tenant_db.db(),
            TriggerUpdateParams {
                tenant_id: params.tenant_id,
                item_id: params.software_item_id,
                host_id: params.host_id,
                to_version: params.to_version.clone(),
                actor_type: params.actor.actor_type.as_str(),
                actor_id: &params.actor.actor_id,
                release_info: params.release_info.clone(),
                interactive: params.interactive,
            },
        )
        .await
        .map_err(|e| {
            emit_update_audit(
                &self.audit_emitter,
                &params,
                uptrakit_audit_log::AuditOutcome::Failed,
                &format!("trigger_failed: {:?}", e.current_context()),
            );
            map_trigger_error(e.current_context())
        })?;

        let update_history_id = trigger_result.update_history_id;

        // 2. Determine initial outcome before any orchestration.
        //    Pending = record created, agent not yet dispatched → Queued.
        //    The agent receives the dispatch only after run_protection_and_dispatch runs;
        //    Sent is set once the agent WS message is confirmed delivered.
        use uptrakit_shared_db::entity::update_history::UpdateStatus;
        let initial_outcome = match trigger_result.initial_status {
            UpdateStatus::Pending => DispatchOutcome::Queued,
            UpdateStatus::Failed => DispatchOutcome::Failed,
            _ => DispatchOutcome::Queued,
        };

        let audit_outcome = if matches!(initial_outcome, DispatchOutcome::Failed) {
            uptrakit_audit_log::AuditOutcome::Failed
        } else {
            uptrakit_audit_log::AuditOutcome::Success
        };

        emit_update_audit(
            &self.audit_emitter,
            &params,
            audit_outcome,
            &format!("dispatch_status: {:?}", initial_outcome),
        );

        // 3. Spawn protection + dispatch for Pending records.
        if let Some(work) = trigger_result.pending_protection_work {
            self.spawn_protection_and_dispatch(work);
        }

        Ok(UpdateDispatchResult {
            update_history_id,
            outcome: initial_outcome,
        })
    }
}

impl ControllerUpdateDispatcher {
    fn spawn_protection_and_dispatch(
        &self,
        work: uptrakit_web_api_queries::queries::update_triggers::PendingProtectionWork,
    ) {
        let db = self.db.clone();
        let service_connections = self.service_connections.clone();
        let notification = self.notification.clone();
        let output_stream = Arc::clone(&self.output_stream);
        let plugin_ops = Arc::clone(&self.plugin_ops);
        let audit_emitter = self.audit_emitter.clone();
        // Actor info (actor_type, actor_id) is NOT passed here — it was already written
        // to the update_history DB record by trigger_update_for_host before this spawn,
        // and emit_update_audit was already called in dispatch() above. No actor info
        // is needed inside the spawned protection/dispatch task.

        tokio::spawn(run_protection_and_dispatch(
            db,
            service_connections,
            notification,
            output_stream,
            plugin_ops,
            audit_emitter,
            work,
        ));
    }
}

#[tracing::instrument(skip_all, fields(update_id = %work.update_history_id))]
async fn run_protection_and_dispatch(
    db: DatabaseConnection,
    service_connections: ServiceConnectionRegistry,
    notification: NotificationState,
    output_stream: Arc<dyn UpdateOutputStream>,
    plugin_ops: Arc<dyn PluginOps>,
    audit_emitter: AuditEmitter,
    work: uptrakit_web_api_queries::queries::update_triggers::PendingProtectionWork,
) {
    // 7 args — within clippy default limit; no suppression needed.
    //
    // Copy the full implementation from web-api/src/update_orchestrator.rs:29–245,
    // applying the substitutions below. Structure (steps 1-10) is identical.
    //
    // Key substitutions:
    //   state.db()                              → &db
    //   state.service_connections               → service_connections
    //   state.notification.notification_service → notification.notification_service
    //   state.notification.event_broadcaster    → notification.event_broadcaster
    //   state.broadcast.update_output_broadcaster → output_stream (via trait methods)
    //   state.controller_update_protection()    → plugin_ops.controller_update_protection()
    //   state.controller_update_hook()          → plugin_ops.controller_update_hook()
    //   state.audit_emitter                     → audit_emitter
    //
    // DO NOT commit with unimplemented bodies — fill in completely before committing.
    // (cargo clippy deny(todo) will fail CI otherwise.)
}

fn emit_update_audit(
    audit_emitter: &AuditEmitter,
    params: &UpdateDispatchParams,
    outcome: uptrakit_audit_log::AuditOutcome,
    detail: &str,
) {
    // Copy the body of emit_software_update_audit from
    // web-api/src/routes/software_items/mod.rs, replacing state.audit_emitter
    // with audit_emitter param and state.default_tenant_id with params.tenant_id.
    //
    // Actor type/id come from params.actor.actor_type / params.actor.actor_id.
    //
    // DO NOT commit with unimplemented body — fill in completely before committing.
    let _ = (audit_emitter, params, outcome, detail);
}

fn map_trigger_error(
    e: &uptrakit_web_api_queries::queries::update_dispatch::TriggerUpdateError,
) -> rootcause::Report<UpdateDispatchError> {
    use rootcause::report;
    use uptrakit_web_api_queries::queries::update_dispatch::TriggerUpdateError;
    let domain_err = match e {
        TriggerUpdateError::HostNotFound => UpdateDispatchError::HostNotFound,
        TriggerUpdateError::SoftwareItemNotFound => UpdateDispatchError::SoftwareItemNotFound,
        TriggerUpdateError::UpdateAlreadyActive => UpdateDispatchError::UpdateAlreadyActive,
        TriggerUpdateError::HostNotAssigned
        | TriggerUpdateError::NoExecuteUpdatePlugin
        | TriggerUpdateError::PluginConfigNotFound
        | TriggerUpdateError::UnknownPluginType(_) => UpdateDispatchError::NotConfigured,
        TriggerUpdateError::NoAgent | TriggerUpdateError::AgentNotApproved => {
            UpdateDispatchError::AgentUnavailable
        }
        _ => {
            tracing::warn!("unhandled TriggerUpdateError variant; mapping to Internal");
            UpdateDispatchError::Internal
        }
    };
    report!(domain_err)
}
```

> **Implementation note:** The `run_protection_and_dispatch` and `emit_update_audit` bodies are left as empty stubs above with `let _ = (...)` suppression
> (NOT `todo!()` — `cargo clippy deny(todo)` fails CI). Before committing, fill in both bodies completely by copying from `update_orchestrator.rs` and
> `routes/software_items/mod.rs` as directed in the comments.

- [ ] **Step 3: Fill in `run_protection_and_dispatch` body**

Copy the full implementation from `crates/ui/web-api/src/update_orchestrator.rs:29–245` into `run_protection_and_dispatch`. Apply all substitutions
listed in the comment. The `let _ = (...)` suppressor lines must be removed.

- [ ] **Step 4: Fill in `emit_update_audit` body**

Find `emit_software_update_audit` in `crates/ui/web-api/src/routes/software_items/mod.rs`:

```bash
grep -n "fn emit_software_update_audit" crates/ui/web-api/src/routes/software_items/mod.rs
```

Copy its body, replacing `state.audit_emitter` with `audit_emitter` param and `state.default_tenant_id` with `params.tenant_id`. Remove the
`let _ = (...)` suppressor line. Actor info comes from `params.actor.actor_type` / `params.actor.actor_id`.

- [ ] **Step 5: Verify compile**

```bash
cargo check --all-features 2>&1 | grep -E "^error" | head -20
```

Expected: no errors.

- [ ] **Step 6: Commit** (single commit after both bodies are filled in)

```bash
git commit --only crates/ui/controller-core/src/update/controller.rs \
  -m "feat(controller-core): implement ControllerUpdateDispatcher"
```

---

## Task 4: Wire `ControllerUpdateDispatcher` into `AppState`

**Files:**

- Modify: `crates/ui/web-api/src/app_state.rs`
- Modify: `crates/ui/web-api/src/update_orchestrator.rs` (delete or empty file)

- [ ] **Step 1: Add `update_dispatcher` field to `AppState` struct**

In `crates/ui/web-api/src/app_state.rs`, add to the `AppState` struct:

```rust
pub update_dispatcher: Arc<dyn uptrakit_controller_core::update::UpdateDispatcher>,
```

- [ ] **Step 2: Add `update_dispatcher` to `AppStateBuilder`**

Add field to builder struct:

```rust
update_dispatcher: Option<Arc<dyn uptrakit_controller_core::update::UpdateDispatcher>>,
```

Add setter method:

```rust
pub fn update_dispatcher(
    mut self,
    v: Arc<dyn uptrakit_controller_core::update::UpdateDispatcher>,
) -> Self {
    self.update_dispatcher = Some(v);
    self
}
```

In `AppStateBuilder::build()`, construct a `ControllerUpdateDispatcher` as the default when none is provided:

```rust
let update_dispatcher = self.update_dispatcher.unwrap_or_else(|| {
    Arc::new(
        uptrakit_controller_core::update::controller::ControllerUpdateDispatcher::new(
            db.clone(),
            service_connections.clone(),
            notification.clone(),
            Arc::new(broadcast.update_output_broadcaster.clone()),
            plugin_ops.clone(),
            audit_emitter.clone(),
        )
    )
});
```

Assign to the struct field: `update_dispatcher`.

- [ ] **Step 3: Update trigger call sites in web-api routes to use `state.update_dispatcher`**

Find all places in `web-api/src/routes/software_items/` and `web-api/src/actions/software_items.rs` that call `trigger_update` + `spawn_protection_and_dispatch`:

```bash
grep -rn "spawn_protection_and_dispatch\|trigger_update\|mcp_trigger_update" \
  crates/ui/web-api/src/routes/ crates/ui/web-api/src/actions/ | grep -v "\.md"
```

For each HTTP route handler that currently calls `trigger_update` then `spawn_protection_and_dispatch`, replace with:

```rust
let result = state.update_dispatcher.dispatch(
    uptrakit_controller_core::update::UpdateDispatchParams::new(
        tenant_id,
        host_id,
        software_item_id,
        to_version,
        uptrakit_controller_core::update::ActorInfo::new(actor_type, actor_id),
        release_info,
        interactive,
    )
).await?;
// result.update_history_id, result.outcome available
```

Map `result.outcome` (a `DispatchOutcome`) to the HTTP response's `TriggerUpdateStatus`:

```rust
let status = match result.outcome {
    DispatchOutcome::Sent => TriggerUpdateStatus::Sent,
    DispatchOutcome::Queued => TriggerUpdateStatus::Queued,
    DispatchOutcome::Failed => TriggerUpdateStatus::Failed,
    _ => {
        tracing::warn!("unhandled DispatchOutcome in HTTP response mapping");
        TriggerUpdateStatus::Failed
    }
};
```

- [ ] **Step 4: Update `mcp_compat.rs` to use `state.update_dispatcher`**

In `crates/ui/web-api/src/mcp_compat.rs`, replace the `mcp_trigger_update` body. It currently calls `trigger_update` + `spawn_protection_and_dispatch`
directly. Change to:

```rust
let result = state.update_dispatcher.dispatch(params).await
    .map(|r| (r.update_history_id, r.outcome))
    .map_err(|e| {
        let mcp_err = match e.current_context() {
            UpdateDispatchError::HostNotFound => McpTriggerError::HostNotFound,
            UpdateDispatchError::SoftwareItemNotFound => McpTriggerError::SoftwareItemNotFound,
            UpdateDispatchError::UpdateAlreadyActive => McpTriggerError::AlreadyInProgress,
            UpdateDispatchError::NotConfigured => McpTriggerError::NotConfigured,
            UpdateDispatchError::AgentUnavailable => McpTriggerError::AgentUnavailable,
            UpdateDispatchError::Internal => McpTriggerError::Internal,
            _ => {
                tracing::warn!("unhandled UpdateDispatchError; mapping to Internal");
                McpTriggerError::Internal
            }
        };
        rootcause::report!(mcp_err)
    })?;
```

- [ ] **Step 5: Delete `crates/ui/web-api/src/update_orchestrator.rs`**

After all callers are migrated, `update_orchestrator.rs` is no longer needed:

```bash
grep -rn "update_orchestrator\|spawn_protection_and_dispatch" \
  crates/ui/web-api/src/ | grep -v "update_orchestrator.rs"
```

Expected: no output. If any remain, migrate them. Then delete the file and remove `pub(crate) mod update_orchestrator;` from `lib.rs`.

- [ ] **Step 6: Verify compile**

```bash
cargo check --all-features 2>&1 | grep -E "^error" | head -20
```

Expected: no errors.

- [ ] **Step 7: Commit**

```bash
git rm crates/ui/web-api/src/update_orchestrator.rs
git commit --only crates/ui/web-api/src/app_state.rs crates/ui/web-api/src/routes/ \
    crates/ui/web-api/src/actions/software_items.rs \
    crates/ui/web-api/src/mcp_compat.rs \
    crates/ui/web-api/src/update_orchestrator.rs \
  -m "feat(web-api): wire ControllerUpdateDispatcher into AppState, remove update_orchestrator"
```

---

## Task 5: Phase 3 CI quality gate

**Files:** None modified — verification only.

- [ ] **Step 1: Format and clean**

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

- [ ] **Step 3: Commit any residual fixes**

```bash
git commit --only crates/ui/controller-core/ crates/ui/web-api/ \
  -m "chore: Phase 3 fmt/clippy cleanup"
```

---

## Self-Review

**Spec coverage:**

- [x] `UpdateDispatcher` trait + `#[async_trait]` (Task 1)
- [x] `UpdateOutputStream` trait + `#[async_trait]` (Task 1)
- [x] `UpdateDispatchParams` `#[non_exhaustive]` + `::new()` constructor (Task 1)
- [x] `DispatchOutcome` `#[non_exhaustive]` enum (Task 1)
- [x] `UpdateDispatchResult` + `UpdateDispatchError` `#[non_exhaustive]` (Task 1)
- [x] `NoopUpdateDispatcher` for tests (Task 1)
- [x] `UpdateOutputBroadcaster` implements `UpdateOutputStream` (Task 2)
- [x] `ControllerUpdateDispatcher::new()` with explicit deps (Task 3)
- [x] `spawn_protection_and_dispatch` inlined — no longer a separate `pub(crate)` fn (Task 3)
- [x] `emit_software_update_audit` absorbed into `emit_update_audit` helper (Task 3)
- [x] `AppState.update_dispatcher: Arc<dyn UpdateDispatcher>` (Task 4)
- [x] HTTP route handlers use dispatcher (Task 4)
- [x] `mcp_compat.rs::mcp_trigger_update` uses dispatcher (Task 4)
- [x] `update_orchestrator.rs` deleted (Task 4)

**Type consistency:** `DispatchOutcome` variants used in `send_completed` (Task 2) match those defined in Task 1. `map_trigger_error` wildcard arm uses
`tracing::warn!` — matches codebase convention for `#[non_exhaustive]` enums.

**Spec reference — ActorType location:** The spec states "ActorType stays in uptrakit-web-api-queries; controller-core imports it from there." This is
implemented: `UpdateDispatchParams.actor_type: ActorType` imports `uptrakit_web_api_queries::queries::update_types::ActorType`. No cycle
(controller-core → web-api-queries is already allowed).

**Module placement note:** The spec places `emit_software_update_audit` logic in `controller-core/src/audit.rs` (not in `controller.rs`). Task 3 puts
it inline in `controller.rs` for simplicity. Implementers may optionally refactor it into `audit.rs` and import from there — the external behaviour
is identical. The stub `audit.rs` file created in Phase 1 Task 1 is the intended home per the spec module layout.
