<!-- markdownlint-disable MD013 MD031 MD032 MD036 MD060 -->

# Update Lifetime Functional Tests Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `crates/core/functional-tests` — a new Rust crate that drives the controller's `run_protection_and_dispatch` orchestration end-to-end against in-memory SQLite + real `PluginCatalog` + `httpmock` and asserts Proxmox snapshot/backup/scaling lifecycle ordering with the dispatched wire payload.

**Architecture:** Six `#[tokio::test]` cases in a single `tests/proxmox_update_lifecycle.rs`, sharing `tests/support/{db,fixtures,stubs}.rs` helpers. Tests call `run_protection_and_dispatch` (re-exported via a new `pub mod testing` block behind `controller-core`'s existing `testing` feature) with a real `PluginCatalog` carrying the Proxmox descriptor. HTTP calls land on `httpmock`; the dispatched `ControllerMessage::ExecuteUpdate` is observed via a real `ServiceConnectionRegistry` channel.

**Tech Stack:** Rust 2021, SeaORM 1.x + sea-orm-migration, tokio (mpsc + macros + rt-multi-thread), httpmock, rootcause::Report, uptrakit workspace crates (`controller-core`, `web-api-queries`, `plugin-infrastructure-{core,proxmox,registry}`, `shared-db`, `wire`, `tenant-db`, `crypto`).

**Spec:** `docs/superpowers/specs/2026-05-20-update-lifetime-functional-tests.md`
**Standards snapshot:** `.superpowers/standards-snapshot.md`

---

## File Structure

| Path                                                             | Responsibility                                                       |
| ---------------------------------------------------------------- | -------------------------------------------------------------------- |
| `crates/core/functional-tests/Cargo.toml`                        | Crate manifest; dev-deps only, no library output                     |
| `crates/core/functional-tests/tests/support/mod.rs`              | Re-export `db`, `fixtures`, `stubs` for test files                   |
| `crates/core/functional-tests/tests/support/db.rs`               | `setup_test_db()` — SQLite in-memory + combined migrations           |
| `crates/core/functional-tests/tests/support/fixtures.rs`         | `TestFixtures` + `insert()` + `pending_work()`; per-test row helpers |
| `crates/core/functional-tests/tests/support/stubs.rs`            | `build_plugin_ops`, `TestNotificationSetup`, `NoopOutputStream`      |
| `crates/core/functional-tests/tests/proxmox_update_lifecycle.rs` | 6 test cases + module-level `#![expect(...)]` lint suppression       |
| `crates/ui/controller-core/src/lib.rs` (modify)                  | Add `#[cfg(feature = "testing")] pub mod testing { ... }`            |
| `crates/ui/controller-core/src/update/controller.rs` (modify)    | Widen `run_protection_and_dispatch` to `pub(crate)`                  |
| `Cargo.toml` (workspace, modify)                                 | Add `crates/core/functional-tests` to `members`                      |
| `.superpowers/pending-specs.md` (modify)                         | Register plan in tracker                                             |

`release-plz.toml` already contains the `uptrakit-functional-tests` stanza (added in commit `33e25b69a`); no change required.

**Binding rules applied throughout (from standards snapshot):**

- Error handling: `rootcause::Report`, `report!()`/`bail!()`, no `Result<T, String>`; `.context_to()?` at boundaries.
- Panic policy: `unwrap`/`expect`/`panic` are workspace deny — module-level `#![expect(...)]` with `reason = "..."` (never bare `#[allow]`) matching `crates/core/integration-tests/tests/helpers/api_client.rs` precedent.
- Locks: `parking_lot::Mutex` only in async code (no direct lock use expected in these tests).
- DB: SeaORM ActiveModel; FK constraints enforced in SQLite test DBs — insert parents before children. `enable_plaintext_mode()` before any `EncryptedString` touchpoint.
- Tests: never test upstream crate behavior; `#[tokio::test]` without `start_paused` (no tokio time API used).
- Lint suppression hygiene: `#![expect(lint, reason = "...")]` — never `#[allow]`.
- Conventional Commits required for every commit.

---

## Task 1: Crate scaffolding and workspace registration

**Files:**

- Create: `crates/core/functional-tests/Cargo.toml`
- Modify: `Cargo.toml` (workspace `members`)

- [ ] **Step 1: Create the crate directory and Cargo.toml**

Write `crates/core/functional-tests/Cargo.toml`:

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
uptrakit-plugin-infrastructure-proxmox  = { workspace = true, features = ["migrations", "plugin-ops", "db-sqlite"] }
uptrakit-plugin-infrastructure-registry = { workspace = true, features = ["plugin-ops"] }
uptrakit-plugin-infrastructure-core     = { workspace = true, features = ["plugin-ops", "migrations", "catalog"] }
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

- [ ] **Step 2: Register crate in workspace**

In root `Cargo.toml`, add `"crates/core/functional-tests"` to the `[workspace] members = [...]` list in alphabetical position next to the existing `"crates/core/integration-tests"` entry.

- [ ] **Step 3: Verify the crate is recognized**

Run: `cargo metadata --no-deps --format-version 1 | grep -o '"name":"uptrakit-functional-tests"'`
Expected: prints `"name":"uptrakit-functional-tests"`.

- [ ] **Step 4: Verify `cargo check` baseline (empty crate)**

Run: `cargo check -p uptrakit-functional-tests --all-features`
Expected: PASS (no targets yet — exit 0).

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/core/functional-tests/Cargo.toml
git commit -m "$(cat <<'EOF'
build(functional-tests): scaffold crate

Adds an empty `uptrakit-functional-tests` workspace member that will host
end-to-end orchestration tests against in-memory SQLite + httpmock.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Expose `run_protection_and_dispatch` via `pub mod testing`

**Files:**

- Modify: `crates/ui/controller-core/src/update/controller.rs` (function visibility)
- Modify: `crates/ui/controller-core/src/lib.rs` (new `pub mod testing` block)

- [ ] **Step 1: Widen function visibility**

In `crates/ui/controller-core/src/update/controller.rs`, change:

```rust
async fn run_protection_and_dispatch(
```

to:

```rust
pub(crate) async fn run_protection_and_dispatch(
```

The function signature, body, and `#[tracing::instrument(skip_all, fields(update_id = %work.update_history_id))]` attribute are unchanged.

- [ ] **Step 2: Add `pub mod testing` re-export with module-level docstring**

Append to `crates/ui/controller-core/src/lib.rs` (after the last existing top-level item):

```rust
/// Test-only re-exports for in-tree functional tests.
///
/// Items exposed here are gated on `feature = "testing"` and exist solely so
/// the in-tree `uptrakit-functional-tests` crate can drive controller
/// orchestration end-to-end. They are **not** part of the stable public API:
/// signatures, naming, and contract may change without semver impact, and
/// out-of-tree callers are unsupported.
#[cfg(feature = "testing")]
pub mod testing {
    pub use crate::update::controller::run_protection_and_dispatch;
}
```

- [ ] **Step 3: Verify both feature builds compile**

Run in parallel:

```bash
cargo check -p uptrakit-controller-core --no-default-features --features db-sqlite
cargo check -p uptrakit-controller-core --all-features
```

Expected: both PASS.

- [ ] **Step 4: Verify the re-export resolves from a downstream crate**

Run: `cargo check -p uptrakit-functional-tests --all-features`
Expected: PASS (still no test targets in the new crate).

- [ ] **Step 5: Commit**

```bash
git add crates/ui/controller-core/src/lib.rs crates/ui/controller-core/src/update/controller.rs
git commit -m "$(cat <<'EOF'
feat(controller-core): expose run_protection_and_dispatch under testing feature

Adds a `pub mod testing` block behind the existing `testing` feature flag and
widens `run_protection_and_dispatch` to `pub(crate)`. Lets the in-tree
`uptrakit-functional-tests` crate drive the orchestrator end-to-end. The
module docstring records the test-only / non-semver contract.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: `support/db.rs` — SQLite + combined migrations

**Files:**

- Create: `crates/core/functional-tests/tests/support/mod.rs`
- Create: `crates/core/functional-tests/tests/support/db.rs`

Snapshot rules applied: `enable_plaintext_mode()` precondition for `EncryptedString` columns; FK constraints enforced by SQLite; combined migration pattern (one `run_migrations_with_plugins` call, not two).

- [ ] **Step 1: Write the failing test (in the eventual lifecycle file)**

Create `crates/core/functional-tests/tests/proxmox_update_lifecycle.rs` with the lint suppression block and a smoke test that drives only `setup_test_db`:

```rust
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "functional test infrastructure: panics acceptable in test helpers and assertions"
)]

