# Extract Surface Proxy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract `surface_proxy.rs`, `surface_proxy/`, and `surface_registry.rs` from
`uptrakit-web-api` into a new `uptrakit-surface-proxy` crate at `crates/ui/surface-proxy/`,
following the six-commit sequence defined in ADR-0001.

**Architecture:** Six sequential commits, each leaving the codebase green. Commits 1–3 resolve
coupling pre-conditions. Commit 4 creates the new standalone crate. Commit 5 wires `web-api` to
use it. Commits 4 and 5 must be consecutive with no other commits between them.

**Tech Stack:** Rust, Cargo workspace, Sea-ORM, `uptrakit-shared-db::raw_settings`,
`uptrakit-web-api-queries`, `uptrakit-plugin-infrastructure-registry`.

**Spec:** `docs/superpowers/specs/2026-05-01-extract-surface-proxy-crate-design.md`

---

## File Map

**Created:**

- `crates/ui/service-connections/Cargo.toml`
- `crates/ui/service-connections/src/lib.rs`
- `crates/ui/web-api-queries/src/notification_settings.rs`
- `crates/ui/surface-proxy/Cargo.toml`
- `crates/ui/surface-proxy/src/lib.rs`
- `crates/ui/surface-proxy/src/proxy.rs` (moved from `web-api/src/surface_proxy.rs`)
- `crates/ui/surface-proxy/src/proxy/` (moved from `web-api/src/surface_proxy/`)
- `crates/ui/surface-proxy/src/registry.rs` (moved from `web-api/src/surface_registry.rs`)

**Modified:**

- `Cargo.toml` (root workspace deps)
- `crates/ui/web-api/Cargo.toml`
- `crates/ui/web-api/src/lib.rs`
- `crates/ui/web-api/src/app_state.rs`
- `crates/ui/web-api/src/service_connections.rs` (re-export shim)
- `crates/ui/web-api/src/notifications/dispatcher.rs`
- `crates/ui/web-api/src/surface_proxy/controller_local/settings_store.rs`
- `crates/ui/web-api-queries/src/lib.rs`
- `crates/ui/web-api/src/routes/notifications.rs`
- `crates/ui/web-api/src/routes/users.rs`
- `docs/adr/0001-web-api-decomposition-strategy.md`

**Deleted:**

- `crates/ui/web-api/src/surface_proxy.rs`
- `crates/ui/web-api/src/surface_proxy/` (entire directory)
- `crates/ui/web-api/src/surface_registry.rs`

---

## Task 1: Extract `ServiceConnectionRegistry` to `uptrakit-service-connections`

**Files:**

- Create: `crates/ui/service-connections/Cargo.toml`
- Create: `crates/ui/service-connections/src/lib.rs`
- Modify: `Cargo.toml` (root)
- Modify: `crates/ui/web-api/Cargo.toml`
- Modify: `crates/ui/web-api/src/service_connections.rs`

- [ ] **Step 1: Create crate directory and Cargo.toml**

```bash
mkdir -p crates/ui/service-connections/src
```

Write `crates/ui/service-connections/Cargo.toml`:

```toml
[package]
name = "uptrakit-service-connections"
description = "Uptrakit service connection registry"
edition.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
version.workspace = true

[lints]
workspace = true

[dependencies]
futures-util  = { workspace = true }
parking_lot   = { workspace = true }
rand          = { workspace = true }
time          = { workspace = true }
tokio         = { workspace = true }
tokio-util    = { workspace = true }
tracing       = { workspace = true }
uptrakit-wire = { workspace = true }
uuid          = { workspace = true }
```

- [ ] **Step 2: Copy `service_connections.rs` to the new crate**

`crates/ui/service-connections/src/lib.rs` is an exact copy of
`crates/ui/web-api/src/service_connections.rs`. Make no logic changes — copy as-is.

```bash
cp crates/ui/web-api/src/service_connections.rs crates/ui/service-connections/src/lib.rs
```

- [ ] **Step 3: Register in workspace `Cargo.toml`**

In the root `Cargo.toml`, under `[workspace.dependencies]`, add after
`uptrakit-notification-delivery`:

```toml
uptrakit-service-connections = { path = "crates/ui/service-connections", version = "0.0.1" }
```

- [ ] **Step 4: Add dep to `web-api/Cargo.toml`**

In `crates/ui/web-api/Cargo.toml`, under `[dependencies]`, add:

```toml
uptrakit-service-connections = { workspace = true }
```

- [ ] **Step 5: Replace `service_connections.rs` with re-export shim**

Overwrite `crates/ui/web-api/src/service_connections.rs` with:

```rust
pub use uptrakit_service_connections::ServiceConnectionRegistry;
```

- [ ] **Step 6: Verify codebase compiles**

```bash
cargo check --no-default-features --features db-sqlite
cargo check --all-features
```

Expected: no errors.

- [ ] **Step 7: Run web-api test suite**

```bash
cargo test -p uptrakit-web-api --all-features 2>&1 | tail -5
```

Expected: all tests pass.

- [ ] **Step 8: Commit**

```bash
git add crates/ui/service-connections/ Cargo.toml crates/ui/web-api/Cargo.toml crates/ui/web-api/src/service_connections.rs
git commit -m "feat(service-connections): extract ServiceConnectionRegistry to uptrakit-service-connections"
```

---

## Task 2: Fix `uptrakit-web-api-auth` Coupling in `settings_store.rs`

**Files:**

- Modify: `crates/ui/web-api/src/surface_proxy/controller_local/settings_store.rs`

**Context:** `settings_store.rs` calls five functions from `uptrakit_web_api_auth::settings_store`.
These are thin wrappers around `uptrakit_shared_db::raw_settings`. Replace each with the direct
`raw_settings` call. `uptrakit_shared_db` is already in `web-api`'s dependencies.

- [ ] **Step 1: Replace `load_typed_settings_by_prefix` call**

In `settings_store.rs`, find the `load_tenant_smtp_settings` implementation (around line 42).
Replace:

```rust
let settings = uptrakit_web_api_auth::settings_store::load_typed_settings_by_prefix(
    self.db(),
    tenant_id,
    SMTP_PREFIX,
)
.await
.map_err(plugin_internal_error)?;
```

