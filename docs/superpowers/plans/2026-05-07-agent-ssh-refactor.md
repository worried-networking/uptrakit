# agent-ssh Refactor + Migration Strategy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move all business logic from `agent-ssh` into `agent-ssh-runtime`, produce a single
`AgentSshHandler` type that both the binary and the controller use, and replace the duplicated SSH
agent migration in `shared-db` with a repair migration + `service_migrations()`.

**Architecture:** `agent-ssh-runtime` absorbs all DB entities, migrations, crypto helpers, SSH logic,
surface runtime, and operations. It exports one `ServiceHandler` implementation (`AgentSshHandler`)
and a transitional `service_migrations()` free function. `agent-ssh` becomes a binary-only crate
(no `lib.rs`) — thin process init, clap parsing, and subcommand dispatch only. `controller-runtime`
drops its `uptrakit-agent-ssh` dep and rewires feature flags to point at the runtime crate.

**Tech Stack:** Rust 2024, SeaORM + sea-orm-migration, tokio, uptrakit-service-sdk, rcgen (for
controller ECIES keygen — stays in controller). Patterns: `#[async_trait] impl ServiceHandler`,
`impl MigrationTrait` with `execute_unprepared` for seaql_migrations surgery, Cargo
optional-dep feature forwarding.

---

## File Map

**Create in `crates/core/agent-ssh-runtime/src/`:**

- `db/` (moved from `agent-ssh/src/db/`) — entities + 13 migrations
- `client.rs`, `error.rs`, `host_info.rs`, `host_ops.rs` (moved)
- `operations/` (moved) — bootstrap, sync, sudoers, proxmox logic
- `remote_exec.rs`, `routeros_executor.rs` (moved)
- `runtime_support.rs` (moved) — `AgentSshRuntimeSupport` impl
- `ssh_executor.rs`, `ssh_key.rs`, `ssh_pool.rs`, `ssh_stdio_tunnel.rs`, `ssh_target.rs`, `ssh_transport.rs` (moved)
- `surface_runtime.rs` + `surface_runtime/` subdirectory (moved)
- `handler.rs` (**new**) — `AgentSshMode`, `EciesKeypair`, `AgentSshHandler`

**Modify:**

- `crates/core/agent-ssh-runtime/Cargo.toml` — add all heavy deps from agent-ssh
- `crates/core/agent-ssh-runtime/src/lib.rs` — export all moved modules + new types + `service_migrations()`
- `crates/core/agent-ssh/src/main.rs` — thin to process init + `AgentSshHandler::new` call
- `crates/core/agent-ssh/src/lib.rs` — **delete** (agent-ssh becomes binary-only)
- `crates/core/agent-ssh/Cargo.toml` — strip to minimal binary deps
- `crates/shared/db/src/migration/m20260331_000002_agent_ssh_migration_history_repair.rs` (**new**)
- `crates/shared/db/src/migration/mod.rs` — remove old migration, add repair migration
- `crates/core/controller-runtime/Cargo.toml` — fix feature chains, remove `dep:uptrakit-agent-ssh`
- `crates/core/controller-runtime/src/migration/mod.rs` — include `service_migrations()`

---

### Task 1: Expand `agent-ssh-runtime/Cargo.toml`

**Files:**

- Modify: `crates/core/agent-ssh-runtime/Cargo.toml`

- [ ] **Step 1: Replace the Cargo.toml**

```toml
[package]
name = "uptrakit-agent-ssh-runtime"
description = "Uptrakit agent-ssh runtime: SSH-based remote-host update orchestration"
edition.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
version = "0.0.2"

[features]
default = []
interactive = [
    "uptrakit-command/interactive",
    "uptrakit-agent-core/interactive",
]
reset-data = []

[dependencies]
async-trait = { workspace = true }
base64 = { workspace = true }
futures-util = { workspace = true }
parking_lot = { workspace = true }
rootcause = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
russh = { workspace = true }
russh-sftp = { workspace = true }
sha2 = { workspace = true }
ssh-key = { workspace = true, features = ["ed25519", "rand_core", "std"] }
strum = { workspace = true }
uptrakit-shared-macros = { workspace = true }
rustls = { workspace = true }
thiserror = { workspace = true }
time = { workspace = true }
tokio = { workspace = true, features = ["macros", "rt-multi-thread", "signal", "fs", "sync", "time"] }
tokio-rustls = { workspace = true }
tracing = { workspace = true }
url = { workspace = true }
uuid = { workspace = true, features = ["v7", "serde"] }
zeroize = { workspace = true }
uptrakit-agent-core = { workspace = true }
uptrakit-audit-log = { workspace = true }
uptrakit-command = { workspace = true }
uptrakit-crypto = { workspace = true, features = ["sea-orm"] }
uptrakit-directories = { workspace = true }
uptrakit-plugin-infrastructure-registry = { workspace = true, features = ["daemon", "agent-infra"] }
uptrakit-service-sdk = { workspace = true, features = ["sensitive-params"] }
uptrakit-shared-types = { workspace = true }
uptrakit-wire = { workspace = true }
sea-orm = { workspace = true, features = ["sqlx-sqlite"] }
sea-orm-migration = { workspace = true, features = ["sqlx-sqlite"] }
sqlx = { workspace = true }

[dev-dependencies]
serde_json = { workspace = true }
tempfile = { workspace = true }
tokio = { workspace = true, features = ["macros", "rt-multi-thread", "time"] }
uptrakit-service-sdk = { workspace = true, features = ["test-support"] }
uptrakit-shared-types = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 2: Compile check** (expect failure — nothing moved yet)

```bash
cargo check -p uptrakit-agent-ssh-runtime 2>&1 | head -20
```

Expected: compile errors about missing modules (not about deps).

- [ ] **Step 3: Commit**

```bash
git add crates/core/agent-ssh-runtime/Cargo.toml
git commit -m "feat(agent-ssh-runtime): expand deps for incoming module migration"
```

---

### Task 2: Move `db/` directory into `agent-ssh-runtime`

**Files:**

- Move: `crates/core/agent-ssh/src/db/` → `crates/core/agent-ssh-runtime/src/db/`

- [ ] **Step 1: Git-move the directory**

```bash
git mv crates/core/agent-ssh/src/db crates/core/agent-ssh-runtime/src/db
```

- [ ] **Step 2: Move the `Migrator::migrations()` plugin-catalog block into `service_migrations()`**

The current `agent-ssh/src/db/migration/mod.rs` has a `build_catalog` block at the bottom of
`migrations()` that appends plugin service migrations — this pattern will not carry over. Keep
the 13 core migrations in the `Migrator` struct exactly as-is; the plugin catalog block is removed
here and handled later via `AgentSshHandler::service_migrations()` + a `TODO` comment.

Replace the `Migrator` impl in `agent-ssh-runtime/src/db/migration/mod.rs`:

```rust
use sea_orm::DatabaseConnection;
use sea_orm_migration::prelude::*;