mod support;

use sea_orm::ConnectionTrait;

#[tokio::test]
async fn setup_test_db_runs_core_and_proxmox_migrations() {
    let db = support::db::setup_test_db().await;
    // proxmox_host_mapping is a proxmox-side table; existence proves the
    // plugin migration ran. tenant is core-side; existence proves the core
    // migration ran. Both in one pass.
    db.execute_unprepared("SELECT id FROM proxmox_host_mapping LIMIT 1")
        .await
        .expect("proxmox_host_mapping table must exist");
    db.execute_unprepared("SELECT id FROM tenants LIMIT 1")
        .await
        .expect("tenants table must exist");
}
```

- [ ] **Step 2: Run the test, observe compilation failure**

Run: `cargo test -p uptrakit-functional-tests --test proxmox_update_lifecycle setup_test_db_runs_core_and_proxmox_migrations`
Expected: FAIL — `unresolved import support::db` (no support module yet).

- [ ] **Step 3: Create `support/mod.rs`**

Write `crates/core/functional-tests/tests/support/mod.rs`:

```rust
pub mod db;
```

(`fixtures` and `stubs` will be added in later tasks.)

- [ ] **Step 4: Implement `setup_test_db`**

Write `crates/core/functional-tests/tests/support/db.rs`:

```rust
use sea_orm::{Database, DatabaseConnection};

/// In-memory SQLite + combined core+proxmox migrations in one pass.
///
/// `CombinedMigrator` uses a thread-local to merge plugin migrations with
/// core migrations; calling core and plugin migrations in separate steps
/// would double-run the schema, so this helper performs both in one
/// `run_migrations_with_plugins` call.
pub async fn setup_test_db() -> DatabaseConnection {
    // EncryptedString columns appear indirectly in plugin_config payloads;
    // plaintext mode keeps them inspectable.
    uptrakit_crypto::enable_plaintext_mode();

    let db = Database::connect("sqlite::memory:")
        .await
        .expect("open in-memory SQLite");

    uptrakit_shared_db::run_migrations_with_plugins(
        &db,
        uptrakit_plugin_infrastructure_proxmox::ProxmoxPlugin::controller_migrations(),
    )
    .await
    .expect("combined migrations");

    db
}
```

- [ ] **Step 5: Run the smoke test, observe pass**

Run: `cargo test -p uptrakit-functional-tests --test proxmox_update_lifecycle setup_test_db_runs_core_and_proxmox_migrations`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/core/functional-tests/tests/
git commit -m "$(cat <<'EOF'
test(functional-tests): in-memory SQLite test database helper

Adds `setup_test_db()` that boots an in-memory SQLite connection and runs
core + proxmox migrations in one CombinedMigrator pass. Plaintext crypto
mode is enabled so EncryptedString columns are inspectable in assertions.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: `support/fixtures.rs` — TestFixtures + base row insertion

**Files:**

- Create: `crates/core/functional-tests/tests/support/fixtures.rs`
- Modify: `crates/core/functional-tests/tests/support/mod.rs`

Snapshot rules applied: SeaORM `ActiveModel { col: Set(value), .. }` pattern; FK insertion order tenant → host → service → service_host → software_item → host_software_item → plugin_config → host_software_item_plugin → update_history.

- [ ] **Step 1: Write the failing test**

Append to `crates/core/functional-tests/tests/proxmox_update_lifecycle.rs`:

```rust
#[tokio::test]
async fn fixtures_insert_seeds_all_base_rows() {
    use sea_orm::EntityTrait;

    let db = support::db::setup_test_db().await;
    let fixtures = support::fixtures::TestFixtures::insert(
        &db,
        "http://127.0.0.1:9999",
    )
    .await;

    let tenant = uptrakit_shared_db::entity::tenant::Entity::find_by_id(fixtures.tenant_id)
        .one(&db)
        .await
        .unwrap();
    assert!(tenant.is_some(), "tenant row must exist");

    let history = uptrakit_shared_db::entity::update_history::Entity::find_by_id(
        fixtures.update_history_id,
    )
    .one(&db)
    .await
    .unwrap();
    assert_eq!(
        history.unwrap().status,
        uptrakit_shared_db::entity::update_history::UpdateStatus::Pending,
        "update_history must start at Pending",
    );
}
```

- [ ] **Step 2: Run the test, observe compilation failure**

Run: `cargo test -p uptrakit-functional-tests --test proxmox_update_lifecycle fixtures_insert_seeds_all_base_rows`
Expected: FAIL — `unresolved module support::fixtures`.

- [ ] **Step 3: Register `fixtures` module**

In `crates/core/functional-tests/tests/support/mod.rs`:

```rust
pub mod db;
pub mod fixtures;
```

- [ ] **Step 4: Implement `TestFixtures::insert`**

Write `crates/core/functional-tests/tests/support/fixtures.rs`:

```rust
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
use serde_json::json;
use time::OffsetDateTime;
use uuid::Uuid;

use uptrakit_shared_db::entity::{
    host, host_software_item, host_software_item_plugin, plugin_config, service,
    service_host, software_item, tenant, update_history,
};
use uptrakit_shared_types::ServiceStatus;

pub struct TestFixtures {
    pub tenant_id: Uuid,
    pub host_id: Uuid,
    pub service_id: Uuid,
    pub software_item_id: Uuid,
    pub host_software_item_id: Uuid,
    pub shell_config_id: Uuid,
    pub proxmox_config_id: Uuid,
    pub execute_update_plugin_id: Uuid,
    pub detect_version_plugin_id: Uuid,
    pub update_history_id: Uuid,
}

