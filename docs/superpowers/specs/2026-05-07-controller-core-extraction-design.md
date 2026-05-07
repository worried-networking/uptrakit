# `controller-core` Extraction and `AppState` Minimisation — Design

**Date:** 2026-05-07
**Predecessor:** `2026-05-01-extract-mcp-crate-design.md`

## Goal

Introduce `uptrakit-controller-core` — a pure business-logic crate with zero knowledge of
`web-api` or `mcp` — and use it to sever the `uptrakit-mcp` → `uptrakit-web-api` dependency
entirely. Paves the way for OAuth 2.1 MCP authorisation (future spec) by establishing clean,
durable crate boundaries before auth machinery is added.

This spec does **not** implement OAuth 2.1. It puts the structure in place so that auth work
lands in the right crate from day one.

---

## Non-negotiable constraints

1. `controller-core` has **zero** `uptrakit-web-api` or `uptrakit-mcp` imports — enforced by
   the absence of those path deps in its `Cargo.toml`.
2. `web-api` and `mcp` never import from each other.
3. After this refactor `uptrakit-mcp` has no `uptrakit-web-api` dep — verified by `cargo
check`.
4. MCP-specific types (`McpRequestContext`, `McpAuthError`, etc.) live exclusively in
   `uptrakit-mcp`.

---

## Dependency graph

```text
web-api-auth ──┐
web-api-queries─┤
shared-types  ──┤──▶  controller-core  ◀──  web-api   (HTTP adapter)
audit-log     ──┤                      ◀──  mcp        (MCP adapter)
plugin-infra  ──┘
```

Each arrow means "depends on". No arrow from `controller-core` points right.

---

## New crate: `crates/ui/controller-core`

Package name: `uptrakit-controller-core`. Add to `[workspace.members]`.

### Module structure

```text
src/
├── lib.rs              # crate-level doc: states the zero-web-api/mcp invariant
├── db.rs               # DbState newtype (moved from web-api)
├── settings/
│   └── mod.rs          # Settings, SettingsSnapshot, NetworkSettings, ZeroconfSnapshot
│                       #   (moved from web-api/src/settings.rs)
├── auth/
│   ├── mod.rs          # AuthState struct
│   ├── jwt.rs          # JwtManager
│   ├── denylist.rs     # TokenDenylist
│   ├── device_flow.rs  # DeviceFlowStore
│   ├── rate_limit.rs   # RateLimitStore
│   └── api_token.rs    # authenticate_api_token + emit_api_token_auth_audit —
│                       #   extracted from web-api/middleware/require_auth.rs;
│                       #   takes (db, default_tenant_id, token) — no &AppState
├── connections.rs      # Re-exports ServiceConnectionRegistry from uptrakit-service-connections
│                       #   (the type already lives in crates/ui/service-connections; no move)
├── workload_claims.rs  # WorkloadClaimRegistry (moved from web-api; no Axum dep —
│                       #   domain state needed by NotificationService)
├── notification.rs     # NotificationState: NotificationService,
│                       #   NotificationDispatcher, EventBroadcaster
│                       #   (moved from web-api; domain events, not HTTP)
├── update/
│   ├── mod.rs          # UpdateDispatcher trait + param/result/error types;
│                       #   UpdateOutputStream trait (abstracts UpdateOutputBroadcaster)
│   └── controller.rs   # ControllerUpdateDispatcher — single prod impl;
│                       #   absorbs trigger_update action,
│                       #   spawn_protection_and_dispatch, audit emission
└── audit.rs            # General audit emission helpers (extracted from
                        #   web-api routes — no longer route-specific)
```

### `Cargo.toml` dependencies

```toml
[dependencies]
uptrakit-web-api-auth                   = { workspace = true }
uptrakit-web-api-queries                = { workspace = true }
uptrakit-shared-types                   = { workspace = true }
uptrakit-shared-db                      = { workspace = true }
uptrakit-audit-log                      = { workspace = true }
uptrakit-plugin-infrastructure-registry = { workspace = true }
uptrakit-wire                           = { workspace = true }
uptrakit-service-connections = { workspace = true }
async-trait  = { workspace = true }
sea-orm      = { workspace = true }
serde_json   = { workspace = true }
time         = { workspace = true }
uuid         = { workspace = true }
tokio        = { workspace = true }
tokio-util   = { workspace = true }
parking_lot  = { workspace = true }
rootcause    = { workspace = true }
tracing      = { workspace = true }
```

