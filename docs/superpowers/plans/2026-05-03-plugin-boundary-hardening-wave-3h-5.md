# Plugin Boundary Hardening — Waves 3h, 4, 5 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** (1) Lift transactional email into `NotificationOps` so `users.rs` stops importing plugin-specific types. (2) Move the 5 Proxmox entity files
out of `shared-db` into the proxmox plugin crate. (3) Convert `PluginSurfaceActionOps` and `SoftwareItemLifecycleOps` from `Pin<Box<dyn Future>>` to
`#[async_trait]`.

**Architecture:** Wave 3h is independent of 3a–3g and may land in any order. Wave 4 depends on Wave 3 (all store traits removed) because entities must
be imported only from within the Proxmox plugin after the move. Wave 5 cleans up after Waves 3 and 4.

**Tech Stack:** Rust, `async-trait`, SeaORM, `uptrakit-shared-db`

**Prerequisites:** Waves 1–2 merged.

---

## Task 1 (Wave 3h, Part A): Add `TransactionalEmailError` and `send_transactional_email` to `NotificationOps`

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/plugin_ops.rs`
- Modify: `crates/plugins/infrastructure/core/Cargo.toml`

- [ ] **Step 1: Add `TransactionalEmailError` enum to `plugin_ops.rs`**

In `crates/plugins/infrastructure/core/src/plugin_ops.rs`, add after the `PluginOpsError` block:

```rust
/// Error type for transactional email delivery via `NotificationOps::send_transactional_email`.
#[non_exhaustive]
#[derive(Debug)]
pub enum TransactionalEmailError {
    /// No SMTP transport is configured for this tenant.
    NotConfigured,
    /// Delivery was attempted but failed; inner string is the error message.
    DeliveryFailed(String),
}

impl std::fmt::Display for TransactionalEmailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConfigured => write!(f, "email transport not configured"),
            Self::DeliveryFailed(msg) => write!(f, "email delivery failed: {msg}"),
        }
    }
}

impl std::error::Error for TransactionalEmailError {}
```

- [ ] **Step 2: Annotate `NotificationOps` with `#[async_trait]` and add the new method**

`async-trait` is already in `[dependencies]` in `plugin-infrastructure-core/Cargo.toml`. Add the import to the top of `plugin_ops.rs`:

```rust
use async_trait::async_trait;
```

Change `NotificationOps` from:

```rust
pub trait NotificationOps: Send + Sync + 'static {
    fn transport(&self, id: &PluginTypeId) -> Option<std::sync::Arc<dyn NotificationTransport>>;
    fn notification_supported_types(&self) -> Vec<PluginTypeId>;
}
```

To:

```rust
#[async_trait]
pub trait NotificationOps: Send + Sync + 'static {
    fn transport(&self, id: &PluginTypeId) -> Option<std::sync::Arc<dyn NotificationTransport>>;
    fn notification_supported_types(&self) -> Vec<PluginTypeId>;

    async fn send_transactional_email(
        &self,
        tenant_db: &uptrakit_tenant_db::TenantDb,
        to: &str,
        subject: &str,
        text_body: &str,
        html_body: &str,
    ) -> std::result::Result<(), TransactionalEmailError> {
        let _ = (tenant_db, to, subject, text_body, html_body);
        Err(TransactionalEmailError::NotConfigured)
    }
}
```

The default body is an explicit noop — returning `Err(NotConfigured)` when no email transport is available. The `let _ = ...` suppresses
unused-variable warnings for the default implementation.

Note: `#[async_trait]` requires importing the trait — add `use async_trait::async_trait;` at the top of `plugin_ops.rs`. The `uptrakit_tenant_db` dep
is already gated behind `plugin-ops` in `Cargo.toml` (added in Wave 1). Since `NotificationOps` doesn't require `plugin-ops`, gate the `tenant_db`
parameter similarly or use the feature-gated import. Check: does `uptrakit_tenant_db` need to be accessible without the `plugin-ops` feature here?

If `NotificationOps::send_transactional_email` should compile even without `plugin-ops`, the parameter type `&uptrakit_tenant_db::TenantDb` must be
available without the feature. Since `plugin-ops` is the gate for `uptrakit-tenant-db` dep in `Cargo.toml`, you have two options:

**Option A (simpler):** Move `uptrakit-tenant-db` to unconditional deps in `plugin-infrastructure-core/Cargo.toml` — the crate is lightweight (just
`sea-orm`, `uuid`), and `sea-orm` is already pulled in via `plugin-ops`. But adding an unconditional `sea-orm` dep could hurt agent-side builds.

