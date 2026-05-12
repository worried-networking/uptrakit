# Graceful Reload — Plan 3: Migration + API + Governance + Reexec

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the user-visible API surface for graceful reload — `If-Match` optimistic locking on settings
mutations, two new permissions, two new endpoints, four new `AuditEvent` variants, `system_alerts` wiring,
the SeaORM migration that purges File-only `SettingKey` rows, the CLI shrink to five flags, and the reexec
helper (LISTEN_FDS + sd-notify + args allowlist + `exec()` swap).

**Architecture:** Three loosely-coupled blocks land together because they share the API-surface PR cluster:
(a) the SettingKey purge migration + CLI shrink remove the old surface; (b) the `IfMatch<T>` extractor + new
endpoints + audit events add the new governance surface; (c) the reexec helper handles the irreversibly-bound
key change branch. All three sit on top of the foundation (Plan 1) and the subsystem Reloadables (Plan 2).

**Tech Stack:** SeaORM 1 migrations, axum 0.8 (`FromRequestParts`), `listenfd` 1, `sd-notify` 0.5, `nix` (already
in workspace), `serde_json`, `uptrakit-audit-log`, `uptrakit-config-reload`, `rootcause::Report`.

**Spec:** `docs/superpowers/specs/2026-05-12-graceful-reload-design.md` (sections §6.3, §6.4, §11, §14, §15, §20).

**Status:** Draft → Ready for review.

---

## Prerequisites

- Plan 1 merged (coordinator + reload framework alive).
- Plan 2 merged (every subsystem has a `Reloadable` impl; coordinator drives them).

## Snapshot binding

- "BEGIN IMMEDIATE for read-then-write transactions" — `settings_version` bump path verified.
- "All HTTP request types implement `Validate` trait" — new endpoint request types satisfy this.
- "Wire-safe enums must have Other(String) catch-all" — `ReloadPhase` (already in foundation), new
  `ConfigReloadOutcome` if added.
- "`#[non_exhaustive]` on all extensible public enums and structs" — every new public type.
- "Use #[expect(lint, reason = …)] instead of #[allow(…)]" — when suppressing lints around `exec()` safety.
- "forbid `unwrap()` in production".
- "Workspace lints: `clippy::large_futures = deny`".
- Conventional Commits: `feat(db)`, `feat(web-api)`, `feat(web-api-types)`, `refactor(controller)`,
  `feat(controller-runtime)`, `test(...)`, `feat(audit-log)`.

---

## File Structure

**New files:**

- `crates/shared/db/src/migration/m20260512_000001_drop_file_keys.rs`
- `crates/ui/web-api/src/extractors/if_match.rs`
- `crates/ui/web-api/src/extractors/etag_source.rs`
- `crates/ui/web-api/src/routes/instance_config_state.rs` — `GET /api/v1/instance/config-state` +
  `POST /api/v1/instance/config-reload/clear-degraded`
- `crates/shared/audit-log/src/events/config_reload.rs` — the four new `AuditEvent` variants
- `crates/core/controller-runtime/src/reexec/mod.rs`
- `crates/core/controller-runtime/src/reexec/listenfd.rs`
- `crates/core/controller-runtime/src/reexec/sd_notify.rs`
- `crates/core/controller-runtime/src/reexec/triage.rs` — irreversibly-bound key detection
- `tests/reexec_integration.rs` — Docker `--ignored` test path

**Modified files:**

- `Cargo.toml` (workspace) — add `listenfd = "1"` and `sd-notify = "0.5"` to `[workspace.dependencies]`
- `crates/shared/db/src/migration/mod.rs` — register the new migration in order
- `crates/ui/web-api-auth/src/setting_key.rs` — remove File-only variants
- `crates/shared/types/src/permissions.rs` — add `ViewInstanceConfigState`, `ManageInstanceConfigState`; update
  variant-count assertion
