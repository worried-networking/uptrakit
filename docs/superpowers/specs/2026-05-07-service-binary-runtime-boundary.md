# Service Binary/Runtime Boundary Refactor

**Date:** 2026-05-07
**Status:** Approved — pending implementation

---

## Problem

Service binary crates (`agent-ssh`, `mqtt`, `scheduler`) have poorly defined boundaries with
their `-runtime` counterparts. The root symptom: `agent-ssh` owns its DB entities and 12
migrations, so when `agent-ssh` runs embedded inside the Controller the same tables must be
created a second way — `shared-db` carries `m20260331_000001_ssh_agent_tables`, a compressed
one-shot migration that duplicates the standalone schema. Any SSH agent schema change must be
applied in two places or drift silently.

The goal is a consistent rule: **binary crates are thin launch shells; runtime crates own all
business logic, DB schema, and shared operations.**

---

## Prerequisite: service-sdk Embedded Transport Abstraction

This spec describes the target architecture. Fully realising the controller side of the unified
`ServiceHandler` path (Work Stream 5) required a prerequisite: making `ControllerConnection`
transport-agnostic so the same handler type works with both a WebSocket and an `EmbeddedTransport`.

**The prerequisite has shipped.** `run_embedded_service`, `ShutdownCause::EmbeddedDrain`,
`dyn ServiceTransport` handler signatures, `EmbeddedTransport::yield_change_notifier`, and
controller-side `ServiceSettings` injection are all in place. Work Stream 5 is now unblocked.

The prerequisite spec remains at
`docs/superpowers/specs/2026-05-07-service-sdk-embedded-transport.md`; its plan was retired into bead
epic `uptrakit-spec-2026-05-07-service-sdk-embedded-transport` at the beads migration (2026-08-16; full
text at `pre-beads-archive`).

---

## Invariant: What a Binary Crate May Contain

After this refactor every Service binary crate satisfies:

| Allowed              | Examples                                                                                                                |
| -------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| Entry point          | `main.rs` — process-level init (crypto, master key, tracing), construct handler, call `run_lifecycle_and_handle_errors` |
| CLI argument structs | `cli.rs` — clap `Parser` derives only                                                                                   |
| Subcommand dispatch  | `host_cli.rs`, `commands/` (agent-ssh only) — argument parsers + thin calls into runtime                                |

**Not allowed in binary crates:** DB entities, migrations, business logic functions, protocol
implementation, transport logic, surface handlers, crypto helpers.

The `-runtime` crate owns everything else. Each runtime crate exports **exactly one**
`ServiceHandler` implementation. The controller and the standalone binary both construct that
handler — passing different dependencies (pre-existing DB vs fresh local SQLite) but using the
same type. The controller **never** depends on a binary crate.

---

## `service_migrations()` on `ServiceHandler`

`uptrakit-service-sdk` gains an associated function on `ServiceHandler`:

```rust
pub trait ServiceHandler {
    // ... existing methods ...

    /// Schema migrations contributed by this Service.
    ///
    /// Called by the Controller at startup to collect embedded Service migrations
    /// before running the combined migrator. Services without a local DB return
    /// the default empty vec.
    ///
    /// `where Self: Sized` is required because this is a static method (no `self` receiver).
    /// The bound excludes the method from the `dyn ServiceHandler` vtable — it cannot be
    /// called through a trait object — which is what allows the trait to remain object-safe.
    fn service_migrations() -> Vec<Box<dyn MigrationTrait>>
    where
        Self: Sized,
    {
        vec![]
    }
}
```

Runtime crates that own a DB override this on their single `ServiceHandler` implementation.
The controller calls it as an associated function on the concrete type:

```rust
// controller-runtime startup:
let migrations = AgentSshHandler::service_migrations();
run_migrations_with_plugins(db, migrations).await?;
```

Until WS5 ships (the controller unified embedded path), the controller calls a transitional
free function `uptrakit_agent_ssh_runtime::service_migrations()` that the `ServiceHandler` impl
delegates to internally. The free function is removed as part of WS5.

---

## Work Stream 1 — `agent-ssh` Refactor

### Single handler in `agent-ssh-runtime`

