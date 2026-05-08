# Controller Unified Embedded Path Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the unified `run_embedded_service<H>` path through the controller for all embedded
services (SSH agent and MQTT). Add `service_migrations()` to the `ServiceHandler` trait. Delete the
hand-rolled embedded loops. Write ADR-0005 and the coding-standards section.

**Architecture:** `ServiceHandler` gains a defaulted static `service_migrations()` method behind a
`service-migrations` feature gate in service-sdk. `AgentSshHandler` overrides it; the transitional
free function is removed. The controller calls `AgentSshHandler::service_migrations()` directly.
`builtins.rs` constructs `AgentSshHandler` / `MqttHandler` and calls `run_embedded_service` instead
of the hand-rolled loops.

**Prerequisites (must be complete):**

- Plan 1 (`2026-05-07-agent-ssh-refactor.md`) — `AgentSshHandler`, `AgentSshMode`, `EciesKeypair`
  must exist in `agent-ssh-runtime`; transitional `service_migrations()` free function must exist.
- Plan 3 (`2026-05-07-mqtt-shell-refactor.md`) — `MqttHandler` must exist in `mqtt-runtime`.

**Tech Stack:** Rust, sea-orm-migration, uptrakit-service-sdk, tokio

---

## File Map

| Action | Path                                                          |
| ------ | ------------------------------------------------------------- |
| Modify | `crates/shared/service-sdk/Cargo.toml`                        |
| Modify | `crates/shared/service-sdk/src/shared_types.rs`               |
| Modify | `crates/core/agent-ssh-runtime/Cargo.toml`                    |
| Modify | `crates/core/agent-ssh-runtime/src/handler.rs`                |
| Modify | `crates/core/controller-runtime/Cargo.toml`                   |
| Modify | `crates/core/controller-runtime/src/migration/mod.rs`         |
| Modify | `crates/core/mqtt-runtime/src/handler.rs`                     |
| Modify | `crates/core/controller-runtime/src/service_host/builtins.rs` |
| Modify | `crates/core/controller-runtime/src/ssh_agent/mod.rs`         |
| Modify | `crates/core/controller-runtime/src/mqtt/mod.rs`              |
| Create | `docs/adr/0005-service-binary-runtime-boundary.md`            |
| Modify | `docs/development/coding-standards.md`                        |

---

### Task 1: Add service_migrations() to ServiceHandler in service-sdk

**Files:**

- Modify: `crates/shared/service-sdk/Cargo.toml`
- Modify: `crates/shared/service-sdk/src/shared_types.rs`

- [ ] **Step 1: Write failing test**

```bash
grep -q "service_migrations" crates/shared/service-sdk/src/shared_types.rs && \
  echo "PASS" || echo "FAIL: service_migrations not in shared_types.rs"
```

Expected: `FAIL`.

- [ ] **Step 2: Add feature and dep to service-sdk Cargo.toml**

In `crates/shared/service-sdk/Cargo.toml`, add to the `[features]` section:

```toml
service-migrations = ["dep:sea-orm-migration"]
```

Add to `[dependencies]`:

```toml
sea-orm-migration = { workspace = true, optional = true }
```

- [ ] **Step 3: Add service_migrations() default method to ServiceHandler trait**

In `crates/shared/service-sdk/src/shared_types.rs`, add the following method inside the
`ServiceHandler` trait definition, after the `on_shutdown` method and before the closing `}`:

```rust
    /// Schema migrations contributed by this Service.
    ///
    /// Called by the Controller at startup to collect embedded Service migrations
    /// before running the combined migrator. Services without a local DB return
    /// the default empty `vec![]`.
    ///
    /// `where Self: Sized` excludes this static method from the `dyn ServiceHandler` vtable,
    /// preserving object safety.
    #[cfg(feature = "service-migrations")]
    fn service_migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>>
    where
        Self: Sized,
    {
        vec![]
    }
```

- [ ] **Step 4: Run the test**