With:

```rust
let settings = uptrakit_shared_db::raw_settings::load_settings_by_prefix(
    self.db(),
    tenant_id,
    SMTP_PREFIX,
)
.await
.and_then(|r| uptrakit_shared_db::raw_settings::decode_prefixed_settings(SMTP_PREFIX, &r))
.map_err(plugin_internal_error)?;
```

- [ ] **Step 2: Replace `load_typed_global_settings_by_prefix` call**

In `load_global_smtp_settings` (around line 69). Replace:

```rust
let settings = uptrakit_web_api_auth::settings_store::load_typed_global_settings_by_prefix(
    self.db(),
    GLOBAL_SMTP_PREFIX,
)
.await
.map_err(plugin_internal_error)?;
```

With:

```rust
let settings = uptrakit_shared_db::raw_settings::load_global_settings_by_prefix(
    self.db(),
    GLOBAL_SMTP_PREFIX,
)
.await
.and_then(|r| uptrakit_shared_db::raw_settings::decode_prefixed_settings(GLOBAL_SMTP_PREFIX, &r))
.map_err(plugin_internal_error)?;
```

- [ ] **Step 3: Replace `load_global_settings_by_prefix` call**

In `load_global_bot_token` (around line 108). Replace:

```rust
let map = uptrakit_web_api_auth::settings_store::load_global_settings_by_prefix(
    self.db(),
    GLOBAL_TELEGRAM_PREFIX,
)
.await
.map_err(plugin_internal_error)?;
```

With:

```rust
let map = uptrakit_shared_db::raw_settings::load_global_settings_by_prefix(
    self.db(),
    GLOBAL_TELEGRAM_PREFIX,
)
.await
.map_err(plugin_internal_error)?;
```

- [ ] **Step 4: Replace `upsert_global_setting_raw` call in `save_global_bot_token`**

Around line 126. Replace:

```rust
uptrakit_web_api_auth::settings_store::upsert_global_setting_raw(
    self.db(),
    KEY_GLOBAL_TELEGRAM_BOT_TOKEN,
    serde_json::json!(bot_token),
)
.await
.map_err(plugin_internal_error)?;
```

With:

```rust
uptrakit_shared_db::raw_settings::upsert_global_setting_raw(
    self.db(),
    KEY_GLOBAL_TELEGRAM_BOT_TOKEN,
    serde_json::json!(bot_token),
)
.await
.map_err(plugin_internal_error)?;
```

- [ ] **Step 5: Replace `upsert_setting_raw` private helper function**

Around line 347. Replace the entire `upsert_tenant_setting_raw` function body:

```rust
async fn upsert_tenant_setting_raw(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    key: &str,
    value: Option<serde_json::Value>,
) -> uptrakit_plugin_infrastructure_registry::PluginResult<()> {
    let payload = value.unwrap_or(serde_json::Value::Null);
    uptrakit_web_api_auth::settings_store::upsert_setting_raw(db, tenant_id, key, payload)
        .await
        .map_err(plugin_internal_error)
}
```

With:

```rust
async fn upsert_tenant_setting_raw(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    key: &str,
    value: Option<serde_json::Value>,
) -> uptrakit_plugin_infrastructure_registry::PluginResult<()> {
    let payload = value.unwrap_or(serde_json::Value::Null);
    uptrakit_shared_db::raw_settings::upsert_setting_raw(db, tenant_id, key, payload)
        .await
        .map_err(plugin_internal_error)
}
```

- [ ] **Step 6: Replace `upsert_global_setting_raw` private helper function**

Around line 359. Replace the entire `upsert_global_setting_raw` function body:

```rust
async fn upsert_global_setting_raw(
    db: &sea_orm::DatabaseConnection,
    key: &str,
    value: Option<serde_json::Value>,
) -> uptrakit_plugin_infrastructure_registry::PluginResult<()> {
    let payload = value.unwrap_or(serde_json::Value::Null);
    uptrakit_web_api_auth::settings_store::upsert_global_setting_raw(db, key, payload)
        .await
        .map_err(plugin_internal_error)
}
```

With:

```rust
async fn upsert_global_setting_raw(
    db: &sea_orm::DatabaseConnection,
    key: &str,
    value: Option<serde_json::Value>,
) -> uptrakit_plugin_infrastructure_registry::PluginResult<()> {
    let payload = value.unwrap_or(serde_json::Value::Null);
    uptrakit_shared_db::raw_settings::upsert_global_setting_raw(db, key, payload)
        .await
        .map_err(plugin_internal_error)
}
```

- [ ] **Step 7: Remove unused `uptrakit_web_api_auth` import from `settings_store.rs`**

The file previously imported from `uptrakit_web_api_auth::settings_store`. After replacing all
six call sites, there is no longer a direct reference to that crate. Remove any
`use uptrakit_web_api_auth::...` lines at the top of `settings_store.rs` if present (the calls
were inlined, so there may be no top-level `use` to remove — the crate was invoked
fully-qualified). Run `cargo check` to confirm.

- [ ] **Step 8: Verify**

```bash
cargo check --no-default-features --features db-sqlite
cargo check --all-features
```

Expected: no errors.

- [ ] **Step 9: Run tests**

```bash
cargo test -p uptrakit-web-api --all-features 2>&1 | tail -5
```

Expected: all tests pass.

- [ ] **Step 10: Commit**

```bash
git add crates/ui/web-api/src/surface_proxy/controller_local/settings_store.rs
git commit -m "refactor(surface-proxy): replace uptrakit-web-api-auth coupling in settings_store with direct raw_settings calls"
```

---

## Task 3: Move `build_settings_bag` to `uptrakit-web-api-queries`

**Files:**

- Create: `crates/ui/web-api-queries/src/notification_settings.rs`
- Modify: `crates/ui/web-api-queries/src/lib.rs`
- Modify: `crates/ui/web-api/src/notifications/dispatcher.rs`
- Modify: `crates/ui/web-api/src/surface_proxy/controller_local/notifications.rs`

- [ ] **Step 1: Create `notification_settings.rs` in `web-api-queries`**

Write `crates/ui/web-api-queries/src/notification_settings.rs` — extracted and promoted
from `dispatcher.rs`. Change `pub(crate)` → `pub` on `build_settings_bag` only; all
helpers remain private:

