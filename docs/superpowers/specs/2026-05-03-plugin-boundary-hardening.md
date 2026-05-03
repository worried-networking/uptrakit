# Plugin Boundary Hardening

**Date:** 2026-05-03  
**Status:** Spec

## Background

`roles.rs` in `uptrakit-plugin-infrastructure-core` contains 142+ plugin-named identifiers:
`ProxmoxXxx`, `DockerXxx`, `EmailSmtpXxx`, `TelegramXxx` store traits, request structs, and
response types. `SurfaceActionController` and `UpdateProtectionController` expose plugin-named
store-accessor methods. Consumers outside the plugin world — `web-api-queries` and
`surface-proxy` — import these types directly and implement the store traits, creating hard
knowledge of which plugins exist throughout the controller layer.

The boundary rule being enforced: **plugin details must not escape the plugin's own crate**.
Only `uptrakit-plugin-infrastructure-registry` (the wiring seam) may aggregate plugin-specific
artefacts, and only for structural assembly — not to leak store traits to consumers.

## Goals

1. `web-api-queries` and `surface-proxy` have zero plugin-specific imports and implement no
   plugin-specific store traits.
2. `roles.rs` contains only generic plugin role traits (`Discoverer`, `ReleaseFetcher`,
   `UpdateExecutor`, `NotificationTransport`, etc.) — no plugin-named types.
3. `SurfaceActionController` and `UpdateProtectionController` expose `fn tenant_db(&self) -> &TenantDb`
   as the sole persistence seam — no plugin-named accessor methods.
4. Plugin-owned DB tables are queried exclusively by the plugin that owns them.
5. `uptrakit-tenant-db` is a standalone crate any plugin can depend on without pulling in SeaORM
   entity definitions.
6. Tenant data reset remains atomic across core and plugin tables via registered callbacks.

## Non-Goals

- Notification channel / rule / log entities — generic infrastructure, stay in `shared-db`.
- `host_software_item_plugin` entity — generic join table shared across plugins, stays in `shared-db`.
- `plugin_config` / `plugin_type_setting` tables — generic config storage, unchanged.
- FK cascade changes to any existing table.
- Public HTTP API types in `uptrakit-web-api-types`.
- Audit table deletion in any reset path — `proxmox_protection_audit` is never deleted.

## Dependency

Wave 3 assumes the **surface-proxy controller-local wiring spec**
(`2026-05-03-surface-proxy-controller-local-wiring-design.md`) has landed. Specifically, the
three `controller_local/` submodules it creates (`docker.rs`, `notification_settings.rs`,
`proxmox_update_protection.rs`) and the `AppStateSurfaceActionController` struct in
`controller_local.rs` must exist before Wave 3 begins.

---

## Wave 1: Create `uptrakit-tenant-db`

**Goal:** Extract `TenantDb` and `TenantScoped` into a standalone crate so
`plugin-infrastructure-core` can reference them without pulling in all SeaORM entity
definitions from `shared-db`.

### New crate

- Path: `crates/shared/tenant-db/`
- Crate name: `uptrakit-tenant-db`
- Dependencies: `sea-orm` (workspace), `uuid` (workspace)
- Public exports: `TenantDb`, `TenantScoped`
- Zero SeaORM entity definitions — only the wrapper struct and the trait

### Moves

| Source                                                                                       | Destination                                    |
| -------------------------------------------------------------------------------------------- | ---------------------------------------------- |
| `crates/shared/db/src/tenant_db.rs`                                                          | `crates/shared/tenant-db/src/tenant_db.rs`     |
| `TenantScoped` **trait definition only** from `crates/shared/db/src/entity/tenant_scoped.rs` | `crates/shared/tenant-db/src/tenant_scoped.rs` |

