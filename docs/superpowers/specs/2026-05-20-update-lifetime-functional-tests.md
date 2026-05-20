# Update Lifetime Functional Tests

**Date:** 2026-05-20
**Status:** Spec

## Background

The controller update dispatch pipeline wires together Proxmox pre-update
protection (snapshot/backup) and resource scaling (pre-update scale-up,
post-update restore) around the agent dispatch step. These plugins are tested
in isolation inside the Proxmox crate (MockDatabase + httpmock). No test
currently verifies that the **orchestration** in `run_protection_and_dispatch`
calls the plugins at the right lifecycle points with the right sequence.

This spec adds a `functional-tests` crate that tests the orchestration
end-to-end with real in-memory SQLite, a real Proxmox plugin instance hitting an
`httpmock` server, and a stub agent sink.

## Goals

1. Verify Proxmox **snapshot API** is called before agent dispatch when protection
   mode is `Snapshot`.
2. Verify Proxmox **backup API** is called before agent dispatch when protection
   mode is `Backup`.
3. Verify Proxmox **resource scale-up API** (GET config + PUT config) is called
   before agent dispatch when scaling is configured.
4. Verify the dispatched `ExecuteUpdate` wire payload contains correct shell
   plugin assignments and `to_version`.
5. Verify Proxmox **resource restore API** (PUT config) is called when
   `finalize_post_update_hook` runs.
6. Verify dispatch proceeds normally (no API calls) when no Proxmox mapping
   exists for the host.

## Non-goals / Deferred

- Agent-side execution (shell plugin `VersionDetector`, `UpdateExecutor`) — covered
  by existing unit tests in the shell crate.
- `finalize_post_update` for protection (no Proxmox API calls; returns only a
  recovery hint string).
- LXC container variants (QEMU qemu only in this spec; LXC can be added later).
- Batch update path.
- Queued (agent-offline) path.

## New Crate: `crates/core/functional-tests`

Non-publishable crate alongside `crates/core/integration-tests`.

```text
crates/core/functional-tests/
├── Cargo.toml
└── tests/
    ├── support/
    │   ├── mod.rs
    │   ├── db.rs          # SQLite + migrations
    │   ├── fixtures.rs    # row insertion + PendingProtectionWork builder
    │   └── stubs.rs       # StubPluginOps, StubNotificationState, NoopOutputStream
    └── proxmox_update_lifecycle.rs   # all 6 test cases
```

### `Cargo.toml`

```toml
[package]
name = "uptrakit-functional-tests"
description = "Uptrakit functional tests for controller orchestration"
edition.workspace = true
version = "0.0.1"
license.workspace = true
authors.workspace = true
repository.workspace = true

[dev-dependencies]
uptrakit-controller-core                = { workspace = true, features = ["testing", "plugin-ops"] }
uptrakit-web-api-queries                = { workspace = true, features = ["plugin-ops"] }
uptrakit-plugin-infrastructure-proxmox  = { workspace = true, features = ["migration", "plugin-ops", "db-sqlite"] }
uptrakit-plugin-infrastructure-registry = { workspace = true, features = ["plugin-ops"] }
uptrakit-plugin-infrastructure-core     = { workspace = true, features = ["plugin-ops", "migration"] }
uptrakit-shared-db                      = { workspace = true, features = ["migration"] }
uptrakit-shared-types                   = { workspace = true }
uptrakit-wire                           = { workspace = true }
uptrakit-tenant-db                      = { workspace = true }
uptrakit-crypto                         = { workspace = true, features = ["testing"] }
sea-orm                                 = { workspace = true, features = ["sqlx-sqlite"] }
sea-orm-migration                       = { workspace = true }
httpmock                                = { workspace = true }
async-trait                             = { workspace = true }
rootcause                               = { workspace = true }
serde_json                              = { workspace = true }
time                                    = { workspace = true }
tokio                                   = { workspace = true, features = ["macros", "rt-multi-thread", "sync"] }
uuid                                    = { workspace = true, features = ["v7"] }

[lints]
workspace = true
```

## `release-plz.toml` Change

