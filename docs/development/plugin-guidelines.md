# Plugin Development Guidelines

Plugins are first-party extension modules that detect, report, and update software on managed hosts.
Each plugin crate declares a `DESCRIPTOR` static via the `declare_plugin!` macro, implements `PluginMeta` plus
role-specific traits, and is registered in `uptrakit-plugin-infrastructure-registry`. This document describes
the full lifecycle and conventions for building and extending plugins.

When adding or changing a plugin, document the full lifecycle:

- How the agent detects the installed version.
- How the controller resolves the latest upstream version.
- Version comparison rules (semver, tag prefixes, build metadata handling).
- Update execution steps, required privileges, and failure modes.
- Required configuration fields with examples.
- Any assumptions about the agent environment or custom scripts.

Plugins should keep parsing and comparison logic in pure functions so they are easy to test.

The plugin registry crate (`uptrakit-plugin-infrastructure-registry`) centralizes config validation, mask/restore
workflows, and creates plugin instances based on `PluginTypeId`. Document plugin behavior so the registry
can continue to validate configs and mask secrets correctly.

`PluginTypeId` is a newtype wrapper around a string identifier. Plugin crates declare their ID with
`PluginTypeId::from_static("my_plugin_id")`. The string representations are: `releases_github`,
`releases_gitlab`, `releases_forgejo`, `releases_docker`, `discovery_proxmox_helper_scripts`,
`package_manager_homebrew`, `package_manager_apt`, `package_manager_npm`, `package_manager_mas`,
`generic_shell`, `infrastructure_proxmox`.

## Plugin Families

The `PluginFamily` enum classifies plugins into functional groups:

| Family | Description |
| :--- | :--- |
| `Software` | Plugins that detect, fetch, and update software packages. |
| `Hook` | Plugins that run pre/post-update lifecycle hooks. |
| `Notification` | Plugins that deliver alerts via external channels (webhook, email, Telegram). |
| `Infrastructure` | Plugins that manage infrastructure resources (Proxmox VE). |
| `Enhancement` | Plugins that enrich software items with supplemental data (dashboard icons). |

The family is declared in `declare_plugin!` and determines which role traits are expected.

## Plugin Capabilities

The `PluginCapability` enum defines optional features a plugin may support. Capabilities are **auto-derived**
from the roles declared in `declare_plugin!`. Plugins do not manually list capabilities -- the macro infers
them from implemented role traits.