**Option B (correct):** Gate `send_transactional_email` behind `#[cfg(feature = "plugin-ops")]`. Since `NotificationOps` is in `plugin_ops.rs` which
is always compiled, add:

```rust
#[cfg(feature = "plugin-ops")]
async fn send_transactional_email(
    &self,
    tenant_db: &uptrakit_tenant_db::TenantDb,
    ...
) -> std::result::Result<(), TransactionalEmailError> { ... }
```

**Choose Option B** — consistent with how `tenant_db()` on the controller traits is gated.

- [ ] **Step 3: Add `TransactionalEmailError` to `lib.rs` re-exports**

In `crates/plugins/infrastructure/core/src/lib.rs`, add to the `plugin_ops` re-export block:

```rust
pub use plugin_ops::{
    ControllerUpdateProtectionOps, NotificationOps, PluginConfigOps, PluginMetadataOps, PluginOps,
    PluginOpsError, PluginSurfaceActionOps, PluginSurfaceOps, SoftwareItemLifecycleOps,
    TransactionalEmailError,
};
```

- [ ] **Step 4: Compile check**

Run: `cargo check -p uptrakit-plugin-infrastructure-core --features plugin-ops`
Expected: clean

Run: `cargo check -p uptrakit-plugin-infrastructure-core --no-default-features`
Expected: clean (default build doesn't include plugin-ops or the new method)

- [ ] **Step 5: Update all `impl NotificationOps` blocks with `#[async_trait]`**

`#[async_trait]` on the trait requires all impls to also have the attribute.

Sites to update (add `#[async_trait]` to the `impl` header only — the two sync methods `transport` and `notification_supported_types` don't change;
the default for `send_transactional_email` is inherited unless overridden):

1. `crates/plugins/infrastructure/core/src/catalog.rs` — `impl NotificationOps for PluginCatalog` (line 287)
2. `crates/ui/web-api/src/routes/service_ws/handler/updates.rs` — `impl NotificationOps for ProtectionOverridePluginOps` (line 2682)
3. `crates/ui/web-api/src/routes/service_ws/handler/update_tracking.rs` — `impl NotificationOps for ProtectionOverridePluginOps` (line 442)
4. `crates/ui/web-api/src/routes/software_items/mod.rs` — `impl NotificationOps for ProtectionOverridePluginOps` (line 2313)
5. `crates/ui/web-api/src/routes/service_ws/handler/messages.rs` — `impl NotificationOps for TestPluginOps` (line 1909)

For each site, add `use async_trait::async_trait;` if not already imported, then add `#[async_trait]` before the `impl` header.

The stubs (2–5) get the default `send_transactional_email` automatically — no method body needed.

- [ ] **Step 6: Compile check all crates**

Run: `cargo check --all-features`
Expected: clean

- [ ] **Step 7: Commit**

```bash
git add crates/plugins/infrastructure/core/src/plugin_ops.rs crates/plugins/infrastructure/core/src/lib.rs \
  crates/ui/web-api/src/routes/
git commit -m "feat(plugin-ops): add send_transactional_email to NotificationOps; add TransactionalEmailError"
```

---

## Task 2 (Wave 3h, Part B): Override `send_transactional_email` in `PluginCatalog`

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/catalog.rs`
- Modify: `crates/plugins/infrastructure/core/Cargo.toml`

- [ ] **Step 1: Add `uptrakit-shared-db` as a `catalog`-feature-gated dep**

In `crates/plugins/infrastructure/core/Cargo.toml`:

Change `[features]`:

```toml
catalog = ["dep:reqwest", "dep:tokio-util", "http-client", "dep:uptrakit-shared-db"]
```

Add to `[dependencies]`:

```toml
uptrakit-shared-db = { workspace = true, optional = true }
```

- [ ] **Step 2: Override `send_transactional_email` in `PluginCatalog`'s `impl NotificationOps`**

In `crates/plugins/infrastructure/core/src/catalog.rs`, the current `impl NotificationOps for PluginCatalog` has two methods. Add the override (all of
this is already under `#[cfg(feature = "catalog")]` since the file is guarded):

```rust
#[cfg(feature = "plugin-ops")]
async fn send_transactional_email(
    &self,
    tenant_db: &uptrakit_tenant_db::TenantDb,
    to: &str,
    subject: &str,
    text_body: &str,
    html_body: &str,
) -> std::result::Result<(), crate::plugin_ops::TransactionalEmailError> {
    use crate::plugin_ops::TransactionalEmailError;
    use uptrakit_shared_types::plugin_ids;

    let transport = self
        .transport(&plugin_ids::EMAIL)
        .ok_or(TransactionalEmailError::NotConfigured)?;

    let settings = build_email_settings_bag(tenant_db).await;

    let config = serde_json::json!({ "to_addresses": [to] });
    let message = uptrakit_notification_plugin_core::DeliveryMessage::new(
        subject.to_string(),
        text_body.to_string(),
        Some(html_body.to_string()),
        serde_json::Value::Null,
        vec![],
    );

    transport
        .deliver(&config, &settings, &message)
        .await
        .map_err(|e| {
            use uptrakit_notification_plugin_core::NotificationPluginError;
            if matches!(e.current_context(), NotificationPluginError::SmtpNotConfigured) {
                TransactionalEmailError::NotConfigured
            } else {
                TransactionalEmailError::DeliveryFailed(e.to_string())
            }
        })
}
```

- [ ] **Step 3: Add `build_email_settings_bag` helper in `catalog.rs`**

This helper replaces the logic from `web-api-queries/notification_settings.rs`. Add as a module-level free function (not a method) below the `impl`
blocks:

```rust
#[cfg(feature = "plugin-ops")]
async fn build_email_settings_bag(
    tenant_db: &uptrakit_tenant_db::TenantDb,
) -> serde_json::Value {
    const SMTP_PREFIX: &str = "smtp.";
    const GLOBAL_SMTP_PREFIX: &str = "global_smtp.";
    const GLOBAL_TELEGRAM_PREFIX: &str = "global_telegram.";
    const SMTP_PASSWORD_AAD: &str = "uptrakit:settings:smtp_password";
    const GLOBAL_SMTP_PASSWORD_AAD: &str = "uptrakit:settings:global_smtp_password";

    let tenant_map = load_and_process_smtp_map(
        tenant_db.db(), tenant_db.tenant_id, SMTP_PREFIX, SMTP_PASSWORD_AAD, "tenant",
    ).await;

    let mut global_map = load_and_process_global_smtp_map(
        tenant_db.db(), GLOBAL_SMTP_PREFIX, GLOBAL_SMTP_PASSWORD_AAD, "global",
    ).await;

    // Merge Telegram global settings into the global map.
    if let Ok(telegram_raw) = uptrakit_shared_db::raw_settings::load_global_settings_by_prefix(
        tenant_db.db(),
        GLOBAL_TELEGRAM_PREFIX,
    ).await {
        global_map.extend(telegram_raw);
    }

    serde_json::json!({ "tenant": tenant_map, "global": global_map })
}

#[cfg(feature = "plugin-ops")]
async fn load_and_process_smtp_map(
    db: &sea_orm::DatabaseConnection,
    tenant_id: uuid::Uuid,
    prefix: &str,
    password_aad: &'static str,
    scope: &'static str,
) -> serde_json::Map<String, serde_json::Value> {
    match uptrakit_shared_db::raw_settings::load_settings_by_prefix(db, tenant_id, prefix).await {
        Ok(raw) => process_smtp_raw_map(raw, password_aad, scope, Some(tenant_id)),
        Err(e) => {
            tracing::warn!(error = ?e, %tenant_id, scope, "failed to load SMTP settings; using empty");
            serde_json::Map::new()
        }
    }
}

#[cfg(feature = "plugin-ops")]
async fn load_and_process_global_smtp_map(
    db: &sea_orm::DatabaseConnection,
    prefix: &str,
    password_aad: &'static str,
    scope: &'static str,
) -> serde_json::Map<String, serde_json::Value> {
    match uptrakit_shared_db::raw_settings::load_global_settings_by_prefix(db, prefix).await {
        Ok(raw) => process_smtp_raw_map(raw, password_aad, scope, None),
        Err(e) => {
            tracing::warn!(error = ?e, scope, "failed to load global SMTP settings; using empty");
            serde_json::Map::new()
        }
    }
}

#[cfg(feature = "plugin-ops")]
fn process_smtp_raw_map(
    raw: std::collections::HashMap<String, serde_json::Value>,
    password_aad: &str,
    scope: &'static str,
    tenant_id: Option<uuid::Uuid>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::new();
    for (k, v) in raw {
        let str_val = match &v {
            serde_json::Value::String(s) if s.is_empty() => continue,
            serde_json::Value::String(s) => s.clone(),
            _ => {
                map.insert(k, v);
                continue;
            }
        };
        // Decrypt password field.
        if k.ends_with(".password") {
            if let Some(decrypted) = decrypt_smtp_password(str_val, password_aad, scope, tenant_id) {
                map.insert(k, serde_json::Value::String(decrypted));
            }
        } else {
            map.insert(k, serde_json::Value::String(str_val));
        }
    }
    map
}

#[cfg(feature = "plugin-ops")]
fn decrypt_smtp_password(
    raw: String,
    aad: &str,
    scope: &'static str,
    tenant_id: Option<uuid::Uuid>,
) -> Option<String> {
    if !uptrakit_crypto::is_encrypted(&raw) {
        return if raw.is_empty() { None } else { Some(raw) };
    }
    match uptrakit_crypto::decrypt_str(&raw, aad) {
        Ok(v) if v.is_empty() => None,
        Ok(v) => Some(v),
        Err(e) => {
            if let Some(tid) = tenant_id {
                tracing::warn!(error = ?e, %tid, scope, "failed to decrypt SMTP password; using empty");
            } else {
                tracing::warn!(error = ?e, scope, "failed to decrypt SMTP password; using empty");
            }
            None
        }
    }
}
```

Also add needed imports at the top of `catalog.rs`:

```rust
#[cfg(feature = "plugin-ops")]
use uptrakit_crypto;  // already a dep transitively? Check: if not, add to Cargo.toml
```

Check: does `plugin-infrastructure-core` with `catalog` feature already depend on `uptrakit-crypto`? Run `grep "uptrakit-crypto"
crates/plugins/infrastructure/core/Cargo.toml`. If not present, add it:

```toml
uptrakit-crypto = { workspace = true, optional = true }
```

And gate it: `catalog = [..., "dep:uptrakit-crypto"]`.

- [ ] **Step 4: Compile check**

Run: `cargo check -p uptrakit-plugin-infrastructure-core --features catalog,plugin-ops`
Expected: clean

- [ ] **Step 5: Commit**

```bash
git add crates/plugins/infrastructure/core/
git commit -m "feat(catalog): override send_transactional_email in PluginCatalog"
```

---

## Task 3 (Wave 3h, Part C): Replace `send_email_change_emails` in `users.rs`

**Files:**

- Modify: `crates/ui/web-api/src/routes/users.rs`

- [ ] **Step 1: Locate the `send_email_change_emails` function**

It starts at line ~1226. The function signature is:

```rust
async fn send_email_change_emails(
    state: &AppState,
    tenant_id: Uuid,
    new_email: &str,
    old_email: &str,
    confirm_url: &str,
) -> Result<(), String>
```

The function body contains:

- `state.plugin_ops.transport(&uptrakit_shared_types::plugin_ids::EMAIL)` — boundary violation
- `uptrakit_web_api_queries::notification_settings::build_settings_bag(state.db(), tenant_id).await` — boundary violation
- `NotificationPluginError::SmtpNotConfigured` — boundary violation

- [ ] **Step 2: Rewrite `send_email_change_emails`**

Replace the function body. The function still uses `DeliveryMessage`, `escape_html` from the registry — those are retained generic infrastructure.
However, with `send_transactional_email`, we no longer need to manually compose and send two separate messages. Instead, we call
`send_transactional_email` twice.

New implementation:

```rust
async fn send_email_change_emails(
    state: &AppState,
    tenant_id: Uuid,
    new_email: &str,
    old_email: &str,
    confirm_url: &str,
) -> Result<(), String> {
    use uptrakit_plugin_infrastructure_registry::escape_html;

    let tenant_db = uptrakit_web_api_queries::TenantDb::new(state.db().clone(), tenant_id);

    // Message 1: to new address — confirm link
    let body1 = format!(
        "A request was made to change the email on account {old_email}. \
        Confirm your new address by clicking the link below (expires in 24 hours).\n\n\
        {confirm_url}\n\nIf you did not request this, contact your administrator.",
    );
    let body1_html = format!(
        "<p>A request was made to change the email on account \
        <strong>{old_email_esc}</strong>.</p>\
        <p>Confirm your new address by clicking the link below (expires in 24 hours).</p>\
        <p><a href=\"{url}\">{url}</a></p>\
        <p>If you did not request this, contact your administrator.</p>",
        old_email_esc = escape_html(old_email),
        url = escape_html(confirm_url),
    );

    state
        .plugin_ops
        .send_transactional_email(
            &tenant_db,
            new_email,
            "Confirm your new email address — Uptrakit",
            &body1,
            &body1_html,
        )
        .await
        .map_err(|e| match e {
            uptrakit_plugin_infrastructure_core::TransactionalEmailError::NotConfigured => {
                "Email delivery not configured".to_string()
            }
            uptrakit_plugin_infrastructure_core::TransactionalEmailError::DeliveryFailed(_) => {
                "Email delivery failed".to_string()
            }
        })?;

    // Message 2: to old address — notification
    let masked_new = mask_email(new_email);
    let body2 = format!(
        "A request was made to change the email address on account {old_email} \
        to {masked_new}. To cancel this change, sign in and go to Profile → \
        Cancel pending change.",
    );
    let body2_html = format!(
        "<p>A request was made to change the email address on account \
        <strong>{old_email_esc}</strong> to <strong>{masked_new_esc}</strong>.</p>\
        <p>To cancel this change, sign in and go to Profile → Cancel pending change.</p>",
        old_email_esc = escape_html(old_email),
        masked_new_esc = escape_html(&masked_new),
    );

    state
        .plugin_ops
        .send_transactional_email(
            &tenant_db,
            old_email,
            "Email address change requested — Uptrakit",
            &body2,
            &body2_html,
        )
        .await
        .map_err(|e| match e {
            uptrakit_plugin_infrastructure_core::TransactionalEmailError::NotConfigured => {
                "Email delivery not configured".to_string()
            }
            uptrakit_plugin_infrastructure_core::TransactionalEmailError::DeliveryFailed(_) => {
                "Email delivery failed".to_string()
            }
        })?;

    Ok(())
}
```

Note: `state.plugin_ops` is `Arc<dyn PluginOps>` and `PluginOps` supertrait includes `NotificationOps`, so `send_transactional_email` is callable on
it.

Remove the `use uptrakit_plugin_infrastructure_registry::{DeliveryMessage, NotificationPluginError};` import from inside `send_email_change_emails`.
Add `use uptrakit_plugin_infrastructure_registry::escape_html;` or keep as fully-qualified. Check what other parts of `users.rs` import from the
registry to avoid breaking existing imports.

- [ ] **Step 3: Compile check**

Run: `cargo check -p uptrakit-web-api --all-features`
Expected: clean

- [ ] **Step 4: Verify boundary violations eliminated**

Run:

```bash
grep -n "plugin_ids::EMAIL\|NotificationPluginError::SmtpNotConfigured\|build_settings_bag" \
  crates/ui/web-api/src/routes/users.rs
```

Expected: zero results.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/web-api/src/routes/users.rs
git commit -m "refactor(users): replace send_email_change_emails with send_transactional_email"
```

---

## Task 4 (Wave 4): Move Proxmox entity files into the proxmox plugin

**Files:**

- Move: `crates/shared/db/src/entity/proxmox_*.rs` → `crates/plugins/infrastructure/proxmox/src/entity/`
- Create: `crates/plugins/infrastructure/proxmox/src/entity/mod.rs`
- Modify: `crates/shared/db/src/entity/mod.rs`
- Modify: `crates/shared/db/src/entity/tenant_scoped.rs`
- Modify: `crates/plugins/infrastructure/proxmox/src/lib.rs`

**Prerequisite:** Wave 3 complete — all Proxmox entity imports must be only within the proxmox plugin at this point. Verify with:

```bash
grep -rn "proxmox_host_mapping\|proxmox_protection_audit\|proxmox_protection_default\|proxmox_protection_item_override\|proxmox_backup_target_cache" \
  crates/ui/ crates/core/ --include="*.rs"
```

Expected: zero results (all moved to plugin in Wave 3).

- [ ] **Step 1: Create `entity/` directory and move files**

```bash
mkdir -p crates/plugins/infrastructure/proxmox/src/entity
cp crates/shared/db/src/entity/proxmox_host_mapping.rs crates/plugins/infrastructure/proxmox/src/entity/
cp crates/shared/db/src/entity/proxmox_protection_audit.rs crates/plugins/infrastructure/proxmox/src/entity/
cp crates/shared/db/src/entity/proxmox_protection_default.rs crates/plugins/infrastructure/proxmox/src/entity/
cp crates/shared/db/src/entity/proxmox_protection_item_override.rs crates/plugins/infrastructure/proxmox/src/entity/
cp crates/shared/db/src/entity/proxmox_backup_target_cache.rs crates/plugins/infrastructure/proxmox/src/entity/
rm crates/shared/db/src/entity/proxmox_host_mapping.rs
rm crates/shared/db/src/entity/proxmox_protection_audit.rs
rm crates/shared/db/src/entity/proxmox_protection_default.rs
rm crates/shared/db/src/entity/proxmox_protection_item_override.rs
rm crates/shared/db/src/entity/proxmox_backup_target_cache.rs
```

- [ ] **Step 2: Create `crates/plugins/infrastructure/proxmox/src/entity/mod.rs`**

```rust
pub(crate) mod proxmox_backup_target_cache;
pub(crate) mod proxmox_host_mapping;
pub(crate) mod proxmox_protection_audit;
pub(crate) mod proxmox_protection_default;
pub(crate) mod proxmox_protection_item_override;
```

- [ ] **Step 3: Add `pub(crate) mod entity;` to proxmox plugin `lib.rs`**

In `crates/plugins/infrastructure/proxmox/src/lib.rs`, add:

```rust
pub(crate) mod entity;
```

- [ ] **Step 4: Update imports within the proxmox plugin**

All files in the proxmox plugin that currently import `use uptrakit_shared_db::entity::proxmox_*` must change to `use crate::entity::proxmox_*`.

Run: `grep -rn "uptrakit_shared_db::entity::proxmox\|shared_db::entity::proxmox" crates/plugins/infrastructure/proxmox/src/ --include="*.rs"`

For each hit, change `uptrakit_shared_db::entity::proxmox_*` → `crate::entity::proxmox_*`.

- [ ] **Step 5: Remove proxmox entity modules from `shared-db/entity/mod.rs`**

In `crates/shared/db/src/entity/mod.rs`, remove the five lines:

```rust
pub mod proxmox_backup_target_cache;
pub mod proxmox_host_mapping;
pub mod proxmox_protection_audit;
pub mod proxmox_protection_default;
pub mod proxmox_protection_item_override;
```

- [ ] **Step 6: Update `shared-db/entity/tenant_scoped.rs`**

Remove the `proxmox_host_mapping` import from the `use super::{ ... }` block in `tenant_scoped.rs`. The `impl TenantScoped for
proxmox_host_mapping::Entity` block must also be removed — this entity is now in the proxmox crate and the impl should move there.

In `crates/plugins/infrastructure/proxmox/src/entity/proxmox_host_mapping.rs` (after moving), add:

```rust
impl uptrakit_tenant_db::TenantScoped for Entity {
    fn tenant_id_column() -> Self::Column {
        Column::TenantId
    }
}
```

Similarly check `proxmox_protection_default.rs` and `proxmox_backup_target_cache.rs` — if any of them had `impl TenantScoped` in
`shared-db/entity/tenant_scoped.rs`, move those impls into the respective entity files in the proxmox crate. Run:

```bash
grep -n "proxmox" crates/shared/db/src/entity/tenant_scoped.rs
```

For each matching `impl TenantScoped for proxmox_*::Entity` block in `tenant_scoped.rs`, delete it and add the equivalent to the entity file in the
proxmox crate (importing `uptrakit_tenant_db::TenantScoped`).

- [ ] **Step 7: Compile check**

Run: `cargo check --all-features`
Expected: clean — no proxmox entity imports outside the proxmox plugin

- [ ] **Step 8: Verify `shared-db` exports no proxmox entities**

Run:

```bash
grep -n "proxmox" crates/shared/db/src/entity/mod.rs
```

Expected: zero results.

- [ ] **Step 9: Commit**

```bash
git add crates/plugins/infrastructure/proxmox/src/ crates/shared/db/src/entity/
git commit -m "refactor(proxmox): move entity files from shared-db into proxmox plugin crate"
```

---

## Task 5 (Wave 5): Convert `PluginSurfaceActionOps` and `SoftwareItemLifecycleOps` to `#[async_trait]`

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/plugin_ops.rs`
- Modify: `crates/plugins/infrastructure/core/src/catalog.rs`
- Modify: `crates/ui/web-api/src/routes/service_ws/handler/updates.rs`
- Modify: `crates/ui/web-api/src/routes/service_ws/handler/update_tracking.rs`
- Modify: `crates/ui/web-api/src/routes/software_items/mod.rs`
- Modify: `crates/ui/web-api/src/routes/service_ws/handler/messages.rs`

- [ ] **Step 1: Convert `PluginSurfaceActionOps` in `plugin_ops.rs`**

Current (starting from the `// ── Trait 3` section in `plugin_ops.rs`):

```rust
pub trait PluginSurfaceActionOps: Send + Sync + 'static {
    fn handle_surface_action<'a>(
        &'a self,
        ctx: &'a SurfaceActionContext<'a>,
        surface_id: &'a str,
        action_id: &'a str,
        params: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<serde_json::Value, SurfaceActionError>> + Send + 'a>>;
}
```

Replace with:

```rust
#[async_trait]
pub trait PluginSurfaceActionOps: Send + Sync + 'static {
    async fn handle_surface_action(
        &self,
        ctx: &SurfaceActionContext<'_>,
        surface_id: &str,
        action_id: &str,
        params: serde_json::Value,
    ) -> std::result::Result<serde_json::Value, SurfaceActionError>;
}
```

Remove the `use std::pin::Pin;` import at the top of `plugin_ops.rs` if it's no longer needed after converting all three traits. Also remove `use
std::future::Future;` if no longer referenced.

- [ ] **Step 2: Convert `SoftwareItemLifecycleOps` in `plugin_ops.rs`**

Current:

```rust
pub trait SoftwareItemLifecycleOps: Send + Sync + 'static {
    fn on_software_item_created<'a>(
        &'a self,
        event: &'a SoftwareItemCreatedEvent,
        ctx: &'a SoftwareItemLifecycleContext,
    ) -> Pin<Box<dyn Future<Output = Option<SoftwareItemPatch>> + Send + 'a>>;

    fn software_item_lifecycle_plugins(&self) -> &[std::sync::Arc<dyn SoftwareItemLifecycle>];
}
```

Replace with:

```rust
#[async_trait]
pub trait SoftwareItemLifecycleOps: Send + Sync + 'static {
    async fn on_software_item_created(
        &self,
        event: &SoftwareItemCreatedEvent,
        ctx: &SoftwareItemLifecycleContext,
    ) -> Option<SoftwareItemPatch>;

    fn software_item_lifecycle_plugins(&self) -> &[std::sync::Arc<dyn SoftwareItemLifecycle>];
}
```

Note: `software_item_lifecycle_plugins` is a sync method and does not need `#[async_trait]` — it coexists fine in an `#[async_trait]` trait.