mod m20260215_000001_initial;
mod m20260222_000002_add_machine_id;
mod m20260224_000003_add_sudo_columns;
mod m20260302_000001_convert_ssh_host_timestamps;
mod m20260302_000002_ensure_machine_id_nullable;
mod m20260306_000001_add_pve_columns;
mod m20260307_000001_add_pve_node_name;
mod m20260307_000002_pending_proxmox_matches;
mod m20260308_000003_ssh_host_uuid_columns;
mod m20260310_000001_data_encryption_keys;
mod m20260313_000001_drop_ssh_host_is_pve_node;
mod m20260322_000001_ssh_hosts_lower_name_index;
mod m20260507_000001_add_routeros_host_config;

pub(crate) struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        // Keep legacy m20260307_000002_pending_proxmox_matches — existing
        // databases already have it recorded in seaql_migrations.
        #[expect(
            clippy::allow_attributes,
            clippy::allow_attributes_without_reason,
            reason = "feature-conditional: `mut` is needed when plugin migrations are appended; \
                      `#[expect]` would fail under feature variants where the binding is never mutated"
        )]
        #[allow(unused_mut)]
        let mut migrations: Vec<Box<dyn MigrationTrait>> = vec![
            Box::new(m20260215_000001_initial::Migration),
            Box::new(m20260222_000002_add_machine_id::Migration),
            Box::new(m20260224_000003_add_sudo_columns::Migration),
            Box::new(m20260302_000001_convert_ssh_host_timestamps::Migration),
            Box::new(m20260302_000002_ensure_machine_id_nullable::Migration),
            Box::new(m20260310_000001_data_encryption_keys::Migration),
            Box::new(m20260306_000001_add_pve_columns::Migration),
            Box::new(m20260307_000001_add_pve_node_name::Migration),
            Box::new(m20260307_000002_pending_proxmox_matches::Migration),
            Box::new(m20260308_000003_ssh_host_uuid_columns::Migration),
            Box::new(m20260313_000001_drop_ssh_host_is_pve_node::Migration),
            Box::new(m20260322_000001_ssh_hosts_lower_name_index::Migration),
            Box::new(m20260507_000001_add_routeros_host_config::Migration),
        ];
        migrations
    }
}

pub(crate) async fn run_migrations(
    db: &DatabaseConnection,
) -> std::result::Result<(), sea_orm::DbErr> {
    Migrator::up(db, None).await
}
```

- [ ] **Step 3: Add `pub mod db;` to `agent-ssh-runtime/src/lib.rs` temporarily**

Add at top of lib.rs:

```rust
pub mod db;
```

- [ ] **Step 4: Compile check**

```bash
cargo check -p uptrakit-agent-ssh-runtime 2>&1 | head -30
```

Expected: compile errors about missing dep types (entity modules may reference `uptrakit-crypto`,
etc.) — fix any missing feature imports by ensuring deps from step 1 are correct. The remaining
errors will be about other missing modules not yet moved.

- [ ] **Step 5: Commit**

```bash
git add crates/core/agent-ssh-runtime/src/db \
        crates/core/agent-ssh-runtime/src/lib.rs \
        crates/core/agent-ssh/src/db
git commit -m "feat(agent-ssh-runtime): move db entities and migrations from agent-ssh"
```

---

### Task 3: Move business logic modules into `agent-ssh-runtime`

**Files:**

- Move: all remaining non-binary source files from `agent-ssh/src/`

- [ ] **Step 1: Git-move all modules**

```bash
git mv crates/core/agent-ssh/src/client.rs       crates/core/agent-ssh-runtime/src/client.rs
git mv crates/core/agent-ssh/src/error.rs        crates/core/agent-ssh-runtime/src/error.rs
git mv crates/core/agent-ssh/src/host_info.rs    crates/core/agent-ssh-runtime/src/host_info.rs
git mv crates/core/agent-ssh/src/host_ops.rs     crates/core/agent-ssh-runtime/src/host_ops.rs
git mv crates/core/agent-ssh/src/operations      crates/core/agent-ssh-runtime/src/operations
git mv crates/core/agent-ssh/src/remote_exec.rs  crates/core/agent-ssh-runtime/src/remote_exec.rs
git mv crates/core/agent-ssh/src/routeros_executor.rs crates/core/agent-ssh-runtime/src/routeros_executor.rs
git mv crates/core/agent-ssh/src/runtime_support.rs  crates/core/agent-ssh-runtime/src/runtime_support.rs
git mv crates/core/agent-ssh/src/ssh_executor.rs crates/core/agent-ssh-runtime/src/ssh_executor.rs
git mv crates/core/agent-ssh/src/ssh_key.rs      crates/core/agent-ssh-runtime/src/ssh_key.rs
git mv crates/core/agent-ssh/src/ssh_pool.rs     crates/core/agent-ssh-runtime/src/ssh_pool.rs
git mv crates/core/agent-ssh/src/ssh_stdio_tunnel.rs crates/core/agent-ssh-runtime/src/ssh_stdio_tunnel.rs
git mv crates/core/agent-ssh/src/ssh_target.rs   crates/core/agent-ssh-runtime/src/ssh_target.rs
git mv crates/core/agent-ssh/src/ssh_transport.rs crates/core/agent-ssh-runtime/src/ssh_transport.rs
git mv crates/core/agent-ssh/src/surface_runtime.rs crates/core/agent-ssh-runtime/src/surface_runtime.rs
git mv crates/core/agent-ssh/src/surface_runtime crates/core/agent-ssh-runtime/src/surface_runtime
```

- [ ] **Step 2: Update `agent-ssh-runtime/src/lib.rs` to declare all moved modules**

Replace lib.rs content:

```rust
pub mod client;
pub mod db;
pub mod error;
pub mod host_ops;
pub mod operations;
pub mod runtime_support;
pub mod ssh_key;
pub mod ssh_pool;
pub mod surface_runtime;

pub(crate) mod host_info;
pub(crate) mod remote_exec;
pub(crate) mod routeros_executor;
pub(crate) mod ssh_executor;
pub(crate) mod ssh_stdio_tunnel;
pub(crate) mod ssh_target;
pub(crate) mod ssh_transport;

use std::collections::HashMap;

pub use uptrakit_service_sdk::ServiceSurfaceProxy;