Add a stanza identical to `uptrakit-integration-tests` — no publish, no release:

```toml
# Functional tests are runtime-only verification.
[[package]]
name = "uptrakit-functional-tests"
release = false
```

The workspace `Cargo.toml` default is `publish = ["uptrakit-private"]`; the
`release-plz.toml` global `publish = false` already prevents crates.io publishing
for the whole workspace. `release = false` additionally suppresses changelog
generation and git tags for this crate, matching the integration-tests pattern.

## `controller-core` Change: Expose `run_protection_and_dispatch`

Two-step change in `crates/ui/controller-core/src/update/controller.rs`:

1. Change the function from `async fn run_protection_and_dispatch(...)` to
   `pub(crate) async fn run_protection_and_dispatch(...)`.
2. Re-export as `pub` from `lib.rs` behind the `testing` feature gate:

```rust
// lib.rs
#[cfg(feature = "testing")]
pub mod testing {
    pub use crate::update::controller::run_protection_and_dispatch;
}
```

`run_protection_and_dispatch` signature stays unchanged; only visibility widens
behind the feature gate.

## Test Support Helpers

### `support/db.rs` — SQLite + Migrations

```rust
pub async fn setup_test_db() -> DatabaseConnection
```

1. Call `uptrakit_crypto::enable_plaintext_mode()` once (required for any code path
   that touches `EncryptedString` columns, even indirectly).
2. Open in-memory SQLite via `sea_orm::Database::connect("sqlite::memory:")`.
3. Run `uptrakit_shared_db::run_migrations_with_plugins(&db,
uptrakit_plugin_infrastructure_proxmox::ProxmoxPlugin::controller_migrations())
.await` — runs all core tables (tenant, host, service, software_item,
   host_software_item, host_software_item_plugin, plugin_config, update_history,
   update_output_line, etc.) combined with the proxmox-specific tables
   (proxmox_host_mapping, proxmox_protection_default, proxmox_protection_item_override,
   proxmox_protection_audit, proxmox_resource_scaling_record, proxmox_scaling_default,
   proxmox_scaling_item_override) in a single `CombinedMigrator` pass. Do NOT call
   core migrations and proxmox migrations in two separate steps — the migrator
   uses a `thread_local` combiner; two passes would double-run the schema.

### `support/fixtures.rs` — Row Insertion

`TestFixtures` struct holds all IDs. `TestFixtures::insert(db, proxmox_api_url)` inserts:

| Table                       | Notes                                                                                                     |
| --------------------------- | --------------------------------------------------------------------------------------------------------- |
| `tenant`                    | default tenant                                                                                            |
| `host`                      | `machine_id = "test-machine"`, active                                                                     |
| `service`                   | type Agent, status Approved, linked to tenant                                                             |
| `service_host`              | links service → host                                                                                      |
| `software_item`             | active                                                                                                    |
| `host_software_item`        | `installed_version = "1.0.0"`                                                                             |
| `plugin_config` (shell)     | `config_payload = {"update_command":"echo ok","version_command":"echo 1.0.0"}`                            |
| `plugin_config` (proxmox)   | `config_payload = {"api_url": "<httpmock_url>", "api_token": "root@pam!tok=secret", "verify_tls": false}` |
| `host_software_item_plugin` | execute_update role → shell config                                                                        |
| `host_software_item_plugin` | detect_version role → shell config                                                                        |
| `update_history`            | status = Pending                                                                                          |

Per-test additional rows (inserted by individual tests):

- `proxmox_host_mapping` — `node="pve1"`, `vmid=100`, `type="qemu"`
- `proxmox_protection_default` — per test scenario
- `proxmox_scaling_default` — per test scenario
- `proxmox_backup_target_cache` — Test 2 only: `target_key="pve1:storage1:dir"`, `storage_id="storage1"`
  (the backup path calls `find_cached_backup_target(plugin_config_id, target_key)` and returns
  failure without making any HTTP call if no matching cache row exists)

