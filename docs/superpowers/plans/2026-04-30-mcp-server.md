# MCP Server Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Embed an MCP server in `uptrakit-web-api` exposing four tools to AI agents —
`get_current_user`, `list_update_history`, `get_update_history_detail`, and `trigger_update`
— authenticated via opaque API tokens.

**Architecture:** `rmcp` `StreamableHttpService` mounted at `/mcp` via Axum `nest_service`.
A Tower auth layer validates `upk_` tokens and inserts `McpRequestContext` per-request.
Tools extract the context from `Extension<http::request::Parts>` injected by rmcp on each call.

**Tech Stack:** `rmcp` 1.x (`transport-streamable-http-server`), `vt100`, existing `ApiTokenService`, SeaORM in-memory SQLite for unit tests.

---

## File Map

### Created

- `crates/ui/web-api/src/mcp/mod.rs` — `build_mcp_router()`, `McpHandler` struct, tool registration
- `crates/ui/web-api/src/mcp/auth.rs` — Tower auth layer, `McpRequestContext`, JWT rejection
- `crates/ui/web-api/src/mcp/terminal.rs` — `render_terminal_output(bytes: &[u8]) -> String` via vt100
- `crates/ui/web-api/src/mcp/tools/mod.rs` — re-exports + shared helpers (`require_permission`)
- `crates/ui/web-api/src/mcp/tools/user.rs` — `get_current_user` tool
- `crates/ui/web-api/src/mcp/tools/history.rs` — `list_update_history` + `get_update_history_detail` tools
- `crates/ui/web-api/src/mcp/tools/update.rs` — `trigger_update` tool

### Modified

- `crates/shared/types/src/permissions.rs` — add `Other(String)` variant + `AccessMcp`;
  update `as_str`, `description`, `FromStr`, `Deserialize`; remove `ParsePermissionError`
- `crates/shared/types/src/lib.rs` — remove `ParsePermissionError` from `pub use`
- `crates/shared/web-api-types/src/permissions.rs` — remove `ParsePermissionError` re-export
- `crates/shared/web-api-types/src/lib.rs` — update `permission_iter_covers_all_variants` count 33→34
  (after `AccessMcp` added in Task 2; `Other` excluded from `EnumIter`)
- `crates/shared/db/src/migration/mod.rs` — register two new migrations
- `crates/shared/db/src/migration/m20260423_000001_permission_wire_safe.rs` — new migration
  (no-op DB change; prerequisite commit for `Other(String)`)
- `crates/shared/db/src/migration/m20260424_000001_access_mcp_permission.rs` — insert
  `access_mcp` permission + grant to roles
- `crates/ui/web-api/Cargo.toml` — add `mcp` feature; `rmcp` + `vt100` behind it
- `crates/core/controller-runtime/Cargo.toml` — thread `mcp` feature through
- `crates/core/controller-runtime/src/server.rs` — merge MCP router before middleware layers
- `crates/ui/web-api/src/lib.rs` — `pub mod mcp` (cfg-gated), export `build_mcp_router`
- `crates/ui/web-api/src/middleware/require_auth.rs` — make `emit_api_token_auth_audit` + `AuthFailure` `pub(crate)`
- `crates/ui/web-api/src/routes/software_items/mod.rs` — make `emit_software_update_audit` `pub(crate)`
- `crates/ui/web-api/src/middleware/request_log.rs` — tolerate absent `AuthenticatedUser` (verify/fix)

---

## Task 1: `Permission` wire-safe `Other(String)` catch-all

**Files:**

- Modify: `crates/shared/types/src/permissions.rs`
- Modify: `crates/shared/web-api-types/src/lib.rs`
- Create: `crates/shared/db/src/migration/m20260423_000001_permission_wire_safe.rs`
- Modify: `crates/shared/db/src/migration/mod.rs`

### Context

`Permission::FromStr` is currently exhaustive — unknown strings return `Err`. An old binary receiving a new unknown variant (like
`access_mcp` added in Task 2) silently drops the permission → user gets 403. The fix: add `Other(String)` with
`#[strum(disabled)]` (excluded from `EnumIter`, count stays 33) and replace `FromStr` + `Deserialize` with infallible versions.

No DB change is needed for this task — the migration is a no-op placeholder that anchors the commit ordering in the migration
table. The actual `access_mcp` row lands in Task 2's migration.

- [ ] **Step 1: Write the failing test — infallible `FromStr`**

In `crates/shared/web-api-types/src/lib.rs`, inside the existing `#[cfg(test)] mod tests`, add:

```rust
#[test]
fn permission_from_str_unknown_becomes_other() {
    let p: Permission = "access_mcp".parse().unwrap();
    assert!(matches!(p, Permission::Other(s) if s == "access_mcp"));
}

#[test]
fn permission_other_not_in_iter() {
    for p in Permission::iter() {
        assert!(!matches!(p, Permission::Other(_)), "Other should not appear in iter");
    }
}

#[test]
fn permission_other_as_str_returns_inner() {
    let p = Permission::Other("foo_bar".to_string());
    assert_eq!(p.as_str(), "foo_bar");
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p uptrakit-web-api-types permission_from_str_unknown 2>&1 | tail -5
cargo test -p uptrakit-web-api-types permission_other_not_in_iter 2>&1 | tail -5
cargo test -p uptrakit-web-api-types permission_other_as_str 2>&1 | tail -5
```

Expected: all three FAIL (compile error — `Other` variant does not exist yet).

- [ ] **Step 3: Add `Other(String)` to the `Permission` enum**

In `crates/shared/types/src/permissions.rs`, make these changes:

1. Add `Other(String)` at the **end** of the enum, before the closing brace, with `#[strum(disabled)]`:

```rust
    // ── Forward-compatibility ────────────────────────────────────────────
    /// An unknown permission received from a newer build.
    ///
    /// Preserved on the wire instead of being dropped, so old binaries
    /// never silently lose permissions added in newer builds.
    #[strum(disabled)]
    Other(String),
```

1. Replace `pub fn as_str(&self) -> &'static str` with `pub fn as_str(&self) -> &str`
   and add the `Other` arm:

```rust
    pub fn as_str(&self) -> &str {
        match self {
            // ... all existing arms unchanged ...
            Permission::TestPluginConfigs => "test_plugin_configs",
            Permission::Other(s) => s.as_str(),
        }
    }
```

1. Replace `pub fn description(&self) -> &'static str` with `pub fn description(&self) -> &str`
   and add the `Other` arm:

```rust
    pub fn description(&self) -> &str {
        match self {
            // ... all existing arms unchanged ...
            Permission::TestPluginConfigs => "Test plugin configurations against hosts",
            Permission::Other(_) => "(unknown permission)",
        }
    }
```

1. Remove the failing `FromStr` impl (the one that returns `ParsePermissionError` —
   the struct itself is removed in Step 4 below). Replace with:

```rust
impl From<String> for Permission {
    fn from(s: String) -> Self {
        match s.as_str() {
            "view_services" => Self::ViewServices,
            "approve_services" => Self::ApproveServices,
            "reject_services" => Self::RejectServices,
            "remove_services" => Self::RemoveServices,
            "update_services" => Self::UpdateServices,
            "view_system_services" => Self::ViewSystemServices,
            "approve_system_services" => Self::ApproveSystemServices,
            "reject_system_services" => Self::RejectSystemServices,
            "remove_system_services" => Self::RemoveSystemServices,
            "update_system_services" => Self::UpdateSystemServices,
            "view_software" => Self::ViewSoftware,
            "create_software" => Self::CreateSoftware,
            "update_software" => Self::UpdateSoftware,
            "delete_software" => Self::DeleteSoftware,
            "trigger_checks" => Self::TriggerChecks,
            "trigger_updates" => Self::TriggerUpdates,
            "manage_scheduler" => Self::ManageScheduler,
            "view_hosts" => Self::ViewHosts,
            "update_hosts" => Self::UpdateHosts,
            "deactivate_hosts" => Self::DeactivateHosts,
            "view_settings" => Self::ViewSettings,
            "manage_auth_settings" => Self::ManageAuthSettings,
            "manage_enrollment_tokens" => Self::ManageEnrollmentTokens,
            "manage_agent_certs" => Self::ManageAgentCerts,
            "manage_global_settings" => Self::ManageGlobalSettings,
            "manage_commands" => Self::ManageCommands,
            "view_notifications" => Self::ViewNotifications,
            "manage_notifications" => Self::ManageNotifications,
            "view_audit_logs" => Self::ViewAuditLogs,
            "view_system_audit_logs" => Self::ViewSystemAuditLogs,
            "manage_users" => Self::ManageUsers,
            "manage_ignores" => Self::ManageIgnores,
            "test_plugin_configs" => Self::TestPluginConfigs,
            _ => {
                tracing::debug!(permission = %s, "unknown permission string mapped to Other");
                Self::Other(s)
            }
        }
    }
}

impl FromStr for Permission {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::from(s.to_string()))
    }
}
```

1. Replace the derived `Deserialize` with a manual infallible impl. Remove `Deserialize`
   from the `#[derive(...)]` line and add:

```rust
impl<'de> serde::Deserialize<'de> for Permission {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Permission::from)
    }
}
```

Also remove `#[serde(rename_all = "snake_case")]` from the enum — the manual `From<String>` handles matching.

- [ ] **Step 4: Remove `ParsePermissionError`**