```bash
grep -q "service_migrations" crates/shared/service-sdk/src/shared_types.rs && \
  echo "PASS" || echo "FAIL"
```

Expected: `PASS`.

- [ ] **Step 5: Verify service-sdk compiles (with and without the feature)**

```bash
cargo check -p uptrakit-service-sdk && \
cargo check -p uptrakit-service-sdk --features service-migrations
```

Expected: no errors for either command.

- [ ] **Step 6: Commit**

```bash
git add crates/shared/service-sdk/Cargo.toml crates/shared/service-sdk/src/shared_types.rs
git commit -m "feat(service-sdk): add service_migrations() to ServiceHandler trait"
```

---

### Task 2: Override service_migrations() in AgentSshHandler; remove transitional free function

**Files:**

- Modify: `crates/core/agent-ssh-runtime/Cargo.toml`
- Modify: `crates/core/agent-ssh-runtime/src/handler.rs`

`handler.rs` was created in Plan 1. `agent-ssh-runtime/Cargo.toml` was updated in Plan 1 with
the full dep list. Plan 1 also added the transitional free function
`pub fn service_migrations() -> Vec<Box<dyn MigrationTrait>>` in `lib.rs`; that function is
removed here.

- [ ] **Step 1: Add service-migrations feature to agent-ssh-runtime Cargo.toml**

In `crates/core/agent-ssh-runtime/Cargo.toml`, add to `[features]`:

```toml
service-migrations = ["uptrakit-service-sdk/service-migrations"]
```