// Re-exports from the main runtime types (lib.rs already defined these before the refactor)
pub use crate::runtime_types::{
    HOST_RELOAD_INTERVAL, UPDATE_COOLDOWN,
    HostSnapshot, SshAgentEvent, SshAgentIdentity, SshAgentSettings, SshAgentRuntime,
    SshAgentRuntimeConfig, SshAgentRuntimeSupport, SshAgentRuntimeSupportTrait,
    RuntimeSessionState, SshInFlightUpdate, diff_host_snapshots, handle_set_update_freeze,
    ssh_agent_capabilities,
};
```

> **Note:** The original `lib.rs` (before refactor) contains `SshAgentRuntime`,
> `SshAgentRuntimeConfig`, `SshAgentRuntimeSupportTrait`, etc. as inline definitions. After the
> move, those stay in lib.rs as-is. The task is to add `pub mod` declarations for the incoming
> modules on top of what's already there. Do NOT delete the existing SshAgentRuntime etc.
> definitions from lib.rs.

Concretely: add these `pub mod` and `pub(crate) mod` declarations at the top of the existing
lib.rs (before the `use` statements), without touching the struct/trait/fn definitions below:

```rust
pub mod client;
pub mod db;
pub mod error;
pub mod host_ops;
pub mod operations;
pub mod runtime_support;
pub mod ssh_key;
pub mod ssh_pool;
pub mod surface_runtime;

pub(crate) mod host_info;
pub(crate) mod remote_exec;
pub(crate) mod routeros_executor;
pub(crate) mod ssh_executor;
pub(crate) mod ssh_stdio_tunnel;
pub(crate) mod ssh_target;
pub(crate) mod ssh_transport;

pub use uptrakit_service_sdk::ServiceSurfaceProxy;
```

Then delete agent-ssh's old re-export lines (these are now in lib.rs directly):

```rust
// These lines in agent-ssh/src/lib.rs are going away:
// pub use uptrakit_agent_ssh_runtime::{HOST_RELOAD_INTERVAL, UPDATE_COOLDOWN, diff_host_snapshots, handle_set_update_freeze};
```

- [ ] **Step 3: Compile check — agent-ssh-runtime**

```bash
cargo check -p uptrakit-agent-ssh-runtime 2>&1 | head -40
```

Fix any import paths that reference `uptrakit_agent_ssh::` — these will have become stale
crate-internal paths. For example, modules that did `use crate::db::entity::ssh_host` still work
because `db` is now a `pub mod` in the same crate. For modules that imported from
`uptrakit_agent_ssh_runtime` (the old, thin runtime crate), those references now resolve to
`crate::` since everything is in one crate.

The most common issue: `surface_runtime.rs` and `runtime_support.rs` previously imported `uptrakit_agent_ssh::db::entity::...` — change these to `crate::db::entity::...`.

- [ ] **Step 4: Compile check — agent-ssh binary**

```bash
cargo check -p uptrakit-agent-ssh 2>&1 | head -40
```

Expected: agent-ssh's `lib.rs` still references `pub mod client; pub mod db; ...` which no longer exist in agent-ssh. The next task fixes this.

- [ ] **Step 5: Commit**

```bash
git add crates/core/agent-ssh-runtime/src \
        crates/core/agent-ssh/src
git commit -m "feat(agent-ssh-runtime): move SSH logic modules from agent-ssh"
```

---

### Task 4: Move crypto helpers and replace `agent-ssh/src/lib.rs`

**Files:**

- Modify: `crates/core/agent-ssh-runtime/src/lib.rs` — add crypto helper fns
- Modify: `crates/core/agent-ssh/src/lib.rs` — replace with thin re-exports (will be deleted in Task 6)

The crypto helpers (`register_ssh_column_aad`, `init_ssh_data_key_ring`, `reencrypt_ssh_to_v3`) live in `agent-ssh/src/lib.rs` today. Move them to `agent-ssh-runtime/src/lib.rs`.

- [ ] **Step 1: Copy the three crypto functions from `agent-ssh/src/lib.rs` into `agent-ssh-runtime/src/lib.rs`**

Append to `agent-ssh-runtime/src/lib.rs`:

```rust
pub const AAD_SSH_PRIVATE_KEY: &str = "uptrakit:ssh_hosts:private_key";

pub fn register_ssh_column_aad() {
    if !uptrakit_crypto::master_key_available() {
        return;
    }
    use uptrakit_crypto::ColumnAadEntry;
    let entries: &[ColumnAadEntry] = &[ColumnAadEntry {
        table: "ssh_hosts",
        column: "private_key",
        aad: AAD_SSH_PRIVATE_KEY,
    }];
    if let Err(e) = uptrakit_crypto::register_column_aad(entries) {
        tracing::warn!(error = %e, "column AAD registry already initialized (harmless)");
    }
}

pub async fn init_ssh_data_key_ring(db: &sea_orm::DatabaseConnection) {
    use sea_orm::{ActiveModelTrait, EntityTrait};

    if !uptrakit_crypto::master_key_available() {
        return;
    }

    let kek_fp = match uptrakit_crypto::master_key_fingerprint() {
        Ok(fp) => fp,
        Err(e) => {
            tracing::error!(error = %e, "failed to compute KEK fingerprint");
            return;
        }
    };

    let rows = match db::entity::data_encryption_key::Entity::find().all(db).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "failed to query data_encryption_keys");
            return;
        }
    };

    if rows.is_empty() {
        let dek = match uptrakit_crypto::generate_data_key() {
            Ok(d) => d,
            Err(e) => { tracing::error!(error = %e, "failed to generate initial DEK"); return; }
        };
        let wrapped = match uptrakit_crypto::wrap_data_key(&dek) {
            Ok(w) => w,
            Err(e) => { tracing::error!(error = %e, "failed to wrap initial DEK"); return; }
        };
        let am = db::entity::data_encryption_key::ActiveModel {
            id: sea_orm::Set(uuid::Uuid::now_v7()),
            key_id: sea_orm::Set(dek.key_id.clone()),
            wrapped_key: sea_orm::Set(wrapped),
            kek_fingerprint: sea_orm::Set(kek_fp.clone()),
            status: sea_orm::Set("active".to_string()),
            created_at: sea_orm::Set(time::OffsetDateTime::now_utc()),
            retired_at: sea_orm::Set(None),
        };
        if let Err(e) = am.insert(db).await {
            tracing::debug!(error = %e, "initial DEK insert failed (may be race), will load existing");
        } else {
            tracing::info!(key_id = %dek.key_id, "generated initial data encryption key");
        }
        let rows = match db::entity::data_encryption_key::Entity::find().all(db).await {
            Ok(r) => r,
            Err(e) => { tracing::error!(error = %e, "failed to re-read data_encryption_keys"); return; }
        };
        build_and_init_ssh_ring(&rows, &kek_fp);
        return;
    }
    build_and_init_ssh_ring(&rows, &kek_fp);
}