`tenant_scoped.rs` in `shared-db` also contains 22+ `impl TenantScoped for X::Entity`
blocks that each reference concrete entity modules from `shared-db`. Those impl blocks
**stay in `shared-db`** — moving them would create a dependency cycle (the new crate
would have to import all of `shared-db`'s entities). Only the trait definition
(`pub trait TenantScoped { ... }`) moves. `shared-db/entity/tenant_scoped.rs` shrinks
to just the impl blocks, which now import the trait from `uptrakit_tenant_db`.

### Dependency updates

| Crate                                    | Change                                                                                                           |
| ---------------------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| `uptrakit-shared-db`                     | Add `uptrakit-tenant-db` dep; re-export `TenantDb` and `TenantScoped` from it; remove the two moved source files |
| `uptrakit-plugin-infrastructure-proxmox` | Add direct dep on `uptrakit-tenant-db`                                                                           |
| `uptrakit-plugin-releases-docker`        | Add direct dep                                                                                                   |
| `uptrakit-notification-plugin-email`     | Add direct dep                                                                                                   |
| `uptrakit-notification-plugin-telegram`  | Add direct dep                                                                                                   |

`uptrakit-web-api-queries` already re-exports from `shared-db` via its
`src/tenant_db.rs` — no change needed there.

In `uptrakit-plugin-infrastructure-core/Cargo.toml`, `uptrakit-tenant-db` must be
added under the `plugin-ops` feature (not unconditional), since `sea-orm` is already
gated behind `plugin-ops` in that crate. Adding it unconditionally would pull `sea-orm`
into every transitive consumer of `plugin-infrastructure-core`, including agent-side
builds that intentionally have no DB dependency.

### Acceptance

- `TenantDb` reachable as both `uptrakit_tenant_db::TenantDb` and `uptrakit_shared_db::TenantDb`.
- All four plugin crates compile with the direct dep.
- `cargo check --all-features` clean.

---

## Wave 2: Add `tenant_db()` to controller traits

**Goal:** Establish the new DB-access seam on both controller traits. Keep existing
plugin-named methods — removal is deferred to Wave 3. Purely additive; no behaviour change.

### `plugin-infrastructure-core` changes

Add `uptrakit-tenant-db` as a dep gated behind the `plugin-ops` feature in `Cargo.toml`
(same gate as `sea-orm`). The `tenant_db()` method on the controller traits and the
`SurfaceActionContext` delegate are only reachable when `plugin-ops` is active, which
is the only context where a `TenantDb` can be constructed.

Add to `SurfaceActionController` in `roles.rs`:

```rust
fn tenant_db(&self) -> &uptrakit_tenant_db::TenantDb;
```

Add to `UpdateProtectionController` in `roles.rs`:

```rust
fn tenant_db(&self) -> &uptrakit_tenant_db::TenantDb;
```

Both are required methods with no default. Add convenience delegate to `SurfaceActionContext`
in `descriptor.rs`:

```rust
pub fn tenant_db(&self) -> &uptrakit_tenant_db::TenantDb {
    self.controller.tenant_db()
}
```

### `surface-proxy` changes

`AppStateSurfaceActionController` in `proxy/controller_local.rs` gains a
`tenant_db: TenantDb` field (constructed from the existing `db` + `tenant_id` in its
constructor) and implements:

```rust
fn tenant_db(&self) -> &TenantDb {
    &self.tenant_db
}
```

### `web-api-queries` changes

`QueryUpdateProtectionController` in `update_dispatch.rs` changes its inner field from
`db: &'a DatabaseConnection` to `tenant_db: &'a TenantDb`. Implements:

```rust
fn tenant_db(&self) -> &TenantDb {
    self.tenant_db
}
```

Construction sites (lines ~842, ~898) currently have `db: &DatabaseConnection` and
`tenant_id: Uuid` in scope. Construct `TenantDb::new(db, tenant_id)` at the call site
and pass `&tenant_db` to the constructor. Update function signatures if `tenant_id` is
not yet in scope at those points.

### Acceptance

- Old plugin-named store methods still compile — not removed.
- New `tenant_db()` callable on both traits.
- `cargo check --all-features` clean.

---

## Wave 3: Migrate plugins + remove all plugin-named store types

**Goal:** Update every plugin that calls a plugin-named store method to query via
`tenant_db()` directly. Simultaneously remove every plugin-named store trait, request
type, and accessor method from `roles.rs` and both controller traits.

This is the largest wave. Sub-steps 3a–3f each update one plugin to query via
`tenant_db()` directly while **leaving the old store method on the controller trait
intact**. Each sub-step is independently CI-green because the store method still
compiles. Sub-step 3g then removes all store methods, store traits, and registry
re-exports in a single atomic commit — this is the only irreversible step. Sub-step
3h is independent and may land before or after 3g.

### 3a — Docker plugin

`surfaces.rs` in `uptrakit-plugin-releases-docker` calls `ctx.controller.docker_surface_store()`.

**After:**

- Surface action handlers call `ctx.tenant_db()` and query `host_software_item_plugin`
  entity directly (filtering by `plugin_type == DOCKER_RELEASES_CONFIG_TYPE`).
- `DockerSurfaceStore` trait deleted from `roles.rs`.
- `DockerItemHostRequest`, `DockerSwitchTagRequest` deleted from `roles.rs`.
- `impl DockerSurfaceStore for AppStateSurfaceActionController` deleted from
  `surface-proxy/proxy/controller_local.rs`.
- `docker_surface_store()` method deleted from `SurfaceActionController`.

### 3b — Email plugin

`uptrakit-notification-plugin-email` surface action handlers use
`ctx.controller.email_smtp_settings_store()`.

**After:**

- Handlers call `ctx.tenant_db()` and query `setting` / `global_setting` entities directly.
- `EmailSmtpSettings`, `EmailSmtpSettingsPatch`, `EmailSmtpSettingsStore` deleted from `roles.rs`.
- `impl EmailSmtpSettingsStore for AppStateSurfaceActionController` deleted.
- `email_smtp_settings_store()` deleted from `SurfaceActionController`.

**`web-api-queries/notification_settings.rs` fix:**

`notification_settings.rs:4` imports `EmailSmtpSettings` from the registry to build
the settings bag for email delivery. Once `EmailSmtpSettings` is removed from the
registry re-exports, this import breaks. Fix: rewrite `build_settings_bag` to query
the `setting` and `global_setting` tables directly via raw DB calls and build the
`serde_json::Value` result without using the typed struct. The public signature
(`async fn build_settings_bag(db: &DatabaseConnection, tenant_id: Uuid) -> serde_json::Value`)
stays unchanged — all callers remain unmodified. Known callers: `dispatcher.rs`,
`notifications.rs`, `surface-proxy/proxy.rs`, and
`surface-proxy/proxy/controller_local/notifications.rs`.

Risk: the rewrite replaces `EmailSmtpSettings` struct-field access with string-keyed
map construction. Any misspelled key would be a silent runtime failure (wrong field
name = empty value in the settings bag). Mitigate by adding a test that exercises
`build_settings_bag` against a real DB with known fixture data.

### 3c — Telegram plugin

Same pattern as Email.

- `TelegramGlobalSettingsStore` deleted from `roles.rs`.
- `impl TelegramGlobalSettingsStore for AppStateSurfaceActionController` deleted.
- `telegram_global_settings_store()` deleted from `SurfaceActionController`.

### 3d — Notification plugins (all three + notification-plugin-core)

Surface action handlers for channel listing/management use
`ctx.controller.notification_channel_store()`.

Note: `notification-plugin-core/list_channels.rs` already accepts
`(db: &DatabaseConnection, tenant_id: Uuid, ...)` directly — no signature change
needed there. The store trait exists on the controller, not inside `list_channels.rs`.

**After:**

- Handlers call `ctx.tenant_db()` and query `notification_channel` entity directly.
- `NotificationChannelStore`, `NotificationChannelListRequest`, `NotificationChannelListItem`,
  `NotificationChannelListPage`, `NotificationActionTokenRecord` deleted from `roles.rs`.
- `impl NotificationChannelStore for AppStateSurfaceActionController` deleted.
- `notification_channel_store()` deleted from `SurfaceActionController`.

### 3e — Proxmox surface actions

`surfaces.rs` in `uptrakit-plugin-infrastructure-proxmox` uses
`ctx.controller.proxmox_surface_store()`.

**After:**

- All surface action handlers call `ctx.tenant_db()` and query Proxmox entities directly.
  At this point entities still live in `shared-db` (moved in Wave 4); imports are
  `uptrakit_shared_db::entity::proxmox_*`.
- `ProxmoxSurfaceStore` deleted from `roles.rs`.
- All `ProxmoxXxx` request/response types deleted from `roles.rs`:
  `ProxmoxHostMappingsRequest`, `ProxmoxPluginConfigRequest`, `ProxmoxManualMatchRequest`,
  `ProxmoxApproveMatchRequest`, `ProxmoxMappingRequest`, `ProxmoxHostInfoRequest`,
  `ProxmoxUnmatchedGuestsRequest`, `ProxmoxScopeSelectionRequest`,
  `ProxmoxItemOverridePreloadRequest`, `ProxmoxItemOverrideSaveRequest`,
  `ProxmoxGlobalDefaultsSaveRequest`, `ProxmoxProtectionAuditRecord`,
  `ProxmoxProtectionMode`, `ProxmoxProtectionPolicyRecord`, `ProxmoxHostMappingRecord`.
- `impl ProxmoxSurfaceStore for AppStateSurfaceActionController` deleted from `surface-proxy`.
- `proxmox_surface_store()` deleted from `SurfaceActionController`.
- `proxmox_protection_store()` also deleted from `SurfaceActionController` here
  (it exists on both traits; its removal from `UpdateProtectionController` is in 3f).

### 3f — Proxmox update protection

`uptrakit-plugin-infrastructure-proxmox`'s `ControllerUpdateProtection` impl calls
`ctx.controller.proxmox_protection_store()`, which routes to
`QueryProxmoxProtectionStore` in `update_dispatch.rs`.

**After:**

- `prepare_pre_update_protection` and `finalize_post_update` call
  `ctx.controller.tenant_db()` and query Proxmox entities directly. Imports from
  `uptrakit_shared_db::entity::proxmox_*` (still in `shared-db` until Wave 4).
- `ProxmoxProtectionStore` trait deleted from `roles.rs`.
- `QueryProxmoxProtectionStore` struct and its `impl ProxmoxProtectionStore` deleted from
  `web-api-queries/update_dispatch.rs`.
- `proxmox_protection_store()` deleted from `UpdateProtectionController`.
- All tests for `QueryProxmoxProtectionStore` in `update_dispatch.rs` deleted or migrated
  to the proxmox plugin crate.

### 3g — Registry cleanup

Remove all plugin-named re-exports from `uptrakit-plugin-infrastructure-registry/src/lib.rs`:
all `Proxmox*`, `Docker*`, `EmailSmtp*`, `TelegramGlobalSettings*`,
`NotificationChannelList*`, `NotificationActionTokenRecord` re-exports. Remove the
`execute_proxmox_controller_*` free-function re-exports (surface proxy no longer routes
through these typed helpers once the surface actions use `tenant_db()` directly). Any
remaining call sites for these free functions must be removed before landing this step.

### 3h — Transactional email abstraction (`users.rs`)

`crates/ui/web-api/src/routes/users.rs` sends email-change confirmation emails.
Its `send_email_change_emails` function (around line 1226) hardcodes three
boundary violations:

- `uptrakit_shared_types::plugin_ids::EMAIL` — routes by plugin ID name
- `NotificationPluginError::SmtpNotConfigured` — matches an email-plugin-specific error variant
- `uptrakit_web_api_queries::notification_settings::build_settings_bag` — builds
  email-specific settings in a non-plugin crate

**New type in `plugin-infrastructure-core/plugin_ops.rs`:**

```rust
#[non_exhaustive]
#[derive(Debug)]
pub enum TransactionalEmailError {
    /// No SMTP transport is configured for this tenant.
    NotConfigured,
    /// Delivery was attempted but failed.
    DeliveryFailed(Box<dyn std::error::Error + Send + Sync>),
}
```

**New method on `NotificationOps` in `plugin_ops.rs`:**

`async-trait` is already a dep of `plugin-infrastructure-core`. Annotate `NotificationOps`
with `#[async_trait]` and add the method as a plain `async fn` with a default body:

```rust
#[async_trait]
pub trait NotificationOps: Send + Sync + 'static {
    // … existing methods unchanged …

    async fn send_transactional_email(
        &self,
        tenant_db: &TenantDb,
        to: &str,
        subject: &str,
        text_body: &str,
        html_body: &str,
    ) -> Result<(), TransactionalEmailError> {
        let _ = (tenant_db, to, subject, text_body, html_body);
        Err(TransactionalEmailError::NotConfigured)
    }
}
```

`TenantDb` (not `DatabaseConnection`) is the right parameter: it already carries
the tenant scoping, and the registry impl uses `tenant_db.db()` +
`tenant_db.tenant_id` internally. The default returns `Err(NotConfigured)` so
existing test stubs compile without changes. The default body is deliberately
explicit (not a one-liner) so reviewers see the noop is intentional, not an
oversight.

The registry's `impl NotificationOps` overrides this default, routing via
`plugin_ids::EMAIL` internally — that knowledge stays inside the registry.
`SmtpNotConfigured` is mapped to `TransactionalEmailError::NotConfigured` within the
registry impl.

**`users.rs` changes:**

- Remove imports of `plugin_ids`, `NotificationPluginError`, and
  `notification_settings::build_settings_bag`.
- Replace the ad-hoc send sequence with
  `state.notification_ops().send_transactional_email(&tenant_db, to, subject, text, html).await`
  where `tenant_db` is constructed from `state.db()` + `tenant_id`.
- Match `TransactionalEmailError::NotConfigured` where `SmtpNotConfigured` was matched.

`web-api-queries/notification_settings.rs` is **not** deleted here — `dispatcher.rs` and
`notifications.rs` still use `build_settings_bag` and do not have boundary violations.
That module's future is tracked separately once the notification delivery pipeline
is refactored.

### Acceptance

- Zero occurrences of `DockerSurfaceStore`, `ProxmoxSurfaceStore`, `ProxmoxProtectionStore`,
  `EmailSmtpSettingsStore`, `TelegramGlobalSettingsStore`, `NotificationChannelStore`
  anywhere in the codebase.
- `web-api-queries/update_dispatch.rs` imports no `proxmox_*` entities.
- `surface-proxy/proxy/controller_local.rs` implements no plugin-specific store traits.
- `web-api-queries/notification_settings.rs` imports no `EmailSmtpSettings` or any other
  plugin-specific type.
- `cargo check --all-features` clean.

---

## Wave 4: Move Proxmox entities

**Goal:** Proxmox entities are now referenced only within `uptrakit-plugin-infrastructure-proxmox`
(after Wave 3). Move them.

### Moves

| Source (`shared-db/entity/`)          | Destination (`proxmox/src/entity/`)   |
| ------------------------------------- | ------------------------------------- |
| `proxmox_host_mapping.rs`             | `proxmox_host_mapping.rs`             |
| `proxmox_protection_audit.rs`         | `proxmox_protection_audit.rs`         |
| `proxmox_protection_default.rs`       | `proxmox_protection_default.rs`       |
| `proxmox_protection_item_override.rs` | `proxmox_protection_item_override.rs` |
| `proxmox_backup_target_cache.rs`      | `proxmox_backup_target_cache.rs`      |

New file: `crates/plugins/infrastructure/proxmox/src/entity/mod.rs` — pub re-exports for
all five modules. Visibility is `pub(crate)` unless the reset fn in Wave 6 requires wider
access.

### Crate changes

- `shared-db`: remove 5 entity files; remove 5 `pub mod proxmox_*` lines from `entity/mod.rs`
  and `prelude.rs`.
- Proxmox plugin: add `pub(crate) mod entity;` to `lib.rs`. Verify `sea-orm` is present
  as an unconditional workspace dep (currently behind `agent-infra` feature — promote to
  unconditional if needed to support controller-side entity use).
- All internal `use uptrakit_shared_db::entity::proxmox_*` imports in the proxmox plugin
  change to `use crate::entity::proxmox_*`.

### Acceptance

- `uptrakit_shared_db::entity` exports no `proxmox_*` module.
- Proxmox plugin compiles with its own entity definitions.
- `cargo check --all-features` clean.

---

## Wave 5: Clean up `roles.rs` and registry

**Goal:** Remove any remaining orphaned identifiers in `roles.rs` and tidy the registry
re-export surface.

### `roles.rs` final audit

Grep `roles.rs` for any remaining `Proxmox`, `Docker`, `EmailSmtp`, `Telegram`,
`NotificationChannel` identifiers (including `#[cfg]`-gated blocks). Delete everything
found that is not a generic role trait.

After this wave `roles.rs` must contain only:

- `PluginMeta`
- `Discoverer`, `VersionDetector`, `ReleaseFetcher`, `PackageIndexer`
- `ExecuteUpdateResult`, `UpdateExecutor`
- `LifecycleHook`
- `NotificationTransport`
- `SoftwareItemLifecycle` and supporting types (`SoftwareItemCreatedEvent`,
  `SoftwareItemLifecycleContext`, `SoftwareItemPatch`)
- `ControllerUpdateProtection`, `ControllerProtectionContext`, `ControllerProtectionDecision`,
  `ControllerPostUpdateContext`, `PostUpdateOutcome`
- `SurfaceActionController` (now containing only `tenant_id()`, `user_id()`, `tenant_db()`)
- `UpdateProtectionController` (now containing only `tenant_db()`)

### Registry re-export tidy

- Verify `uptrakit-plugin-infrastructure-registry/src/lib.rs` contains no
  `Proxmox*`, `Docker*`, `EmailSmtp*`, `TelegramGlobalSettings*`,
  `NotificationChannelList*` re-exports.
- `DeliveryMessage`, `MessageAction`, `escape_html`, `NotificationPluginError` from
  `uptrakit-notification-plugin-core` are **retained** — they are legitimate notification
  infrastructure exports, not store-trait types.

### `notification-plugin-core` tidy

`list_channels.rs` now accepts `&TenantDb` directly. Remove any vestigial
`NotificationChannelStore` trait usage. Ensure `uptrakit-tenant-db` is a dep of
`uptrakit-notification-plugin-core`.

### Acceptance

- `roles.rs` contains zero plugin-named identifiers (grep clean).
- Registry `lib.rs` contains zero plugin-named store-trait re-exports.
- `cargo check --all-features` clean.
- `cargo clippy --all-targets --all-features` clean.

---

## Wave 6: Reset callbacks + final boundary audit

**Goal:** Restore tenant-data-reset coverage for Proxmox tables; perform final boundary audit.

### New type in `plugin-infrastructure-core/descriptor.rs`

Gated on the `migrations` feature (same gate as `MigrationsFn`):

```rust
#[cfg(feature = "migrations")]
pub type ResetTenantDataFn =
    for<'a> fn(
        tenant_id: uuid::Uuid,
        txn: &'a sea_orm::DatabaseTransaction,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), sea_orm::DbErr>> + Send + 'a>,
    >;
```

### `PluginDescriptor` field addition

Add the field to `PluginDescriptor`:

```rust
#[cfg(feature = "migrations")]
pub reset_tenant_data: Option<ResetTenantDataFn>,
```

All `PluginDescriptor` initializers are generated by the `declare_plugin!` macro.
Update the macro to include `reset_tenant_data: None` in the generated struct
literal — this sweeps all plugin crates atomically. Do NOT add `#[non_exhaustive]`
to `PluginDescriptor`: all construction goes through `declare_plugin!` already, so
`#[non_exhaustive]` would break the macro-generated struct literals without providing
any practical benefit.

### Proxmox plugin registration

`ProxmoxPlugin::DESCRIPTOR` sets `reset_tenant_data: Some(proxmox_reset_tenant_data)`.

`proxmox_reset_tenant_data` in the proxmox plugin (new `src/reset.rs` or inside
`controller_migration.rs`) deletes plugin-owned rows in FK-safe order:

```rust
fn proxmox_reset_tenant_data<'a>(
    tenant_id: Uuid,
    txn: &'a DatabaseTransaction,
) -> Pin<Box<dyn Future<Output = Result<(), DbErr>> + Send + 'a>> {
    Box::pin(async move {
        // proxmox_protection_item_override has no tenant_id column.
        // It has Restrict FKs to both plugin_config and software_items.
        // Delete via plugin_config_id subquery.
        let config_ids: Vec<Uuid> = plugin_config::Entity::find()
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .select_only()
            .column(plugin_config::Column::Id)
            .into_tuple::<Uuid>()
            .all(txn)
            .await?;
        proxmox_protection_item_override::Entity::delete_many()
            .filter(proxmox_protection_item_override::Column::PluginConfigId.is_in(config_ids))
            .exec(txn).await?;
        // The remaining tables have direct tenant_id columns.
        proxmox_protection_default::Entity::delete_many()
            .filter(proxmox_protection_default::Column::TenantId.eq(tenant_id))
            .exec(txn).await?;
        proxmox_backup_target_cache::Entity::delete_many()
            .filter(proxmox_backup_target_cache::Column::TenantId.eq(tenant_id))
            .exec(txn).await?;
        proxmox_host_mapping::Entity::delete_many()
            .filter(proxmox_host_mapping::Column::TenantId.eq(tenant_id))
            .exec(txn).await?;
        // proxmox_protection_audit: audit table — never deleted
        Ok(())
    })
}
```

`plugin_config` is from `uptrakit_shared_db::entity::plugin_config` — the proxmox
plugin already depends on `shared-db`, so this import is already available. After
Wave 4 moves the Proxmox entities out of `shared-db`, the proxmox plugin's `shared-db`
dep must be **retained** (not removed) because `reset.rs` references `plugin_config::Entity`.
Add a comment in `Cargo.toml` at the `shared-db` dep line to make this explicit.

### Registry: `reset_tenant_data` helper

```rust
#[cfg(feature = "migrations")]
pub async fn reset_plugin_tenant_data(
    tenant_id: Uuid,
    txn: &sea_orm::DatabaseTransaction,
) -> Result<(), sea_orm::DbErr> {
    for descriptor in all_descriptors() {
        if let Some(reset_fn) = descriptor.reset_tenant_data {
            reset_fn(tenant_id, txn).await?;
        }
    }
    Ok(())
}
```

### `web-api-queries/reset_data.rs`

- **Replace step 6** (`proxmox_host_mapping::Entity::delete_many()...`) with a call to
  `uptrakit_plugin_infrastructure_registry::reset_plugin_tenant_data(tenant_id, &txn).await.context_to()?`.
  (`reset_plugin_tenant_data` returns `Result<(), sea_orm::DbErr>`; the existing
  `impl_report_conversion!(sea_orm::DbErr => ResetDataQueryError::Database)` makes
  `context_to()?` work without additional conversion.)
  Running at the step-6 position is required for correctness:
  - `proxmox_protection_item_override` has a `Restrict` FK to `software_items` (deleted
    at step 8) — the callback must precede step 8.
  - `proxmox_protection_default` and `proxmox_protection_item_override` also have
    `Restrict` FKs to `plugin_config` (deleted at step 10) — the callback must
    precede step 10.
  - Step 6 satisfies both constraints and was already the Proxmox-specific step.
- Remove `proxmox_host_mapping` from the entity import list.
- This also fixes a pre-existing bug: the current `reset_data.rs` never deleted
  `proxmox_protection_defaults` or `proxmox_protection_item_overrides`, making step 10
  fail with a FK constraint violation for any tenant that has Proxmox protection
  defaults configured.

### Final boundary audit

Run across all non-plugin crates:

```sh
grep -rn "Proxmox\|proxmox_host_mapping\|proxmox_protection\|DockerSurface\|EmailSmtp\|TelegramGlobal\|NotificationChannelStore" \
  crates/ui/ crates/core/ crates/shared/ --include="*.rs" \
  | grep "use " \
  | grep -v '"discovery_proxmox'
```

Do not exclude `plugin_ids` here — after Wave 3h removes `plugin_ids::EMAIL` from
`users.rs`, any remaining `plugin_ids` reference in non-plugin code is a genuine
violation that the CI shell script should catch independently.

Zero results expected. Fix any remaining hits.

### Acceptance

- `reset_data.rs` imports no `proxmox_*` entity.
- `proxmox_reset_tenant_data` registered and invoked correctly; existing reset integration
  tests pass with proxy tenant-data assertions intact.
- Final boundary audit returns zero hits.
- `cargo check --no-default-features --features db-sqlite` clean.
- `cargo check --all-features` clean.
- `cargo clippy --all-targets --all-features` clean.
- `cargo test --all-features` passes.

---

## Wave 7: Update boundary check scripts

**Goal:** Make `ci/check_plugin_semantic_boundary.py` and
`ci/check_plugin_semantic_boundary.sh` detect all violations fixed in this spec,
so regressions are caught automatically.

### Why current violations are not caught

Understanding the gaps is necessary to fix them correctly.

**Gap 1 — Registry allowlist (covers almost all store-trait violations).**
`RULE_CONCRETE_PLUGIN_IMPORT` in the Python checker exempts every crate in
`ALLOWED_REGISTRY_CATALOGUE_IMPORT_CRATES`, which includes
`uptrakit_plugin_infrastructure_registry`. Every store-trait violation —
`ProxmoxProtectionStore`, `EmailSmtpSettings`, `NotificationPluginError::SmtpNotConfigured`,
`TelegramGlobalSettingsStore` — is imported through the registry and therefore
invisible to the checker. The allowlist was correct when written (block direct
concrete-plugin imports), but it does not detect plugin-specific types leaking
through the registry's re-export surface.

**Gap 2 — Shell script not in CI.**
`check_plugin_semantic_boundary.sh` already flags `plugin_ids::EMAIL` in `users.rs`:

```text
semantic-boundary violation: plugin_ids token references in non-plugin production code
./ui/web-api/src/routes/users.rs:1237:    .transport(&uptrakit_shared_types::plugin_ids::EMAIL)
```

But only `check_plugin_semantic_boundary.py` runs in CI (`.github/workflows/ci.yml:57`).
The shell script is an unexecuted local tool.

**Gap 3 — Python `plugin_ids` rule misses inline qualified paths.**
`RULE_PLUGIN_IDS_REFERENCE` tracks `plugin_ids` through `use` statement bindings.
`users.rs` uses `uptrakit_shared_types::plugin_ids::EMAIL` as an inline fully-qualified
path with no `use plugin_ids` import — no binding is registered, so the usage is never
matched.

### Shell script (`check_plugin_semantic_boundary.sh`)

**Add to CI** (`ci.yml`): add a step running `./ci/check_plugin_semantic_boundary.sh`
immediately after the Python checker step. The existing `deny_plugin_ids_rule` already
covers `plugin_ids::EMAIL` and requires no new patterns.

### Python script (`check_plugin_semantic_boundary.py`)

Two fixes needed.

**Fix A — Inline qualified `plugin_ids` paths (Gap 3).**

`add_plugin_ids_reference_findings` currently only fires on `use`-imported bindings.
Add a secondary regex scan within the same function to catch inline qualified paths
that bypass `use`:

```python
PLUGIN_IDS_INLINE_QUALIFIED_RE = re.compile(
    r"\bplugin_ids\s*::\s*[A-Z_][A-Z0-9_]*\b"
)
```

Emit findings with `rule_id=RULE_PLUGIN_IDS_REFERENCE`, `match_kind="module_token"`.
Scope: same `looks_like_production_code` files as the binding-based scan.

**Fix B — Transport-specific error variant escapes (Gap 1).**

Add a new rule to detect plugin-specific `NotificationPluginError` variants in
non-plugin code:

```python
RULE_PLUGIN_TRANSPORT_ESCAPE = "plugin-transport-escape"
```

Add to `KNOWN_RULE_IDS` and `RULE_MATCH_KINDS`:

```python
RULE_PLUGIN_TRANSPORT_ESCAPE: {"symbol_name"},
```

Detection regex:

```python
NOTIFICATION_PLUGIN_TRANSPORT_VARIANT_RE = re.compile(
    r"\bNotificationPluginError\s*::\s*(?:SmtpNotConfigured|SmtpDeliveryFailed"
    r"|TelegramApiError|TelegramNotConfigured)\b"
)
```

Emit a `Finding` with `rule_id=RULE_PLUGIN_TRANSPORT_ESCAPE`,
`match_kind="symbol_name"`. Add `RULE_PLUGIN_TRANSPORT_ESCAPE` to the allowlist
schema as a valid `rule_id`.

Note: no new rule is needed for the store-trait violations
(`ProxmoxProtectionStore`, `EmailSmtpSettings`, etc.) — those types are deleted
in Waves 3–5 and will no longer exist in the registry to be imported.

**Structural gap (out of scope for this spec):** `ALLOWED_REGISTRY_CATALOGUE_IMPORT_CRATES`
allows any type from the registry to be imported by non-plugin code. This means a
future developer who adds a plugin-specific type to the registry re-export surface
will not be detected by any checker. Add a comment at `ALLOWED_REGISTRY_CATALOGUE_IMPORT_CRATES`
in `check_plugin_semantic_boundary.py` explicitly noting this limitation, so the next
person understands what the allowlist does and does not enforce.

### Acceptance

- Before Wave 3h: `./ci/check_plugin_semantic_boundary.sh` reports `plugin_ids
token references` violation for `users.rs` (confirming Gap 2 is real).
- Before Wave 7 Python fix A: `python3 ci/check_plugin_semantic_boundary.py` does
  NOT flag `plugin_ids::EMAIL` inline path in `users.rs` (confirming Gap 3 is real).
- After Wave 7: Python checker flags both `plugin_ids::EMAIL` inline paths and
  `NotificationPluginError::SmtpNotConfigured` if either appears in non-plugin code.
- Shell script runs in CI and exits 0 against the refactored codebase.
- After Wave 3h + Wave 7: both scripts exit 0.
- `python3 ci/check_plugin_semantic_boundary.py --help` and dry-run complete without
  config errors.

---

## Full Acceptance Criteria

| Check                                                           | Expected                                              |
| --------------------------------------------------------------- | ----------------------------------------------------- |
| `roles.rs` plugin-named identifiers                             | Zero                                                  |
| `SurfaceActionController` plugin-named methods                  | Zero (only `tenant_id()`, `user_id()`, `tenant_db()`) |
| `UpdateProtectionController` plugin-named methods               | Zero (only `tenant_db()`)                             |
| Plugin-specific store trait impls in `surface-proxy`            | Zero                                                  |
| Plugin-specific store trait impls in `web-api-queries`          | Zero                                                  |
| Proxmox entity imports in `web-api-queries`                     | Zero                                                  |
| `uptrakit-tenant-db` SeaORM entity definitions                  | Zero                                                  |
| `plugin_ids::EMAIL` in `users.rs`                               | Zero                                                  |
| `NotificationPluginError::SmtpNotConfigured` in non-plugin code | Zero                                                  |
| `./ci/check_plugin_semantic_boundary.sh`                        | Exits 0                                               |
| `python3 ci/check_plugin_semantic_boundary.py`                  | Exits 0                                               |
| `cargo check --all-features`                                    | Clean                                                 |
| `cargo clippy --all-targets --all-features`                     | Clean                                                 |
| `cargo test --all-features`                                     | Passes                                                |