- [ ] **Step 3: Check for remaining `Pin<Box<dyn Future` in `plugin_ops.rs`**

After the conversions, `plugin_ops.rs` must have zero occurrences of `Pin<Box<dyn Future` (the `Pin` and `Future` imports from `std` can be removed if
unused):

Run: `grep -n "Pin<Box<dyn Future" crates/plugins/infrastructure/core/src/plugin_ops.rs`
Expected: zero results.

- [ ] **Step 4: Update `impl PluginSurfaceActionOps for PluginCatalog` in `catalog.rs`**

Current (line 252):

```rust
impl PluginSurfaceActionOps for PluginCatalog {
    fn handle_surface_action<'a>(
        &'a self,
        ctx: &'a SurfaceActionContext<'a>,
        surface_id: &'a str,
        action_id: &'a str,
        params: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<serde_json::Value, SurfaceActionError>> + Send + 'a>> {
        Box::pin(async move {
            let handler = self.route_surface_action(surface_id).ok_or_else(|| {
                SurfaceActionError::InvalidInput(format!("no plugin handles surface '{surface_id}'"))
            })?;
            handler(ctx, surface_id, action_id, params).await
        })
    }
}
```

Replace with:

```rust
#[async_trait]
impl PluginSurfaceActionOps for PluginCatalog {
    async fn handle_surface_action(
        &self,
        ctx: &SurfaceActionContext<'_>,
        surface_id: &str,
        action_id: &str,
        params: serde_json::Value,
    ) -> std::result::Result<serde_json::Value, SurfaceActionError> {
        let handler = self.route_surface_action(surface_id).ok_or_else(|| {
            SurfaceActionError::InvalidInput(format!("no plugin handles surface '{surface_id}'"))
        })?;
        handler(ctx, surface_id, action_id, params).await
    }
}
```

