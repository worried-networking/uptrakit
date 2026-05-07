# RouterOS Support — Plan B: agent-ssh Wiring

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire RouterOS into agent-ssh: a typed executor (`RouterOsSshExecutor`), DB schema for `allow_reboot`, a RouterOS-specific bootstrap flow,
auto-detection in `bootstrap_connect`, and per-host runtime selection in `client.rs` so that version-check and update operations reach the correct
runtime.

**Architecture:** Plan A left `client.rs` always building `StandardHostRuntime`. This plan replaces that with a two-path dispatch: check for a
`routeros_host_config` row; if present, build `RouterOsSshExecutor` + `RouterOsHostRuntime`; otherwise build `PosixSshCommandExecutor` +
`StandardHostRuntime`. Bootstrap auto-detection runs `/system resource print` before executing any plan and routes to either `bootstrap_routeros.rs`
or the existing POSIX bootstrap.

**Tech Stack:** Rust (async/tokio), SeaORM (SQLite), russh, russh-sftp, rootcause

**Prerequisites:** Plan A must be merged before starting this plan.

---

## Tasks

### Task 1: Add RouterOsSshExecutor wrapping the base SshCommandExecutor

**Files:**

- Create: `crates/core/agent-ssh/src/routeros_executor.rs`
- Modify: `crates/core/agent-ssh/src/lib.rs` — add `mod routeros_executor;`

- [ ] **Step 1: Create routeros_executor.rs**

```rust
//! RouterOS-specific SSH executor.
//!
//! [`RouterOsSshExecutor`] wraps [`SshCommandExecutor`] with typed RouterOS CLI
//! methods and implements [`RouterOsExecutor`] from `plugin-infrastructure-core`.

use std::sync::Arc;
use std::time::Duration;

use uptrakit_plugin_infrastructure_core::{PluginError, RouterOsExecutor};

use crate::ssh_executor::SshCommandExecutor;
use crate::ssh_transport::SshSession;

/// Timeout for individual RouterOS CLI commands.
const ROS_CMD_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) struct RouterOsSshExecutor {
    inner: SshCommandExecutor,
}

impl RouterOsSshExecutor {
    pub(crate) fn new(session: Arc<SshSession>) -> Self {
        Self {
            inner: SshCommandExecutor::new(session),
        }
    }
}

#[async_trait::async_trait]
impl RouterOsExecutor for RouterOsSshExecutor {
    async fn resource_print(&self) -> std::result::Result<String, PluginError> {
        self.inner
            .exec_raw("/system resource print", Some(ROS_CMD_TIMEOUT))
            .await
            .map_err(|e| PluginError::PluginInternal(e.to_string()))
    }

    async fn routerboard_print(&self) -> std::result::Result<String, PluginError> {
        self.inner
            .exec_raw("/system routerboard print", Some(ROS_CMD_TIMEOUT))
            .await
            .map_err(|e| PluginError::PluginInternal(e.to_string()))
    }

    async fn license_print(&self) -> std::result::Result<String, PluginError> {
        self.inner
            .exec_raw("/system license print", Some(ROS_CMD_TIMEOUT))
            .await
            .map_err(|e| PluginError::PluginInternal(e.to_string()))
    }

    async fn check_for_updates(&self) -> std::result::Result<(), PluginError> {
        self.inner
            .exec_raw("/system package update check-for-updates", Some(ROS_CMD_TIMEOUT))
            .await
            .map(|_| ())
            .map_err(|e| PluginError::PluginInternal(e.to_string()))
    }

    async fn package_update_print(&self) -> std::result::Result<String, PluginError> {
        self.inner
            .exec_raw("/system package update print", Some(ROS_CMD_TIMEOUT))
            .await
            .map_err(|e| PluginError::PluginInternal(e.to_string()))
    }

    async fn package_install(&self) -> std::result::Result<(), PluginError> {
        self.inner
            .exec_raw("/system package update install", Some(ROS_CMD_TIMEOUT))
            .await
            .map(|_| ())
            .map_err(|e| PluginError::PluginInternal(e.to_string()))
    }

    async fn package_download(&self) -> std::result::Result<(), PluginError> {
        self.inner
            .exec_raw("/system package update download", Some(ROS_CMD_TIMEOUT))
            .await
            .map(|_| ())
            .map_err(|e| PluginError::PluginInternal(e.to_string()))
    }
}

/// Parse a `key: value` line from RouterOS CLI output.
///
/// Both `key` and `value` are trimmed. Returns `None` if the key is absent.
pub(crate) fn parse_routeros_field<'a>(output: &'a str, key: &str) -> Option<&'a str> {
    for line in output.lines() {
        if let Some(rest) = line.trim_start().strip_prefix(key) {
            if let Some(val) = rest.strip_prefix(':') {
                return Some(val.trim());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_routeros_field_found() {
        let output = "version: 7.14.2 (stable)\nplatform: MikroTik\n";
        assert_eq!(parse_routeros_field(output, "version"), Some("7.14.2 (stable)"));
    }

    #[test]
    fn parse_routeros_field_missing() {
        let output = "platform: MikroTik\n";
        assert_eq!(parse_routeros_field(output, "version"), None);
    }

    #[test]
    fn parse_routeros_field_trims_whitespace() {
        let output = "  serial-number:  ABC123  \n";
        assert_eq!(parse_routeros_field(output, "serial-number"), Some("ABC123"));
    }
}
```