```rust
use sea_orm::DatabaseConnection;
use uuid::Uuid;

use uptrakit_plugin_infrastructure_registry::EmailSmtpSettings;

const SMTP_PREFIX: &str = "smtp.";
const GLOBAL_SMTP_PREFIX: &str = "global_smtp.";
const GLOBAL_TELEGRAM_PREFIX: &str = "global_telegram.";
const SMTP_PASSWORD_AAD: &str = "uptrakit:settings:smtp_password";
const GLOBAL_SMTP_PASSWORD_AAD: &str = "uptrakit:settings:global_smtp_password";

pub async fn build_settings_bag(
    db: &DatabaseConnection,
    tenant_id: Uuid,
) -> serde_json::Value {
    let tenant_smtp = typed_smtp_settings_or_empty(
        {
            let raw = uptrakit_shared_db::raw_settings::load_settings_by_prefix(
                db,
                tenant_id,
                SMTP_PREFIX,
            )
            .await;
            raw.and_then(|r| {
                uptrakit_shared_db::raw_settings::decode_prefixed_settings(SMTP_PREFIX, &r)
            })
        },
        "tenant",
        Some(tenant_id),
        SMTP_PASSWORD_AAD,
    );

    let global_smtp = typed_smtp_settings_or_empty(
        {
            let raw = uptrakit_shared_db::raw_settings::load_global_settings_by_prefix(
                db,
                GLOBAL_SMTP_PREFIX,
            )
            .await;
            raw.and_then(|r| {
                uptrakit_shared_db::raw_settings::decode_prefixed_settings(GLOBAL_SMTP_PREFIX, &r)
            })
        },
        "global",
        None,
        GLOBAL_SMTP_PASSWORD_AAD,
    );

    let global_telegram = uptrakit_shared_db::raw_settings::load_global_settings_by_prefix(
        db,
        GLOBAL_TELEGRAM_PREFIX,
    )
    .await
    .unwrap_or_default();

    let mut global = smtp_settings_to_prefixed_map(GLOBAL_SMTP_PREFIX, &global_smtp);
    for (k, v) in &global_telegram {
        global.insert(k.clone(), v.clone());
    }

    let tenant_map = smtp_settings_to_prefixed_map(SMTP_PREFIX, &tenant_smtp);

    serde_json::json!({ "tenant": tenant_map, "global": global })
}

fn typed_smtp_settings_or_empty(
    result: uptrakit_shared_db::raw_settings::Result<EmailSmtpSettings>,
    scope: &'static str,
    tenant_id: Option<Uuid>,
    password_aad: &str,
) -> EmailSmtpSettings {
    match result {
        Ok(settings) => normalize_smtp_settings(settings, password_aad, scope, tenant_id),
        Err(error) => {
            if let Some(tenant_id) = tenant_id {
                tracing::warn!(
                    error = ?error,
                    %tenant_id,
                    scope,
                    "failed to load typed SMTP settings for notification dispatch; using empty fallback"
                );
            } else {
                tracing::warn!(
                    error = ?error,
                    scope,
                    "failed to load typed SMTP settings for notification dispatch; using empty fallback"
                );
            }
            EmailSmtpSettings::default()
        }
    }
}

fn normalize_smtp_settings(
    settings: EmailSmtpSettings,
    password_aad: &str,
    scope: &'static str,
    tenant_id: Option<Uuid>,
) -> EmailSmtpSettings {
    EmailSmtpSettings {
        host: normalize_non_empty_string(settings.host),
        port: settings.port,
        username: normalize_non_empty_string(settings.username),
        password: decode_smtp_password(settings.password, password_aad, scope, tenant_id),
        from_address: normalize_non_empty_string(settings.from_address),
        from_name: normalize_non_empty_string(settings.from_name),
        tls_mode: normalize_non_empty_string(settings.tls_mode),
        helo_host: normalize_non_empty_string(settings.helo_host),
    }
}

fn normalize_non_empty_string(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.is_empty())
}

fn decode_smtp_password(
    value: Option<String>,
    aad: &str,
    scope: &'static str,
    tenant_id: Option<Uuid>,
) -> Option<String> {
    let raw = normalize_non_empty_string(value)?;

    if uptrakit_crypto::is_encrypted(&raw) {
        return match uptrakit_crypto::decrypt_str(&raw, aad) {
            Ok(value) => normalize_non_empty_string(Some(value)),
            Err(error) => {
                if let Some(tenant_id) = tenant_id {
                    tracing::warn!(
                        error = ?error,
                        %tenant_id,
                        scope,
                        "failed to decrypt SMTP password for notification dispatch; using empty fallback"
                    );
                } else {
                    tracing::warn!(
                        error = ?error,
                        scope,
                        "failed to decrypt SMTP password for notification dispatch; using empty fallback"
                    );
                }
                None
            }
        };
    }

    Some(raw)
}

fn smtp_settings_to_prefixed_map(
    prefix: &str,
    settings: &EmailSmtpSettings,
) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::new();

    insert_prefixed_string(&mut map, prefix, "host", settings.host.as_deref());
    insert_prefixed_u16(&mut map, prefix, "port", settings.port);
    insert_prefixed_string(&mut map, prefix, "username", settings.username.as_deref());
    insert_prefixed_string(&mut map, prefix, "password", settings.password.as_deref());
    insert_prefixed_string(
        &mut map,
        prefix,
        "from_address",
        settings.from_address.as_deref(),
    );
    insert_prefixed_string(&mut map, prefix, "from_name", settings.from_name.as_deref());
    insert_prefixed_string(&mut map, prefix, "tls_mode", settings.tls_mode.as_deref());
    insert_prefixed_string(&mut map, prefix, "helo_host", settings.helo_host.as_deref());

    map
}

fn insert_prefixed_string(
    map: &mut serde_json::Map<String, serde_json::Value>,
    prefix: &str,
    suffix: &str,
    value: Option<&str>,
) {
    if let Some(value) = value {
        map.insert(
            format!("{prefix}{suffix}"),
            serde_json::Value::String(value.to_string()),
        );
    }
}

fn insert_prefixed_u16(
    map: &mut serde_json::Map<String, serde_json::Value>,
    prefix: &str,
    suffix: &str,
    value: Option<u16>,
) {
    if let Some(value) = value {
        map.insert(format!("{prefix}{suffix}"), serde_json::json!(value));
    }
}
```