`TestFixtures::pending_work(&self, to_version: &str) -> PendingProtectionWork` constructs
`ValidatedUpdateTarget` from ORM model structs built directly (no DB query), populates
`PendingProtectionWork` with `update_history_id`, `to_version`, `interactive=false`.

The `ValidatedUpdateTarget` fields:

- `item`: `software_item::Model` with correct IDs
- `host`: `host::Model` with correct IDs
- `hsi_link`: `host_software_item::Model`
- `agent`: `service::Model` (Approved)
- `execute_update_data`: `(host_software_item_plugin::Model, Option<plugin_config::Model>)` — shell assignment + shell config
- `detect_version_data`: `Some((host_software_item_plugin::Model, Option<plugin_config::Model>))` — wrapped in `Option`; same shell config row
- `fetch_releases_config`: `None`
- `pre_update_hook_plugins`: `[]`
- `post_update_hook_plugins`: `[]`

### `support/stubs.rs` — Stub Implementations

**`StubPluginOps`**

Implements all `PluginOps` sub-traits. No-op methods must return appropriate
typed defaults — never `todo!()` or `unimplemented!()` (workspace deny-level
lints). Trait methods returning `Result<T>` return `Ok(Default::default())` or
`Ok(None)`; methods returning `()` have empty bodies.

Overrides:

- `controller_update_protection()` → `Some(ControllerUpdateProtectionPlugin::create(&catalog_config)?)` where `CatalogConfig` holds the proxmox `plugin_config_id`
- `controller_update_hook()` → `Some(ControllerUpdateHookPlugin::create(&catalog_config)?)` same config

`ControllerUpdateHookPlugin` and its `create()` are currently `pub(crate)` in the
proxmox crate. Before `StubPluginOps` can call them from an external crate, their
visibility must be widened to `pub` (gated on `plugin-ops` feature, matching the
existing pattern used by `ControllerUpdateProtectionPlugin`).

For tests that don't need Proxmox (e.g., "no mapping" test), both methods return
`None`.

**`TestNotificationSetup`**

`NotificationService` is a concrete struct — it cannot be replaced with a stub.
To capture the dispatched `ExecuteUpdate` payload and ensure `is_connected` returns
`true` for the agent, register a real tokio channel in `ServiceConnectionRegistry`
before calling `run_protection_and_dispatch`:

```rust
pub struct TestNotificationSetup {
    pub notification_state: NotificationState,
    pub message_rx: mpsc::Receiver<ControllerMessage>,
}

impl TestNotificationSetup {
    pub async fn new(agent_service_id: Uuid) -> Self {
        let registry = ServiceConnectionRegistry::new();
        // register() creates the channel internally and returns the receiver
        let (message_rx, _handle) = registry
            .register(agent_service_id, BTreeSet::new(), None, None, None)
            .await;
        // controller_id can be any UUID in tests
        let notification_service = NotificationService::new(registry, Uuid::now_v7());
        let (dispatcher, _event_rx) = NotificationDispatcher::test_channel();
        let event_broadcaster = EventBroadcaster::new();
        Self {
            notification_state: NotificationState::new(
                notification_service, dispatcher, event_broadcaster,
            ),
            message_rx,
        }
    }

    pub fn captured_messages(&mut self) -> Vec<ControllerMessage> {
        let mut msgs = vec![];
        while let Ok(m) = self.message_rx.try_recv() {
            msgs.push(m);
        }
        msgs
    }
}
```

`ServiceConnectionRegistry::register` is async and creates its own internal
channel — never inject a sender manually. `NotificationDispatcher::test_channel()`
returns `(Self, mpsc::Receiver<NotificationEvent>)`; discard the second element
with `_event_rx` if notification events are not asserted on.

**`NoopOutputStream`**

Implements `UpdateOutputStream`, all methods have empty bodies.

## Test Cases

All tests: `#[tokio::test]` (no `start_paused` — SQLite connection pool uses
internal Tokio timers).

### Synchronization note