No `uptrakit-web-api`, no `uptrakit-web-api-types`, no `uptrakit-mcp`, no `axum`.
`DispatchOutcome` is defined in `controller-core` itself; adapters map it to their
own output types (`TriggerUpdateStatus` in `web-api`, etc.).

> **Note on `NotificationState` internals:** `WorkloadClaimRegistry` moves to
> `controller-core` in Phase 1 (it has no Axum dependency — it is domain state). This
> unblocks Phase 2 moving `NotificationService` (which holds `Arc<WorkloadClaimRegistry>`).
> Before Phase 2, verify that `NotificationService`, `NotificationDispatcher`, and
> `EventBroadcaster` carry no remaining `uptrakit-web-api`-specific imports beyond
> `WorkloadClaimRegistry` (e.g. Axum SSE types). If any such dep is found, the type
> must be extracted first or the dep eliminated.

---

## `authenticate_api_token` signature change

Current (in `web-api`, `pub(crate)`):

```rust
async fn authenticate_api_token(state: &AppState, token: &str)
    -> Result<(AuthenticatedUser, Uuid), AuthFailure>
```

New (in `controller-core/src/auth/api_token.rs`, `pub`):

```rust
pub async fn authenticate_api_token(
    db: &DatabaseConnection,
    default_tenant_id: Uuid,
    token: &str,
) -> Result<(AuthenticatedUser, Uuid), AuthFailure>
```

Explicit deps replace `&AppState` threading. Web-api call sites pass `state.db()` and
`state.default_tenant_id`. MCP call sites pass `state.db.db()` and `state.default_tenant_id`.

---

## `UpdateDispatcher` trait and `ControllerUpdateDispatcher`

```rust
// controller-core/src/update/mod.rs

// async fn in trait uses #[async_trait] — consistent with every other async trait in this
// codebase (PluginOps, AgentCertSigner, CommandExecutor, etc.). Bare RPITIT is avoided
// because the returned future must be Send for Arc<dyn UpdateDispatcher> in Tokio spawns.
#[async_trait::async_trait]
pub trait UpdateDispatcher: Send + Sync {
    async fn dispatch(
        &self,
        params: UpdateDispatchParams,
    ) -> Result<UpdateDispatchResult, Report<UpdateDispatchError>>;
}

// Abstraction over UpdateOutputBroadcaster (web-api SSE type) so ControllerUpdateDispatcher
// can stream output without knowing about Axum/SSE. web-api provides the concrete impl.
#[async_trait::async_trait]
pub trait UpdateOutputStream: Send + Sync {
    async fn create_channel(&self, update_id: Uuid);
    async fn send_line(&self, update_id: Uuid, line_id: Uuid, text: String,
                       stream: OutputStreamType, ts: OffsetDateTime);
    // Typed outcome: compiler enforces valid values; adapter serialises at the boundary.
    async fn send_completed(&self, update_id: Uuid, outcome: DispatchOutcome, error: Option<String>);
}

#[non_exhaustive]
pub struct UpdateDispatchParams {
    pub tenant_id: Uuid,
    pub host_id: Uuid,
    pub software_item_id: Uuid,
    pub to_version: String,
    // ActorType stays in uptrakit-web-api-queries (used as query param there).
    // controller-core imports it via its uptrakit-web-api-queries dep.
    pub actor_type: ActorType,
    pub actor_id: String,
    pub release_info: Option<serde_json::Value>, // serialised release metadata; avoids pulling in
                                                 // a web-api-specific ReleaseInfo type
    pub interactive: bool,
}

impl UpdateDispatchParams {
    #[expect(clippy::too_many_arguments, reason = "constructor for non_exhaustive struct")]
    pub fn new(
        tenant_id: Uuid,
        host_id: Uuid,
        software_item_id: Uuid,
        to_version: String,
        actor_type: ActorType,
        actor_id: String,
        release_info: Option<serde_json::Value>,
        interactive: bool,
    ) -> Self {
        Self { tenant_id, host_id, software_item_id, to_version, actor_type, actor_id,
               release_info, interactive }
    }
}

// Domain name for dispatch outcome — avoids importing the HTTP DTO TriggerUpdateStatus
// from uptrakit-web-api-types. Each adapter maps this to its own output type.
#[non_exhaustive]
pub enum DispatchOutcome {
    Sent,    // agent was connected and message dispatched
    Queued,  // record created; agent offline — reconnect recovery will pick it up
    Failed,  // pre-dispatch validation or protection step failed
}

#[non_exhaustive]
pub struct UpdateDispatchResult {
    pub update_history_id: Uuid,
    pub outcome: DispatchOutcome,
}

#[non_exhaustive]
pub enum UpdateDispatchError {
    HostNotFound,
    SoftwareItemNotFound,
    UpdateAlreadyActive,
    NotConfigured,
    AgentUnavailable,
    Internal,
}
```