- [ ] **Step 2: Register module in `web-api-queries/src/lib.rs`**

Add one line to `crates/ui/web-api-queries/src/lib.rs` — place it before `pub mod notifier;`
(alphabetical order). Do **not** replace the file; append only the new `mod` declaration:

```rust
pub mod notification_settings;  // add this line before pub mod notifier;
```

The existing `pub use notifier::ServiceNotifier;` and `pub use tenant_db::TenantDb;`
re-exports must remain untouched.

- [ ] **Step 3: Update `dispatcher.rs` — replace definition with `pub(crate) use`**

In `crates/ui/web-api/src/notifications/dispatcher.rs`, delete the `build_settings_bag`
function definition and all its helper functions and constants (lines 17–22 constants
`SMTP_PREFIX` etc., plus `build_settings_bag`, `typed_smtp_settings_or_empty`,
`normalize_smtp_settings`, `normalize_non_empty_string`, `decode_smtp_password`,
`smtp_settings_to_prefixed_map`, `insert_prefixed_string`, `insert_prefixed_u16`).

Also delete the `EmailSmtpSettings` import from `uptrakit_plugin_infrastructure_registry`
since it is no longer used directly in `dispatcher.rs`.

Replace with a single `pub(crate) use` line (place it after the remaining `use` declarations
at the top of the file):

```rust
pub(crate) use uptrakit_web_api_queries::notification_settings::build_settings_bag;
```

The `dispatch_loop` function's call to `build_settings_bag` (line ~374) is unchanged — it
resolves through the re-export.

- [ ] **Step 4: Update dead-code `controller_local/notifications.rs`**

In `crates/ui/web-api/src/surface_proxy/controller_local/notifications.rs`:

a) Find the `build_settings_bag` call (line ~123) and replace:

```rust
crate::notifications::dispatcher::build_settings_bag(tenant_db.db(), tenant_db.tenant_id)
```

With:

```rust
uptrakit_web_api_queries::notification_settings::build_settings_bag(tenant_db.db(), tenant_db.tenant_id)
```