fn build_and_init_ssh_ring(rows: &[db::entity::data_encryption_key::Model], kek_fp: &str) {
    let mut keys = std::collections::HashMap::new();
    let mut active_key_id: Option<String> = None;
    for row in rows {
        if row.kek_fingerprint != kek_fp {
            tracing::error!(
                key_id = %row.key_id,
                stored_fp = %row.kek_fingerprint,
                current_fp = %kek_fp,
                "DEK was wrapped with a different KEK — master key mismatch"
            );
            return;
        }
        let dek = match uptrakit_crypto::unwrap_data_key(&row.wrapped_key, &row.key_id) {
            Ok(d) => d,
            Err(e) => { tracing::error!(key_id = %row.key_id, error = %e, "failed to unwrap DEK"); return; }
        };
        keys.insert(dek.key_id.clone(), dek.key);
        if row.status == "active" {
            active_key_id = Some(row.key_id.clone());
        }
    }
    let active = match active_key_id {
        Some(id) => id,
        None => { tracing::error!("no active DEK found in data_encryption_keys table"); return; }
    };
    let ring = match uptrakit_crypto::DataKeyRing::new(keys, active.clone()) {
        Ok(r) => r,
        Err(e) => { tracing::error!(error = %e, "failed to construct data key ring"); return; }
    };
    if let Err(e) = uptrakit_crypto::init_data_key_ring(ring) {
        tracing::warn!(error = %e, "data key ring already initialized (harmless)");
    } else {
        tracing::info!(active_key_id = %active, "data key ring initialized");
    }
}

pub async fn reencrypt_ssh_to_v3(db: &sea_orm::DatabaseConnection) {
    use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel};
    use uptrakit_crypto::EncryptedString;

    if !uptrakit_crypto::master_key_available() {
        return;
    }
    let rows = match db::entity::ssh_host::Entity::find().all(db).await {
        Ok(r) => r,
        Err(e) => { tracing::error!(error = %e, "failed to query ssh_hosts for v3 upgrade"); return; }
    };
    let mut count = 0u64;
    for row in rows {
        if !row.private_key.needs_v3_upgrade() {
            continue;
        }
        let plaintext = row.private_key.expose_secret().to_string();
        let id = row.id;
        match EncryptedString::new(plaintext, AAD_SSH_PRIVATE_KEY) {
            Ok(encrypted) => {
                let mut am = row.into_active_model();
                am.private_key = sea_orm::Set(encrypted);
                if let Err(e) = am.update(db).await {
                    tracing::error!(id = %id, error = %e, "v3 upgrade failed: ssh_hosts.private_key");
                } else {
                    count += 1;
                }
            }
            Err(e) => tracing::error!(id = %id, error = %e, "v3 encrypt failed: ssh_hosts.private_key"),
        }
    }
    if count > 0 {
        tracing::info!(table = "ssh_hosts", column = "private_key", count, "upgraded to ENC:v3");
    }
}
```

- [ ] **Step 2: Add `rotate_ssh_master_key` function to `agent-ssh-runtime/src/lib.rs`**

Find `rotate_ssh_master_key` in `agent-ssh/src/main.rs` (it's a private fn at the bottom). Copy it to `agent-ssh-runtime/src/lib.rs` and make it `pub`:

```bash
grep -n "async fn rotate_ssh_master_key" crates/core/agent-ssh/src/main.rs
```

Copy the full function body verbatim, change `fn` to `pub async fn`.

- [ ] **Step 3: Compile check**

```bash
cargo check -p uptrakit-agent-ssh-runtime 2>&1 | head -30
```

Expected: should now compile cleanly (or with only minor unresolved items).

- [ ] **Step 4: Commit**

```bash
git add crates/core/agent-ssh-runtime/src/lib.rs
git commit -m "feat(agent-ssh-runtime): add crypto helpers and key rotation fn"
```

---

### Task 5: Create `AgentSshHandler` in `agent-ssh-runtime`

**Files:**

- Create: `crates/core/agent-ssh-runtime/src/handler.rs`
- Modify: `crates/core/agent-ssh-runtime/src/lib.rs` — add `pub mod handler;` and `service_migrations()`

- [ ] **Step 1: Write `crates/core/agent-ssh-runtime/src/handler.rs`**

```rust
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine as _;
use rootcause::prelude::*;
use sea_orm::DatabaseConnection;
use sea_orm_migration::MigrationTrait;
use uptrakit_audit_log::RuntimeAuditEmitter;
use uptrakit_service_sdk::{
    LoopError, LoopOutcome, LoopResult, ServiceHandler, ServiceIdentityState, ShutdownCause,
    default_resolve_shutdown,
};
use uptrakit_wire::{Capability, ControllerMessage, ServiceTransport};

use crate::runtime_support::AgentSshRuntimeSupport;
use crate::ssh_pool::SshConnectionPool;
use crate::{
    SshAgentEvent, SshAgentIdentity, SshAgentRuntime, SshAgentRuntimeConfig, SshAgentSettings,
    ServiceSurfaceProxy, ssh_agent_capabilities,
};

/// Internal mode discriminator. `#[non_exhaustive]` NOT applied — this is
/// an internal type, not a wire type. "Standalone" is an avoided term per
/// the domain glossary; `Binary` names the mode where the service runs as
/// its own process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSshMode {
    /// Service runs as its own binary process with a local SQLite DB.
    /// Performs surface self-registration.
    Binary,
    /// Service runs embedded inside the Controller process, using the shared DB.
    /// Skips surface self-registration (controller registers on its behalf).
    Embedded,
}

/// ECIES keypair injected at construction time. Named struct (not a raw tuple)
/// so the controller-side generator and the constructor share a stable typed boundary.
///
/// Invariant: `private_key_der` is always `Some` when constructed from
/// `generate_ecies_keypair()` in controller-runtime. The `Option` reflects
/// the return type of the generator; callers must not construct with `None`.
pub struct EciesKeypair {
    pub private_key_der: Option<Vec<u8>>,
    pub encryption_public_key: String,
}

pub struct AgentSshHandler {
    runtime: SshAgentRuntime<AgentSshRuntimeSupport>,
    ecies_keypair: Option<EciesKeypair>,
}

