# Plugin Boundary Hardening — Waves 1–2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create the `uptrakit-tenant-db` crate and wire `tenant_db()` onto both controller traits, establishing the DB-access seam without breaking
anything.

**Architecture:** Extract `TenantDb` + `TenantScoped` trait into a new minimal crate. Add `uptrakit-tenant-db` as a `plugin-ops`-gated dependency of
`plugin-infrastructure-core`. Add `#[cfg(feature = "plugin-ops")] fn tenant_db(&self) -> &TenantDb` to both controller traits and implement it in the
two concrete structs. Zero behaviour change.

**Tech Stack:** Rust, SeaORM, sea-orm (workspace), uuid (workspace)

---

## Task 1: Create `uptrakit-tenant-db` crate

**Files:**

- Create: `crates/shared/tenant-db/Cargo.toml`
- Create: `crates/shared/tenant-db/src/lib.rs`
- Create: `crates/shared/tenant-db/src/tenant_db.rs`
- Create: `crates/shared/tenant-db/src/tenant_scoped.rs`
- Modify: `Cargo.toml` (workspace root — add member + workspace dep)

- [ ] **Step 1: Create crate directory and Cargo.toml**

```toml
# crates/shared/tenant-db/Cargo.toml
[package]
name = "uptrakit-tenant-db"
description = "Uptrakit tenant-scoped database access primitives"
edition.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
version.workspace = true

[dependencies]
sea-orm = { workspace = true }
uuid = { workspace = true, features = ["v7", "serde"] }

[lints]
workspace = true
```

- [ ] **Step 2: Copy `TenantDb` into the new crate**

Read `crates/shared/db/src/tenant_db.rs`. The struct uses:

- `sea_orm::{ColumnTrait, DatabaseConnection, DeleteMany, JoinType, PrimaryKeyTrait, QueryFilter, QuerySelect, RelationDef, Select, UpdateMany}`
- `uuid::Uuid`
- `crate::entity::TenantScoped` (will become `crate::tenant_scoped::TenantScoped`)

Create `crates/shared/tenant-db/src/tenant_db.rs` with the exact content of `crates/shared/db/src/tenant_db.rs`, changing the `TenantScoped` import
to:

```rust
use crate::tenant_scoped::TenantScoped;
```

(Remove the `use crate::entity::TenantScoped;` line and replace with the above.)

- [ ] **Step 3: Copy the `TenantScoped` trait definition into the new crate**

`crates/shared/db/src/entity/tenant_scoped.rs` contains:

1. A `use` block importing entity modules
2. `pub trait TenantScoped: EntityTrait { fn tenant_id_column() -> Self::Column; }`
3. 22+ `impl TenantScoped for X::Entity` blocks

Create `crates/shared/tenant-db/src/tenant_scoped.rs` with **only the trait definition**:

```rust
use sea_orm::EntityTrait;

/// Marker trait for SeaORM entities scoped to a tenant via a `tenant_id` column.
///
/// Implementing this trait allows `TenantDb` to apply tenant-scoping filters automatically.
pub trait TenantScoped: EntityTrait {
    fn tenant_id_column() -> Self::Column;
}
```

Do NOT copy the `impl` blocks — they stay in `shared-db`.

- [ ] **Step 4: Create `crates/shared/tenant-db/src/lib.rs`**

```rust
pub mod tenant_db;
pub mod tenant_scoped;

pub use tenant_db::TenantDb;
pub use tenant_scoped::TenantScoped;
```

- [ ] **Step 5: Register crate in workspace `Cargo.toml`**

Add to the `[workspace] members` array in the root `Cargo.toml`:

```toml
"crates/shared/tenant-db",
```

Add to `[workspace.dependencies]`:

```toml
uptrakit-tenant-db = { path = "crates/shared/tenant-db", version = "0.0.1" }
```

- [ ] **Step 6: Verify crate compiles in isolation**

Run: `cargo check -p uptrakit-tenant-db`
Expected: compiles clean

- [ ] **Step 7: Commit**

```bash
git add crates/shared/tenant-db/ Cargo.toml Cargo.lock
git commit -m "feat(tenant-db): create uptrakit-tenant-db crate with TenantDb and TenantScoped"
```

---

## Task 2: Update `shared-db` to re-export from `uptrakit-tenant-db`

**Files:**

- Modify: `crates/shared/db/Cargo.toml`
- Modify: `crates/shared/db/src/lib.rs`
- Modify: `crates/shared/db/src/entity/tenant_scoped.rs`

- [ ] **Step 1: Add dep in `shared-db/Cargo.toml`**