```rust
// controller-core/src/update/controller.rs

pub struct ControllerUpdateDispatcher {
    db: DatabaseConnection,
    service_connections: ServiceConnectionRegistry,
    notification: NotificationState,
    output_stream: Arc<dyn UpdateOutputStream>,  // web-api passes UpdateOutputBroadcaster impl
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
    ) -> Self { ... }
}

#[async_trait::async_trait]
impl UpdateDispatcher for ControllerUpdateDispatcher { ... }
```

`ControllerUpdateDispatcher::dispatch` inlines the logic currently spread across:

- `web-api/src/actions/software_items.rs` (`trigger_update`)
- `web-api/src/update_orchestrator.rs` (`spawn_protection_and_dispatch`)
- `web-api/src/routes/software_items/mod.rs` (`emit_software_update_audit`)

`spawn_protection_and_dispatch` is no longer a separate `pub(crate)` function.
`emit_software_update_audit` is replaced by the general helper in `controller-core/src/audit.rs`.

**Testability**: both `AppState` and `McpState` hold `Arc<dyn UpdateDispatcher>`. The single
production implementation is `ControllerUpdateDispatcher`. Tests inject a `MockUpdateDispatcher`
or `NoopUpdateDispatcher`, consistent with `PluginOps`, `AgentCertSigner`, and
`CommandExecutor` in this codebase.

At controller startup one `ControllerUpdateDispatcher` is constructed and its `Arc` is cloned
into both `AppState` and `McpState`.

---

## `AppState` after refactor (`web-api`)

```rust
pub struct AppState {
    // ── controller-core types ─────────────────────────────────────────
    pub(crate) db: DbState,
    pub auth: AuthState,
    pub settings: Settings,
    pub default_tenant_id: Uuid,
    pub controller_id: Uuid,
    pub service_connections: ServiceConnectionRegistry,
    pub notification: NotificationState,
    pub update_dispatcher: Arc<dyn UpdateDispatcher>,
    pub audit_log_filter: AuditFilter,
    pub audit_log_dispatcher: AuditLogDispatcher,
    pub audit_emitter: AuditEmitter,
    pub shutdown_token: CancellationToken,
    pub workload_claim_registry: Arc<WorkloadClaimRegistry>, // moved to controller-core

    // ── HTTP-specific sub-structs ─────────────────────────────────────
    pub cert: CertState,                   // unchanged
    pub broadcast: BroadcastState,         // unchanged (SSE streaming)
    #[cfg(feature = "oidc")]
    pub oidc: OidcState,                   // unchanged
    pub server: ServerState,               // NEW
    pub plugin: PluginState,               // NEW
    pub surfaces: SurfaceProxyDeps,        // unchanged
    pub cert_signer: Arc<dyn AgentCertSigner>,
    pub config_test_proxy: Arc<ConfigTestProxy>,
    pub embedded_service_notifier: Option<Arc<dyn EmbeddedServiceNotifier>>,
    pub credential_sources: ServiceCredentialSources,
    pub reject_dangerous_commands: bool,
    #[cfg(feature = "interactive")]
    pub interactive_sessions: InteractiveSessionRegistry,
}

// Two new sub-structs replace four loose top-level fields:

pub struct ServerState {
    pub pki_path: PathBuf,
    pub rustls_config: RustlsConfig,
}

pub struct PluginState {
    pub plugin_ops: Arc<dyn PluginOps>,
    pub global_providers: Arc<GlobalProviders>,
}
```

`mcp_compat.rs` is **deleted**. No replacement in `web-api`.

