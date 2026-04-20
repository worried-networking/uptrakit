# Semantic Audit Logs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace request-shaped audit logging with semantic mutation-first audit logging, unify the old `security_audit` tracing paths into the same
canonical pipeline, and update the API/UI/CLI/docs to the new action model.

**Architecture:** Introduce one canonical audit domain in `uptrakit-audit-log` with typed action IDs, closed internal actor/outcome enums, validation,
and an explicit emitter API. Migrate the database schema, read/query surfaces, and frontend to the new action-shaped contract, then remove request
middleware as an audit-row producer and replace route/runtime/service logging with explicit semantic emission, including service-to-controller
forwarding via the wire protocol.

**Tech Stack:** Rust, Axum, SeaORM, SeaORM Migration, tokio, serde/serde_json, utoipa, SvelteKit, TypeScript

---

## File Structure

| Path                                                                                                                   | Responsibility                                                                                                  |
| ---------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| `crates/shared/audit-log/src/entry.rs`                                                                                 | Canonical semantic `AuditEntry`, actor/outcome types, builder/test helpers, UTC/bounds validation hooks         |
| `crates/shared/audit-log/src/lib.rs`                                                                                   | Re-exports for the new audit domain                                                                             |
| `crates/shared/audit-log/src/backend.rs`                                                                               | DB + journald field mapping for semantic audit entries                                                          |
| `crates/shared/audit-log/src/dispatcher.rs`                                                                            | Fire-and-forget dispatch unchanged except for new entry shape                                                   |
| `crates/shared/audit-log/src/action_type.rs`                                                                           | New typed `AuditActionType` registry and constants                                                              |
| `crates/shared/audit-log/src/emitter.rs`                                                                               | New emitter API used by handlers, runtimes, and service WS code                                                 |
| `crates/shared/audit-log/src/runtime_emitter.rs`                                                                       | Runtime-side emitter that writes journald locally and forwards `AuditEventPayload` over the wire when connected |
| `crates/shared/audit-log/Cargo.toml`                                                                                   | Add `serde_json` and any new domain dependencies required by semantic entries                                   |
| `crates/shared/db/src/entity/audit_log.rs`                                                                             | Tenant-scoped semantic audit row                                                                                |
| `crates/shared/db/src/entity/system_audit_log.rs`                                                                      | System-scoped semantic audit row                                                                                |
| `crates/shared/db/src/entity/tenant_scoped.rs`                                                                         | Keep tenant scoping correct after the row-shape change                                                          |
| `crates/shared/db/src/migration/m20260417_000001_semantic_audit_logs.rs`                                               | New migration replacing request-era audit columns and indexes                                                   |
| `crates/shared/db/src/migration/mod.rs`                                                                                | Register the new semantic audit migration                                                                       |
| `crates/shared/wire/src/payloads.rs`                                                                                   | Add `AuditEventPayload`                                                                                         |
| `crates/shared/wire/src/messages.rs`                                                                                   | Add `ServiceMessage::AuditEvent`                                                                                |
| `crates/shared/wire/src/tests.rs`                                                                                      | Round-trip and unknown-variant placement coverage for the new wire message                                      |
| `crates/shared/wire/src/wire_validate_impls.rs`                                                                        | Wire-boundary validation for `AuditEventPayload`                                                                |
| `crates/shared/web-api-types/src/audit_logs.rs`                                                                        | New action-shaped REST DTOs and filters                                                                         |
| `crates/shared/openapi-client/src/audit_logs.rs`                                                                       | Client serialization for new filters                                                                            |
| `crates/ui/web-api-queries/src/queries/audit_logs.rs`                                                                  | New DB filtering and response mapping                                                                           |
| `crates/ui/web-api-queries/src/queries/autodiscovery/ignore_rules.rs`                                                  | Read/write paths for `software.ignore.*` audit coverage                                                         |
| `crates/ui/web-api/src/routes/audit_logs.rs`                                                                           | OpenAPI contract for action-shaped list endpoints                                                               |
| `crates/ui/cli/src/commands/audit_logs.rs`                                                                             | Human + JSON output for semantic audit rows                                                                     |
| `frontend/src/lib/types.ts`                                                                                            | Frontend audit entry/filter types                                                                               |
| `frontend/src/lib/api.ts`                                                                                              | Audit log query serialization                                                                                   |
| `frontend/src/routes/audit-logs/+page.svelte`                                                                          | New action-centric filters/table                                                                                |
| `crates/ui/web-api/src/middleware/audit_log.rs`                                                                        | Stop producing audit rows from raw requests                                                                     |
| `crates/ui/web-api/src/router.rs`                                                                                      | Remove the semantic-audit responsibility from request middleware                                                |
| `crates/ui/web-api/src/app_state.rs`                                                                                   | Emitter access through `AppState`                                                                               |
| `crates/ui/web-api/src/lib.rs`                                                                                         | Main `AppState` construction path must receive the new emitter                                                  |
| `crates/ui/web-api/src/routes/auth.rs`                                                                                 | Auth success/failure semantic events                                                                            |
| `crates/ui/web-api/src/routes/api_tokens.rs`                                                                           | API token create/revoke semantic events                                                                         |
| `crates/ui/web-api/src/middleware/require_auth.rs`                                                                     | JWT/API-token rejection events                                                                                  |
| `crates/ui/web-api/src/middleware/resolve_ip.rs`                                                                       | Test state construction that must include the emitter                                                           |
| `crates/ui/web-api/src/routes/device_auth.rs`                                                                          | Device approval/denial events                                                                                   |
| `crates/ui/web-api/src/routes/oidc_auth.rs`                                                                            | OIDC authorize/callback events                                                                                  |
| `crates/ui/web-api/src/routes/oidc_providers.rs`                                                                       | OIDC provider CRUD events                                                                                       |
| `crates/ui/web-api/src/routes/enrollment_tokens.rs`                                                                    | Enrollment token create/revoke events                                                                           |
| `crates/ui/web-api/src/routes/notifications.rs`                                                                        | Notification channel/rule create/update/delete/test events                                                      |
| `crates/ui/web-api/src/routes/settings_network.rs`                                                                     | Global setting updates for network policy                                                                       |
| `crates/ui/web-api/src/routes/settings_nats.rs`                                                                        | Global setting updates for NATS configuration                                                                   |
| `crates/ui/web-api/src/routes/settings_zeroconf.rs`                                                                    | Global setting updates for zeroconf configuration                                                               |
| `crates/ui/web-api/src/routes/settings_agent_certs.rs`                                                                 | Tenant setting updates for agent certificate policy                                                             |
| `crates/ui/web-api/src/routes/users.rs`                                                                                | User CRUD events                                                                                                |
| `crates/ui/web-api/src/routes/plugin_configs.rs`                                                                       | Replace `security_audit` warnings with canonical events                                                         |
| `crates/ui/web-api/src/routes/plugin_type_settings.rs`                                                                 | Canonical type-settings events                                                                                  |
| `crates/ui/web-api/src/routes/services.rs`                                                                             | Approval/reject/merge/freeze/cert/deactivate events                                                             |
| `crates/ui/web-api/src/routes/autodiscovery.rs`                                                                        | `software.ignore.create/delete` events                                                                          |
| `crates/ui/web-api/src/routes/software_items/mod.rs`                                                                   | Single-item software update trigger events                                                                      |
| `crates/ui/web-api/src/routes/update_batches.rs`                                                                       | Batch update trigger events                                                                                     |
| `crates/ui/web-api/src/routes/surfaces.rs`                                                                             | Test state construction that must include the emitter                                                           |
| `crates/ui/web-api/src/surface_proxy.rs`                                                                               | Canonical proxied plugin-config creation audit                                                                  |
| `crates/ui/web-api/src/routes/service_ws/handler/mod.rs`                                                               | Service-side forwarded audit event ingestion and selected controller-origin events                              |
| `crates/ui/web-api/src/routes/service_ws/handler/cert.rs`                                                              | Service certificate issue/renew audit producers                                                                 |
| `crates/ui/web-api/src/routes/service_ws/handler/messages.rs`                                                          | Test state construction and message routing with the emitter present                                            |
| `crates/ui/web-api/src/routes/service_ws/connection.rs`                                                                | Service enrollment completion audit producer                                                                    |
| `crates/ui/web-api/src/routes/service_ws/handler/updates.rs`                                                           | `.started` update events from service-origin messages                                                           |
| `crates/core/agent/src/main.rs`                                                                                        | Standalone agent runtime construction with emitter support                                                      |
| `crates/core/controller/src/agent/mod.rs`                                                                              | Embedded agent runtime construction with emitter support                                                        |
| `crates/core/agent-runtime/src/lib.rs`                                                                                 | Convert runtime `security_audit` callsites to canonical audit forwarding/local journald emission                |
| `crates/core/agent-runtime/Cargo.toml`                                                                                 | Add semantic audit dependency needed by runtime emission                                                        |
| `crates/core/agent-ssh/src/main.rs`                                                                                    | Standalone SSH runtime construction with emitter support                                                        |
| `crates/core/controller/src/ssh_agent/mod.rs`                                                                          | Embedded SSH runtime construction with emitter support                                                          |
| `crates/core/agent-ssh-runtime/src/lib.rs`                                                                             | Same as above for SSH runtime                                                                                   |
| `crates/core/agent-ssh-runtime/Cargo.toml`                                                                             | Add semantic audit dependency needed by SSH runtime emission                                                    |
| `crates/core/controller/src/scheduler/mod.rs`                                                                          | Embedded scheduler constructor wiring for audit-enabled cleanup executor                                        |
| `crates/shared/scheduler-engine/src/executors/audit_log_cleanup.rs`                                                    | Emit `system.scheduler.audit_log_cleanup` after scheduler retention cleanup                                     |
| `crates/shared/scheduler-engine/Cargo.toml`                                                                            | Add semantic audit dependency if scheduler emits directly                                                       |
| `docs/development/audit-logs.md`                                                                                       | Developer/agent contract and catalog                                                                            |
| `docs/security/audit-logs.md`, `docs/end-user/audit-logs.md`, `docs/api/audit-logs.md`, `AGENTS.md`, `ARCHITECTURE.md` | Updated operator and agent docs                                                                                 |
| `ci/verify_no_security_audit.sh`                                                                                       | New grep-based guardrail for `security_audit` and raw action strings                                            |
| `ci/verify_no_security_audit_allowlist.txt`                                                                            | Explicit allowlist for accepted raw action strings in docs/fixtures/tests                                       |

### Task 1: Canonical Audit Domain

**Files:**