- `crates/shared/openapi-client/src/paths.rs` — add the two new endpoint constants
- `crates/shared/openapi-client/src/settings.rs` — strip the dropped SettingKey aliases
- `crates/shared/audit-log/src/events/mod.rs` — re-export new variants; widen `AuditEvent` enum
  (it's `#[non_exhaustive]` already, so additive)
- `crates/ui/web-api/src/routes/settings*.rs` + `crates/ui/web-api/src/routes/plugin_configs.rs` — add
  `IfMatch<SettingsVersion>` extractor on every mutation handler
- `crates/core/controller/src/main.rs` + `crates/core/controller-standalone/src/main.rs` — final CLI shrink
- `docs/development/coding-standards.md` — File-vs-DB section table (move to Plan 4 docs)

---

## Task 1: SeaORM migration `m20260512_000001_drop_file_keys`

**Files:**

- Create: `crates/shared/db/src/migration/m20260512_000001_drop_file_keys.rs`
- Modify: `crates/shared/db/src/migration/mod.rs`

The migration:

1. Reads every row from `global_settings` that matches one of the File-only `SettingKey` strings.
2. Emits a `tracing::warn!` for each row.
3. Deletes those rows.
4. Per-tenant `audit_log.*` overrides remain in the `settings` table — untouched.

- [ ] **Step 1: Implement migration**

```rust
use sea_orm_migration::prelude::*;
use sea_orm::{
    ColumnTrait, EntityTrait, QueryFilter, TransactionOptions, TransactionTrait,
};
use sea_orm::sea_query::SqliteTransactionMode;

#[derive(DeriveMigrationName)]
pub struct Migration;

const FILE_ONLY_KEYS: &[&str] = &[
    "network.https_addr",
    "network.pki_addr",
    "network.trusted_proxies",
    "network.real_ip_header",
    "network.sans",
    "network.forwarded_client_cert_info_header",
    "network.forwarded_client_cert_pem_header",
    "nats.url",
    "zeroconf.enabled",
    "zeroconf.url",
    "zeroconf.pki_addr",
    "audit_log.filter",
    "audit_log.retention_days",
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Read-then-write inside one transaction. Per the snapshot rule
        // ("BEGIN IMMEDIATE for read-then-write"), we use SqliteTransactionMode::Immediate so
        // that a concurrent writer between SELECT and DELETE does not produce
        // SQLITE_BUSY_SNAPSHOT (code 5). No-op on Postgres.
        let txn = manager
            .get_connection()
            .begin_with_options(TransactionOptions {
                sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
                ..Default::default()
            })
            .await?;

        let rows = uptrakit_shared_db::entity::global_setting::Entity::find()
            .filter(
                uptrakit_shared_db::entity::global_setting::Column::Key
                    .is_in(FILE_ONLY_KEYS.iter().copied()),
            )
            .all(&txn)
            .await?;
        for row in &rows {
            tracing::warn!(
                key = %row.key,
                "dropping global_settings row; key moved to TOML (spec §6.3, §20)"
            );
        }
        uptrakit_shared_db::entity::global_setting::Entity::delete_many()
            .filter(
                uptrakit_shared_db::entity::global_setting::Column::Key
                    .is_in(FILE_ONLY_KEYS.iter().copied()),
            )
            .exec(&txn)
            .await?;
        txn.commit().await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Irreversible — File-only keys do not round-trip back to DB after the upgrade.
        // Operators rolling back must re-populate via the prior TOML/env mechanism (out of scope).
        Ok(())
    }
}
```

Register in `mod.rs`:

```rust
mod m20260512_000001_drop_file_keys;
// ... add to MigratorTrait::migrations() Vec, after the existing latest migration
```

- [ ] **Step 2: Test against sqlite in-memory** — seed rows, run migration, assert rows gone, assert WARN
      emitted via `tracing_test::traced_test`
- [ ] **Step 3:** `cargo test -p uptrakit-shared-db --test migrations`
- [ ] **Step 4:** Commit — `feat(db): m20260512_000001_drop_file_keys migration`

---

## Task 2: SettingKey enum cleanup

**Files:**

- Modify: `crates/ui/web-api-auth/src/setting_key.rs`

Remove these variants entirely: `TrustedProxies`, `RealIpHeader`, `Sans`, `HttpsAddr`,
`ForwardedClientCertInfoHeader`, `ForwardedClientCertPemHeader`, `PkiAddr`, `NatsUrl`, `ZeroconfEnabled`,
`ZeroconfUrl`, `ZeroconfPkiAddr`. Keep `AuditLogFilter` / `AuditLogRetentionDays` because per-tenant overrides
remain DB-rooted; per-spec §6.3 only the **global** rows are purged by the migration.

- [ ] **Step 1: Delete the variants**
- [ ] **Step 2: Update `as_str` + `from_db_key` matches**
- [ ] **Step 3:** Fix every call site that still references the dropped variants — they should already be unused
      after Plan 2, but search defensively: `rg 'SettingKey::HttpsAddr' src/`
- [ ] **Step 4:** Update the `strum::EnumIter` count assertion in tests
- [ ] **Step 5:** Run full quality gate suite
- [ ] **Step 6:** Commit — `refactor(web-api-auth): drop File-only SettingKey variants`

---

## Task 3: CLI shrink

**Files:**

- Modify: `crates/core/controller/src/main.rs` + `cli.rs` if present
- Modify: `crates/core/controller-standalone/src/main.rs`

Final surviving flags (per spec §6.4):

```rust
#[derive(Parser)]
struct Cli {
    #[arg(long, env = "UPTRAKIT_CONFIG", default_value = "/etc/uptrakit/controller.toml")]
    config: PathBuf,

    #[arg(long, env = "UPTRAKIT_MASTER_KEY_FROM")]
    master_key_from: Option<String>,

    #[arg(long)]
    check_config: bool,

    #[arg(long)]
    migrate_and_exit: bool,
}
```

Delete every other flag. Hard break per spec §3.1 + §20.

- [ ] **Step 1: Delete dropped flags from clap structs**
- [ ] **Step 2: Update every reference to dropped flags** (search for old flag names)
- [ ] **Step 3:** `cargo run --bin uptrakit-controller -- --help` — manual smoke
- [ ] **Step 4:** Commit — `feat(controller)!: CLI shrink to five flags (breaking)`

---

## Task 4: `Permission::ViewInstanceConfigState` + `ManageInstanceConfigState` + extractors

**Files:**

- Modify: `crates/shared/types/src/permissions.rs`
- Modify: `crates/ui/web-api/src/middleware/permission.rs`

Add to the existing `#[non_exhaustive]` `Permission` enum:

```rust
ViewInstanceConfigState,
ManageInstanceConfigState,
```

Update the hardcoded variant-count assertion (matches snapshot test pattern in the repo). Update
`as_str` / `from_str` / `Display`.

In `crates/ui/web-api/src/middleware/permission.rs`, the existing `permission_extractor!` macro (the established
in-repo idiom — see `CanViewSettings`, `CanManageGlobalSettings`, etc. at line 91) generates one named extractor
struct per permission. Add two entries to the existing block:

```rust
permission_extractor! {
    // ... existing entries ...

    /// Extractor that requires [`Permission::ViewInstanceConfigState`].
    CanViewInstanceConfigState => Permission::ViewInstanceConfigState,
    /// Extractor that requires [`Permission::ManageInstanceConfigState`].
    CanManageInstanceConfigState => Permission::ManageInstanceConfigState,
}
```

Do **not** invent a generic `RequirePermission<const P: …>` extractor — `Permission` carries `Other(String)` and
therefore cannot be a const-generic discriminant.

- [ ] **Step 1: Add variants** to `Permission`
- [ ] **Step 2: Update `as_str` / `from_str` / `Display`**
- [ ] **Step 3: Add the two extractors** to the existing `permission_extractor!` block
- [ ] **Step 4: Update test asserting `Permission::iter().count() == N` to `N + 2`**
- [ ] **Step 5:** `cargo test -p uptrakit-shared-types -p uptrakit-web-api`
- [ ] **Step 6:** Commit — `feat(types): add ViewInstanceConfigState + ManageInstanceConfigState permissions`

---

## Task 5: `EtagSource` trait + `IfMatch<T>` extractor

**Files:**

- Create: `crates/ui/web-api/src/extractors/etag_source.rs`
- Create: `crates/ui/web-api/src/extractors/if_match.rs`
- Modify: `crates/ui/web-api/src/extractors/mod.rs`

- [ ] **Step 1: Implement `EtagSource`**

```rust
use async_trait::async_trait;
use axum::http::request::Parts;
use rootcause::Report;

use crate::app_state::AppState;

#[async_trait]
pub trait EtagSource: Sized + Send + Sync + 'static {
    async fn current_etag(parts: &mut Parts, state: &AppState) -> Result<String, Report>;
}
```

- [ ] **Step 2: Implement `IfMatch<T>` extractor**

```rust
use axum::extract::FromRequestParts;
use axum::http::{header::IF_MATCH, request::Parts, StatusCode};
use rootcause::Report;

use crate::app_state::AppState;
use crate::extractors::etag_source::EtagSource;

pub struct IfMatch<T: EtagSource> {
    pub client_etag: String,
    _marker: std::marker::PhantomData<T>,
}

#[async_trait::async_trait]
impl<T: EtagSource> FromRequestParts<AppState> for IfMatch<T> {
    type Rejection = (StatusCode, String);

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let header = parts.headers.get(IF_MATCH).ok_or((
            StatusCode::PRECONDITION_REQUIRED,
            "If-Match header is required".to_string(),
        ))?;
        let client = header
            .to_str()
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("If-Match parse error: {e}")))?
            .to_string();
        let current = T::current_etag(parts, state).await.map_err(|e| {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("etag lookup failed: {e}"))
        })?;
        // Weak ETag semantic comparison: strip optional `W/` prefix.
        let strip = |s: &str| -> String { s.trim_start_matches("W/").trim_matches('"').to_string() };
        if strip(&client) != strip(&current) {
            return Err((StatusCode::CONFLICT, "ETag mismatch (stale settings_version)".into()));
        }
        Ok(Self { client_etag: client, _marker: std::marker::PhantomData })
    }
}
```

- [ ] **Step 3: Implement `SettingsVersion` and `GlobalSettingsVersion` marker types**

```rust
pub struct SettingsVersion;

#[async_trait]
impl EtagSource for SettingsVersion {
    async fn current_etag(parts: &mut Parts, state: &AppState) -> Result<String, Report> {
        // resolve_tenant is a **free function** with signature
        //   pub async fn resolve_tenant(parts: &mut Parts, state: &AppState) -> Result<Tenant, Report>
        // (NOT an axum FromRequestParts extractor). Defined in
        // `crates/ui/web-api/src/middleware/tenant.rs` alongside the existing tenant resolver
        // — extract the existing resolver logic into a free fn so EtagSource can call it
        // without going through an extractor (axum extractors can be chained only by calling
        // `OtherType::from_request_parts(parts, state).await`, which is more ceremony than
        // necessary here — a free fn keeps the EtagSource impl flat).
        let tenant = crate::middleware::tenant::resolve_tenant(parts, state).await?;
        let version = state
            .settings_version_cache
            .get(uptrakit_config_reload::config::Scope::Tenant(tenant.id))
            .unwrap_or(0);
        Ok(format!("W/\"settings-v{version}\""))
    }
}
```

If the existing tenant resolver lives only as an extractor (`TenantContext: FromRequestParts`), refactor it so the
inner async function (the part that reads Parts + state and returns the tenant) becomes a `pub` free fn, and the
extractor's `from_request_parts` delegates to it. This is a small, isolated refactor — list it as Step 3a.

`state.settings_version_cache` is the field that Plan 1 Task 16 already added to `AppState` per the foundation
follow-up (spec §14.3). The cache is updated every 2 s by `ConfigReconciler`; staleness window matches the
reconciler poll interval. The 409 / 428 short-circuit happens against the cache, not the DB.

- [ ] **Step 4: Write extractor unit tests** (axum test harness with mock `AppState`)
- [ ] **Step 5:** `cargo test -p uptrakit-web-api --test if_match`
- [ ] **Step 6:** Commit — `feat(web-api): IfMatch<T> extractor + EtagSource trait`

---

## Task 6: Wire `IfMatch<SettingsVersion>` into every settings mutation route

**Files:**

- Modify: every file under `crates/ui/web-api/src/routes/settings*.rs`
- Modify: `crates/ui/web-api/src/routes/plugin_configs.rs`

Each mutation route signature becomes:

```rust
async fn update_settings(
    State(state): State<AppState>,
    _if_match: IfMatch<SettingsVersion>,
    Json(body): Json<UpdateSettingsRequest>,
) -> Result<Response, ApiError> {
    // existing handler
}
```

- [ ] **Step 1: Walk every mutation route** with `rg 'pub async fn .*(Json|axum_extra::extract::Json).*'
crates/ui/web-api/src/routes/settings`
- [ ] **Step 2: Add the extractor parameter to each**
- [ ] **Step 3: Update OpenAPI specs in `crates/shared/openapi-client/src/paths.rs`** to document the
      required `If-Match` header on each mutation endpoint
- [ ] **Step 4: Integration test** — `cargo test -p uptrakit-web-api --test settings_if_match` — verify
      428 on missing header, 409 on stale ETag, 200 on fresh ETag
- [ ] **Step 5:** Commit — `feat(web-api): require If-Match on every settings mutation route`

---

## Task 7: New audit event variants

**Files:**

- Create: `crates/shared/audit-log/src/events/config_reload.rs`
- Modify: `crates/shared/audit-log/src/events/mod.rs`

```rust
// crates/shared/audit-log/src/events/config_reload.rs
use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use uptrakit_config_reload::coordinator::{ReloadPhase, ReloadSource};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ConfigReloadRequested {
    pub source: ReloadSource,
    pub file_path: Option<PathBuf>,
    pub changed_sections: Vec<String>,
    pub reexec: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ConfigReloadApplied {
    pub sections: Vec<String>,
    pub duration_ms: u64,
    /// Per-subsystem timing. Keys are `Reloadable::name()` returns (closed key space).
    pub per_subsystem_ms: BTreeMap<String, u64>,
    pub reexec: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ConfigReloadFailed {
    pub phase: ReloadPhase,
    pub subsystem: Option<String>,
    pub error: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ConfigReloadReverted {
    pub subsystem: String,
    pub reason: String,
}
```

Add four variants to the existing `AuditEvent` enum in `crates/shared/audit-log/src/events/mod.rs`
(it's `#[non_exhaustive]`, additive):

```rust
ConfigReloadRequested(ConfigReloadRequested),
ConfigReloadApplied(ConfigReloadApplied),
ConfigReloadFailed(ConfigReloadFailed),
ConfigReloadReverted(ConfigReloadReverted),
```

- [ ] **Step 1: Implement payload structs**
- [ ] **Step 2: Wire variants into the enum**
- [ ] **Step 3: Update OpenAPI specs** to include the new variants in any client schema generation
- [ ] **Step 4: Round-trip JSON test** — assert `Other(String)` survives serialization round-trip for both
      `ReloadSource` and `ReloadPhase` inside these payloads
- [ ] **Step 5:** Commit — `feat(audit-log): ConfigReload* audit event variants`

---

## Task 8: `system_alerts` wiring

**Files:**

- Modify: `crates/shared/audit-log/src/emitter.rs` — add `write_system_alert(severity, message)` convenience
- Modify: `crates/shared/config-reload/src/alerts.rs` (new) — boundary trait + severity enum
- Modify: `crates/shared/config-reload/src/coordinator/state_machine.rs` — receive
  `Arc<dyn SystemAlertWriter>` at construction
- Modify: `crates/core/controller-runtime/src/reload/audit.rs` — adapter that implements
  `SystemAlertWriter` over `AuditEmitter`

The repo already has `AuditEmitter` + a `system_alerts` table. Adding a parallel `SystemAlertsSink` would create
two paths for operational alerts. Instead, extend the existing audit infrastructure and let
`uptrakit-config-reload` depend on a thin boundary trait so the crate stays ignorant of `uptrakit-audit-log`.

```rust
// crates/shared/config-reload/src/alerts.rs
#[async_trait::async_trait]
pub trait SystemAlertWriter: Send + Sync {
    async fn write(&self, severity: AlertSeverity, message: String);
}

#[non_exhaustive]
#[derive(Clone, Copy, Debug)]
pub enum AlertSeverity {
    Warning,
    Error,
    Critical,
}
```

The controller-runtime crate (which already depends on both `uptrakit-config-reload` and `uptrakit-audit-log`)
provides the adapter implementing `SystemAlertWriter` by delegating to `AuditEmitter::write_system_alert`.

- [ ] **Step 1: Define `SystemAlertWriter` + `AlertSeverity`** in `uptrakit-config-reload`
- [ ] **Step 2: Add `AuditEmitter::write_system_alert(severity, message)`** in `uptrakit-audit-log`
- [ ] **Step 3: Implement the controller-runtime adapter** wrapping `AuditEmitter` as `dyn SystemAlertWriter`
- [ ] **Step 4: Coordinator emits alerts** on validate-fail (Warning) / apply-fail (Error) / watchdog-revert
      (Error) / revert-of-revert (Critical) / reexec-pre-fail (Critical) per spec §15.2
- [ ] **Step 5: Integration test** — inject failing Reloadable, assert correct severity row in `system_alerts`
- [ ] **Step 6:** Commit — `feat(audit-log): system_alerts via AuditEmitter for reload outcomes`

---

## Task 9: `GET /api/v1/instance/config-state` endpoint

**Files:**

- Create: `crates/ui/web-api/src/routes/instance_config_state.rs`
- Modify: `crates/ui/web-api/src/lib.rs` (router registration)

```rust
use axum::{Json, extract::State};
use serde::Serialize;
use time::OffsetDateTime;
use uptrakit_config_reload::CoordinatorState;

use crate::app_state::AppState;
use crate::middleware::auth::RequirePermission;
use uptrakit_shared_types::permissions::Permission;

#[derive(Serialize)]
pub struct ConfigStateResponse {
    pub file: FileState,
    pub last_reload: Option<LastReload>,
    pub sections: serde_json::Value,
    pub recent_events: Vec<serde_json::Value>,
    pub coordinator_state: &'static str,
    pub degraded: Option<DegradedInfoView>,
}

#[derive(Serialize)]
pub struct FileState {
    pub path: String,
    pub digest: String,
    pub loaded_at: OffsetDateTime,
    pub pending_digest: Option<String>,
    pub pending_detected_at: Option<OffsetDateTime>,
}

#[derive(Serialize)]
pub struct LastReload { /* mirror spec §15.4 */ }

#[derive(Serialize)]
pub struct DegradedInfoView { /* mirror spec §8.3 + §15.4 */ }

pub async fn get_config_state(
    State(state): State<AppState>,
    _perm: crate::middleware::permission::CanViewInstanceConfigState,
) -> Result<Json<ConfigStateResponse>, crate::error::ApiError> {
    let coordinator_state = state.coordinator_handle.state();
    let resp = ConfigStateResponse {
        file: state.config_file_state.borrow().clone(),
        last_reload: state.last_reload.borrow().clone(),
        sections: render_sections(&state)?,
        recent_events: state.recent_reload_events.borrow().clone(),
        coordinator_state: state_label(&coordinator_state),
        degraded: degraded_view(&coordinator_state),
    };
    Ok(Json(resp))
}
```

Each `Serialize` struct above gets a real definition. Secret fields render as the literal string `"<redacted>"`.

- [ ] **Step 1: Implement response types**
- [ ] **Step 2: Implement handler**
- [ ] **Step 3: Add to router**
- [ ] **Step 4:** OpenAPI spec entry in `paths.rs`
- [ ] **Step 5:** Integration test asserting permission gate + redaction
- [ ] **Step 6:** Commit — `feat(web-api): GET /api/v1/instance/config-state`

---

## Task 10: `POST /api/v1/instance/config-reload/clear-degraded` endpoint

**Files:**

- Modify: `crates/ui/web-api/src/routes/instance_config_state.rs`

```rust
pub async fn clear_degraded(
    State(state): State<AppState>,
    manage: crate::middleware::permission::CanManageInstanceConfigState,
) -> Result<Json<ConfigStateResponse>, crate::error::ApiError> {
    state.coordinator_handle.clear_degraded().await.map_err(|e| {
        crate::error::ApiError::internal(format!("clear_degraded failed: {e}"))
    })?;
    // Manage implies View; build a View extractor by reusing the inner AuthenticatedUser so we
    // don't re-run the auth check.
    let view = crate::middleware::permission::CanViewInstanceConfigState::new(manage.0);
    get_config_state(State(state), view).await
}
```

`ReloadCoordinatorHandle::clear_degraded` becomes a real method in this task (Plan 1 left it as a stub).
Implementation: send a `ControlMessage::ClearDegraded` via a dedicated control channel; the coordinator's `run`
loop dispatches it.

- [ ] **Step 1: Add `ControlMessage::ClearDegraded` to coordinator's internal channel**
- [ ] **Step 2: Wire run-loop dispatch + call into existing `ReloadCoordinator::clear_degraded`**
- [ ] **Step 3: Implement handler**
- [ ] **Step 4: Integration test** — boot coordinator into Degraded (via injected failing Reloadable), POST,
      assert state returns to Idle
- [ ] **Step 5:** Commit — `feat(web-api): POST /api/v1/instance/config-reload/clear-degraded`

---

## Task 11: Reexec helper — listenfd integration

**Files:**

- Create: `crates/core/controller-runtime/src/reexec/mod.rs`
- Create: `crates/core/controller-runtime/src/reexec/listenfd.rs`
- Modify: `Cargo.toml` (workspace + `controller-runtime`) — add `listenfd`
- Modify: `crates/core/controller-runtime/src/startup/mod.rs` — claim inherited sockets on boot

```rust
// reexec/listenfd.rs
use listenfd::ListenFd;
use rootcause::Report;
use tokio::net::TcpListener;

/// Ordered slot enum for inherited sockets. Adding a new listener (metrics, gRPC admin,
/// liveness probe, …) **requires** appending to this enum AND updating `INHERITED_SLOT_COUNT`
/// AND the parent's pre-`exec()` FD-clearing logic in lockstep. Positional `take_tcp_listener(N)`
/// alone is too easy to get wrong silently — a future PR that inserts a slot in the middle
/// would swap PKI ↔ metrics and the test suite would not catch it.
#[repr(usize)]
#[non_exhaustive]
pub enum ListenerSlot {
    Https = 0,
    Pki = 1,
}

pub const INHERITED_SLOT_COUNT: usize = 2;

/// Compile-time sanity: if a contributor adds a variant to `ListenerSlot` without bumping the
/// count, this assertion fails at build time. (Variant count check is best-effort — the real
/// guarantee comes from the explicit `#[non_exhaustive]` discipline + ADR amendment.)
const _: () = assert!(
    INHERITED_SLOT_COUNT == 2,
    "ListenerSlot count out of sync with INHERITED_SLOT_COUNT; update both together"
);

pub fn take_inherited_listeners() -> Result<Option<InheritedSockets>, Report> {
    let mut lf = ListenFd::from_env();
    if lf.len() == 0 {
        return Ok(None);
    }
    if lf.len() != INHERITED_SLOT_COUNT {
        return Err(rootcause::report!(
            "LISTEN_FDS={} but binary expects {INHERITED_SLOT_COUNT}",
            lf.len()
        ));
    }
    let https = take_one(&mut lf, ListenerSlot::Https)?;
    let pki = take_one(&mut lf, ListenerSlot::Pki)?;
    https.set_nonblocking(true).ok();
    pki.set_nonblocking(true).ok();
    Ok(Some(InheritedSockets {
        https: TcpListener::from_std(https)?,
        pki: TcpListener::from_std(pki)?,
    }))
}

fn take_one(lf: &mut ListenFd, slot: ListenerSlot) -> Result<std::net::TcpListener, Report> {
    let idx = slot as usize;
    lf.take_tcp_listener(idx)
        .map_err(|e| rootcause::report!("take_tcp_listener({idx}) failed: {e}"))?
        .ok_or_else(|| rootcause::report!("LISTEN_FDS slot {idx} ({:?}) empty", slot_name(slot)))
}

fn slot_name(slot: ListenerSlot) -> &'static str {
    match slot {
        ListenerSlot::Https => "Https",
        ListenerSlot::Pki => "Pki",
    }
}

#[non_exhaustive]
pub struct InheritedSockets {
    pub https: TcpListener,
    pub pki: TcpListener,
}
```

The parent's pre-`exec()` `FD_CLOEXEC` clearing loop (Task 13) iterates over the listeners in the same
`ListenerSlot` enum order. Adding a future listener is a three-touch change: enum variant + `INHERITED_SLOT_COUNT`
bump + parent loop. The compile-time assert plus `lf.len() != INHERITED_SLOT_COUNT` runtime guard catches
desyncs in either direction.

`startup` checks for inherited sockets and uses them in place of fresh `bind()`.

- [ ] **Step 1: Add `listenfd` to workspace + crate deps**
- [ ] **Step 2: Implement helper**
- [ ] **Step 3: Wire into startup**
- [ ] **Step 4:** Test by setting `LISTEN_FDS=2 LISTEN_PID=$$` in a smoke harness
- [ ] **Step 5:** Commit — `feat(controller-runtime): claim inherited LISTEN_FDS sockets`

---

## Task 12: Reexec helper — sd-notify READY signal

**Files:**

- Create: `crates/core/controller-runtime/src/reexec/sd_notify.rs`
- Modify: `Cargo.toml` — add `sd-notify`

```rust
use sd_notify::NotifyState;

pub fn signal_ready() {
    // No-op when NOTIFY_SOCKET is unset (e.g., macOS dev, FreeBSD without supervisor).
    let _ = sd_notify::notify(false, &[NotifyState::Ready]);
}

pub fn signal_status(text: &str) {
    let _ = sd_notify::notify(false, &[NotifyState::Status(text)]);
}
```

After every Reloadable's boot-time `health_check()` passes, controller-runtime calls `signal_ready()`. Also prints
the literal `READY` line on stdout for non-systemd supervisors.

- [ ] **Step 1: Add dep + helper**
- [ ] **Step 2: Wire into startup completion**
- [ ] **Step 3:** Smoke test with `systemd-run --user --pty -p Type=notify -- cargo run --bin uptrakit-controller`
- [ ] **Step 4:** Commit — `feat(controller-runtime): sd_notify READY=1 + stdout READY fallback`

---

## Task 13: Reexec triage + the actual `exec()` swap

**Files:**

- Create: `crates/core/controller-runtime/src/reexec/triage.rs`
- Create (continued): `crates/core/controller-runtime/src/reexec/mod.rs`
- Modify: `crates/core/controller-runtime/src/startup/mod.rs` — register triage callback

```rust
// reexec/triage.rs
use uptrakit_config_reload::config::RuntimeConfig;

pub struct ReexecDecision {
    pub needed: bool,
    pub reasons: Vec<&'static str>,
}

pub fn decide(prior: &RuntimeConfig, new: &RuntimeConfig) -> ReexecDecision {
    let mut reasons = Vec::new();
    if prior.db.url != new.db.url { reasons.push("db.url"); }
    if prior.master_key.path != new.master_key.path { reasons.push("master_key.path"); }
    if prior.log.path != new.log.path { reasons.push("log.path"); }
    if prior.embedded_services != new.embedded_services {
        reasons.push("embedded_services topology");
    }
    ReexecDecision { needed: !reasons.is_empty(), reasons }
}
```

`reexec/mod.rs` performs the swap. Args **allowlist** (no `env::args().skip(1)` passthrough — explicit rebuild):

```rust
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

use rootcause::Report;

pub struct ReexecPlan {
    pub current_exe: std::path::PathBuf,
    pub config_path: std::path::PathBuf,
    pub master_key_from: Option<String>,
    pub listener_count: usize,
    pub generation: u64,
}

pub fn perform_reexec(plan: &ReexecPlan) -> Result<std::convert::Infallible, Report> {
    let mut cmd = Command::new(&plan.current_exe);
    cmd.arg("--config").arg(&plan.config_path);
    if let Some(mk) = &plan.master_key_from {
        cmd.arg("--master-key-from").arg(mk);
    }
    cmd.env("LISTEN_FDS", plan.listener_count.to_string());
    // exec() preserves PID; getpid() in the parent is also the child's PID, which is what
    // sd_listen_fds requires.
    cmd.env("LISTEN_PID", std::process::id().to_string());
    cmd.env("UPTRAKIT_REEXEC_GENERATION", (plan.generation + 1).to_string());
    #[expect(
        unreachable_code,
        reason = "CommandExt::exec() diverges into the new process image on success; only the \
                  error branch is reachable here. Workspace lint clippy::diverging_sub_expression \
                  would otherwise warn."
    )]
    let err = cmd.exec();
    Err(rootcause::report!("exec failed: {err}"))
}
```

The coordinator's apply path checks the triage decision:

```rust
let decision = reexec::triage::decide(&prior_runtime, &new_runtime);
if decision.needed {
    audit_emitter.emit(AuditEvent::ConfigReloadRequested(ConfigReloadRequested {
        source: source.clone(),
        file_path: Some(config_path.clone()),
        changed_sections: decision.reasons.iter().map(|s| (*s).to_string()).collect(),
        reexec: true,
    }));
    audit_dispatcher.flush().await;
    // Clear FD_CLOEXEC on listeners (`nix::fcntl::fcntl(fd, F_SETFD, FdFlag::empty())`).
    clear_cloexec_on_listeners(&listeners)?;
    let plan = ReexecPlan { /* … */ };
    reexec::perform_reexec(&plan)?; // diverges
}
```

- [ ] **Step 1: Implement triage + reexec helpers**
- [ ] **Step 2: Wire into the coordinator's pre-apply step (only the controller-runtime owner of the coordinator
      knows about reexec — keep config-reload crate ignorant of it)**
- [ ] **Step 3:** Commit — `feat(controller-runtime): reexec via LISTEN_FDS + arg allowlist`

---

## Task 14: Reexec Docker integration test

**Files:**

- Create: `crates/integration-tests/tests/reexec.rs`
- Modify: `docker/Dockerfile.test` if needed

The test:

1. Build the controller in the existing `uptrakit-test` image.
2. Start a long-lived HTTP client connection to the HTTPS listener.
3. Bind-mount a writable TOML file.
4. Edit the TOML to flip `embedded_services.scheduler` (triggers reexec triage).
5. Send SIGHUP.
6. Assert the post-reexec invariants:
   - The **process PID is unchanged** (`exec()` preserves PID — that's the LISTEN_PID protocol's foundation).
     Read `/proc/self/status` (or the container equivalent) before and after; both must show the same `Pid:`
     line.
   - The `UPTRAKIT_REEXEC_GENERATION` environment variable in the running process increments by 1 (read via
     a debug endpoint or via `/proc/self/environ`).
   - The listening port stays available (the test client's keep-alive reconnect succeeds within 2 s, even
     though already-accepted TCP connections were reset per spec §11.3 honest disclosure).

Marked `#[ignore]` so only the existing Docker-gated path runs it.