In `crates/shared/types/src/permissions.rs`, delete the `ParsePermissionError` struct and its `#[derive]`/`#[error]` lines (currently around line 219):

```rust
// DELETE these lines:
#[derive(Debug, thiserror::Error)]
#[error("invalid permission value")]
pub struct ParsePermissionError;
```

In `crates/shared/types/src/lib.rs`, remove `ParsePermissionError` from the `pub use` line:

```rust
// Before:
pub use permissions::{ParsePermissionError, Permission};
// After:
pub use permissions::Permission;
```

In `crates/shared/web-api-types/src/permissions.rs`, remove `ParsePermissionError` from the re-export:

```rust
// Before:
pub use uptrakit_shared_types::{ParsePermissionError, Permission};
// After:
pub use uptrakit_shared_types::Permission;
```

Check nothing else imports `ParsePermissionError`:

```bash
grep -rn "ParsePermissionError" crates/ --include="*.rs"
```

Expected: no remaining references.

- [ ] **Step 5: Fix `Display` and `From<Permission> for String`**

The existing impls delegate to `as_str()`. They still compile — `&str` (not `&'static str`) is now returned.
The impls in `permissions.rs` do not change. Verify with:

```bash
cargo check -p uptrakit-shared-types 2>&1 | head -30
```

- [ ] **Step 6: Fix `require_auth.rs` — `Permission::from_str` no longer returns `Err`**

In `crates/ui/web-api/src/middleware/require_auth.rs` (and anywhere else `Permission` is parsed), the
`.parse::<Permission>().ok()` pattern still works since `Infallible` is always `Ok`. No change needed. Verify:

```bash
cargo check -p uptrakit-web-api --no-default-features --features db-sqlite 2>&1 | head -30
```

- [ ] **Step 7: Create no-op migration as ordering anchor**

Create `crates/shared/db/src/migration/m20260423_000001_permission_wire_safe.rs`:

```rust
use sea_orm_migration::prelude::*;

/// No-op migration that anchors the wire-safe Permission::Other(String) change
/// in the migration sequence. The code change is in uptrakit-shared-types;
/// no schema modification is required.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
```

Register it in `crates/shared/db/src/migration/mod.rs` — find the `vec![...]` in `fn migrator()` and append:

```rust
Box::new(m20260423_000001_permission_wire_safe::Migration),
```

Also add the `pub(super) mod m20260423_000001_permission_wire_safe;` declaration at the top of the file.

- [ ] **Step 8: Run the tests**

```bash
cargo test -p uptrakit-web-api-types permission_from_str_unknown 2>&1 | tail -5
cargo test -p uptrakit-web-api-types permission_other_not_in_iter 2>&1 | tail -5
cargo test -p uptrakit-web-api-types permission_other_as_str 2>&1 | tail -5
cargo test -p uptrakit-web-api-types permission_iter_covers_all_variants 2>&1 | tail -5
```

Expected: all PASS (count still 33 — `Other` excluded from `EnumIter`).

- [ ] **Step 9: Quality gates**

```bash
cargo fmt --all
cargo clippy -p uptrakit-shared-types -p uptrakit-web-api-types -p uptrakit-web-api --all-targets --no-default-features --features db-sqlite 2>&1 | grep "^error" | head -20
```

Expected: no errors.

- [ ] **Step 10: Commit**

```bash
git add crates/shared/types/src/permissions.rs \
        crates/shared/types/src/lib.rs \
        crates/shared/web-api-types/src/permissions.rs \
        crates/shared/web-api-types/src/lib.rs \
        crates/shared/db/src/migration/m20260423_000001_permission_wire_safe.rs \
        crates/shared/db/src/migration/mod.rs
git commit -m "feat(permissions): wire-safe Other(String) catch-all for unknown variants

Old binaries encountering a permission string they don't know (e.g.
access_mcp added in a newer build) previously silently dropped it via
the exhaustive FromStr, causing a 403 on every MCP request. Now unknown
strings map to Other(s) instead of Err. Other is excluded from EnumIter
via #[strum(disabled)] so permission_iter_covers_all_variants count
stays at 33."
```

---

## Task 2: Add `AccessMcp` permission and DB migration

**Files:**

- Modify: `crates/shared/types/src/permissions.rs`
- Modify: `crates/shared/web-api-types/src/lib.rs`
- Create: `crates/shared/db/src/migration/m20260424_000001_access_mcp_permission.rs`
- Modify: `crates/shared/db/src/migration/mod.rs`

### Context

`AccessMcp` is the gate to the `/mcp` endpoint. Grant it only to roles that already hold `view_software` OR `trigger_updates`
— roles with neither permission have no useful MCP tools available. The `operator` role has `trigger_updates` but not
`view_software`; it receives `AccessMcp` but can only call `trigger_update` via MCP. This partial-access scenario is
documented in the migration comment and is acceptable for MVP.

Roles to receive `access_mcp` (verified from `m20260310_000002_granular_permissions`):

- `viewer` (has `view_software`)
- `operator` (has `trigger_updates`)
- `software_manager` (has `trigger_updates`; does NOT have `view_software` — can trigger
  updates but will 403 on history tools without `viewer` role also assigned)

**Note:** There is no `"admin"` or `"updater"` role. Old `admin`/`owner`/`user` roles were
migrated to the new granular set. Current roles: `viewer`, `operator`, `service_manager`,
`software_manager`, `host_manager`, `settings_manager`, `command_manager`,
`system_administrator`. `settings_manager` lacks both `view_software` and `trigger_updates`
— does NOT receive `access_mcp`.

- [ ] **Step 1: Write the failing test**

In `crates/shared/web-api-types/src/lib.rs`, add to the existing test module:

```rust
#[test]
fn permission_iter_covers_all_variants_with_access_mcp() {
    // Count is 34 after AccessMcp is added. Other(String) is excluded via #[strum(disabled)].
    assert_eq!(Permission::iter().count(), 34);
}

#[test]
fn access_mcp_as_str() {
    assert_eq!(Permission::AccessMcp.as_str(), "access_mcp");
}

#[test]
fn access_mcp_round_trips_from_str() {
    let p: Permission = "access_mcp".parse().unwrap();
    assert_eq!(p, Permission::AccessMcp);
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p uptrakit-web-api-types permission_iter_covers_all_variants_with_access_mcp 2>&1 | tail -5
```

Expected: FAIL (compile error — `AccessMcp` does not exist yet).

- [ ] **Step 3: Add `AccessMcp` variant**

In `crates/shared/types/src/permissions.rs`:

1. Add to the enum (before the `Other(String)` variant):

```rust
    // ── MCP ──────────────────────────────────────────────────────────────
    /// Access the MCP server endpoint (`/mcp`).
    ///
    /// Gate to the MCP endpoint. Tools enforce their own additional
    /// fine-grained permission checks (`ViewSoftware`, `TriggerUpdates`).
    AccessMcp,
```

1. Add to `as_str()` match:

```rust
            Permission::AccessMcp => "access_mcp",
```

1. Add to `description()` match:

```rust
            Permission::AccessMcp => "Access the MCP server endpoint",
```

1. Add to `From<String>` match:

```rust
            "access_mcp" => Self::AccessMcp,
```

- [ ] **Step 4: Update the count assertion**

In `crates/shared/web-api-types/src/lib.rs`, change the existing test:

```rust
// Before:
assert_eq!(Permission::iter().count(), 33);
// After:
assert_eq!(Permission::iter().count(), 34);
```

Remove the old `permission_iter_covers_all_variants` test (or rename it) — it conflicts with the new one.
Keep only the new `permission_iter_covers_all_variants_with_access_mcp` test.

- [ ] **Step 5: Create the DB migration**

Create `crates/shared/db/src/migration/m20260424_000001_access_mcp_permission.rs`:

```rust
use sea_orm::ConnectionTrait;
use sea_orm_migration::prelude::*;
use uuid::Uuid;

/// Add the `access_mcp` permission and grant it to roles that hold
/// `view_software` OR `trigger_updates`.
///
/// ## Role assignments
///
/// - `viewer`: granted (holds `view_software`)
/// - `operator`: granted (holds `trigger_updates`)
///   NOTE: `operator` lacks `view_software`, so it can call `trigger_update`
///   via MCP but will receive 403 on all history tools. In practice,
///   `AccessPreset::Operator` always bundles `viewer` + `operator`, so this
///   only affects users assigned the raw role directly.
/// - `software_manager`: granted (holds `trigger_updates`; lacks `view_software` — same
///   partial-access as `operator`: can call `trigger_update` via MCP but gets 403 on
///   history tools without the `viewer` role also assigned)
/// - `settings_manager`: NOT granted (lacks `view_software` and `trigger_updates`)
///
/// ## Idempotency
///
/// INSERT uses ON CONFLICT DO NOTHING. Role grants use WHERE NOT EXISTS.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

async fn grant_permission(
    manager: &SchemaManager<'_>,
    role_name: &str,
    perm_name: &str,
) -> Result<(), DbErr> {
    let sql = format!(
        "INSERT INTO role_permissions (role_id, permission_id) \
         SELECT r.id, p.id \
         FROM roles r, permissions p \
         WHERE r.name = '{role_name}' AND p.name = '{perm_name}' \
         AND NOT EXISTS ( \
           SELECT 1 FROM role_permissions rp \
           WHERE rp.role_id = r.id AND rp.permission_id = p.id \
         )"
    );
    manager.get_connection().execute_unprepared(&sql).await?;
    Ok(())
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let now = time::OffsetDateTime::now_utc();

        let exists = manager
            .get_connection()
            .query_one_raw(sea_orm::Statement::from_string(
                manager.get_database_backend(),
                "SELECT 1 FROM permissions WHERE name = 'access_mcp' LIMIT 1".to_string(),
            ))
            .await?;
        if exists.is_none() {
            manager
                .exec_stmt(
                    Query::insert()
                        .into_table(Alias::new("permissions"))
                        .columns([
                            Alias::new("id"),
                            Alias::new("name"),
                            Alias::new("description"),
                            Alias::new("created_at"),
                        ])
                        .values_panic([
                            Uuid::now_v7().into(),
                            "access_mcp".into(),
                            "Access the MCP server endpoint".into(),
                            now.into(),
                        ])
                        .to_owned(),
                )
                .await?;
        }

        for role in ["viewer", "operator", "software_manager"] {
            grant_permission(manager, role, "access_mcp").await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .exec_stmt(
                Query::delete()
                    .from_table(Alias::new("role_permissions"))
                    .and_where(
                        Expr::col(Alias::new("permission_id")).in_subquery(
                            Query::select()
                                .from(Alias::new("permissions"))
                                .column(Alias::new("id"))
                                .and_where(Expr::col(Alias::new("name")).eq("access_mcp"))
                                .to_owned(),
                        ),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .exec_stmt(
                Query::delete()
                    .from_table(Alias::new("permissions"))
                    .and_where(Expr::col(Alias::new("name")).eq("access_mcp"))
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}
```

Register in `crates/shared/db/src/migration/mod.rs`:

```rust
pub(super) mod m20260424_000001_access_mcp_permission;
// in fn migrator() vec!:
Box::new(m20260424_000001_access_mcp_permission::Migration),
```

- [ ] **Step 6: Run the tests**

```bash
cargo test -p uptrakit-web-api-types access_mcp 2>&1 | tail -10
cargo test -p uptrakit-web-api-types permission_iter_covers_all_variants_with_access_mcp 2>&1 | tail -5
```

Expected: all PASS.

- [ ] **Step 7: Quality gates**

```bash
cargo fmt --all
cargo clippy -p uptrakit-shared-types -p uptrakit-web-api-types --all-targets --no-default-features --features db-sqlite 2>&1 | grep "^error" | head -20
```

- [ ] **Step 8: Commit**

```bash
git add crates/shared/types/src/permissions.rs \
        crates/shared/web-api-types/src/lib.rs \
        crates/shared/db/src/migration/m20260424_000001_access_mcp_permission.rs \
        crates/shared/db/src/migration/mod.rs
git commit -m "feat(permissions): add AccessMcp variant and DB migration

access_mcp grants access to the /mcp endpoint. Granted to roles that
already hold view_software or trigger_updates. operator receives it but
can only call trigger_update via MCP (lacks view_software for history
tools) — acceptable MVP gap per spec."
```

---

## Task 3: MCP scaffold — feature flag, skeleton router, server.rs merge

**Files:**

- Modify: `crates/ui/web-api/Cargo.toml`
- Modify: `crates/core/controller-runtime/Cargo.toml`
- Modify: `crates/ui/web-api/src/lib.rs`
- Create: `crates/ui/web-api/src/mcp/mod.rs`
- Create: `crates/ui/web-api/src/mcp/auth.rs` (stub — returns `todo!()`)
- Create: `crates/ui/web-api/src/mcp/tools/mod.rs` (stub)
- Modify: `crates/core/controller-runtime/src/server.rs`

### Context

The `mcp` feature is default-on. `build_mcp_router()` is defined in `uptrakit-web-api` and returns `Router<Arc<AppState>>`.
It is merged in `server.rs` before middleware layers. The merge must happen before `.layer(...)` calls so all middleware
(request_log, request_id, security_headers) covers both routers.

`StreamableHttpServerConfig::default()` allows only `localhost`. For remote deployments, `allowed_hosts` must be populated
from `state.settings.sans()`. SANs don't include port numbers but HTTP `Host` headers do (e.g.,
`controller.example.com:9443`). Strip ports and add both bare-hostname and `hostname:port` forms.

- [ ] **Step 1: Write the failing test — `allowed_hosts` port-stripping**

Create `crates/ui/web-api/src/mcp/mod.rs` with a test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_hosts_includes_port_variants() {
        let sans = vec![
            "controller.example.com".to_string(),
            "localhost".to_string(),
        ];
        let hosts = build_allowed_hosts(&sans);
        assert!(hosts.contains(&"controller.example.com".to_string()));
        assert!(hosts.contains(&"controller.example.com:9443".to_string()));
        assert!(hosts.contains(&"localhost".to_string()));
    }

    #[test]
    fn allowed_hosts_strips_port_from_san_if_present() {
        // SANs normally don't have ports, but be defensive
        let sans = vec!["controller.example.com:8443".to_string()];
        let hosts = build_allowed_hosts(&sans);
        // bare hostname must be present
        assert!(hosts.contains(&"controller.example.com".to_string()));
    }
}
```

(The `build_allowed_hosts` function doesn't exist yet — test will fail to compile.)

- [ ] **Step 2: Add `rmcp` and `vt100` to `web-api/Cargo.toml`**

Add the `mcp` feature and optional deps:

```toml
[features]
default = ["oidc", "mcp"]
# ... existing features ...
mcp = ["dep:rmcp", "dep:vt100"]

[dependencies]
# ... existing deps ...
rmcp = { workspace = true, features = ["transport-streamable-http-server"], optional = true }
vt100 = { workspace = true, optional = true }
```

Add `rmcp` and `vt100` to workspace `Cargo.toml` if not already there:

```bash
grep -n "rmcp\|vt100" Cargo.toml
```

If missing, add to `[workspace.dependencies]`:

```toml
rmcp = "1"
vt100 = "0.15"
```

- [ ] **Step 3: Thread `mcp` through `controller-runtime/Cargo.toml`**

In `crates/core/controller-runtime/Cargo.toml`, add to `[features]`:

```toml
mcp = ["uptrakit-web-api/mcp"]
default = ["uptrakit-web-api/default", "mcp"]
```

(Check existing default feature list first — append `"mcp"` only if a `default` feature already exists; otherwise create it.)

- [ ] **Step 4: Implement `build_allowed_hosts` and `build_mcp_router` skeleton**

Write `crates/ui/web-api/src/mcp/mod.rs`:

```rust
#[cfg(feature = "mcp")]
pub mod auth;
#[cfg(feature = "mcp")]
pub mod terminal;
#[cfg(feature = "mcp")]
pub mod tools;

use std::sync::Arc;

use axum::Router;

use crate::AppState;

/// Build the MCP sub-router (mounts at `/mcp` when merged into the main router).
pub fn build_mcp_router(state: Arc<AppState>) -> Router<Arc<AppState>> {
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService,
        session::local::LocalSessionManager,
    };

    let allowed_hosts = build_allowed_hosts(&state.settings.sans());
    let config = StreamableHttpServerConfig {
        allowed_hosts,
        ..StreamableHttpServerConfig::default()
    };

    let handler = crate::mcp::tools::McpHandler::new(Arc::clone(&state));
    // StreamableHttpService::new(service_factory, session_manager, config)
    // Verify 3-arg signature against installed rmcp version before assuming.
    let svc = StreamableHttpService::new(
        move || Ok(handler.clone()),
        Arc::new(LocalSessionManager::default()),
        config,
    );

    Router::new().nest_service("/mcp", svc)
}

/// Build the allowed_hosts list for StreamableHttpServerConfig.
///
/// SANs don't include ports, but HTTP Host headers do. Add both bare-hostname
/// and hostname:port variants for each SAN. The standard port 9443 is always
/// added. Wildcard SANs (*.example.com) are not expanded — rmcp does not
/// support wildcard matching; document in prod notes if needed.
pub(crate) fn build_allowed_hosts(sans: &[String]) -> Vec<String> {
    const HTTPS_PORT: u16 = 9443;
    let mut hosts: Vec<String> = Vec::new();

    for san in sans {
        // Strip port if SAN unexpectedly includes one
        let bare = san.split(':').next().unwrap_or(san).to_string();
        if !hosts.contains(&bare) {
            hosts.push(bare.clone());
        }
        let with_port = format!("{}:{}", bare, HTTPS_PORT);
        if !hosts.contains(&with_port) {
            hosts.push(with_port);
        }
    }

    // Always allow localhost variants (dev + health checks)
    for h in ["localhost", "localhost:9443", "127.0.0.1", "127.0.0.1:9443", "[::1]", "[::1]:9443"] {
        let s = h.to_string();
        if !hosts.contains(&s) {
            hosts.push(s);
        }
    }

    hosts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_hosts_includes_port_variants() {
        let sans = vec![
            "controller.example.com".to_string(),
            "localhost".to_string(),
        ];
        let hosts = build_allowed_hosts(&sans);
        assert!(hosts.contains(&"controller.example.com".to_string()));
        assert!(hosts.contains(&"controller.example.com:9443".to_string()));
        assert!(hosts.contains(&"localhost".to_string()));
    }

    #[test]
    fn allowed_hosts_strips_port_from_san_if_present() {
        let sans = vec!["controller.example.com:8443".to_string()];
        let hosts = build_allowed_hosts(&sans);
        assert!(hosts.contains(&"controller.example.com".to_string()));
    }
}
```

Create stub `crates/ui/web-api/src/mcp/tools/mod.rs`. The stub must include a minimal `rmcp::ServerHandler` impl so
`build_mcp_router` compiles — without it, `StreamableHttpService::new` cannot accept `McpHandler`
(it requires `S: rmcp::ServerHandler`):

```rust
use std::sync::Arc;
use crate::AppState;