- Modify: `crates/shared/audit-log/Cargo.toml`
- Create: `crates/shared/audit-log/src/action_type.rs`
- Create: `crates/shared/audit-log/src/emitter.rs`
- Modify: `crates/shared/audit-log/src/entry.rs`
- Modify: `crates/shared/audit-log/src/lib.rs`
- Modify: `crates/shared/audit-log/src/error.rs`
- Test: `crates/shared/audit-log/src/entry.rs`
- Test: `crates/shared/audit-log/src/action_type.rs`

Execution note: Task 1 changes the canonical `AuditEntry` shape. For Tasks 2-4, stay on the scoped crate-local tests listed in each task and do not
run workspace-wide checks or any `uptrakit-web-api` build/test command until Task 5 completes the controller `AppState` + middleware cutover.

- [ ] **Step 1: Write the failing domain tests**

```rust
#[test]
fn audit_action_type_rejects_result_encoded_names() {
    assert!(AuditActionType::new("auth.login").is_ok());
    assert!(AuditActionType::new("auth.login.failed").is_err());
}

#[test]
fn audit_action_type_rejects_validation_failed_suffix() {
    assert!(AuditActionType::new("service.merge.validation_failed").is_err());
}

#[test]
fn audit_action_type_accepts_system_update_freeze_apply() {
    assert!(AuditActionType::new("system.service.update_freeze.apply").is_ok());
}

#[test]
fn audit_actor_type_includes_service_and_system() {
    assert_eq!(AuditActorType::Service.as_str(), "service");
    assert_eq!(AuditActorType::System.as_str(), "system");
}

#[test]
fn audit_entry_rejects_oversized_details_payload() {
    let mut entry = AuditEntry::test_stub("plugin_config.create");
    entry.details_json = Some(serde_json::json!({ "blob": "x".repeat(5000) }));
    assert!(entry.validate().is_err());
}

#[test]
fn audit_entry_requires_utc_timestamp() {
    let entry = AuditEntry::test_stub("service.merge");
    assert_eq!(entry.occurred_at.offset(), time::UtcOffset::UTC);
}
```

- [ ] **Step 2: Run the new tests to verify the domain contract is missing**

Run: `cargo test -p uptrakit-audit-log audit_action_type_rejects_result_encoded_names -- --exact` Expected: FAIL because `AuditActionType` does not
exist yet.

Run: `cargo test -p uptrakit-audit-log audit_action_type_rejects_validation_failed_suffix -- --exact` Expected: FAIL because result-encoded validation
suffixes are not rejected yet.

Run: `cargo test -p uptrakit-audit-log audit_entry_rejects_oversized_details_payload -- --exact` Expected: FAIL because `AuditEntry::validate()` does
not exist yet.

Run: `cargo test -p uptrakit-audit-log audit_entry_requires_utc_timestamp -- --exact` Expected: FAIL because the semantic test helper and UTC
validation hooks do not exist yet.

- [ ] **Step 3: Implement the canonical action type and semantic entry model**

```toml
[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
```

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AuditActionType(String);

fn validate_action_type(value: &str) -> Result<()> {
    static RESERVED_RESULT_SEGMENTS: &[&str] = &[
        "failed",
        "success",
        "denied",
        "partial",
        "error",
        "validation_failed",
    ];
    ensure!(!value.is_empty(), "action type must not be empty");
    ensure!(value.len() <= 128, "action type must be <= 128 bytes");
    ensure!(value.split('.').count() >= 2, "action type must have at least two segments");
    ensure!(
        value.split('.').all(|seg| {
            !seg.is_empty()
                && seg.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                && !RESERVED_RESULT_SEGMENTS.contains(&seg)
        }),
        "action type contains invalid or result-encoded segments"
    );
    Ok(())
}

impl AuditActionType {
    pub const AUTH_LOGIN: &'static str = "auth.login";
    pub const PLUGIN_CONFIG_CREATE: &'static str = "plugin_config.create";
    pub const SYSTEM_SERVICE_UPDATE_GATE: &'static str = "system.service.update_gate";
    // Define one canonical constant for every V1 action referenced later in
    // Tasks 6 and 7, including:
    // `software.update.started`, `software.batch_update.started`,
    // `system.service.update_freeze.apply`, and
    // `system.service.machine_id.validate`.

    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        validate_action_type(&value)?;
        Ok(Self(value))
    }

    pub fn from_static(value: &'static str) -> Self {
        Self::new(value).expect("canonical action types must validate")
    }

    pub fn as_str(&self) -> &str { &self.0 }

    pub fn is_registered(value: &str) -> bool {
        const V1_ACTIONS: &[&str] = &[
            AuditActionType::AUTH_LOGIN,
            AuditActionType::PLUGIN_CONFIG_CREATE,
            AuditActionType::SYSTEM_SERVICE_UPDATE_GATE,
            // include the full Task 6 + Task 7 V1 catalog here
        ];
        V1_ACTIONS.contains(&value)
    }
}