- [ ] **Step 1: Implement test**
- [ ] **Step 2:** Run via `docker build -f docker/Dockerfile.test -t uptrakit-test:latest .
&& cargo test -p uptrakit-integration-tests reexec -- --ignored`
- [ ] **Step 3:** Commit — `test(integration): reexec end-to-end with socket inheritance`

---

## Task 15: ETag round-trip integration test

**Files:**

- Create: `crates/ui/web-api/tests/if_match.rs`

- [ ] **Step 1: Boot the web-api with in-memory SQLite + seeded `settings_version` row**
- [ ] **Step 2: PUT a settings endpoint without `If-Match` → 428**
- [ ] **Step 3: PUT with stale `If-Match` → 409**
- [ ] **Step 4: GET → response includes ETag header**
- [ ] **Step 5: PUT with fresh ETag → 200**
- [ ] **Step 6: After mutation, the ETag changes (next GET shows higher version)**
- [ ] **Step 7:** Commit — `test(web-api): If-Match optimistic locking round-trip`

---

## Task 16: Quality gates + PR

- [ ] **Step 1:** `cargo fmt --all -- --check`
- [ ] **Step 2:** `cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings`
- [ ] **Step 3:** `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] **Step 4:** `cargo deny check`
- [ ] **Step 5:** `cargo test --no-default-features --features db-sqlite`
- [ ] **Step 6:** `cargo test --all-features`
- [ ] **Step 7:** Docker reload-integration suite

```bash
docker build -f docker/Dockerfile.test -t uptrakit-test:latest .
cargo test -p uptrakit-integration-tests -- --ignored
```

- [ ] **Step 8:** PR titled `feat(controller)!: migration + API surface + reexec for graceful reload`.
      Body must call out the CLI hard break + the migration's destructive nature.

## Self-review

- Spec §6.4 CLI shrink — Task 3 ✓
- Spec §6.3 SettingKey purge — Tasks 1, 2 ✓
- Spec §11 Reexec — Tasks 11–14 ✓
- Spec §11.3 args allowlist (NOT `env::args().skip(1)`) — Task 13 ✓
- Spec §11.4 sd-notify READY — Task 12 ✓
- Spec §14 IfMatch / ETag — Tasks 5, 6, 15 ✓
- Spec §15.1 audit event variants — Task 7 ✓
- Spec §15.2 system_alerts severity map — Task 8 ✓
- Spec §15.3 permissions — Task 4 ✓
- Spec §15.4 new endpoints — Tasks 9, 10 ✓
- Spec §20 breaking changes — Tasks 1, 2, 3 ✓
- Snapshot rules: every wire-exposed type has `Other(String)` or `#[non_exhaustive]`; every error path returns
  `rootcause::Report`; no `unwrap()` in production; `clippy::large_futures = "deny"` honoured (no `join_all`
  introduced).