- [ ] **Step 5: Update `impl SoftwareItemLifecycleOps for PluginCatalog` in `catalog.rs`**

Current (line 300):

```rust
impl SoftwareItemLifecycleOps for PluginCatalog {
    fn on_software_item_created<'a>(
        &'a self,
        event: &'a SoftwareItemCreatedEvent,
        ctx: &'a SoftwareItemLifecycleContext,
    ) -> Pin<Box<dyn Future<Output = Option<SoftwareItemPatch>> + Send + 'a>> {
        Box::pin(async move {
            // ... body
        })
    }
    fn software_item_lifecycle_plugins(&self) -> &[Arc<dyn SoftwareItemLifecycle>] {
        &self.lifecycle_plugins
    }
}
```

Replace with:

```rust
#[async_trait]
impl SoftwareItemLifecycleOps for PluginCatalog {
    async fn on_software_item_created(
        &self,
        event: &SoftwareItemCreatedEvent,
        ctx: &SoftwareItemLifecycleContext,
    ) -> Option<SoftwareItemPatch> {
        let mut merged: Option<SoftwareItemPatch> = None;
        for plugin in &self.lifecycle_plugins {
            match plugin.on_software_item_created(event, ctx).await {
                Ok(Some(patch)) => {
                    let m = merged.get_or_insert_with(SoftwareItemPatch::new);
                    if patch.icon_url.is_some() {
                        m.icon_url = patch.icon_url;
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(
                        plugin = %plugin.plugin_type_id(),
                        error = %e,
                        "software item lifecycle plugin error"
                    );
                }
            }
        }
        merged
    }

    fn software_item_lifecycle_plugins(&self) -> &[Arc<dyn SoftwareItemLifecycle>] {
        &self.lifecycle_plugins
    }
}
```