- [ ] **Step 2: Add mod declaration in lib.rs**

In `crates/core/agent-ssh/src/lib.rs`, add:

```rust
pub(crate) mod routeros_executor;
```

- [ ] **Step 3: Build and test**

```bash
cargo test -p uptrakit-agent-ssh routeros_executor
```

Expected: `parse_routeros_field` tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/core/agent-ssh/src/routeros_executor.rs crates/core/agent-ssh/src/lib.rs
git commit -m "feat(agent-ssh): add RouterOsSshExecutor with typed RouterOS CLI methods"
```

---

### Task 2: DB migration — routeros_host_config table

**Files:**

- Create: `crates/core/agent-ssh/src/db/migration/m20260507_000001_add_routeros_host_config.rs`
- Modify: `crates/core/agent-ssh/src/db/migration/mod.rs`

- [ ] **Step 1: Create migration file**

```rust
use sea_orm_migration::prelude::*;

pub(super) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260507_000001_add_routeros_host_config"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE routeros_host_config (
                    ssh_host_id  BLOB    NOT NULL PRIMARY KEY
                                         REFERENCES ssh_hosts(id) ON DELETE CASCADE,
                    allow_reboot INTEGER NOT NULL DEFAULT 0
                )",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS routeros_host_config")
            .await?;
        Ok(())
    }
}
```

- [ ] **Step 2: Register migration in mod.rs**

Add `mod m20260507_000001_add_routeros_host_config;` to the `mod` list.

In `fn migrations()`, append to the `vec!`:

```rust
Box::new(m20260507_000001_add_routeros_host_config::Migration),
```

- [ ] **Step 3: Build**

```bash
cargo check -p uptrakit-agent-ssh
```

Expected: compiles.

- [ ] **Step 4: Commit**

```bash
git add crates/core/agent-ssh/src/db/migration/
git commit -m "feat(agent-ssh): add routeros_host_config migration"
```

---

### Task 3: DB entity — routeros_host_config

**Files:**

- Create: `crates/core/agent-ssh/src/db/entity/routeros_host_config.rs`
- Modify: `crates/core/agent-ssh/src/db/entity/mod.rs`

- [ ] **Step 1: Create entity**

```rust
use sea_orm::entity::prelude::*;

/// Persisted RouterOS-specific host configuration.
///
/// Keyed by `ssh_host_id` (FK to `ssh_hosts.id` ON DELETE CASCADE).
/// Created during RouterOS bootstrap; absent for POSIX hosts.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "routeros_host_config")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub ssh_host_id: uuid::Uuid,
    /// Whether the `uptrakit` RouterOS group has the `reboot` policy.
    /// Set once at bootstrap time — cannot change without re-bootstrapping.
    pub allow_reboot: bool,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::ssh_host::Entity",
        from = "Column::SshHostId",
        to = "super::ssh_host::Column::Id",
        on_delete = "Cascade"
    )]
    SshHost,
}

impl ActiveModelBehavior for ActiveModel {}

impl Related<super::ssh_host::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::SshHost.def()
    }
}
```

- [ ] **Step 2: Add to entity mod.rs**

In `crates/core/agent-ssh/src/db/entity/mod.rs`:

```rust
pub mod routeros_host_config;
```

- [ ] **Step 3: Build and verify**

```bash
cargo check -p uptrakit-agent-ssh
```

- [ ] **Step 4: Commit**

```bash
git add crates/core/agent-ssh/src/db/entity/
git commit -m "feat(agent-ssh): add routeros_host_config SeaORM entity"
```

---

### Task 4: Add collect_remote_host_info_routeros to host_info.rs

**Files:**

- Modify: `crates/core/agent-ssh/src/host_info.rs`

- [ ] **Step 1: Write failing tests first**

At the end of `host_info.rs` tests module, add:

```rust
// ── RouterOS machine-ID extraction ───────────────────────────────────

#[test]
fn routerboard_serial_extracts_correctly() {
    let output = "routerboard: yes\nserial-number: ABC123XYZ\ncurrent-firmware: 7.14.2\n";
    assert_eq!(
        extract_machine_id_routerboard(output),
        Some("ABC123XYZ".to_string())
    );
}

#[test]
fn routerboard_serial_missing_returns_none() {
    let output = "routerboard: yes\ncurrent-firmware: 7.14.2\n"; // no serial-number
    assert_eq!(extract_machine_id_routerboard(output), None);
}

#[test]
fn license_software_id_extracts_correctly() {
    let output = "software-id: ABCD-EFGH\nlevel: 6\n";
    assert_eq!(
        extract_machine_id_license(output),
        Some("ABCD-EFGH".to_string())
    );
}
```

Run: `cargo test -p uptrakit-agent-ssh host_info -- --nocapture` Expected: FAIL (functions not defined yet).

- [ ] **Step 2: Implement the extraction helpers and the collection function**

```rust
use crate::routeros_executor::{RouterOsSshExecutor, parse_routeros_field};
use uptrakit_shared_types::{OsFamily, host_features};
use uptrakit_wire::HostInfo;