impl AgentSshHandler {
    pub fn new(
        db: DatabaseConnection,
        state_dir: PathBuf,
        mode: AgentSshMode,
        ecies_keypair: Option<EciesKeypair>,
    ) -> Self {
        let catalog_config = uptrakit_plugin_infrastructure_registry::CatalogConfig::default();
        #[expect(
            clippy::expect_used,
            reason = "infallible at startup: catalog construction failures are static \
                      configuration errors that must abort process initialization"
        )]
        let catalog = uptrakit_plugin_infrastructure_registry::build_catalog(&catalog_config)
            .expect("plugin catalog must build successfully");
        let infra_bundles = Arc::new(catalog.create_infra_bundles(&catalog_config));
        let surface_proxy = Arc::new(ServiceSurfaceProxy::new());
        let is_standalone = matches!(mode, AgentSshMode::Binary);
        let support = AgentSshRuntimeSupport::new(
            db,
            state_dir.clone(),
            SshConnectionPool::new(),
            surface_proxy,
            infra_bundles,
            is_standalone,
        );
        let runtime = SshAgentRuntime::new(SshAgentRuntimeConfig::with_audit_emitter(
            support,
            state_dir.join("update-freeze"),
            RuntimeAuditEmitter::new(),
        ));
        Self { runtime, ecies_keypair }
    }
}

#[async_trait::async_trait]
impl ServiceHandler for AgentSshHandler {
    const DIR_NAME: &'static str = "agent-ssh";
    const SERVICE_LABEL: &'static str = "uptrakit-agent-ssh service";
    /// Hardcoded string literal — NOT `env!("CARGO_PKG_NAME")`.
    /// Enrollment and controller builtins match by this exact string.
    const SERVICE_APP_NAME: &'static str = "uptrakit-agent-ssh";

    type ServiceEvent = SshAgentEvent;

    async fn on_connected(
        &mut self,
        conn: &mut dyn ServiceTransport,
        identity: &ServiceIdentityState,
    ) -> LoopResult<()> {
        let (private_key_der, encryption_public_key) = match &self.ecies_keypair {
            Some(kp) => (kp.private_key_der.clone(), Some(kp.encryption_public_key.clone())),
            None => {
                let enc_pub = identity.public_key_raw().map(|bytes| {
                    base64::engine::general_purpose::STANDARD.encode(bytes)
                });
                (identity.private_key_pkcs8_der(), enc_pub)
            }
        };
        self.runtime
            .on_connected(
                conn,
                SshAgentIdentity {
                    service_id: identity.service_id(),
                    private_key_der,
                    encryption_public_key,
                },
            )
            .await
            .map_err(|error| report!(LoopError::Other(error.to_string())))
    }

    async fn on_message(
        &mut self,
        msg: ControllerMessage,
        conn: &mut dyn ServiceTransport,
    ) -> LoopResult<Option<LoopOutcome>> {
        self.runtime.handle_controller_message(msg, conn).await;
        Ok(None)
    }

    async fn on_settings(
        &mut self,
        settings: &uptrakit_wire::ServiceSettingsPayload,
        conn: &mut dyn ServiceTransport,
        agreed_capabilities: &BTreeSet<Capability>,
    ) {
        if let Err(error) = self
            .runtime
            .apply_settings(
                SshAgentSettings {
                    tenant_id: settings.tenant_id,
                    ui_surfaces_enabled: agreed_capabilities.contains(&Capability::UiSurfaces),
                    persist_tenant_id: matches!(
                        // In binary mode the tenant_id is persisted to disk for reconnects;
                        // in embedded mode the controller re-injects it on every settings push.
                        self.ecies_keypair,
                        None  // None = binary (ecies_keypair is Some only in embedded)
                    ),
                },
                conn,
            )
            .await
        {
            tracing::warn!(error = %error, "failed to apply SSH agent service settings");
        }
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        ssh_agent_capabilities()
    }

    async fn poll_service_event(&mut self) -> Self::ServiceEvent {
        self.runtime.poll_event().await
    }

    async fn on_service_event(
        &mut self,
        event: Self::ServiceEvent,
        conn: &mut dyn ServiceTransport,
    ) -> LoopResult<Option<LoopOutcome>> {
        Ok(self.runtime.handle_event(event, conn).await)
    }

    fn on_surface_action_response(
        &mut self,
        response: uptrakit_wire::surfaces::SurfaceActionResponse,
    ) {
        self.runtime.handle_surface_action_response(response);
    }

    async fn on_surface_action_request(
        &mut self,
        request: uptrakit_wire::surfaces::SurfaceActionRequest,
        conn: &mut dyn ServiceTransport,
    ) -> LoopResult<()> {
        self.runtime
            .handle_controller_message(
                ControllerMessage::SurfaceActionRequest(request),
                conn,
            )
            .await;
        Ok(())
    }

    async fn on_shutdown(
        &mut self,
        conn: &mut dyn ServiceTransport,
        cause: ShutdownCause,
        shutdown_timeout: std::time::Duration,
    ) -> LoopOutcome {
        let (disconnect_reason, outcome) = default_resolve_shutdown(cause);
        self.runtime
            .shutdown(conn, shutdown_timeout, disconnect_reason, outcome)
            .await
    }
}
```

- [ ] **Step 2: Add `pub mod handler;` and `service_migrations()` to `agent-ssh-runtime/src/lib.rs`**

Add at the top (with other mod declarations):

```rust
pub mod handler;
```

Add after the crypto helpers, before the tests block:

```rust
/// Transitional free function returning the 13 agent-ssh schema migrations.
///
/// Called by `controller-runtime` until WS5 replaces this with
/// `AgentSshHandler::service_migrations()` on the `ServiceHandler` trait.
pub fn service_migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
    crate::db::migration::Migrator::migrations()
}
```

Add public re-exports for handler types:

```rust
pub use handler::{AgentSshHandler, AgentSshMode, EciesKeypair};
```

- [ ] **Step 3: Compile check**

```bash
cargo check -p uptrakit-agent-ssh-runtime --all-features 2>&1 | head -40
```

Fix any remaining import issues. Common fix: `persist_tenant_id` logic — the `None` check for
`ecies_keypair` in `on_settings` may need adjustment. In binary mode `ecies_keypair` is `None`
(no pre-generated key passed); in embedded mode it is `Some`. So
`persist_tenant_id: self.ecies_keypair.is_none()` is the correct idiom.

- [ ] **Step 4: Run agent-ssh-runtime tests**

```bash
cargo test -p uptrakit-agent-ssh-runtime --all-features 2>&1 | tail -20
```

Expected: all existing runtime tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/core/agent-ssh-runtime/src/handler.rs \
        crates/core/agent-ssh-runtime/src/lib.rs
git commit -m "feat(agent-ssh-runtime): add AgentSshHandler, AgentSshMode, EciesKeypair, service_migrations()"
```

---

### Task 6: Thin out `agent-ssh` binary

**Files:**