`AppStateBuilder` gains setters for `ServerState` and `PluginState`. Existing
`FromRef<Arc<AppState>>` impls are retained and expanded to cover new sub-structs. No changes
to existing route handler call sites.

The existing `PluginOpsState` and `GlobalProvidersState` Axum newtype extractors are **kept**.
Their `FromRef<Arc<AppState>>` impls are updated to go through the new sub-struct:

```rust
// Before:  PluginOpsState(state.plugin_ops.clone())
// After:   PluginOpsState(state.plugin.plugin_ops.clone())
```

Route handlers that extract `PluginOpsState` or `GlobalProvidersState` are unchanged.

---

## `McpState` and MCP-bounded types (`uptrakit-mcp`)

```rust
// mcp/src/state.rs
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
```

```rust
// mcp/src/settings.rs  — MCP's own projection of Settings
pub struct McpSettings {
    pub sans: Vec<String>,
    pub https_addr: SocketAddr,
}

impl From<&Settings> for McpSettings { ... }
```

MCP-bounded types (all new files in `uptrakit-mcp`, moved from deleted `mcp_compat.rs`):

```rust
// mcp/src/context.rs
#[non_exhaustive]
pub struct McpRequestContext { pub user_id, token_id, tenant_id, permissions }

#[non_exhaustive]
pub enum McpAuthError { MissingCredentials, JwtNotAccepted, Unauthorized, Forbidden, Internal }

#[non_exhaustive]
pub enum McpTriggerError { PermissionDenied, HostNotFound, SoftwareItemNotFound,
                           NotConfigured, AgentUnavailable, AlreadyInProgress, Internal }
```

```rust
// mcp/src/auth.rs
pub async fn validate_api_token_for_mcp(
    state: &McpState,
    token: Option<&str>,
) -> Result<McpRequestContext, McpAuthError> {
    // calls controller_core::auth::api_token::authenticate_api_token(
    //     state.db.db(), state.default_tenant_id, token)
    // then calls controller_core::auth::api_token::emit_api_token_auth_audit(
    //     &state.audit_emitter, state.default_tenant_id, &result)
    // Audit emission is preserved via McpState.audit_emitter — no AppState needed.
}
```

`controller-core/src/auth/api_token.rs` exports both functions:

```rust
pub async fn authenticate_api_token(
    db: &DatabaseConnection,
    default_tenant_id: Uuid,
    token: &str,
) -> Result<(AuthenticatedUser, Uuid), AuthFailure> { ... }

pub async fn emit_api_token_auth_audit(
    audit_emitter: &AuditEmitter,
    default_tenant_id: Uuid,
    result: &Result<(AuthenticatedUser, Uuid), AuthFailure>,
) { ... }
```

```rust
// mcp/src/tools/update.rs
pub async fn mcp_trigger_update(
    state: &McpState,
    ctx: &McpRequestContext,
    host_id: Uuid,
    software_item_id: Uuid,
    to_version: String,
) -> Result<(Uuid, DispatchOutcome), McpTriggerError> {
    // UpdateDispatchParams is #[non_exhaustive] — use ::new() (struct literal forbidden
    // outside the defining crate).
    let params = UpdateDispatchParams::new(
        ctx.tenant_id,
        host_id,
        software_item_id,
        to_version,
        ActorType::ApiToken,
        ctx.token_id.to_string(),
        None,
        false,
    );
    state.update_dispatcher.dispatch(params).await
        .map(|r| (r.update_history_id, r.outcome))
        .map_err(|e| McpTriggerError::from(e.current_context()))
}

// McpTriggerError is NOT a wire type — it is converted to an MCP tool error response
// internally and never transmitted over a protocol boundary. No Other(String) needed.
//
// From impl must use a wildcard arm with tracing::warn! so that new UpdateDispatchError
// variants don't silently become McpTriggerError::Internal without logging.
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
                tracing::warn!("unhandled UpdateDispatchError variant; mapping to Internal");
                McpTriggerError::Internal
            }
        }
    }
}
```

`build_mcp_router` takes `McpState` (not `Arc<AppState>`).

`uptrakit-mcp` `Cargo.toml` changes:

- **Remove**: `uptrakit-web-api`
- **Add**: `uptrakit-controller-core`
- **Retain**: `uptrakit-web-api-queries` (already used directly for history queries),
  `uptrakit-web-api-types`, `uptrakit-shared-db`