b) Also update the three `crate::queries::notifications::` calls (lines ~58, ~69, ~83) to use
the fully-qualified path (this file is compiled as a module even though it's dead code):

```rust
// before (three occurrences):
crate::queries::notifications::create_channel(...)
crate::queries::notifications::update_channel(...)
crate::queries::notifications::delete_channel(...)

// after:
uptrakit_web_api_queries::queries::notifications::create_channel(...)
uptrakit_web_api_queries::queries::notifications::update_channel(...)
uptrakit_web_api_queries::queries::notifications::delete_channel(...)
```

- [ ] **Step 5: Verify**

```bash
cargo check --no-default-features --features db-sqlite
cargo check --all-features
```

Expected: no errors.

- [ ] **Step 6: Run tests**

```bash
cargo test -p uptrakit-web-api-queries --all-features 2>&1 | tail -5
cargo test -p uptrakit-web-api --all-features 2>&1 | tail -5
```

Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/ui/web-api-queries/src/notification_settings.rs \
        crates/ui/web-api-queries/src/lib.rs \
        crates/ui/web-api/src/notifications/dispatcher.rs \
        crates/ui/web-api/src/surface_proxy/controller_local/notifications.rs
git commit -m "refactor(web-api-queries): move build_settings_bag + SMTP helpers to notification_settings module"
```

---

## Task 4: Create `uptrakit-surface-proxy` Crate

**Files:**

- Create: `crates/ui/surface-proxy/Cargo.toml`
- Create: `crates/ui/surface-proxy/src/lib.rs`
- Move (git mv): `web-api/src/surface_proxy.rs` → `surface-proxy/src/proxy.rs`
- Move (git mv): `web-api/src/surface_proxy/` → `surface-proxy/src/proxy/`
- Move (git mv): `web-api/src/surface_registry.rs` → `surface-proxy/src/registry.rs`
- Modify: `Cargo.toml` (root workspace)
- Edit (in place): all moved files — import paths, visibility

This commit **must not** modify `web-api`'s `lib.rs` or `app_state.rs` — the
original files stay in place until Task 5. Commits 4 and 5 must be consecutive.

- [ ] **Step 1: Create crate directory structure**

```bash
mkdir -p crates/ui/surface-proxy/src/proxy
```

Do **not** pre-create `controller_local/` here. `git mv` on a directory moves it INTO an
existing destination, which would produce the wrong nesting
(`proxy/controller_local/controller_local/`). Create only the parent `proxy/` dir; git will
create `controller_local/` as part of the directory move in Step 4.

- [ ] **Step 2: Write `crates/ui/surface-proxy/Cargo.toml`**

```toml
[package]
name = "uptrakit-surface-proxy"
description = "Uptrakit surface proxy and surface registry"
edition.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
version.workspace = true

[lints]
workspace = true

[features]
default = []
db-all                 = ["db-sqlite", "db-postgres"]
db-sqlite              = ["sea-orm/sqlx-sqlite", "uptrakit-web-api-queries/db-sqlite"]
db-postgres            = ["sea-orm/sqlx-postgres", "uptrakit-web-api-queries/db-postgres"]
notifications-email    = ["uptrakit-plugin-infrastructure-registry/notifications-email"]
notifications-telegram = ["uptrakit-plugin-infrastructure-registry/notifications-telegram"]
notifications-all      = ["notifications-email", "notifications-telegram"]

[dependencies]
async-trait = { workspace = true }
parking_lot = { workspace = true }
rand        = { workspace = true }
rootcause   = { workspace = true }
sea-orm     = { workspace = true }
serde       = { workspace = true }
serde_json  = { workspace = true }
time        = { workspace = true }
tokio       = { workspace = true }
tracing     = { workspace = true }
uuid        = { workspace = true }
uptrakit-audit-log                      = { workspace = true, features = ["db"] }
uptrakit-crypto                         = { workspace = true, features = ["sea-orm"] }
uptrakit-notification-delivery          = { workspace = true }
uptrakit-plugin-infrastructure-registry = { workspace = true, features = ["notifications", "notifications-webhook"] }
uptrakit-service-connections            = { workspace = true }
uptrakit-shared-db                      = { workspace = true }
uptrakit-shared-types                   = { workspace = true }
uptrakit-web-api-queries                = { workspace = true }
uptrakit-web-api-types                  = { workspace = true }
uptrakit-wire                           = { workspace = true }
```

- [ ] **Step 3: Move `surface_proxy.rs` and `surface_registry.rs`**

```bash
git mv crates/ui/web-api/src/surface_proxy.rs crates/ui/surface-proxy/src/proxy.rs
git mv crates/ui/web-api/src/surface_registry.rs crates/ui/surface-proxy/src/registry.rs
```

- [ ] **Step 4: Move all `surface_proxy/` submodules**

Move the `controller_local/` directory first (before the `.rs` file) so git mv has a
non-existing destination and creates the directory rather than nesting it:

```bash
git mv crates/ui/web-api/src/surface_proxy/controller_local      crates/ui/surface-proxy/src/proxy/controller_local
git mv crates/ui/web-api/src/surface_proxy/controller_local.rs   crates/ui/surface-proxy/src/proxy/controller_local.rs
git mv crates/ui/web-api/src/surface_proxy/entity_enrichment.rs  crates/ui/surface-proxy/src/proxy/entity_enrichment.rs
git mv crates/ui/web-api/src/surface_proxy/bookkeeping.rs        crates/ui/surface-proxy/src/proxy/bookkeeping.rs
git mv crates/ui/web-api/src/surface_proxy/dispatch.rs           crates/ui/surface-proxy/src/proxy/dispatch.rs
git mv crates/ui/web-api/src/surface_proxy/idempotency.rs        crates/ui/surface-proxy/src/proxy/idempotency.rs
git mv crates/ui/web-api/src/surface_proxy/local_executor.rs     crates/ui/surface-proxy/src/proxy/local_executor.rs
git mv crates/ui/web-api/src/surface_proxy/prepared.rs           crates/ui/surface-proxy/src/proxy/prepared.rs
git mv crates/ui/web-api/src/surface_proxy/validation.rs         crates/ui/surface-proxy/src/proxy/validation.rs
git mv crates/ui/web-api/src/surface_proxy/tests.rs              crates/ui/surface-proxy/src/proxy/tests.rs
git mv crates/ui/web-api/src/surface_proxy/tests                 crates/ui/surface-proxy/src/proxy/tests
```

- [ ] **Step 5: Write `crates/ui/surface-proxy/src/lib.rs`**

```rust
mod proxy;
mod registry;

pub use proxy::{
    AppStateSurfaceActionController,
    PluginOpsSurfaceActionInvoker,
    PluginSurfaceActionInvoker,
    PluginSurfaceLocalExecutor,
    SurfaceCallerOrigin,
    SurfaceInvokeRequest,
    SurfaceLocalActionExecutor,
    SurfaceProxy,
    SurfaceProxyError,
};
pub use proxy::entity_enrichment;
pub use registry::{
    ResolvedSurfaceAction,
    ResolvedSurfaceRead,
    SurfaceCatalogItem,
    SurfaceProviderRejection,
    SurfaceProviderRejectionCode,
    SurfaceProviderRejectionReason,
    SurfaceProviderSummary,
    SurfaceRegistry,
    SurfaceRegistryConfig,
    SurfaceRegistryError,
    SurfaceRegistryLookupError,
};
```

- [ ] **Step 6: Fix module declarations in `proxy.rs`**

In `crates/ui/surface-proxy/src/proxy.rs` (the moved `surface_proxy.rs`):

a) Change `pub(crate) mod entity_enrichment;` to `pub mod entity_enrichment;`

b) Change `pub(crate) use controller_local::AppStateSurfaceActionController;` to
`pub use controller_local::AppStateSurfaceActionController;` — `lib.rs` re-exports it
publicly, which requires the intermediate re-export to also be `pub`.

c) The existing `mod controller_local;` declaration is correct — leave it.

c) Update the top-level import from `crate::service_connections::ServiceConnectionRegistry`:

```rust
// before (line ~19)
use crate::service_connections::ServiceConnectionRegistry;

// after
use uptrakit_service_connections::ServiceConnectionRegistry;
```

d) Update `crate::surface_registry` references throughout `proxy.rs`. These appear inline as
`crate::surface_registry::SomeName` (not as top-level `use` imports). Replace all occurrences:

```text
crate::surface_registry::  →  crate::registry::
```

Affected lines (approx): 20, 154, 327, 1836, 2338, 2418, 2607 and the inline test at the
bottom. Use a global search-replace within the file.

e) Update notification queries path (lines ~845, ~856, ~870):

```rust
// before
crate::queries::notifications::create_channel(...)
crate::queries::notifications::update_channel(...)
crate::queries::notifications::delete_channel(...)

// after
uptrakit_web_api_queries::queries::notifications::create_channel(...)
uptrakit_web_api_queries::queries::notifications::update_channel(...)
uptrakit_web_api_queries::queries::notifications::delete_channel(...)
```

f) Update `build_settings_bag` call (line ~1556):

```rust
// before
crate::notifications::dispatcher::build_settings_bag(tenant_db.db(), tenant_db.tenant_id)

// after
uptrakit_web_api_queries::notification_settings::build_settings_bag(tenant_db.db(), tenant_db.tenant_id)
```

- [ ] **Step 7: Fix visibility and `from_app_state` in `proxy/controller_local.rs`**

In `crates/ui/surface-proxy/src/proxy/controller_local.rs`:

a) Change `pub(crate) struct AppStateSurfaceActionController` → `pub struct AppStateSurfaceActionController`

b) Change `pub(crate) fn from_database_connection` → `pub fn from_database_connection`

c) **Delete the `from_app_state` function entirely** (it takes `&crate::AppState` which does not
exist in this crate):

```rust
// Delete this entire function:
pub(crate) fn from_app_state(
    state: &'a crate::AppState,
    tenant_id: Uuid,
    caller_user_id: Option<Uuid>,
) -> Self {
    Self::from_database_connection(
        state.db(),
        state.plugin_ops.as_ref(),
        tenant_id,
        caller_user_id,
    )
}
```

d) Replace `crate::service_connections::ServiceConnectionRegistry` with
`uptrakit_service_connections::ServiceConnectionRegistry` where it appears in
`controller_local.rs`.

e) Replace `crate::surface_registry::` with `crate::registry::` throughout.

f) Replace `crate::queries::notifications::find_log_by_action_token(...)` (line ~233) with
`uptrakit_web_api_queries::queries::notifications::find_log_by_action_token(...)`.

- [ ] **Step 8: Promote `enrich_entity_links` visibility in `proxy/entity_enrichment.rs`**

In `crates/ui/surface-proxy/src/proxy/entity_enrichment.rs`:

Change:

```rust
pub(crate) async fn enrich_entity_links(
```

To:

```rust
pub async fn enrich_entity_links(
```

This is required because `lib.rs` re-exports `proxy::entity_enrichment` publicly. Rust rejects
a `pub use` of a `pub(crate)` item with wider visibility under `unreachable_pub = "deny"`.

- [ ] **Step 9: Update stale `crate::` references in all orphaned files**

These files are not declared as modules (`mod X;`) so they compile only when wired. Update
their stale paths now to prevent hidden breakage when the `local_executor.rs` wiring spec
wires them in.

**`proxy/dispatch.rs`** — two `crate::surface_registry::ResolvedSurfaceAction` refs (lines 7, 66):

```rust
// before
use crate::surface_registry::ResolvedSurfaceAction;       // or inline form
// after
use crate::registry::ResolvedSurfaceAction;
```

**`proxy/prepared.rs`** — uses `crate::service_connections::ServiceConnectionRegistry` and
`crate::surface_registry::*`. Replace:

```text
crate::service_connections::ServiceConnectionRegistry
  →  uptrakit_service_connections::ServiceConnectionRegistry
crate::surface_registry::  →  crate::registry::
```

**`proxy/validation.rs`** — same replacements as `prepared.rs`:

```text
crate::service_connections::ServiceConnectionRegistry
  →  uptrakit_service_connections::ServiceConnectionRegistry
crate::surface_registry::  →  crate::registry::
```

**`proxy/local_executor.rs`** — replace any `crate::surface_registry::` refs with
`crate::registry::` and any `crate::service_connections::` with `uptrakit_service_connections::`.

- [ ] **Step 10: Register in workspace `Cargo.toml`**

In root `Cargo.toml` under `[workspace.dependencies]`, add:

```toml
uptrakit-surface-proxy = { path = "crates/ui/surface-proxy", version = "0.0.1", default-features = false }
```

- [ ] **Step 11: Verify new crate compiles and tests pass**

```bash
cargo check -p uptrakit-surface-proxy --no-default-features --features db-sqlite
cargo check -p uptrakit-surface-proxy --all-features
cargo test -p uptrakit-surface-proxy --all-features 2>&1 | tail -10
```

Expected: no errors, all inline tests pass.

- [ ] **Step 12: Note — web-api is intentionally broken after this commit**

`git mv` in Steps 3–4 removed `surface_proxy.rs`, `surface_proxy/`, and `surface_registry.rs`
from web-api's source tree. web-api's `lib.rs` still declares `pub mod surface_proxy;` and
`pub mod surface_registry;`, so `cargo check -p uptrakit-web-api` will fail here. **This is
expected.** Task 5 (the next commit) wires the shims and restores green. Do not run a
`cargo check -p uptrakit-web-api` check at this stage.

- [ ] **Step 13: Commit**

```bash
git add crates/ui/surface-proxy/ Cargo.toml
git commit -m "feat(surface-proxy): create uptrakit-surface-proxy crate scaffold with all files moved in"
```

---

## Task 5: Wire `web-api` to Use `uptrakit-surface-proxy`

**Files:**

- Modify: `crates/ui/web-api/Cargo.toml`
- Modify: `crates/ui/web-api/src/lib.rs`
- Modify: `crates/ui/web-api/src/app_state.rs`
- Modify: `crates/ui/web-api/src/notifications/dispatcher.rs`
- Modify: `crates/ui/web-api/src/routes/notifications.rs`
- Modify: `crates/ui/web-api/src/routes/users.rs`
- Delete: `crates/ui/web-api/src/surface_proxy.rs`
- Delete: `crates/ui/web-api/src/surface_proxy/` (entire directory)
- Delete: `crates/ui/web-api/src/surface_registry.rs`
- Update: All `state.surface_registry.*` and `state.surface_proxy.*` access sites
- Update: All inline `AppState` construction sites

**Pre-commit verification:** Before making any changes, run these greps to produce
exhaustive site inventories:

```bash
# All inline AppState construction sites (both forms)
grep -rn "surface_registry[,:]" crates/ui/web-api/src/ | grep -v "::"
grep -rn "surface_proxy[,:]" crates/ui/web-api/src/ | grep -v "::"

# All remaining build_settings_bag callers in web-api
grep -rn "build_settings_bag" crates/ui/web-api/src/

# All notifications-* cfg guards remaining after surface files are deleted
grep -rn 'cfg(feature = "notifications' crates/ui/web-api/src/
```

Compare the grep output against the lists in this task. Add any additional sites you find.

- [ ] **Step 1: Update `web-api/Cargo.toml`**

Add `uptrakit-surface-proxy` dependency under `[dependencies]`:

```toml
uptrakit-surface-proxy = { workspace = true }
```

Update the `notifications-email` and `notifications-telegram` feature definitions to forward
through `uptrakit-surface-proxy` instead of pointing directly to
`uptrakit-plugin-infrastructure-registry`:

```toml
# before
notifications-telegram = ["uptrakit-plugin-infrastructure-registry/notifications-telegram"]
notifications-email    = ["uptrakit-plugin-infrastructure-registry/notifications-email"]

# after
notifications-telegram = ["uptrakit-surface-proxy/notifications-telegram"]
notifications-email    = ["uptrakit-surface-proxy/notifications-email"]
```

- [ ] **Step 2: Confirm original source files are already gone**

`git mv` in Task 4 Steps 3–4 already deleted `surface_proxy.rs`, `surface_proxy/`, and
`surface_registry.rs` from web-api. Nothing to `git rm` here — they are absent from both
the working tree and git's index. Verify:

```bash
ls crates/ui/web-api/src/surface_proxy.rs 2>&1    # expect: No such file or directory
ls crates/ui/web-api/src/surface_proxy/  2>&1     # expect: No such file or directory
ls crates/ui/web-api/src/surface_registry.rs 2>&1 # expect: No such file or directory
```

- [ ] **Step 3: Audit all public names in `surface_registry.rs` before writing the shim**

Before writing the `pub mod surface_registry` shim, verify the explicit re-export list is
complete. Run:

```bash
grep -n "^pub " crates/ui/web-api/src/surface_registry.rs
```

Every `pub` type/enum/fn/struct returned by this grep must appear in the `pub mod surface_registry`
re-export block in the next step. The shim uses an explicit name list (not a glob) to avoid
polluting the namespace with proxy types; any name omitted here will break `controller-runtime`
and other external callers. Cross-reference the output against the list in Step 4 before proceeding.

- [ ] **Step 4: Update `web-api/src/lib.rs`**

Remove:

```rust
pub mod surface_proxy;
pub mod surface_registry;
```

Add in their place (preserve the `service_connections` mod as a re-export shim):

```rust
// Preserves uptrakit_web_api::surface_proxy::* paths used by controller-runtime
// and routes within web-api.
pub use uptrakit_surface_proxy as surface_proxy;

// Preserves uptrakit_web_api::surface_registry::* paths used by controller-runtime
// and routes/service_ws/handler.
pub mod surface_registry {
    pub use uptrakit_surface_proxy::{
        ResolvedSurfaceAction,
        ResolvedSurfaceRead,
        SurfaceCatalogItem,
        SurfaceProviderRejection,
        SurfaceProviderRejectionCode,
        SurfaceProviderRejectionReason,
        SurfaceProviderSummary,
        SurfaceRegistry,
        SurfaceRegistryConfig,
        SurfaceRegistryError,
        SurfaceRegistryLookupError,
    };
}
```

- [ ] **Step 5: Add `SurfaceProxyDeps` to `app_state.rs`**

In `crates/ui/web-api/src/app_state.rs`, add the struct definition after the existing
`use` imports (near the top of the file, before `AppState`):

```rust
#[non_exhaustive]
pub struct SurfaceProxyDeps {
    pub registry: Arc<SurfaceRegistry>,
    pub proxy: Arc<SurfaceProxy>,
}

impl SurfaceProxyDeps {
    pub fn new(registry: Arc<SurfaceRegistry>, proxy: Arc<SurfaceProxy>) -> Self {
        Self { registry, proxy }
    }
}
```

In `AppState`, replace:

```rust
pub surface_registry: Arc<SurfaceRegistry>,
/// Request/response proxy for surface interaction invocations.
pub surface_proxy: Arc<SurfaceProxy>,
```

With:

```rust
pub surface_proxy_deps: SurfaceProxyDeps,
```

In `AppStateBuilder`, the staging fields stay separate — leave
`surface_registry: Option<Arc<SurfaceRegistry>>` and
`surface_proxy: Option<Arc<SurfaceProxy>>` unchanged.

In `AppStateBuilder::build()`, replace the two separate field assignments:

```rust
surface_registry: self.surface_registry.unwrap_or_else(|| {
    Arc::new(SurfaceRegistry::new(
        crate::surface_registry::SurfaceRegistryConfig::default(),
    ))
}),
surface_proxy: self
    .surface_proxy
    .unwrap_or_else(|| Arc::new(SurfaceProxy::new())),
```

With:

```rust
surface_proxy_deps: SurfaceProxyDeps::new(
    self.surface_registry.unwrap_or_else(|| {
        Arc::new(SurfaceRegistry::new(
            crate::surface_registry::SurfaceRegistryConfig::default(),
        ))
    }),
    self.surface_proxy
        .unwrap_or_else(|| Arc::new(SurfaceProxy::new())),
),
```

Also update `app_state.rs` imports: replace
`use crate::surface_proxy::SurfaceProxy;` and `use crate::surface_registry::SurfaceRegistry;`
with the re-export paths now available via the shims:

```rust
use crate::surface_proxy::SurfaceProxy;     // resolves through pub use uptrakit_surface_proxy as surface_proxy
use crate::surface_registry::SurfaceRegistry; // resolves through pub mod surface_registry
```

These paths remain valid because of the lib.rs shims added in Step 4.

- [ ] **Step 6: Update inline `AppState` construction sites**

The following files construct `AppState` with `surface_registry: Arc::new(...)` and
`surface_proxy: Arc::new(...)` fields. In each, replace the two separate fields with
`surface_proxy_deps: SurfaceProxyDeps::new(surface_registry_value, surface_proxy_value)`.

**`crates/ui/web-api/src/lib.rs`** (default construction, ~line 255):

```rust
// before
surface_registry: Arc::new(crate::surface_registry::SurfaceRegistry::new(
    crate::surface_registry::SurfaceRegistryConfig::default(),
)),
surface_proxy: Arc::new(crate::surface_proxy::SurfaceProxy::new()),

// after
surface_proxy_deps: crate::app_state::SurfaceProxyDeps::new(
    Arc::new(crate::surface_registry::SurfaceRegistry::new(
        crate::surface_registry::SurfaceRegistryConfig::default(),
    )),
    Arc::new(crate::surface_proxy::SurfaceProxy::new()),
),
```

Apply the same pattern to:

- `crates/ui/web-api/src/middleware/resolve_ip.rs` (~line 290)
- `crates/ui/web-api/src/middleware/require_auth.rs` (~line 623)
- `crates/ui/web-api/src/test_harness/mod.rs` (~line 257)
- `crates/ui/web-api/src/routes/settings_nats.rs` (~line 396)
- `crates/ui/web-api/src/routes/surfaces.rs` (~line 1096)
- `crates/ui/web-api/src/routes/services.rs` (~line 1133)
- `crates/ui/web-api/src/routes/auth.rs` (~line 956)

**`crates/ui/web-api/src/routes/service_ws/handler/mod.rs` (~line 3422)** — uses struct
shorthand. Replace:

```rust
surface_registry,
surface_proxy,
```

With:

```rust
surface_proxy_deps: crate::app_state::SurfaceProxyDeps::new(surface_registry, surface_proxy),
```

- [ ] **Step 7: Update `state.surface_registry.*` / `state.surface_proxy.*` access sites**

Every place in `web-api` that accesses `state.surface_registry` (or `self.state.surface_registry`)
must change to `state.surface_proxy_deps.registry`. Same for `surface_proxy` →
`surface_proxy_deps.proxy`.

Key files with access sites:

- `routes/surfaces.rs` — accesses via `state.surface_registry` and `state.surface_proxy`
- `routes/service_ws/handler/mod.rs` — many sites (~1455, ~1618, ~1655, ~2209, ~2214, ~2514,
  ~2519, ~4095, ~5365 and others)
- `app_state.rs` — builder accessor methods `.surface_registry()` and `.surface_proxy()` return
  the sub-fields; update the return-path getters if they access the struct fields directly

Run a search to find all:

```bash
grep -rn "\.surface_registry\b\|\.surface_proxy\b" crates/ui/web-api/src/ | grep -v "surface_proxy_deps"
```

For each hit, change:

```text
state.surface_registry  →  state.surface_proxy_deps.registry
state.surface_proxy     →  state.surface_proxy_deps.proxy
self.state.surface_registry  →  self.state.surface_proxy_deps.registry
self.state.surface_proxy     →  self.state.surface_proxy_deps.proxy
```

- [ ] **Step 8: Fix `routes/notifications.rs:1146` — caller of deleted `from_app_state`**

`from_app_state` was deleted from `proxy/controller_local.rs` in Task 4. Update the caller:

```rust
// before (~line 1146)
let controller = crate::surface_proxy::AppStateSurfaceActionController::from_app_state(
    &state, tenant_id, caller_user_id,
);

// after
let controller = uptrakit_surface_proxy::AppStateSurfaceActionController::from_database_connection(
    state.db(), state.plugin_ops.as_ref(), tenant_id, caller_user_id,
);
```

- [ ] **Step 9: Update remaining `build_settings_bag` callers in `routes/`**

`routes/users.rs:1234` and `routes/notifications.rs:625` still call
`crate::notifications::dispatcher::build_settings_bag`. Update both to call the function
directly from `web-api-queries`:

```rust
// before
crate::notifications::dispatcher::build_settings_bag(state.db(), tenant_id).await

// after
uptrakit_web_api_queries::notification_settings::build_settings_bag(state.db(), tenant_id).await
```

After these two callers are updated, the `pub(crate) use` in `dispatcher.rs` can be downgraded
to a private import since no callers outside `dispatcher.rs` remain:

```rust
// in dispatcher.rs — change pub(crate) use to private use:
use uptrakit_web_api_queries::notification_settings::build_settings_bag;
```

- [ ] **Step 10: Verify both crates compile and tests pass**

```bash
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo test -p uptrakit-surface-proxy --all-features 2>&1 | tail -5
cargo test -p uptrakit-web-api --all-features 2>&1 | tail -10
```

Expected: no errors, all tests pass.

- [ ] **Step 11: Commit**

```bash
git add crates/ui/web-api/
git commit -m "refactor(web-api): wire uptrakit-surface-proxy, introduce SurfaceProxyDeps, remove original surface files"
```

---

## Task 6: Update ADR-0001

**Files:**

- Modify: `docs/adr/0001-web-api-decomposition-strategy.md`

- [ ] **Step 1: Update the candidates table**

In the `| surface_proxy/ |` row, change status from `Approved — after notification` to
`Completed` and add a spec reference:

```markdown
| `surface_proxy/` | Completed | Spec: `docs/superpowers/specs/2026-05-01-extract-surface-proxy-crate-design.md` |
```

- [ ] **Step 2: Add a note about `SurfaceRuntimeRolloutState` future ownership**

In the Consequences section, add one line noting that
`tests/provider_proxied/rollout.rs` (moved but undeclared) references
`crate::SurfaceRuntimeRolloutState`, which currently lives in `app_state.rs` of
`web-api`. When the test directory is eventually wired in, a decision is needed on
whether `SurfaceRuntimeRolloutState` moves to `uptrakit-surface-proxy` or stays in
`web-api` and is imported. Record this as a known deferred design decision.

- [ ] **Step 3: Add a Consequences entry for the three pre-conditions**

Under the `## Consequences` section, add:

```markdown
- **surface_proxy pre-conditions (fulfilled):**
  1. `ServiceConnectionRegistry` (used by 8+ callers beyond surface_proxy) extracted to
     `uptrakit-service-connections` (commit 1 of extraction).
  2. `surface_proxy/controller_local/settings_store.rs` decoupled from
     `uptrakit-web-api-auth::settings_store` — replaced with direct
     `uptrakit_shared_db::raw_settings` calls (commit 2).
  3. `build_settings_bag` + SMTP helpers moved from `dispatcher.rs` to
     `uptrakit_web_api_queries::notification_settings` (commit 3). The dispatcher and route
     callers in `web-api` import it from there; the surface-proxy crate calls it directly.
```

- [ ] **Step 4: Record build-time gate measurements**

Per ADR-0001, measure incremental build times before and after extraction and add the results
to this Consequences entry.

Before extraction (run before Task 4):

```bash
touch crates/ui/web-api/src/surface_proxy/controller_local.rs
time cargo build -p uptrakit-web-api 2>&1 | tail -3
```

After extraction (run after Task 5 is complete):

```bash
touch crates/ui/surface-proxy/src/proxy/controller_local.rs
time cargo build -p uptrakit-surface-proxy 2>&1 | tail -3
```

Record both deltas in the Consequences entry as:
`Build-time gate: web-api incremental: Xs / surface-proxy incremental: Ys`

- [ ] **Step 5: Verify markdownlint**

```bash
npx markdownlint --config .markdownlint.json 'docs/adr/0001-web-api-decomposition-strategy.md'
```

Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git add docs/adr/0001-web-api-decomposition-strategy.md
git commit -m "docs(adr): mark surface_proxy extraction as completed, record build-time gate"
```

---

## Quality Gate

After all six tasks, run the full check suite:

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo deny check
npx markdownlint --config .markdownlint.json '**/*.md'
```

All commands must exit 0.