- Delete: `crates/core/agent-ssh/src/lib.rs`
- Modify: `crates/core/agent-ssh/src/main.rs`
- Modify: `crates/core/agent-ssh/Cargo.toml`

- [ ] **Step 1: Delete `agent-ssh/src/lib.rs`**

```bash
git rm crates/core/agent-ssh/src/lib.rs
```

- [ ] **Step 2: Rewrite `agent-ssh/src/main.rs` to use `AgentSshHandler`**

The new main.rs imports from `uptrakit_agent_ssh_runtime` for everything except clap and the raw process setup:

```rust
mod cli;
mod host_cli;
mod commands;

use std::path::PathBuf;

use clap::Parser;
use rootcause::prelude::*;

use uptrakit_agent_ssh_runtime::{
    AgentSshHandler, AgentSshMode, db,
    init_ssh_data_key_ring, reencrypt_ssh_to_v3, register_ssh_column_aad, rotate_ssh_master_key,
};
use uptrakit_service_sdk::run_lifecycle_and_handle_errors;

use cli::{Args, Commands};

#[derive(Debug, thiserror::Error)]
enum InitError {
    #[error("{0}")]
    Directory(String),
    #[error("{0}")]
    MasterKey(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Hex(String),
}

type InitResult<T> = std::result::Result<T, rootcause::Report<InitError>>;

#[tokio::main]
async fn main() {
    let args = Args::parse();
    if args.common.version {
        uptrakit_service_sdk::print_build_info(
            "uptrakit-agent-ssh",
            env!("CARGO_PKG_VERSION"),
            option_env!("UPTRAKIT_BUILD_ENABLED_FEATURES"),
        );
        return;
    }

    if let Some(Commands::Host { command }) = args.command {
        uptrakit_service_sdk::TracingBuilder::new()
            .verbosity(args.common.verbose)
            .init();

        if let Err(error) = init_master_key(&args.master_key_file, args.allow_plaintext_secrets) {
            eprintln!("error: {error}");
            std::process::exit(1);
        }
        register_ssh_column_aad();

        let state_dir = match resolve_state_dir_from_common(&args.common).await {
            Ok(dir) => dir,
            Err(error) => {
                eprintln!("error: {error}");
                std::process::exit(1);
            }
        };

        match db::init_db(&state_dir).await {
            Ok(host_db) => {
                init_ssh_data_key_ring(&host_db).await;
                #[expect(
                    clippy::let_underscore_must_use,
                    reason = "best-effort cleanup on subcommand exit; failures are non-actionable"
                )]
                let _ = host_db.close().await;
            }
            Err(error) => {
                tracing::error!(error = %error, "could not init DEK ring for host subcommand");
            }
        }

        if let Err(error) = host_cli::run(&state_dir, command).await {
            eprintln!("error: {error}");
            std::process::exit(1);
        }
        return;
    }

    if args.common.url.is_none() {
        eprintln!("error: --url is required for daemon mode");
        std::process::exit(1);
    }

    uptrakit_service_sdk::TracingBuilder::new()
        .verbosity(args.common.verbose)
        .init();
    uptrakit_service_sdk::init_crypto();

    if let Err(error) = init_master_key(&args.master_key_file, args.allow_plaintext_secrets) {
        tracing::error!("{error}");
        std::process::exit(1);
    }
    register_ssh_column_aad();

    let state_dir = match resolve_state_dir_from_common(&args.common).await {
        Ok(dir) => dir,
        Err(error) => {
            tracing::error!("{error}");
            std::process::exit(1);
        }
    };

    let local_db = match db::init_db(&state_dir).await {
        Ok(db) => db,
        Err(error) => {
            tracing::error!("failed to initialize local database: {error}");
            std::process::exit(1);
        }
    };

    init_ssh_data_key_ring(&local_db).await;
    reencrypt_ssh_to_v3(&local_db).await;

    if let Some(ref new_key_path) = args.rotate_master_key_file {
        rotate_ssh_master_key(&local_db, new_key_path).await;
    }

    let mut handler = AgentSshHandler::new(local_db, state_dir, AgentSshMode::Binary, None);

    run_lifecycle_and_handle_errors("uptrakit-agent-ssh", &args.common, &mut handler).await;
}

async fn resolve_state_dir_from_common(
    common: &uptrakit_service_sdk::cli::CommonServiceArgs,
) -> InitResult<PathBuf> {
    let dirs = common.resolve_dirs("agent-ssh").map_err(|error| {
        report!(InitError::Directory(format!(
            "failed to resolve directories: {error}"
        )))
    })?;
    dirs.ensure_state_dir().await.map_err(|error| {
        report!(InitError::Directory(format!(
            "failed to ensure state directory: {error}"
        )))
    })?;
    Ok(dirs.state_dir().to_path_buf())
}

fn init_master_key(
    master_key_file: &Option<PathBuf>,
    allow_plaintext: bool,
) -> InitResult<()> {
    // Copy verbatim from old main.rs — this fn did not change.
    // (grep for `fn init_master_key` in the git history if needed)
    todo!("copy from old main.rs")
}
```

> **Note:** `init_master_key` and the subcommand parsing infra in `cli.rs`/`host_cli.rs`/`commands/`
> stay as-is. Copy `init_master_key` verbatim from the old main.rs. The only structural changes
> are: remove `SshAgentHandler` struct, remove its `impl ServiceHandler`, replace the handler
> construction block with `AgentSshHandler::new(...)`.

- [ ] **Step 3: Update `agent-ssh/Cargo.toml` — strip to binary-only deps**

```toml
[package]
name = "uptrakit-agent-ssh"
description = "Uptrakit SSH agent for remote host update management"
edition.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
version = "0.0.2"

[features]
default = ["zeroconf", "interactive", "reset-data"]
zeroconf = ["uptrakit-service-sdk/zeroconf"]
interactive = ["uptrakit-agent-ssh-runtime/interactive"]
reset-data  = ["uptrakit-agent-ssh-runtime/reset-data"]

[dependencies]
clap = { workspace = true }
rootcause = { workspace = true }
thiserror = { workspace = true }
time = { workspace = true }
tokio = { workspace = true, features = ["macros", "rt-multi-thread", "signal"] }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
uptrakit-agent-ssh-runtime = { workspace = true }
uptrakit-crypto = { workspace = true }
uptrakit-directories = { workspace = true }
uptrakit-service-sdk = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
tokio = { workspace = true, features = ["test-util"] }

[build-dependencies]
uptrakit-build-info = { workspace = true }

[lints]
workspace = true

[package.metadata.binstall]
pkg-url = "{ repo }/releases/download/uptrakit-agent-ssh-v{ version }/uptrakit-agent-ssh-{ version }-{ target }.tar.gz"
bin-dir = "{ bin }{ binary-ext }"
pkg-fmt = "tgz"
```