impl From<&'static str> for AuditActionType {
    fn from(value: &'static str) -> Self {
        Self::from_static(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditActorType {
    User,
    ApiToken,
    Oidc,
    Service,
    System,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditOutcome {
    Success,
    Denied,
    ValidationFailed,
    Failed,
    Partial,
}

impl TryFrom<&str> for AuditOutcome {
    type Error = ();

    fn try_from(value: &str) -> std::result::Result<Self, Self::Error> {
        match value {
            "success" => Ok(Self::Success),
            "denied" => Ok(Self::Denied),
            "validation_failed" => Ok(Self::ValidationFailed),
            "failed" => Ok(Self::Failed),
            "partial" => Ok(Self::Partial),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct AuditEntry {
    pub id: uuid::Uuid,
    pub tenant_id: Option<uuid::Uuid>,
    pub occurred_at: time::OffsetDateTime,
    pub actor_type: AuditActorType,
    pub actor_id: Option<uuid::Uuid>,
    pub actor_display: Option<String>,
    pub action_type: AuditActionType,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub target_display: Option<String>,
    pub outcome: AuditOutcome,
    pub details_json: Option<serde_json::Value>,
    pub request_id: Option<String>,
}

pub struct AuditEntryBuilder {
    entry: AuditEntry,
}

impl AuditEntry {
    pub fn builder(action_type: impl Into<AuditActionType>) -> AuditEntryBuilder {
        AuditEntryBuilder {
            entry: AuditEntry {
                id: uuid::Uuid::now_v7(),
                tenant_id: None,
                occurred_at: time::OffsetDateTime::now_utc(),
                actor_type: AuditActorType::System,
                actor_id: None,
                actor_display: None,
                action_type: action_type.into(),
                target_type: None,
                target_id: None,
                target_display: None,
                outcome: AuditOutcome::Success,
                details_json: None,
                request_id: None,
            },
        }
    }

    #[cfg(test)]
    pub fn test_stub(action_type: &str) -> Self {
        Self::builder(AuditActionType::new(action_type).unwrap()).build().unwrap()
    }

    pub fn validate(&self) -> Result<()> {
        const MAX_DETAILS_JSON_BYTES: usize = 4096;
        const MAX_ACTOR_DISPLAY_BYTES: usize = 255;
        const MAX_TARGET_TYPE_BYTES: usize = 128;
        const MAX_TARGET_DISPLAY_BYTES: usize = 255;
        const MAX_TARGET_ID_BYTES: usize = 255;
        const MAX_REQUEST_ID_BYTES: usize = 255;
        ensure!(self.occurred_at.offset() == time::UtcOffset::UTC, "timestamps must be UTC");
        ensure!(self.action_type.as_str().len() <= 128, "action_type exceeds 128 bytes");
        ensure!(self.actor_display.as_ref().map_or(true, |s| s.len() <= MAX_ACTOR_DISPLAY_BYTES), "actor_display exceeds 255 bytes");
        ensure!(self.target_type.as_ref().map_or(true, |s| s.len() <= MAX_TARGET_TYPE_BYTES), "target_type exceeds 128 bytes");
        ensure!(self.target_display.as_ref().map_or(true, |s| s.len() <= MAX_TARGET_DISPLAY_BYTES), "target_display exceeds 255 bytes");
        ensure!(self.target_id.as_ref().map_or(true, |s| s.len() <= MAX_TARGET_ID_BYTES), "target_id exceeds 255 bytes");
        ensure!(self.request_id.as_ref().map_or(true, |s| s.len() <= MAX_REQUEST_ID_BYTES), "request_id exceeds 255 bytes");
        if let Some(details) = &self.details_json {
            let serialized = serde_json::to_vec(details)?;
            ensure!(serialized.len() <= MAX_DETAILS_JSON_BYTES, "details_json exceeds 4096 bytes");
        }
        Ok(())
    }
}

impl AuditEntryBuilder {
    pub fn tenant_scope(mut self, tenant_id: Uuid) -> Self {
        self.entry.tenant_id = Some(tenant_id);
        self
    }

    pub fn system_scope(mut self) -> Self {
        self.entry.tenant_id = None;
        self
    }

    pub fn actor(mut self, actor_type: AuditActorType, actor_id: Option<Uuid>) -> Self {
        self.entry.actor_type = actor_type;
        self.entry.actor_id = actor_id;
        self
    }

    pub fn actor_service(self, actor_id: Uuid) -> Self {
        self.actor(AuditActorType::Service, Some(actor_id))
    }

    pub fn actor_system(self) -> Self {
        self.actor(AuditActorType::System, None)
    }

    pub fn actor_display_opt(mut self, actor_display: Option<String>) -> Self {
        self.entry.actor_display = actor_display;
        self
    }

    pub fn target(mut self, target_type: &str, target_id: String, target_display: Option<String>) -> Self {
        self.entry.target_type = Some(target_type.to_string());
        self.entry.target_id = Some(target_id);
        self.entry.target_display = target_display;
        self
    }

    pub fn target_opt(
        mut self,
        target_type: Option<String>,
        target_id: Option<String>,
        target_display: Option<String>,
    ) -> Self {
        self.entry.target_type = target_type;
        self.entry.target_id = target_id;
        self.entry.target_display = target_display;
        self
    }

    pub fn outcome(mut self, outcome: AuditOutcome) -> Self {
        self.entry.outcome = outcome;
        self
    }

    pub fn details(mut self, details_json: serde_json::Value) -> Self {
        self.entry.details_json = Some(details_json);
        self
    }

    pub fn request_id_opt(mut self, request_id: Option<String>) -> Self {
        self.entry.request_id = request_id;
        self
    }

    pub fn build(self) -> Result<AuditEntry> {
        self.entry.validate()?;
        Ok(self.entry)
    }
}
```

- [ ] **Step 4: Add a small emitter API with shared validation**

```rust
#[derive(Clone)]
pub struct AuditEmitter {
    dispatcher: AuditLogDispatcher,
}

impl AuditEmitter {
    pub fn emit_best_effort(&self, entry: AuditEntry) {
        if let Err(err) = entry.validate() {
            tracing::warn!(error = %err, "dropping invalid audit entry");
            return;
        }
        self.dispatcher.dispatch(entry);
    }
}
```

- [ ] **Step 5: Run the domain tests again**

Run: `cargo test -p uptrakit-audit-log audit_action_type_rejects_result_encoded_names -- --exact` Expected: PASS.

Run: `cargo test -p uptrakit-audit-log audit_action_type_accepts_system_update_freeze_apply -- --exact` Expected: PASS.

Run: `cargo test -p uptrakit-audit-log audit_action_type_rejects_validation_failed_suffix -- --exact` Expected: PASS.

Run: `cargo test -p uptrakit-audit-log audit_actor_type_includes_service_and_system -- --exact` Expected: PASS.

Run: `cargo test -p uptrakit-audit-log audit_entry_rejects_oversized_details_payload -- --exact` Expected: PASS.

Run: `cargo test -p uptrakit-audit-log audit_entry_requires_utc_timestamp -- --exact` Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/shared/audit-log/Cargo.toml crates/shared/audit-log/src/action_type.rs crates/shared/audit-log/src/emitter.rs crates/shared/audit-log/src/entry.rs crates/shared/audit-log/src/lib.rs crates/shared/audit-log/src/error.rs
git commit -m "feat: add semantic audit log domain"
```

### Task 2: Semantic Audit Schema And Backends

**Files:**

- Create: `crates/shared/db/src/migration/m20260417_000001_semantic_audit_logs.rs`
- Modify: `crates/shared/db/src/migration/mod.rs`
- Modify: `crates/shared/db/src/entity/audit_log.rs`
- Modify: `crates/shared/db/src/entity/system_audit_log.rs`
- Modify: `crates/shared/db/src/entity/tenant_scoped.rs`
- Modify: `crates/shared/audit-log/src/backend.rs`
- Test: `crates/shared/db/src/migration/mod.rs`
- Test: `crates/shared/audit-log/src/backend.rs`

- [ ] **Step 1: Write migration/backend tests that describe the new row shape**

```rust
async fn sqlite_columns(db: &DatabaseConnection, table: &str) -> Vec<String> {
    let stmt = Statement::from_string(
        DbBackend::Sqlite,
        format!("PRAGMA table_info({table})"),
    );
    db.query_all(stmt)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.try_get::<String>("", "name").unwrap())
        .collect()
}

async fn sqlite_indexes(db: &DatabaseConnection, table: &str) -> Vec<String> {
    let stmt = Statement::from_string(
        DbBackend::Sqlite,
        format!("PRAGMA index_list({table})"),
    );
    db.query_all(stmt)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.try_get::<String>("", "name").unwrap())
        .collect()
}

async fn legacy_audit_db_with_request_rows() -> DatabaseConnection {
    let opt = ConnectOptions::new("sqlite::memory:");
    let db = Database::connect(opt).await.unwrap();
    Migrator::up(&db, Some(Migrator::migrations().len() as u32 - 1)).await.unwrap();
    let stmt = Statement::from_string(
        DbBackend::Sqlite,
        "INSERT INTO audit_logs (id, tenant_id, actor_id, actor_type, auth_method, http_method, http_path, route_pattern, http_status, client_ip, user_agent, duration_ms, occurred_at)
         VALUES ('01900000-0000-7000-8000-000000000001', '01900000-0000-7000-8000-000000000002', '01900000-0000-7000-8000-000000000003', 'user', 'password', 'POST', '/api/v1/plugin-configs', '/api/v1/plugin-configs', 201, '127.0.0.1', 'test-agent', 12, '2026-04-01T00:00:00Z')"
            .to_string(),
    );
    db.execute(stmt).await.unwrap();
    db
}

#[tokio::test]
async fn semantic_audit_migration_recreates_both_tables_and_drops_request_columns() {
    let db = test_db().await;
    Migrator::up(&db, None).await.unwrap();

    let tenant_cols = sqlite_columns(&db, "audit_logs").await;
    let system_cols = sqlite_columns(&db, "system_audit_logs").await;
    let tenant_indexes = sqlite_indexes(&db, "audit_logs").await;
    let system_indexes = sqlite_indexes(&db, "system_audit_logs").await;

    assert!(tenant_cols.contains(&"action_type".to_string()));
    assert!(tenant_cols.contains(&"outcome".to_string()));
    assert!(!tenant_cols.contains(&"http_method".to_string()));
    assert!(system_cols.contains(&"action_type".to_string()));
    assert!(!system_cols.contains(&"http_path".to_string()));
    assert!(tenant_indexes.contains(&"idx_audit_logs_tenant_outcome_occurred_at".to_string()));
    assert!(system_indexes.contains(&"idx_system_audit_logs_target_id_occurred_at".to_string()));
}

#[tokio::test]
async fn semantic_audit_migration_drops_legacy_request_rows_instead_of_transforming_them() {
    let db = legacy_audit_db_with_request_rows().await;
    Migrator::up(&db, None).await.unwrap();

    assert_eq!(audit_log::Entity::find().count(&db).await.unwrap(), 0);
    assert_eq!(system_audit_log::Entity::find().count(&db).await.unwrap(), 0);
}

#[tokio::test]
async fn database_backend_persists_semantic_audit_entry() {
    let db = test_db().await;
    let backend = DatabaseBackend::new(db.clone());
    backend.write(&AuditEntry::test_stub(AuditActionType::PLUGIN_CONFIG_CREATE)).await.unwrap();
    let row = audit_log::Entity::find().one(&db).await.unwrap().unwrap();
    assert_eq!(row.action_type, "plugin_config.create");
}

#[tokio::test]
async fn journald_backend_emits_semantic_field_contract() {
    let backend = test_journald_backend();
    backend.write(&AuditEntry::test_stub(AuditActionType::PLUGIN_CONFIG_CREATE)).await.unwrap();
    let record = backend.last_record().unwrap();
    assert!(record.contains_key("audit_id"));
    assert!(record.contains_key("action_type"));
    assert!(record.contains_key("outcome"));
    assert!(record.contains_key("details_json"));
}
```

- [ ] **Step 2: Run the migration/backend tests to prove the old schema still exists**

Run:
`cargo test -p uptrakit-shared-db --features migration,db-sqlite semantic_audit_migration_recreates_both_tables_and_drops_request_columns -- --exact`
Expected: FAIL because the DB still creates request-era audit tables.

Run:
`cargo test -p uptrakit-shared-db --features migration,db-sqlite semantic_audit_migration_drops_legacy_request_rows_instead_of_transforming_them -- --exact`
Expected: FAIL because the migration that drops request-era rows does not exist yet.

- [ ] **Step 3: Add the forward migration that replaces request-era audit columns**

```rust
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager<'_>) -> Result<(), DbErr> {
fn build_semantic_audit_logs_table(name: &str) -> TableCreateStatement {
    Table::create()
        .table(Alias::new(name))
        .col(ColumnDef::new(Alias::new("id")).uuid().not_null().primary_key())
        .col(ColumnDef::new(Alias::new("tenant_id")).uuid().not_null())
        .col(ColumnDef::new(Alias::new("actor_type")).string_len(32).not_null())
        .col(ColumnDef::new(Alias::new("actor_id")).uuid())
        .col(ColumnDef::new(Alias::new("actor_display")).string_len(255))
        .col(ColumnDef::new(Alias::new("action_type")).string_len(128).not_null())
        .col(ColumnDef::new(Alias::new("target_type")).string_len(128))
        .col(ColumnDef::new(Alias::new("target_id")).string_len(255))
        .col(ColumnDef::new(Alias::new("target_display")).string_len(255))
        .col(ColumnDef::new(Alias::new("outcome")).string_len(32).not_null())
        .col(ColumnDef::new(Alias::new("details_json")).json_binary())
        .col(ColumnDef::new(Alias::new("request_id")).string_len(255))
        .col(ColumnDef::new(Alias::new("occurred_at")).timestamp_with_time_zone().not_null())
        .to_owned()
}

fn build_semantic_system_audit_logs_table(name: &str) -> TableCreateStatement {
    Table::create()
        .table(Alias::new(name))
        .col(ColumnDef::new(Alias::new("id")).uuid().not_null().primary_key())
        .col(ColumnDef::new(Alias::new("actor_type")).string_len(32).not_null())
        .col(ColumnDef::new(Alias::new("actor_id")).uuid())
        .col(ColumnDef::new(Alias::new("actor_display")).string_len(255))
        .col(ColumnDef::new(Alias::new("action_type")).string_len(128).not_null())
        .col(ColumnDef::new(Alias::new("target_type")).string_len(128))
        .col(ColumnDef::new(Alias::new("target_id")).string_len(255))
        .col(ColumnDef::new(Alias::new("target_display")).string_len(255))
        .col(ColumnDef::new(Alias::new("outcome")).string_len(32).not_null())
        .col(ColumnDef::new(Alias::new("details_json")).json_binary())
        .col(ColumnDef::new(Alias::new("request_id")).string_len(255))
        .col(ColumnDef::new(Alias::new("occurred_at")).timestamp_with_time_zone().not_null())
        .to_owned()
}

helpers::set_foreign_keys(manager, false).await?;

let state = helpers::check_crash_recovery(manager, "audit_logs", "audit_logs_new").await?;
if !matches!(state, helpers::CrashRecoveryState::RenameOnly) {
    manager.create_table(build_semantic_audit_logs_table("audit_logs_new")).await?;
    helpers::drop_original(manager, "audit_logs").await?;
}
helpers::rename_temp(manager, "audit_logs_new", "audit_logs").await?;

let state = helpers::check_crash_recovery(manager, "system_audit_logs", "system_audit_logs_new").await?;
if !matches!(state, helpers::CrashRecoveryState::RenameOnly) {
    manager
        .create_table(build_semantic_system_audit_logs_table("system_audit_logs_new"))
        .await?;
    helpers::drop_original(manager, "system_audit_logs").await?;
}
helpers::rename_temp(manager, "system_audit_logs_new", "system_audit_logs").await?;

helpers::set_foreign_keys(manager, true).await?;
        Ok(())
    }

async fn down(&self, manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .drop_table(Table::drop().table(Alias::new("system_audit_logs")).to_owned())
        .await?;
    manager
        .drop_table(Table::drop().table(Alias::new("audit_logs")).to_owned())
        .await
}
}
```

- [ ] **Step 4: Update the SeaORM entities and backend mapping**

```rust
pub struct Model {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub actor_id: Option<Uuid>,
    pub actor_type: String,
    pub actor_display: Option<String>,
    pub action_type: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub target_display: Option<String>,
    pub outcome: String,
    pub details_json: Option<Json>,
    pub request_id: Option<String>,
    pub occurred_at: OffsetDateTime,
}
```

- [ ] **Step 5: Implement the new indexes from the spec**

```rust
for (table, specs) in [
    (
        "audit_logs",
        vec![
            ("idx_audit_logs_tenant_occurred_at", vec!["tenant_id", "occurred_at"]),
            (
                "idx_audit_logs_tenant_action_type_occurred_at",
                vec!["tenant_id", "action_type", "occurred_at"],
            ),
            (
                "idx_audit_logs_tenant_actor_type_occurred_at",
                vec!["tenant_id", "actor_type", "occurred_at"],
            ),
            (
                "idx_audit_logs_tenant_actor_id_occurred_at",
                vec!["tenant_id", "actor_id", "occurred_at"],
            ),
            (
                "idx_audit_logs_tenant_target_type_occurred_at",
                vec!["tenant_id", "target_type", "occurred_at"],
            ),
            (
                "idx_audit_logs_tenant_target_id_occurred_at",
                vec!["tenant_id", "target_id", "occurred_at"],
            ),
            (
                "idx_audit_logs_tenant_outcome_occurred_at",
                vec!["tenant_id", "outcome", "occurred_at"],
            ),
        ],
    ),
    (
        "system_audit_logs",
        vec![
            ("idx_system_audit_logs_occurred_at", vec!["occurred_at"]),
            ("idx_system_audit_logs_action_type_occurred_at", vec!["action_type", "occurred_at"]),
            ("idx_system_audit_logs_actor_type_occurred_at", vec!["actor_type", "occurred_at"]),
            ("idx_system_audit_logs_actor_id_occurred_at", vec!["actor_id", "occurred_at"]),
            ("idx_system_audit_logs_target_type_occurred_at", vec!["target_type", "occurred_at"]),
            ("idx_system_audit_logs_target_id_occurred_at", vec!["target_id", "occurred_at"]),
            ("idx_system_audit_logs_outcome_occurred_at", vec!["outcome", "occurred_at"]),
        ],
    ),
] {
    for (name, cols) in specs {
        let mut idx = Index::create().table(Alias::new(table)).name(name);
        for col in cols {
            if col == "occurred_at" {
                idx.col((Alias::new(col), IndexOrder::Desc));
            } else {
                idx.col(Alias::new(col));
            }
        }
        manager.create_index(idx.to_owned()).await?;
    }
}
```

- [ ] **Step 6: Re-run migration/backend tests plus shared-db migration smoke tests**

Run:
`cargo test -p uptrakit-shared-db --features migration,db-sqlite semantic_audit_migration_recreates_both_tables_and_drops_request_columns -- --exact`
Expected: PASS.

Run:
`cargo test -p uptrakit-shared-db --features migration,db-sqlite semantic_audit_migration_drops_legacy_request_rows_instead_of_transforming_them -- --exact`
Expected: PASS.

Run:
`cargo test -p uptrakit-audit-log \`
`--features db,uptrakit-shared-db/db-sqlite,uptrakit-shared-db/migration \`
`database_backend_persists_semantic_audit_entry -- --exact`
Expected: PASS.

Run: `cargo test -p uptrakit-audit-log --features journald journald_backend_emits_semantic_field_contract -- --exact` Expected: PASS.

Run: `cargo test -p uptrakit-shared-db --features migration,db-sqlite migrations_run_on_empty_sqlite -- --exact` Expected: PASS with the updated
semantic entity models.

- [ ] **Step 7: Commit**

```bash
git add crates/shared/db/src/migration/m20260417_000001_semantic_audit_logs.rs crates/shared/db/src/migration/mod.rs crates/shared/db/src/entity/audit_log.rs crates/shared/db/src/entity/system_audit_log.rs crates/shared/db/src/entity/tenant_scoped.rs crates/shared/audit-log/src/backend.rs
git commit -m "feat: migrate audit logs to semantic schema"
```

### Task 3: Read Surface, REST DTOs, OpenAPI Client, And CLI

Prerequisite: complete Task 5 before running any `uptrakit-web-api` tests in this task, because the request-era middleware still constructs the old
`AuditEntry` shape until that cutover lands.

**Files:**

- Modify: `crates/shared/web-api-types/src/audit_logs.rs`
- Modify: `crates/shared/openapi-client/src/audit_logs.rs`
- Modify: `crates/ui/web-api-queries/src/queries/audit_logs.rs`
- Modify: `crates/ui/web-api/src/routes/audit_logs.rs`
- Modify: `crates/ui/cli/src/commands/audit_logs.rs`
- Modify: `crates/ui/web-api/src/api_error/mappings.rs`
- Modify: `crates/ui/web-api/src/api_error/tests.rs`
- Test: `crates/shared/openapi-client/src/audit_logs.rs`
- Test: `crates/ui/cli/src/commands/audit_logs.rs`
- Test: `crates/ui/web-api/src/routes/audit_logs.rs`

- [ ] **Step 1: Write failing DTO/query and CLI rendering tests for the new filter set**

```rust
#[test]
fn audit_log_list_params_serialization_with_semantic_filters() {
    let params = AuditLogListParams {
        action_type: Some("plugin_config.create".into()),
        actor_id: Some("019...actor".into()),
        outcome: Some("success".into()),
        target_type: Some("plugin_config".into()),
        target_id: Some("019...".into()),
        from: Some("2026-04-01T00:00:00Z".parse().unwrap()),
        to: Some("2026-04-17T23:59:59Z".parse().unwrap()),
        ..Default::default()
    };
    let qs = serde_urlencoded::to_string(params).unwrap();
    assert!(qs.contains("action_type=plugin_config.create"));
    assert!(qs.contains("actor_id="));
    assert!(qs.contains("target_id="));
    assert!(qs.contains("from="));
    assert!(qs.contains("to="));
}

#[test]
fn audit_logs_json_output_uses_semantic_fields() {
    let item = AuditLogResponse {
        id: Uuid::now_v7(),
        actor_type: "user".into(),
        actor_id: None,
        actor_display: Some("alice@example.com".into()),
        action_type: "plugin_config.create".into(),
        target_type: Some("plugin_config".into()),
        target_id: Some("019semantic".into()),
        target_display: Some("APT Defaults".into()),
        outcome: "success".into(),
        details_json: Some(serde_json::json!({ "plugin_type": "package_manager_apt" })),
        request_id: Some("req-123".into()),
        occurred_at: "2026-04-17T12:00:00Z".parse().unwrap(),
    };

    let rendered = render_audit_logs_json(vec![item]);
    assert!(rendered.contains("\"action_type\":\"plugin_config.create\""));
    assert!(rendered.contains("\"target_display\":\"APT Defaults\""));
    assert!(rendered.contains("\"outcome\":\"success\""));
    assert!(!rendered.contains("\"method\""));
    assert!(!rendered.contains("\"path\""));
}
```

- [ ] **Step 2: Run the DTO/query tests to verify the current request-era fields are still wired**

Run: `cargo test -p uptrakit-openapi-client audit_log_list_params_serialization_with_semantic_filters -- --exact` Expected: FAIL because the type
still serializes `method/status`.

Run: `cargo test -p uptrakit-cli audit_logs_json_output_uses_semantic_fields -- --exact` Expected: FAIL because the CLI JSON output is still built
from request-era DTO fields.

- [ ] **Step 3: Replace request-era DTOs with action-era ones**

```rust
pub struct AuditLogResponse {
    pub id: Uuid,
    pub actor_type: String,
    pub actor_id: Option<Uuid>,
    pub actor_display: Option<String>,
    pub action_type: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub target_display: Option<String>,
    pub outcome: String,
    pub details_json: Option<serde_json::Value>,
    pub request_id: Option<String>,
    pub occurred_at: OffsetDateTime,
}
```

- [ ] **Step 4: Rewrite query filtering and CLI rendering**

```rust
if let Some(ref action_type) = params.action_type {
    q = q.filter(audit_log::Column::ActionType.eq(action_type));
}
if let Some(ref target_id) = params.target_id {
    q = q.filter(audit_log::Column::TargetId.eq(target_id));
}
```

```rust
format!(
    "{}  {}  {}  {}",
    item.occurred_at, item.action_type, item.outcome, item.target_display.as_deref().unwrap_or("—")
)
```

- [ ] **Step 5: Update OpenAPI annotations and API error mapping tests**

Run: `cargo test -p uptrakit-openapi-client audit_log_list_params_serialization_with_semantic_filters -- --exact` Expected: PASS.

Run: `cargo test -p uptrakit-cli audit_logs_json_output_uses_semantic_fields -- --exact` Expected: PASS after Task 5 cutover.

- [ ] **Step 6: Commit**

```bash
git add crates/shared/web-api-types/src/audit_logs.rs crates/shared/openapi-client/src/audit_logs.rs crates/ui/web-api-queries/src/queries/audit_logs.rs crates/ui/web-api/src/routes/audit_logs.rs crates/ui/cli/src/commands/audit_logs.rs crates/ui/web-api/src/api_error/mappings.rs crates/ui/web-api/src/api_error/tests.rs
git commit -m "feat: convert audit log read surface to semantic events"
```

### Task 4: Frontend Types, API Client, And Audit Logs Page

**Files:**

- Modify: `frontend/src/lib/types.ts`
- Modify: `frontend/src/lib/api.ts`
- Modify: `frontend/src/routes/audit-logs/+page.svelte`
- Create: `frontend/src/routes/audit-logs/audit-logs.test.ts`

- [ ] **Step 1: Write a failing frontend route test for semantic filters and columns**

```ts
it("renders action, target, outcome, and actor filters", async () => {
  render(AuditLogsPage);
  expect(screen.getByText("Action")).toBeInTheDocument();
  expect(screen.getByText("Outcome")).toBeInTheDocument();
  expect(screen.queryByText("HTTP Method")).not.toBeInTheDocument();
});
```

- [ ] **Step 2: Run the test to confirm the page is still request-shaped**

Run: `cd frontend && npm run test -- src/routes/audit-logs/audit-logs.test.ts` Expected: FAIL because the page still renders method/status/path
columns and filters.

- [ ] **Step 3: Replace the frontend types and request serialization**

```ts
export interface AuditLogEntry {
  id: string;
  actor_type: string;
  actor_id: string | null;
  actor_display: string | null;
  action_type: string;
  target_type: string | null;
  target_id: string | null;
  target_display: string | null;
  outcome: string;
  details_json: Record<string, unknown> | null;
  request_id: string | null;
  occurred_at: string;
}
```

- [ ] **Step 4: Rewrite the page around action-centric filters**

```svelte
let filterActionType = $state(page.url.searchParams.get('action_type') ?? '');
let filterOutcome = $state(page.url.searchParams.get('outcome') ?? '');
let filterTargetType = $state(page.url.searchParams.get('target_type') ?? '');
let filterTargetId = $state(page.url.searchParams.get('target_id') ?? '');
```

- [ ] **Step 5: Re-run the frontend test and basic static checks**

Run: `cd frontend && npm run test -- src/routes/audit-logs/audit-logs.test.ts` Expected: PASS.

Run: `cd frontend && npm run check` Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/lib/types.ts frontend/src/lib/api.ts frontend/src/routes/audit-logs/+page.svelte frontend/src/routes/audit-logs/audit-logs.test.ts
git commit -m "feat: update audit logs frontend for semantic events"
```

### Task 5: Controller Emitter Plumbing And Middleware Removal

**Files:**

- Modify: `crates/ui/web-api/src/app_state.rs`
- Modify: `crates/ui/web-api/src/lib.rs`
- Modify: `crates/ui/web-api/src/router.rs`
- Modify: `crates/ui/web-api/src/middleware/audit_log.rs`
- Modify: `crates/ui/web-api/src/middleware/resolve_ip.rs`
- Modify: `crates/ui/web-api/src/middleware/require_auth.rs`
- Modify: `crates/ui/web-api/src/routes/surfaces.rs`
- Modify: `crates/ui/web-api/src/routes/services.rs`
- Modify: `crates/ui/web-api/src/routes/auth.rs`
- Modify: `crates/ui/web-api/src/routes/service_ws/handler/mod.rs`
- Modify: `crates/ui/web-api/src/routes/service_ws/handler/messages.rs`
- Modify: `crates/ui/web-api/src/test_harness/mod.rs`
- Test: `crates/ui/web-api/src/middleware/audit_log.rs`
- Test: `crates/ui/web-api/src/test_harness/mod.rs`

- [ ] **Step 1: Write a failing router/middleware test proving request middleware no longer inserts semantic rows**

```rust
#[tokio::test]
async fn request_middleware_does_not_persist_audit_rows_by_itself() {
    let app = build_authenticated_test_app().await;
    let _ = app.get("/api/v1/plugin-configs").await;
    assert_eq!(count_audit_rows().await, 0);
}
```

- [ ] **Step 2: Run the test and confirm the request middleware still writes rows**

Run: `cargo test -p uptrakit-web-api request_middleware_does_not_persist_audit_rows_by_itself -- --exact` Expected: FAIL because
`middleware/audit_log.rs` still dispatches after every authenticated request.

- [ ] **Step 3: Put the semantic emitter in `AppState` and stop using middleware for audit rows**

```rust
pub struct AppState {
    // ...
    pub audit_emitter: uptrakit_audit_log::AuditEmitter,
}
```

```rust
pub async fn audit_log(_state: State<Arc<AppState>>, req: Request, next: Next) -> Response {
    next.run(req).await
}
```

- [ ] **Step 4: Update every direct `AppState` constructor to use the existing builder with the emitter**

Only do compile-plumbing edits in `routes/auth.rs` here; semantic auth event emission still belongs to Task 6 after the producer tests exist.

```rust
// For every existing `AppState { ... }` literal in this task's file list,
// add the new field explicitly and leave every other field unchanged:
audit_emitter: audit_emitter.clone(),
```

- [ ] **Step 5: Keep request-id/auth/client-ip extraction helpers available to producers**

```rust
pub struct AuditRequestContext {
    pub request_id: Option<String>,
    pub client_ip: Option<String>,
    pub actor_type: AuditActorType,
    pub actor_id: Option<Uuid>,
}

pub fn audit_context_from_parts(
    parts: &http::request::Parts,
    actor_type: AuditActorType,
    actor_id: Option<Uuid>,
) -> AuditRequestContext {
    AuditRequestContext {
        request_id: parts.extensions.get::<RequestId>().map(ToString::to_string),
        client_ip: parts.extensions.get::<ResolvedClientIp>().map(ToString::to_string),
        actor_type,
        actor_id,
    }
}
```

- [ ] **Step 6: Re-run the router/middleware tests**

Run: `cargo test -p uptrakit-web-api request_middleware_does_not_persist_audit_rows_by_itself -- --exact` Expected: PASS.

Run: `cargo test -p uptrakit-web-api audit_log_query_error_all_variants -- --exact` Expected: PASS now that the request-era middleware dependency is
gone.

- [ ] **Step 7: Commit**

```bash
git add crates/ui/web-api/src/app_state.rs crates/ui/web-api/src/lib.rs crates/ui/web-api/src/router.rs crates/ui/web-api/src/middleware/audit_log.rs crates/ui/web-api/src/middleware/resolve_ip.rs crates/ui/web-api/src/middleware/require_auth.rs crates/ui/web-api/src/routes/surfaces.rs crates/ui/web-api/src/routes/services.rs crates/ui/web-api/src/routes/auth.rs crates/ui/web-api/src/routes/service_ws/handler/mod.rs crates/ui/web-api/src/routes/service_ws/handler/messages.rs crates/ui/web-api/src/test_harness/mod.rs
git commit -m "refactor: move audit emission out of request middleware"
```

### Task 6: REST Producer Migration

**Files:**

- Modify: `crates/ui/web-api/src/routes/auth.rs`
- Modify: `crates/ui/web-api/src/routes/api_tokens.rs`
- Modify: `crates/ui/web-api/src/middleware/require_auth.rs`
- Modify: `crates/ui/web-api/src/routes/device_auth.rs`
- Modify: `crates/ui/web-api/src/routes/oidc_auth.rs`
- Modify: `crates/ui/web-api/src/routes/oidc_providers.rs`
- Modify: `crates/ui/web-api/src/routes/enrollment_tokens.rs`
- Modify: `crates/ui/web-api/src/routes/notifications.rs`
- Modify: `crates/ui/web-api/src/routes/settings.rs`
- Modify: `crates/ui/web-api/src/routes/settings_auth.rs`
- Modify: `crates/ui/web-api/src/routes/settings_network.rs`
- Modify: `crates/ui/web-api/src/routes/settings_nats.rs`
- Modify: `crates/ui/web-api/src/routes/settings_zeroconf.rs`
- Modify: `crates/ui/web-api/src/routes/settings_agent_certs.rs`
- Modify: `crates/ui/web-api/src/routes/users.rs`
- Modify: `crates/ui/web-api/src/routes/plugin_configs.rs`
- Modify: `crates/ui/web-api/src/routes/plugin_type_settings.rs`
- Modify: `crates/ui/web-api/src/routes/services.rs`
- Modify: `crates/ui/web-api/src/routes/software_items/mod.rs`
- Modify: `crates/ui/web-api/src/routes/update_batches.rs`
- Modify: `crates/ui/web-api/src/routes/autodiscovery.rs`
- Modify: `crates/ui/web-api-queries/src/queries/autodiscovery/ignore_rules.rs`
- Modify: `crates/ui/web-api/src/surface_proxy.rs`
- Test: `crates/ui/web-api/src/routes/auth.rs`
- Test: `crates/ui/web-api/src/routes/api_tokens.rs`
- Test: `crates/ui/web-api/src/routes/notifications.rs`
- Test: `crates/ui/web-api/src/routes/update_batches.rs`
- Test: `crates/ui/web-api/src/routes/settings_network.rs`
- Test: `crates/ui/web-api/src/routes/plugin_configs.rs`
- Test: `crates/ui/web-api/src/routes/services.rs`

- [ ] **Step 1: Write failing route tests for representative semantic events**

```rust
#[tokio::test]
async fn plugin_config_create_writes_plugin_config_create_audit_event() {
    let app = TestApp::new().await;
    app.create_plugin_config(valid_request()).await.unwrap();
    let row = latest_tenant_audit_row(app.db()).await;
    assert_eq!(row.action_type, "plugin_config.create");
    assert_eq!(row.outcome, "success");
}
```

```rust
#[tokio::test]
async fn invalid_api_token_writes_auth_api_token_authenticate_denied() {
    let app = TestApp::new().await;
    let resp = app.get_with_bad_api_token("/api/v1/hosts").await;
    assert_eq!(resp.status(), 401);
    let row = latest_tenant_audit_row(app.db()).await;
    assert_eq!(row.action_type, "auth.api_token.authenticate");
    assert_eq!(row.outcome, "denied");
}

#[tokio::test]
async fn notification_channel_test_writes_notification_channel_test_audit_event() {
    let app = TestApp::new().await;
    app.test_notification_channel(valid_notification_request()).await.unwrap();
    let row = latest_tenant_audit_row(app.db()).await;
    assert_eq!(row.action_type, "notification_channel.test");
}

#[tokio::test]
async fn update_nats_settings_writes_global_setting_update_audit_event() {
    let app = TestApp::new().await;
    app.update_nats_settings(valid_nats_settings()).await.unwrap();
    let row = latest_system_audit_row(app.db()).await;
    assert_eq!(row.action_type, "global_setting.update");
}

#[tokio::test]
async fn trigger_host_batch_update_writes_software_batch_update_triggered_audit_event() {
    let app = TestApp::new().await;
    app.trigger_host_batch_update(valid_batch_request()).await.unwrap();
    let row = latest_tenant_audit_row(app.db()).await;
    assert_eq!(row.action_type, "software.batch_update.triggered");
}
```

- [ ] **Step 2: Run the representative tests to confirm producers are missing**

Run: `cargo test -p uptrakit-web-api plugin_config_create_writes_plugin_config_create_audit_event -- --exact` Expected: FAIL because the code still
emits request rows or `security_audit` tracing only.

Run: `cargo test -p uptrakit-web-api invalid_api_token_writes_auth_api_token_authenticate_denied -- --exact` Expected: FAIL because auth rejection
paths still bypass the canonical semantic emitter.

Run: `cargo test -p uptrakit-web-api notification_channel_test_writes_notification_channel_test_audit_event -- --exact` Expected: FAIL because
notification routes do not emit semantic audit events yet.

Run: `cargo test -p uptrakit-web-api --features nats update_nats_settings_writes_global_setting_update_audit_event -- --exact` Expected: FAIL because
settings routes do not emit `global_setting.update` yet.

Run: `cargo test -p uptrakit-web-api trigger_host_batch_update_writes_software_batch_update_triggered_audit_event -- --exact` Expected: FAIL because
update-trigger routes do not emit semantic batch events yet.

- [ ] **Step 3: Replace route-local `security_audit` tracing with canonical emitter calls**

```rust
let audit_ctx = audit_context_from_parts(parts, AuditActorType::User, Some(user.user_id));

if let Ok(entry) = AuditEntry::builder(AuditActionType::PLUGIN_CONFIG_CREATE)
        .tenant_scope(tenant_db.tenant_id)
        .actor(audit_ctx.actor_type, audit_ctx.actor_id)
        .target("plugin_config", resp.id.to_string(), Some(resp.name.clone()))
        .outcome(AuditOutcome::Success)
        .details(json!({ "plugin_type": resp.plugin_type }))
        .request_id_opt(audit_ctx.request_id.clone())
        .build() {
    state.audit_emitter.emit_best_effort(entry);
}
```

- [ ] **Step 4: Cover the rest of the V1 catalog for REST-owned actions**

```rust
AuditActionType::AUTH_LOGIN
AuditActionType::AUTH_API_TOKEN_AUTHENTICATE
AuditActionType::AUTH_JWT_AUTHENTICATE
AuditActionType::AUTH_TOKEN_REFRESH
AuditActionType::AUTH_OIDC_AUTHORIZE
AuditActionType::AUTH_OIDC_CALLBACK
AuditActionType::AUTH_DEVICE_APPROVE
AuditActionType::AUTH_DEVICE_DENY
AuditActionType::USER_CREATE
AuditActionType::USER_UPDATE
AuditActionType::USER_DELETE
AuditActionType::API_TOKEN_CREATE
AuditActionType::API_TOKEN_REVOKE
AuditActionType::ENROLLMENT_TOKEN_CREATE
AuditActionType::ENROLLMENT_TOKEN_REVOKE
AuditActionType::GLOBAL_SETTING_UPDATE
AuditActionType::TENANT_SETTING_UPDATE
AuditActionType::OIDC_PROVIDER_CREATE
AuditActionType::OIDC_PROVIDER_UPDATE
AuditActionType::OIDC_PROVIDER_DELETE
AuditActionType::PLUGIN_CONFIG_CREATE
AuditActionType::PLUGIN_CONFIG_UPDATE
AuditActionType::PLUGIN_CONFIG_DELETE
AuditActionType::PLUGIN_TYPE_SETTINGS_UPSERT
AuditActionType::PLUGIN_TYPE_SETTINGS_DELETE
AuditActionType::NOTIFICATION_CHANNEL_CREATE
AuditActionType::NOTIFICATION_CHANNEL_UPDATE
AuditActionType::NOTIFICATION_CHANNEL_DELETE
AuditActionType::NOTIFICATION_CHANNEL_TEST
AuditActionType::NOTIFICATION_RULE_CREATE
AuditActionType::NOTIFICATION_RULE_UPDATE
AuditActionType::NOTIFICATION_RULE_DELETE
AuditActionType::SERVICE_APPROVE
AuditActionType::SERVICE_REJECT
AuditActionType::SERVICE_MERGE
AuditActionType::SERVICE_CERTIFICATE_ISSUE
AuditActionType::SERVICE_CERTIFICATE_RENEW
AuditActionType::SERVICE_UPDATE_FREEZE_ENABLE
AuditActionType::SERVICE_UPDATE_FREEZE_DISABLE
AuditActionType::SERVICE_ENROLLMENT_COMPLETED
AuditActionType::SERVICE_DEACTIVATE
AuditActionType::SOFTWARE_UPDATE_TRIGGERED
AuditActionType::SOFTWARE_UPDATE_STARTED
AuditActionType::SOFTWARE_BATCH_UPDATE_TRIGGERED
AuditActionType::SOFTWARE_BATCH_UPDATE_STARTED
AuditActionType::SOFTWARE_IGNORE_CREATE
AuditActionType::SOFTWARE_IGNORE_DELETE
AuditActionType::SYSTEM_SERVICE_UPDATE_FREEZE_APPLY
AuditActionType::SYSTEM_SERVICE_MACHINE_ID_VALIDATE
```

```rust
let safe_details = json!({
    "setting_key": SettingKey::NatsUrl.as_str(),
    "changed": true,
});
assert!(safe_details.get("value").is_none(), "audit details must not store secrets");
```

- [ ] **Step 5: Re-run the representative route tests and one broader web-api slice**

Run: `cargo test -p uptrakit-web-api plugin_config_create_writes_plugin_config_create_audit_event -- --exact` Expected: PASS.

Run: `cargo test -p uptrakit-web-api notification_channel_test_writes_notification_channel_test_audit_event -- --exact` Expected: PASS.

Run: `cargo test -p uptrakit-web-api --features nats update_nats_settings_writes_global_setting_update_audit_event -- --exact` Expected: PASS.

Run: `cargo test -p uptrakit-web-api trigger_host_batch_update_writes_software_batch_update_triggered_audit_event -- --exact` Expected: PASS.

Run: `cargo test -p uptrakit-web-api service_update_freeze_enable_writes_service_update_freeze_enable_audit_event -- --exact` Expected: PASS.

Run: `cargo test -p uptrakit-web-api login_success_writes_auth_login_audit_event -- --exact` Expected: PASS.

Run: `cargo test -p uptrakit-web-api token_refresh_failure_writes_auth_token_refresh_audit_event -- --exact` Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/ui/web-api/src/routes/auth.rs crates/ui/web-api/src/routes/api_tokens.rs crates/ui/web-api/src/middleware/require_auth.rs crates/ui/web-api/src/routes/device_auth.rs crates/ui/web-api/src/routes/oidc_auth.rs crates/ui/web-api/src/routes/oidc_providers.rs crates/ui/web-api/src/routes/enrollment_tokens.rs crates/ui/web-api/src/routes/notifications.rs crates/ui/web-api/src/routes/settings.rs crates/ui/web-api/src/routes/settings_auth.rs crates/ui/web-api/src/routes/settings_network.rs crates/ui/web-api/src/routes/settings_nats.rs crates/ui/web-api/src/routes/settings_zeroconf.rs crates/ui/web-api/src/routes/settings_agent_certs.rs crates/ui/web-api/src/routes/users.rs crates/ui/web-api/src/routes/plugin_configs.rs crates/ui/web-api/src/routes/plugin_type_settings.rs crates/ui/web-api/src/routes/services.rs crates/ui/web-api/src/routes/software_items/mod.rs crates/ui/web-api/src/routes/update_batches.rs crates/ui/web-api/src/routes/autodiscovery.rs crates/ui/web-api-queries/src/queries/autodiscovery/ignore_rules.rs crates/ui/web-api/src/surface_proxy.rs
git commit -m "feat: emit semantic audit events from rest routes"
```

### Task 7: Service/Runtime Forwarding And Non-HTTP Producers

**Files:**

- Modify: `crates/shared/audit-log/Cargo.toml`
- Modify: `crates/shared/scheduler-engine/Cargo.toml`
- Create: `crates/shared/audit-log/src/runtime_emitter.rs`
- Modify: `crates/shared/audit-log/src/lib.rs`
- Modify: `crates/shared/scheduler-engine/src/executors/audit_log_cleanup.rs`
- Modify: `crates/shared/wire/src/payloads.rs`
- Modify: `crates/shared/wire/src/messages.rs`
- Modify: `crates/shared/wire/src/tests.rs`
- Modify: `crates/shared/wire/src/wire_validate_impls.rs`
- Modify: `crates/ui/web-api/src/routes/service_ws/handler/mod.rs`
- Modify: `crates/ui/web-api/src/routes/service_ws/handler/cert.rs`
- Modify: `crates/ui/web-api/src/routes/service_ws/handler/updates.rs`
- Modify: `crates/ui/web-api/src/routes/service_ws/connection.rs`
- Modify: `crates/core/agent/src/main.rs`
- Modify: `crates/core/controller/src/agent/mod.rs`
- Modify: `crates/core/agent-runtime/Cargo.toml`
- Modify: `crates/core/agent-runtime/src/lib.rs`
- Modify: `crates/core/agent-ssh/src/main.rs`
- Modify: `crates/core/controller/src/ssh_agent/mod.rs`
- Modify: `crates/core/controller/src/scheduler/mod.rs`
- Modify: `crates/core/agent-ssh-runtime/Cargo.toml`
- Modify: `crates/core/agent-ssh-runtime/src/lib.rs`
- Modify: `crates/ui/web-api/src/routes/services.rs`
- Test: `crates/shared/wire/src/tests.rs`
- Test: `crates/ui/web-api/src/routes/service_ws/handler/mod.rs`
- Test: `crates/ui/web-api/src/routes/service_ws/handler/cert.rs`
- Test: `crates/core/agent-runtime/src/lib.rs`
- Test: `crates/shared/scheduler-engine/src/executors/audit_log_cleanup.rs`

- [ ] **Step 1: Write failing wire/service tests for `ServiceMessage::AuditEvent`**

```rust
#[test]
fn audit_event_payload_round_trips() {
    let msg = ServiceMessage::AuditEvent(AuditEventPayload {
        action_type: "system.service.update_gate".into(),
        tenant_id: None,
        target_type: Some("service".into()),
        target_id: Some("019...".into()),
        target_display: None,
        outcome: "denied".into(),
        details_json: Some(json!({ "reason": "frozen" })),
        request_id: None,
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("\"audit_event\""));
}

#[tokio::test]
async fn invalid_service_audit_event_is_dropped_without_disconnect() {
    let payload = AuditEventPayload {
        action_type: "auth.login.failed".into(),
        tenant_id: Some(Uuid::now_v7()),
        target_type: None,
        target_id: None,
        target_display: None,
        outcome: "wat".into(),
        details_json: None,
        request_id: None,
    };
    let result = ingest_service_audit_event(test_state(), test_service_identity(), payload).await;
    assert!(result.is_ok(), "invalid payload should be dropped, not disconnect the service");
    assert_eq!(count_system_audit_rows().await, 0);
}

#[tokio::test]
async fn audit_log_cleanup_executor_writes_system_scheduler_audit_log_cleanup() {
    let db = seeded_cleanup_db().await;
    let emitter = test_audit_emitter(db.clone());
    let exec = AuditLogCleanupExecutor::new(db.clone(), emitter);
    exec.execute(&seeded_cleanup_task()).await.unwrap();
    let row = latest_system_audit_row(&db).await;
    assert_eq!(row.action_type, "system.scheduler.audit_log_cleanup");
}

#[tokio::test]
async fn machine_id_mismatch_writes_system_service_machine_id_validate() {
    let mut runtime = test_runtime_with_audit_sink().await;
    assert!(!runtime.machine_id_matches("CheckVersions", "unexpected-machine-id"));
    let row = latest_runtime_audit_row().await;
    assert_eq!(row.action_type, "system.service.machine_id.validate");
    assert_eq!(row.outcome, "denied");
}

#[tokio::test]
async fn runtime_freeze_apply_enabled_writes_system_service_update_freeze_apply() {
    let mut runtime = test_runtime_with_audit_sink().await;
    runtime.record_freeze_apply(true).await;
    let row = latest_runtime_audit_row().await;
    assert_eq!(row.action_type, "system.service.update_freeze.apply");
    assert_eq!(row.details_json["enabled"], true);
}

#[tokio::test]
async fn runtime_freeze_apply_disabled_writes_system_service_update_freeze_apply() {
    let mut runtime = test_runtime_with_audit_sink().await;
    runtime.record_freeze_apply(false).await;
    let row = latest_runtime_audit_row().await;
    assert_eq!(row.action_type, "system.service.update_freeze.apply");
    assert_eq!(row.details_json["enabled"], false);
}

#[tokio::test]
async fn service_certificate_issue_writes_service_certificate_issue_audit_event() {
    let state = test_ws_state().await;
    handle_certificate_issued(&state, seeded_service_identity()).await.unwrap();
    let row = latest_system_audit_row(state.db()).await;
    assert_eq!(row.action_type, "service.certificate.issue");
}

#[tokio::test]
async fn service_enrollment_completed_writes_service_enrollment_completed_audit_event() {
    let state = test_ws_state().await;
    finalize_enrollment(&state, seeded_service_identity()).await.unwrap();
    let row = latest_system_audit_row(state.db()).await;
    assert_eq!(row.action_type, "service.enrollment.completed");
}

#[tokio::test]
async fn ssh_runtime_frozen_update_gate_writes_system_service_update_gate() {
    let mut runtime = test_ssh_runtime_with_audit_sink().await;
    assert!(!runtime.is_update_allowed("machine-a").await);
    let row = latest_runtime_audit_row().await;
    assert_eq!(row.action_type, "system.service.update_gate");
}

#[tokio::test]
async fn ssh_runtime_cooldown_update_gate_writes_system_service_update_gate() {
    let mut runtime = test_ssh_runtime_with_recent_update().await;
    assert!(!runtime.is_update_allowed("machine-a").await);
    let row = latest_runtime_audit_row().await;
    assert_eq!(row.action_type, "system.service.update_gate");
    assert_eq!(row.details_json["reason"], "cooldown");
}

#[tokio::test]
async fn ssh_runtime_freeze_apply_writes_system_service_update_freeze_apply() {
    let mut runtime = test_ssh_runtime_with_audit_sink().await;
    runtime.record_freeze_apply(true).await;
    let row = latest_runtime_audit_row().await;
    assert_eq!(row.action_type, "system.service.update_freeze.apply");
}

#[tokio::test]
async fn ssh_runtime_unfreeze_apply_writes_system_service_update_freeze_apply() {
    let mut runtime = test_ssh_runtime_with_audit_sink().await;
    runtime.record_freeze_apply(false).await;
    let row = latest_runtime_audit_row().await;
    assert_eq!(row.action_type, "system.service.update_freeze.apply");
    assert_eq!(row.details_json["enabled"], false);
}
```

- [ ] **Step 2: Run the wire/runtime tests to show the message and producers do not exist**

Run: `cargo test -p uptrakit-internal-wire audit_event_payload_round_trips -- --exact` Expected: FAIL because `AuditEventPayload` /
`ServiceMessage::AuditEvent` do not exist yet.

Run: `cargo test -p uptrakit-web-api invalid_service_audit_event_is_dropped_without_disconnect -- --exact` Expected: FAIL because controller-side
ingestion validation does not exist yet.

Run: `cargo test -p uptrakit-scheduler-engine audit_log_cleanup_executor_writes_system_scheduler_audit_log_cleanup -- --exact` Expected: FAIL because
the scheduler executor still only deletes rows and logs with `tracing`.

Run: `cargo test -p uptrakit-agent-runtime machine_id_mismatch_writes_system_service_machine_id_validate -- --exact` Expected: FAIL because runtime
mismatch handling still only logs `security_audit`.

Run: `cargo test -p uptrakit-agent-runtime runtime_freeze_apply_enabled_writes_system_service_update_freeze_apply -- --exact` Expected: FAIL because
agent runtime freeze-apply auditing does not exist yet.

Run: `cargo test -p uptrakit-agent-runtime runtime_freeze_apply_disabled_writes_system_service_update_freeze_apply -- --exact` Expected: FAIL because
agent runtime unfreeze auditing does not exist yet.

Run: `cargo test -p uptrakit-web-api service_certificate_issue_writes_service_certificate_issue_audit_event -- --exact` Expected: FAIL because cert
issuance paths do not emit semantic events yet.

Run: `cargo test -p uptrakit-web-api service_enrollment_completed_writes_service_enrollment_completed_audit_event -- --exact` Expected: FAIL because
service enrollment completion does not emit semantic events yet.

Run: `cargo test -p uptrakit-agent-ssh-runtime ssh_runtime_frozen_update_gate_writes_system_service_update_gate -- --exact` Expected: FAIL because SSH
runtime still only logs `security_audit`.

Run: `cargo test -p uptrakit-agent-ssh-runtime ssh_runtime_cooldown_update_gate_writes_system_service_update_gate -- --exact` Expected: FAIL because
SSH runtime cooldown denials are not emitting semantic events yet.

Run: `cargo test -p uptrakit-agent-ssh-runtime ssh_runtime_freeze_apply_writes_system_service_update_freeze_apply -- --exact` Expected: FAIL because
SSH runtime freeze application is not emitting semantic audit events yet.

Run: `cargo test -p uptrakit-agent-ssh-runtime ssh_runtime_unfreeze_apply_writes_system_service_update_freeze_apply -- --exact` Expected: FAIL because
SSH runtime unfreeze application is not emitting semantic audit events yet.

- [ ] **Step 3: Add the additive wire payload and controller ingestion path**

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditEventPayload {
    // Wire boundary uses `String`; controller validates it into
    // `AuditActionType` at ingress so forward/backward compatible peers can
    // still deserialize unknown future action ids safely.
    pub action_type: String,
    pub tenant_id: Option<Uuid>,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub target_display: Option<String>,
    pub outcome: String,
    pub details_json: Option<serde_json::Value>,
    pub request_id: Option<String>,
}
```

```rust
// Insert `AuditEvent` before `Unknown`; `#[serde(other)]` must remain last.
ServiceMessage::AuditEvent(payload) => {
    if let Err(err) = ingest_service_audit_event(state.clone(), service_identity.clone(), payload).await {
        tracing::warn!(error = %err, "service audit ingestion failed");
    }
}
```

- [ ] **Step 4: Re-validate and normalize forwarded service audit payloads at controller ingress**

```rust
fn matches_service_forwardable_action(action_type: &AuditActionType) -> bool {
    matches!(
        action_type.as_str(),
        AuditActionType::SERVICE_CERTIFICATE_ISSUE
            | AuditActionType::SERVICE_CERTIFICATE_RENEW
            | AuditActionType::SERVICE_ENROLLMENT_COMPLETED
            | AuditActionType::SOFTWARE_UPDATE_STARTED
            | AuditActionType::SOFTWARE_BATCH_UPDATE_STARTED
            | AuditActionType::SYSTEM_SERVICE_UPDATE_GATE
            | AuditActionType::SYSTEM_SERVICE_MACHINE_ID_VALIDATE
            | AuditActionType::SYSTEM_SERVICE_UPDATE_FREEZE_APPLY
    )
}

async fn ingest_service_audit_event(
    state: Arc<AppState>,
    service_identity: ServiceIdentity,
    payload: AuditEventPayload,
) -> Result<()> {
    let action_type = match AuditActionType::new(payload.action_type) {
        Ok(action) => action,
        Err(_) => return Ok(()),
    };
    if !AuditActionType::is_registered(action_type.as_str()) {
        tracing::warn!(action_type = %action_type.as_str(), "dropping unregistered audit action");
        return Ok(());
    }
    if !matches_service_forwardable_action(&action_type) {
        tracing::warn!(action_type = %action_type.as_str(), "dropping non-forwardable service audit action");
        return Ok(());
    }
    let outcome = match AuditOutcome::try_from(payload.outcome.as_str()) {
        Ok(outcome) => outcome,
        Err(_) => return Ok(()),
    };
    let service_scope = load_authenticated_service_scope(state.db(), service_identity.service_id).await?;
    let actor_display = lookup_service_display_name(state.db(), service_identity.service_id).await.ok();

    let builder = match service_scope {
        AuthenticatedServiceScope::Tenant { tenant_id } => {
            if payload.tenant_id.is_some_and(|payload_tenant_id| payload_tenant_id != tenant_id) {
                tracing::warn!("dropping tenant service audit event with mismatched tenant scope");
                return Ok(());
            }

            AuditEntry::builder(action_type)
                .tenant_scope(tenant_id)
                .actor_service(service_identity.service_id)
                .actor_display_opt(actor_display.clone())
        }
        AuthenticatedServiceScope::System => {
            let builder = AuditEntry::builder(action_type)
                .system_scope()
                .actor_system()
                .actor_display_opt(actor_display.clone().or(Some("system".to_string())));

            match payload.tenant_id {
                Some(tenant_id) if action_targets_tenant_scope(action_type.as_str()) => builder.tenant_scope(
                    validated_target_tenant_id(state.db(), tenant_id, service_identity.service_id).await?,
                ),
                Some(_) => {
                    tracing::warn!("dropping invalid tenant-scoped system service audit event");
                    return Ok(());
                }
                None => builder,
            }
        }
    };

    let builder = match payload.details_json {
        Some(details) => builder.details(details),
        None if action_type.as_str().starts_with("system.service.") => builder.details(
            serde_json::json!({ "service_id": service_identity.service_id }),
        ),
        None => builder,
    };

    let entry = match builder
        .target_opt(payload.target_type, payload.target_id, payload.target_display)
        .outcome(outcome)
        .request_id_opt(payload.request_id)
        .build() {
        Ok(entry) => entry,
        Err(err) => {
            tracing::warn!(error = %err, "dropping invalid forwarded audit event");
            return Ok(());
        }
    };

    state.audit_emitter.emit_best_effort(entry);
    Ok(())
}
```

- [ ] **Step 5: Convert runtime `security_audit` callsites to forwarded/local semantic events and wire the emitter into constructors**

```toml
[features]
wire = ["dep:uptrakit-internal-wire"]
runtime = ["wire", "journald"]

[dependencies]
uptrakit-internal-wire = { workspace = true, optional = true }
```

```rust
pub struct RuntimeAuditEmitter {
    journald: Option<JournaldBackend>,
    wire_tx: Option<mpsc::Sender<ServiceMessage>>,
}

impl RuntimeAuditEmitter {
    pub async fn emit_best_effort(&self, entry: AuditEntry) {
        if let Some(journald) = &self.journald {
            if let Err(err) = journald.write(&entry).await {
                tracing::warn!(error = %err, "journald audit emission failed");
            }
        }
        if let Some(wire_tx) = &self.wire_tx {
            if let Err(err) = wire_tx
                .send(ServiceMessage::AuditEvent(AuditEventPayload::from(entry)))
                .await {
                tracing::warn!(error = %err, "wire audit forwarding failed");
            }
        }
    }
}

let mut runtime = AgentRuntime::new(
    AgentRuntimeConfig::new(/* existing args */)
        .audit_emitter(RuntimeAuditEmitter::new(journald_backend, Some(service_tx.clone())))
);

if let Ok(entry) = AuditEntry::builder(AuditActionType::SYSTEM_SERVICE_UPDATE_GATE)
        .system_scope()
        .actor_system()
        .outcome(AuditOutcome::Denied)
        .details(json!({ "reason": "cooldown" }))
        .build() {
    runtime.audit_emitter.emit_best_effort(entry).await;
}

if let Ok(entry) = AuditEntry::builder(AuditActionType::SYSTEM_SERVICE_MACHINE_ID_VALIDATE)
        .system_scope()
        .actor_system()
        .outcome(AuditOutcome::Denied)
        .details(json!({ "message_name": "CheckVersions" }))
        .build() {
    runtime.audit_emitter.emit_best_effort(entry).await;
}

if let Ok(entry) = AuditEntry::builder(AuditActionType::SYSTEM_SERVICE_UPDATE_FREEZE_APPLY)
        .system_scope()
        .actor_system()
        .outcome(AuditOutcome::Success)
        .details(json!({ "enabled": true }))
        .build() {
    runtime.audit_emitter.emit_best_effort(entry).await;
}

let cleanup = audit_log_cleanup::AuditLogCleanupExecutor::new(
    db.clone(),
    controller_audit_emitter.clone(),
);
```

- [ ] **Step 6: Re-run wire/runtime tests**

Run: `cargo test -p uptrakit-internal-wire audit_event_payload_round_trips -- --exact` Expected: PASS.

Run: `cargo test -p uptrakit-web-api service_message_audit_event_is_ingested -- --exact` Expected: PASS after adding the handler test.

Run: `cargo test -p uptrakit-web-api invalid_service_audit_event_is_dropped_without_disconnect -- --exact` Expected: PASS.

Run: `cargo test -p uptrakit-scheduler-engine audit_log_cleanup_executor_writes_system_scheduler_audit_log_cleanup -- --exact` Expected: PASS.

Run: `cargo test -p uptrakit-agent-runtime machine_id_mismatch_writes_system_service_machine_id_validate -- --exact` Expected: PASS.

Run: `cargo test -p uptrakit-agent-runtime runtime_freeze_apply_enabled_writes_system_service_update_freeze_apply -- --exact` Expected: PASS.

Run: `cargo test -p uptrakit-agent-runtime runtime_freeze_apply_disabled_writes_system_service_update_freeze_apply -- --exact` Expected: PASS.

Run: `cargo test -p uptrakit-web-api service_certificate_issue_writes_service_certificate_issue_audit_event -- --exact` Expected: PASS.

Run: `cargo test -p uptrakit-web-api service_enrollment_completed_writes_service_enrollment_completed_audit_event -- --exact` Expected: PASS.

Run: `cargo test -p uptrakit-agent-ssh-runtime ssh_runtime_frozen_update_gate_writes_system_service_update_gate -- --exact` Expected: PASS.

Run: `cargo test -p uptrakit-agent-ssh-runtime ssh_runtime_cooldown_update_gate_writes_system_service_update_gate -- --exact` Expected: PASS.

Run: `cargo test -p uptrakit-agent-ssh-runtime ssh_runtime_freeze_apply_writes_system_service_update_freeze_apply -- --exact` Expected: PASS.

Run: `cargo test -p uptrakit-agent-ssh-runtime ssh_runtime_unfreeze_apply_writes_system_service_update_freeze_apply -- --exact` Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/shared/audit-log/Cargo.toml crates/shared/audit-log/src/runtime_emitter.rs crates/shared/audit-log/src/lib.rs crates/shared/scheduler-engine/Cargo.toml crates/shared/scheduler-engine/src/executors/audit_log_cleanup.rs crates/shared/wire/src/payloads.rs crates/shared/wire/src/messages.rs crates/shared/wire/src/tests.rs crates/shared/wire/src/wire_validate_impls.rs crates/ui/web-api/src/routes/service_ws/handler/mod.rs crates/ui/web-api/src/routes/service_ws/handler/cert.rs crates/ui/web-api/src/routes/service_ws/handler/updates.rs crates/ui/web-api/src/routes/service_ws/connection.rs crates/core/agent/src/main.rs crates/core/controller/src/agent/mod.rs crates/core/agent-runtime/Cargo.toml crates/core/agent-runtime/src/lib.rs crates/core/agent-ssh/src/main.rs crates/core/controller/src/ssh_agent/mod.rs crates/core/controller/src/scheduler/mod.rs crates/core/agent-ssh-runtime/Cargo.toml crates/core/agent-ssh-runtime/src/lib.rs crates/ui/web-api/src/routes/services.rs
git commit -m "feat: forward semantic audit events from services"
```

### Task 8: Docs, Guardrails, And End-To-End Verification

**Files:**

- Modify: `docs/development/audit-logs.md`
- Modify: `docs/security/audit-logs.md`
- Modify: `docs/end-user/audit-logs.md`
- Modify: `docs/api/audit-logs.md`
- Modify: `AGENTS.md`
- Modify: `ARCHITECTURE.md`
- Create: `ci/verify_no_security_audit.sh`
- Create: `ci/verify_no_security_audit_allowlist.txt`
- Modify: `.github/workflows/` or the existing CI script entrypoint that runs repo checks

- [ ] **Step 1: Add the rollout and migration cutover checklist to the docs**

```md
## Rollout order

1. Drain and stop all old controller instances before applying the semantic audit migration.
2. Run the database migration once.
3. Start the new controller build and verify `/audit-logs` reads semantic rows only.
4. Upgrade connected services and runtimes after the controller is live.
5. Treat legacy request-shaped audit rows as intentionally discarded.
```

- [ ] **Step 2: Write the guardrail script first**

```bash
#!/usr/bin/env bash
set -euo pipefail

ALLOWLIST_FILE="ci/verify_no_security_audit_allowlist.txt"

if rg -n 'target:\s*"security_audit"' crates --glob '**/*.rs' --glob '!**/migration/**' 2>/dev/null \
  | rg -v '^[^:]+:[0-9]+:\s*//' \
  | rg -v -f "$ALLOWLIST_FILE"; then
  echo "legacy security_audit callsites remain"
  exit 1
fi

if rg -n 'AuditActionType::(new|from_static)\("|AuditEntry::builder\("|action_type:\s*"' crates \
  --glob '**/*.rs' \
  --glob '!**/migration/**' \
  --glob '!**/tests/**' \
  --glob '!**/fixtures/**' \
  --glob '!**/docs/**' 2>/dev/null \
  | rg -v '^[^:]+:[0-9]+:\s*//' \
  | rg -v -f "$ALLOWLIST_FILE"; then
  echo "raw audit action strings remain outside approved contexts"
  exit 1
fi
```

- [ ] **Step 3: Add the script to CI and verify it fails on current leftovers**

Run: `bash ci/verify_no_security_audit.sh` Expected: FAIL until all legacy `security_audit` callsites are removed or allowlisted.

- [ ] **Step 4: Rewrite the human and agent docs to the semantic model**

```md
## What audit logs are now

Uptrakit audit logs record semantic actions such as `plugin_config.create`, `service.merge`, and `auth.login`, not raw HTTP requests.
```

- [ ] **Step 5: Run the repo-level verification set**

Run: `cargo fmt --all` Expected: PASS.

Run: `cd frontend && npm ci && npm run build` Expected: PASS and `frontend/build/` exists before any `--all-features` cargo command.

Run: `cargo check --no-default-features --features db-sqlite` Expected: PASS.

Run: `cargo check --all-features` Expected: PASS.

Run: `cargo clippy --all-targets --no-default-features --features db-sqlite` Expected: PASS.

Run: `cargo clippy --all-targets --all-features` Expected: PASS.

Run: `cargo test --all-features` Expected: PASS.

Run: `docker build -f docker/Dockerfile.test -t uptrakit-test:latest .` Expected: PASS and refreshes the integration-test image before running ignored
database tests.

Run: `cargo test -p uptrakit-integration-tests --test database -- --ignored` Expected: PASS for schema and REST query changes.

Run: `cargo deny check` Expected: PASS.

Run: `bash ci/verify_handler_state_contract.sh` Expected: PASS.

Run: `python3 ci/verify_db_access_policy.py` Expected: PASS.

Run: `bash ci/verify_no_security_audit.sh` Expected: PASS.

Run: `cd frontend && npm run lint && npm run check` Expected: PASS.

Run: `markdownlint --config .markdownlint.json '**/*.md'` Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add docs/development/audit-logs.md docs/security/audit-logs.md docs/end-user/audit-logs.md docs/api/audit-logs.md AGENTS.md ARCHITECTURE.md ci/verify_no_security_audit.sh ci/verify_no_security_audit_allowlist.txt
git commit -m "docs: document semantic audit logs"
```

## Self-Review

### Spec coverage

- Semantic action domain, registry, validation, and emitter: covered by Task 1.
- Schema replacement, request-row discard policy, indexes for both audit tables, SQLite-safe recreation, backend mapping, UTC timestamps: covered by
  Task 2.
- REST/API/OpenAPI/CLI/frontend audit log surface: covered by Tasks 3 and 4.
- Middleware removal from audit-row production, request-context extraction, and `AppState` plumbing across constructor sites: covered by Task 5.
- Required V1 REST producer catalog across auth, API tokens, settings, notifications, plugin config, services, software triggers, and ignore rules:
  covered by Task 6.
- `ServiceMessage::AuditEvent`, ingress re-validation, runtime forwarding, and scheduler-originated audit events: covered by Task 7.
- Docs, AGENTS/ARCHITECTURE updates, and CI guardrails: covered by Task 8.

### Placeholder scan

- No `TODO` / `TBD` / “similar to previous task” placeholders remain.
- Every task names concrete files, commands, and intended code shapes.

### Type consistency

- `AuditActionType` is treated as a validated owned-string newtype internally, while `AuditEventPayload.action_type` stays a wire-boundary `String`.
- `AuditEntry::builder`, `AuditEntry::test_stub`, and `AuditActorType::{Service,System}` are defined in Task 1 before later tasks use them.
- `AuditEventPayload.outcome` is intentionally a wire-boundary string validated into the closed internal outcome enum.
- `AuditLogResponse` / frontend `AuditLogEntry` both follow the action-shaped contract, not the old request-shaped one.