`agent-ssh-runtime` exports one handler type:

```rust
pub struct AgentSshHandler { /* SshAgentRuntime + AgentSshRuntimeSupport */ }

impl ServiceHandler for AgentSshHandler {
    fn service_migrations() -> Vec<Box<dyn MigrationTrait>> {
        // returns the 13 standalone migrations
    }
    // on_connected, on_message, on_settings, on_shutdown, ...
}
```

Constructor accepts only the externally-sourced dependencies that differ between modes. All
internal-only components (`SshConnectionPool`, `ServiceSurfaceProxy`, infra bundles) are
constructed inside the runtime — callers have no reason to know about them.

`AgentSshMode` is a two-variant internal discriminator enum (not `#[non_exhaustive]`, implements
`Copy`); it follows the typed-enum pattern used for other internal discriminators in the codebase.

```rust
/// Two-variant mode discriminator. Typed enum over bare `bool` per coding-standards.
/// `#[non_exhaustive]` is NOT applied — this is an internal discriminator, not a wire type.
///
/// Note: "standalone" is an avoided term per the domain glossary (CONTEXT.md).
/// `Binary` names the mode where the service runs as its own process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSshMode {
    /// Service runs as its own binary process with a local SQLite DB.
    /// Performs surface self-registration.
    Binary,
    /// Service runs embedded inside the Controller process, using the shared DB.
    /// Skips surface self-registration (controller registers on its behalf).
    Embedded,
}

/// ECIES keypair injected at construction time. Named type (not a raw tuple) so the
/// controller-side generator and the constructor share a stable typed boundary.
/// Defined in `agent-ssh-runtime`; constructed from `generate_ecies_keypair()` in the controller.
///
/// Invariant: when this struct is constructed from `generate_ecies_keypair()`, `private_key_der`
/// is always `Some`. The `Option` reflects the existing return type of the generator; callers
/// must not construct `EciesKeypair` with `private_key_der: None`.
pub struct EciesKeypair {
    pub private_key_der: Option<Vec<u8>>,
    pub encryption_public_key: String,
}

