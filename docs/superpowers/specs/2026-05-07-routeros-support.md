# RouterOS Support

## Problem

MikroTik RouterOS hosts are unmanageable today. `OsFamily::RouterOs` and `host_features::ROUTER_OS_CLI` exist as groundwork stubs with doc comments
that say "no runtime implementation yet." There is no plugin, no bootstrap path, no executor, and no migration that persists RouterOS-specific
configuration. Operators managing RouterOS fleets cannot enroll, version-detect, or update any device.

## Approach

Four parallel tracks deliver full RouterOS support:

1. **SSH executor split** — rename the current `SshCommandExecutor` to `PosixSshCommandExecutor`; extract a thin base `SshCommandExecutor` (raw exec
   channel, no POSIX assumptions); add `RouterOsSshExecutor` on top of the base with typed RouterOS CLI methods. Extend `SshSession` with SFTP
   primitives (`sftp_put`, `sftp_remove`) using the `russh-sftp` workspace dependency (new).
2. **Bootstrap auto-detection** — the existing `bootstrap-connect` step runs `/system resource print` over SSH; exit 0 routes to a new RouterOS
   bootstrap plan; non-zero exit falls through to the existing POSIX plan. No user-facing type selector is added.
3. **`RouterOsHostRuntime`** — new runtime type in `plugin-infrastructure-core`, returned by `construct_host_runtime` when
   `caps.os_family == Some(OsFamily::RouterOs)`. Embeds `Arc<RouterOsSshExecutor>` and `allow_reboot: bool` (loaded from `routeros_host_config` by
   agent-ssh before calling `construct_host_runtime`). The RouterOS plugin downcasts `runtime.as_any()` to `RouterOsHostRuntime` to access both. No DB
   dependency in the plugin crate.
4. **RouterOS plugin** — new crate `crates/plugins/package-managers/routeros/` implementing `VersionDetector` + `UpdateExecutor` + `ReleaseFetcher`
   roles under `PluginFamily::Software`.

## Architecture

### `agent-ssh/src/ssh_executor.rs` — executor split

The current `SshCommandExecutor` is renamed to `PosixSshCommandExecutor` (internal; same file). A new base `SshCommandExecutor` exposes only raw SSH
exec channel access with no POSIX-specific command building or shell invocation:

```rust
/// Raw SSH exec channel. No POSIX assumptions.
pub(crate) struct SshCommandExecutor {
    session: Arc<SshSession>,
}

impl SshCommandExecutor {
    pub(crate) fn new(session: Arc<SshSession>) -> Self { ... }

    /// Execute a pre-formed command string and collect stdout/stderr.
    /// Returns raw stdout. Does not apply shell quoting or env var handling.
    pub(crate) async fn exec_raw(
        &self,
        cmd: &str,
        timeout: Option<Duration>,
    ) -> Result<String, SshExecError> { ... }

    /// Upload bytes to a remote path via SFTP.
    ///
    /// Requires `russh-sftp` (new workspace dependency). Opens an SFTP subsystem
    /// channel on the existing session; does not open a second SSH connection.
    pub(crate) async fn sftp_put(
        &self,
        remote_path: &str,
        data: &[u8],
    ) -> Result<(), SshExecError> { ... }

    /// Delete a remote file via SFTP.
    pub(crate) async fn sftp_remove(&self, remote_path: &str) -> Result<(), SshExecError> { ... }
}
```

`PosixSshCommandExecutor` wraps `SshCommandExecutor` and keeps all current `CommandExecutor` trait logic unchanged. All existing POSIX call sites are
updated from `SshCommandExecutor::new` to `PosixSshCommandExecutor::new`.

### `agent-ssh/src/routeros_executor.rs` — new file

`RouterOsSshExecutor` wraps `SshCommandExecutor` and exposes typed RouterOS CLI methods. Each method issues a single RouterOS command via `exec_raw`,
then parses the key-value output format (`key: value\n`).