Add to `[dependencies]` in `crates/shared/db/Cargo.toml`:

```toml
uptrakit-tenant-db = { workspace = true }
```

- [ ] **Step 2: Update `shared-db/src/lib.rs`**

Find the existing `pub mod tenant_db;` line and change it to re-export from the new crate. Also re-export `TenantScoped`. The existing `use
crate::entity::TenantScoped` references in `tenant_db.rs` should now pull from the new crate.

Open `crates/shared/db/src/lib.rs`. Find:

```rust
pub mod tenant_db;
```

Replace with:

```rust
pub use uptrakit_tenant_db::{TenantDb, TenantScoped};
```

Then delete the file `crates/shared/db/src/tenant_db.rs` (it has moved; the crate now re-exports).

- [ ] **Step 3: Shrink `shared-db/src/entity/tenant_scoped.rs` to impl-blocks only**

The current `crates/shared/db/src/entity/tenant_scoped.rs` starts with:

```rust
use sea_orm::EntityTrait;
use super::{audit_log, ...};

pub trait TenantScoped: EntityTrait { ... }

impl TenantScoped for audit_log::Entity { ... }
// ... 21 more impl blocks
```

Replace the trait definition and its `EntityTrait` import with an import from the new crate:

```rust
use uptrakit_tenant_db::TenantScoped;
use super::{
    audit_log, enrollment_token, host, host_discovery_allowlist, host_tag, notification_channel,
    notification_log, notification_rule, oidc_provider, plugin_config, plugin_type_setting,
    proxmox_host_mapping, scheduled_task, service, setting, settings_version, software_ignore,
    software_item, tenant_discovery_allowlist, update_batch, update_history, user_role,
};
// (Keep all existing `impl TenantScoped for X::Entity` blocks unchanged)
```

Remove `use sea_orm::EntityTrait;` (no longer needed — `TenantScoped` re-exported from `uptrakit_tenant_db` already carries its bound).

- [ ] **Step 4: Update `shared-db/src/entity/mod.rs`**

Find `pub mod tenant_scoped;` and `pub use tenant_scoped::TenantScoped;` lines.