- [ ] **Step 4: Update `host_cli.rs` imports**

`host_cli.rs` previously imported from `crate::` (the lib). Now it must import from `uptrakit_agent_ssh_runtime::`:

```bash
grep -n "use crate::" crates/core/agent-ssh/src/host_cli.rs | head -20
```

Replace `use crate::` references with `use uptrakit_agent_ssh_runtime::` for all non-local items (db, operations, error, etc.).

Do the same for any file in `commands/` that imported from `crate::`.

- [ ] **Step 5: Compile check — both crates**

```bash
cargo check -p uptrakit-agent-ssh --all-features 2>&1 | head -40
cargo check -p uptrakit-agent-ssh-runtime --all-features 2>&1 | head -20
```

Fix remaining import errors.

- [ ] **Step 6: Run tests**

```bash
cargo test -p uptrakit-agent-ssh --all-features 2>&1 | tail -20
cargo test -p uptrakit-agent-ssh-runtime --all-features 2>&1 | tail -20
```

- [ ] **Step 7: Commit**

```bash
git add crates/core/agent-ssh/src \
        crates/core/agent-ssh/Cargo.toml
git commit -m "refactor(agent-ssh): thin to binary shell, delegate all logic to runtime"
```

---

### Task 7: Create repair migration + update `shared-db`

**Files:**

- Create: `crates/shared/db/src/migration/m20260331_000002_agent_ssh_migration_history_repair.rs`
- Delete: `crates/shared/db/src/migration/m20260331_000001_ssh_agent_tables.rs`
- Modify: `crates/shared/db/src/migration/mod.rs`

- [ ] **Step 1: Write the repair migration**

Create `crates/shared/db/src/migration/m20260331_000002_agent_ssh_migration_history_repair.rs`:

```rust
use sea_orm_migration::prelude::*;

/// Repair migration for controller deployments that previously ran the
/// monolithic `m20260331_000001_ssh_agent_tables` migration.
///
/// That migration created all SSH agent tables in one shot. The new schema
/// ownership model puts each migration in `agent-ssh-runtime`, contributed
/// via `service_migrations()`. SeaORM would try to re-run the 13 standalone
/// migrations unless their names are already present in `seaql_migrations`.
///
/// This migration:
/// 1. Detects whether the old monolithic row exists.
/// 2. If so, inserts the 13 individual migration names and deletes the old row.
/// 3. If not, no-ops (fresh install or standalone agent-ssh DB).
///
/// Frozen-list constraint: the INSERT list reflects the 13 migrations
/// that existed when this repair was written. No new agent-ssh migrations
/// may land between writing this repair and shipping the release. See ADR-0005.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        // Check whether the old monolithic row exists.
        let exists = conn
            .query_one_raw(Statement::from_string(
                manager.get_database_backend(),
                "SELECT 1 FROM seaql_migrations \
                 WHERE version = 'm20260331_000001_ssh_agent_tables' LIMIT 1",
            ))
            .await?
            .is_some();

        if !exists {
            return Ok(());
        }

        // Note on transaction safety: SeaORM's migration runner wraps up() in its own
        // outer transaction before calling this method. A nested BEGIN IMMEDIATE here
        // would open a SAVEPOINT (always deferred in SQLite), not a true BEGIN IMMEDIATE.
        // The actual safety guarantee is ON CONFLICT DO NOTHING: duplicates are silently
        // skipped, and the DELETE is a single-row keyed write. No extra locking needed.
        conn.execute_unprepared(
            "INSERT INTO seaql_migrations (version, applied_at) VALUES
               ('m20260215_000001_initial',                     unixepoch()),
               ('m20260222_000002_add_machine_id',              unixepoch()),
               ('m20260224_000003_add_sudo_columns',            unixepoch()),
               ('m20260302_000001_convert_ssh_host_timestamps', unixepoch()),
               ('m20260302_000002_ensure_machine_id_nullable',  unixepoch()),
               ('m20260310_000001_data_encryption_keys',        unixepoch()),
               ('m20260306_000001_add_pve_columns',             unixepoch()),
               ('m20260307_000001_add_pve_node_name',           unixepoch()),
               ('m20260307_000002_pending_proxmox_matches',     unixepoch()),
               ('m20260308_000003_ssh_host_uuid_columns',       unixepoch()),
               ('m20260313_000001_drop_ssh_host_is_pve_node',   unixepoch()),
               ('m20260322_000001_ssh_hosts_lower_name_index',  unixepoch()),
               ('m20260507_000001_add_routeros_host_config',    unixepoch())
             ON CONFLICT DO NOTHING",
        )
        .await?;

        conn.execute_unprepared(
            "DELETE FROM seaql_migrations \
             WHERE version = 'm20260331_000001_ssh_agent_tables'",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Not reversible — data-only migration.
        Ok(())
    }
}
```

- [ ] **Step 2: Write the repair migration test**

In the test module at the bottom of `crates/shared/db/src/migration/mod.rs`, add:

```rust
#[cfg(test)]
mod repair_migration_tests {
    use sea_orm::{ConnectionTrait, Database, DatabaseConnection};
    use sea_orm_migration::MigratorTrait;

    use super::*;

    async fn open_test_db() -> DatabaseConnection {
        Database::connect("sqlite::memory:").await.expect("db")
    }

    #[tokio::test]
    async fn repair_migration_converts_monolithic_row_to_individual_rows() {
        let db = open_test_db().await;

        // Bootstrap the seaql_migrations table (SeaORM creates it on first up()).
        // We run Migrator::up up to but not including the old monolithic migration,
        // then manually insert it to simulate a pre-existing deployment.
        // Simpler: just create the table and insert the old row directly.
        db.execute_unprepared(
            "CREATE TABLE IF NOT EXISTS seaql_migrations \
             (version TEXT NOT NULL PRIMARY KEY, applied_at INTEGER NOT NULL)",
        )
        .await
        .expect("create table");

        db.execute_unprepared(
            "INSERT INTO seaql_migrations (version, applied_at) \
             VALUES ('m20260331_000001_ssh_agent_tables', 1711929600)",
        )
        .await
        .expect("insert old row");

        // Run the repair migration.
        let migration = m20260331_000002_agent_ssh_migration_history_repair::Migration;
        let schema_manager = sea_orm_migration::SchemaManager::new(&db);
        migration.up(&schema_manager).await.expect("repair up");

        // Assert: old row is gone.
        let old_row = db
            .query_one_raw(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT 1 FROM seaql_migrations \
                 WHERE version = 'm20260331_000001_ssh_agent_tables'",
            ))
            .await
            .expect("query");
        assert!(old_row.is_none(), "old monolithic row must be deleted");

        // Assert: all 13 individual rows are present.
        let rows: Vec<sea_orm::QueryResult> = db
            .query_all_raw(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT version FROM seaql_migrations ORDER BY version",
            ))
            .await
            .expect("query all");
        assert_eq!(rows.len(), 13, "must have exactly 13 individual rows");
    }

    #[tokio::test]
    async fn repair_migration_is_noop_on_fresh_install() {
        let db = open_test_db().await;
        db.execute_unprepared(
            "CREATE TABLE IF NOT EXISTS seaql_migrations \
             (version TEXT NOT NULL PRIMARY KEY, applied_at INTEGER NOT NULL)",
        )
        .await
        .expect("create table");
        // No rows inserted — simulates fresh install.

        let migration = m20260331_000002_agent_ssh_migration_history_repair::Migration;
        let schema_manager = sea_orm_migration::SchemaManager::new(&db);
        migration.up(&schema_manager).await.expect("repair up no-op");

        let rows: Vec<sea_orm::QueryResult> = db
            .query_all_raw(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT 1 FROM seaql_migrations",
            ))
            .await
            .expect("query");
        assert!(rows.is_empty(), "no-op on fresh install must leave table empty");
    }
}
```