```rust
pub(crate) struct RouterOsSshExecutor {
    inner: SshCommandExecutor,
}

impl RouterOsSshExecutor {
    pub(crate) fn new(session: Arc<SshSession>) -> Self { ... }

    /// `/system resource print` — returns raw output for version parsing.
    pub(crate) async fn resource_print(&self) -> Result<String, SshExecError> { ... }

    /// `/system routerboard print` — returns raw output for serial-number.
    pub(crate) async fn routerboard_print(&self) -> Result<String, SshExecError> { ... }

    /// `/system license print` — returns raw output for software-id fallback.
    pub(crate) async fn license_print(&self) -> Result<String, SshExecError> { ... }

    /// `/system package update check-for-updates`.
    pub(crate) async fn check_for_updates(&self) -> Result<(), SshExecError> { ... }

    /// `/system package update print` — returns raw output for latest-version.
    pub(crate) async fn package_update_print(&self) -> Result<String, SshExecError> { ... }

    /// `/user group add name=uptrakit policy=<policies>`.
    pub(crate) async fn create_group(&self, policies: &[&str]) -> Result<(), SshExecError> { ... }

    /// `/user add name=uptrakit group=uptrakit password=""`.
    pub(crate) async fn create_user(&self) -> Result<(), SshExecError> { ... }

    /// `/user ssh-keys import public-key-file=<path> user=uptrakit`.
    pub(crate) async fn import_ssh_key(&self, remote_path: &str) -> Result<(), SshExecError> { ... }

    /// `/system package update install` — triggers upgrade and reboot.
    pub(crate) async fn package_install(&self) -> Result<(), SshExecError> { ... }

    /// `/system package update download` — downloads without rebooting.
    pub(crate) async fn package_download(&self) -> Result<(), SshExecError> { ... }
}
```

A shared `parse_routeros_field(output: &str, key: &str) -> Option<&str>` free function handles the `key: value` line format used by all `print`
commands. Leading and trailing whitespace on both key and value is trimmed.

### `agent-ssh/src/host_info.rs` — RouterOS machine ID

Add `collect_remote_host_info_routeros(exec: &RouterOsSshExecutor)` alongside the existing POSIX equivalent. Machine ID resolution:

1. `routerboard_print()` → parse `serial-number` field → use as machine ID.
2. On parse failure or empty value: `license_print()` → parse `software-id` field.
3. On failure or empty: generate `unknown-<uuidv7>` and log a warning.

The returned `machine_id: String` is stored in `ssh_hosts.machine_id` using the existing column (no schema change needed for the host row).

`OsFamily::RouterOs` is set in `HostCapabilities` and `host_features::ROUTER_OS_CLI` is added to `HostCapabilities::features` when this path executes.

### `agent-ssh/src/operations/bootstrap.rs` — auto-detection

In the `bootstrap-connect` step, after the initial SSH session is established, probe the remote host:

```rust
async fn detect_host_os(
    exec: &SshCommandExecutor,
) -> Result<HostOs, BootstrapError> {
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
            Err(BootstrapError::RouterOsPermissionDenied {
                hint: "grant `read` policy to the connecting account".into(),
            })
        }
        _ => Ok(HostOs::Posix),
    }
}
```

`PROBE_TIMEOUT` is `Duration::from_secs(5)`. The function returns `Result<HostOs, BootstrapError>` so permission errors fail fast with a diagnostic
message rather than silently routing a RouterOS device through the POSIX path.

Three arms:

1. **Exit 0 + RouterOS marker** (`"platform:"` or `"MikroTik"` in output) → `RouterOs`. Prevents false-positive on POSIX hosts that accept unknown
   commands without error.
2. **Exit 0 + `"not enough permissions"` without POSIX error tokens** → `BootstrapError::RouterOsPermissionDenied`. RouterOS returns this string
   (lowercase, no trailing period) with exit 0 when the connecting account lacks the `read` policy. The negative guards
   (`"No such file or directory"`, `"command not found"`, `"Permission denied"`) exclude the most common POSIX restricted-shell false-positive
   patterns. This is a best-effort heuristic; a honeypot or exotic restricted shell could still trigger it.
3. **Anything else** (non-zero exit, connection error, unrecognised output) → `Posix`.

`HostOs` is a private enum `{ RouterOs, Posix }` used only within the bootstrap module to route plan construction. `HostOs::RouterOs` calls
`plan_bootstrap_routeros(params, &session)`. `HostOs::Posix` calls the existing `plan_bootstrap_posix(params, &session)`.

### `agent-ssh/src/operations/bootstrap_routeros.rs` — new file