---

## Migration phasing

Each phase compiles and passes all CI quality gates independently.

| Phase | Scope                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| ----- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1     | Create `controller-core` crate scaffold. Move `DbState`, `Settings` + snapshots, `AuthState` + component types, `WorkloadClaimRegistry`. `web-api/src/settings.rs` becomes a re-export shim (`pub use uptrakit_controller_core::settings::*`) so all `crate::settings::…` paths inside `web-api` continue to compile unchanged during this phase. Add `uptrakit-controller-core` entry to `release-plz.toml` and to both controller `changelog_include` arrays (the crate must exist in workspace first). |
| 2a    | Move `ServiceConnectionRegistry`. Extract `authenticate_api_token` + `emit_api_token_auth_audit` with new explicit-param signatures. `web-api` adapts call sites. Remove re-export shims from Phase 1.                                                                                                                                                                                                                                                                                                    |
| 2b    | **Pre-step:** Audit `NotificationService` deps — specifically `NatsTransport` (`crate::nats_transport`) and any other `crate::`-qualified imports. Decision required: (a) move `NatsTransport` with it and add `uptrakit-nats` to `controller-core` `Cargo.toml`, OR (b) introduce `Arc<dyn NatsNotifier>` trait so `NotificationService` holds an abstraction rather than the concrete type. Choose before committing. Then move `NotificationState` to `controller-core`.                               |
| 3     | **Requires 2b complete** (needs `NotificationState` in `controller-core`). Introduce `UpdateDispatcher` + `UpdateOutputStream` traits + `ControllerUpdateDispatcher`. `ActorType` stays in `uptrakit-web-api-queries`; `controller-core` imports it from there (no cycle). Inline `spawn_protection_and_dispatch` and audit helpers. `web-api` wires `UpdateOutputBroadcaster` as `UpdateOutputStream` impl and switches to the new dispatcher.                                                           |
| 4     | Decouple `uptrakit-mcp` **and** minimise `AppState` in a single phase: add `McpState`, move MCP-bounded types from `mcp_compat.rs`, delete `mcp_compat.rs`, drop `uptrakit-web-api` dep from mcp `Cargo.toml`. Introduce `ServerState` and `PluginState` sub-structs, update `PluginOpsState`/`GlobalProvidersState` `FromRef` impls to go through `state.plugin.*`. No route handler changes.                                                                                                            |

---

## Quality gates

Every phase must pass:

```sh
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo deny check
```

After Phase 4, verify:

```sh
cargo tree -p uptrakit-mcp | grep uptrakit-web-api
# must produce no output
```

---

## Documentation deliverables

1. **ADR** — `docs/adr/NNNN-controller-core-boundary.md`
   Records: decision to introduce `controller-core` as the business-logic boundary,
   context (OAuth 2.1 MCP auth prep), consequences, alternatives considered
   (keep mcp→web-api dep; god-struct bundle). Required: decision is hard to reverse,
   surprising without context, result of a real tradeoff.

   ADR must also document the **`crates/ui/` placement decision**: `controller-core`
   is pure domain logic yet lives in `crates/ui/` alongside HTTP/CLI crates. Alternatives
   considered: `crates/core/controller-core` (more idiomatic for domain logic; avoids the
   directory signal problem) vs `crates/ui/` (co-located with its primary consumers). The
   ADR should record which was chosen and why, so contributors know not to use directory
   location as a signal about the crate's concerns.

2. **`controller-core/src/lib.rs`** — crate-level doc block states the zero-web-api /
   zero-mcp invariant explicitly so contributors see it on first `cargo doc` or IDE hover.

3. **`CONTEXT.md`** — no domain term changes required; this is an architectural boundary,
   not a new domain concept.

4. **Workspace `Cargo.toml`** — add `crates/ui/controller-core` to `[workspace.members]`
   (or verify glob coverage).

5. **`release-plz.toml`** — as part of Phase 1, once the crate exists in the workspace, add:

   ```toml
   [[package]]
   name = "uptrakit-controller-core"
   ```

   And add `"uptrakit-controller-core"` to the `changelog_include` arrays of both
   `uptrakit-controller` and `uptrakit-controller-standalone`. **Do not add this entry
   before the crate exists** — release-plz will fail on unknown workspace members.
   `uptrakit-mcp` has already been added (it is an existing crate).