Remove `use std::pin::Pin; use std::future::Future;` from `catalog.rs` if they become unused.

- [ ] **Step 6: Update test stubs — add `#[async_trait]` to impl headers**

For each of the 4 test files, add `#[async_trait]` to the `impl PluginSurfaceActionOps` and `impl SoftwareItemLifecycleOps` headers. The stubs only
need the attribute added — the method bodies are already `-> Pin<Box<dyn Future...>>` closures that now need to be plain `async fn`.

**`service_ws/handler/updates.rs` (lines ~2657 and ~2695):**

`impl PluginSurfaceActionOps for ProtectionOverridePluginOps` — change method signature from `fn handle_surface_action<'a>(...)  -> Pin<Box<...>>` to
`async fn handle_surface_action(...)` and remove the `Box::pin(async move { ... })` wrapper if present. Keep the inner `unimplemented!()` or
equivalent.

`impl SoftwareItemLifecycleOps for ProtectionOverridePluginOps` — same conversion.

Repeat for:

- **`service_ws/handler/update_tracking.rs`** (lines ~413 and ~455)
- **`software_items/mod.rs`** (lines ~2284 and ~2326)
- **`service_ws/handler/messages.rs`** (lines ~1880 and ~1921) for `TestPluginOps`

Add `use async_trait::async_trait;` to each file if not already imported.