/// Parse `serial-number` from `/system routerboard print` output.
pub(crate) fn extract_machine_id_routerboard(output: &str) -> Option<String> {
    let val = parse_routeros_field(output, "serial-number")?;
    if val.is_empty() { None } else { Some(val.to_string()) }
}

/// Parse `software-id` from `/system license print` output.
pub(crate) fn extract_machine_id_license(output: &str) -> Option<String> {
    let val = parse_routeros_field(output, "software-id")?;
    if val.is_empty() { None } else { Some(val.to_string()) }
}

/// Collect host information from a RouterOS device via its typed executor.
///
/// Sets `os_type = "routeros"` and `features = ["router_os_cli"]` so that
/// `HostCapabilities::new` correctly populates `OsFamily::RouterOs` and the
/// `ROUTER_OS_CLI` feature.
pub(crate) async fn collect_remote_host_info_routeros(
    exec: &RouterOsSshExecutor,
) -> HostInfo {
    let machine_id = collect_routeros_machine_id(exec).await;

    HostInfo {
        machine_id,
        os_type: Some("routeros".to_string()),
        os_version: None,
        architecture: None,
        hostname: None,
        ip_address: None,
        agent_host_id: None,
        features: Some(vec![host_features::ROUTER_OS_CLI.as_str().to_string()]),
    }
}

async fn collect_routeros_machine_id(exec: &RouterOsSshExecutor) -> String {
    // Attempt 1: routerboard serial-number
    if let Ok(output) = exec.routerboard_print().await {
        if let Some(id) = extract_machine_id_routerboard(&output) {
            return id;
        }
    }
    // Attempt 2: license software-id
    if let Ok(output) = exec.license_print().await {
        if let Some(id) = extract_machine_id_license(&output) {
            return id;
        }
    }
    // Fallback
    let fallback = format!("unknown-{}", uuid::Uuid::now_v7());
    tracing::warn!(
        fallback,
        "RouterOS machine-ID could not be determined; using session-unique fallback"
    );
    fallback
}
```

> **Note:** `HostInfo.features` is `Option<Vec<String>>` — confirm the field name matches the actual wire type definition in
> `uptrakit_wire::HostInfo`.

- [ ] **Step 3: Run tests**

```bash
cargo test -p uptrakit-agent-ssh host_info
```

Expected: all pass.

- [ ] **Step 4: Commit**

```bash
git add crates/core/agent-ssh/src/host_info.rs
git commit -m "feat(agent-ssh): add collect_remote_host_info_routeros and machine-ID extraction helpers"
```

---

### Task 5: Create bootstrap_routeros.rs — plan + execute

**Files:**

- Create: `crates/core/agent-ssh/src/operations/bootstrap_routeros.rs`
- Modify: `crates/core/agent-ssh/src/operations/mod.rs` — add `pub(crate) mod bootstrap_routeros;`

- [ ] **Step 1: Write failing tests for plan_bootstrap_routeros**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn stub_params() -> RouterOsBootstrapParams {
        RouterOsBootstrapParams {
            name: "test-router".to_string(),
            hostname: "192.168.1.1".to_string(),
            port: 22,
            auth_username: "admin".to_string(),
            auth_password: None,
            auth_private_key_pem: None,
            use_ssh_agent: false,
            host_key_fingerprint: None,
            strict_host_key_checking: false,
            allow_reboot: false,
            host_id: uuid::Uuid::nil(),
        }
    }

    #[test]
    fn plan_includes_reboot_policy_when_allowed() {
        let params = RouterOsBootstrapParams { allow_reboot: true, ..stub_params() };
        let plan = plan_bootstrap_routeros(&params);
        let policies = plan.iter().find_map(|a| match a {
            RouterOsPlannedAction::CreateGroup { policies } => Some(policies),
            _ => None,
        });
        assert!(policies.unwrap().iter().any(|p| p == "reboot"));
    }

    #[test]
    fn plan_excludes_reboot_policy_when_not_allowed() {
        let plan = plan_bootstrap_routeros(&stub_params()); // allow_reboot: false
        let policies = plan.iter().find_map(|a| match a {
            RouterOsPlannedAction::CreateGroup { policies } => Some(policies),
            _ => None,
        });
        assert!(!policies.unwrap().iter().any(|p| p == "reboot"));
    }

    #[test]
    fn plan_upload_precedes_import_precedes_delete() {
        use std::mem::discriminant;
        let plan = plan_bootstrap_routeros(&stub_params());
        let ds: Vec<_> = plan.iter().map(|a| discriminant(a)).collect();
        let upload = ds.iter().position(|&d| d == discriminant(&RouterOsPlannedAction::UploadPublicKey { remote_path: String::new() })).unwrap();
        let import = ds.iter().position(|&d| d == discriminant(&RouterOsPlannedAction::ImportSshKey { remote_path: String::new() })).unwrap();
        let delete = ds.iter().position(|&d| d == discriminant(&RouterOsPlannedAction::DeletePublicKey { remote_path: String::new() })).unwrap();
        assert!(upload < import && import < delete);
    }

    #[test]
    fn plan_default_policies_are_read_test_update() {
        let plan = plan_bootstrap_routeros(&stub_params());
        let policies = plan.iter().find_map(|a| match a {
            RouterOsPlannedAction::CreateGroup { policies } => Some(policies.clone()),
            _ => None,
        });
        let p = policies.unwrap();
        assert!(p.iter().any(|s| s == "read"));
        assert!(p.iter().any(|s| s == "test"));
        assert!(p.iter().any(|s| s == "update"));
    }
}
```