impl AgentSshHandler {
    pub fn new(
        db: DatabaseConnection,       // local SQLite (Binary mode) or shared DB (Embedded mode)
        state_dir: PathBuf,
        mode: AgentSshMode,           // controls surface registration behaviour
        ecies_keypair: Option<EciesKeypair>, // Some in embedded mode (generated by controller),
                                      // None in binary process mode (generated lazily at enrollment)
    ) -> Self {
        // SshConnectionPool::new(), ServiceSurfaceProxy::new(),
        // build_catalog + create_infra_bundles all happen here
    }
}
```

`generate_ecies_keypair` is a controller-side private function in
`controller-runtime/src/ssh_agent/mod.rs` that generates ECIES keypairs for the embedded SSH
agent. It does not use agent-ssh DB entities and stays in the controller after the refactor;
its output is passed into `AgentSshHandler::new` as the `ecies_keypair` parameter.

`SERVICE_APP_NAME` on `AgentSshHandler` **must** be set to the string literal `"uptrakit-agent-ssh"`,
not `env!("CARGO_PKG_NAME")`. The enrollment system and `controller-runtime/src/service_host/builtins.rs`
match service registrations by app name; changing it breaks existing enrolled agents.

**Standalone binary** (`agent-ssh/main.rs`): opens local SQLite, inits master key + DEK ring,
calls `AgentSshHandler::new(local_db, state_dir, AgentSshMode::Binary, None)`,
then `run_lifecycle_and_handle_errors`.

**Controller embedded** (WS5): generates ECIES keypair via
`generate_ecies_keypair()`, passes shared `DatabaseConnection`,
calls `AgentSshHandler::new(shared_db, ssh_state_dir, AgentSshMode::Embedded, Some(keypair))`,
then `run_embedded_service::<AgentSshHandler>`.

### What moves from `agent-ssh` → `agent-ssh-runtime`

| Module / file                                                              | Notes                                                   |
| -------------------------------------------------------------------------- | ------------------------------------------------------- |
| `src/db/` (entities + 13 migrations)                                       | Runtime owns its schema                                 |
| `src/runtime_support.rs` (`AgentSshRuntimeSupport` impl)                   | Shared by standalone + embedded                         |
| `src/operations/`                                                          | Business logic shared by CLI commands and surface calls |
| `src/surface_runtime/`                                                     | Shared by web-api surfaces and CLI dispatch             |
| `src/client.rs`, `src/host_ops.rs`                                         | SSH operation helpers                                   |
| `src/ssh_pool.rs`, `src/ssh_transport.rs`, `src/ssh_target.rs`             | SSH transport layer                                     |
| `src/ssh_executor.rs`, `src/ssh_key.rs`, `src/ssh_stdio_tunnel.rs`         | SSH execution                                           |
| `src/remote_exec.rs`, `src/host_info.rs`                                   | Remote execution helpers                                |
| `src/error.rs`                                                             | Error types follow the entities                         |
| `init_ssh_data_key_ring`, `reencrypt_ssh_to_v3`, `register_ssh_column_aad` | Use DB entities                                         |
| `rotate_ssh_master_key` (currently in `main.rs`)                           | Uses DB entities, belongs in runtime                    |
| `SshAgentHandler` (currently in `main.rs`)                                 | Becomes `AgentSshHandler` in runtime                    |

### What stays in the `agent-ssh` binary

| File              | Contents                                                                                                         |
| ----------------- | ---------------------------------------------------------------------------------------------------------------- |
| `src/main.rs`     | Process init: crypto, master key, state dir, local DB open, DEK ring, construct `AgentSshHandler`, run lifecycle |
| `src/cli.rs`      | clap argument structs                                                                                            |
| `src/host_cli.rs` | Subcommand dispatch (calls runtime operations)                                                                   |
| `src/commands/`   | Argument parsers for bootstrap/sync/sudoers/proxmox subcommands                                                  |

### Sharing between CLI and surfaces

`operations/` in `agent-ssh-runtime` is the single implementation of bootstrap, sync, sudoers,
and Proxmox logic. Both callers are thin:

- `agent-ssh/src/commands/` parses CLI args → calls `agent_ssh_runtime::operations::*`
- `agent-ssh-runtime/src/surface_runtime/` handles surface action requests → calls the same
  `operations::*` functions

No business logic lives in either caller.

### Dependency flip

| Before                                                            | After                                                                      |
| ----------------------------------------------------------------- | -------------------------------------------------------------------------- |
| `controller-runtime` depends on `uptrakit-agent-ssh`              | `controller-runtime` depends on `uptrakit-agent-ssh-runtime` only          |
| `agent-ssh` binary depends on `uptrakit-agent-ssh-runtime`        | unchanged                                                                  |
| `uptrakit-agent-ssh` exported a library surface to the controller | binary crate has no public library surface; controller never depends on it |

Additionally, `controller-runtime/Cargo.toml` feature flag chains that forward to the binary
crate must be re-pointed to the runtime crate:

```toml
# Before:
interactive = ["uptrakit-web-api/interactive", "uptrakit-agent-runtime?/interactive", "uptrakit-agent-ssh?/interactive"]
reset-data  = ["uptrakit-web-api/reset-data", "uptrakit-agent-ssh?/reset-data"]