```rust
pub(crate) struct RouterOsBootstrapParams {
    /// Inherited from outer BootstrapParams fields shared with POSIX path.
    pub(crate) name: String,
    pub(crate) hostname: String,
    pub(crate) port: i32,
    pub(crate) auth_username: String,
    pub(crate) auth_password: Option<SecretString>,
    pub(crate) auth_private_key_pem: Option<SecretString>,
    pub(crate) use_ssh_agent: bool,
    pub(crate) host_key_fingerprint: Option<String>,
    pub(crate) strict_host_key_checking: bool,
    /// Grant `/system reboot` policy to the uptrakit user group.
    /// Stored in `routeros_host_config.allow_reboot`. The bootstrap wizard pre-checks
    /// this option (defaults to true); the operator may uncheck it before confirming.
    pub(crate) allow_reboot: bool,
}

pub(crate) enum RouterOsPlannedAction {
    CreateGroup { policies: Vec<String> },
    CreateUser,
    UploadPublicKey { remote_path: String },
    ImportSshKey { remote_path: String },
    DeletePublicKey { remote_path: String },
    SaveHostEntry,
}
```

`plan_bootstrap_routeros` returns `Vec<RouterOsPlannedAction>`. The plan always includes `CreateGroup`, `CreateUser`, `UploadPublicKey`,
`ImportSshKey`, `DeletePublicKey`, `SaveHostEntry`. When `allow_reboot=true`, the `CreateGroup` action includes `"reboot"` in its policy list;
otherwise policies are `["read", "test", "update"]`.

`execute_bootstrap_routeros` runs the plan step by step. After `SaveHostEntry`, it inserts a row into `routeros_host_config` with `ssh_host_id` and
`allow_reboot`.

SSH key generation uses the existing `ssh_key` module (Ed25519). The public key is SFTP-uploaded via `SshCommandExecutor::sftp_put` to
`/uptrakit-bootstrap.pub` before `ImportSshKey`, then removed via `sftp_remove` after.

### `agent-ssh/src/db/migration/<next>_add_routeros_host_config.rs` — new migration

```sql
CREATE TABLE routeros_host_config (
    ssh_host_id  BLOB    NOT NULL PRIMARY KEY
                         REFERENCES ssh_hosts(id) ON DELETE CASCADE,
    allow_reboot INTEGER NOT NULL DEFAULT 0
);
```

Migration number follows the existing sequence. The `ssh_host_id` column is the full `uuid::Uuid` stored as `BLOB`, matching the `ssh_hosts.id` column
type.

### `agent-ssh/src/db/entity/routeros_host_config.rs` — new entity

```rust
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "routeros_host_config")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub ssh_host_id: uuid::Uuid,
    pub allow_reboot: bool,
}
```

`Relation` has a `BelongsTo` relation to `ssh_host::Entity`. `ActiveModelBehavior` is default.

### `shared/types/src/os_family.rs` and `host_feature.rs` — remove groundwork stubs

Remove "Groundwork only — no runtime implementation yet." from the doc comment on `OsFamily::RouterOs`. Replace with:

```rust
/// MikroTik RouterOS. Detected via `/system resource print` during bootstrap.
RouterOs,
```

Remove "Groundwork only — no runtime implementation yet." from the doc comment on `host_features::ROUTER_OS_CLI`. Replace with:

```rust
/// RouterOS CLI available. Set during bootstrap when `/system resource print` succeeds.
pub const ROUTER_OS_CLI: HostFeature = HostFeature::from_static("router_os_cli");
```

`ROUTER_OS_CLI` is intentionally absent from `PROBEABLE_FEATURES` — detection is non-POSIX and happens in the bootstrap probe, not via the standard
feature-probe loop. No change to `PROBEABLE_FEATURES`.

### `plugins/infrastructure/core/src/host_requirements.rs` — add constant

Add a `feature_arrays` entry and a named constant:

```rust
pub(super) static ROUTER_OS_CLI: [HostFeature; 1] = [host_features::ROUTER_OS_CLI];
```

```rust
/// RouterOS host with CLI access.
pub const ROUTER_OS: Self = Self::new(
    &[OsFamily::RouterOs],
    &feature_arrays::ROUTER_OS_CLI,
    false,
);
```

### `plugins/infrastructure/core/src/host_runtime.rs` — `RouterOsHostRuntime`

Add `RouterOsHostRuntime` alongside `StandardHostRuntime`. It carries the RouterOS-specific executor and the `allow_reboot` flag loaded from DB by
agent-ssh before construction. The plugin accesses both via `runtime.as_any()` downcast.

