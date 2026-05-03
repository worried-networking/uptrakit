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

| Source | Destination |
| ------ | ----------- |
| `crates/shared/db/src/tenant_db.rs` | `crates/shared/tenant-db/src/tenant_db.rs` |
| `crates/shared/db/src/entity/tenant_scoped.rs` | `crates/shared/tenant-db/src/tenant_scoped.rs` |

`TenantScoped` imports only `EntityTrait` and `ColumnTrait` from `sea-orm` — no entity
file deps, safe to extract.

### Dependency updates

| Crate | Change |
| ----- | ------ |
| `uptrakit-shared-db` | Add `uptrakit-tenant-db` dep; re-export `TenantDb` and `TenantScoped` from it; remove the two moved source files |
| `uptrakit-plugin-infrastructure-proxmox` | Add direct dep on `uptrakit-tenant-db` |
| `uptrakit-plugin-releases-docker` | Add direct dep |
| `uptrakit-notification-plugin-email` | Add direct dep |
| `uptrakit-notification-plugin-telegram` | Add direct dep |

`uptrakit-web-api-queries` already re-exports from `shared-db` via its
`src/tenant_db.rs` — no change needed there.

### Acceptance

- `TenantDb` reachable as both `uptrakit_tenant_db::TenantDb` and `uptrakit_shared_db::TenantDb`.
- All four plugin crates compile with the direct dep.
- `cargo check --all-features` clean.

---

## Wave 2: Add `tenant_db()` to controller traits

**Goal:** Establish the new DB-access seam on both controller traits. Keep existing
plugin-named methods — removal is deferred to Wave 3. Purely additive; no behaviour change.

### `plugin-infrastructure-core` changes

Add `uptrakit-tenant-db` as a non-optional dependency in `Cargo.toml`.

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

Construction sites (lines ~842, ~898) updated to pass `&tenant_db` (the `TenantDb`
already in scope from the function signature).

### Acceptance

- Old plugin-named store methods still compile — not removed.
- New `tenant_db()` callable on both traits.
- `cargo check --all-features` clean.

---

## Wave 3: Migrate plugins + remove all plugin-named store types

**Goal:** Update every plugin that calls a plugin-named store method to query via
`tenant_db()` directly. Simultaneously remove every plugin-named store trait, request
type, and accessor method from `roles.rs` and both controller traits.

This is the largest wave. All sub-steps must land atomically (single commit or
fast-follow commit series with CI green at each step).

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

### 3c — Telegram plugin

Same pattern as Email.

- `TelegramGlobalSettingsStore` deleted from `roles.rs`.
- `impl TelegramGlobalSettingsStore for AppStateSurfaceActionController` deleted.
- `telegram_global_settings_store()` deleted from `SurfaceActionController`.

### 3d — Notification plugins (all three + notification-plugin-core)

Surface action handlers for channel listing/management use
`ctx.controller.notification_channel_store()`. `notification-plugin-core/list_channels.rs`
accepts a `&dyn NotificationChannelStore` parameter.

**After:**

- Handlers call `ctx.tenant_db()` and query `notification_channel` entity directly.
- `list_channels.rs` signature changes to accept `&TenantDb` instead of `&dyn NotificationChannelStore`.
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

```rust
async fn send_transactional_email(
    &self,
    db: &DatabaseConnection,
    tenant_id: Uuid,
    to: &str,
    subject: &str,
    text_body: &str,
    html_body: &str,
) -> Result<(), TransactionalEmailError>;
```

The registry's `impl NotificationOps` routes via `plugin_ids::EMAIL` internally —
that knowledge stays inside the registry crate, which is the approved crossing point.
`SmtpNotConfigured` is mapped to `TransactionalEmailError::NotConfigured` within the
registry impl.

**`users.rs` changes:**

- Remove imports of `plugin_ids`, `NotificationPluginError`, and
  `notification_settings::build_settings_bag`.
- Replace the ad-hoc send sequence with
  `state.notification_ops().send_transactional_email(db, tenant_id, to, subject, text, html).await`.
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
- `cargo check --all-features` clean.

---

## Wave 4: Move Proxmox entities

**Goal:** Proxmox entities are now referenced only within `uptrakit-plugin-infrastructure-proxmox`
(after Wave 3). Move them.

### Moves

| Source (`shared-db/entity/`) | Destination (`proxmox/src/entity/`) |
| ---------------------------- | ----------------------------------- |
| `proxmox_host_mapping.rs` | `proxmox_host_mapping.rs` |
| `proxmox_protection_audit.rs` | `proxmox_protection_audit.rs` |
| `proxmox_protection_default.rs` | `proxmox_protection_default.rs` |
| `proxmox_protection_item_override.rs` | `proxmox_protection_item_override.rs` |
| `proxmox_backup_target_cache.rs` | `proxmox_backup_target_cache.rs` |

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
- `SurfaceActionController`, `UpdateProtectionController`
  (both now containing only `tenant_id()`, `user_id()`, `tenant_db()`)

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

```rust
#[cfg(feature = "migrations")]
pub reset_tenant_data: Option<ResetTenantDataFn>,
```

All existing `PluginDescriptor` static initializers gain `reset_tenant_data: None`.
`#[non_exhaustive]` already covers external construction.

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
        // FK order: item_overrides → defaults → backup_target_cache → host_mappings
        proxmox_protection_item_override::Entity::delete_many()
            .filter(proxmox_protection_item_override::Column::TenantId.eq(tenant_id))
            .exec(txn).await?;
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