- [ ] **Step 7: Check `#[async_trait]` is in scope for callers of the converted traits**

Callers using `.await` on `handle_surface_action` and `on_software_item_created` are unaffected — `async_trait` expands to the same `Pin<Box<dyn
Future>>` internally. No call site changes needed.

- [ ] **Step 8: Compile check**

Run: `cargo check --all-features`
Expected: clean

- [ ] **Step 9: Verify no `Pin<Box<dyn Future` remains in `plugin_ops.rs`**

Run: `grep -n "Pin<Box<dyn Future" crates/plugins/infrastructure/core/src/plugin_ops.rs`
Expected: zero results.

- [ ] **Step 10: Commit**

```bash
git add crates/plugins/infrastructure/core/src/plugin_ops.rs \
  crates/plugins/infrastructure/core/src/catalog.rs \
  crates/ui/web-api/src/routes/
git commit -m "refactor(plugin-ops): convert PluginSurfaceActionOps and SoftwareItemLifecycleOps to async_trait"
```

---

## Task 6: `roles.rs` final audit and registry tidy

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/roles.rs`
- Modify: `crates/plugins/infrastructure/registry/src/lib.rs`

- [ ] **Step 1: Grep `roles.rs` for any remaining plugin-named identifiers**

Run:

```bash
grep -n "Proxmox\|Docker\|EmailSmtp\|Telegram\|NotificationChannel" \
  crates/plugins/infrastructure/core/src/roles.rs