Run: `cargo test -p uptrakit-agent-ssh bootstrap_routeros` Expected: FAIL (module not yet created).

- [ ] **Step 2: Create bootstrap_routeros.rs**

```rust
//! RouterOS bootstrap: user/group creation, SSH key upload, host entry persistence.

use std::sync::Arc;

use rootcause::prelude::*;
use sea_orm::DatabaseConnection;
use uptrakit_shared_types::SecretString;

use crate::db::entity::routeros_host_config;
use crate::error::{Error, Result};
use crate::routeros_executor::RouterOsSshExecutor;
use crate::ssh_executor::SshCommandExecutor;
use crate::ssh_key;
use crate::ssh_transport::SshSession;
use crate::{host_ops, host_info};

/// Key file path used on the router during bootstrap.
const KEY_REMOTE_PATH: &str = "uptrakit-bootstrap.pub";

// ── Bootstrap params ─────────────────────────────────────────────────

pub(crate) struct RouterOsBootstrapParams {
    pub name: String,
    pub hostname: String,
    pub port: i32,
    pub auth_username: String,
    pub auth_password: Option<SecretString>,
    pub auth_private_key_pem: Option<SecretString>,
    pub use_ssh_agent: bool,
    pub host_key_fingerprint: Option<String>,
    pub strict_host_key_checking: bool,
    /// Default true — wizard pre-checks this; operator may uncheck before confirming.
    pub allow_reboot: bool,
    /// Pre-generated UUID for the new host DB entry.
    pub host_id: uuid::Uuid,
}

// ── Planned actions ──────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(crate) enum RouterOsPlannedAction {
    CreateGroup { policies: Vec<String> },
    CreateUser,
    UploadPublicKey { remote_path: String },
    ImportSshKey { remote_path: String },
    DeletePublicKey { remote_path: String },
    SaveHostEntry,
}

/// Build the ordered plan for RouterOS bootstrap.
///
/// Does not touch the router — pure data transformation.
pub(crate) fn plan_bootstrap_routeros(
    params: &RouterOsBootstrapParams,
) -> Vec<RouterOsPlannedAction> {
    let mut policies = vec![
        "read".to_string(),
        "test".to_string(),
        "update".to_string(),
    ];
    if params.allow_reboot {
        policies.push("reboot".to_string());
    }

    vec![
        RouterOsPlannedAction::CreateGroup { policies },
        RouterOsPlannedAction::CreateUser,
        RouterOsPlannedAction::UploadPublicKey {
            remote_path: KEY_REMOTE_PATH.to_string(),
        },
        RouterOsPlannedAction::ImportSshKey {
            remote_path: KEY_REMOTE_PATH.to_string(),
        },
        RouterOsPlannedAction::DeletePublicKey {
            remote_path: KEY_REMOTE_PATH.to_string(),
        },
        RouterOsPlannedAction::SaveHostEntry,
    ]
}

// ── Execution ────────────────────────────────────────────────────────

/// Execute the RouterOS bootstrap plan.
///
/// Generates an Ed25519 key pair, uploads the public key via SFTP,
/// creates the `uptrakit` group + user, imports the key, removes the
/// temp file, and saves the host entry + `routeros_host_config` row.
pub(crate) async fn execute_bootstrap_routeros(
    params: &RouterOsBootstrapParams,
    session: Arc<SshSession>,
    db: &DatabaseConnection,
) -> Result<()> {
    let base_exec = SshCommandExecutor::new(Arc::clone(&session));
    let ros_exec = RouterOsSshExecutor::new(Arc::clone(&session));

    let plan = plan_bootstrap_routeros(params);

    // Generate Ed25519 key pair for the uptrakit agent user
    let (private_key_pem, public_key_openssh) =
        ssh_key::generate_ed25519_key_pair().map_err(|e| {
            report!(Error::Bootstrap(format!(
                "SSH key generation failed: {e}"
            )))
        })?;

    let public_key_bytes = public_key_openssh.as_bytes();

    for action in &plan {
        match action {
            RouterOsPlannedAction::CreateGroup { policies } => {
                let policy_str = policies.join(",");
                ros_exec
                    .create_group(&policy_str)
                    .await
                    .map_err(|e| report!(Error::Bootstrap(format!("create group failed: {e}"))))?;
            }
            RouterOsPlannedAction::CreateUser => {
                ros_exec
                    .create_user()
                    .await
                    .map_err(|e| report!(Error::Bootstrap(format!("create user failed: {e}"))))?;
            }
            RouterOsPlannedAction::UploadPublicKey { remote_path } => {
                base_exec
                    .sftp_put(remote_path, public_key_bytes)
                    .await
                    .map_err(|e| {
                        report!(Error::Bootstrap(format!(
                            "SFTP upload to '{remote_path}' failed: {e}"
                        )))
                    })?;
            }
            RouterOsPlannedAction::ImportSshKey { remote_path } => {
                ros_exec
                    .import_ssh_key(remote_path)
                    .await
                    .map_err(|e| {
                        report!(Error::Bootstrap(format!("SSH key import failed: {e}")))
                    })?;
            }
            RouterOsPlannedAction::DeletePublicKey { remote_path } => {
                // Best-effort: log on failure but do not abort bootstrap
                if let Err(e) = base_exec.sftp_remove(remote_path).await {
                    tracing::warn!(
                        remote_path,
                        error = %e,
                        "failed to delete temporary public key from router"
                    );
                }
            }
            RouterOsPlannedAction::SaveHostEntry => {
                save_routeros_host_entry(params, &private_key_pem, db).await?;
            }
        }
    }

    Ok(())
}

async fn save_routeros_host_entry(
    params: &RouterOsBootstrapParams,
    private_key_pem: &str,
    db: &DatabaseConnection,
) -> Result<()> {
    use uptrakit_crypto::EncryptedString;
    use sea_orm::ActiveValue::Set;

    let encrypted_key = EncryptedString::encrypt(private_key_pem).map_err(|e| {
        report!(Error::Bootstrap(format!("key encryption failed: {e}")))
    })?;

    // Save host entry using the existing host_ops infrastructure
    let add_params = host_ops::AddHostParams {
        id: params.host_id,
        name: params.name.clone(),
        hostname: params.hostname.clone(),
        port: params.port,
        username: "uptrakit".to_string(),
        private_key: encrypted_key,
        key_type: crate::db::entity::ssh_host::SshKeyType::Ed25519,
        host_key_fingerprint: params.host_key_fingerprint.clone(),
    };
    host_ops::add_host(db, add_params).await?;

    // Save routeros_host_config row
    let config = routeros_host_config::ActiveModel {
        ssh_host_id: Set(params.host_id),
        allow_reboot: Set(params.allow_reboot),
    };
    use sea_orm::EntityTrait as _;
    routeros_host_config::Entity::insert(config)
        .exec(db)
        .await
        .map_err(|e| {
            report!(Error::Database(e))
        })?;

    Ok(())
}
```