| Capability | Source | Description |
| :--- | :--- | :--- |
| `DiscoverLocalSoftware` | `DiscoveryPlugin` role | Enumerate software the plugin can manage on the local system. |
| `RefreshPackageIndex` | `PackageIndexPlugin` role | Refresh local package index (for example, `apt update`). |
| `DetectHostCompatibility` | `HostCompatibilityPlugin` role | Determine whether this plugin is applicable to the current host environment. |
| `UpdateLifecycle` | `UpdateLifecyclePlugin` role | Plugin implements pre/post-update hooks. See [Update Lifecycle Plugins](update-hooks.md). |
| `ControllerSideFetchReleases` | `HostRequirements::CONTROLLER_ONLY` | `fetch_releases()` runs on the controller without local system state. |
| `ConfigTest` | `config_test: [...]` in `declare_plugin!` | Plugin supports configuration testing. See [Config Test Capability](#config-test-capability). |

## Host Requirements

Each plugin role declares its host requirements via the `HostRequirements` enum:

| Variant | Meaning | Example |
| :--- | :--- | :--- |
| `POSIX` | Requires a POSIX command executor (default). | APT version detection, Homebrew discovery. |
| `POSIX_PRIVILEGED` | Requires POSIX with elevated privileges (sudo). | APT package installation, systemctl operations. |
| `CONTROLLER_ONLY` | Runs on the controller; no agent needed. | GitHub Releases fetch, Docker registry queries. |

A single plugin can have different requirements per role. For example, APT uses `POSIX` for detection
and `POSIX_PRIVILEGED` for updates.

## Host Compatibility Detection

Plugins that implement the `HostCompatibilityPlugin` role trait provide `detect_host_compatibility()` to
determine whether the plugin is applicable to the host where the agent is running. The method returns a
`HostCompatibility` enum:

```rust
pub enum HostCompatibility {
    /// The plugin is compatible with this host.
    Compatible,
    /// The plugin is not compatible with this host (e.g. required tool not found).
    Incompatible(String),
}
```

This allows the agent to skip discovery for plugins that are not applicable to the current host
(e.g. no Docker daemon, no APT), and ensures that helper scripts are only installed on compatible
hosts during bootstrap (preventing failures on read-only filesystems such as Flatcar Linux).

### Pattern examples

**APT plugin** -- checks whether `apt-get` is available:

```rust
async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
    let result = self
        .executor
        .execute_quiet(&CommandSpec::exec("which", ["apt-get".to_string()]))
        .await
        .map_err(|e| report!(PluginError::PluginInternal(format!("which apt-get failed: {e}"))))?;
    if result.exit_code == 0 {
        Ok(HostCompatibility::Compatible)
    } else {
        Ok(HostCompatibility::Incompatible("apt-get not found".to_string()))
    }
}
```

**Docker plugin** -- pings the Docker daemon directly (daemon build only):

```rust
#[cfg(feature = "daemon")]
async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
    match self.docker_client.ping().await {
        Ok(()) => Ok(HostCompatibility::Compatible),
        Err(e) => Ok(HostCompatibility::Incompatible(format!(
            "Docker daemon not accessible: {e}"
        ))),
    }
}
```

## Update Lifecycle Plugins

Plugins that implement the `UpdateLifecyclePlugin` role trait receive an `UpdateLifecycleContext`:

```rust
pub struct UpdateLifecycleContext {
    pub package_identifier: String,
    pub to_version: String,
    pub from_version: Option<String>,
    pub release_info: Option<ReleaseInfo>,
    /// `None` during pre-hooks, `Some(true/false)` during post-hooks.
    pub update_succeeded: Option<bool>,
}
```

### Pre-update hook

`execute_pre_hook()` is called before the update. It returns `PreUpdateHookResult`:

- `PreUpdateHookResult::proceed()` -- continue with the update.
- `PreUpdateHookResult::abort(reason)` -- cancel the update with a reason message.

If a pre-update hook returns abort, no further pre-hooks or the update itself are executed.

### Post-update hook

`execute_post_hook()` is called after the update completes. It is non-fatal: any error is
logged as a warning but does not mark the update as failed. The `update_succeeded` field
indicates whether the update succeeded.

For full details on the built-in hook plugins (`hook_systemd`, `hook_shell`), see
[Update Lifecycle Plugins](update-hooks.md).

## Declaring `ControllerSideFetchReleases`

Plugins with `HostRequirements::CONTROLLER_ONLY` automatically receive the
`ControllerSideFetchReleases` capability. This signals that `fetch_releases()` makes only HTTP/API
calls and does not depend on any local system state.

**When to use `CONTROLLER_ONLY`:**

- Your `fetch_releases()` only performs HTTP requests to an external API (GitHub REST API, OCI
  registry, etc.).
- It does not call `self.executor.execute()` or `self.executor.execute_quiet()`.
- It does not read from the local filesystem or depend on a locally synced package index.

**When NOT to use it:**

- Your plugin needs a local package index (e.g. Homebrew's `brew info --json`, APT's
  `apt-cache policy`). These must run agent-side.
- Your `fetch_releases()` shells out to a local CLI tool.

**Effect:** When `execution_site` is `auto` (the default), the controller runs `fetch_releases()`
once per unique `(plugin_config_id, package_identifier)` combination and propagates the result
to all hosts sharing that combination. This avoids redundant API calls when many hosts track the
same upstream release. The controller uses a `NoopCommandExecutor` -- if your plugin accidentally
calls it, the process will panic.

**Current plugins with this capability:** `GitHubPlugin`, `GitLabPlugin`, `ForgejoPlugin`,
`DockerPlugin`, `NpmPlugin`.

## Config Test Capability

Plugins declare config test support in `declare_plugin!` using the `config_test:` field. The first
kind listed is the default:

```rust
declare_plugin! {
    // ... other fields ...
    config_test: [VersionDetection, UpdateCommandValidation],
}
```

This generates a `ConfigTestOps` implementation and adds `PluginCapability::ConfigTest` automatically.
All built-in plugins declare this capability.

The test executes without creating any database records or triggering real updates. The kind of test
performed depends on the plugin type and its capabilities:

| `ConfigTestKind` | Description | When used |
| :--- | :--- | :--- |
| `VersionDetection` | Runs `detect_installed_version()` against the host and returns the detected version. | Agent-side plugins that support version detection (Shell, APT, Homebrew, etc.). |
| `UpdateCommandValidation` | Validates the update command syntax (e.g. `sh -n` check) without executing it. | Agent-side plugins with an update command (Shell). |
| `PreUpdateHook` | Executes the pre-update hook with a mock `UpdateLifecycleContext`. | Hook plugins (hook\_systemd, hook\_shell) assigned to `pre_update_hook`. |
| `PostUpdateHook` | Executes the post-update hook with a mock `UpdateLifecycleContext`. | Hook plugins (hook\_systemd, hook\_shell) assigned to `post_update_hook`. |
| `Connectivity` | Tests upstream API connectivity by performing a lightweight `fetch_releases()` call. | Controller-side plugins (GitHub, GitLab, Forgejo, Docker, npm, Cargo). |

### Two execution paths

The test endpoint dispatches to one of two paths based on the plugin's host requirements:

**Controller-side test path** -- For plugins with `HostRequirements::CONTROLLER_ONLY`, the
controller runs the test directly in its own process. No agent connection is required and `host_id`
is optional. The controller instantiates the plugin with a `NoopCommandExecutor`, calls
`fetch_releases()` (or equivalent), and returns the result. This path handles the `Connectivity`
test kind.

**Agent-side test path** -- For plugins that require local system access (Shell, package managers,
hooks), the controller sends a `ControllerMessage::TestPluginConfig` wire message to the agent
that owns the specified `host_id`. The agent executes the test locally and responds with
`ServiceMessage::TestPluginConfigResult`. The controller correlates the response via
`ConfigTestProxy` (same request/response pattern as `ExtensionProxy`) and returns it to the
HTTP caller. The `host_id` field is required for this path.

## The `declare_plugin!` Macro

The `declare_plugin!` macro is the single entry point for plugin declaration. It generates:

- A `DESCRIPTOR` static containing all plugin metadata.
- A `PluginMeta` trait implementation.
- Config delegation functions (validation, masking, form schema).
- `ConfigTestOps` if `config_test:` is specified.
- Capability list derived from declared roles.

### Syntax reference

```rust
declare_plugin! {
    id: PluginTypeId::from_static("my_plugin_id"),
    name: "My Plugin",
    family: PluginFamily::Software,
    config_model: ConfigModel::PluginConfig,
    config: MyConfig,
    plugin: MyPlugin,
    host_requirements: HostRequirements::POSIX,
    config_test: [VersionDetection],
}
```

| Field | Required | Description |
| :--- | :--- | :--- |
| `id` | Yes | `PluginTypeId` identifying this plugin. |
| `name` | Yes | Human-readable display name. |
| `family` | Yes | `PluginFamily` variant. |
| `config_model` | Yes | `ConfigModel::PluginConfig` or `ConfigModel::NotificationChannel`. |
| `config` | Yes | Config struct type (must implement `PluginConfig`). |
| `plugin` | Yes | Plugin struct type (must implement `PluginMeta` + role traits). |
| `host_requirements` | Yes | Default `HostRequirements` for the plugin. |
| `config_test` | No | List of `ConfigTestKind` variants (first is default). |

## The `PluginConfig` Trait

The `PluginConfig` trait unifies configuration validation, secret masking, form schema, and identifier
validation into a single trait. It replaces the former `ConfigFormSchema` and `SecretMasking` traits.

```rust
pub trait PluginConfig: Serialize + DeserializeOwned + Clone + Send + Sync {
    /// Validate the configuration. Called during plugin creation.
    fn validate(&self) -> Result<(), String>;

    /// Return a copy with secret fields replaced by "***".
    fn with_secrets_masked(self) -> Self { self }

    /// Restore secret fields from an existing config where `self` contains "***" sentinels.
    fn restore_secrets_from(&mut self, _existing: &Self) {}

    /// Return typed form field definitions for the frontend.
    fn form_schema() -> Vec<FieldDef>;

    /// Validate a package identifier for this plugin type.
    fn validate_identifier(_value: &str) -> Result<(), String> { Ok(()) }
}
```

### Secret masking

Plugins with no secrets (Homebrew, Proxmox Helper Scripts) use the default no-op implementations
of `with_secrets_masked()` and `restore_secrets_from()`.

Plugins with secrets (GitHub, Docker) override both methods with field-level masking logic:

```rust
impl PluginConfig for GitHubConfig {
    fn with_secrets_masked(mut self) -> Self {
        if self.auth_token.is_some() {
            self.auth_token = Some("***".to_string());
        }
        self
    }

    fn restore_secrets_from(&mut self, existing: &Self) {
        if self.auth_token.as_deref() == Some("***") {
            self.auth_token = existing.auth_token.clone();
        }
    }

    // ... other methods
}
```

The registry uses generic helpers `mask_secrets_for::<T>()` and `restore_secrets_for::<T>()` that
deserialize the JSON config, apply the trait methods, and re-serialize. This eliminates duplicated
deserialize-method-serialize boilerplate per plugin.

### Config form schema

The `form_schema()` method on `PluginConfig` allows plugins to declare typed form field definitions
for their configuration. The frontend renders these as structured forms instead of raw JSON textareas.

```rust
impl PluginConfig for GitHubConfig {
    fn form_schema() -> Vec<FieldDef> {
        vec![
            FieldDef::new("auth_token", "Auth Token")
                .with_type(FieldType::Password)
                .sensitive()
                .with_help_text("Personal access token for private repos"),
            FieldDef::new("include_prereleases", "Include Pre-releases")
                .with_type(FieldType::Toggle)
                .with_help_text("Include draft/pre-release versions"),
        ]
    }

    // ... other methods
}
```

Plugins with no configurable fields (MAS, Proxmox Helper Scripts) return an empty `Vec`.

For nested configuration objects (e.g., Docker's `auth` enum), use dot-separated keys with a
`_` prefix for tagged enum discriminators:

- `auth._type` -- select field for the enum variant (maps to JSON `auth.type`)
- `auth.username` -- text field visible when `auth._type` is `"basic"`
- `auth.password` -- password field visible when `auth._type` is `"basic"`

Use `FieldDef::with_visible_when()` for conditional visibility based on another field's value.

The schema is served to the frontend via `GET /api/v1/plugin-types` in the `config_form_fields`
array of `PluginTypeInfo`.

## The `TypeSettings` Trait

Plugins that support tenant-level settings implement the separate `TypeSettings` trait:

```rust
pub trait TypeSettings {
    fn type_settings_form_schema() -> Vec<FieldDef>;
    fn type_settings_sample() -> serde_json::Value;
}
```

### Type settings vs plugin configs

Plugin configuration uses a **two-tier model**:

- **Type settings** (`plugin_type_settings` table) -- tenant-level defaults per plugin type.
  These store discovery preferences and behavioral defaults that apply to all instances of a
  plugin type within a tenant. Examples: APT `discovery_filter` (`manual` vs `all`), Homebrew
  `package_type` (`formula` vs `cask`), Pacman `discovery_filter` (`all` vs `explicit`).
- **Plugin configs** (`plugin_configs` table) -- named configuration profiles. These store
  credentials, API endpoints, and per-profile settings that vary between configurations.
  Examples: GitHub `auth_token`, Docker `auth` credentials, Forgejo `api_base_url`.

**When to use which:**

| Use type settings for | Use plugin configs for |
| --- | --- |
| Discovery preferences (`discovery_filter`) | Authentication credentials (`auth_token`) |
| Behavioral defaults (`package_type`) | API endpoints (`api_base_url`) |
| Settings shared across all configs of a type | Settings that differ between profiles |

**Implementing type settings:**

```rust
impl TypeSettings for AptConfig {
    fn type_settings_form_schema() -> Vec<FieldDef> {
        vec![
            FieldDef::new("discovery_filter", "Discovery Filter")
                .with_type(FieldType::Select)
                .with_options(vec![
                    ("manual", "Manual packages only"),
                    ("all", "All installed packages"),
                ]),
        ]
    }

    fn type_settings_sample() -> serde_json::Value {
        serde_json::json!({ "discovery_filter": "manual" })
    }
}
```

Plugins with no type-level settings do not implement `TypeSettings`.

The `PluginTypeInfo` response (from `GET /api/v1/plugin-types`) includes
`type_settings_form_fields` and `type_settings_sample` so the frontend can render a type
settings form.

### Three-layer config merge

When the system needs the effective configuration for a plugin operation,
`resolve_effective_config()` merges three layers (broadest to narrowest):

1. **Type settings** -- tenant-level defaults from `plugin_type_settings`.
2. **Profile config** -- from the `plugin_configs` row.
3. **Assignment config** -- per-host override from `host_software_item_plugins.config`.

Each layer's JSON is shallow-merged on top of the previous one. Fields present in a narrower
layer override the same field from a broader layer. This replaces the previous two-layer
model (plugin config + `config_override`).

## Plugin Construction

Plugin construction is **synchronous**. Each plugin's `new()` takes its typed config and an
`Arc<dyn HostRuntime>`:

```rust
fn new(config: MyConfig, runtime: Arc<dyn HostRuntime>) -> Result<Self>
```

### POSIX plugins

POSIX plugins extract the `CommandExecutor` from the runtime at construction time:

```rust
impl AptPlugin {
    pub fn new(config: AptConfig, runtime: Arc<dyn HostRuntime>) -> Result<Self> {
        let executor = require_posix_executor(&runtime)?;
        Ok(Self { config, executor })
    }
}
```

The `require_posix_executor(&runtime)` helper returns `Arc<dyn CommandExecutor>` or fails with
a clear error if the runtime does not provide one.

### Controller-side plugins

Controller-side plugins downcast to `ControllerRuntime` to access shared services:

```rust
impl GitHubPlugin {
    pub fn new(config: GitHubConfig, runtime: Arc<dyn HostRuntime>) -> Result<Self> {
        let controller = runtime
            .as_any()
            .downcast_ref::<ControllerRuntime>()
            .ok_or_else(|| report!(PluginError::Configuration(
                "GitHub plugin requires ControllerRuntime".to_string()
            )))?;
        let http_client = controller.http_client().clone();
        Ok(Self { config, http_client })
    }
}
```

All plugin `new()` constructors must return `Result<Self, Report<PluginError>>` so the registry can
handle instantiation failures uniformly. The constructor should validate its configuration before
returning.

## The Role Model for New Plugins

When implementing a new plugin, consider which roles it will serve. Plugins implement `PluginMeta`
plus one or more role traits:

### Single-plugin-for-all-roles (common case)

Most plugins implement all three roles (`VersionDetectorPlugin`, `ReleaseFetcherPlugin`,
`UpdateExecutorPlugin`) in a single plugin crate. When autodiscovery or manual assignment creates
host assignments, all three role rows point to the same `plugin_config_id`. This is the default
and requires no special handling.

### Partial-role plugins

Some plugins only make sense for a subset of roles:

- **Discovery-only plugins** (e.g. `ProxmoxHelperScriptsPlugin`) -- implement `DiscoveryPlugin`
  but do not participate in any of the three version/update roles. The controller synthesizes
  downstream plugin configs that fill the role assignments.
- **Fetch-only plugins** -- a hypothetical plugin that only knows how to fetch upstream releases
  (e.g. a custom changelog scraper). Users would pair it with another plugin for detection and
  updates.
- **Enhancement plugins** -- implement `SoftwareItemLifecycle` to enrich software items with
  supplemental data (e.g. dashboard icons) without participating in version/update roles.

When your plugin does not implement a particular role's method, the default trait implementation
returns an error. The system will not call a role method on a plugin that is not assigned to
that role, so this is safe.

### Testing role assignments

When writing integration tests for a new plugin, verify that:

1. The plugin's declared roles in `declare_plugin!` accurately reflect its capabilities.
2. All implemented role methods work correctly, and unimplemented roles return clear errors.

## `declare_plugin!` Examples

### POSIX software plugin (APT)

APT has mixed privilege requirements: detection is unprivileged, updates require sudo.

```rust
use uptrakit_plugin_infrastructure_core::prelude::*;

declare_plugin! {
    id: PluginTypeId::from_static("package_manager_apt"),
    name: "APT",
    family: PluginFamily::Software,
    config_model: ConfigModel::PluginConfig,
    config: AptConfig,
    plugin: AptPlugin,
    host_requirements: HostRequirements::POSIX,
    config_test: [VersionDetection],
}

pub struct AptPlugin {
    config: AptConfig,
    executor: Arc<dyn CommandExecutor>,
}

impl AptPlugin {
    pub fn new(config: AptConfig, runtime: Arc<dyn HostRuntime>) -> Result<Self> {
        let executor = require_posix_executor(&runtime)?;
        Ok(Self { config, executor })
    }
}

// Role traits: VersionDetectorPlugin, ReleaseFetcherPlugin,
// UpdateExecutorPlugin (POSIX_PRIVILEGED), DiscoveryPlugin,
// PackageIndexPlugin, HostCompatibilityPlugin
```

### Controller-side software plugin (GitHub Releases)

GitHub Releases runs entirely on the controller. It gets a shared HTTP client from `ControllerRuntime`.

```rust
use uptrakit_plugin_infrastructure_core::prelude::*;

declare_plugin! {
    id: PluginTypeId::from_static("releases_github"),
    name: "GitHub Releases",
    family: PluginFamily::Software,
    config_model: ConfigModel::PluginConfig,
    config: GitHubConfig,
    plugin: GitHubPlugin,
    host_requirements: HostRequirements::CONTROLLER_ONLY,
    config_test: [Connectivity],
}

pub struct GitHubPlugin {
    config: GitHubConfig,
    http_client: reqwest::Client,
}

impl GitHubPlugin {
    pub fn new(config: GitHubConfig, runtime: Arc<dyn HostRuntime>) -> Result<Self> {
        let controller = runtime
            .as_any()
            .downcast_ref::<ControllerRuntime>()
            .ok_or_else(|| report!(PluginError::Configuration(
                "GitHub plugin requires ControllerRuntime".to_string()
            )))?;
        let http_client = controller.http_client().clone();
        Ok(Self { config, http_client })
    }
}

// Role traits: ReleaseFetcherPlugin (controller-side),
// VersionDetectorPlugin (optional, via detect_installed_version_command),
// UpdateExecutorPlugin (optional, via install_command)
```

### Hook plugin (Shell Hook)

Hook plugins implement the `UpdateLifecyclePlugin` role.

```rust
use uptrakit_plugin_infrastructure_core::prelude::*;

declare_plugin! {
    id: PluginTypeId::from_static("hook_shell"),
    name: "Shell Hook",
    family: PluginFamily::Hook,
    config_model: ConfigModel::PluginConfig,
    config: ShellHookConfig,
    plugin: ShellHookPlugin,
    host_requirements: HostRequirements::POSIX,
    config_test: [PreUpdateHook, PostUpdateHook],
}

pub struct ShellHookPlugin {
    config: ShellHookConfig,
    executor: Arc<dyn CommandExecutor>,
}

impl ShellHookPlugin {
    pub fn new(config: ShellHookConfig, runtime: Arc<dyn HostRuntime>) -> Result<Self> {
        let executor = require_posix_executor(&runtime)?;
        Ok(Self { config, executor })
    }
}

// Role trait: UpdateLifecyclePlugin
```

### Notification plugin (Webhook)

Notification plugins use `PluginFamily::Notification`, `ConfigModel::NotificationChannel`, and provide
a `create_notification_transport()` function.

```rust
use uptrakit_plugin_infrastructure_core::prelude::*;

declare_plugin! {
    id: PluginTypeId::from_static("notification_webhook"),
    name: "Webhook",
    family: PluginFamily::Notification,
    config_model: ConfigModel::NotificationChannel,
    config: WebhookConfig,
    plugin: WebhookPlugin,
    host_requirements: HostRequirements::CONTROLLER_ONLY,
    config_test: [Connectivity],
}

impl WebhookPlugin {
    pub fn create_notification_transport(
        config: &CatalogConfig,
    ) -> PluginResult<Arc<dyn NotificationTransport>> {
        let typed: WebhookConfig = serde_json::from_value(config.settings.clone())
            .map_err(|e| report!(PluginError::Configuration(e.to_string())))?;
        Ok(Arc::new(WebhookTransport::new(typed)?))
    }
}
```

### Enhancement plugin (Dashboard Icons)

Enhancement plugins implement the `SoftwareItemLifecycle` role.

```rust
use uptrakit_plugin_infrastructure_core::prelude::*;

declare_plugin! {
    id: PluginTypeId::from_static("enhancement_dashboard_icons"),
    name: "Dashboard Icons",
    family: PluginFamily::Enhancement,
    config_model: ConfigModel::PluginConfig,
    config: DashboardIconsConfig,
    plugin: DashboardIconsPlugin,
    host_requirements: HostRequirements::CONTROLLER_ONLY,
}

pub struct DashboardIconsPlugin {
    config: DashboardIconsConfig,
    http_client: reqwest::Client,
}

impl DashboardIconsPlugin {
    pub fn new(config: DashboardIconsConfig, runtime: Arc<dyn HostRuntime>) -> Result<Self> {
        let controller = runtime
            .as_any()
            .downcast_ref::<ControllerRuntime>()
            .ok_or_else(|| report!(PluginError::Configuration(
                "Dashboard Icons plugin requires ControllerRuntime".to_string()
            )))?;
        let http_client = controller.http_client().clone();
        Ok(Self { config, http_client })
    }
}

// Role trait: SoftwareItemLifecycle
```

## Dependencies and Re-exports

Plugin crates should avoid unnecessary direct dependencies. The `uptrakit-plugin-infrastructure-core` crate
re-exports commonly needed types:

- **`uptrakit_plugin_infrastructure_core::prelude`** -- re-exports of `PluginTypeId`, `PluginFamily`,
  `PluginMeta`, `PluginConfig`, `HostRequirements`, `ConfigModel`, `HostRuntime`, `ControllerRuntime`,
  `require_posix_executor`, `PluginError`, `PluginCapability`, `report!`, `bail!`, and `Arc`.
- **`uptrakit_plugin_infrastructure_core::mpsc`** -- re-export of `tokio::sync::mpsc`. Use this instead of
  depending on tokio directly. Tokio should only be in `[dev-dependencies]` (for `#[tokio::test]`).
- **`uptrakit_plugin_infrastructure_core::CommandExecutor`**, **`CommandSpec`**, etc. -- re-exports from
  `uptrakit-command`.
- **`uptrakit_plugin_infrastructure_core::SecretString`** -- re-export from `uptrakit-shared-types`.

See [Dependency Policy](dependency-policy.md) for the full re-export strategy.

## HTTP Client Requirements

Controller-side plugins obtain a shared `reqwest::Client` from `ControllerRuntime` instead of building
their own. This client is pre-configured with all security and reliability requirements:

```rust
let controller = runtime
    .as_any()
    .downcast_ref::<ControllerRuntime>()
    .ok_or_else(|| report!(PluginError::Configuration(
        "Plugin requires ControllerRuntime".to_string()
    )))?;
let http_client = controller.http_client().clone();
```

**What the shared client enforces:**

- **SSRF protection** -- `SsrfSafeResolver` blocks requests to private IP ranges and link-local
  addresses by default.
- **TLS hardening** -- WebPKI roots via `webpki_client_config()`, no system-trust drift.
- **Connect timeout** -- always 10 s; non-configurable.
- **Request timeout** -- 60 s default.

**Auth headers are applied per-request**, not as default headers on the client. This prevents
credential leakage across redirects:

```rust
let mut request = self.http_client.get(&url);
if let Some(token) = &self.config.auth_token {
    request = request.header("Authorization", format!("token {token}"));
}
let response = request.send().await?;
```

Do **not** call `reqwest::Client::builder()` directly in plugin code. Using the shared client from
`ControllerRuntime` ensures that all security settings are applied consistently and that future
hardening improvements propagate automatically.

## Command Executor Pattern

Plugins do not spawn processes directly. Instead, each POSIX plugin receives an `Arc<dyn CommandExecutor>`
via `require_posix_executor(&runtime)` at construction time and delegates all command execution through
that trait. This decouples plugin logic from the execution transport, enabling the same plugin code to
run commands locally (via `LocalCommandExecutor`) or remotely (via `SshCommandExecutor`).

See [Command Executor](command-executor.md) for the full trait reference, `CommandSpec` constructors,
and guidance on implementing custom executors.

### `execute_and_capture` helper

For the common pattern of running a command and capturing its stdout as a `String`, use the shared
helper from `uptrakit-plugin-infrastructure-core`:

```rust
use uptrakit_plugin_infrastructure_core::execute_and_capture;

let output = execute_and_capture(
    self.executor.as_ref(),
    CommandSpec::exec("dpkg-query", [...]),
    "dpkg-query list",
).await?;
```

This helper calls `execute_quiet`, maps any spawn/IO error to
`PluginError::PluginInternal(context)`, and propagates a non-zero exit code as
`PluginError::CommandFailed(code)`. Use it instead of inline `.execute_quiet()` + `.map_err()`
boilerplate.

**Do not use it** for commands where a non-zero exit code has a meaningful non-error
interpretation (e.g. `rpm -q` exit 1 = package not installed, `dnf check-update` exit 100 =
updates available). In those cases, call `execute_quiet` directly and inspect `exit_code`.

### Declaring privileged commands with `required_sudo_commands()`

If your plugin needs passwordless `sudo` to run certain commands, implement
`required_sudo_commands()` on the plugin struct:

```rust
use uptrakit_plugin_infrastructure_core::SudoCommandEntry;

fn required_sudo_commands(&self) -> Vec<SudoCommandEntry> {
    vec![SudoCommandEntry {
        command: "apt-get".into(),
        explanation: "Package installation and index refresh require root privileges".into(),
        helper_script: None,
        args_suffix: None,
        needs_setenv: false,
    }]
}
```

**Contract:**

- `command` is the **bare command name** (e.g. `"apt-get"`, `"systemctl"`), not an absolute path.
  The bootstrap and `sync` commands resolve the absolute path on the target host via
  `command -v <name>`.
- `args_suffix` optionally restricts the sudoers entry to specific subcommands (e.g.
  `Some("stop *")` -> `/usr/bin/systemctl stop *`). The suffix is written as normal
  command tokens; the SSH agent escapes sudoers-special characters when rendering the
  file while preserving wildcard tokens such as `*` anywhere in the argument list. Use
  this instead of a helper script when positional argument matching is sufficient.
- `explanation` is shown as a comment in the generated sudoers file and in CLI output. Keep it
  concise and factual.
- Return an empty `vec![]` (the default) when your plugin never needs elevated privileges.
- Do **not** hardcode `sudo` in your `CommandSpec` programs or arguments. Instead, call
  `.privileged()` on the spec and declare the corresponding command here so the sudoers file
  can be kept minimal.

The `PluginRegistry::all_required_sudo_commands()` static method aggregates declarations from all
registered plugins (using the same minimal-config instantiation as `create_plugin_for_discovery()`).
The bootstrap and `sync` SSH agent commands call this to build a per-command sudoers entry.

**Testing:**

Add a unit test verifying that your plugin returns the expected entries:

```rust
#[test]
fn required_sudo_commands_returns_expected_entries() {
    let runtime = Arc::new(TestRuntime::with_posix_executor());
    let plugin = MyPlugin::new(MyConfig::default(), runtime).unwrap();
    let cmds = plugin.required_sudo_commands();
    assert_eq!(cmds.len(), 1);
    assert_eq!(cmds[0].command, "my-tool");
    assert!(!cmds[0].explanation.is_empty());
}
```

See [Sudoers Management](../security/sudoers-management.md) for the full security rationale and
operator guidance.

## Plugin Architecture

Each software item on a host is managed through **role-based plugin assignments**. There are three
plugin roles, and each `(host, software_item)` pair can have up to one plugin assignment per role:

| Role | Default execution site | Responsibility |
| :--- | :--- | :--- |
| `detect_version` | Agent | Detect the currently installed version on the host. |
| `fetch_releases` | Agent or Controller (depends on `execution_site` and plugin capabilities) | Fetch latest version metadata from an upstream source. |
| `execute_update` | Agent | Run the update (via sudo-allowlisted commands or custom script). |

Different plugins can be assigned to different roles for the same software item on the same host.
For example, a PHS-discovered container might use APT for `detect_version` and `execute_update`
but a GitHub Releases config for `fetch_releases`.

See [Plugin System Architecture: Role-Based Plugin Assignments](plugin-system.md#role-based-plugin-assignments)
for the full data model and execution site decision logic.

Plugin crates:

| Crate | Path | Purpose |
| :--- | :--- | :--- |
| `uptrakit-shared-types` | `crates/shared/types/` | Canonical home for `PluginTypeId`, `ReleaseAsset`, and `ReleaseInfo` (plus `SecretString`, hex helpers). |
| `uptrakit-command` | `crates/shared/command/` | Shell execution, `CommandExecutor` trait, `CommandSpec`, `LocalCommandExecutor`. |
| `uptrakit-plugin-infrastructure-core` | `crates/plugins/infrastructure/core/` | Plugin trait/abstractions; `PluginConfig` trait; re-exports shared types and executor types. |
| `uptrakit-plugin-infrastructure-registry` | `crates/plugins/infrastructure/registry/` | Centralized plugin dispatch and validation; re-exports `PluginTypeId`. |
| `uptrakit-plugin-releases-docker` | `crates/plugins/releases/docker/` | Docker/OCI image tracking and container discovery. Implements `HostCompatibilityPlugin`. |
| `uptrakit-plugin-releases-github` | `crates/plugins/releases/github/` | GitHub Releases: controller-side fetch; agent-side install. |
| `uptrakit-plugin-releases-gitlab` | `crates/plugins/releases/gitlab/` | GitLab Releases: controller-side fetch; supports nested namespaces; PRIVATE-TOKEN auth. |
| `uptrakit-plugin-releases-forgejo` | `crates/plugins/releases/forgejo/` | Forgejo / Codeberg Releases: controller-side fetch; requires `api_base_url`. |
| `uptrakit-plugin-package-manager-homebrew` | `crates/plugins/package-managers/homebrew/` | Homebrew: agent-side version tracking and updates. Implements `HostCompatibilityPlugin`. |
| `uptrakit-plugin-discovery-proxmox-helper-scripts` | `crates/plugins/discovery/proxmox-helper-scripts/` | Proxmox VE: auto-discovers and manages helper scripts. Implements `HostCompatibilityPlugin`. |
| `uptrakit-plugin-package-manager-apt` | `crates/plugins/package-managers/apt/` | APT: Debian/Ubuntu package management. Implements `HostCompatibilityPlugin`. |
| `uptrakit-plugin-package-manager-npm` | `crates/plugins/package-managers/npm/` | npm: global-package tracking via `registry.npmjs.org`. Implements `HostCompatibilityPlugin`. |
| `uptrakit-plugin-generic-shell` | `crates/plugins/generic/shell/` | Generic shell plugin: custom `version_command` and `update_command`; agent-side only. |
| `uptrakit-plugin-hook-systemd` | `crates/plugins/hooks/systemd/` | Systemd hook: stops/starts a systemd service around updates. Implements `UpdateLifecyclePlugin`. |
| `uptrakit-plugin-hook-shell` | `crates/plugins/hooks/shell/` | Shell hook: runs arbitrary shell commands before/after updates. Implements `UpdateLifecyclePlugin`. |

## Plugin Source Layout

For package-manager and similar multi-trait plugins, split implementation across focused submodules
rather than putting everything in a single `plugin.rs` god file. The canonical structure is:

```text
crates/plugins/package-managers/mypkg/src/
  lib.rs         -- crate root: module declarations + pub re-exports
  config.rs      -- Config struct + PluginConfig impl + validate_identifier/validate_version
  error.rs       -- plugin-specific error type (if needed)
  plugin.rs      -- MyPlugin struct + PluginMeta impls + constants (~200 lines)
  discovery.rs   -- DiscoveryPlugin impl + discovery helpers
  detection.rs   -- VersionDetectorPlugin impl + parsing helpers
  releases.rs    -- ReleaseFetcherPlugin impl (or ControllerSideFetchReleases)
  update.rs      -- UpdateExecutorPlugin + PackageIndexPlugin impls
```

Tests live at the bottom of the file containing the code under test (using `#[cfg(test)]`).
The `#[cfg(test)] use super::*;` pattern gives test blocks access to private helpers in the
same module.

## Adding a New Plugin

Checklist for adding a new first-party plugin:

1. **Create crate** -- add a new crate under `crates/plugins/` (e.g. `crates/plugins/my-plugin/`).
2. **Follow the plugin source layout** above -- split trait impls into focused submodules from the start.
3. **Implement `PluginConfig`** -- implement `validate()`, `form_schema()`, and secret masking methods on
   your config struct. Implement `TypeSettings` if the plugin needs tenant-level settings.
4. **Use `declare_plugin!`** -- declare the plugin's ID, family, config, host requirements, and config
   test kinds. The macro generates the `DESCRIPTOR`, `PluginMeta` impl, and capability list.
5. **Implement role traits** -- implement the role-specific traits your plugin supports
   (`VersionDetectorPlugin`, `ReleaseFetcherPlugin`, `UpdateExecutorPlugin`, `DiscoveryPlugin`, etc.).
6. **Register in `PluginRegistry`** -- add a single entry to the `register_plugins!` macro invocation
   in `crates/plugins/infrastructure/registry/src/registry.rs`. The macro generates all dispatch methods
   automatically.
7. **Add tests** -- cover success and failure paths for all implemented methods.

### The `register_plugins!` macro

The **Plugin Registry** crate centralizes all plugin operations using a `register_plugins!` macro
that generates all dispatch methods from a single declaration:

```rust
register_plugins! {
    ReleasesGithub   => { config: GitHubConfig,                plugin: GitHubPlugin },
    ReleasesGitlab   => { config: GitLabConfig,                plugin: GitLabPlugin },
    ReleasesForgejo  => { config: ForgejoConfig,               plugin: ForgejoPlugin },
    ReleasesDocker   => { config: DockerConfig,                plugin: DockerPlugin },
    DiscoveryProxmoxHelperScripts =>
                        { config: ProxmoxHelperScriptsConfig,  plugin: ProxmoxHelperScriptsPlugin },
    PackageManagerHomebrew => { config: HomebrewConfig,        plugin: HomebrewPlugin },
    PackageManagerApt =>     { config: AptConfig,              plugin: AptPlugin },
    PackageManagerNpm =>     { config: NpmConfig,              plugin: NpmPlugin },
    PackageManagerMas =>     { config: MasConfig,              plugin: MasPlugin },
    GenericShell =>          { config: ShellConfig,            plugin: ShellPlugin },
}
```

The macro generates these methods:

- `PluginRegistry::create_plugin()` -- deserializes config, validates it, and instantiates the plugin.
- `PluginRegistry::validate_config()` -- deserializes and validates plugin configuration JSON.
- `PluginRegistry::mask_config_secrets()` / `restore_config_secrets()` -- handles secret masking for
  API responses (delegates to the `PluginConfig` trait implemented on each config struct).
- `PluginRegistry::create_plugin_for_discovery()` -- same as `create_plugin` but without calling
  `validate()`, so discovery works with empty or minimal configs.
- `PluginRegistry::discovery_plugins()` -- returns the list of `PluginTypeId` values whose plugin
  reports `PluginCapability::DiscoverLocalSoftware`. Fully auto-derived from the macro -- no manual
  list needed.
- `PluginRegistry::validate_package_identifier()` -- dispatches to
  `<Config>::validate_identifier(value)` for each registered plugin type.
- `PluginRegistry::config_form_schema()` -- dispatches to `<Config>::form_schema()` for each plugin type.

**Discovery capability is registry-derived.** Use `state.plugin_ops.discovery_plugins()` in
route handlers (or `PluginRegistry::discovery_plugins()` statically) to get the current list
of discovery-capable plugin types. Do not maintain a separate static list or override method.

### Package identifier validation

Some plugins impose format constraints on the `package_identifier` field of a `SoftwareItem`. For
example, Homebrew identifiers must not contain whitespace, path-traversal segments (`..`, `.`), or
characters outside `[A-Za-z0-9\-_.@+/]`.

Plugin-specific identifier validation is exposed through two APIs:

**Static dispatch (preferred for internal use):**

```rust
PluginRegistry::validate_package_identifier(
    &PluginTypeId::from_static("package_manager_homebrew"),
    value,
)?;
```

**Trait object dispatch (via `PluginOps`):**

```rust
state.plugin_ops.validate_package_identifier_str(&config.plugin_type, value)?;
```

Returns `Ok(())` for unknown plugin types (no constraints apply). Returns `Err(String)` with a
human-readable message when the identifier is invalid.

**When adding a new plugin with identifier constraints:**

1. Implement `validate_identifier()` on your `PluginConfig`:

   ```rust
   impl PluginConfig for MyConfig {
       fn validate_identifier(value: &str) -> Result<(), String> {
           IDENTIFIER_RULES.validate(value)
       }

       // ... other PluginConfig methods
   }
   ```

   Use a `const IDENTIFIER_RULES: PackageIdentifierRules` for the common case:

   ```rust
   use uptrakit_shared_types::PackageIdentifierRules;

   const IDENTIFIER_RULES: PackageIdentifierRules = PackageIdentifierRules {
       min_len: 2,
       max_len: 64,
       first_char_valid: |c| c.is_ascii_alphanumeric() || c == '_',
       char_valid: |c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'),
       reject_double_dot: true,
   };
   ```

   For plugins with non-trivial identifier formats (e.g. npm's `@scope/name`), add extra checks
   after calling `IDENTIFIER_RULES.validate(value)?`.

   If your plugin imposes **no** constraints on `package_identifier`, use the default no-op
   implementation (returns `Ok(())`).

2. Add your plugin to the `register_plugins!` macro invocation in
   `crates/plugins/infrastructure/registry/src/registry.rs`. The macro automatically generates
   `PluginRegistry::validate_package_identifier()` by calling
   `<YourConfig>::validate_identifier(value)` for each registered plugin type -- no manual match
   arm is required.

3. Add unit tests in your plugin crate covering valid identifiers, empty identifiers, and all
   constraint violations.

Do **not** add plugin-specific identifier validation logic to the web API layer or query helpers. All
identifier validation must go through `PluginRegistry::validate_package_identifier`.

> **Implementation note:** `validate_package_identifier` is generated by the `register_plugins!`
> macro. Adding a plugin to the macro and implementing `PluginConfig::validate_identifier` is
> sufficient; the registry dispatch is updated automatically.

### Version string validation

Plugins that interpolate a `to_version` parameter into install commands must validate the version
string before command construction. This provides defense in depth even though `CommandSpec::exec()`
mode prevents shell injection -- package managers have their own argument parsing that can be
exploited with crafted version strings.

**Required pattern:** Add a `pub fn validate_version(version: &str) -> Result<(), String>` to your
plugin crate and call it at the top of `execute_update()`:

```rust
pub fn validate_version(version: &str) -> Result<(), String> {
    if version.is_empty() {
        return Err("version must not be empty".to_string());
    }
    if version.len() > 256 {
        return Err("version must not exceed 256 characters".to_string());
    }
    // Plugin-specific character whitelist and prefix rejection
    // ...
    Ok(())
}
```

In `execute_update()`:

```rust
validate_version(to_version)
    .map_err(|e| report!(PluginError::Configuration(e)))?;
```

**Per-plugin validation rules:**

| Plugin | Allowed characters | Additional rejections |
| :--- | :--- | :--- |
| npm | `[a-zA-Z0-9._+-]` | Protocol prefixes: `file:`, `git+`, `http:`, `https:` |
| apt | `[a-zA-Z0-9.+~:-]` | Leading `-` (would be interpreted as a flag) |

**Testing:** Add unit tests covering valid versions, boundary cases (empty, max length), and
injection attempts (protocol prefixes for npm, flag injection for apt).

See also: [Security -- Input Validation](../security/secure-development.md#plugin-input-validation).

### Bidirectional error conversion

Every plugin crate defines its own error enum (e.g., `DockerError`, `GitHubError`) and implements
**bidirectional** `impl_report_conversion!` between the plugin-specific error and the shared
`PluginError`:

```rust
use uptrakit_shared_macros::impl_report_conversion;

// Plugin-specific -> shared (for the registry to propagate errors)
impl_report_conversion!(DockerError => PluginError, |e| PluginError::PluginInternal(e.to_string()));

// Shared -> plugin-specific (for plugins calling shared code that returns PluginError)
impl_report_conversion!(PluginError => DockerError, |e| DockerError::Configuration(e.to_string()));
```

This bidirectional pattern allows:

- The plugin registry to convert plugin errors into `PluginError` when dispatching.
- Plugin implementations to call shared code (e.g., from `uptrakit-plugin-infrastructure-core`) and convert
  `PluginError` back into their local error type.

When adding a new plugin, always implement both directions.

The agent crate imports `uptrakit-command` for shell execution and `uptrakit-plugin-infrastructure-registry` for
plugin dispatch -- it does not depend on `uptrakit-plugin-infrastructure-core` directly. The web-api crate imports
`uptrakit-plugin-infrastructure-registry` (not `uptrakit-plugin-infrastructure-core`). The wire protocol crate
(`uptrakit-internal-wire`) imports `PluginTypeId`, `ReleaseAsset`, and `ReleaseInfo` directly from
`uptrakit-shared-types`, keeping it free of plugin-implementation dependencies.

The update step can always be overridden by a custom shell script, regardless of plugin.

## Software Discovery

The `DiscoveryPlugin` role trait includes a `discover_software()` method that allows plugins to enumerate
software they can manage on the local system. The method returns a `Vec<DiscoveredSoftware>`, where each
entry contains:

| Field | Type | Description |
| :--- | :--- | :--- |
| `package_identifier` | `String` | Plugin-specific identifier (maps to `host_software_items.package_identifier`). |
| `name` | `String` | Human-readable display name. |
| `installed_version` | `String` | Currently installed version (required; plugins omit items with unknown versions). |
| `featured` | `bool` | Controls visibility: `true` = individual entry in Software list, `false` = aggregated per-host summary. |
| `targets` | `Vec<DiscoveryTarget>` | Structured targets for plugin config creation. Empty = use discovering plugin's config. |
| `extra` | `Option<serde_json::Value>` | Informational metadata only (e.g. Docker container names). Not used for config synthesis. |

The default implementation returns an empty list. Plugins that support discovery (e.g.,
Proxmox Helper-Scripts) override this method to scan the local system.

### Featured flag routing

Every discovery plugin must set the `featured` field on each `DiscoveredSoftware` item.
This flag controls how the controller presents the discovered item in the UI:

- **`featured: true`** -- the item appears individually in the main Software list with
  role-based plugin assignments (`host_software_item_plugins`). Use this for items the user
  explicitly wants to track (Docker images, GitHub releases, PHS-discovered apps).

- **`featured: false`** -- the item appears as part of aggregated per-host package summaries.
  The `plugin_config_id` and `package_identifier` are stored directly on the
  `host_software_items` junction row. Use this for package managers that discover large numbers
  of system packages (APT, Homebrew, npm, Cargo, Snap).

All discovered items are created immediately with `enabled: true` -- there is no pending state
or approval workflow.

```rust
DiscoveredSoftware {
    package_identifier: "nginx".to_string(),
    name: "nginx".to_string(),
    installed_version: "1.24.0".to_string(),
    featured: false,
    targets: vec![],
    extra: None,
}
```

**Current plugin featured assignment:**

| Plugin | Mode | Featured |
| :--- | :--- | :--- |
| APT | all modes | `false` |
| Homebrew | all modes | `false` |
| npm | all modes | `false` |
| Cargo | all modes | `false` |
| Snap | all modes | `false` |
| Docker | all modes | `true` |
| Proxmox Helper Scripts | all modes | `true` |

The controller's `process_discovery_results()` uses the `featured` flag to determine the
storage strategy. For non-featured items the controller resolves the plugin config ID:

1. If `result.plugin_config_id` is `Some(_)` (pre-existing config), that ID is used directly.
2. Otherwise, the controller reads `item.targets.first()` and calls
   `find_or_create_default_plugin_config()` to auto-create the config on the first run.
3. If neither is present the item is skipped with a warning.

Once the config ID is known, the controller checks the software ignore list and either updates
an existing `host_software_items` record or creates a new one. For featured items, the controller
follows the `find_or_create_software_item()` path with role-based plugin assignments.

See [Autodiscovery](../end-user/autodiscovery.md) for the end-user perspective.

### Emitting `DiscoveryTarget` values

When your plugin discovers software that should be tracked by a **different** plugin type (cross-plugin
discovery), or when running without a pre-existing plugin config, emit `DiscoveryTarget` values in the
`targets` field of each `DiscoveredSoftware` item:

```rust
use uptrakit_plugin_infrastructure_core::{DiscoveredSoftware, DiscoveryTarget, PluginRole, PluginTypeId};

DiscoveredSoftware {
    package_identifier: "booklore".to_string(),
    name: "BookLore".to_string(),
    installed_version: "1.18.5".to_string(),
    targets: vec![DiscoveryTarget {
        plugin_type: PluginTypeId::from_static("releases_github"),
        plugin_config: serde_json::json!({
            "owner": "BookLore",
            "repo": "BookLore",
        }),
        plugin_config_name: "BookLore/BookLore".to_string(),
        roles: vec![
            PluginRole::DetectVersion,
            PluginRole::FetchReleases,
            PluginRole::ExecuteUpdate,
        ],
        package_identifier: None,
        config: None,
        execution_site: None,
    }],
    extra: None,
}
```

**When to emit targets:**

- Your plugin discovers software that should be managed by a different plugin type (e.g. PHS
  discovers GitHub-managed apps).
- Your plugin needs the controller to auto-create named configs (e.g. Homebrew emitting
  `"Homebrew (Formulae)"` / `"Homebrew (Casks)"`). All package manager plugins always emit targets.

**When to leave targets empty:**

- Your plugin is running with an existing `plugin_config_id` and all discovered items should use
  that config for all roles. The controller will use the config-ID-based path.

The controller processes targets generically: for each target, it finds or creates a plugin config
matching `(plugin_type, plugin_config)` and creates role assignments per `target.roles`. No
plugin-specific synthesis logic exists in the controller.

## Batch Updates

The `UpdateExecutorPlugin` role trait includes an optional `execute_batch_update()` method for plugins
that can update multiple packages in a single system command. This is primarily used by non-featured
software items where a package manager might update dozens of packages at once.

### Trait method

```rust
async fn execute_batch_update(
    &self,
    updates: &[BatchUpdateItem],
    output_tx: &mpsc::Sender<UpdateOutputLine>,
) -> Result<Vec<BatchUpdateResult>>
```

The default implementation falls back to calling `execute_update()` sequentially for each item.
Plugins that support efficient batch operations should override this method.

### Types

```rust
pub struct BatchUpdateItem {
    pub package_identifier: String,
    pub to_version: String,
    pub release_info: Option<ReleaseInfo>,
}

pub struct BatchUpdateResult {
    pub package_identifier: String,
    pub success: bool,
    pub output: String,
}
```

Both types are defined in `crates/plugins/infrastructure/core/src/batch_update.rs` and re-exported
from `uptrakit-plugin-infrastructure-core`.

### Implementation examples

**APT** -- uses `apt_preferences` pin-priority mechanism for safe, targeted upgrades:

1. Generate a preferences file that blocks all upgrades (`Pin-Priority: -1`) except the requested
   packages (pin at priority 990).
2. Write to a temp file (no sudo needed -- agent owns it).
3. Run `sudo apt-get -o Dir::Etc::Preferences=<temp-file> upgrade --yes`.
4. Delete the temp file.

This approach preserves auto/manual package marks and is crash-safe (the temp file is not in
`/etc/apt/preferences.d/`).

**Homebrew** -- runs `brew upgrade pkg1 pkg2 ...` as a single command.

**npm** -- runs `npm install -g pkg1@v1 pkg2@v2 ...` as a single command.

### When to implement batch updates

Override `execute_batch_update()` when your plugin's package manager supports updating multiple
packages in a single command invocation. This avoids the overhead of separate process spawns and
index refreshes per package.

If your plugin does not benefit from batching (e.g., each update requires a unique download and
install), the default sequential fallback is sufficient.

See [Unified Software Tracking](../architecture/unified-software-tracking.md) for the full
batch update flow from controller to agent.

## Batch Version Check

The plugin role traits include two optional batch methods for efficient version checking when the same
plugin handles many packages. Both have default sequential fallbacks so existing plugins need no
changes; native implementations reduce N subprocess or API calls to one.

### Trait methods

```rust
async fn batch_detect_installed_version(
    &self,
    items: &[BatchDetectItem],
) -> Result<Vec<BatchDetectResult>>

async fn batch_fetch_releases(
    &self,
    items: &[BatchFetchItem],
) -> Result<Vec<BatchFetchResult>>
```

### Types

```rust
pub struct BatchDetectItem {
    pub package_identifier: String,
}

pub struct BatchDetectResult {
    pub package_identifier: String,
    pub installed_version: Option<Version>, // None = not installed
    pub error: Option<String>,              // None = success
}

pub struct BatchFetchItem {
    pub package_identifier: String,
}

pub struct BatchFetchResult {
    pub package_identifier: String,
    pub releases: Vec<UpstreamRelease>,
    pub error: Option<String>,
}
```

All four types are defined in `crates/plugins/infrastructure/core/src/batch_detect.rs` and
`crates/plugins/infrastructure/core/src/batch_fetch.rs`, and re-exported from
`uptrakit-plugin-infrastructure-core`.

### Default behaviour

The default implementations call `detect_installed_version()` or `fetch_releases()` sequentially for
each item. A per-item error is stored in `BatchDetectResult::error` or `BatchFetchResult::error`
rather than failing the entire batch. An empty input slice returns an empty result.

### When to override

Override these methods when your plugin's package manager accepts multiple packages in a single
command invocation, producing equivalent results to N individual calls:

- **`batch_detect_installed_version`**: your detection command accepts a package list (e.g.
  `dpkg-query pkg1 pkg2`, `brew info --json=v2 pkg1 pkg2`, `npm list -g --depth=0 --json`).
- **`batch_fetch_releases`**: your local index query accepts a package list (e.g.
  `apt-cache madison pkg1 pkg2`, `brew info --json=v2 pkg1 pkg2`).

Do **not** override these for plugins whose detection or fetching is inherently per-package (e.g.
one HTTP request per package to a remote API). The sequential default is correct in those cases.

### Implementation examples

#### APT -- `batch_detect_installed_version`

Runs one `dpkg-query` call for all packages:

```text
dpkg-query --show --showformat='${Package}\t${Version}\n' pkg1 pkg2 pkg3
```

- Exit code is ignored (non-zero when any package is unknown; found packages still appear in stdout).
- Parse stdout line-by-line: split on `\t` -> `(package, version)`.
- Empty version string -> `installed_version: None, error: None` (known-uninstalled).
- Package absent from stdout -> `installed_version: None, error: None` (not installed).

#### APT -- `batch_fetch_releases`

Runs one `apt-cache madison` call for all packages:

```text
apt-cache madison pkg1 pkg2 pkg3
```

- Lines grouped by the first `|`-delimited field (package name, trimmed).
- First line per package is the highest-priority available version.

#### Homebrew -- both methods

Passes all packages to a single `brew info --json=v2` call:

```text
brew info --json=v2 pkg1 pkg2 pkg3
```

The existing `parse_installed_version(json, pkg, is_cask)` and `parse_latest_version(json, pkg,
is_cask)` helpers already search the returned JSON array by name, so they work for batch results
without modification.

#### npm -- `batch_detect_installed_version`

Fetches all globally installed packages in one call (no package filter):

```text
npm list -g --depth=0 --json
```

Results are filtered in memory via a `HashMap`. If the command fails, all items are treated as
not installed (consistent with the single-item behaviour). `batch_fetch_releases` keeps the default
sequential fallback because the npm registry has no batch endpoint.

### How the system uses batch methods

#### Agent-core `batch_check_versions()`

`run_check_versions` (via `batch_check_versions`) groups `VersionCheckAssignment` entries by
`(PluginTypeId, effective_config_json)` before calling plugins. For each group:

1. One plugin instance is created.
2. `batch_detect_installed_version` is called for all items in the detect group.
3. `batch_fetch_releases` is called for all items in the fetch group.
4. Groups run in parallel via `join_all`.

`RefreshPackageIndex` is called once per unique group, regardless of the number of items sharing
that group.

#### Scheduler Phase A

The controller-side `run_controller_side_fetch_releases` groups rows by `plugin_config_id` (instead
of `(plugin_config_id, package_identifier)`). A single `batch_fetch_releases` call per config
replaces the previous N-per-package loop.

## Notification Plugin Extension Actions

Notification plugins extend the UI via the extension framework. Each notification plugin
owns its own `extensions.rs` module with:

- `extension_manifests()` -- returns UI manifests for channel management
- `extension_actions()` -- returns the action catalogue
- `handle_action(ctx, extension_id, action_id, params)` -- dispatches actions

### `handle_action` signature

```rust
pub async fn handle_action(
    ctx: &ExtensionActionContext<'_>,
    extension_id: &str,
    action_id: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    match (extension_id, action_id) {
        ("notifications.mytype", "list") => {
            list_channels(ctx, "mytype", params).await
        }
        // ... other actions
        _ => Err(format!("unknown action '{action_id}'")),
    }
}
```

`ExtensionActionContext` provides `db`, `tenant_id`, and `caller_user_id` (for actions
that need the calling user, such as sending test emails to the user's profile address).

### Shared `list_channels` helper

All notification plugins share a `list_channels` helper from `uptrakit-notification-plugin-core`
(behind the `extensions` feature). It handles:

- Querying channels by type with tenant scoping
- Decrypting and parsing channel config
- Masking secrets via the plugin's `PluginConfig::with_secrets_masked()`
- Flattening top-level config keys into the row object for `DataTable` rendering
- Pagination with `page`/`per_page` parameters

Usage:

```rust
use uptrakit_notification_plugin_core::list_channels::list_channels;

("notifications.mytype", "list") => {
    list_channels(ctx, "mytype", params).await
}
```

### Settings management via raw-key functions

Plugins use raw-key settings store functions instead of `SettingKey` enum variants.
This keeps notification-specific settings decoupled from the shared `SettingKey` type:

- `upsert_setting_raw(db, tenant_id, key, value)` -- tenant setting
- `upsert_global_setting_raw(db, key, value)` -- global setting
- `load_settings_by_prefix(db, tenant_id, prefix)` -- load tenant settings by prefix
- `load_global_settings_by_prefix(db, prefix)` -- load global settings by prefix

See the email plugin's `extensions.rs` for a complete example of SMTP settings management
using these functions.

## Testing

### Mock executor pattern

Use a `FixedExitCodeExecutor` style mock in unit tests to avoid spawning real processes. This
pattern keeps tests fast, deterministic, and independent of the host environment:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use uptrakit_plugin_infrastructure_core::{CommandOutput, HostCompatibility};
    use uptrakit_command::{CommandExecutor, CommandSpec};

    struct FixedExitCodeExecutor {
        exit_code: i32,
    }

    impl FixedExitCodeExecutor {
        fn with_exit_code(exit_code: i32) -> Arc<dyn CommandExecutor> {
            Arc::new(Self { exit_code })
        }
    }

    #[async_trait::async_trait]
    impl CommandExecutor for FixedExitCodeExecutor {
        async fn execute(
            &self,
            _spec: &CommandSpec,
            _output_tx: &tokio::sync::mpsc::Sender<UpdateOutputLine>,
        ) -> uptrakit_command::Result<CommandOutput> {
            Ok(CommandOutput { output: String::new(), exit_code: self.exit_code })
        }

        async fn execute_quiet(
            &self,
            _spec: &CommandSpec,
        ) -> uptrakit_command::Result<CommandOutput> {
            Ok(CommandOutput { output: String::new(), exit_code: self.exit_code })
        }
    }
}
```

### Testing POSIX plugins

POSIX plugins require a runtime that provides a `CommandExecutor`. Use `TestRuntime::with_posix_executor()`
or wrap a mock executor:

```rust
#[tokio::test]
async fn detect_host_compatibility_when_tool_present() {
    let runtime = Arc::new(TestRuntime::with_executor(
        FixedExitCodeExecutor::with_exit_code(0),
    ));
    let plugin = AptPlugin::new(AptConfig::default(), runtime).unwrap();
    let result = plugin.detect_host_compatibility().await.unwrap();
    assert_eq!(result, HostCompatibility::Compatible);
}

#[tokio::test]
async fn detect_host_compatibility_when_tool_absent() {
    let runtime = Arc::new(TestRuntime::with_executor(
        FixedExitCodeExecutor::with_exit_code(1),
    ));
    let plugin = AptPlugin::new(AptConfig::default(), runtime).unwrap();
    let result = plugin.detect_host_compatibility().await.unwrap();
    assert!(matches!(result, HostCompatibility::Incompatible(_)));
}
```

### Testing controller-side plugins

Controller-side plugins require a `ControllerRuntime`. Use `TestControllerRuntime` in tests:

```rust
#[tokio::test]
async fn fetch_releases_returns_versions() {
    let runtime = Arc::new(TestControllerRuntime::new());
    let config = GitHubConfig {
        owner: "test".to_string(),
        repo: "repo".to_string(),
        ..Default::default()
    };
    let plugin = GitHubPlugin::new(config, runtime).unwrap();
    // ... test fetch_releases
}
```

See also:

- [Plugin System Architecture](plugin-system.md) -- how plugins relate to software items and host assignments.
- [Command Executor](command-executor.md) -- full `CommandExecutor` trait reference.
- [Sudoers Management](../security/sudoers-management.md) -- security model for privileged commands.

## GitHub Releases plugin (`uptrakit-plugin-releases-github`)

Fetches release metadata from the GitHub API and converts it into `UpstreamRelease` values.

**Config fields (`GitHubConfig`):**

| Field | Type | Required | Default | Description |
| :--- | :--- | :--- | :--- | :--- |
| `owner` | String | Yes | -- | GitHub repository owner. |
| `repo` | String | Yes | -- | GitHub repository name. |
| `auth_token` | String | No | `null` | Personal access token (private repos or higher rate limits). |
| `api_base_url` | String | No | `https://api.github.com` | API base URL (for GitHub Enterprise). |
| `include_prereleases` | bool | No | `false` | Whether to include pre-release versions. |
| `tag_strip_prefix` | String | No | `"v"` | Prefix to strip from tag names to extract version strings. |
| `asset_patterns` | `Vec<String>` | No | `[]` | Regex patterns to filter release assets (empty means all). |
| `install_command` | `Option<String>` | No | `null` | Custom shell command to execute after downloading the release asset. Supports `{version}`, `{tag}`, `{asset_url}`, `{asset_name}` placeholders (shell-escaped). |
| `detect_installed_version_command` | `Option<String>` | No | `null` | Shell command to detect the installed version on the agent host. The first non-empty trimmed line of stdout is used. Supports `{package_identifier}` placeholder (shell-escaped). If absent, `detect_installed_version()` returns `None`. |

**Behaviour:**

- Drafts are always skipped
- Rate limit headers are checked; warnings logged when remaining < 10
- 403/429 responses with `x-ratelimit-remaining: 0` return a rate-limit error
- Asset filtering uses regex matching against asset names
- `detect_installed_version_command` runs via the injected `CommandExecutor` (local or SSH)

## Docker plugin (`uptrakit-plugin-releases-docker`)

Tracks container image tags from OCI/Docker registries. Supports Docker Hub, GHCR, and any OCI
Distribution Spec-compliant registry. Supports both controller-side upstream version resolution and
agent-side container discovery.

See [Docker Plugin](../end-user/plugins/docker.md) for the full end-user reference.

## Proxmox Helper Scripts plugin (`uptrakit-plugin-discovery-proxmox-helper-scripts`)

Discovery-only plugin for software installed via [Proxmox VE community helper scripts](https://github.com/community-scripts/ProxmoxVE).
Does **not** perform version detection, upstream release fetching, or update execution. Its sole
responsibility is to discover which PHS-managed apps are present in a container and emit
`DiscoveryTarget` values that tell the controller which downstream plugin configs to create.

**Config (`ProxmoxHelperScriptsConfig`):** No fields -- the config is always `{}`.

**Capabilities:** `DiscoverLocalSoftware` only (auto-derived from `DiscoveryPlugin` role).

**Discovery targets emitted:**

- GitHub-managed apps: `DiscoveryTarget { plugin_type: PluginTypeId::from_static("releases_github"), ... }` with
  owner, repo, `detect_installed_version_command`, and `install_command` pre-configured. Constants
  `PHS_DETECT_VERSION_CMD` and `PHS_INSTALL_CMD` are defined in
  `crates/plugins/discovery/proxmox-helper-scripts/src/discovery.rs`.
- APT-managed apps: `DiscoveryTarget { plugin_type: PluginTypeId::from_static("package_manager_apt"), config: {}, name: "APT (auto)" }`.

Cross-reference: [PHS end-user guide](../end-user/autodiscovery.md#proxmox-helper-scripts-discovery),

## Shared Update Helpers

All package-manager plugins must use the shared helper functions from
`uptrakit-plugin-infrastructure-core` instead of hand-rolling their own command execution or
validation boilerplate. This keeps each plugin's trait implementation under ten lines,
guarantees consistent output formatting, and centralises error mapping in one tested location.

### Available helpers

#### `require_package_identifier`

```rust
pub fn require_package_identifier(
    value: &str,
    predicate: impl FnMut(&str) -> std::result::Result<(), String>,
) -> Result<()>
```

One-liner wrapper for identifier validation. Calls `predicate(value)` and maps any `Err(message)`
to `PluginError::Configuration`. Every package-manager plugin's `require_package_identifier` method
delegates here:

```rust
fn require_package_identifier(&self, id: &str) -> Result<()> {
    uptrakit_plugin_infrastructure_core::require_package_identifier(id, validate_identifier)
}
```

#### `execute_command_update` and `CommandUpdateParams`

Executes a single-package update via `executor.execute()`. Sends `"Running: {binary} {args}"` to
the output stream and returns the combined output string. Supports an optional `spec_modifier`
closure (e.g., setting `DEBIAN_FRONTEND=noninteractive` for APT) and configurable exit-code
success predicates (e.g., `Some(|_| true)` for `mas upgrade` which may exit non-zero even on
success).

#### `execute_batch_versioned_command` and `BatchVersionedParams`

Executes a batch update in a single command where each package argument includes a version
(e.g., `pkg1-ver1 pkg2-ver2`). Use this for DNF (`pkg-ver` separator), APK (`pkg=ver`), and
npm (`pkg@ver`). Validates all identifiers and versions before execution. Returns one
`BatchUpdateResult` per input item, all sharing the same success flag and output.

#### `execute_batch_names_command` and `BatchNamesParams`

Executes a batch update in a single command where only package names are passed (the package
manager resolves the version itself from the install instruction). Use this for Pacman, snap,
pkg (BSD), and Homebrew. Supports optional `suffix_args` (e.g., `--channel=stable` for snap)
and an optional version validator (pre-flight check only -- the version is not forwarded to the
command).

#### `refresh_package_index_command`

Refreshes a package index by running a `CommandSpec` quietly via `execute_and_capture`. Logs
`"refreshing {label}"` before and `"{label} refreshed"` after. Use this in `PackageIndexer`
implementations (APT, APK, DNF, pkg):

```rust
async fn refresh_package_index(&self) -> Result<()> {
    uptrakit_plugin_infrastructure_core::refresh_package_index_command(
        self.executor.as_ref(),
        CommandSpec::exec("dnf", ["makecache".to_string(), "-q".to_string()]).privileged(),
        "DNF package index",
    )
    .await
}
```

### Unit test helpers

The `testing` Cargo feature (enabled via `[dev-dependencies]`) exposes:

- `testing::test_runtime()` -- builds a `HostRuntime` backed by `LocalCommandExecutor`. Use
  only for tests that need to detect real host compatibility against the current environment.
- `testing::test_runtime_with_executor(executor)` -- builds a `HostRuntime` backed by any
  `Arc<dyn CommandExecutor>`. Use this in the vast majority of tests where you control output
  through `FixedOutputExecutor` or `RoutedOutputExecutor`.
- `testing::FixedOutputExecutor` -- returns the same output and exit code for every command.
  Constructors: `::success(output)`, `::failure(exit_code)`, `::new(output, exit_code)`.
- `testing::RoutedOutputExecutor` -- routes calls by command program name. Constructors:
  `::success(pairs)` (all exit 0), `::new(triples)` (per-route exit codes).

Enable in `Cargo.toml`:

```toml
[dev-dependencies]
uptrakit-plugin-infrastructure-core = { workspace = true, features = ["testing"] }
```

These replace any locally defined `test_runtime`, `test_runtime_with_executor`, or
`FixedExitCodeExecutor` structs that previously lived inside individual plugin test modules.

### Canonical import path

All helpers are re-exported from the crate root:

```rust
use uptrakit_plugin_infrastructure_core::{
    require_package_identifier, ValidatorFn,
    CommandUpdateParams, execute_command_update,
    BatchVersionedParams, execute_batch_versioned_command,
    BatchNamesParams, execute_batch_names_command,
    refresh_package_index_command,
};
use uptrakit_plugin_infrastructure_core::testing::{
    FixedOutputExecutor, RoutedOutputExecutor, test_runtime, test_runtime_with_executor,
};
```

Cross-reference: inline module documentation in
`crates/plugins/infrastructure/core/src/helpers.rs` and `src/testing.rs`.
[PHS API notes](../api/autodiscovery.md#plugin-driven-discovery-targets).