```rust
pub struct RouterOsHostRuntime {
    /// Typed RouterOS CLI executor wrapping the raw SSH base.
    routeros_exec: Arc<RouterOsSshExecutor>,
    capabilities: HostCapabilities,
    /// Loaded from `routeros_host_config.allow_reboot` by agent-ssh at construction.
    pub allow_reboot: bool,
}

impl RouterOsHostRuntime {
    pub fn new(
        routeros_exec: Arc<RouterOsSshExecutor>,
        caps: HostCapabilities,
        allow_reboot: bool,
    ) -> Self { ... }

    pub fn routeros_executor(&self) -> Arc<RouterOsSshExecutor> {
        Arc::clone(&self.routeros_exec)
    }
}

impl HostRuntime for RouterOsHostRuntime {
    fn capabilities(&self) -> &HostCapabilities { &self.capabilities }
    fn as_any(&self) -> &dyn std::any::Any { self }
    fn executor(&self) -> Arc<dyn CommandExecutor> {
        // RouterOS does not use CommandExecutor. Return a NoopCommandExecutor and
        // log an error so misuse is visible in traces without crashing the process.
        tracing::error!("RouterOsHostRuntime::executor() called — use routeros_executor()");
        Arc::new(uptrakit_command::NoopCommandExecutor)
    }
}
```

`construct_host_runtime` gains a new overload for RouterOS:

```rust
pub fn construct_routeros_host_runtime(
    routeros_exec: Arc<RouterOsSshExecutor>,
    caps: HostCapabilities,
    allow_reboot: bool,
) -> Arc<dyn HostRuntime> {
    Arc::new(RouterOsHostRuntime::new(routeros_exec, caps, allow_reboot))
}
```

Agent-ssh calls `construct_routeros_host_runtime` (instead of `construct_host_runtime`) when dispatching plugin roles for a host whose
`ssh_hosts.machine_id` maps to a `routeros_host_config` row. The existing `construct_host_runtime` body is unchanged.

`RouterOsSshExecutor` is defined in `agent-ssh` (crate-internal). `RouterOsHostRuntime` is defined in `plugin-infrastructure-core` and must reference
it. Since the executor lives in a different crate, `RouterOsHostRuntime` stores it as `Arc<dyn Any + Send + Sync>` and downcasts internally, OR a
shared `RouterOsExecutorTrait` pub trait is defined in `plugin-infrastructure-core` that `RouterOsSshExecutor` implements. The trait exposes the typed
methods the plugin needs. **Recommended:** define `pub trait RouterOsExecutor: Send + Sync + 'static { ... }` in `plugin-infrastructure-core` with the
same method signatures as `RouterOsSshExecutor`, and have `RouterOsSshExecutor` implement it. `RouterOsHostRuntime` stores
`Arc<dyn RouterOsExecutor>`.

### `plugins/package-managers/routeros/` — new crate

**`Cargo.toml`**

```toml
[package]
name = "uptrakit-package-manager-routeros"
version.workspace = true
edition.workspace = true

[dependencies]
uptrakit-plugin-infrastructure-core = { workspace = true }
uptrakit-shared-types = { workspace = true }
uptrakit-shared-macros = { workspace = true }
rootcause = { workspace = true }
async-trait = { workspace = true }
thiserror = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
tracing = { workspace = true }

[dev-dependencies]
tokio = { workspace = true, features = ["macros", "rt"] }

[lints]
workspace = true
```

**`src/lib.rs`** — re-exports `RouterOsPlugin` and calls `declare_plugin!`.

**`src/config.rs`**

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct RouterOsConfig {
    /// RouterOS update channel: `"stable"`, `"long-term"`, `"testing"`, or `None`
    /// to leave the channel unchanged on the router.
    #[serde(default)]
    pub channel: Option<String>,
    /// Whether to reboot after downloading the update. Has no effect if
    /// `routeros_host_config.allow_reboot` is `false` for the host
    /// (RouterOS group lacks the `reboot` policy).
    #[serde(default)]
    pub reboot: bool,
}
```

`RouterOsConfig` implements `Validate` (no field constraints; both fields have safe defaults).

**`src/version.rs`** — version parsing helpers:

```rust
/// Parse the `version:` field from `/system resource print` output.
/// Strips channel suffix in parentheses, e.g. `7.14.2 (stable)` → `7.14.2`.
pub fn parse_resource_version(output: &str) -> Option<String> { ... }