> **Note:** Add `create_group`, `create_user`, `import_ssh_key` methods to `RouterOsSshExecutor` (they were listed in the spec but not in Task 1 of
> Plan B). Add them now in `routeros_executor.rs`:

```rust
// Add these to RouterOsSshExecutor impl:

pub(crate) async fn create_group(&self, policy_str: &str) -> crate::error::Result<()> {
    let cmd = format!("/user group add name=uptrakit policy={policy_str}");
    self.inner
        .exec_raw(&cmd, Some(ROS_CMD_TIMEOUT))
        .await
        .map(|_| ())
        .map_err(|e| rootcause::report!(crate::error::Error::Bootstrap(e.to_string())))
}

pub(crate) async fn create_user(&self) -> crate::error::Result<()> {
    self.inner
        .exec_raw(
            r#"/user add name=uptrakit group=uptrakit password="""#,
            Some(ROS_CMD_TIMEOUT),
        )
        .await
        .map(|_| ())
        .map_err(|e| rootcause::report!(crate::error::Error::Bootstrap(e.to_string())))
}

pub(crate) async fn import_ssh_key(&self, remote_path: &str) -> crate::error::Result<()> {
    let cmd = format!("/user ssh-keys import public-key-file={remote_path} user=uptrakit");
    self.inner
        .exec_raw(&cmd, Some(ROS_CMD_TIMEOUT))
        .await
        .map(|_| ())
        .map_err(|e| rootcause::report!(crate::error::Error::Bootstrap(e.to_string())))
}
```

- [ ] **Step 3: Verify ssh_key module has generate_ed25519_key_pair**

```bash
grep -n "pub.*fn generate_ed25519\|fn generate_ed25519" crates/core/agent-ssh/src/ssh_key.rs
```

If it doesn't exist, check what function actually generates keys and use that instead. The existing bootstrap.rs generates keys — look at how it calls
`ssh_key` and replicate.

- [ ] **Step 4: Run tests**

```bash
cargo test -p uptrakit-agent-ssh bootstrap_routeros
```

Expected: all tests from Step 1 pass.

- [ ] **Step 5: Build**

```bash
cargo check -p uptrakit-agent-ssh
```

Expected: compiles.

- [ ] **Step 6: Commit**

```bash
git add crates/core/agent-ssh/src/operations/bootstrap_routeros.rs crates/core/agent-ssh/src/operations/mod.rs crates/core/agent-ssh/src/routeros_executor.rs
git commit -m "feat(agent-ssh): add RouterOS bootstrap plan + execute (bootstrap_routeros.rs)"
```