(`sea-orm-migration` is already a direct dep from Plan 1's Cargo.toml update.)

- [ ] **Step 2: Add the service_migrations() override in handler.rs**

In `crates/core/agent-ssh-runtime/src/handler.rs`, add the following method inside
`impl ServiceHandler for AgentSshHandler`, immediately after the `on_shutdown` method:

```rust
    #[cfg(feature = "service-migrations")]
    fn service_migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>>
    where
        Self: Sized,
    {
        crate::db::migration::service_migrations()
    }
```

`crate::db::migration::service_migrations()` is the existing function from Plan 1's
`agent-ssh-runtime/src/db/migration/mod.rs` that returns the 13 agent-ssh migrations.

- [ ] **Step 3: Remove the transitional free function from agent-ssh-runtime/src/lib.rs**

In `crates/core/agent-ssh-runtime/src/lib.rs`, delete the transitional function:

```rust
pub fn service_migrations() -> Vec<Box<dyn MigrationTrait>> {
    db::migration::service_migrations()
}
```

(Also remove the `MigrationTrait` import in lib.rs that was only used by that function, if it
exists as a standalone use item.)

- [ ] **Step 4: Verify agent-ssh-runtime compiles**

```bash
cargo check -p uptrakit-agent-ssh-runtime --features service-migrations,db-sqlite
```

Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add crates/core/agent-ssh-runtime/Cargo.toml crates/core/agent-ssh-runtime/src/handler.rs crates/core/agent-ssh-runtime/src/lib.rs
git commit -m "feat(agent-ssh-runtime): override service_migrations() on AgentSshHandler; remove transitional free fn"
```

---

### Task 3: Update controller-runtime migration/mod.rs to call AgentSshHandler::service_migrations()

**Files:**

- Modify: `crates/core/controller-runtime/Cargo.toml`
- Modify: `crates/core/controller-runtime/src/migration/mod.rs`

- [ ] **Step 1: Enable service-migrations in controller-runtime's embedded-ssh-agent feature**

In `crates/core/controller-runtime/Cargo.toml`, find the `embedded-ssh-agent` feature and add
`"uptrakit-agent-ssh-runtime/service-migrations"`:

```toml
embedded-ssh-agent = ["dep:uptrakit-agent-core", "dep:uptrakit-agent-ssh-runtime", "dep:base64", "uptrakit-agent-ssh-runtime/service-migrations"]
```

- [ ] **Step 2: Update migration/mod.rs to use the trait method**

Replace the entire contents of
`crates/core/controller-runtime/src/migration/mod.rs` with:

```rust
use crate::db::{DbError, Result};
use rootcause::prelude::*;
use sea_orm::DatabaseConnection;

pub(crate) async fn run_migrations(db: &DatabaseConnection) -> Result<()> {
    let mut plugin_migrations = uptrakit_plugin_infrastructure_registry::all_descriptors()
        .into_iter()
        .filter_map(|d| d.migrations)
        .flat_map(|f| f())
        .collect::<Vec<_>>();

    #[cfg(feature = "embedded-ssh-agent")]
    plugin_migrations.extend(
        uptrakit_agent_ssh_runtime::AgentSshHandler::service_migrations(),
    );

    uptrakit_shared_db::migration::run_migrations_with_plugins(db, plugin_migrations)
        .await
        .context_to::<DbError>()
        .context(DbError::Migration(
            "running database migrations".to_string(),
        ))
}
```

- [ ] **Step 3: Verify controller-runtime migration compiles**

```bash
cargo check -p uptrakit-controller-runtime --features embedded-ssh-agent,db-sqlite
```

Expected: no errors.

- [ ] **Step 4: Run existing repair-migration test**

```bash
cargo test -p uptrakit-shared-db --features db-sqlite -- --nocapture
```

Expected: the repair migration test (added in Plan 1) passes — 13 rows present, old row gone.

- [ ] **Step 5: Commit**

```bash
git add crates/core/controller-runtime/Cargo.toml crates/core/controller-runtime/src/migration/mod.rs
git commit -m "feat(controller-runtime): call AgentSshHandler::service_migrations() in run_migrations"
```

---

### Task 4: Add MqttHandler::new_embedded() for embedded use

**Files:**

- Modify: `crates/core/mqtt-runtime/src/handler.rs`

`MqttHandler` (from Plan 3) wraps `MqttRuntime`. In embedded mode, the controller generates an
ECIES keypair and passes it as `MqttRuntimeIdentity`. The embedded handler stores it and calls
`runtime.on_connected` from inside `on_settings` (since `run_embedded_service` does not call
`on_connected`).

- [ ] **Step 1: Write failing test**

```bash
grep -q "new_embedded" crates/core/mqtt-runtime/src/handler.rs && \
  echo "PASS" || echo "FAIL: new_embedded not found"
```

Expected: `FAIL`.

- [ ] **Step 2: Add embedded_identity field and new_embedded() constructor**

In `crates/core/mqtt-runtime/src/handler.rs`:

Change the `MqttHandler` struct from:

```rust
pub struct MqttHandler {
    runtime: MqttRuntime,
}
```

to:

```rust
pub struct MqttHandler {
    runtime: MqttRuntime,
    embedded_identity: Option<MqttRuntimeIdentity>,
}
```

Change `MqttHandler::new()` from:

```rust
    pub fn new() -> Self {
        Self {
            runtime: MqttRuntime::new(),
        }
    }
```

to:

```rust
    pub fn new() -> Self {
        Self {
            runtime: MqttRuntime::new(),
            embedded_identity: None,
        }
    }

    pub fn new_embedded(identity: MqttRuntimeIdentity) -> Self {
        Self {
            runtime: MqttRuntime::new(),
            embedded_identity: Some(identity),
        }
    }
```

Change the `Default` impl to remain consistent (calls `new()`; already correct).

- [ ] **Step 3: Update on_settings to call on_connected when embedded_identity is set**

Replace the existing `on_settings` method:

```rust
    async fn on_settings(
        &mut self,
        settings: &uptrakit_wire::ServiceSettingsPayload,
        conn: &mut dyn ServiceTransport,
        agreed_capabilities: &BTreeSet<Capability>,
    ) {
        self.runtime
            .apply_settings(
                MqttRuntimeSettings {
                    ui_surfaces_enabled: agreed_capabilities.contains(&Capability::UiSurfaces),
                    tenant_id: settings.tenant_id,
                },
                conn,
            )
            .await;
    }
```

with:

```rust
    async fn on_settings(
        &mut self,
        settings: &uptrakit_wire::ServiceSettingsPayload,
        conn: &mut dyn ServiceTransport,
        agreed_capabilities: &BTreeSet<Capability>,
    ) {
        if let Some(identity) = self.embedded_identity.take() {
            if let Err(e) = self.runtime.on_connected(conn, identity).await {
                tracing::error!(error = %e, "embedded MQTT: failed to initialize runtime");
                return;
            }
        }
        self.runtime
            .apply_settings(
                MqttRuntimeSettings {
                    ui_surfaces_enabled: agreed_capabilities.contains(&Capability::UiSurfaces),
                    tenant_id: settings.tenant_id,
                },
                conn,
            )
            .await;
    }
```

- [ ] **Step 4: Run the test**

```bash
grep -q "new_embedded" crates/core/mqtt-runtime/src/handler.rs && \
  echo "PASS" || echo "FAIL"
```

Expected: `PASS`.

- [ ] **Step 5: Verify mqtt-runtime compiles**

```bash
cargo check -p uptrakit-mqtt-runtime
```

Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git add crates/core/mqtt-runtime/src/handler.rs
git commit -m "feat(mqtt-runtime): add MqttHandler::new_embedded() for embedded controller use"
```

---

### Task 5: Replace run_embedded_ssh_agent in builtins.rs

**Files:**

- Modify: `crates/core/controller-runtime/src/service_host/builtins.rs`

The `register_agent_ssh` function currently builds a closure that calls the now-deleted
`run_embedded_ssh_agent`. It becomes: generate ECIES keypair, construct `AgentSshHandler`, call
`run_embedded_service`.

- [ ] **Step 1: Write failing check**

```bash
grep -q "run_embedded_ssh_agent" crates/core/controller-runtime/src/service_host/builtins.rs && \
  echo "FAIL: old call still present" || echo "PASS: already updated"
```

Expected: `FAIL: old call still present` (not updated yet).

- [ ] **Step 2: Update register_agent_ssh()**

In `crates/core/controller-runtime/src/service_host/builtins.rs`, replace the body of
`register_agent_ssh()` from the keypair/closure portion:

```rust
    let add_result = host
        .add(
            &AGENT_SSH,
            ssh_caps.clone(),
            false,
            Some(default_tenant_id),
            controller_installation_id,
            map_yield_policy(&AGENT_SSH, None),
            move |transport, tokens| {
                Box::pin(crate::ssh_agent::run_embedded_ssh_agent(
                    transport,
                    tokens,
                    state_dir,
                    db_for_ssh,
                    default_tenant_id,
                ))
            },
            app_state,
            bg,
            None,
        )
        .await?;
```

with:

```rust
    let (private_key_der, encryption_public_key) =
        crate::ssh_agent::generate_ecies_keypair()?;
    let keypair = uptrakit_agent_ssh_runtime::EciesKeypair {
        private_key_der,
        encryption_public_key,
    };
    let handler = uptrakit_agent_ssh_runtime::AgentSshHandler::new(
        db_for_ssh,
        state_dir,
        uptrakit_agent_ssh_runtime::AgentSshMode::Embedded,
        Some(keypair),
    );

    let add_result = host
        .add(
            &AGENT_SSH,
            ssh_caps.clone(),
            false,
            Some(default_tenant_id),
            controller_installation_id,
            map_yield_policy(&AGENT_SSH, None),
            move |transport, tokens| {
                Box::pin(uptrakit_service_sdk::run_embedded_service(
                    handler,
                    transport,
                    tokens.drain,
                    tokens.abort,
                ))
            },
            app_state,
            bg,
            None,
        )
        .await?;
```

- [ ] **Step 3: Run the check**

```bash
grep -q "run_embedded_ssh_agent" crates/core/controller-runtime/src/service_host/builtins.rs && \
  echo "FAIL: old call still present" || echo "PASS"
```

Expected: `PASS`.

- [ ] **Step 4: Verify controller-runtime compiles with embedded-ssh-agent**

```bash
cargo check -p uptrakit-controller-runtime --features embedded-ssh-agent,db-sqlite
```

Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add crates/core/controller-runtime/src/service_host/builtins.rs
git commit -m "feat(controller-runtime): replace run_embedded_ssh_agent with run_embedded_service<AgentSshHandler>"
```

---

### Task 6: Replace run_embedded_mqtt in builtins.rs and update mqtt/mod.rs

**Files:**

- Modify: `crates/core/controller-runtime/src/service_host/builtins.rs`
- Modify: `crates/core/controller-runtime/src/mqtt/mod.rs`

- [ ] **Step 1: Update generate_ecies_keypair in mqtt/mod.rs to return rootcause::Result**

In `crates/core/controller-runtime/src/mqtt/mod.rs`, replace:

```rust
fn generate_ecies_keypair() -> Result<MqttRuntimeIdentity, String> {
    let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .map_err(|error| format!("P-256 key generation failed: {error}"))?;
    let private_der = key_pair.serialize_der();
    let public_raw = key_pair.public_key_raw().to_vec();
    let public_b64 = base64::engine::general_purpose::STANDARD.encode(&public_raw);

    Ok(MqttRuntimeIdentity {
        service_id: None,
        private_key_der: Some(private_der),
        encryption_public_key: Some(public_b64),
    })
}
```

with:

```rust
fn generate_ecies_keypair() -> rootcause::Result<MqttRuntimeIdentity> {
    use rootcause::prelude::*;
    let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .map_err(|e| report!(std::io::Error::other(format!("P-256 key generation failed: {e}"))))?;
    let private_der = key_pair.serialize_der();
    let public_raw = key_pair.public_key_raw().to_vec();
    let public_b64 = base64::engine::general_purpose::STANDARD.encode(&public_raw);

    Ok(MqttRuntimeIdentity {
        service_id: None,
        private_key_der: Some(private_der),
        encryption_public_key: Some(public_b64),
    })
}
```

- [ ] **Step 2: Update register_mqtt() in builtins.rs**

In `crates/core/controller-runtime/src/service_host/builtins.rs`, replace the closure
portion of `register_mqtt()`:

```rust
    let add_result = host
        .add(
            &MQTT,
            mqtt_caps.clone(),
            true,
            None,
            controller_installation_id,
            map_yield_policy(&MQTT, None),
            move |transport, tokens| {
                Box::pin(crate::mqtt::run_embedded_mqtt(
                    transport,
                    tokens,
                    default_tenant_id,
                ))
            },
            app_state,
```

with:

```rust
    let identity = crate::mqtt::generate_ecies_keypair()?;
    let handler = uptrakit_mqtt_runtime::MqttHandler::new_embedded(identity);

    let add_result = host
        .add(
            &MQTT,
            mqtt_caps.clone(),
            true,
            None,
            controller_installation_id,
            map_yield_policy(&MQTT, None),
            move |transport, tokens| {
                Box::pin(uptrakit_service_sdk::run_embedded_service(
                    handler,
                    transport,
                    tokens.drain,
                    tokens.abort,
                ))
            },
            app_state,
```

Also add the missing `uptrakit_mqtt_runtime` import at the top of the `register_mqtt` function
body if not already present (it may be a `use` at crate level — check).

- [ ] **Step 3: Make generate_ecies_keypair pub(crate) accessible from builtins.rs**

`generate_ecies_keypair` is currently private in `mqtt/mod.rs`. Change to `pub(crate)`:

```rust
pub(crate) fn generate_ecies_keypair() -> rootcause::Result<MqttRuntimeIdentity> {
```

- [ ] **Step 4: Verify controller-runtime compiles with embedded-mqtt**

```bash
cargo check -p uptrakit-controller-runtime --features embedded-mqtt,db-sqlite
```

Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add crates/core/controller-runtime/src/service_host/builtins.rs crates/core/controller-runtime/src/mqtt/mod.rs
git commit -m "feat(controller-runtime): replace run_embedded_mqtt with run_embedded_service<MqttHandler>"
```

---

### Task 7: Delete the hand-rolled embedded loops

**Files:**

- Modify: `crates/core/controller-runtime/src/ssh_agent/mod.rs`
- Modify: `crates/core/controller-runtime/src/mqtt/mod.rs`

- [ ] **Step 1: Delete run_embedded_ssh_agent from ssh_agent/mod.rs**

In `crates/core/controller-runtime/src/ssh_agent/mod.rs`, delete:

- The entire `pub(crate) async fn run_embedded_ssh_agent(...)` function and all its content.
- Any imports that are now unused (check with `cargo check`).

The file should retain only:

- `ssh_agent_capabilities()` re-export
- `generate_ecies_keypair()` function
- Any imports needed by those two items

- [ ] **Step 2: Delete run_embedded_mqtt loop from mqtt/mod.rs**

In `crates/core/controller-runtime/src/mqtt/mod.rs`, delete:

- The entire `pub(crate) async fn run_embedded_mqtt(...)` function.
- The local `async fn handle_controller_message(...)` helper (used only by
  `run_embedded_mqtt`).
- All imports now made unused by this deletion.

The file should retain:

- `mqtt_capabilities()` re-export
- `send_initial_service_config()` function
- `pub(crate) fn generate_ecies_keypair()` function
- Imports needed by those three items: `Arc`, `BTreeSet`, `base64::Engine`, `MqttRuntimeIdentity`,
  `mqtt_capabilities as runtime_capabilities`, `Capability`, `ControllerMessage`

- [ ] **Step 3: Verify ssh_agent/mod.rs compiles**

```bash
cargo check -p uptrakit-controller-runtime --features embedded-ssh-agent,db-sqlite
```

Expected: no errors.

- [ ] **Step 4: Verify mqtt/mod.rs compiles**

```bash
cargo check -p uptrakit-controller-runtime --features embedded-mqtt,db-sqlite
```

Expected: no errors.

- [ ] **Step 5: Verify full-features compile**

```bash
cargo check -p uptrakit-controller-runtime --all-features
```

Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git add crates/core/controller-runtime/src/ssh_agent/mod.rs crates/core/controller-runtime/src/mqtt/mod.rs
git commit -m "refactor(controller-runtime): delete hand-rolled embedded service loops"
```

---

### Task 8: Write docs/adr/0005-service-binary-runtime-boundary.md

**Files:**

- Create: `docs/adr/0005-service-binary-runtime-boundary.md`

- [ ] **Step 1: Write the ADR**

```markdown
# 0005 — Service Binary/Runtime Boundary

**Date:** 2026-05-07
**Status:** Accepted

## Context

Service binary crates (`agent-ssh`, `mqtt`, `scheduler`) had poorly defined boundaries with
their `-runtime` counterparts. The root symptom: `agent-ssh` owned its DB entities and 12
migrations, so when `agent-ssh` ran embedded inside the Controller the same tables had to be
created a second way — `shared-db` carried `m20260331_000001_ssh_agent_tables`, a compressed
one-shot migration that duplicated the standalone schema. Any SSH agent schema change had to be
applied in two places or drift silently.

Additionally, `controller-runtime` depended on `uptrakit-agent-ssh` (a binary crate) to access
the embedded handler, violating the principle that the controller only depends on library crates.

The prerequisite for this ADR — `run_embedded_service`, `ShutdownCause::EmbeddedDrain`,
`dyn ServiceTransport` handler signatures, and controller-side `ServiceSettings` injection — was
shipped as ADR-0004.

## Decision

**Binary crates are thin launch shells; runtime crates own all business logic, DB schema, and
shared operations.**

The invariant for every Service binary crate:

| Allowed              | Examples                                                                                 |
| -------------------- | ---------------------------------------------------------------------------------------- |
| Entry point          | `main.rs` — process init, construct handler, call `run_lifecycle_and_handle_errors`      |
| CLI argument structs | `cli.rs` — clap derives only                                                             |
| Subcommand dispatch  | `host_cli.rs`, `commands/` (agent-ssh only) — argument parsers + thin calls into runtime |

Not allowed in binary crates: DB entities, migrations, business logic, protocol implementation,
transport logic, surface handlers, crypto helpers.

Each `-runtime` crate exports exactly one `ServiceHandler` implementation. The controller and
the standalone binary both construct that handler with different dependencies but the same type.
The controller never depends on a binary crate.

### service_migrations()

`ServiceHandler` gains a static method `service_migrations()` (behind the `service-migrations`
feature gate in `uptrakit-service-sdk`) returning the migrations this service owns. The default
returns `vec![]`. Runtime crates with a local DB override it.

The controller calls `AgentSshHandler::service_migrations()` at startup and passes the result
to `run_migrations_with_plugins`.

### Migration Strategy for Existing Controller Deployments (B+B1 over Tombstone)

Replacing `shared-db`'s monolithic `m20260331_000001_ssh_agent_tables` with a repair migration
(`m20260331_000002_agent_ssh_migration_history_repair`) uses a **B+B1 strategy** rather than
a tombstone:

- **B (repair migration):** Checks whether the old one-shot row is present in
  `seaql_migrations`. If so, inserts the 13 individual agent-ssh migration rows with
  `ON CONFLICT DO NOTHING`, then deletes the old row.
- **B1 (service migrations):** The 13 migrations from `agent-ssh-runtime` are contributed at
  startup. On existing deployments they are already recorded (B ran first); on fresh installs
  they run normally.

**Frozen-list constraint:** The 13 migration names in the repair migration's INSERT list are
frozen at the time the repair is written. No new `agent-ssh` migrations may be added between
writing the repair and shipping it to production. If a new agent-ssh migration lands in the
same release, it must be added to the INSERT list before the release cuts.

### Crate structure after refactor

| Before                                               | After                                                       |
| ---------------------------------------------------- | ----------------------------------------------------------- |
| `controller-runtime` dep on `uptrakit-agent-ssh`     | `controller-runtime` dep on `uptrakit-agent-ssh-runtime`    |
| `agent-ssh` owned 13 migrations + all business logic | `agent-ssh-runtime` owns all; `agent-ssh` is a thin shell   |
| `shared-db` carried duplicated SSH schema migration  | `shared-db` carries only repair migration; no SSH schema    |
| Hand-rolled embedded loops in `controller-runtime`   | Unified `run_embedded_service<H>` for all embedded services |

## Consequences

**Positive:**

- SSH schema changes happen in one place (agent-ssh-runtime migrations only).
- Controller never depends on binary crates.
- Adding a new embedded service requires only constructing its `ServiceHandler` and calling
  `run_embedded_service` — no hand-written event loops.
- `service_migrations()` provides a compile-time, type-safe mechanism for services to
  contribute schema changes.

**Negative:**

- Each embedded service's `ServiceHandler` must be constructible with controller-provided deps
  (DB connection, state dir, ECIES keypair). Handler constructors must not hardcode internal
  paths or open their own DB connections.
- The frozen-list constraint on the repair migration requires coordination when new agent-ssh
  migrations land during the same release as the refactor.
```

- [ ] **Step 2: Verify markdown lint**

```bash
npx prettier --check docs/adr/0005-service-binary-runtime-boundary.md
```

If it reports issues:

```bash
npx prettier --write docs/adr/0005-service-binary-runtime-boundary.md
```

- [ ] **Step 3: Commit**

```bash
git add docs/adr/0005-service-binary-runtime-boundary.md
git commit -m "docs(adr): add ADR-0005 service binary/runtime boundary"
```

---

### Task 9: Add coding-standards section

**Files:**

- Modify: `docs/development/coding-standards.md`

- [ ] **Step 1: Append the new section at the end of coding-standards.md**

Add the following after the last line of `docs/development/coding-standards.md`:

````markdown
## Service Binary/Runtime Boundary

Every Service binary crate (`agent-ssh`, `mqtt`, `scheduler`, …) is a **thin launch shell**.
All business logic, DB entities, migrations, protocol handling, and crypto helpers live in the
corresponding `-runtime` crate. See [ADR-0005](../adr/0005-service-binary-runtime-boundary.md).

### What belongs where

| Binary crate (`*`)       | Runtime crate (`*-runtime`)                         |
| ------------------------ | --------------------------------------------------- |
| `main.rs` — process init | `ServiceHandler` implementation                     |
| `cli.rs` — clap structs  | DB entities and migrations (`service_migrations()`) |
| Subcommand dispatch      | Business logic, surface handlers, crypto helpers    |
| _Nothing else_           | Protocol implementation, transport adapters         |

### service_migrations()

Runtime crates that own a local DB override `ServiceHandler::service_migrations()`
(feature-gated via `uptrakit-service-sdk/service-migrations`) to return their migration list.
The controller calls it as a static method on the concrete handler type at startup:

```rust
let migrations = AgentSshHandler::service_migrations();
run_migrations_with_plugins(db, migrations).await?;
```
````

Services without a DB rely on the default `vec![]`.

### Embedded service construction

The controller constructs the handler with controller-sourced deps (shared DB, state dir,
pre-generated ECIES keypair), then passes it to `run_embedded_service::<H>`. The handler's
constructor must not open its own DB connections or read paths from the environment.

```rust
let handler = AgentSshHandler::new(shared_db, state_dir, AgentSshMode::Embedded, Some(keypair));
run_embedded_service(handler, transport, tokens.drain, tokens.abort).await;
```

The standalone binary does the same with `AgentSshMode::Binary` and `None` for the keypair.

````text

- [ ] **Step 2: Lint and format**

```bash
npx prettier --write docs/development/coding-standards.md
````

- [ ] **Step 3: Verify markdown lint passes**

```bash
npx prettier --check docs/development/coding-standards.md
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add docs/development/coding-standards.md
git commit -m "docs(coding-standards): add service binary/runtime boundary section"
```

---

### Task 10: Quality gates

- [ ] **Step 1: Format**

```bash
cargo fmt --all
```

- [ ] **Step 2: Clippy — no-default-features**

```bash
cargo clippy --all-targets --no-default-features --features db-sqlite 2>&1 | grep -E "^error"
```

Expected: no errors.

- [ ] **Step 3: Clippy — all features**

```bash
cargo clippy --all-targets --all-features 2>&1 | grep -E "^error"
```

Expected: no errors.

- [ ] **Step 4: Full test suite**

```bash
cargo test --all-features
```

Expected: all tests pass.

- [ ] **Step 5: Verify no binary crate is a controller dep**

```bash
cargo tree -p uptrakit-controller-runtime --all-features | grep "uptrakit-agent-ssh " | grep -v "runtime"
```

Expected: no output (the binary crate `uptrakit-agent-ssh` does not appear as a controller dep).

- [ ] **Step 6: cargo deny check**

```bash
cargo deny check
```

Expected: no issues.

- [ ] **Step 7: Commit formatting if needed**

```bash
git add -u && git diff --cached --quiet || git commit -m "style: cargo fmt after WS5 unified embedded path"
```

---

**Plan complete and saved to `docs/superpowers/plans/2026-05-07-controller-unified-embedded-path.md`.**