- Remove the explicit step 6 (`proxmox_host_mapping::Entity::delete_many()...`).
- Add call to `uptrakit_plugin_infrastructure_registry::reset_plugin_tenant_data(tenant_id, &txn).await?`
  **before step 10** (plugin_configs deletion). Rationale: `proxmox_protection_defaults` and
  `proxmox_protection_item_overrides` both have FK to `plugin_config` with `Restrict` — the
  callback must delete them first or step 10 fails with a constraint violation. This also
  fixes a pre-existing bug: the current `reset_data.rs` never deleted those tables, making
  step 10 silently fail for any tenant that has Proxmox protection defaults configured.
- Remove `proxmox_host_mapping` from the entity import list.

### Final boundary audit

Run across all non-plugin crates:

```sh
grep -rn "Proxmox\|proxmox_host_mapping\|proxmox_protection\|DockerSurface\|EmailSmtp\|TelegramGlobal\|NotificationChannelStore" \
  crates/ui/ crates/core/ crates/shared/ --include="*.rs" \
  | grep "use " \
  | grep -v 'plugin_ids\|"discovery_proxmox'
```

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

### Shell script (`check_plugin_semantic_boundary.sh`)

The existing `deny_plugin_ids_rule` call scans `ui/web-api/src/**/*.rs` and
`ui/web-api-queries/src/queries/**/*.rs` for the token `plugin_ids`. This already
catches `plugin_ids::EMAIL` in `users.rs` (and any future `plugin_ids::*` reference
in those directories). **No new rule needed** — the existing rule covers this class
of violation. Confirm by running the script before Wave 3h lands and verifying it
reports the `users.rs` hit.

### Python script (`check_plugin_semantic_boundary.py`)

The Python checker has no rule covering `NotificationPluginError::SmtpNotConfigured`
because that import comes from `uptrakit_plugin_infrastructure_registry`, which is in
`ALLOWED_REGISTRY_CATALOGUE_IMPORT_CRATES` and thus exempt from
`RULE_CONCRETE_PLUGIN_IMPORT`. A dedicated rule is required.

**New rule constant:**

```python
RULE_PLUGIN_TRANSPORT_ESCAPE = "plugin-transport-escape"
```

Add to `KNOWN_RULE_IDS` and `RULE_MATCH_KINDS`:

```python
RULE_PLUGIN_TRANSPORT_ESCAPE: {"symbol_name"},
```

**Detection:** Within `looks_like_production_code` files, flag any occurrence of
transport-specific `NotificationPluginError` variant names:

```python
NOTIFICATION_PLUGIN_TRANSPORT_VARIANT_RE = re.compile(
    r"\bNotificationPluginError\s*::\s*(?:SmtpNotConfigured|SmtpDeliveryFailed"
    r"|TelegramApiError|TelegramNotConfigured)\b"
)
```

Emit a `Finding` with `rule_id=RULE_PLUGIN_TRANSPORT_ESCAPE`,
`match_kind="symbol_name"`, `match_value` = the matched variant path. The check runs
in the same file-walk loop as existing findings; no separate scan pass needed.

Add `RULE_PLUGIN_TRANSPORT_ESCAPE` to the allowlist schema as a valid `rule_id` so the
allowlist mechanism can handle any legitimate future exemptions.

**Scope:** `looks_like_production_code` already excludes `crates/plugins/` and test
modules — no additional scoping needed.

### Acceptance

- Before Wave 3h: running `./ci/check_plugin_semantic_boundary.sh` reports
  `plugin_ids token references` violation for `users.rs`.
- Before Wave 7 Python update: `python3 ci/check_plugin_semantic_boundary.py`
  does NOT flag `SmtpNotConfigured` (confirming the gap this wave closes).
- After Wave 7: `python3 ci/check_plugin_semantic_boundary.py` flags
  `NotificationPluginError::SmtpNotConfigured` if it appears in non-plugin code.
- After Wave 3h + Wave 7: both scripts exit 0 against the refactored codebase.
- `python3 ci/check_plugin_semantic_boundary.py --help` and dry-run complete without
  config errors.

---

## Full Acceptance Criteria

| Check | Expected |
| ----- | -------- |
| `roles.rs` plugin-named identifiers | Zero |
| `SurfaceActionController` plugin-named methods | Zero (only `tenant_id`, `user_id`, `tenant_db`) |
| `UpdateProtectionController` plugin-named methods | Zero (only `tenant_db`) |
| Plugin-specific store trait impls in `surface-proxy` | Zero |
| Plugin-specific store trait impls in `web-api-queries` | Zero |
| Proxmox entity imports in `web-api-queries` | Zero |
| `uptrakit-tenant-db` SeaORM entity definitions | Zero |
| `plugin_ids::EMAIL` in `users.rs` | Zero |
| `NotificationPluginError::SmtpNotConfigured` in non-plugin code | Zero |
| `./ci/check_plugin_semantic_boundary.sh` | Exits 0 |
| `python3 ci/check_plugin_semantic_boundary.py` | Exits 0 |
| `cargo check --all-features` | Clean |
| `cargo clippy --all-targets --all-features` | Clean |
| `cargo test --all-features` | Passes |