#[derive(Clone)]
pub struct McpHandler {
    pub(crate) state: Arc<AppState>,
}

impl McpHandler {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }
}

// Minimal ServerHandler impl so build_mcp_router compiles.
// Tools are wired in Task 6–8; this stub returns an empty tool list.
impl rmcp::ServerHandler for McpHandler {
    fn get_info(&self) -> rmcp::model::ServerInfo {
        rmcp::model::ServerInfo {
            name: "uptrakit".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            ..Default::default()
        }
    }
}
```

**Adaptation note:** Verify `rmcp::ServerHandler` trait name and `ServerInfo` struct against the installed rmcp version.
After adding rmcp to the workspace, run `cargo doc -p rmcp --features transport-streamable-http-server` to confirm exact
API. The trait may be named `Handler` or `Server`.

- [ ] **Step 5: Export `build_mcp_router` from `lib.rs`**

In `crates/ui/web-api/src/lib.rs`, add (cfg-gated):

```rust
#[cfg(feature = "mcp")]
pub mod mcp;
```

And export the function at the crate root so `controller-runtime` can call it:

```rust
#[cfg(feature = "mcp")]
pub use mcp::build_mcp_router;
```

- [ ] **Step 6: Merge MCP router in `server.rs`**

In `crates/core/controller-runtime/src/server.rs`, after `let mut router = uptrakit_web_api::build_router(cfg.app_state);`
and before any `.layer(...)` calls:

```rust
let mut router = uptrakit_web_api::build_router(cfg.app_state.clone());

#[cfg(feature = "mcp")]
{
    router = router.merge(uptrakit_web_api::build_mcp_router(Arc::clone(&cfg.app_state)));
}
```

- [ ] **Step 7: Run the tests**

```bash
cargo test -p uptrakit-web-api --features mcp -- mcp::tests 2>&1 | tail -10
```

Expected: PASS.

- [ ] **Step 8: Check it compiles with and without the feature**

```bash
cargo check -p uptrakit-web-api --no-default-features --features db-sqlite 2>&1 | grep "^error" | head -10
cargo check -p uptrakit-web-api --no-default-features --features db-sqlite,mcp 2>&1 | grep "^error" | head -10
cargo check -p uptrakit-controller-runtime --no-default-features 2>&1 | grep "^error" | head -10
cargo check -p uptrakit-controller-runtime 2>&1 | grep "^error" | head -10
```

Expected: no errors.

- [ ] **Step 9: Commit**

```bash
git add crates/ui/web-api/Cargo.toml \
        crates/ui/web-api/src/lib.rs \
        crates/ui/web-api/src/mcp/mod.rs \
        crates/ui/web-api/src/mcp/tools/mod.rs \
        crates/core/controller-runtime/Cargo.toml \
        crates/core/controller-runtime/src/server.rs \
        Cargo.toml
git commit -m "feat(mcp): scaffold mcp feature flag, skeleton router, server.rs merge