- [ ] **Step 3: Delete old monolithic migration file**

```bash
git rm crates/shared/db/src/migration/m20260331_000001_ssh_agent_tables.rs
```

- [ ] **Step 4: Update `shared-db/src/migration/mod.rs`**

In the module declarations section, replace:

```rust
mod m20260331_000001_ssh_agent_tables;
```

with:

```rust
mod m20260331_000002_agent_ssh_migration_history_repair;
```

In `Migrator::migrations()`, replace:

```rust
Box::new(m20260331_000001_ssh_agent_tables::Migration),
```

with:

```rust
Box::new(m20260331_000002_agent_ssh_migration_history_repair::Migration),
```

- [ ] **Step 5: Compile check**

```bash
cargo check -p uptrakit-shared-db --all-features 2>&1 | head -20
```

- [ ] **Step 6: Run repair migration tests**

```bash
cargo test -p uptrakit-shared-db migration::repair_migration_tests 2>&1
```

Expected: both tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/shared/db/src/migration/
git commit -m "feat(shared-db): replace monolithic ssh_agent_tables migration with repair migration"
```

---

### Task 8: Wire `service_migrations()` into `controller-runtime`

**Files:**

- Modify: `crates/core/controller-runtime/Cargo.toml`
- Modify: `crates/core/controller-runtime/src/migration/mod.rs`

- [ ] **Step 1: Update `controller-runtime/Cargo.toml` feature chains and deps**

In `[features]`:

```toml
embedded-ssh-agent = ["dep:uptrakit-agent-core", "dep:uptrakit-agent-ssh-runtime", "dep:base64"]
interactive = ["uptrakit-web-api/interactive", "uptrakit-agent-runtime?/interactive", "uptrakit-agent-ssh-runtime?/interactive"]
reset-data  = ["uptrakit-web-api/reset-data", "uptrakit-agent-ssh-runtime?/reset-data"]
```

In `[dependencies]`, remove:

```toml
uptrakit-agent-ssh = { workspace = true, optional = true }
```

The `uptrakit-agent-ssh-runtime` dep line already exists — no change needed there.

- [ ] **Step 2: Update `controller-runtime/src/migration/mod.rs`**

Replace:

```rust
pub(crate) async fn run_migrations(db: &DatabaseConnection) -> Result<()> {
    let plugin_migrations = uptrakit_plugin_infrastructure_registry::all_descriptors()
        .into_iter()
        .filter_map(|d| d.migrations)
        .flat_map(|f| f())
        .collect();
    uptrakit_shared_db::migration::run_migrations_with_plugins(db, plugin_migrations)
        .await
        .context_to::<DbError>()
        .context(DbError::Migration(
            "running database migrations".to_string(),
        ))
}
```

With:

```rust
pub(crate) async fn run_migrations(db: &DatabaseConnection) -> Result<()> {
    let mut plugin_migrations: Vec<Box<dyn sea_orm_migration::MigrationTrait>> =
        uptrakit_plugin_infrastructure_registry::all_descriptors()
            .into_iter()
            .filter_map(|d| d.migrations)
            .flat_map(|f| f())
            .collect();

    #[cfg(feature = "embedded-ssh-agent")]
    plugin_migrations.extend(uptrakit_agent_ssh_runtime::service_migrations());

    uptrakit_shared_db::migration::run_migrations_with_plugins(db, plugin_migrations)
        .await
        .context_to::<DbError>()
        .context(DbError::Migration(
            "running database migrations".to_string(),
        ))
}
```

Also add the import at the top of the file:

```rust
use sea_orm_migration::MigrationTrait;
```

- [ ] **Step 3: Compile check**

```bash
cargo check -p uptrakit-controller-runtime --no-default-features --features db-sqlite 2>&1 | head -20
cargo check -p uptrakit-controller-runtime --all-features 2>&1 | head -20
```

- [ ] **Step 4: Run all controller-runtime tests**

```bash
cargo test -p uptrakit-controller-runtime --all-features 2>&1 | tail -20
```

- [ ] **Step 5: Commit**

```bash
git add crates/core/controller-runtime/Cargo.toml \
        crates/core/controller-runtime/src/migration/mod.rs
git commit -m "feat(controller-runtime): wire agent-ssh service_migrations into migration runner"
```

---

### Task 9: Final quality gates

- [ ] **Step 1: Format**

```bash
cargo fmt --all
```

- [ ] **Step 2: Clippy — no-default-features**

```bash
cargo clippy --all-targets --no-default-features --features db-sqlite 2>&1 | grep "^error" | head -20
```

Fix all errors.

- [ ] **Step 3: Clippy — all-features**

```bash
cargo clippy --all-targets --all-features 2>&1 | grep "^error" | head -20
```

Fix all errors.

- [ ] **Step 4: Full test suite**

```bash
cargo test --all-features 2>&1 | tail -30
```

- [ ] **Step 5: cargo deny**

```bash
cargo deny check
```

- [ ] **Step 6: Markdownlint** (no .md files changed — skip if none in this PR)

- [ ] **Step 7: Verify controller no longer imports `uptrakit-agent-ssh`**

```bash
grep -r "uptrakit_agent_ssh::" crates/core/controller-runtime/src/
```

Expected: no output.

- [ ] **Step 8: Final commit**

```bash
git add -u
git commit -m "chore: fmt and clippy fixes for agent-ssh refactor"
```