---

### Task 6: Bootstrap auto-detection in bootstrap.rs

**Files:**

- Modify: `crates/core/agent-ssh/src/operations/bootstrap.rs`

- [ ] **Step 0: Add `allow_reboot` field to `BootstrapParams`**

`BootstrapParams` is `pub(crate)` in `bootstrap.rs` — safe to add a field. In `crates/core/agent-ssh/src/operations/bootstrap.rs`, add to the struct:

```rust
/// Whether to grant the `reboot` policy to the RouterOS `uptrakit` group.
/// Collected from the bootstrap wizard; default is `true` (wizard pre-checks it).
/// Only relevant for RouterOS hosts; ignored during POSIX bootstrap.
pub allow_reboot: bool,
```

In `crates/core/agent-ssh/src/surface_runtime.rs`, in the `parse_bootstrap_params` function (around line 1641), add:

```rust
allow_reboot: params
    .get("allow_reboot")
    .and_then(|v| v.as_bool())
    .unwrap_or(true),
```

Verify the new field is threaded through everywhere `BootstrapParams { ... }` is constructed:

```bash
grep -n "BootstrapParams {" crates/core/agent-ssh/src/
```

Add `allow_reboot: true` (or the resolved value) to every construction site.

- [ ] **Step 1: Write the failing test for detect_host_os**

Add to `bootstrap.rs` tests:

```rust
// ── detect_host_os ────────────────────────────────────────────────────

#[test]
fn host_os_enum_is_private() {
    // HostOs is a module-private enum — this test just verifies the
    // routing functions compile correctly. Integration tests cover the
    // full SSH probe path.
    let _ = HostOs::Posix;
    let _ = HostOs::RouterOs;
}
```

