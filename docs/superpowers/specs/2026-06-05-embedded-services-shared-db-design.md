# Embedded Services — Shared Database Connection

**Date:** 2026-06-05
**Status:** Approved

## Problem

When Agent-SSH runs embedded inside `controller-standalone`, it should share the controller's
existing database connection. The `AgentSshHandler` constructor already receives
`app_state.db().clone()` — that part is correct (ADR-0005). However, six production call sites
inside `agent-ssh-runtime` call `crate::db::init_db(&state_dir)` directly, opening a second
SQLite connection to a separate `agent-ssh.db` file in the state directory. Surface actions
(bootstrap, sync, infra plugin orchestration) spawn tasks that use this separate connection,
which in embedded mode is either empty or nonexistent — breaking those workflows entirely.

Additionally, two SSH-specific startup steps are missing in the embedded path:
`register_ssh_column_aad` (registers the `ssh_hosts.private_key` column AAD — the controller's
Phase 4b only registers its own columns) and `reencrypt_ssh_to_v3` (re-encrypts SSH private
keys to ENC:v3 — the controller's Phase 4d only handles controller tables). Without these,
SSH private key reads and writes will use a wrong or absent AAD context, and legacy SSH host
records will not be upgraded on startup.

## Relationship to ADR-0005

ADR-0005 states: "Handler constructors must not hardcode internal paths or open their own DB
connections." This spec extends that invariant from constructors to all spawned tasks inside
the runtime — the same principle applied consistently. No new ADR is warranted; this is an
implementation fix, not a new architectural decision. ADR-0005's "Consequences" section should
be amended to record that the invariant covers spawned tasks, not only constructors.

The SSH schema tables (`ssh_hosts`, `data_encryption_keys`, `pending_proxmox_match`,
`routeros_host_config`) are already present in the controller's database in embedded mode:
`controller-runtime/src/migration/mod.rs` calls `AgentSshHandler::service_migrations()` and
passes the result to `run_migrations_with_plugins`, which applies them at startup.

## Non-Goals

- No automated migration of existing `agent-ssh.db` files. The embedded surface-action path
  was broken (tasks opened an empty DB on surface-action requests), so accumulated SSH host
  data via the embedded path is unlikely. Document in CHANGELOG; add a `tracing::warn!` in
  `register_agent_ssh` if `state_dir/agent-ssh.db` exists and is non-empty (detectable via
  file size or a quick `PRAGMA user_version` read).
- No changes to the standalone `uptrakit-agent-ssh` binary path. `db::init_db` stays, called
  only from `agent-ssh/src/main.rs` and `agent-ssh/src/host_cli.rs`.

## Tradeoffs

**SQLite pool contention**: SSH surface actions (bootstrap, sync) are write-heavy and
long-running. After this change they join the controller's shared SQLite pool instead of using
an independent connection. The controller's pool is configured with `busy_timeout = 5000 ms`
and `BEGIN IMMEDIATE` for read-then-write paths; SSH surface actions follow the same rules.
This is an accepted tradeoff — SSH bootstrap is a rare, human-driven UI action, not a
background workload. Monitor for write-lock stalls if this becomes a concern.

**Pre-existing `BEGIN IMMEDIATE` gap in `init_ssh_data_key_ring`**: this function performs a
read-then-write (check DEK existence, insert if absent) without a `BEGIN IMMEDIATE` transaction.
This is a pre-existing issue in the standalone path; calling it from the shared pool in
embedded mode does not make it worse but it is worth a follow-up fix in that function.

## Design

### Part 1 — Thread `DatabaseConnection` through spawned tasks

Six production `init_db` call sites in `agent-ssh-runtime` are replaced by cloning the
existing connection from context. `DatabaseConnection::clone()` is a cheap `Arc<Pool>` clone.
The `surface_runtime/` subdirectory also contains two orphaned `init_db` calls but is dead
code (see Affected Files) — those are removed via deletion.

**Spawn sites** — add `let db = ctx.db.clone()` before each `tokio::spawn`, pass `db` into
the closure. Three spawn sites in the monolithic `surface_runtime.rs` call `init_db` directly;
two more spawn sites delegate through private wrappers to `operations/bootstrap.rs` (see below):

| File                 | Function                    | How `db` flows                                                           |
| -------------------- | --------------------------- | ------------------------------------------------------------------------ |
| `surface_runtime.rs` | `spawn_infra_plugin_action` | direct `init_db` call in spawn closure                                   |
| `surface_runtime.rs` | `spawn_sync_connect`        | direct `init_db` call in spawn closure                                   |
| `surface_runtime.rs` | `spawn_sync_execute`        | direct `init_db` call in spawn closure                                   |
| `surface_runtime.rs` | `spawn_bootstrap_connect`   | via `run_bootstrap_connect` (same file) → `bootstrap::bootstrap_connect` |
| `surface_runtime.rs` | `spawn_bootstrap_execute`   | via `run_bootstrap_execute` (same file) → `bootstrap::bootstrap_execute` |

**Private wrappers in `surface_runtime.rs`** — these sit between the spawn closures and the
operations functions; add `db: &sea_orm::DatabaseConnection` to each, and add `db` to
`BootstrapExecuteArgs`:

| Function                | Change                                                                      |
| ----------------------- | --------------------------------------------------------------------------- |
| `run_bootstrap_connect` | add `db: &sea_orm::DatabaseConnection` param; thread to `bootstrap_connect` |
| `BootstrapExecuteArgs`  | add `db: sea_orm::DatabaseConnection` field                                 |
| `run_bootstrap_execute` | thread `&args.db` through to `bootstrap_execute`                            |

**Operations functions** — add `db: &DatabaseConnection` parameter, remove internal
`init_db` call. `state_dir` parameter is kept; these functions use it for SSH key files and
sudoers, not only for DB access:

| File                              | Function                     | Notes                                                           |
| --------------------------------- | ---------------------------- | --------------------------------------------------------------- |
| `operations/bootstrap.rs`         | `bootstrap_connect`          | called by `run_bootstrap_connect` in `surface_runtime.rs`       |
| `operations/bootstrap.rs`         | `bootstrap_execute`          | called by `run_bootstrap_execute` in `surface_runtime.rs`       |
| `operations/bootstrap_proxmox.rs` | `load_and_validate_pve_host` | return type changes (see below)                                 |
| `operations/bootstrap_proxmox.rs` | `proxmox_bootstrap_connect`  | calls `load_and_validate_pve_host`; needs `db` threaded through |
| `operations/bootstrap_proxmox.rs` | `proxmox_bootstrap_execute`  | uses DB from `load_and_validate_pve_host` for subsequent ops    |
| `operations/bootstrap_proxmox.rs` | `run_proxmox_bootstrap`      | gains `db: &DatabaseConnection`; passes to connect + execute    |

`load_and_validate_pve_host` previously returned `(DatabaseConnection, ssh_host::Model)` —
after the change it returns only `ssh_host::Model` because callers supply the connection.
`proxmox_bootstrap_connect` and `proxmox_bootstrap_execute` are updated accordingly.

**`AgentGuestBootstrapExecutor`** — add `db: DatabaseConnection` field. Constructed inside
`spawn_infra_plugin_action` which already has the cloned connection. Its `bootstrap_guest`
method calls `run_proxmox_bootstrap` with `&self.db` so the DB flows through the entire
Proxmox bootstrap chain.

All callers of the changed operations functions are inside `agent-ssh-runtime`. The standalone
binary path calls `db::init_db` in `main.rs`/`host_cli.rs` and passes the result in;
it is unaffected.

### Part 2 — Embedded SSH agent startup initialisation

Two SSH-specific startup steps that the standalone binary performs but the embedded path
skips must be added. A third call (`init_ssh_data_key_ring`) is included for symmetry but is
a no-op in embedded mode — the controller's Phase 4c already initializes the same key ring and
the call will emit a harmless `warn!` "data key ring already initialized":

They are called at the top of `register_agent_ssh` in
`controller-runtime/src/service_host/builtins.rs`, before `AgentSshHandler::new`:

```rust
#[cfg(feature = "embedded-ssh-agent")]
pub(crate) async fn register_agent_ssh(
    host: &BuiltinServiceHost,
    app_state: &Arc<uptrakit_web_api::AppState>,
    ...
) -> rootcause::Result<()> {
    uptrakit_agent_ssh_runtime::register_ssh_column_aad();
    uptrakit_agent_ssh_runtime::init_ssh_data_key_ring(app_state.db()).await;
    uptrakit_agent_ssh_runtime::reencrypt_ssh_to_v3(app_state.db()).await;

    let db_for_ssh = app_state.db().clone();
    let handler = uptrakit_agent_ssh_runtime::AgentSshHandler::new(db_for_ssh, state_dir);
    // ... existing registration logic unchanged
}
```

No new `#[cfg]` blocks in `lib.rs`. The single existing `#[cfg(feature = "embedded-ssh-agent")]`
gate on `register_agent_ssh` covers all three calls. All SSH-specific knowledge stays in
the `builtins.rs` integration seam — consistent with how `register_scheduler`,
`register_agent`, and `register_mqtt` each own their service's registration details.

The three functions are already `pub` in `agent-ssh-runtime/src/lib.rs` and each guards
itself with `if !uptrakit_crypto::master_key_available() { return; }`, so calling them in
embedded mode is safe even when encryption is not configured. `init_ssh_data_key_ring` is
idempotent: it reads existing DEK rows before inserting and catches `AlreadyInitialized` from
`uptrakit_crypto` with `tracing::warn!`, so the expected "already initialized" warn is benign.

## Affected Files

**`agent-ssh-runtime`** (internal refactoring, no public API break):

- `src/surface_runtime.rs`
- `src/operations/bootstrap.rs`
- `src/operations/bootstrap_proxmox.rs`
- `src/operations/bootstrap_routeros.rs` — no `init_db` calls; no changes required
- `src/surface_runtime/` subdirectory — dead code (no `mod.rs`; `lib.rs` resolves
  `pub mod surface_runtime` to the monolithic `.rs` file). Contains 9+ files including
  `infra_plugin_orchestration.rs` and `sync.rs` which have their own `init_db` calls.
  Delete the **entire directory** as part of this change.

**`controller-runtime`** (one function, additive):

- `src/service_host/builtins.rs`

## Quality Gates

Standard gates apply (`cargo fmt`, `cargo check`, `cargo clippy`, `cargo test --all-features`).
No Docker tests triggered (no enrollment/wire/service-lifecycle changes).

## Documentation

CHANGELOG: note that existing `agent-ssh.db` files in the state directory of
`controller-standalone` deployments are no longer used. SSH host data must be migrated
manually if needed (see DB entities in `agent-ssh-runtime/src/db/entity/`).

`CONTEXT.md`: update the **Embedded Mode** glossary entry to note that embedded Services share
the controller's `DatabaseConnection` rather than opening their own.

`docs/adr/` (ADR-0005): amend the Consequences section to record that the "no own DB
connections" invariant extends to spawned tasks, not only handler constructors.