The `pub use tenant_scoped::TenantScoped;` line now re-exports from the `impl`-only file, which in turn imports `TenantScoped` from
`uptrakit_tenant_db`. This is fine — `pub use tenant_scoped::TenantScoped;` just needs to remain as-is (the trait is imported and thus re-exported
through the `impl` file's public import). However, there's a simpler approach: change `entity/mod.rs` to:

```rust
// (keep existing pub mod lines for other entities unchanged)
pub mod tenant_scoped;
pub use uptrakit_tenant_db::TenantScoped;  // re-export directly from source
```

This is cleaner than relying on re-export-of-re-export. Check whether `pub use tenant_scoped::TenantScoped` exists in `entity/mod.rs` and replace it
with `pub use uptrakit_tenant_db::TenantScoped;`.

- [ ] **Step 5: Compile check**

Run: `cargo check -p uptrakit-shared-db --all-features`
Expected: compiles clean

- [ ] **Step 6: Verify TenantDb still reachable from shared-db**

```rust
// Quick smoke check — should compile
use uptrakit_shared_db::TenantDb;
use uptrakit_shared_db::TenantScoped;
```

Run: `cargo check --all-features`
Expected: clean

- [ ] **Step 7: Commit**

```bash
git add crates/shared/db/
git commit -m "refactor(shared-db): re-export TenantDb and TenantScoped from uptrakit-tenant-db"
```

---

## Task 3: Add `uptrakit-tenant-db` dep to the four plugin crates

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/Cargo.toml`
- Modify: `crates/plugins/releases/docker/Cargo.toml`
- Modify: `crates/plugins/notifications/email/Cargo.toml`
- Modify: `crates/plugins/notifications/telegram/Cargo.toml`

- [ ] **Step 1: Add direct dep to each crate**

In each `Cargo.toml` add to `[dependencies]`:

```toml
uptrakit-tenant-db = { workspace = true }
```

Find each crate's `Cargo.toml`:

- `crates/plugins/infrastructure/proxmox/Cargo.toml`
- `crates/plugins/releases/docker/Cargo.toml` (verify path: `crates/plugins/releases/docker/`)
- `crates/plugins/notifications/email/Cargo.toml`
- `crates/plugins/notifications/telegram/Cargo.toml`

- [ ] **Step 2: Compile check all four crates**

Run: `cargo check -p uptrakit-plugin-infrastructure-proxmox -p uptrakit-plugin-releases-docker -p uptrakit-notification-plugin-email -p
uptrakit-notification-plugin-telegram --all-features`
Expected: clean (no code changes needed yet — adding the dep is additive)

- [ ] **Step 3: Commit**

```bash
git add crates/plugins/
git commit -m "chore(deps): add uptrakit-tenant-db direct dep to four plugin crates"
```

---

## Task 4: Gate `uptrakit-tenant-db` in `plugin-infrastructure-core`

**Files:**

- Modify: `crates/plugins/infrastructure/core/Cargo.toml`

- [ ] **Step 1: Add gated dep**

In `crates/plugins/infrastructure/core/Cargo.toml`, the `plugin-ops` feature currently reads:

```toml
plugin-ops = ["dep:sea-orm"]
```

Add `uptrakit-tenant-db` as an optional dep gated behind `plugin-ops`:

```toml
plugin-ops = ["dep:sea-orm", "dep:uptrakit-tenant-db"]
```

And in `[dependencies]`:

```toml
uptrakit-tenant-db = { workspace = true, optional = true }
```

- [ ] **Step 2: Compile check without plugin-ops**

Run: `cargo check -p uptrakit-plugin-infrastructure-core --no-default-features`
Expected: clean (no `uptrakit-tenant-db` pulled in without the feature)

- [ ] **Step 3: Compile check with plugin-ops**

Run: `cargo check -p uptrakit-plugin-infrastructure-core --features plugin-ops`
Expected: clean

- [ ] **Step 4: Commit**

```bash
git add crates/plugins/infrastructure/core/Cargo.toml
git commit -m "feat(plugin-core): gate uptrakit-tenant-db behind plugin-ops feature"
```

---

## Task 5: Add `tenant_db()` to `SurfaceActionController` and `UpdateProtectionController`

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/roles.rs`

- [ ] **Step 1: Write a failing test**

At the bottom of `crates/plugins/infrastructure/core/src/roles.rs`, inside an existing `#[cfg(test)]` module or a new one, add:

```rust
#[cfg(test)]
mod wave2_tests {
    // No unit test possible here — trait method existence is verified at compile time.
    // Compile-test: the trait must have tenant_db() under plugin-ops feature.
    // This test is a build-gate: if it fails to compile, Wave 2 is broken.
    //
    // Real integration-level compile tests are in Tasks 6 and 7.
}
```

(Skip — compile-time tests for trait method presence aren't needed separately; the impl in Tasks 6 and 7 will serve as the compile gate.)

- [ ] **Step 2: Add `tenant_db()` to `SurfaceActionController` in `roles.rs`**

Locate `SurfaceActionController` in `crates/plugins/infrastructure/core/src/roles.rs` (around line 618). The trait currently has `fn tenant_id()`, `fn
user_id()`, and plugin-named store methods. Add the new method under a feature gate, inside the trait body:

```rust
/// Tenant-scoped database access — the sole persistence seam for plugin surface actions.
///
/// Only available when the `plugin-ops` feature is active.
#[cfg(feature = "plugin-ops")]
fn tenant_db(&self) -> &uptrakit_tenant_db::TenantDb;
```

Place it after `fn user_id()` and before the plugin-named store methods.

- [ ] **Step 3: Add `tenant_db()` to `UpdateProtectionController` in `roles.rs`**

Locate `UpdateProtectionController` in the same file. Add inside its trait body:

```rust
/// Tenant-scoped database access for the update protection workflow.
#[cfg(feature = "plugin-ops")]
fn tenant_db(&self) -> &uptrakit_tenant_db::TenantDb;
```

- [ ] **Step 3b: Update `TestController` impls to satisfy the new trait requirement**

`roles.rs` has a `TestController` in its `#[cfg(test)]` module (around line 1269) that implements
both `SurfaceActionController` and `UpdateProtectionController`. `update_protection.rs` in the
proxmox plugin also has a `TestController` that implements `UpdateProtectionController`. Once
`tenant_db()` is a required trait method (under `plugin-ops` which `cargo test --all-features`
enables), both will fail to compile.

In `crates/plugins/infrastructure/core/src/roles.rs` test module, add to the
`impl SurfaceActionController for TestController` block:

```rust
fn tenant_db(&self) -> &uptrakit_tenant_db::TenantDb {
    unimplemented!("tenant_db not used in roles.rs surface action tests")
}
```

And add to `impl UpdateProtectionController for TestController`:

```rust
fn tenant_db(&self) -> &uptrakit_tenant_db::TenantDb {
    unimplemented!("tenant_db not used in roles.rs protection tests")
}
```

In `crates/plugins/infrastructure/proxmox/src/update_protection.rs` test module (around line 830),
add to `impl UpdateProtectionController for TestController`:

```rust
fn tenant_db(&self) -> &uptrakit_tenant_db::TenantDb {
    unimplemented!("tenant_db not used in proxmox update_protection tests — will be replaced in Wave 3f")
}
```

Note: In Wave 3f these test stubs will be replaced with real `TenantDb` instances when the
protection tests are updated to use direct DB queries.

- [ ] **Step 4: Add `tenant_db()` convenience delegate to `SurfaceActionContext`**

`SurfaceActionContext` lives in `crates/plugins/infrastructure/core/src/descriptor.rs`. Locate the `impl SurfaceActionContext<'_>` block. Add:

```rust
/// Convenience delegate — tenant-scoped database access via the controller.
#[cfg(feature = "plugin-ops")]
pub fn tenant_db(&self) -> &uptrakit_tenant_db::TenantDb {
    self.controller.tenant_db()
}
```

- [ ] **Step 5: Compile check**

Run: `cargo check -p uptrakit-plugin-infrastructure-core --features plugin-ops`
Expected: compile error — `SurfaceActionController` now requires `tenant_db()` but the impl in `surface-proxy` doesn't have it yet (that's Task 6). If
it compiles clean, check whether the `#[cfg]` gate is working correctly.

Actually: compile errors are expected at this point — the impls in `surface-proxy` and `web-api-queries` don't satisfy the trait yet. Check only the
core crate:

Run: `cargo check -p uptrakit-plugin-infrastructure-core --features plugin-ops`
Expected: **clean** — trait additions are valid even without impls (we haven't tried to create an instance).

- [ ] **Step 6: Commit**

```bash
git add crates/plugins/infrastructure/core/src/roles.rs crates/plugins/infrastructure/core/src/descriptor.rs
git commit -m "feat(plugin-core): add tenant_db() seam to SurfaceActionController and UpdateProtectionController"
```

---

## Task 6: Implement `tenant_db()` on `AppStateSurfaceActionController`

**Files:**

- Modify: `crates/ui/surface-proxy/src/proxy/controller_local.rs`
- Modify: `crates/ui/surface-proxy/Cargo.toml`

- [ ] **Step 1: Add `uptrakit-tenant-db` dep to surface-proxy (if not already present via shared-db)**

Check `crates/ui/surface-proxy/Cargo.toml`. `surface-proxy` already depends on `uptrakit-shared-db` which re-exports `TenantDb`. Confirm that
`uptrakit_shared_db::TenantDb` resolves (it does after Task 2). No new dep needed in `surface-proxy/Cargo.toml`.

- [ ] **Step 2: Add `tenant_db` field to `AppStateSurfaceActionController`**

In `crates/ui/surface-proxy/src/proxy/controller_local.rs`, the struct currently reads:

```rust
pub struct AppStateSurfaceActionController<'a> {
    db: &'a sea_orm::DatabaseConnection,
    plugin_ops: &'a dyn PluginOps,
    tenant_id: Uuid,
    caller_user_id: Option<Uuid>,
}
```

Add the field — **no `#[cfg]` guard** (`surface-proxy` has no `plugin-ops` feature of its own; the
guard would always evaluate false, preventing the vtable from providing the method at runtime):

```rust
pub struct AppStateSurfaceActionController<'a> {
    db: &'a sea_orm::DatabaseConnection,
    plugin_ops: &'a dyn PluginOps,
    tenant_id: Uuid,
    caller_user_id: Option<Uuid>,
    tenant_db: uptrakit_shared_db::TenantDb,
}
```

`surface-proxy` already depends on `uptrakit-shared-db`; `TenantDb` is re-exported from it after
Task 2. No new dep entry needed.

- [ ] **Step 3: Update the constructor**

The `from_database_connection` constructor currently ends with:

```rust
Self {
    db,
    plugin_ops,
    tenant_id,
    caller_user_id,
}
```

Add the new field:

```rust
Self {
    db,
    plugin_ops,
    tenant_id,
    caller_user_id,
    tenant_db: uptrakit_shared_db::TenantDb::new(db.clone(), tenant_id),
}
```

`DatabaseConnection::clone()` is a cheap Arc clone of the connection pool.

- [ ] **Step 4: Implement the trait method**

Find `impl SurfaceActionController for AppStateSurfaceActionController<'_>` (around line 116). Add
(**no `#[cfg]` guard** — the field is always present; always satisfies the trait regardless of feature
unification):

```rust
fn tenant_db(&self) -> &uptrakit_shared_db::TenantDb {
    &self.tenant_db
}
```

- [ ] **Step 5: Compile check**

Run: `cargo check -p uptrakit-surface-proxy --all-features`
Expected: clean

- [ ] **Step 6: Commit**

```bash
git add crates/ui/surface-proxy/src/proxy/controller_local.rs crates/ui/surface-proxy/Cargo.toml
git commit -m "feat(surface-proxy): implement tenant_db() on AppStateSurfaceActionController"
```

---

## Task 7: Implement `tenant_db()` on `QueryUpdateProtectionController`

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/update_dispatch.rs`

- [ ] **Step 1: Update `QueryUpdateProtectionController` struct**

In `crates/ui/web-api-queries/src/queries/update_dispatch.rs` around line 440:

```rust
struct QueryUpdateProtectionController<'a> {
    proxmox_store: QueryProxmoxProtectionStore<'a>,
}
```

Add the new field. `web-api-queries` already uses `crate::TenantDb` (confirmed at line 993). The `#[cfg]` gate here matches the trait's gate; the
crate's `plugin-ops` feature must be active for the controller trait to require the method. In practice `web-api-queries` always activates
`plugin-ops`. Add without cfg gate (since the struct is only constructed when plugin-ops is active anyway):

```rust
struct QueryUpdateProtectionController<'a> {
    proxmox_store: QueryProxmoxProtectionStore<'a>,
    tenant_db: &'a crate::TenantDb,
}
```

- [ ] **Step 2: Update `new()` constructor**

Current (line ~445):

```rust
impl<'a> QueryUpdateProtectionController<'a> {
    fn new(db: &'a DatabaseConnection) -> Self {
        Self {
            proxmox_store: QueryProxmoxProtectionStore { db },
        }
    }
}
```

Change the constructor to accept a `&'a TenantDb`:

```rust
impl<'a> QueryUpdateProtectionController<'a> {
    fn new(tenant_db: &'a crate::TenantDb) -> Self {
        Self {
            proxmox_store: QueryProxmoxProtectionStore { db: tenant_db.db() },
            tenant_db,
        }
    }
}
```

- [ ] **Step 3: Implement `tenant_db()` on the trait**

In `impl UpdateProtectionController for QueryUpdateProtectionController<'_>` (around line 452), add
(**no `#[cfg]` guard** — `web-api-queries` has no `plugin-ops` feature; guard would always be false):

```rust
fn tenant_db(&self) -> &crate::TenantDb {
    self.tenant_db
}
```

- [ ] **Step 4: Update the two construction sites**

Line ~842 (`prepare_pre_update_protection`): `target: &ValidatedUpdateTarget` has `target.tenant_id: Uuid`. Change:

```rust
// Before:
let controller = QueryUpdateProtectionController::new(db);
// After:
let tenant_db = crate::TenantDb::new(db.clone(), target.tenant_id);
let controller = QueryUpdateProtectionController::new(&tenant_db);
```

Line ~898 (`finalize_post_update_inner`): `record: &update_history::Model` has `record.tenant_id: Uuid`. Change:

```rust
// Before:
let controller = QueryUpdateProtectionController::new(db);
// After:
let tenant_db = crate::TenantDb::new(db.clone(), record.tenant_id);
let controller = QueryUpdateProtectionController::new(&tenant_db);
```

- [ ] **Step 5: Compile check**

Run: `cargo check -p uptrakit-web-api-queries --all-features`
Expected: clean

- [ ] **Step 6: Run tests**

Run: `cargo test -p uptrakit-web-api-queries --all-features`
Expected: all pass — no behaviour change

- [ ] **Step 7: Full compile check**

Run: `cargo check --all-features`
Expected: clean

- [ ] **Step 8: Commit**

```bash
git add crates/ui/web-api-queries/src/queries/update_dispatch.rs
git commit -m "feat(web-api-queries): implement tenant_db() on QueryUpdateProtectionController"
```

---

## Task 8: Final verification

- [ ] **Step 1: Full quality gates for Waves 1–2**

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
```

All must pass before proceeding to Wave 3.

- [ ] **Step 2: Acceptance criteria check**

Verify:

- `TenantDb` reachable as `uptrakit_tenant_db::TenantDb` ✓
- `TenantDb` reachable as `uptrakit_shared_db::TenantDb` ✓
- `tenant_db()` callable on both controller traits ✓
- Old plugin-named store methods still compile (not removed yet) ✓