`run_protection_and_dispatch` may spawn fire-and-forget output-forwarding tasks
internally. These do not affect the assertions in this spec — `protection_status`
and `proxmox_resource_scaling_record` are written by directly-awaited calls
(`prepare_pre_update_protection`, `prepare_pre_update_hook`) before any spawn.
Do NOT add assertions on `update_output_line` rows without explicit
synchronization (e.g. awaiting the returned `JoinHandle` if one is exposed).

### CAS sentinel assertion

Every test that calls `run_protection_and_dispatch` must include as its first
assertion:

```rust
let updated = update_history::Entity::find_by_id(fixtures.update_history_id)
    .one(&db).await.unwrap().unwrap();
assert_eq!(updated.status, UpdateStatus::InProgress,
    "CAS Pending→InProgress failed: run_protection_and_dispatch exited early");
```

This catches the case where a wrong fixture status or a connection check failure
causes the function to return silently before doing any work, which would make
all subsequent mock and DB assertions meaningless.

---

### Test 1: `snapshot_protection_and_scaling_before_dispatch`

**Setup:**

- Full fixtures, including proxmox mapping
- `proxmox_protection_default`: mode = `snapshot`, timeout = default
- `proxmox_scaling_default`: mode = delta, `delta_cores = 2`, `delta_memory_mb = 1024`

**httpmock expectations:**

```text
POST /api2/json/nodes/pve1/qemu/100/snapshot
  → 200 {"data": "UPID:pve1:001:snapshot"}

GET  path_contains("/tasks/")  AND  path_contains("/status")
  → 200 {"data": {"status": "stopped", "exitstatus": "OK"}}
  (matches dynamic UPID; registered before POST so httpmock routes correctly)

GET /api2/json/nodes/pve1/qemu/100/config
  → 200 {"data": {"cores": 2, "memory": 2048, "hotplug": "cpu,memory"}}

PUT /api2/json/nodes/pve1/qemu/100/config
  → 200 {"data": null}
```

**Assertions:**

- CAS sentinel (see above)
- Snapshot POST mock: `assert_calls(1)`
- Task-status GET mock: `assert_calls(1)` (mock returns `"stopped"` immediately → exactly one poll)
- Scale GET mock: `assert_calls(1)`
- Scale PUT mock: `assert_calls(1)`
- `TestNotificationSetup::captured_messages()` contains exactly 1 message
- That message is `ControllerMessage::ExecuteUpdate(payload)` where `payload.to_version == "2.0.0"`
- DB: `update_history.pre_update_protection_status == Some("protected")`
- DB: one `proxmox_resource_scaling_record` row with `restore_status == "pending"`

---

### Test 2: `backup_protection_before_dispatch`

**Setup:**

- Full fixtures, including proxmox mapping
- `proxmox_protection_default`: mode = `backup`, `backup_target_key = "pve1:storage1:dir"`
  (composite format `"{node}:{storage_id}:{storage_type}"` matching the cache lookup key)
- `proxmox_backup_target_cache` row: `plugin_config_id` → proxmox config, `target_key =
"pve1:storage1:dir"`, `storage_id = "storage1"` (the actual storage identifier
  sent as `storage=storage1` in the vzdump request body)
- No scaling configured

**httpmock expectations:**

```text
POST /api2/json/nodes/pve1/vzdump
  body contains: vmid=100&storage=storage1
  → 200 {"data": "UPID:pve1:002:backup"}

GET  path_contains("/tasks/")  AND  path_contains("/status")
  → 200 {"data": {"status": "stopped", "exitstatus": "OK"}}
```

**Assertions:**

- CAS sentinel (see above)
- Vzdump POST mock: `assert_calls(1)`
- Task-status GET mock: `assert_calls(1)` (mock returns `"stopped"` immediately → exactly one poll)
- No GET /config or PUT /config calls
- `ExecuteUpdate` dispatched
- DB: `update_history.pre_update_protection_status == Some("protected")`

---

### Test 3: `no_proxmox_mapping_dispatch_proceeds`

**Setup:**

- Full fixtures, **no** `proxmox_host_mapping` row
- No protection, no scaling default

**httpmock:** no expectations registered

**Assertions:**