/// Parse the `latest-version:` field from `/system package update print` output.
pub fn parse_latest_version(output: &str) -> Option<String> { ... }
```

Both functions have a local copy of `parse_routeros_field` (a ~5-line helper). `routeros_executor.rs` lives in `agent-ssh` (crate-internal); the
plugin crate has no dependency on `agent-ssh`, so duplication is correct — do not re-export across the crate boundary. Both copies apply
`.split_once('(').map(|(v, _)| v.trim())` for channel-suffix stripping.

**`src/executor.rs`**

```rust
pub(crate) struct RouterOsUpdateExecutor {
    exec: Arc<RouterOsSshExecutor>,
    /// From `routeros_host_config` — whether the RouterOS `reboot` policy was
    /// granted at bootstrap time. Hard gate: reboot is impossible without it.
    allow_reboot: bool,
    /// From `RouterOsConfig.reboot` — whether to reboot after download.
    reboot: bool,
}
```

`execute_update()`:

```rust
if self.reboot && self.allow_reboot {
    self.exec.package_install().await
} else {
    self.exec.package_download().await
}
```

`allow_reboot` is loaded from `routeros_host_config` at executor construction time. `reboot` comes from `RouterOsConfig.reboot`.

**`src/plugin.rs`**

`RouterOsPlugin` implements `declare_plugin!` with:

- `plugin_type_id`: `"package_manager_routeros"`
- `family`: `PluginFamily::Software`
- `VersionDetector` role: `HostRequirements::ROUTER_OS`
- `UpdateExecutor` role: `HostRequirements::ROUTER_OS`
- `ReleaseFetcher` role: `HostRequirements::CONTROLLER_ONLY`

`detect_version`, `execute_update`, and `fetch_releases` all call `runtime.as_any().downcast_ref::<RouterOsHostRuntime>()` to obtain
`Arc<dyn RouterOsExecutor>`. `allow_reboot` is read directly from `RouterOsHostRuntime::allow_reboot`.

`detect_version` calls `routeros_exec.resource_print()` and delegates to `parse_resource_version`.

`fetch_releases` calls `routeros_exec.package_update_print()` only and delegates to `parse_latest_version`. It does NOT call `check_for_updates()` —
that triggers a network call from the router to MikroTik servers and must not run on every scheduler tick. The returned `latest-version` reflects the
last time the router ran `check-for-updates` (manual, scheduled on the router, or from a prior update). This is a known limitation documented in the
Non-Goals section.

When `parse_latest_version` returns `None` (field absent — common on a freshly bootstrapped router that has never run `check-for-updates`),
`fetch_releases` returns `Err(report!(RouterOsError::VersionUnavailable("run check-for-updates on the router first")))`. This surfaces the reason in
the UI rather than silently showing an empty release list.

`execute_update` calls `package_install()` or `package_download()` directly. It does NOT pre-call `check_for_updates()` — RouterOS's `install` command
is self-contained (fetches and installs atomically). A pre-check would fire asynchronously and return exit 0 before completing, causing `install` to
run against stale metadata.

`execute_update` constructs `RouterOsUpdateExecutor` with `reboot` from `RouterOsConfig` and `allow_reboot` from the downcast runtime. Final decision:
`config.reboot && host.allow_reboot`.

**`src/error.rs`**

```rust
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RouterOsError {
    #[error("SSH exec failed: {0}")]
    SshExec(String),
    #[error("failed to parse RouterOS output field '{field}' from: {context}")]
    ParseFailure { field: &'static str, context: String },
    #[error("version not available: {0}")]
    VersionUnavailable(String),
}
```

`impl_report_conversion!(RouterOsError => PluginError, |e| PluginError::PluginInternal(e.to_string()))` — all variants map to `PluginInternal`. These
are runtime SSH/parse failures, not unsupported-operation conditions.

## File Map

| File                                                                                  | Change                                                                                                                                |
| ------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/core/agent-ssh/src/ssh_executor.rs`                                           | Rename `SshCommandExecutor` → `PosixSshCommandExecutor`; extract base `SshCommandExecutor` with `exec_raw`, `sftp_put`, `sftp_remove` |
| `crates/core/agent-ssh/src/routeros_executor.rs`                                      | New: `RouterOsSshExecutor` with typed RouterOS CLI methods; `parse_routeros_field` helper                                             |
| `crates/core/agent-ssh/src/host_info.rs`                                              | Add `collect_remote_host_info_routeros()`; sets `OsFamily::RouterOs` + `ROUTER_OS_CLI` feature                                        |
| `crates/core/agent-ssh/src/operations/bootstrap.rs`                                   | Add `detect_host_os()` probe; route to `plan_bootstrap_routeros` or `plan_bootstrap_posix`                                            |
| `crates/core/agent-ssh/src/operations/bootstrap_routeros.rs`                          | New: `RouterOsBootstrapParams`, `RouterOsPlannedAction`, `plan_bootstrap_routeros`, `execute_bootstrap_routeros`                      |
| `crates/core/agent-ssh/src/db/migration/m20260507_000001_add_routeros_host_config.rs` | New: `CREATE TABLE routeros_host_config` with FK to `ssh_hosts`                                                                       |
| `crates/core/agent-ssh/src/db/entity/routeros_host_config.rs`                         | New: `Model { ssh_host_id, allow_reboot }` entity                                                                                     |
| `crates/core/agent-ssh/src/lib.rs`                                                    | Register new migration; expose new entity module                                                                                      |
| `crates/shared/types/src/os_family.rs`                                                | Remove "groundwork" comment from `OsFamily::RouterOs`                                                                                 |
| `crates/shared/types/src/host_feature.rs`                                             | Remove "groundwork" comment from `ROUTER_OS_CLI`                                                                                      |
| `crates/plugins/infrastructure/core/src/host_requirements.rs`                         | Add `feature_arrays::ROUTER_OS_CLI` static; add `HostRequirements::ROUTER_OS` constant                                                |
| `crates/plugins/infrastructure/core/src/host_runtime.rs`                              | Add `RouterOsExecutor` trait; add `RouterOsHostRuntime`; add `construct_routeros_host_runtime`                                        |
| `crates/plugins/infrastructure/core/src/lib.rs`                                       | Re-export `RouterOsExecutor`, `RouterOsHostRuntime`, `construct_routeros_host_runtime`                                                |
| `Cargo.toml` (workspace)                                                              | Add `russh-sftp` workspace dependency                                                                                                 |
| `crates/plugins/package-managers/routeros/Cargo.toml`                                 | New crate manifest                                                                                                                    |
| `crates/plugins/package-managers/routeros/src/lib.rs`                                 | New: crate root, `declare_plugin!`                                                                                                    |
| `crates/plugins/package-managers/routeros/src/plugin.rs`                              | New: `RouterOsPlugin` with `VersionDetector`, `UpdateExecutor`, `ReleaseFetcher` roles                                                |
| `crates/plugins/package-managers/routeros/src/config.rs`                              | New: `RouterOsConfig`                                                                                                                 |
| `crates/plugins/package-managers/routeros/src/version.rs`                             | New: `parse_resource_version`, `parse_latest_version`                                                                                 |
| `crates/plugins/package-managers/routeros/src/executor.rs`                            | New: `RouterOsUpdateExecutor`; install-vs-download routing                                                                            |
| `crates/plugins/package-managers/routeros/src/error.rs`                               | New: `RouterOsError`; `impl_report_conversion!`                                                                                       |
| `Cargo.toml` (workspace)                                                              | Add `uptrakit-package-manager-routeros` to workspace members                                                                          |

## Testing

**Version parsing** (`crates/plugins/package-managers/routeros/src/version.rs`):

```rust
#[test]
fn parse_resource_version_strips_channel_suffix() {
    let output = "version: 7.14.2 (stable)\nplatform: MikroTik\n";
    assert_eq!(parse_resource_version(output), Some("7.14.2".to_string()));
}

#[test]
fn parse_resource_version_no_suffix() {
    let output = "version: 7.15\nuptime: 1d\n";
    assert_eq!(parse_resource_version(output), Some("7.15".to_string()));
}

#[test]
fn parse_latest_version_returns_field() {
    let output = "channel: stable\ninstalled-version: 7.14.2\nlatest-version: 7.15\n";
    assert_eq!(parse_latest_version(output), Some("7.15".to_string()));
}

#[test]
fn parse_resource_version_missing_field_returns_none() {
    assert_eq!(parse_resource_version("uptime: 3d\n"), None);
}
```

**Machine ID parsing** (`crates/core/agent-ssh/src/host_info.rs`):

```rust
#[test]
fn machine_id_from_routerboard_serial() {
    let output = "routerboard: yes\nserial-number: ABC123\nmodel: RB4011\n";
    assert_eq!(extract_machine_id_routerboard(output), Some("ABC123".to_string()));
}

#[test]
fn machine_id_fallback_to_license_software_id() {
    let routerboard = "routerboard: yes\n"; // no serial-number
    let license = "software-id: XXXX-YYYY\nlevel: 6\n";
    assert_eq!(extract_machine_id_routerboard(routerboard), None);
    assert_eq!(extract_machine_id_license(license), Some("XXXX-YYYY".to_string()));
}
```

**Bootstrap plan generation** (`crates/core/agent-ssh/src/operations/bootstrap_routeros.rs`):

```rust
#[test]
fn plan_includes_reboot_policy_when_allowed() {
    let params = RouterOsBootstrapParams { allow_reboot: true, ..stub_params() };
    let plan = plan_bootstrap_routeros(&params);
    let create_group = plan.iter().find_map(|a| match a {
        RouterOsPlannedAction::CreateGroup { policies } => Some(policies),
        _ => None,
    });
    assert!(create_group.unwrap().contains(&"reboot".to_string()));
}

#[test]
fn plan_excludes_reboot_policy_when_not_allowed() {
    let params = RouterOsBootstrapParams { allow_reboot: false, ..stub_params() };
    let plan = plan_bootstrap_routeros(&params);
    let create_group = plan.iter().find_map(|a| match a {
        RouterOsPlannedAction::CreateGroup { policies } => Some(policies),
        _ => None,
    });
    assert!(!create_group.unwrap().contains(&"reboot".to_string()));
}

#[test]
fn plan_contains_key_upload_and_import_and_delete() {
    let plan = plan_bootstrap_routeros(&stub_params());
    let actions: Vec<_> = plan.iter().map(std::mem::discriminant).collect();
    // UploadPublicKey must precede ImportSshKey, which must precede DeletePublicKey
    let upload_pos = actions.iter().position(|&d| d == discriminant(&RouterOsPlannedAction::UploadPublicKey { remote_path: String::new() }));
    let import_pos = actions.iter().position(|&d| d == discriminant(&RouterOsPlannedAction::ImportSshKey { remote_path: String::new() }));
    let delete_pos = actions.iter().position(|&d| d == discriminant(&RouterOsPlannedAction::DeletePublicKey { remote_path: String::new() }));
    assert!(upload_pos.unwrap() < import_pos.unwrap() && import_pos.unwrap() < delete_pos.unwrap());
}
```

**Update execution routing** (`crates/plugins/package-managers/routeros/src/executor.rs`):

```rust
#[tokio::test]
async fn install_called_when_config_reboot_true_and_host_allows() { ... }

#[tokio::test]
async fn download_called_when_config_reboot_true_but_host_disallows() { ... }

#[tokio::test]
async fn download_called_when_config_reboot_false() { ... }
```

These tests use a mock `RouterOsSshExecutor` (constructed with a `MockSshSession` or equivalent test double) to assert which method is called.

## Documentation Deliverables

- **`CONTEXT.md`** — update `Host` definition to note RouterOS as a supported host type; add RouterOS to the list of managed host OS families.
- **`docs/development/plugin-guidelines.md`** — document `HostRequirements::ROUTER_OS` constant and the `RouterOsExecutor` trait as the pattern for
  non-POSIX plugin execution; note that `RouterOsHostRuntime` downcast replaces `executor()` for non-POSIX hosts.
- **`docs/adr/`** — new ADR: "Non-POSIX SSH bootstrap via probe-then-route detection" documenting the `detect_host_os()` two-gate probe, the
  `RouterOsHostRuntime` downcast pattern, and the trade-off of placing `RouterOsExecutor` in `plugin-infrastructure-core` (convenience vs. core crate
  scope creep). This is a hard-to-reverse, surprising-without-context architectural decision (the first non-POSIX host type).
- **`crates/plugins/infrastructure/core/src/host_runtime.rs`** — update `construct_host_runtime` doc comment to remove "Currently always returns
  `StandardHostRuntime`" and reference the new RouterOS dispatch.

## Non-Goals

- Docker container discovery on RouterOS (CHR containers are not a supported target).
- RouterOS infrastructure plugin (PVE-style node management, CHR provisioning) — future work.
- CHR-specific testing in CI (requires a CHR license; integration tests target physical/VM RouterOS only via `--ignored`).
- RouterOS API (REST/JSON) transport — SSH is the only supported transport; the REST API is not used.
- Updating individual packages by name — only full RouterOS system upgrades (`/system package update`) are supported.
- Windows `OsFamily` support — unrelated and deferred.