# After:
interactive = ["uptrakit-web-api/interactive", "uptrakit-agent-runtime?/interactive", "uptrakit-agent-ssh-runtime?/interactive"]
reset-data  = ["uptrakit-web-api/reset-data", "uptrakit-agent-ssh-runtime?/reset-data"]
```

The `dep:uptrakit-agent-ssh` optional dep entry in `controller-runtime/Cargo.toml` is removed;
`embedded-ssh-agent` feature becomes `["dep:uptrakit-agent-core", "dep:uptrakit-agent-ssh-runtime", "dep:base64"]`.

---

## Work Stream 2 — Migration Strategy

### Standalone `agent-ssh`

Unchanged. The 13 migrations (now in `agent-ssh-runtime/src/db/migration/`) run against the
agent's own local SQLite file, tracked in that file's `seaql_migrations`. Migration names and
order are preserved.

### Embedded (controller) — removal of duplicated migration

1. **Delete** `shared-db/src/migration/m20260331_000001_ssh_agent_tables.rs` and its entry in
   `Migrator::migrations()`.

2. **Add repair migration** `m20260331_000002_agent_ssh_migration_history_repair` to the
   `shared-db` migration chain:

   ```text
   up():
     -- Note on transaction safety: SeaORM's migration runner wraps up() in its own
     -- outer transaction before calling the method. begin_with_options(Immediate) inside
     -- up() would open a nested SAVEPOINT, which is always deferred in SQLite regardless
     -- of the mode flag — not a true BEGIN IMMEDIATE. The actual protection against a
     -- read-check / write race here is the ON CONFLICT DO NOTHING guard: the INSERT is
     -- unconditional; duplicates are silently skipped. The DELETE is also safe because
     -- it is a single-row keyed write. No BEGIN IMMEDIATE wrapper is needed.
     if seaql_migrations contains 'm20260331_000001_ssh_agent_tables':
       INSERT INTO seaql_migrations (version, applied_at) VALUES
         ('m20260215_000001_initial',                      now()),
         ('m20260222_000002_add_machine_id',               now()),
         ('m20260224_000003_add_sudo_columns',             now()),
         ('m20260302_000001_convert_ssh_host_timestamps',  now()),
         ('m20260302_000002_ensure_machine_id_nullable',   now()),
         ('m20260310_000001_data_encryption_keys',         now()),
         ('m20260306_000001_add_pve_columns',              now()),
         ('m20260307_000001_add_pve_node_name',            now()),
         ('m20260307_000002_pending_proxmox_matches',      now()),
         ('m20260308_000003_ssh_host_uuid_columns',        now()),
         ('m20260313_000001_drop_ssh_host_is_pve_node',    now()),
         ('m20260322_000001_ssh_hosts_lower_name_index',   now()),
         ('m20260507_000001_add_routeros_host_config',     now())
       ON CONFLICT DO NOTHING;
       DELETE FROM seaql_migrations
         WHERE version = 'm20260331_000001_ssh_agent_tables';
     -- if not found: no-op (fresh install or standalone agent-ssh DB)
   ```

   The 13th migration `m20260507_000001_add_routeros_host_config` must be included in the INSERT
   list. Omitting it would cause SeaORM to re-run the RouterOS table creation against the
   controller's shared DB on upgrade, creating an out-of-place table.

   **Frozen-list constraint:** The INSERT list is a snapshot frozen at the time this repair
   migration is written. No new `agent-ssh` migrations may be added between writing the repair
   migration and shipping it to production. If a new agent-ssh migration lands in the same
   release, it must be added to the INSERT list before the release cuts. This constraint must
   also be recorded in the ADR to prevent future contributors from unknowingly violating it.

   This runs before the 13 service migrations are contributed via `service_migrations()`, so
   SeaORM sees them as already recorded and skips their `up()` on existing deployments.

3. **Controller startup** calls `AgentSshHandler::service_migrations()` and passes the result
   into `run_migrations_with_plugins`. On fresh installs the 13 migrations run normally. On
   existing deployments the repair migration marks them as already applied.

### Future SSH agent schema changes

New migrations go in `agent-ssh-runtime/src/db/migration/`, appended to `service_migrations()`.
`shared-db` is never touched for agent-ssh schema again.

---

## Work Stream 3 — `scheduler-engine` Merge

`crates/shared/scheduler-engine/` is merged into `crates/core/scheduler-runtime/`. No behaviour
changes.

`scheduler-runtime` exports one `ServiceHandler` implementation (`SchedulerHandler`, which
already exists as `StandaloneSchedulerHandler`) after absorbing the engine.

### Steps

1. Move all files from `scheduler-engine/src/` into `scheduler-runtime/src/`.
2. `scheduler-runtime/Cargo.toml` absorbs all `scheduler-engine` dependencies; preserves `oidc`
   feature flag.
3. Delete `crates/shared/scheduler-engine/` and remove from workspace `Cargo.toml`.
4. Update imports:
   - `controller-runtime/src/scheduler/mod.rs`: `uptrakit_scheduler_engine::` →
     `uptrakit_scheduler_runtime::`
   - `controller-runtime/Cargo.toml` `embedded-scheduler` feature: remove
     `dep:uptrakit-scheduler-engine`; `oidc` feature: remove `uptrakit-scheduler-engine?/oidc`
   - `scheduler/Cargo.toml`: remove `dep:uptrakit-scheduler-engine`; update `oidc` feature from
     `["uptrakit-scheduler-engine/oidc", "uptrakit-scheduler-runtime/oidc"]` to
     `["uptrakit-scheduler-runtime/oidc"]` (not a deletion — the feature must still forward to
     the runtime or `cargo check --no-default-features --features oidc` will fail)
   - `web-api-queries/Cargo.toml`: remove dead `uptrakit-scheduler-engine` dep
   - Root `Cargo.toml` `[workspace.dependencies]`: remove `uptrakit-scheduler-engine` entry

---

## Work Stream 4 — `mqtt` and `agent` (Confirmation)

### `mqtt`

Already satisfies the contract. `mqtt-runtime` exports `MqttRuntime`; the binary's
`StandaloneMqttHandler` is the sole `ServiceHandler` wrapper. After this spec: move
`StandaloneMqttHandler` from `mqtt/src/main.rs` into `mqtt-runtime` and rename to `MqttHandler`,
making the binary a minimal shell. No migrations; `service_migrations()` uses the default
`vec![]`.

The rename happens as part of this WS4 implementation. The prerequisite has already merged
with `StandaloneMqttHandler` intact, so no transitional alias is needed — rename directly.

### `agent`

Already satisfies the contract: no DB, `AgentRuntime` in `agent-runtime`, 127-line `main.rs`.
No changes required.

---

## Work Stream 5 — Controller Unified Embedded Path

**Prerequisite shipped — now actionable.**

`run_embedded_service::<H>` is in service-sdk and `ControllerConnection` is transport-agnostic:

- `controller-runtime/src/ssh_agent/mod.rs`'s `run_embedded_ssh_agent` free function is deleted.
- Controller constructs `AgentSshHandler::new(shared_db, ...)` and calls
  `run_embedded_service::<AgentSshHandler>(transport, tokens, ...)`.
- Same pattern for embedded mqtt, scheduler.
- The transitional `uptrakit_agent_ssh_runtime::service_migrations()` free function is removed;
  controller calls `AgentSshHandler::service_migrations()` directly.

---

## Documentation Deliverables

| Deliverable                                        | Action                                                                                        |
| -------------------------------------------------- | --------------------------------------------------------------------------------------------- |
| `docs/adr/0005-service-binary-runtime-boundary.md` | New ADR: contract, migration strategy (B+B1 over tombstone), prerequisite split               |
| `docs/development/coding-standards.md`             | Add section: "Service binary/runtime boundary" — invariant, what belongs where, reference ADR |
| `CONTEXT.md`                                       | No changes needed; Embedded Mode already defined                                              |

ADR numbering context: 0001 = web-api decomposition, 0002 = routeros non-POSIX probe,
0003 = controller-core-boundary, 0004 = service-handler transport abstraction (prerequisite),
0005 = this spec.

---

## Quality Gates

All existing quality gates apply. Additionally verify:

- `cargo check --all-features` passes with `uptrakit-agent-ssh` no longer a library dep of the controller
- `cargo test --all-features` includes repair migration test: seed `m20260331_000001_ssh_agent_tables` in
  `seaql_migrations`, run migrations, assert the 13 rows are present and the old row is gone
- `cargo deny check` — workspace clean after `scheduler-engine` deletion
- Controller embedded integration tests pass with the new migration sequence

---

## Non-Goals

- The service-sdk embedded transport abstraction (separate prerequisite spec)
- Changing any external-facing behaviour (wire protocol, REST API, CLI flags)
- Moving shared infrastructure (service-sdk, wire, shared-types) into service crates
- Splitting `agent-ssh-runtime` further (e.g. separate `agent-ssh-db` crate)
- Applying this pattern to plugin crates
- Dynamic (runtime-registered) service migration contribution; `service_migrations()` is a
  compile-time static dispatch — the set of embedded services is fixed at compile time and
  called directly on the concrete handler type. This is intentional; all current embedded
  services are known at compile time and this keeps the interface simple.