- CAS sentinel (see above)
- httpmock server received 0 total requests
- `ExecuteUpdate` dispatched (captured 1 message)
- DB: `update_history.pre_update_protection_status == None` (`StubPluginOps` returns `None`
  for `controller_update_protection()` — no plugin runs, nothing writes the status column)

---

### Test 4: `do_nothing_protection_scaling_still_runs`

**Setup:**

- Full fixtures, proxmox mapping present
- `proxmox_protection_default`: mode = `do_nothing`
- `proxmox_scaling_default`: mode = absolute, `absolute_cores = 4`, `absolute_memory_mb = 4096`

**httpmock expectations:**

```text
GET /api2/json/nodes/pve1/qemu/100/config
  → 200 {"data": {"cores": 2, "memory": 2048, "hotplug": "cpu,memory"}}

PUT /api2/json/nodes/pve1/qemu/100/config
  → 200 {"data": null}
```

No snapshot or vzdump endpoints registered (any unexpected call → httpmock returns 404, PUT assertion will fail).

**Assertions:**

- CAS sentinel (see above)
- GET config mock: `assert_calls(1)`
- PUT config mock: `assert_calls(1)`
- `ExecuteUpdate` dispatched
- DB: `update_history.pre_update_protection_status == Some("skipped")`
- DB: `proxmox_resource_scaling_record` row inserted

---

### Test 5: `dispatch_payload_has_correct_plugin_assignments`

**Setup:**

- Full fixtures, no proxmox mapping (protection irrelevant here)
- Focus: verify the wire payload structure

**httpmock:** none

**Assertions on captured `ExecuteUpdate` payload (CAS sentinel applies here too):**

```rust
let payload = /* extract from captured ControllerMessage::ExecuteUpdate */;

assert_eq!(payload.to_version, "2.0.0");
assert_eq!(payload.software_item_id, fixtures.software_item_id);

let exec = &payload.execute_update_plugin;
assert_eq!(exec.plugin_type, "generic_shell");
assert_eq!(exec.config_id, fixtures.shell_config_id);

let detect = payload.detect_version_plugin.as_ref().expect("detect_version present");
assert_eq!(detect.plugin_type, "generic_shell");
assert_eq!(detect.config_id, fixtures.shell_config_id);
```

---

### Test 6: `post_update_resource_restore` (calls `finalize_post_update_hook` directly)

**Setup:**

- `setup_test_db()` + insert minimal rows (tenant, host, service, software_item,
  plugin_config (proxmox), proxmox_host_mapping)
- Insert `proxmox_resource_scaling_record` manually:
  - `update_history_id` = test UUID
  - `proxmox_node = "pve1"`, `proxmox_vmid = 100`, `proxmox_type = "qemu"`
  - `original_cores = Some(2)`, `original_memory_mb = 2048`
  - `scaled_cores = Some(4)`, `scaled_memory_mb = 4096`
  - `restore_status = "pending"`
  - `plugin_config_id` → the proxmox config row

**httpmock expectations:**

```text
PUT /api2/json/nodes/pve1/qemu/100/config
  body contains: cores=2&memory=2048
  → 200 {"data": null}
```

**Invocation:**

```rust
uptrakit_web_api_queries::queries::update_dispatch::finalize_post_update_hook(
    &db,
    Some(hook_plugin),
    &stub_plugin_ops as &dyn NotificationOps,  // StubPluginOps implements NotificationOps
    &update_history_record,
)
.await
.expect("finalize hook ok");
```

**Assertions:**

- PUT mock: `assert_calls(1)`, body contains `cores=2` and `memory=2048`
- DB: `proxmox_resource_scaling_record.restore_status == "restored"`

---

## Quality Gates

All existing quality gates apply. No new gates added. The `functional-tests`
crate must compile and pass under:

```sh
cargo check --all-features -p uptrakit-functional-tests
cargo test -p uptrakit-functional-tests
```

Tests are **not** `#[ignore]` — they run in normal `cargo test` without Docker.

## Documentation Impact

None. These are internal test infrastructure additions with no user-facing
behavior change. No `CONTEXT.md`, ADR, or API doc updates required.