(The real `detect_host_os` cannot be unit-tested without a live SSH session — that's covered by integration tests.)

- [ ] **Step 2: Add HostOs enum and detect_host_os function**

Add near the top of `bootstrap.rs` (after imports):

```rust
/// Probe timeout for the OS-detection command.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Bootstrap routing decision.
#[derive(Debug)]
enum HostOs {
    RouterOs,
    Posix,
}

/// Probe the remote host to determine if it is RouterOS or a POSIX system.
///
/// Exit 0 + RouterOS marker in output → [`HostOs::RouterOs`].
/// Exit 0 + "not enough permissions" without POSIX error tokens → fail with
///   [`Error::Bootstrap`] containing a diagnostic hint.
/// Anything else → [`HostOs::Posix`].
async fn detect_host_os(exec: &SshCommandExecutor) -> Result<HostOs> {
    match exec.exec_raw("/system resource print", Some(PROBE_TIMEOUT)).await {
        Ok(output) if output.contains("platform:") || output.contains("MikroTik") => {
            Ok(HostOs::RouterOs)
        }
        Ok(output)
            if output.contains("not enough permissions")
                && !output.contains("No such file or directory")
                && !output.contains("command not found")
                && !output.contains("Permission denied") =>
        {
            Err(report!(Error::Bootstrap(
                "RouterOS device detected but insufficient permissions for \
                 `/system resource print` — grant `read` policy to the connecting account"
                    .to_string()
            )))
        }
        _ => Ok(HostOs::Posix),
    }
}
```

- [ ] **Step 3: Wire detection into bootstrap_connect**

In `bootstrap_connect`, after `prepare_bootstrap_connection` returns the session and before `gather_remote_host_info`, add:

```rust
use crate::ssh_executor::SshCommandExecutor;
use crate::operations::bootstrap_routeros;

let base_exec = SshCommandExecutor::new(Arc::clone(&session));
let host_os = detect_host_os(&base_exec).await?;

match host_os {
    HostOs::RouterOs => {
        let ros_params = bootstrap_routeros::RouterOsBootstrapParams {
            name: params.name.clone(),
            hostname: params.hostname.clone(),
            port: params.port,
            auth_username: params.auth_username.clone(),
            auth_password: params.auth_password.clone(),
            auth_private_key_pem: params.auth_private_key_pem.clone(),
            use_ssh_agent: params.use_ssh_agent,
            host_key_fingerprint: params.host_key_fingerprint.clone(),
            strict_host_key_checking: params.strict_host_key_checking,
            allow_reboot: params.allow_reboot,
            host_id: params.host_id,
        };
        // Build RouterOS plan for user review
        let ros_plan = bootstrap_routeros::plan_bootstrap_routeros(&ros_params);
        let actions = routeros_planned_actions_to_planned_actions(ros_plan);
        let host_info = BootstrapHostInfo {
            hostname: params.hostname.clone(),
            port,
            auth_user: params.auth_username.clone(),
            is_root: false,
            os_info: Some("MikroTik RouterOS".to_string()),
            host_key_fingerprint: observed_fp,
            target_user_exists: false,
        };
        SshSession::disconnect_shared(session).await;
        return Ok(BootstrapPlan { host_info, actions });
    }
    HostOs::Posix => {
        // fall through to existing POSIX gather_remote_host_info path
    }
}
```

Add `routeros_planned_actions_to_planned_actions` helper that maps `RouterOsPlannedAction` variants to the existing `PlannedAction` struct:

```rust
fn routeros_planned_actions_to_planned_actions(
    ros_actions: Vec<bootstrap_routeros::RouterOsPlannedAction>,
) -> Vec<PlannedAction> {
    use bootstrap_routeros::RouterOsPlannedAction as A;
    ros_actions.into_iter().map(|a| match a {
        A::CreateGroup { policies } => PlannedAction {
            id: "create_group".to_string(),
            label: "Create uptrakit group".to_string(),
            description: format!("Create RouterOS group `uptrakit` with policies: {}", policies.join(", ")),
            security_impact: uptrakit_shared_types::Severity::Medium,
            default_enabled: true,
            skippable: false,
            commands: vec![format!("/user group add name=uptrakit policy={}", policies.join(","))],
        },
        A::CreateUser => PlannedAction {
            id: "create_user".to_string(),
            label: "Create uptrakit user".to_string(),
            description: "Create RouterOS user `uptrakit` in the `uptrakit` group.".to_string(),
            security_impact: uptrakit_shared_types::Severity::Medium,
            default_enabled: true,
            skippable: false,
            commands: vec![r#"/user add name=uptrakit group=uptrakit password="""#.to_string()],
        },
        A::UploadPublicKey { remote_path } => PlannedAction {
            id: "upload_public_key".to_string(),
            label: "Upload SSH public key".to_string(),
            description: format!("SFTP-upload Ed25519 public key to `{remote_path}` on the router."),
            security_impact: uptrakit_shared_types::Severity::Low,
            default_enabled: true,
            skippable: false,
            commands: vec![format!("sftp: put uptrakit-bootstrap.pub {remote_path}")],
        },
        A::ImportSshKey { remote_path } => PlannedAction {
            id: "import_ssh_key".to_string(),
            label: "Import SSH key".to_string(),
            description: "Import the uploaded public key for the `uptrakit` user.".to_string(),
            security_impact: uptrakit_shared_types::Severity::Low,
            default_enabled: true,
            skippable: false,
            commands: vec![format!("/user ssh-keys import public-key-file={remote_path} user=uptrakit")],
        },
        A::DeletePublicKey { remote_path } => PlannedAction {
            id: "delete_public_key".to_string(),
            label: "Delete temporary public key file".to_string(),
            description: format!("Remove `{remote_path}` from the router filesystem."),
            security_impact: uptrakit_shared_types::Severity::Low,
            default_enabled: true,
            skippable: true,
            commands: vec![format!("sftp: rm {remote_path}")],
        },
        A::SaveHostEntry => PlannedAction {
            id: "save_host_entry".to_string(),
            label: "Save host entry".to_string(),
            description: "Persist the host and private key in the agent-ssh local database.".to_string(),
            security_impact: uptrakit_shared_types::Severity::Low,
            default_enabled: true,
            skippable: false,
            commands: vec![],
        },
    }).collect()
}
```

Also wire `execute_bootstrap_routeros` into `run_bootstrap` (the second phase that actually executes the plan). After the user confirms the plan, the
execution phase calls `execute_bootstrap_routeros` for RouterOS hosts. The signal is: if `params.host_id` maps to a `routeros_host_config` row OR if a
session re-probe identifies RouterOS, call the RouterOS path.

Simplest approach: repeat the `detect_host_os` probe in `run_bootstrap` (the execution phase) and dispatch accordingly:

```rust
// In run_bootstrap, after SSH session re-establishment:
let base_exec = SshCommandExecutor::new(Arc::clone(&session));
match detect_host_os(&base_exec).await? {
    HostOs::RouterOs => {
        bootstrap_routeros::execute_bootstrap_routeros(&ros_params, session, &db).await?;
    }
    HostOs::Posix => {
        // existing POSIX execute path
    }
}
```

> **Note:** `ros_params` needs `allow_reboot` from the confirmed user input. The CLI / UI passes this in the `BootstrapParams` for the execution
> phase. Add `allow_reboot: bool` to `BootstrapParams` with `#[serde(default = "default_true")]` (default true matches the wizard pre-check).

- [ ] **Step 4: Build**

```bash
cargo check -p uptrakit-agent-ssh 2>&1 | head -40
```

Fix any compilation errors before committing.

- [ ] **Step 5: Run all agent-ssh tests**

```bash
cargo test -p uptrakit-agent-ssh
```

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add crates/core/agent-ssh/src/operations/bootstrap.rs
git commit -m "feat(agent-ssh): auto-detect RouterOS in bootstrap_connect; route to RouterOS bootstrap"
```

---

### Task 7: Per-host runtime selection in client.rs

**Files:**

- Modify: `crates/core/agent-ssh/src/client.rs`

This is the final wiring step. Wherever `client.rs` currently passes a `StandardHostRuntime` placeholder (from Plan A Task 8), replace with a proper
dispatch: query `routeros_host_config` by `ssh_host_id`; if found, build `RouterOsHostRuntime`; else build `StandardHostRuntime`.

- [ ] **Step 1: Add helper function build_host_runtime**

```rust
use uptrakit_plugin_infrastructure_core::{
    HostCapabilities, HostRuntime, RouterOsSshExecutor, StandardHostRuntime,
    construct_routeros_host_runtime,
};
use crate::db::entity::routeros_host_config;
use crate::routeros_executor::RouterOsSshExecutor as AgentRouterOsSshExecutor;

/// Build the appropriate `HostRuntime` for an SSH host.
///
/// Checks `routeros_host_config` by host ID. If a row exists, wraps the
/// session in `RouterOsSshExecutor` and returns `RouterOsHostRuntime` with
/// the persisted `allow_reboot` flag. Otherwise returns `StandardHostRuntime`.
async fn build_host_runtime(
    host: &crate::db::entity::ssh_host::Model,
    session: Arc<crate::ssh_transport::SshSession>,
    executor: Arc<dyn uptrakit_command::CommandExecutor>,
    db: &sea_orm::DatabaseConnection,
) -> Arc<dyn HostRuntime> {
    use sea_orm::EntityTrait as _;

    match routeros_host_config::Entity::find_by_id(host.id)
        .one(db)
        .await
    {
        Ok(Some(ros_config)) => {
            let ros_exec = Arc::new(AgentRouterOsSshExecutor::new(Arc::clone(&session)));
            let caps = HostCapabilities::new(
                Some("routeros"),
                None,
                None,
                &[uptrakit_shared_types::host_features::ROUTER_OS_CLI.to_string()],
            );
            construct_routeros_host_runtime(ros_exec, caps, ros_config.allow_reboot)
        }
        Ok(None) => {
            // POSIX host — use the already-built SudoAware executor
            let caps = HostCapabilities::new(Some("linux"), None, None, &[]);
            Arc::new(StandardHostRuntime::new(executor, caps))
        }
        Err(e) => {
            tracing::warn!(
                host_id = %host.id,
                error = %e,
                "failed to query routeros_host_config; defaulting to StandardHostRuntime"
            );
            let caps = HostCapabilities::new(Some("linux"), None, None, &[]);
            Arc::new(StandardHostRuntime::new(executor, caps))
        }
    }
}
```

> **Note on caps for POSIX:** Passing `Some("linux")` as placeholder is acceptable — the HostCapabilities for POSIX hosts are not used for
> `HostRequirements` validation in the version-check path (the controller handles that). A more precise approach: read `host.os_type` from the DB if
> it's persisted there. If not stored, `Some("linux")` is a safe default for now.

- [ ] **Step 2: Replace all executor-based runtime construction in client.rs**

Find every occurrence of:

```rust
// The placeholder added in Plan A Task 8:
let caps = HostCapabilities::new(Some("linux"), None, None, &[]);
let runtime: Arc<dyn HostRuntime> = Arc::new(StandardHostRuntime::new(Arc::clone(&executor), caps));
```

Replace each with:

```rust
let runtime = build_host_runtime(&host, Arc::clone(&session), Arc::clone(&executor), &self.db).await;
```

(Pass `db` via `self.db` or however it's available in the call context.)

- [ ] **Step 3: Build**

```bash
cargo check -p uptrakit-agent-ssh --all-features 2>&1 | grep "^error" | head -20
```

- [ ] **Step 4: Run all tests**

```bash
cargo test -p uptrakit-agent-ssh
```

- [ ] **Step 5: Commit**

```bash
git add crates/core/agent-ssh/src/client.rs
git commit -m "feat(agent-ssh): select RouterOsHostRuntime or StandardHostRuntime per host in client.rs"
```

---

## Self-Review

**Spec coverage check:**

| Spec requirement                                                                    | Covered  |
| ----------------------------------------------------------------------------------- | -------- |
| RouterOsSshExecutor with typed methods                                              | Task 1 ✓ |
| parse_routeros_field helper in agent-ssh                                            | Task 1 ✓ |
| routeros_host_config DB migration                                                   | Task 2 ✓ |
| routeros_host_config entity                                                         | Task 3 ✓ |
| collect_remote_host_info_routeros; OsFamily::RouterOs + ROUTER_OS_CLI in HostInfo   | Task 4 ✓ |
| RouterOsBootstrapParams + RouterOsPlannedAction                                     | Task 5 ✓ |
| plan_bootstrap_routeros (group policies, key upload order)                          | Task 5 ✓ |
| execute_bootstrap_routeros (SFTP upload, create_group, create_user, import, delete) | Task 5 ✓ |
| detect_host_os() probe with two-gate + permission-denied guard                      | Task 6 ✓ |
| bootstrap_connect routing to RouterOS path                                          | Task 6 ✓ |
| allow_reboot wizard default = true                                                  | Task 6 ✓ |
| per-host runtime selection in client.rs                                             | Task 7 ✓ |

**Type consistency:** `RouterOsBootstrapParams.allow_reboot` defined in Task 5, read in Task 6. `RouterOsSshExecutor` defined in Task 1, used in Tasks
5 and 7. `routeros_host_config::Model.allow_reboot` defined in Task 3, read in Task 7.