Adds mcp feature (default-on) to web-api and controller-runtime.
build_mcp_router() returns Router<Arc<AppState>> merged before middleware
layers in server.rs so all middleware covers /mcp. allowed_hosts built
from settings SANs with port-stripping and :9443 variants to avoid rmcp
403 on non-localhost deployments."
```

---

## Task 4: MCP auth Tower layer

**Files:**

- Modify: `crates/ui/web-api/src/mcp/auth.rs`
- Modify: `crates/ui/web-api/src/mcp/mod.rs` — wire auth layer into `build_mcp_router`
- Modify: `crates/ui/web-api/src/middleware/require_auth.rs` — make `emit_api_token_auth_audit` `pub(crate)` and export `AuthFailure`
- Modify: `crates/ui/web-api/src/middleware/request_log.rs` — verify tolerates absent `AuthenticatedUser`

### Context

The Tower auth layer wraps `StreamableHttpService`. It reads the `Authorization` header, validates via
`ApiTokenService::verify_token()`, checks `Permission::AccessMcp`, and inserts `McpRequestContext` into
`http::request::Extensions`. JWT tokens (no `upk_` prefix) are rejected with a descriptive error. The layer emits
`AUTH_API_TOKEN_AUTHENTICATE` audit events — success and failure — identical to `require_auth`.

`McpRequestContext` is extracted in tool handlers via `Extension<http::request::Parts>` (rmcp's own extension type,
not Axum's) — rmcp injects the original HTTP request parts per tool call.

`emit_api_token_auth_audit` is currently `fn` (private) in `require_auth.rs`. Make it `pub(crate)` so `auth.rs`
can reuse it without duplicating audit logic.

- [ ] **Step 1: Write the failing tests**

Create `crates/ui/web-api/src/mcp/auth.rs` with a test module at the bottom. These are unit tests using a
mock/stub approach — full integration requires a live DB, so test the layer logic by constructing minimal requests.

```rust
#[cfg(test)]
mod tests {
    // Tests added after implementation — see Step 3.
    // Stub test to verify compilation:
    #[test]
    fn mcp_request_context_is_clone_send_sync() {
        fn assert_clone_send_sync<T: Clone + Send + Sync + 'static>() {}
        assert_clone_send_sync::<super::McpRequestContext>();
    }
}
```

- [ ] **Step 2: Make `emit_api_token_auth_audit` and `authenticate_api_token` accessible**

In `crates/ui/web-api/src/middleware/require_auth.rs`:

Change:

```rust
fn emit_api_token_auth_audit(
```

To:

```rust
pub(crate) fn emit_api_token_auth_audit(
```

(Already `pub(crate)` for `authenticate_api_token` — verify with `grep pub.*authenticate_api_token require_auth.rs`.)

- [ ] **Step 3: Implement `McpRequestContext` and the auth Tower layer**

Write `crates/ui/web-api/src/mcp/auth.rs`:

```rust
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::http::{Request, Response, StatusCode};
use tower::{Layer, Service};
use uuid::Uuid;

use crate::AppState;
use crate::auth::api_token::ApiTokenService;
use crate::auth::permissions::Permission;
use crate::middleware::require_auth::{authenticate_api_token, emit_api_token_auth_audit};

/// Per-request context injected by the MCP auth layer.
///
/// Extracted in tool handlers via `Extension<http::request::Parts>` (rmcp's
/// extension type) then `parts.extensions.get::<McpRequestContext>()`.
/// Must NOT be stored on the McpHandler struct — the struct lives for the
/// entire session; this context is per-request.
#[derive(Clone, Debug)]
pub struct McpRequestContext {
    pub user_id: Uuid,
    pub token_id: Uuid,
    pub tenant_id: Uuid,
    pub permissions: Vec<Permission>,
}

impl McpRequestContext {
    pub fn has_permission(&self, perm: &Permission) -> bool {
        self.permissions.contains(perm)
    }
}

/// Tower layer that validates Bearer tokens for MCP requests.
///
/// Only accepts `upk_`-prefixed API tokens. JWT access tokens are rejected
/// with a descriptive message explaining why they're unsuitable for persistent
/// MCP connections.
#[derive(Clone)]
pub struct McpAuthLayer {
    pub(crate) state: Arc<AppState>,
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

#[derive(Clone)]
pub struct McpAuthService<S> {
    inner: S,
    state: Arc<AppState>,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for McpAuthService<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
    ResBody: Default + Send + 'static,
{
    type Response = Response<ResBody>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<ReqBody>) -> Self::Future {
        let state = Arc::clone(&self.state);
        let inner = self.inner.clone();
        // Required: take the ready inner and replace with the clone
        let mut inner = std::mem::replace(&mut self.inner, inner);

        Box::pin(async move {
            let token = match extract_bearer_token(req.headers()) {
                Some(t) => t,
                None => {
                    emit_api_token_auth_audit(
                        &state,
                        None,
                        uptrakit_audit_log::AuditOutcome::Denied,
                        "missing_authorization_header",
                    );
                    return Ok(unauthorized_response());
                }
            };

            // Reject JWT tokens upfront — they expire and break persistent connections
            if !token.starts_with("upk_") {
                emit_api_token_auth_audit(
                    &state,
                    None,
                    uptrakit_audit_log::AuditOutcome::Denied,
                    "jwt_not_accepted_for_mcp",
                );
                return Ok(unauthorized_response());
            }

            let (auth_user, token_id) = match authenticate_api_token(&state, &token).await {
                Ok(result) => result,
                Err(failure) => {
                    if let Some(reason) = failure.api_token_reason_code() {
                        emit_api_token_auth_audit(
                            &state,
                            None,
                            uptrakit_audit_log::AuditOutcome::Denied,
                            reason,
                        );
                    }
                    use crate::middleware::require_auth::AuthFailure;
                    let resp = match failure {
                        AuthFailure::UserDeactivated => forbidden_response(),
                        _ => unauthorized_response(),
                    };
                    return Ok(resp);
                }
            };

            if !auth_user.has_permission(Permission::AccessMcp) {
                emit_api_token_auth_audit(
                    &state,
                    None,
                    uptrakit_audit_log::AuditOutcome::Denied,
                    "missing_access_mcp_permission",
                );
                return Ok(forbidden_response());
            }

            emit_api_token_auth_audit(
                &state,
                None,
                uptrakit_audit_log::AuditOutcome::Success,
                "authenticated",
            );

            let ctx = McpRequestContext {
                user_id: auth_user.user_id,
                token_id,
                tenant_id: state.default_tenant_id,
                permissions: auth_user.permissions,
            };
            req.extensions_mut().insert(ctx);

            inner.call(req).await
        })
    }
}

fn extract_bearer_token(headers: &axum::http::HeaderMap) -> Option<String> {
    let value = headers.get(axum::http::header::AUTHORIZATION)?.to_str().ok()?;
    value.strip_prefix("Bearer ").map(|t| t.to_string())
}

fn unauthorized_response<B: Default>() -> Response<B> {
    let mut resp = Response::new(B::default());
    *resp.status_mut() = StatusCode::UNAUTHORIZED;
    resp
}

fn forbidden_response<B: Default>() -> Response<B> {
    let mut resp = Response::new(B::default());
    *resp.status_mut() = StatusCode::FORBIDDEN;
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_request_context_is_clone_send_sync() {
        fn assert_clone_send_sync<T: Clone + Send + Sync + 'static>() {}
        assert_clone_send_sync::<McpRequestContext>();
    }
}
```

**Note on `api_token_reason_code`:** `AuthFailure::api_token_reason_code()` is currently `fn` (not `pub`). Make it
`pub(crate)` in `require_auth.rs`, along with `AuthFailure` itself. The auth layer maps `AuthFailure` to a
status-only `Response<B>` (body is `B::default()`) — `UserDeactivated` → 403, everything else → 401. This avoids
needing `error_response()` which returns a typed `Response<Body>` incompatible with the generic `B`.

Make the following changes to `require_auth.rs`:

```rust
// Change:
pub(crate) enum AuthFailure {
// Also make api_token_reason_code pub(crate):
pub(crate) fn api_token_reason_code(&self) -> Option<&'static str> {
```

- [ ] **Step 4: Wire auth layer into `build_mcp_router`**

In `crates/ui/web-api/src/mcp/mod.rs`, update `build_mcp_router` to wrap the service with the auth layer:

```rust
pub fn build_mcp_router(state: Arc<AppState>) -> Router<Arc<AppState>> {
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService,
        session::local::LocalSessionManager,
    };
    use tower::ServiceBuilder;

    let allowed_hosts = build_allowed_hosts(&state.settings.sans());
    let config = StreamableHttpServerConfig {
        allowed_hosts,
        ..StreamableHttpServerConfig::default()
    };

    let handler = crate::mcp::tools::McpHandler::new(Arc::clone(&state));
    // 3-arg form: (service_factory, session_manager, config) — matches Task 3 scaffold.
    let raw_svc = StreamableHttpService::new(
        move || Ok(handler.clone()),
        Arc::new(LocalSessionManager::default()),
        config,
    );

    let auth_layer = crate::mcp::auth::McpAuthLayer::new(Arc::clone(&state));
    let authed_svc = ServiceBuilder::new().layer(auth_layer).service(raw_svc);

    Router::new().nest_service("/mcp", authed_svc)
}
```

- [ ] **Step 5: Verify request_log tolerates absent `AuthenticatedUser`**

Check `crates/ui/web-api/src/middleware/request_log.rs` — find all reads of `AuthenticatedUser`:

```bash
grep -n "AuthenticatedUser\|extensions().get" crates/ui/web-api/src/middleware/request_log.rs
```

If any `.get::<AuthenticatedUser>()` is followed by `.unwrap()` or similar (not `if let Some`), fix it to use `Option` safely.

- [ ] **Step 6: Run the tests and check compilation**

```bash
cargo test -p uptrakit-web-api --features mcp -- mcp::auth::tests 2>&1 | tail -10
cargo check -p uptrakit-web-api --features mcp,db-sqlite 2>&1 | grep "^error" | head -20
```

Expected: test PASS, no compile errors.

- [ ] **Step 7: Commit**

```bash
git add crates/ui/web-api/src/mcp/auth.rs \
        crates/ui/web-api/src/mcp/mod.rs \
        crates/ui/web-api/src/middleware/require_auth.rs \
        crates/ui/web-api/src/middleware/request_log.rs
git commit -m "feat(mcp): Tower auth layer — API token validation, AccessMcp check, audit events

Wraps StreamableHttpService with a dedicated Tower layer. Validates upk_
prefixed API tokens via ApiTokenService, checks Permission::AccessMcp,
rejects JWT tokens with a descriptive error. Emits AUTH_API_TOKEN_AUTHENTICATE
audit events on both success and failure paths."
```

---

## Task 5: MCP terminal output renderer

**Files:**

- Create: `crates/ui/web-api/src/mcp/terminal.rs`

### Context

`vt100` crate processes raw bytes through a virtual terminal emulator (width=220) and extracts the final screen state as
plain text. This correctly handles `\r` rewrites, cursor-up/down progress bar tricks (APT, pip, etc.), and strips all
ANSI escape sequences. Width 220 avoids wrapping on typical package manager output.

The output field from `UpdateHistoryResponse` is a `String`. Convert to bytes via `.as_bytes()` before feeding to vt100.
Extract each row of the final screen and join with newlines. Trim trailing blank rows.

- [ ] **Step 1: Write the failing tests**

Create `crates/ui/web-api/src/mcp/terminal.rs`:

```rust
pub fn render_terminal_output(raw: &[u8]) -> String {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_passthrough() {
        let input = b"hello\nworld\n";
        let result = render_terminal_output(input);
        assert!(result.contains("hello"), "expected 'hello' in output, got: {result:?}");
        assert!(result.contains("world"), "expected 'world' in output, got: {result:?}");
    }

    #[test]
    fn carriage_return_collapses() {
        // \r overwrites the beginning of the line — final line should be "done"
        let input = b"loading\rdone\n";
        let result = render_terminal_output(input);
        assert!(result.contains("done"), "expected 'done', got: {result:?}");
        assert!(!result.contains("loading"), "expected 'loading' to be overwritten, got: {result:?}");
    }

    #[test]
    fn ansi_sequences_stripped() {
        // ESC[1m is bold, ESC[0m is reset
        let input = b"\x1b[1mBold text\x1b[0m normal";
        let result = render_terminal_output(input);
        assert!(result.contains("Bold text"), "expected 'Bold text', got: {result:?}");
        assert!(!result.contains("\x1b"), "expected no ANSI escapes, got: {result:?}");
    }

    #[test]
    fn cursor_up_progress_bar_collapses() {
        // Simulate a progress bar: print line, then move cursor up and overwrite
        // ESC[1A moves cursor up one line
        let input = b"0%\n\x1b[1A100%\n";
        let result = render_terminal_output(input);
        assert!(result.contains("100%"), "expected '100%', got: {result:?}");
    }

    #[test]
    fn multibyte_utf8_boundary_safe() {
        // "Hello" in Japanese: ハロー (3 bytes each)
        let input = "ハロー\n".as_bytes();
        let result = render_terminal_output(input);
        assert!(result.contains("ハロー"), "expected Japanese text, got: {result:?}");
    }

    #[test]
    fn empty_input_returns_empty_string() {
        assert_eq!(render_terminal_output(b""), "");
    }

    #[test]
    fn trailing_blank_rows_trimmed() {
        let input = b"output line\n";
        let result = render_terminal_output(input);
        // Should not end with many trailing newlines from blank vt100 screen rows
        let trimmed = result.trim_end();
        assert_eq!(result.trim_end(), trimmed);
        // Specifically: should not have more than one trailing newline
        assert!(!result.ends_with("\n\n"), "trailing blank rows not trimmed: {result:?}");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p uptrakit-web-api --features mcp -- mcp::terminal::tests 2>&1 | tail -10
```

Expected: FAIL with `not yet implemented`.

- [ ] **Step 3: Implement `render_terminal_output`**

```rust
/// Render raw terminal output bytes into plain text.
///
/// Feeds bytes through a vt100 terminal emulator at width=220 to correctly
/// collapse \r rewrites, cursor-up/down progress bar tricks, and strip ANSI
/// escape sequences. Width 220 avoids line wrapping on typical package manager
/// output.
pub fn render_terminal_output(raw: &[u8]) -> String {
    if raw.is_empty() {
        return String::new();
    }

    let mut parser = vt100::Parser::new(80, 220, 0);
    parser.process(raw);

    let screen = parser.screen();
    let rows = screen.rows_formatted(0, 220);

    let lines: Vec<String> = rows
        .map(|row| {
            // vt100 rows are byte sequences; convert to String and trim trailing spaces
            String::from_utf8_lossy(&row).trim_end().to_string()
        })
        .collect();

    // Trim trailing blank rows
    let last_non_empty = lines.iter().rposition(|l| !l.is_empty());
    match last_non_empty {
        Some(idx) => lines[..=idx].join("\n"),
        None => String::new(),
    }
}
```

**Note on `vt100` API:** `Parser::new(rows, cols, scrollback)` — use `rows=80` (screen height, enough for most output),
`cols=220`. `screen.rows_formatted(start_row, cols)` returns an iterator of `Vec<u8>` byte sequences per row.
Verify the exact API by checking the vt100 crate docs:

```bash
cargo doc -p vt100 --open 2>/dev/null || cargo doc -p uptrakit-web-api --features mcp 2>&1 | tail -5
```

If the API differs, adjust accordingly. The key methods are `Parser::process(&mut self, bytes: &[u8])`,
`Parser::screen() -> &Screen`, `Screen::rows_formatted(start: u16, width: u16) -> impl Iterator<Item = Vec<u8>>`.

- [ ] **Step 4: Run the tests**

```bash
cargo test -p uptrakit-web-api --features mcp -- mcp::terminal::tests 2>&1 | tail -15
```

Expected: all PASS. If `cursor_up_progress_bar_collapses` or `carriage_return_collapses` fail, verify the vt100 API is
being called correctly — the emulator must process the bytes, not just strip escape codes.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/web-api/src/mcp/terminal.rs
git commit -m "feat(mcp): vt100 terminal renderer for update history output

render_terminal_output() feeds raw bytes through a vt100 emulator at
width=220, extracts final screen state as plain text. Correctly collapses
\\r rewrites, cursor-up/down progress bars, and strips ANSI sequences."
```

---

## Task 6: `get_current_user` tool

**Files:**

- Modify: `crates/ui/web-api/src/mcp/tools/mod.rs` — add `rmcp::ServerHandler` impl, wire `get_current_user`
- Create: `crates/ui/web-api/src/mcp/tools/user.rs`

### Context

`get_current_user` fetches identity for the token owner. It uses `McpRequestContext.user_id` from
`parts.extensions.get::<McpRequestContext>()`. One DB lookup: `User::find_by_id(user_id)` for `email`,
`first_name`, `last_name`. Returns `user_id`, `email`, `first_name`, `last_name`, `permissions[]`.
Does NOT require `ManageUsers` — this is a self-profile lookup equivalent to `GET /api/v1/auth/me`.

rmcp tool handlers receive `Extension<http::request::Parts>` as a parameter
(import: `rmcp::handler::server::tool::Extension`, not `axum::extract::Extension`).
rmcp injects the HTTP request parts once per tool call.

- [ ] **Step 1: Write the failing test**

Create `crates/ui/web-api/src/mcp/tools/user.rs`:

```rust
#[cfg(test)]
mod tests {
    // Integration test requires live DB — tested manually in Task 8 end-to-end.
    // Unit test: verify McpRequestContext extraction compiles and the tool struct exists.
    #[test]
    fn get_current_user_tool_exists() {
        // Compilation test — if GetCurrentUserTool struct is defined, this passes.
        let _: Option<super::GetCurrentUserResult> = None;
    }
}
```

- [ ] **Step 2: Implement `user.rs`**

```rust
use rmcp::handler::server::tool::Extension;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use uptrakit_shared_db::entity::prelude::*;

use crate::mcp::auth::McpRequestContext;
use crate::mcp::tools::mcp_error;

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GetCurrentUserResult {
    pub user_id: Uuid,
    pub email: String,
    pub first_name: String,
    pub last_name: String,
    pub permissions: Vec<String>,
}

pub async fn get_current_user(
    state: std::sync::Arc<crate::AppState>,
    Extension(parts): Extension<http::request::Parts>,
) -> Result<rmcp::model::CallToolResult, rmcp::Error> {
    let ctx = parts
        .extensions
        .get::<McpRequestContext>()
        .ok_or_else(|| mcp_error("missing MCP request context"))?;

    let user = User::find_by_id(ctx.user_id)
        .one(state.db())
        .await
        .map_err(|e| mcp_error(format!("database error: {e}")))?
        .ok_or_else(|| mcp_error("user not found"))?;

    let result = GetCurrentUserResult {
        user_id: ctx.user_id,
        email: user.email,
        first_name: user.first_name,
        last_name: user.last_name,
        permissions: ctx.permissions.iter().map(|p| p.as_str().to_string()).collect(),
    };

    Ok(rmcp::model::CallToolResult::success(vec![
        rmcp::model::Content::text(
            serde_json::to_string_pretty(&result)
                .unwrap_or_else(|e| format!("serialization error: {e}")),
        ),
    ]))
}
```

Create `crates/ui/web-api/src/mcp/tools/mod.rs` with the `ServerHandler` impl:

```rust
pub mod user;

use std::sync::Arc;
use crate::AppState;

#[derive(Clone)]
pub struct McpHandler {
    pub(crate) state: Arc<AppState>,
}

impl McpHandler {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }
}

/// Helper: create an rmcp internal error from a message string.
pub(crate) fn mcp_error(msg: impl Into<String>) -> rmcp::Error {
    rmcp::Error::internal_error(msg.into(), None)
}

#[rmcp::tool(tool_box)]
impl McpHandler {
    /// Returns identity information about the API token owner.
    #[rmcp::tool(description = "Returns identity information about the API token owner.")]
    pub async fn get_current_user(
        &self,
        #[rmcp::tool(extension)] ext: rmcp::handler::server::tool::Extension<http::request::Parts>,
    ) -> Result<rmcp::model::CallToolResult, rmcp::Error> {
        user::get_current_user(Arc::clone(&self.state), ext).await
    }
}
```

**Note on rmcp macro API:** The exact rmcp 1.x `#[tool]` macro syntax needs verification.
Check the rmcp docs or examples:

```bash
cargo doc -p rmcp --features transport-streamable-http-server 2>&1 | tail -5
```

The key: implement `rmcp::ServerHandler` for `McpHandler`. Tools are async methods annotated with `#[rmcp::tool]`.
The `Extension<T>` parameter type is `rmcp::handler::server::tool::Extension<T>`.
Adjust macro syntax to match what rmcp 1.x actually provides.

Also implement the required `rmcp::ServerHandler` trait (capabilities + handler info):

```rust
impl rmcp::ServerHandler for McpHandler {
    fn get_info(&self) -> rmcp::model::ServerInfo {
        rmcp::model::ServerInfo {
            name: "uptrakit".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            ..Default::default()
        }
    }
}
```

- [ ] **Step 3: Run the test and check compilation**

```bash
cargo test -p uptrakit-web-api --features mcp -- mcp::tools::user::tests 2>&1 | tail -10
cargo check -p uptrakit-web-api --features mcp,db-sqlite 2>&1 | grep "^error" | head -20
```

Expected: test PASS, compiles.

- [ ] **Step 4: Commit**

```bash
git add crates/ui/web-api/src/mcp/tools/mod.rs \
        crates/ui/web-api/src/mcp/tools/user.rs
git commit -m "feat(mcp): get_current_user tool — identity for the token owner

Returns user_id, email, first/last name, permissions. Self-profile
lookup equivalent to GET /api/v1/auth/me; ManageUsers not required."
```

---

## Task 7: `list_update_history` and `get_update_history_detail` tools

**Files:**

- Create: `crates/ui/web-api/src/mcp/tools/history.rs`
- Modify: `crates/ui/web-api/src/mcp/tools/mod.rs` — wire both tools

### Context

Both tools require `AccessMcp` + `ViewSoftware`. `TenantDb` is constructed from `state.default_tenant_id` and
`state.db()`. The `output` field on `UpdateHistoryResponse` always carries content — `list_update_history` must
clear it (set to empty string) on each record before returning. `get_update_history_detail` feeds the raw output
bytes through `render_terminal_output`.

- [ ] **Step 1: Write the failing tests**

Create `crates/ui/web-api/src/mcp/tools/history.rs`:

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn history_tool_types_exist() {
        let _: Option<super::ListUpdateHistoryInput> = None;
        let _: Option<super::GetUpdateHistoryDetailInput> = None;
    }
}
```

- [ ] **Step 2: Implement `history.rs`**

```rust
use rmcp::handler::server::tool::Extension;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use uptrakit_web_api_queries::queries::update_history;
use uptrakit_web_api_queries::tenant_db::TenantDb;
use uptrakit_web_api_types::update_history::{UpdateHistoryQuery, UpdateHistoryResponse};

use crate::auth::permissions::Permission;
use crate::mcp::auth::McpRequestContext;
use crate::mcp::terminal::render_terminal_output;
use crate::mcp::tools::mcp_error;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListUpdateHistoryInput {
    pub host_id: Option<Uuid>,
    pub software_item_id: Option<Uuid>,
    pub status: Option<String>,
    pub page: Option<u64>,
    pub per_page: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetUpdateHistoryDetailInput {
    pub update_history_id: Uuid,
}

pub async fn list_update_history(
    state: std::sync::Arc<crate::AppState>,
    Extension(parts): Extension<http::request::Parts>,
    input: ListUpdateHistoryInput,
) -> Result<rmcp::model::CallToolResult, rmcp::Error> {
    let ctx = parts
        .extensions
        .get::<McpRequestContext>()
        .ok_or_else(|| mcp_error("missing MCP request context"))?;

    if !ctx.has_permission(&Permission::ViewSoftware) {
        return Err(mcp_error("permission denied: requires view_software"));
    }

    let tenant_db = TenantDb::new(state.db().clone(), ctx.tenant_id);

    let status = input
        .status
        .as_deref()
        .map(|s| s.parse().ok())
        .flatten();

    let query = UpdateHistoryQuery::new(
        input.host_id,
        input.software_item_id,
        status,
        input.page,
        input.per_page,
    );

    let mut paginated = update_history::list_update_history(&tenant_db, &query)
        .await
        .map_err(|e| mcp_error(format!("database error: {e}")))?;

    // Clear output on each record — list view does not include terminal output
    for item in &mut paginated.items {
        item.output = String::new();
    }

    Ok(rmcp::model::CallToolResult::success(vec![
        rmcp::model::Content::text(
            serde_json::to_string_pretty(&paginated)
                .unwrap_or_else(|e| format!("serialization error: {e}")),
        ),
    ]))
}

pub async fn get_update_history_detail(
    state: std::sync::Arc<crate::AppState>,
    Extension(parts): Extension<http::request::Parts>,
    input: GetUpdateHistoryDetailInput,
) -> Result<rmcp::model::CallToolResult, rmcp::Error> {
    let ctx = parts
        .extensions
        .get::<McpRequestContext>()
        .ok_or_else(|| mcp_error("missing MCP request context"))?;

    if !ctx.has_permission(&Permission::ViewSoftware) {
        return Err(mcp_error("permission denied: requires view_software"));
    }

    let tenant_db = TenantDb::new(state.db().clone(), ctx.tenant_id);

    let mut record = update_history::get_update_history(&tenant_db, input.update_history_id)
        .await
        .map_err(|e| mcp_error(format!("database error: {e}")))?
        .ok_or_else(|| mcp_error("update history record not found"))?;

    // Render terminal output: strip ANSI, collapse \r and cursor tricks
    record.output = render_terminal_output(record.output.as_bytes());

    Ok(rmcp::model::CallToolResult::success(vec![
        rmcp::model::Content::text(
            serde_json::to_string_pretty(&record)
                .unwrap_or_else(|e| format!("serialization error: {e}")),
        ),
    ]))
}

#[cfg(test)]
mod tests {
    #[test]
    fn history_tool_types_exist() {
        let _: Option<super::ListUpdateHistoryInput> = None;
        let _: Option<super::GetUpdateHistoryDetailInput> = None;
    }
}
```

**Note on `TenantDb::new`:** Check the actual constructor in `crates/ui/web-api-queries/src/tenant_db.rs`:

```bash
grep -n "pub fn new\|pub(crate) fn new" crates/ui/web-api-queries/src/tenant_db.rs
```

Use whatever constructor is available.

**Note on `UpdateHistoryResponse.output` mutability:** `UpdateHistoryResponse` derives `Serialize` + `Deserialize` but
may not have pub fields or a mutable `output`. If `output` is not pub-mutable, use struct update syntax or create a
wrapper type. Check:

```bash
grep -n "pub output" crates/shared/web-api-types/src/update_history.rs
```

If `output` is `pub`, direct assignment works. If not, consider adding a helper or making the field pub in the types crate.

- [ ] **Step 3: Wire tools into `McpHandler`**

In `crates/ui/web-api/src/mcp/tools/mod.rs`, add `pub mod history;` and extend the `#[rmcp::tool(tool_box)]` impl:

```rust
pub mod history;
pub mod user;

// ... existing McpHandler struct and mcp_error fn ...

#[rmcp::tool(tool_box)]
impl McpHandler {
    #[rmcp::tool(description = "Returns identity information about the API token owner.")]
    pub async fn get_current_user(
        &self,
        #[rmcp::tool(extension)] ext: rmcp::handler::server::tool::Extension<http::request::Parts>,
    ) -> Result<rmcp::model::CallToolResult, rmcp::Error> {
        user::get_current_user(Arc::clone(&self.state), ext).await
    }

    #[rmcp::tool(description = "Returns a paginated list of update records. Output field is excluded.")]
    pub async fn list_update_history(
        &self,
        #[rmcp::tool(extension)] ext: rmcp::handler::server::tool::Extension<http::request::Parts>,
        #[rmcp::tool(aggr)] input: history::ListUpdateHistoryInput,
    ) -> Result<rmcp::model::CallToolResult, rmcp::Error> {
        history::list_update_history(Arc::clone(&self.state), ext, input).await
    }

    #[rmcp::tool(description = "Returns full detail for a single update record including rendered terminal output.")]
    pub async fn get_update_history_detail(
        &self,
        #[rmcp::tool(extension)] ext: rmcp::handler::server::tool::Extension<http::request::Parts>,
        #[rmcp::tool(aggr)] input: history::GetUpdateHistoryDetailInput,
    ) -> Result<rmcp::model::CallToolResult, rmcp::Error> {
        history::get_update_history_detail(Arc::clone(&self.state), ext, input).await
    }
}
```

- [ ] **Step 4: Run tests and check compilation**

```bash
cargo test -p uptrakit-web-api --features mcp -- mcp::tools::history::tests 2>&1 | tail -10
cargo check -p uptrakit-web-api --features mcp,db-sqlite 2>&1 | grep "^error" | head -20
```

Expected: test PASS, compiles.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/web-api/src/mcp/tools/history.rs \
        crates/ui/web-api/src/mcp/tools/mod.rs
git commit -m "feat(mcp): list_update_history and get_update_history_detail tools

Both require ViewSoftware. list_update_history clears output field on
each record — list view never exposes terminal output. Detail view feeds
raw output through render_terminal_output (vt100 renderer)."
```

---

## Task 8: `trigger_update` tool

**Files:**

- Create: `crates/ui/web-api/src/mcp/tools/update.rs`
- Modify: `crates/ui/web-api/src/mcp/tools/mod.rs` — wire tool
- Modify: `crates/ui/web-api/src/routes/software_items/mod.rs` — make `emit_software_update_audit` `pub(crate)`

### Context

`trigger_update` requires `AccessMcp` + `TriggerUpdates`. Calls `item_actions::trigger_update()` directly (same path
as the REST handler). Must call `spawn_protection_and_dispatch` when `result.pending_protection_work` is `Some`.
Must emit `emit_software_update_audit` on both success and failure paths — MCP-triggered updates must appear in the
audit log.

`item_actions::trigger_update()` is `pub(crate)` in `crates/ui/web-api/src/actions/software_items.rs`. Since
`mcp/tools/update.rs` is in the same crate, this is accessible.

`emit_software_update_audit` is currently `fn` (private) in `routes/software_items/mod.rs`. Make it `pub(crate)` to
reuse from `mcp/tools/update.rs`.

Actor type logic: API token auth → `ActorType::ApiToken` / `actor_id = token_id.to_string()`. Since MCP only accepts
API tokens (JWT rejected in auth layer), always use `ActorType::ApiToken` and `ctx.token_id`.

`spawn_protection_and_dispatch` is `pub(crate)` in `crates/ui/web-api/src/update_orchestrator.rs` — accessible from
`mcp/tools/update.rs` in the same crate.

- [ ] **Step 1: Make `emit_software_update_audit` `pub(crate)`**

In `crates/ui/web-api/src/routes/software_items/mod.rs`, change line 117:

```rust
// Before:
fn emit_software_update_audit(

// After:
pub(crate) fn emit_software_update_audit(
```

- [ ] **Step 2: Write the failing test**

Create `crates/ui/web-api/src/mcp/tools/update.rs`:

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn trigger_update_input_type_exists() {
        let _: Option<super::TriggerUpdateInput> = None;
    }
}
```

- [ ] **Step 3: Implement `update.rs`**

```rust
use rmcp::handler::server::tool::Extension;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::sync::Arc;

use uptrakit_shared_db::entity::update_history::UpdateStatus as DbUpdateStatus;
use uptrakit_web_api_queries::tenant_db::TenantDb;
use uptrakit_web_api_types::software_items::TriggerUpdateStatus;

use crate::actions::software_items as item_actions;
use crate::auth::permissions::Permission;
use crate::mcp::auth::McpRequestContext;
use crate::mcp::tools::mcp_error;
use crate::queries::update_types::ActorType;
use crate::routes::software_items::emit_software_update_audit;
use crate::AppState;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TriggerUpdateInput {
    pub host_id: Uuid,
    pub software_item_id: Uuid,
    pub to_version: String,
}

#[derive(Debug, Serialize)]
pub struct TriggerUpdateResult {
    pub update_history_id: Uuid,
    pub status: String,
}

pub async fn trigger_update(
    state: Arc<AppState>,
    Extension(parts): Extension<http::request::Parts>,
    input: TriggerUpdateInput,
) -> Result<rmcp::model::CallToolResult, rmcp::Error> {
    let ctx = parts
        .extensions
        .get::<McpRequestContext>()
        .ok_or_else(|| mcp_error("missing MCP request context"))?;

    if !ctx.has_permission(&Permission::TriggerUpdates) {
        return Err(mcp_error("permission denied: requires trigger_updates"));
    }

    // MCP only accepts API tokens (JWT rejected in auth layer) — always ApiToken actor
    let actor_id = ctx.token_id.to_string();
    let tenant_db = TenantDb::new(state.db().clone(), ctx.tenant_id);

    let ctx_snapshot = ctx.clone();
    let to_version = input.to_version.clone();
    let mut_ctx = state.mutation_context();

    let result = match item_actions::trigger_update(
        &tenant_db,
        &mut_ctx,
        uptrakit_web_api_queries::queries::update_triggers::TriggerUpdateParams {
            tenant_id: tenant_db.tenant_id,
            item_id: input.software_item_id,
            host_id: input.host_id,
            to_version: to_version.clone(),
            actor_type: ActorType::ApiToken.as_str(),
            actor_id: &actor_id,
            release_info: None, // MVP: omitted
            interactive: false, // AI agent cannot interact with PTY
        },
    )
    .await
    {
        Ok(r) => r,
        Err(err) => {
            let (outcome, reason_code) = err.current_context().trigger_audit_classification();
            let audit_user = build_audit_user(&ctx_snapshot);
            let audit_token = crate::middleware::require_auth::AuthenticatedApiTokenId(ctx_snapshot.token_id);
            emit_software_update_audit(
                &state,
                ctx_snapshot.tenant_id,
                &audit_user,
                Some(audit_token),
                input.software_item_id,
                outcome,
                serde_json::json!({
                    "host_id": input.host_id,
                    "to_version": to_version,
                    "interactive": false,
                    "reason_code": reason_code,
                }),
            );
            return Err(mcp_error(format!("trigger_update failed: {err}")));
        }
    };

    if let Some(work) = result.pending_protection_work {
        crate::update_orchestrator::spawn_protection_and_dispatch(Arc::clone(&state), *work);
    }

    let status = match result.initial_status {
        DbUpdateStatus::Pending => TriggerUpdateStatus::Pending,
        DbUpdateStatus::Failed => TriggerUpdateStatus::Failed,
        _ => TriggerUpdateStatus::Queued,
    };

    let audit_user = build_audit_user(&ctx_snapshot);
    let audit_token = crate::middleware::require_auth::AuthenticatedApiTokenId(ctx_snapshot.token_id);
    emit_software_update_audit(
        &state,
        ctx_snapshot.tenant_id,
        &audit_user,
        Some(audit_token),
        input.software_item_id,
        classify_dispatch_outcome(result.initial_status),
        serde_json::json!({
            "host_id": input.host_id,
            "to_version": to_version,
            "interactive": false,
            "update_history_id": result.update_history_id,
            "dispatch_status": status.to_string(),
        }),
    );

    let out = TriggerUpdateResult {
        update_history_id: result.update_history_id,
        status: status.to_string(),
    };

    Ok(rmcp::model::CallToolResult::success(vec![
        rmcp::model::Content::text(
            serde_json::to_string_pretty(&out)
                .unwrap_or_else(|e| format!("serialization error: {e}")),
        ),
    ]))
}

/// Build a minimal AuthenticatedUser for audit emission from McpRequestContext.
///
/// MCP always uses API token auth. `jti` is None (API tokens have no JTI).
fn build_audit_user(ctx: &McpRequestContext) -> crate::middleware::require_auth::AuthenticatedUser {
    crate::middleware::require_auth::AuthenticatedUser {
        user_id: ctx.user_id,
        auth_method: crate::auth::AuthMethod::ApiToken,
        permissions: ctx.permissions.clone(),
        jti: None,
    }
}

fn classify_dispatch_outcome(status: DbUpdateStatus) -> uptrakit_audit_log::AuditOutcome {
    match status {
        DbUpdateStatus::Failed => uptrakit_audit_log::AuditOutcome::Failed,
        _ => uptrakit_audit_log::AuditOutcome::Success,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn trigger_update_input_type_exists() {
        let _: Option<super::TriggerUpdateInput> = None;
    }
}
```

**Note on `emit_software_update_audit` signature:** It takes `&AuthenticatedUser`. The `build_audit_user` helper
constructs one from `McpRequestContext`. Verify `AuthMethod::ApiToken` is in scope — it's in `crate::auth::AuthMethod`.
Check:

```bash
grep -n "pub enum AuthMethod\|ApiToken" crates/ui/web-api-auth/src/auth/mod.rs
```

**Note on `TriggerUpdateStatus::to_string`:** Check if `TriggerUpdateStatus` implements `Display` or `ToString`:

```bash
grep -n "Display\|to_string\|TriggerUpdateStatus" crates/shared/web-api-types/src/update_history.rs | head -10
```

If not, use `format!("{:?}", status)` or match to a string literal.

**Note on `TriggerUpdateParams` import path:** It's in `uptrakit_web_api_queries::queries::update_triggers`. Verify:

```bash
grep -n "pub struct TriggerUpdateParams" crates/ui/web-api-queries/src/queries/update_triggers.rs
```

- [ ] **Step 4: Wire into `McpHandler`**

In `crates/ui/web-api/src/mcp/tools/mod.rs`, add `pub mod update;` and extend the tool_box impl:

```rust
pub mod update;

// In #[rmcp::tool(tool_box)] impl McpHandler:
    #[rmcp::tool(description = "Triggers a software update for a specific host. interactive is always false for AI agents.")]
    pub async fn trigger_update(
        &self,
        #[rmcp::tool(extension)] ext: rmcp::handler::server::tool::Extension<http::request::Parts>,
        #[rmcp::tool(aggr)] input: update::TriggerUpdateInput,
    ) -> Result<rmcp::model::CallToolResult, rmcp::Error> {
        update::trigger_update(Arc::clone(&self.state), ext, input).await
    }
```

- [ ] **Step 5: Run tests and check compilation**

```bash
cargo test -p uptrakit-web-api --features mcp -- mcp::tools::update::tests 2>&1 | tail -10
cargo check -p uptrakit-web-api --features mcp,db-sqlite 2>&1 | grep "^error" | head -20
cargo check -p uptrakit-web-api --no-default-features --features db-sqlite 2>&1 | grep "^error" | head -10
```

Expected: test PASS, both feature variants compile.

- [ ] **Step 6: Full quality gates**

```bash
cargo fmt --all
cargo clippy --all-targets --no-default-features --features db-sqlite 2>&1 | grep "^error" | head -20
cargo clippy --all-targets --all-features 2>&1 | grep "^error" | head -20
cargo test --all-features 2>&1 | tail -20
```

Expected: no errors.

- [ ] **Step 7: Commit**

```bash
git add crates/ui/web-api/src/mcp/tools/update.rs \
        crates/ui/web-api/src/mcp/tools/mod.rs \
        crates/ui/web-api/src/routes/software_items/mod.rs
git commit -m "feat(mcp): trigger_update tool with protection dispatch and audit emission

Requires TriggerUpdates permission. Calls item_actions::trigger_update()
directly, spawns spawn_protection_and_dispatch when pending_protection_work
is Some, emits SOFTWARE_UPDATE_TRIGGERED audit on both success and failure
paths. interactive always false — AI agent cannot interact with PTY."
```

---

## Self-Review

**Spec coverage check:**

| Spec requirement | Task covering it |
| --- | --- |
| `Permission::Other(String)` with `#[strum(disabled)]`, infallible `FromStr`/`Deserialize`; remove `ParsePermissionError` | Task 1 |
| `AccessMcp` variant + DB migration to `viewer`, `operator`, `software_manager` | Task 2 |
| count assertion 33→34 | Task 2 |
| `mcp` feature (default-on), `rmcp` + `vt100` deps | Task 3 |
| `build_mcp_router` merged before middleware in `server.rs` | Task 3 |
| `allowed_hosts` from SANs with port-stripping + unit test | Task 3 |
| Tower auth layer, API-token-only, JWT rejection with descriptive message | Task 4 |
| `AUTH_API_TOKEN_AUTHENTICATE` audit on success and failure | Task 4 |
| Request-log middleware tolerates absent `AuthenticatedUser` | Task 4 |
| `McpRequestContext` per-request via `http::request::Extensions` | Task 4 |
| `render_terminal_output` via vt100 width=220 | Task 5 |
| `\r` collapse, cursor-up collapse, ANSI strip, UTF-8 safety tests | Task 5 |
| `get_current_user` tool — no `ManageUsers` required | Task 6 |
| `list_update_history` — clears output field | Task 7 |
| `get_update_history_detail` — vt100-rendered output | Task 7 |
| `trigger_update` — `interactive=false`, `spawn_protection_and_dispatch`, `emit_software_update_audit` | Task 8 |
| Actor type `ApiToken` for MCP, `actor_id = token_id` | Task 8 |
| `operator` partial-access documented in migration comment | Task 2 |

**Placeholder scan:** All code blocks contain actual implementation. No "TBD" or "TODO" in step content.

**Type consistency:**

- `McpRequestContext` — defined in Task 4, used consistently in Tasks 6, 7, 8
- `mcp_error` helper — defined in Task 6 `tools/mod.rs`, used in 6, 7, 8
- `render_terminal_output(raw: &[u8]) -> String` — defined Task 5, called Task 7
  as `render_terminal_output(record.output.as_bytes())`
- `build_allowed_hosts(sans: &[String]) -> Vec<String>` — defined Task 3, used in
  `build_mcp_router`
- `emit_software_update_audit` — made `pub(crate)` in Task 8 step 1

**Known adaptation points (not placeholders — require verification against actual rmcp 1.x API):**

- `#[rmcp::tool(tool_box)]` macro syntax and `Extension<T>` parameter form
- `vt100::Parser::new` row/col order and `Screen::rows_formatted` iterator item type
- `TriggerUpdateStatus::to_string` availability
- `TenantDb::new` constructor visibility