impl TestFixtures {
    pub async fn insert(db: &DatabaseConnection, proxmox_api_url: &str) -> Self {
        let now = OffsetDateTime::now_utc();

        let tenant_id = Uuid::now_v7();
        tenant::ActiveModel {
            id: Set(tenant_id),
            name: Set("test-tenant".to_string()),
            slug: Set("test-tenant".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(db)
        .await
        .expect("insert tenant");

        let host_id = Uuid::now_v7();
        host::ActiveModel {
            id: Set(host_id),
            tenant_id: Set(tenant_id),
            machine_id: Set("test-machine".to_string()),
            hostname: Set("test-host".to_string()),
            active: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(db)
        .await
        .expect("insert host");

        let service_id = Uuid::now_v7();
        service::ActiveModel {
            id: Set(service_id),
            tenant_id: Set(tenant_id),
            name: Set("test-agent".to_string()),
            service_type: Set(uptrakit_shared_types::ServiceType::Agent),
            status: Set(ServiceStatus::Approved),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(db)
        .await
        .expect("insert service");

        service_host::ActiveModel {
            id: Set(Uuid::now_v7()),
            service_id: Set(service_id),
            host_id: Set(host_id),
            created_at: Set(now),
            ..Default::default()
        }
        .insert(db)
        .await
        .expect("insert service_host");

        let software_item_id = Uuid::now_v7();
        software_item::ActiveModel {
            id: Set(software_item_id),
            tenant_id: Set(tenant_id),
            name: Set("test-software".to_string()),
            active: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(db)
        .await
        .expect("insert software_item");

        let host_software_item_id = Uuid::now_v7();
        host_software_item::ActiveModel {
            id: Set(host_software_item_id),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            tenant_id: Set(tenant_id),
            installed_version: Set(Some("1.0.0".to_string())),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(db)
        .await
        .expect("insert host_software_item");

        let shell_config_id = Uuid::now_v7();
        plugin_config::ActiveModel {
            id: Set(shell_config_id),
            tenant_id: Set(tenant_id),
            name: Set("test-shell-config".to_string()),
            plugin_type: Set("generic_shell".to_string()),
            config: Set(json!({
                "update_command": "echo ok",
                "version_command": "echo 1.0.0"
            })),
            enabled: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert shell plugin_config");

        let proxmox_config_id = Uuid::now_v7();
        plugin_config::ActiveModel {
            id: Set(proxmox_config_id),
            tenant_id: Set(tenant_id),
            name: Set("test-proxmox-config".to_string()),
            plugin_type: Set("infrastructure_proxmox".to_string()),
            config: Set(json!({
                "api_url": proxmox_api_url,
                "api_token": "root@pam!tok=secret",
                "verify_tls": false
            })),
            enabled: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert proxmox plugin_config");

        let execute_update_plugin_id = Uuid::now_v7();
        host_software_item_plugin::ActiveModel {
            id: Set(execute_update_plugin_id),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            host_software_item_id: Set(host_software_item_id),
            plugin_config_id: Set(Some(shell_config_id)),
            plugin_type: Set("generic_shell".to_string()),
            role: Set("execute_update".to_string()),
            ordinal: Set(0),
            package_identifier: Set("test-shell-pkg".to_string()),
            config: Set(None),
            execution_site: Set("agent".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .expect("insert execute_update plugin");

        let detect_version_plugin_id = Uuid::now_v7();
        host_software_item_plugin::ActiveModel {
            id: Set(detect_version_plugin_id),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            host_software_item_id: Set(host_software_item_id),
            plugin_config_id: Set(Some(shell_config_id)),
            plugin_type: Set("generic_shell".to_string()),
            role: Set("detect_version".to_string()),
            ordinal: Set(0),
            package_identifier: Set("test-shell-pkg".to_string()),
            config: Set(None),
            execution_site: Set("agent".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .expect("insert detect_version plugin");

        let update_history_id = Uuid::now_v7();
        update_history::ActiveModel {
            id: Set(update_history_id),
            tenant_id: Set(tenant_id),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            host_software_item_id: Set(host_software_item_id),
            from_version: Set(Some("1.0.0".to_string())),
            to_version: Set("2.0.0".to_string()),
            status: Set(update_history::UpdateStatus::Pending),
            interactive: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(db)
        .await
        .expect("insert update_history");

        Self {
            tenant_id,
            host_id,
            service_id,
            software_item_id,
            host_software_item_id,
            shell_config_id,
            proxmox_config_id,
            execute_update_plugin_id,
            detect_version_plugin_id,
            update_history_id,
        }
    }
}
```

> **Implementer note:** the exact `ActiveModel` field set for each entity is what the SeaORM `DeriveEntityModel` generates from the schema in `crates/shared/db/src/entity/`. The block above lists every column referenced by spec assertions; any column with `..Default::default()` is a column not relied on. If a NOT-NULL column without a default trips the insert, set it explicitly to the entity's documented default (do not chase the error by adding `Default` derives upstream — that is scope creep).

- [ ] **Step 5: Run the test, observe pass**

Run: `cargo test -p uptrakit-functional-tests --test proxmox_update_lifecycle fixtures_insert_seeds_all_base_rows`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/core/functional-tests/tests/support/
git commit -m "$(cat <<'EOF'
test(functional-tests): base row insertion fixtures

Adds `TestFixtures::insert()` which seeds the FK-ordered minimum set of rows
(tenant, host, service, service_host, software_item, host_software_item,
shell + proxmox plugin_config, two host_software_item_plugin rows for the
execute_update and detect_version roles, update_history at Pending). Each
test extends this base with scenario-specific rows (proxmox_host_mapping,
protection_default, scaling_default, backup_target_cache).

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: `TestFixtures::pending_work` — `PendingProtectionWork` builder

**Files:**

- Modify: `crates/core/functional-tests/tests/support/fixtures.rs`

Snapshot rule applied: typed-boundaries-internally — build `PendingProtectionWork` from ORM `Model` structs directly so future schema changes surface at compile time.

- [ ] **Step 1: Write the failing test**

Append to `crates/core/functional-tests/tests/proxmox_update_lifecycle.rs`:

```rust
#[tokio::test]
async fn pending_work_builder_matches_fixtures() {
    let db = support::db::setup_test_db().await;
    let fixtures = support::fixtures::TestFixtures::insert(
        &db,
        "http://127.0.0.1:9999",
    )
    .await;
    let work = fixtures.pending_work(&db, "2.0.0").await;
    assert_eq!(work.update_history_id, fixtures.update_history_id);
    assert_eq!(work.to_version, "2.0.0");
    assert!(!work.interactive);
    assert_eq!(work.target.item.id, fixtures.software_item_id);
    assert_eq!(work.target.host.id, fixtures.host_id);
}
```

- [ ] **Step 2: Run the test, observe failure**

Run: `cargo test -p uptrakit-functional-tests --test proxmox_update_lifecycle pending_work_builder_matches_fixtures`
Expected: FAIL — `no method named pending_work`.

- [ ] **Step 3: Implement `pending_work`**

Append to `crates/core/functional-tests/tests/support/fixtures.rs`:

```rust
use uptrakit_web_api_queries::queries::update_dispatch::{
    PendingProtectionWork, ValidatedUpdateTarget,
};

impl TestFixtures {
    pub async fn pending_work(
        &self,
        db: &DatabaseConnection,
        to_version: &str,
    ) -> PendingProtectionWork {
        let item = software_item::Entity::find_by_id(self.software_item_id)
            .one(db)
            .await
            .unwrap()
            .expect("software_item row");
        let host = host::Entity::find_by_id(self.host_id)
            .one(db)
            .await
            .unwrap()
            .expect("host row");
        let hsi_link = host_software_item::Entity::find_by_id(self.host_software_item_id)
            .one(db)
            .await
            .unwrap()
            .expect("host_software_item row");
        let agent = service::Entity::find_by_id(self.service_id)
            .one(db)
            .await
            .unwrap()
            .expect("service row");
        let shell_cfg = plugin_config::Entity::find_by_id(self.shell_config_id)
            .one(db)
            .await
            .unwrap()
            .expect("shell plugin_config row");
        let execute_plugin =
            host_software_item_plugin::Entity::find_by_id(self.execute_update_plugin_id)
                .one(db)
                .await
                .unwrap()
                .expect("execute_update plugin row");
        let detect_plugin =
            host_software_item_plugin::Entity::find_by_id(self.detect_version_plugin_id)
                .one(db)
                .await
                .unwrap()
                .expect("detect_version plugin row");

        let target = ValidatedUpdateTarget {
            item,
            host,
            hsi_link,
            agent,
            execute_update_data: (execute_plugin, Some(shell_cfg.clone())),
            detect_version_data: Some((detect_plugin, Some(shell_cfg))),
            fetch_releases_config: None,
            pre_update_hook_plugins: vec![],
            post_update_hook_plugins: vec![],
        };

        PendingProtectionWork {
            target,
            update_history_id: self.update_history_id,
            to_version: to_version.to_string(),
            release_info: None,
            interactive: false,
        }
    }
}
```

> **Implementer note:** if `ValidatedUpdateTarget` or `PendingProtectionWork` carries non-`pub` fields that block direct construction, surface that as a blocker rather than widening their visibility — the spec did not authorize widening those types. Falling back to whatever public constructor exists (`ValidatedUpdateTarget::new(...)` or similar) is the correct fix; check `crates/ui/web-api-queries/src/queries/update_dispatch.rs` for the published API.

- [ ] **Step 4: Run the test, observe pass**

Run: `cargo test -p uptrakit-functional-tests --test proxmox_update_lifecycle pending_work_builder_matches_fixtures`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core/functional-tests/tests/support/fixtures.rs
git commit -m "$(cat <<'EOF'
test(functional-tests): PendingProtectionWork builder

Adds `TestFixtures::pending_work()` which loads the fixture rows from the
DB and assembles `ValidatedUpdateTarget` + `PendingProtectionWork`.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: `support/stubs.rs` — `build_plugin_ops`, `TestNotificationSetup`, `NoopOutputStream`

**Files:**

- Create: `crates/core/functional-tests/tests/support/stubs.rs`
- Modify: `crates/core/functional-tests/tests/support/mod.rs`

Snapshot rules applied: real production types over hand-rolled stubs (`PluginCatalog`); typed boundaries (concrete return types for stubs).

- [ ] **Step 1: Write the failing test**

Append to `crates/core/functional-tests/tests/proxmox_update_lifecycle.rs`:

```rust
#[tokio::test]
async fn build_plugin_ops_with_proxmox_returns_protection_and_hook() {
    let plugin_ops = support::stubs::build_plugin_ops(true);
    use uptrakit_plugin_infrastructure_core::roles::{
        ControllerUpdateHookOps, ControllerUpdateProtectionOps,
    };
    assert!(plugin_ops.controller_update_protection().is_some());
    assert!(plugin_ops.controller_update_hook().is_some());
}

#[tokio::test]
async fn build_plugin_ops_without_proxmox_returns_none() {
    let plugin_ops = support::stubs::build_plugin_ops(false);
    use uptrakit_plugin_infrastructure_core::roles::{
        ControllerUpdateHookOps, ControllerUpdateProtectionOps,
    };
    assert!(plugin_ops.controller_update_protection().is_none());
    assert!(plugin_ops.controller_update_hook().is_none());
}
```

> **Implementer note:** the precise import path for the `ControllerUpdate{Protection,Hook}Ops` traits varies by feature set. If the import above fails to resolve, run `rg "pub trait ControllerUpdateProtectionOps" crates/plugins/infrastructure/core/src` to find the actual path; do not reach beyond `crates/plugins/infrastructure/core/` to discover it.

- [ ] **Step 2: Run, observe failure**

Run: `cargo test -p uptrakit-functional-tests --test proxmox_update_lifecycle build_plugin_ops`
Expected: FAIL — `unresolved module support::stubs`.

- [ ] **Step 3: Register the stubs module**

Update `crates/core/functional-tests/tests/support/mod.rs`:

```rust
pub mod db;
pub mod fixtures;
pub mod stubs;
```

- [ ] **Step 4: Implement `build_plugin_ops`, `TestNotificationSetup`, `NoopOutputStream`**

Write `crates/core/functional-tests/tests/support/stubs.rs`:

```rust
use std::collections::BTreeSet;
use std::sync::Arc;

use tokio::sync::mpsc;
use uuid::Uuid;

use uptrakit_controller_core::{
    EventBroadcaster, NotificationDispatcher, NotificationService, NotificationState,
};
use uptrakit_plugin_infrastructure_core::{
    plugin_ops::PluginOps, CatalogConfig, InstancePluginStates, PluginCatalog,
};
use uptrakit_plugin_infrastructure_proxmox::DESCRIPTOR as PROXMOX_DESCRIPTOR;
use uptrakit_wire::ControllerMessage;

/// Build a real `PluginCatalog` over `Arc<dyn PluginOps>`.
///
/// `with_proxmox = true` registers the Proxmox descriptor — both
/// `controller_update_protection()` and `controller_update_hook()` return the
/// real plugin. `false` returns an empty catalog where both accessors are
/// `None` (used by Test 3 — no Proxmox path).
pub fn build_plugin_ops(with_proxmox: bool) -> Arc<dyn PluginOps> {
    let descriptors = if with_proxmox {
        vec![&PROXMOX_DESCRIPTOR]
    } else {
        vec![]
    };
    let catalog = PluginCatalog::new(
        descriptors,
        &CatalogConfig {
            allow_private_urls: false,
            global_provider_lookup: None,
            #[cfg(feature = "catalog")]
            http_client: None,
            #[cfg(feature = "catalog")]
            cancellation_token: None,
        },
        // Proxmox is Tenant-scoped; InstancePluginStates only gates
        // Instance-scoped plugins, so `all_disabled()` is correct here.
        InstancePluginStates::all_disabled(),
    )
    .expect("PluginCatalog::new must succeed");
    Arc::new(catalog)
}

/// Holds a real `NotificationState` plus the receiver end of the agent's
/// `ControllerMessage` channel, so tests can capture the dispatched
/// `ExecuteUpdate` payload.
pub struct TestNotificationSetup {
    pub notification_state: NotificationState,
    pub message_rx: mpsc::Receiver<ControllerMessage>,
}

impl TestNotificationSetup {
    pub async fn new(agent_service_id: Uuid) -> Self {
        use uptrakit_controller_core::ServiceConnectionRegistry;

        let registry = ServiceConnectionRegistry::new();
        let (message_rx, _handle) = registry
            .register(agent_service_id, BTreeSet::new(), None, None, None)
            .await;
        let notification_service =
            NotificationService::new(registry, Uuid::now_v7());
        let (dispatcher, _event_rx) = NotificationDispatcher::test_channel();
        let event_broadcaster = EventBroadcaster::new();
        Self {
            notification_state: NotificationState::new(
                notification_service,
                dispatcher,
                event_broadcaster,
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

/// No-op `UpdateOutputStream`. All methods have empty bodies.
pub struct NoopOutputStream;

#[async_trait::async_trait]
impl uptrakit_controller_core::UpdateOutputStream for NoopOutputStream {
    // Implement every required method with an empty body returning the
    // appropriate typed Ok / unit value. Verify the full method list against
    // the trait definition at `crates/ui/controller-core/src/<update_output_stream module>`
    // — if a method returns Result<()>, return Ok(()); if it returns a stream,
    // return an empty stream.
}
```

> **Implementer note on imports:** `EventBroadcaster`, `NotificationDispatcher::test_channel`, `NotificationService`, `NotificationState`, `ServiceConnectionRegistry`, `UpdateOutputStream` — each is re-exported somewhere in `crates/ui/controller-core/src/lib.rs` (some behind feature gates). If a given import path resolves to a different module, follow the actual re-export — do not duplicate or add intermediate `pub use` lines unless the existing surface genuinely omits the symbol. If a symbol is missing from the public surface, surface that as a blocker, not as an opportunity to widen the API.

- [ ] **Step 5: Run, observe pass**

Run: `cargo test -p uptrakit-functional-tests --test proxmox_update_lifecycle build_plugin_ops`
Expected: both tests PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/core/functional-tests/tests/support/stubs.rs crates/core/functional-tests/tests/support/mod.rs
git commit -m "$(cat <<'EOF'
test(functional-tests): plugin catalog + notification + output stream helpers

`build_plugin_ops()` wraps the real `PluginCatalog` over `Arc<dyn PluginOps>`
with the Proxmox descriptor optionally registered. `TestNotificationSetup`
constructs a real `NotificationState` whose `ServiceConnectionRegistry`
channel is observable from the test thread, capturing the dispatched
`ControllerMessage::ExecuteUpdate`. `NoopOutputStream` discards forwarded
output lines.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Per-test helper rows (proxmox mapping / protection / scaling / backup cache)

**Files:**

- Modify: `crates/core/functional-tests/tests/support/fixtures.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/core/functional-tests/tests/proxmox_update_lifecycle.rs`:

```rust
#[tokio::test]
async fn fixtures_insert_proxmox_mapping_writes_row() {
    use sea_orm::EntityTrait;

    let db = support::db::setup_test_db().await;
    let fixtures = support::fixtures::TestFixtures::insert(
        &db,
        "http://127.0.0.1:9999",
    )
    .await;
    fixtures.insert_proxmox_mapping(&db, "pve1", 100, "qemu").await;

    let count = uptrakit_plugin_infrastructure_proxmox::entity::proxmox_host_mapping::Entity::find()
        .all(&db)
        .await
        .unwrap()
        .len();
    assert_eq!(count, 1);
}
```

- [ ] **Step 2: Run, observe failure**

Run: `cargo test -p uptrakit-functional-tests --test proxmox_update_lifecycle fixtures_insert_proxmox_mapping_writes_row`
Expected: FAIL — `no method named insert_proxmox_mapping` (and possibly an entity path mismatch — adjust import to the actual `pub use` published by `uptrakit-plugin-infrastructure-proxmox` after checking the crate's `lib.rs`).

- [ ] **Step 3: Implement the four per-test helpers**

Append to `crates/core/functional-tests/tests/support/fixtures.rs`:

```rust
use uptrakit_plugin_infrastructure_proxmox::entity::{
    proxmox_backup_target_cache, proxmox_host_mapping,
    proxmox_protection_default, proxmox_scaling_default,
};

impl TestFixtures {
    pub async fn insert_proxmox_mapping(
        &self,
        db: &DatabaseConnection,
        node: &str,
        vmid: i64,
        vm_type: &str,
    ) {
        let now = OffsetDateTime::now_utc();
        proxmox_host_mapping::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(self.tenant_id),
            host_id: Set(self.host_id),
            plugin_config_id: Set(self.proxmox_config_id),
            proxmox_node: Set(node.to_string()),
            proxmox_vmid: Set(vmid),
            proxmox_type: Set(vm_type.to_string()),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(db)
        .await
        .expect("insert proxmox_host_mapping");
    }

    pub async fn insert_protection_default_snapshot(&self, db: &DatabaseConnection) {
        self.insert_protection_default_raw(db, "snapshot", None)
            .await;
    }

    pub async fn insert_protection_default_backup(
        &self,
        db: &DatabaseConnection,
        backup_target_key: &str,
    ) {
        self.insert_protection_default_raw(db, "backup", Some(backup_target_key))
            .await;
    }

    pub async fn insert_protection_default_do_nothing(&self, db: &DatabaseConnection) {
        self.insert_protection_default_raw(db, "do_nothing", None)
            .await;
    }

    async fn insert_protection_default_raw(
        &self,
        db: &DatabaseConnection,
        mode: &str,
        backup_target_key: Option<&str>,
    ) {
        let now = OffsetDateTime::now_utc();
        proxmox_protection_default::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(self.tenant_id),
            plugin_config_id: Set(self.proxmox_config_id),
            mode: Set(mode.to_string()),
            backup_target_key: Set(backup_target_key.map(str::to_string)),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(db)
        .await
        .expect("insert proxmox_protection_default");
    }

    pub async fn insert_scaling_default_delta(
        &self,
        db: &DatabaseConnection,
        delta_cores: i32,
        delta_memory_mb: i64,
    ) {
        let now = OffsetDateTime::now_utc();
        proxmox_scaling_default::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(self.tenant_id),
            plugin_config_id: Set(self.proxmox_config_id),
            mode: Set("delta".to_string()),
            delta_cores: Set(Some(delta_cores)),
            delta_memory_mb: Set(Some(delta_memory_mb)),
            absolute_cores: Set(None),
            absolute_memory_mb: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(db)
        .await
        .expect("insert proxmox_scaling_default (delta)");
    }

    pub async fn insert_scaling_default_absolute(
        &self,
        db: &DatabaseConnection,
        absolute_cores: i32,
        absolute_memory_mb: i64,
    ) {
        let now = OffsetDateTime::now_utc();
        proxmox_scaling_default::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(self.tenant_id),
            plugin_config_id: Set(self.proxmox_config_id),
            mode: Set("absolute".to_string()),
            delta_cores: Set(None),
            delta_memory_mb: Set(None),
            absolute_cores: Set(Some(absolute_cores)),
            absolute_memory_mb: Set(Some(absolute_memory_mb)),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(db)
        .await
        .expect("insert proxmox_scaling_default (absolute)");
    }

    pub async fn insert_backup_target_cache(
        &self,
        db: &DatabaseConnection,
        target_key: &str,
        storage_id: &str,
    ) {
        let now = OffsetDateTime::now_utc();
        proxmox_backup_target_cache::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(self.tenant_id),
            plugin_config_id: Set(self.proxmox_config_id),
            target_key: Set(target_key.to_string()),
            storage_id: Set(storage_id.to_string()),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(db)
        .await
        .expect("insert proxmox_backup_target_cache");
    }
}
```

> **Implementer note:** column lists and types for each proxmox entity above are the spec's best understanding of the schema; cross-check against `crates/plugins/infrastructure/proxmox/src/entity/*.rs` and the controller migrations for the actual column set. Add missing NOT NULL columns with the schema's default; do not silently widen types.

- [ ] **Step 4: Run, observe pass**

Run: `cargo test -p uptrakit-functional-tests --test proxmox_update_lifecycle fixtures_insert_proxmox_mapping_writes_row`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core/functional-tests/tests/support/fixtures.rs
git commit -m "$(cat <<'EOF'
test(functional-tests): per-test proxmox row helpers

Adds insertion helpers for `proxmox_host_mapping`,
`proxmox_protection_default` (snapshot/backup/do_nothing),
`proxmox_scaling_default` (delta/absolute), and
`proxmox_backup_target_cache`.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: CAS sentinel helper

**Files:**

- Modify: `crates/core/functional-tests/tests/proxmox_update_lifecycle.rs`

The CAS sentinel runs as the first assertion of every lifecycle test. Factor it out so the six tests stay readable.

- [ ] **Step 1: Write the failing test**

Append to `crates/core/functional-tests/tests/proxmox_update_lifecycle.rs`:

```rust
#[tokio::test]
async fn cas_sentinel_passes_when_status_is_in_progress() {
    use sea_orm::{ActiveModelTrait, Set};
    use uptrakit_shared_db::entity::update_history;

    let db = support::db::setup_test_db().await;
    let fixtures = support::fixtures::TestFixtures::insert(
        &db,
        "http://127.0.0.1:9999",
    )
    .await;

    // Manually flip the row to InProgress to simulate
    // `set_inprogress_for_orchestrator` having run.
    let row = update_history::Entity::find_by_id(fixtures.update_history_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let mut am: update_history::ActiveModel = row.into();
    am.status = Set(update_history::UpdateStatus::InProgress);
    am.update(&db).await.unwrap();

    assert_cas_sentinel(&db, fixtures.update_history_id).await;
}
```

- [ ] **Step 2: Run, observe failure**

Run: `cargo test -p uptrakit-functional-tests --test proxmox_update_lifecycle cas_sentinel_passes_when_status_is_in_progress`
Expected: FAIL — `assert_cas_sentinel` undefined.

- [ ] **Step 3: Implement the helper**

Append to `crates/core/functional-tests/tests/proxmox_update_lifecycle.rs` (above the first test):

```rust
use sea_orm::EntityTrait;
use uuid::Uuid;

async fn assert_cas_sentinel(db: &sea_orm::DatabaseConnection, update_history_id: Uuid) {
    let updated = uptrakit_shared_db::entity::update_history::Entity::find_by_id(update_history_id)
        .one(db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        updated.status,
        uptrakit_shared_db::entity::update_history::UpdateStatus::InProgress,
        "CAS Pending->InProgress failed: run_protection_and_dispatch exited early",
    );
}
```

- [ ] **Step 4: Run, observe pass**

Run: `cargo test -p uptrakit-functional-tests --test proxmox_update_lifecycle cas_sentinel_passes_when_status_is_in_progress`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core/functional-tests/tests/proxmox_update_lifecycle.rs
git commit -m "$(cat <<'EOF'
test(functional-tests): CAS sentinel assertion helper

Adds `assert_cas_sentinel` for the first-assertion guard that catches
silent early returns from `run_protection_and_dispatch` (wrong fixture
status, connectivity check failure, etc.).

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: Test 1 — `snapshot_protection_and_scaling_before_dispatch`

**Files:**

- Modify: `crates/core/functional-tests/tests/proxmox_update_lifecycle.rs`

- [ ] **Step 1: Add the test (failing — wires the full path for the first time)**

Append to `crates/core/functional-tests/tests/proxmox_update_lifecycle.rs`:

```rust
#[tokio::test]
async fn snapshot_protection_and_scaling_before_dispatch() {
    use httpmock::prelude::*;
    use uptrakit_controller_core::testing::run_protection_and_dispatch;
    use uptrakit_plugin_infrastructure_proxmox::entity::proxmox_resource_scaling_record;
    use uptrakit_shared_db::entity::update_history;
    use uptrakit_wire::ControllerMessage;

    let server = MockServer::start_async().await;
    let task_status_mock = server.mock_async(|when, then| {
        when.method(GET)
            .path_contains("/tasks/")
            .path_contains("/status");
        then.status(200)
            .json_body(serde_json::json!({
                "data": {"status": "stopped", "exitstatus": "OK"}
            }));
    })
    .await;
    let snapshot_mock = server.mock_async(|when, then| {
        when.method(POST)
            .path("/api2/json/nodes/pve1/qemu/100/snapshot");
        then.status(200)
            .json_body(serde_json::json!({"data": "UPID:pve1:001:snapshot"}));
    })
    .await;
    let scale_get_mock = server.mock_async(|when, then| {
        when.method(GET)
            .path("/api2/json/nodes/pve1/qemu/100/config");
        then.status(200)
            .json_body(serde_json::json!({
                "data": {"cores": 2, "memory": 2048, "hotplug": "cpu,memory"}
            }));
    })
    .await;
    let scale_put_mock = server.mock_async(|when, then| {
        when.method(PUT)
            .path("/api2/json/nodes/pve1/qemu/100/config");
        then.status(200).json_body(serde_json::json!({"data": null}));
    })
    .await;

    let db = support::db::setup_test_db().await;
    let fixtures = support::fixtures::TestFixtures::insert(&db, &server.base_url()).await;
    fixtures.insert_proxmox_mapping(&db, "pve1", 100, "qemu").await;
    fixtures.insert_protection_default_snapshot(&db).await;
    fixtures.insert_scaling_default_delta(&db, 2, 1024).await;

    let mut notif = support::stubs::TestNotificationSetup::new(fixtures.service_id).await;
    let plugin_ops = support::stubs::build_plugin_ops(true);
    let work = fixtures.pending_work(&db, "2.0.0").await;

    run_protection_and_dispatch(
        db.clone(),
        notif.notification_state.clone(),
        std::sync::Arc::new(support::stubs::NoopOutputStream),
        plugin_ops,
        work,
    )
    .await;

    assert_cas_sentinel(&db, fixtures.update_history_id).await;

    snapshot_mock.assert_calls_async(1).await;
    task_status_mock.assert_calls_async(1).await;
    scale_get_mock.assert_calls_async(1).await;
    scale_put_mock.assert_calls_async(1).await;

    let msgs = notif.captured_messages();
    assert_eq!(msgs.len(), 1, "exactly one ExecuteUpdate dispatched");
    let payload = match &msgs[0] {
        ControllerMessage::ExecuteUpdate(p) => p,
        other => panic!("expected ExecuteUpdate, got {other:?}"),
    };
    assert_eq!(payload.to_version, "2.0.0");

    let hist = update_history::Entity::find_by_id(fixtures.update_history_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        hist.pre_update_protection_status,
        Some("protected".to_string()),
    );

    use sea_orm::EntityTrait;
    let scaling_rows = proxmox_resource_scaling_record::Entity::find()
        .all(&db)
        .await
        .unwrap();
    assert_eq!(scaling_rows.len(), 1);
    assert_eq!(scaling_rows[0].restore_status, "pending");
}
```

- [ ] **Step 2: Run, observe outcome**

Run: `cargo test -p uptrakit-functional-tests --test proxmox_update_lifecycle snapshot_protection_and_scaling_before_dispatch -- --nocapture`
Expected: PASS. If it fails, treat the failure as a real signal — do not silence assertions. Diagnose against `crates/ui/controller-core/src/update/controller.rs` and `crates/plugins/infrastructure/proxmox/src/update_protection.rs`.

- [ ] **Step 3: Commit**

```bash
git add crates/core/functional-tests/tests/proxmox_update_lifecycle.rs
git commit -m "$(cat <<'EOF'
test(functional-tests): snapshot protection + scaling lifecycle

Test 1: drives `run_protection_and_dispatch` with snapshot protection +
delta scaling. Asserts (in order) CAS sentinel, snapshot POST + task status
GET, scale GET + PUT, exactly one dispatched ExecuteUpdate with
to_version=2.0.0, update_history.pre_update_protection_status="protected",
one proxmox_resource_scaling_record row with restore_status="pending".

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: Test 2 — `backup_protection_before_dispatch`

**Files:**

- Modify: `crates/core/functional-tests/tests/proxmox_update_lifecycle.rs`

- [ ] **Step 1: Add the test**

```rust
#[tokio::test]
async fn backup_protection_before_dispatch() {
    use httpmock::prelude::*;
    use uptrakit_controller_core::testing::run_protection_and_dispatch;
    use uptrakit_shared_db::entity::update_history;
    use uptrakit_wire::ControllerMessage;

    let server = MockServer::start_async().await;
    let task_status_mock = server.mock_async(|when, then| {
        when.method(GET)
            .path_contains("/tasks/")
            .path_contains("/status");
        then.status(200)
            .json_body(serde_json::json!({
                "data": {"status": "stopped", "exitstatus": "OK"}
            }));
    })
    .await;
    let vzdump_mock = server.mock_async(|when, then| {
        when.method(POST)
            .path("/api2/json/nodes/pve1/vzdump")
            .body_contains("vmid=100")
            .body_contains("storage=storage1");
        then.status(200)
            .json_body(serde_json::json!({"data": "UPID:pve1:002:backup"}));
    })
    .await;

    let db = support::db::setup_test_db().await;
    let fixtures = support::fixtures::TestFixtures::insert(&db, &server.base_url()).await;
    fixtures.insert_proxmox_mapping(&db, "pve1", 100, "qemu").await;
    fixtures
        .insert_protection_default_backup(&db, "pve1:storage1:dir")
        .await;
    fixtures
        .insert_backup_target_cache(&db, "pve1:storage1:dir", "storage1")
        .await;

    let mut notif = support::stubs::TestNotificationSetup::new(fixtures.service_id).await;
    let plugin_ops = support::stubs::build_plugin_ops(true);
    let work = fixtures.pending_work(&db, "2.0.0").await;

    run_protection_and_dispatch(
        db.clone(),
        notif.notification_state.clone(),
        std::sync::Arc::new(support::stubs::NoopOutputStream),
        plugin_ops,
        work,
    )
    .await;

    assert_cas_sentinel(&db, fixtures.update_history_id).await;
    vzdump_mock.assert_calls_async(1).await;
    task_status_mock.assert_calls_async(1).await;

    let msgs = notif.captured_messages();
    assert_eq!(msgs.len(), 1);
    assert!(matches!(msgs[0], ControllerMessage::ExecuteUpdate(_)));

    let hist = update_history::Entity::find_by_id(fixtures.update_history_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        hist.pre_update_protection_status,
        Some("protected".to_string()),
    );

    use sea_orm::EntityTrait;
}
```

- [ ] **Step 2: Run, observe pass**

Run: `cargo test -p uptrakit-functional-tests --test proxmox_update_lifecycle backup_protection_before_dispatch`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/core/functional-tests/tests/proxmox_update_lifecycle.rs
git commit -m "$(cat <<'EOF'
test(functional-tests): backup protection lifecycle

Test 2: backup mode with cached target. Asserts vzdump POST with
`storage=storage1` body, task-status poll, dispatched ExecuteUpdate,
`pre_update_protection_status="protected"`.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 11: Test 3 — `no_proxmox_mapping_dispatch_proceeds`

**Files:**

- Modify: `crates/core/functional-tests/tests/proxmox_update_lifecycle.rs`

- [ ] **Step 1: Add the test**

```rust
#[tokio::test]
async fn no_proxmox_mapping_dispatch_proceeds() {
    use httpmock::prelude::*;
    use uptrakit_controller_core::testing::run_protection_and_dispatch;
    use uptrakit_plugin_infrastructure_proxmox::entity::proxmox_protection_audit;
    use uptrakit_wire::ControllerMessage;

    let server = MockServer::start_async().await;

    let db = support::db::setup_test_db().await;
    let fixtures = support::fixtures::TestFixtures::insert(&db, &server.base_url()).await;
    // No proxmox_host_mapping, no protection_default, no scaling_default.

    let mut notif = support::stubs::TestNotificationSetup::new(fixtures.service_id).await;
    // Empty catalog — both controller_update_protection() and
    // controller_update_hook() accessors return None.
    let plugin_ops = support::stubs::build_plugin_ops(false);
    let work = fixtures.pending_work(&db, "2.0.0").await;

    run_protection_and_dispatch(
        db.clone(),
        notif.notification_state.clone(),
        std::sync::Arc::new(support::stubs::NoopOutputStream),
        plugin_ops,
        work,
    )
    .await;

    assert_cas_sentinel(&db, fixtures.update_history_id).await;
    assert_eq!(
        server.hits_async().await,
        0,
        "no HTTP requests when no proxmox mapping",
    );

    let msgs = notif.captured_messages();
    assert_eq!(msgs.len(), 1);
    assert!(matches!(msgs[0], ControllerMessage::ExecuteUpdate(_)));

    // Durable contract: no protection plugin ran, so no audit row.
    use sea_orm::EntityTrait;
    let audits = proxmox_protection_audit::Entity::find()
        .all(&db)
        .await
        .unwrap();
    assert!(
        audits.is_empty(),
        "no proxmox_protection_audit rows when no plugin runs",
    );
    // Do NOT assert a specific pre_update_protection_status value here —
    // see spec Test 3 notes.
}
```

- [ ] **Step 2: Run, observe pass**

Run: `cargo test -p uptrakit-functional-tests --test proxmox_update_lifecycle no_proxmox_mapping_dispatch_proceeds`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/core/functional-tests/tests/proxmox_update_lifecycle.rs
git commit -m "$(cat <<'EOF'
test(functional-tests): no proxmox mapping dispatch path

Test 3: empty plugin catalog, no proxmox rows. Asserts zero HTTP requests,
ExecuteUpdate dispatched, no proxmox_protection_audit rows. Deliberately
does not pin `pre_update_protection_status` because the current value is a
contract violation between `set_inprogress_for_orchestrator` (writes
"in_progress") and `prepare_pre_update_protection`'s documented `None`
behavior.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 12: Test 4 — `do_nothing_protection_scaling_still_runs`

**Files:**

- Modify: `crates/core/functional-tests/tests/proxmox_update_lifecycle.rs`

- [ ] **Step 1: Add the test**

```rust
#[tokio::test]
async fn do_nothing_protection_scaling_still_runs() {
    use httpmock::prelude::*;
    use uptrakit_controller_core::testing::run_protection_and_dispatch;
    use uptrakit_plugin_infrastructure_proxmox::entity::proxmox_resource_scaling_record;
    use uptrakit_shared_db::entity::update_history;
    use uptrakit_wire::ControllerMessage;

    let server = MockServer::start_async().await;
    let scale_get_mock = server.mock_async(|when, then| {
        when.method(GET)
            .path("/api2/json/nodes/pve1/qemu/100/config");
        then.status(200)
            .json_body(serde_json::json!({
                "data": {"cores": 2, "memory": 2048, "hotplug": "cpu,memory"}
            }));
    })
    .await;
    let scale_put_mock = server.mock_async(|when, then| {
        when.method(PUT)
            .path("/api2/json/nodes/pve1/qemu/100/config");
        then.status(200).json_body(serde_json::json!({"data": null}));
    })
    .await;

    let db = support::db::setup_test_db().await;
    let fixtures = support::fixtures::TestFixtures::insert(&db, &server.base_url()).await;
    fixtures.insert_proxmox_mapping(&db, "pve1", 100, "qemu").await;
    fixtures.insert_protection_default_do_nothing(&db).await;
    fixtures.insert_scaling_default_absolute(&db, 4, 4096).await;

    let mut notif = support::stubs::TestNotificationSetup::new(fixtures.service_id).await;
    let plugin_ops = support::stubs::build_plugin_ops(true);
    let work = fixtures.pending_work(&db, "2.0.0").await;

    run_protection_and_dispatch(
        db.clone(),
        notif.notification_state.clone(),
        std::sync::Arc::new(support::stubs::NoopOutputStream),
        plugin_ops,
        work,
    )
    .await;

    assert_cas_sentinel(&db, fixtures.update_history_id).await;
    scale_get_mock.assert_calls_async(1).await;
    scale_put_mock.assert_calls_async(1).await;

    let msgs = notif.captured_messages();
    assert_eq!(msgs.len(), 1);
    assert!(matches!(msgs[0], ControllerMessage::ExecuteUpdate(_)));

    let hist = update_history::Entity::find_by_id(fixtures.update_history_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        hist.pre_update_protection_status,
        Some("skipped".to_string()),
    );

    use sea_orm::EntityTrait;
    let scaling_rows = proxmox_resource_scaling_record::Entity::find()
        .all(&db)
        .await
        .unwrap();
    assert_eq!(scaling_rows.len(), 1);
}
```

- [ ] **Step 2: Run, observe pass**

Run: `cargo test -p uptrakit-functional-tests --test proxmox_update_lifecycle do_nothing_protection_scaling_still_runs`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/core/functional-tests/tests/proxmox_update_lifecycle.rs
git commit -m "$(cat <<'EOF'
test(functional-tests): do_nothing protection still scales

Test 4: do_nothing protection mode with absolute scaling. Asserts no
snapshot/backup HTTP, scale GET + PUT each called once,
`pre_update_protection_status="skipped"`, scaling record inserted.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 13: Test 5 — `dispatch_payload_has_correct_plugin_assignments`

**Files:**

- Modify: `crates/core/functional-tests/tests/proxmox_update_lifecycle.rs`

Snapshot rule applied: wire type assertion must use the actual struct fields. `PluginAssignment` has `plugin_type: PluginTypeId`, `package_identifier: String`, `config: serde_json::Value` — there is **no** `config_id`. Use `PluginTypeId: PartialEq<&str>` for the plugin-type compare; assert on `config["update_command"]` / `config["version_command"]` for the config-origin check.

- [ ] **Step 1: Add the test**

```rust
#[tokio::test]
async fn dispatch_payload_has_correct_plugin_assignments() {
    use uptrakit_controller_core::testing::run_protection_and_dispatch;
    use uptrakit_wire::ControllerMessage;

    let db = support::db::setup_test_db().await;
    let fixtures = support::fixtures::TestFixtures::insert(&db, "http://127.0.0.1:9999").await;

    let mut notif = support::stubs::TestNotificationSetup::new(fixtures.service_id).await;
    let plugin_ops = support::stubs::build_plugin_ops(false);
    let work = fixtures.pending_work(&db, "2.0.0").await;

    run_protection_and_dispatch(
        db.clone(),
        notif.notification_state.clone(),
        std::sync::Arc::new(support::stubs::NoopOutputStream),
        plugin_ops,
        work,
    )
    .await;

    assert_cas_sentinel(&db, fixtures.update_history_id).await;

    let msgs = notif.captured_messages();
    assert_eq!(msgs.len(), 1);
    let payload = match &msgs[0] {
        ControllerMessage::ExecuteUpdate(p) => p,
        other => panic!("expected ExecuteUpdate, got {other:?}"),
    };

    assert_eq!(payload.to_version, "2.0.0");
    assert_eq!(payload.software_item_id, fixtures.software_item_id);

    let exec = &payload.execute_update_plugin;
    assert_eq!(exec.plugin_type, "generic_shell");
    assert_eq!(
        exec.config.get("update_command").and_then(|v| v.as_str()),
        Some("echo ok"),
        "execute_update assignment must carry shell config payload from fixtures",
    );

    let detect = payload
        .detect_version_plugin
        .as_ref()
        .expect("detect_version present");
    assert_eq!(detect.plugin_type, "generic_shell");
    assert_eq!(
        detect.config.get("version_command").and_then(|v| v.as_str()),
        Some("echo 1.0.0"),
        "detect_version assignment must carry shell config payload from fixtures",
    );
}
```

- [ ] **Step 2: Run, observe pass**

Run: `cargo test -p uptrakit-functional-tests --test proxmox_update_lifecycle dispatch_payload_has_correct_plugin_assignments`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/core/functional-tests/tests/proxmox_update_lifecycle.rs
git commit -m "$(cat <<'EOF'
test(functional-tests): wire payload assignment shape

Test 5: asserts the dispatched ExecuteUpdate payload carries the right
software_item_id, to_version, and per-role plugin assignments — exec uses
`generic_shell` with `config.update_command == "echo ok"`, detect uses
`generic_shell` with `config.version_command == "echo 1.0.0"`.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 14: Test 6 — `post_update_resource_restore`

**Files:**

- Modify: `crates/core/functional-tests/tests/proxmox_update_lifecycle.rs`

- [ ] **Step 1: Add the test**

```rust
#[tokio::test]
async fn post_update_resource_restore() {
    use httpmock::prelude::*;
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};
    use uptrakit_plugin_infrastructure_core::roles::ControllerUpdateHookOps;
    use uptrakit_plugin_infrastructure_proxmox::entity::proxmox_resource_scaling_record;
    use uptrakit_shared_db::entity::update_history;
    use uptrakit_web_api_queries::queries::update_dispatch::finalize_post_update_hook;

    let server = MockServer::start_async().await;
    let restore_mock = server.mock_async(|when, then| {
        when.method(PUT)
            .path("/api2/json/nodes/pve1/qemu/100/config")
            .body_contains("cores=2")
            .body_contains("memory=2048");
        then.status(200).json_body(serde_json::json!({"data": null}));
    })
    .await;

    let db = support::db::setup_test_db().await;
    let fixtures = support::fixtures::TestFixtures::insert(&db, &server.base_url()).await;
    fixtures.insert_proxmox_mapping(&db, "pve1", 100, "qemu").await;

    // Seed a scaling record that the hook is expected to restore.
    let now = time::OffsetDateTime::now_utc();
    proxmox_resource_scaling_record::ActiveModel {
        id: Set(uuid::Uuid::now_v7()),
        tenant_id: Set(fixtures.tenant_id),
        update_history_id: Set(fixtures.update_history_id),
        plugin_config_id: Set(fixtures.proxmox_config_id),
        proxmox_node: Set("pve1".to_string()),
        proxmox_vmid: Set(100),
        proxmox_type: Set("qemu".to_string()),
        original_cores: Set(Some(2)),
        original_memory_mb: Set(Some(2048)),
        scaled_cores: Set(Some(4)),
        scaled_memory_mb: Set(Some(4096)),
        restore_status: Set("pending".to_string()),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&db)
    .await
    .expect("insert proxmox_resource_scaling_record");

    let plugin_ops = support::stubs::build_plugin_ops(true);
    let hook = plugin_ops
        .controller_update_hook()
        .expect("proxmox update hook present");

    let record = update_history::Entity::find_by_id(fixtures.update_history_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();

    finalize_post_update_hook(&db, Some(hook), plugin_ops.as_ref(), &record)
        .await
        .expect("finalize hook ok");

    restore_mock.assert_calls_async(1).await;

    let scaling = proxmox_resource_scaling_record::Entity::find()
        .all(&db)
        .await
        .unwrap();
    assert_eq!(scaling.len(), 1);
    assert_eq!(scaling[0].restore_status, "restored");
}
```

- [ ] **Step 2: Run, observe pass**

Run: `cargo test -p uptrakit-functional-tests --test proxmox_update_lifecycle post_update_resource_restore`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/core/functional-tests/tests/proxmox_update_lifecycle.rs
git commit -m "$(cat <<'EOF'
test(functional-tests): post-update resource restore

Test 6: invokes `finalize_post_update_hook` directly against a seeded
`pending` scaling record. Asserts PUT /config body carries restored cores
and memory, and the record transitions to `restored`.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 15: Quality gates pass

**Files:**

- (verification only; no code changes expected)

Snapshot rules applied: full QA suite per `docs/development/quality-gates.md`.

- [ ] **Step 1: Format**

Run: `cargo fmt --all`
Expected: no changes; if any reformatting happens, stage and commit it.

- [ ] **Step 2: Both clippy passes**

Run in parallel:

```bash
cargo clippy -p uptrakit-functional-tests --all-targets --no-default-features --features db-sqlite
cargo clippy -p uptrakit-functional-tests --all-targets --all-features
```

Expected: both PASS with no warnings.

- [ ] **Step 3: Check both feature combinations workspace-wide for ripple effects**

Run in parallel:

```bash
cargo check --no-default-features --features db-sqlite
cargo check --all-features
```

Expected: both PASS.

- [ ] **Step 4: Full test run**

Run: `cargo test --all-features -p uptrakit-functional-tests`
Expected: all 6 lifecycle tests + the 5 helper tests PASS.

- [ ] **Step 5: Sentrux architectural check (only if hook installed locally)**

Run: `sentrux check 2>/dev/null || echo "(sentrux not available; pre-push hook will run it)"`
Expected: PASS or skip.

- [ ] **Step 6: Markdownlint on this plan and spec**

Run: `markdownlint --config .markdownlint.json docs/superpowers/specs/2026-05-20-update-lifetime-functional-tests.md docs/superpowers/plans/2026-05-20-update-lifetime-functional-tests.md`
Expected: PASS.

- [ ] **Step 7: If any formatting fixes were made, commit them**

```bash
git add -A
git status
git diff --cached --quiet || git commit -m "$(cat <<'EOF'
chore(functional-tests): quality-gate fixups

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Documentation Impact

Per spec section "Documentation Impact":

- **Task 2 already adds** the required module-level `//!` docstring on the new `pub mod testing { ... }` block in `crates/ui/controller-core/src/lib.rs` (test-only / non-semver contract). No further doc tasks needed.
- **No** `CONTEXT.md`, ADR, README, public docstring, or runbook changes — orchestration behavior, wire types, schema, and runtime contract are unchanged.
- **No** glossary additions.
- **No** proxmox-crate visibility changes (real `PluginCatalog` calls the existing `pub(crate)` constructors from inside the proxmox crate).

Carry-forward justification: this is internal test infrastructure adding zero user-observable behavior. The single test-only re-export documents itself per Task 2 Step 2.

---

## Self-Review

**1. Spec coverage**

| Spec section                                              | Plan task(s)                                                            |
| --------------------------------------------------------- | ----------------------------------------------------------------------- |
| Goal 1 — snapshot before dispatch                         | Task 9                                                                  |
| Goal 2 — backup before dispatch                           | Task 10                                                                 |
| Goal 3 — scaling before dispatch                          | Tasks 9, 12 (delta + absolute)                                          |
| Goal 4 — `ExecuteUpdate` payload shape                    | Task 13                                                                 |
| Goal 5 — restore on `finalize_post_update_hook`           | Task 14                                                                 |
| Goal 6 — no-mapping passthrough                           | Task 11                                                                 |
| Crate scaffold + Cargo.toml                               | Task 1                                                                  |
| `release-plz.toml` stanza                                 | Already committed in `33e25b69a`; spec note retained.                   |
| `controller-core` `pub mod testing` re-export             | Task 2                                                                  |
| `support/db.rs`                                           | Task 3                                                                  |
| `support/fixtures.rs` base + builder                      | Tasks 4, 5                                                              |
| `support/fixtures.rs` per-test rows                       | Task 7                                                                  |
| `support/stubs.rs`                                        | Task 6                                                                  |
| CAS sentinel block                                        | Task 8                                                                  |
| Synchronization note (no `update_output_line` assertions) | Honored implicitly in Tasks 9–14 — none assert on `update_output_line`. |
| Lint suppression (`#![expect(...)]`)                      | Task 3 Step 1 (inserts the block at the top of the lifecycle file).     |
| Quality gates                                             | Task 15                                                                 |
| Documentation impact                                      | Task 2 Step 2                                                           |

No gaps.

**2. Placeholder scan**

No "TBD", "TODO", "implement later", or "similar to Task N" anywhere. Every code step contains complete code. One implementer-note block in Task 6 acknowledges that `NoopOutputStream`'s method bodies depend on a trait shape that varies with the public surface and points the implementer at the source file — this is a _specific_ instruction, not a placeholder. Same for the entity-column notes in Tasks 4 and 7: they direct the implementer to the schema source rather than guessing.

**3. Type consistency**

- `TestFixtures` field names (`tenant_id`, `host_id`, `service_id`, `software_item_id`, `host_software_item_id`, `shell_config_id`, `proxmox_config_id`, `execute_update_plugin_id`, `detect_version_plugin_id`, `update_history_id`) are referenced consistently across Tasks 4–14.
- `build_plugin_ops(bool)` signature consistent across Tasks 6, 9–14.
- `TestNotificationSetup::new(Uuid) -> Self` + `.notification_state` field + `.captured_messages() -> Vec<ControllerMessage>` consistent across Tasks 6, 9–14.
- `assert_cas_sentinel(&DatabaseConnection, Uuid)` consistent across Tasks 8–14.
- `pending_work(&self, &DatabaseConnection, &str) -> PendingProtectionWork` consistent across Tasks 5, 9–14.
- Per-test helpers (`insert_proxmox_mapping`, `insert_protection_default_{snapshot,backup,do_nothing}`, `insert_scaling_default_{delta,absolute}`, `insert_backup_target_cache`) consistent across Tasks 7, 9, 10, 12.

All types align.

**4. Post-draft idiom audit**

- Tasks 9–14 wire `run_protection_and_dispatch` directly — idiomatic for the chosen test boundary; no framework-fighting.
- Lint suppression (Task 3 Step 1) uses `#![expect(...)]` with `reason = "..."`, matching the workspace `allow_attributes_without_reason = deny` rule. No `#[allow]` anywhere.
- Real `PluginCatalog` (Task 6) over hand-rolled stubs — production type reuse, idiomatic.
- `bail!()`/`report!()` not needed in this plan since test code uses `.unwrap()`/`.expect()` under the module-level expect block (consistent with `integration-tests` precedent).
- httpmock async methods (`mock_async`, `assert_calls_async`, `hits_async`, `start_async`) used consistently — idiomatic for `#[tokio::test]` contexts.

**5. Dependency version audit**

No new external dependencies introduced. Every dev-dep in Task 1's `Cargo.toml` uses `workspace = true` — versions are pinned at the workspace root (`Cargo.toml`), not in this crate. Nothing to bump here.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-20-update-lifetime-functional-tests.md`. Two execution options:

**1. Subagent-Driven (recommended)** — fresh subagent per task, two-stage review between tasks, fast iteration

**2. Inline Execution** — execute tasks in this session using `executing-plans`, batched checkpoints

Which approach?