```

Expected: zero results. If any remain, delete them.

After Wave 5, `roles.rs` should contain only:

- `PluginMeta`
- `Discoverer`, `VersionDetector`, `ReleaseFetcher`, `PackageIndexer`
- `ExecuteUpdateResult`, `UpdateExecutor`
- `LifecycleHook`
- `NotificationTransport`
- `SoftwareItemLifecycle`, `SoftwareItemCreatedEvent`, `SoftwareItemLifecycleContext`, `SoftwareItemPatch`
- `ControllerUpdateProtection`, `ControllerProtectionContext`, `ControllerProtectionDecision`, `ControllerPostUpdateContext`, `PostUpdateOutcome`
- `SurfaceActionController` (only `tenant_id()`, `user_id()`, `tenant_db()`)
- `UpdateProtectionController` (only `tenant_db()`)

- [ ] **Step 2: Grep registry `lib.rs` for plugin-named store-trait re-exports**

Run:

```bash
grep -n "Proxmox\|Docker\|EmailSmtp\|TelegramGlobal\|NotificationChannelStore\|execute_proxmox" \
  crates/plugins/infrastructure/registry/src/lib.rs
```

Expected: zero results.

Confirm these ARE still present (retained generic infrastructure):

- `DeliveryMessage`
- `escape_html`
- `NotificationPluginError`

- [ ] **Step 3: Run full quality gates**

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
```

All must pass.

- [ ] **Step 4: Commit**

```bash
git add crates/plugins/infrastructure/core/src/roles.rs crates/plugins/infrastructure/registry/
git commit -m "refactor(roles): final audit — roles.rs and registry contain zero plugin-named store types"
```
