# Plugin Development Guidelines

Plugins are first-party extension modules that detect, report, and update software on managed hosts.
Each plugin crate implements the `Plugin` trait and is registered in `uptrakit-plugin-infrastructure-registry`. This
document describes the full lifecycle and conventions for building and extending plugins.

When adding or changing a plugin, document the full lifecycle:

- How the agent detects the installed version.
- How the controller resolves the latest upstream version.
- Version comparison rules (semver, tag prefixes, build metadata handling).
- Update execution steps, required privileges, and failure modes.
- Required configuration fields with examples.
- Any assumptions about the agent environment or custom scripts.

Plugins should keep parsing and comparison logic in pure functions so they are easy to test.

The plugin registry crate (`uptrakit-plugin-infrastructure-registry`) centralizes config validation, mask/restore
workflows, and creates plugin instances based on `PluginType`. Document plugin behavior so the registry
can continue to validate configs and mask secrets correctly.

`PluginType` implements `FromStr`, `Display`, and `as_str()` for string conversion. Use
`s.parse::<PluginType>()` to convert strings (returns `ParsePluginTypeError` on failure). The string
representations are: `releases_github`, `releases_gitlab`, `releases_forgejo`, `releases_docker`,
`discovery_proxmox_helper_scripts`, `package_manager_homebrew`, `package_manager_apt`,
`package_manager_npm`, `generic_shell`.

## Plugin Capabilities

The `PluginCapability` enum defines optional features a plugin may support. Plugins declare their
capabilities by implementing `capabilities() -> Vec<PluginCapability>` on the `Plugin` trait. All
other trait methods have default no-op implementations; plugins override only what they support.

| Capability | Trait method | Description |
| :--- | :--- | :--- |
| `DiscoverLocalSoftware` | `discover_software()` | Enumerate software the plugin can manage on the local system. |
| `RefreshPackageIndex` | `refresh_package_index()` | Refresh local package index (for example, `apt update`). |
| `DetectHostCompatibility` | `detect_host_compatibility()` | Determine whether this plugin is applicable to the current host environment. |
| `PreUpdateHook` | `pre_update_hook()` | Run plugin-level logic before an update begins; can abort the update. |
| `PostUpdateHook` | `post_update_hook()` | Run plugin-level logic after an update completes; non-fatal. |
| `ControllerSideFetchReleases` | _(no trait method)_ | Declares that `fetch_releases()` can run on the controller without local system state. See [Declaring ControllerSideFetchReleases](#declaring-controllersidefetchreleases). |

## Host Compatibility Detection

Plugins that declare `PluginCapability::DetectHostCompatibility` implement
`detect_host_compatibility()` to determine whether the plugin is applicable to the host where
the agent is running. The method returns a `HostCompatibility` enum:

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

**APT plugin** — checks whether `apt-get` is available:

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

**Docker plugin** — pings the Docker daemon directly (daemon build only):

```rust
#[cfg(feature = "daemon")]
async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
    // Use bollard's ping to verify the daemon is actually reachable,
    // including over SSH tunnels for remote hosts.
    match self.docker_client.ping().await {
        Ok(()) => Ok(HostCompatibility::Compatible),
        Err(e) => Ok(HostCompatibility::Incompatible(format!(
            "Docker daemon not accessible: {e}"
        ))),
    }
}
```

## Plugin Lifecycle Hooks

Plugins that declare `PreUpdateHook` or `PostUpdateHook` receive an `UpdateHookContext` containing
information about the update being performed:

```rust
pub struct UpdateHookContext {
    /// The package identifier for the software item being updated.
    pub package_identifier: String,
    /// The version being updated to.
    pub to_version: String,
    /// The currently installed version, if known.
    pub from_version: Option<String>,
}
```

### Pre-update hook

`pre_update_hook()` is called before the update step runs. It returns a `PreUpdateHookResult`:

```rust
pub enum PreUpdateHookResult {
    /// Continue with the update as planned.
    Proceed,
    /// Abort the update with a reason message (non-error; logged as a warning).
    Abort { reason: String },
}
```

If a pre-update hook returns `Abort`, the update is cancelled with the provided reason. The update
history entry records the abort reason. No post-update hook is run for aborted updates.

### Post-update hook

`post_update_hook()` is called after a successful update completes. It is non-fatal: any error
returned is logged as a warning but does not mark the update as failed.

**Example: APT plugin post-update hook** — checks `/var/run/reboot-required`:

```rust
fn post_update_hook(&self, context: &UpdateHookContext, executor: &dyn CommandExecutor)
    -> Result<(), PluginError>
{
    let output = executor.run(CommandSpec::new("test").arg("-f").arg("/var/run/reboot-required"))?;
    if output.exit_code == 0 {
        tracing::warn!(
            package = %context.package_identifier,
            "Reboot required after updating package"
        );
    }
    Ok(())
}
```

## Declaring `ControllerSideFetchReleases`

If your plugin's `fetch_releases()` implementation makes only HTTP/API calls and does not depend on
any local system state (no `CommandExecutor` calls, no filesystem access, no local package index),
you should declare `ControllerSideFetchReleases` in your `capabilities()`:

```rust
fn capabilities(&self) -> &'static [PluginCapability] {
    &[PluginCapability::ControllerSideFetchReleases]
}
```

**When to declare it:**

- Your `fetch_releases()` only performs HTTP requests to an external API (GitHub REST API, OCI
  registry, etc.).
- It does not call `self.executor.execute()` or `self.executor.execute_quiet()`.
- It does not read from the local filesystem or depend on a locally synced package index.

**When NOT to declare it:**

- Your plugin needs a local package index (e.g. Homebrew's `brew info --json`, APT's
  `apt-cache policy`). These must run agent-side.
- Your `fetch_releases()` shells out to a local CLI tool.

**Effect:** When `execution_site` is `auto` (the default), the controller runs `fetch_releases()`
once per unique `(plugin_config_id, package_identifier)` combination and propagates the result
to all hosts sharing that combination. This avoids redundant API calls when many hosts track the
same upstream release. The controller uses a `NoopCommandExecutor` — if your plugin accidentally
calls it, the process will panic.

**Current plugins with this capability:** `GitHubPlugin`, `GitLabPlugin`, `ForgejoPlugin`,
`DockerPlugin`, `NpmPlugin`.

## The Role Model for New Plugins

When implementing a new plugin, consider which of the three roles it will serve:

### Single-plugin-for-all-roles (common case)

Most plugins implement all three roles (`detect_version`, `fetch_releases`, `execute_update`) in
a single plugin crate. When autodiscovery or manual assignment creates host assignments, all three
role rows point to the same `plugin_config_id`. This is the default and requires no special
handling.

### Partial-role plugins

Some plugins only make sense for a subset of roles:

- **Discovery-only plugins** (e.g. `ProxmoxHelperScriptsPlugin`) — implement `discover_software()`
  but do not participate in any of the three version/update roles. The controller synthesizes
  downstream plugin configs that fill the role assignments.
- **Fetch-only plugins** — a hypothetical plugin that only knows how to fetch upstream releases
  (e.g. a custom changelog scraper). Users would pair it with another plugin for detection and
  updates.

When your plugin does not implement a particular role's method (e.g. it has no `execute_update()`),
the default trait implementation returns an error. The system will not call a role method on a
plugin that is not assigned to that role, so this is safe.

### Testing role assignments

When writing integration tests for a new plugin, verify that:

1. The plugin's `capabilities()` accurately reflects whether it declares
   `ControllerSideFetchReleases`.
2. All three role methods (`detect_installed_version`, `fetch_releases`, `execute_update`) either
   work correctly or return a clear error if the plugin does not support that role.

## Dependencies and re-exports

Plugin crates should avoid unnecessary direct dependencies. The `uptrakit-plugin-infrastructure-core` crate
re-exports commonly needed types:

- **`uptrakit_plugin_infrastructure_core::mpsc`** — re-export of `tokio::sync::mpsc`. Use this instead of
  depending on tokio directly. Tokio should only be in `[dev-dependencies]` (for `#[tokio::test]`).
- **`uptrakit_plugin_infrastructure_core::CommandExecutor`**, **`CommandSpec`**, etc. — re-exports from
  `uptrakit-command`.
- **`uptrakit_plugin_infrastructure_core::SecretString`** — re-export from `uptrakit-shared-types`.

See [Dependency Policy](dependency-policy.md) for the full re-export strategy.

## HTTP Client Requirements

Any plugin that builds its own `reqwest::Client` (e.g. for fetching upstream release metadata) **must**
configure at minimum a connect timeout and a total request timeout. An unconfigured client will hang
indefinitely against an unresponsive or slow registry, creating a denial-of-service vector against the
agent or controller process that loaded the plugin.

```rust
use std::time::Duration;

let client = reqwest::Client::builder()
    .user_agent(concat!(
        "uptrakit-plugin-my-plugin/",
        env!("CARGO_PKG_VERSION")
    ))
    .connect_timeout(Duration::from_secs(10))   // prevents hangs on unreachable hosts
    .timeout(Duration::from_secs(60))            // caps total request duration
    .build()
    .context_to::<MyPluginError>()?;
```

**Required timeouts:**

- `.connect_timeout(Duration::from_secs(10))` — prevents hanging on a host that accepts the TCP
  connection but never sends data.
- `.timeout(Duration::from_secs(60))` — caps the total wall-clock time of any single request
  (connect + read + write). Adjust upward only for endpoints with documented large response bodies.

**User-Agent:** Set a descriptive `User-Agent` that includes the crate name and version so that
upstream services can identify traffic originating from uptrakit. Use `env!("CARGO_PKG_VERSION")` to
keep the version in sync automatically.

## Command Executor Pattern

Plugins do not spawn processes directly. Instead, each plugin receives an `Arc<dyn CommandExecutor>`
at construction time and delegates all command execution through that trait. This decouples plugin
logic from the execution transport, enabling the same plugin code to run commands locally (via
`LocalCommandExecutor`) or remotely (via `SshCommandExecutor`).

See [Command Executor](command-executor.md) for the full trait reference, `CommandSpec` constructors,
and guidance on implementing custom executors.

### Declaring privileged commands with `required_sudo_commands()`

If your plugin needs passwordless `sudo` to run certain commands, implement
`required_sudo_commands()` on the `Plugin` trait:

```rust
use uptrakit_plugin_infrastructure_core::SudoCommandEntry;

fn required_sudo_commands(&self) -> Vec<SudoCommandEntry> {
    vec![SudoCommandEntry {
        command: "apt-get".into(),
        explanation: "Package installation and index refresh require root privileges".into(),
    }]
}
```

**Contract:**

- `command` is the **bare command name** (e.g. `"apt-get"`, `"systemctl"`), not an absolute path.
  The bootstrap and `update-sudoers` commands resolve the absolute path on the target host via
  `command -v <name>`.
- `explanation` is shown as a comment in the generated sudoers file and in CLI output. Keep it
  concise and factual.
- Return an empty `vec![]` (the default) when your plugin never needs elevated privileges.
- Do **not** hardcode `sudo` in your `CommandSpec` programs or arguments. Instead, call
  `.privileged()` on the spec and declare the corresponding command here so the sudoers file
  can be kept minimal.

The `PluginRegistry::all_required_sudo_commands()` static method aggregates declarations from all
registered plugins (using the same minimal-config instantiation as `create_plugin_for_discovery()`).
The bootstrap and `update-sudoers` SSH agent commands call this to build a per-command sudoers entry.

**Testing:**

Add a unit test verifying that your plugin returns the expected entries:

```rust
#[test]
fn required_sudo_commands_returns_expected_entries() {
    let plugin = MyPlugin::new(MyConfig::default(), Arc::new(LocalCommandExecutor)).unwrap();
    let cmds = plugin.required_sudo_commands();
    assert_eq!(cmds.len(), 1);
    assert_eq!(cmds[0].command, "my-tool");
    assert!(!cmds[0].explanation.is_empty());
}
```

See [Sudoers Management](../security/sudoers-management.md) for the full security rationale and
operator guidance.

## Plugin Trait: Required Methods

The `Plugin` trait (`crates/plugins/infrastructure/core/src/traits.rs`) defines the contract for all plugin
implementations. Two methods are required (no default implementation):

| Method | Signature | Description |
| :--- | :--- | :--- |
| `plugin_type` | `fn plugin_type(&self) -> PluginType` | Returns the plugin's type for introspection, logging, and telemetry. |
| `capabilities` | `fn capabilities(&self) -> Vec<PluginCapability>` | Declares which optional features the plugin supports. |

All other methods (`detect_installed_version`, `fetch_releases`, `execute_update`,
`discover_software`, `refresh_package_index`, `detect_host_compatibility`, `pre_update_hook`,
`post_update_hook`) have default implementations that return errors or empty results, so plugins
override only what they support.

When implementing a new plugin, always return the correct `PluginType` variant from `plugin_type()`.
This ensures that boxed `dyn Plugin` objects can be introspected after creation by
`PluginRegistry::create_plugin()`.

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
| `uptrakit-shared-types` | `crates/shared/types/` | Canonical home for `PluginType`, `ReleaseAsset`, and `ReleaseInfo` (plus `SecretString`, hex helpers). |
| `uptrakit-command` | `crates/shared/command/` | Shell execution, `CommandExecutor` trait, `CommandSpec`, `LocalCommandExecutor`. |
| `uptrakit-plugin-infrastructure-core` | `crates/plugins/infrastructure/core/` | Plugin trait/abstractions; re-exports shared types and executor types. |
| `uptrakit-plugin-infrastructure-registry` | `crates/plugins/infrastructure/registry/` | Centralized plugin dispatch and validation; re-exports `PluginType`. |
| `uptrakit-plugin-releases-docker` | `crates/plugins/releases/docker/` | Docker/OCI image tracking and container discovery. Implements `DetectHostCompatibility` (daemon build only, pings the Docker daemon via bollard `GET /_ping`). |
| `uptrakit-plugin-releases-github` | `crates/plugins/releases/github/` | GitHub Releases: controller-side fetch; agent-side install. |
| `uptrakit-plugin-releases-gitlab` | `crates/plugins/releases/gitlab/` | GitLab Releases: controller-side fetch; supports nested namespaces; PRIVATE-TOKEN auth. |
| `uptrakit-plugin-releases-forgejo` | `crates/plugins/releases/forgejo/` | Forgejo / Codeberg Releases: controller-side fetch; requires `api_base_url`; auto-detected by PHS discovery. |
| `uptrakit-plugin-package-manager-homebrew` | `crates/plugins/package-managers/homebrew/` | Homebrew: agent-side version tracking and updates. Implements `DetectHostCompatibility` (checks `which brew`). |
| `uptrakit-plugin-discovery-proxmox-helper-scripts` | `crates/plugins/discovery/proxmox-helper-scripts/` | Proxmox VE: auto-discovers and manages helper scripts. Implements `DetectHostCompatibility` (tests for `/usr/bin/update`, Proxmox VE only). |
| `uptrakit-plugin-package-manager-apt` | `crates/plugins/package-managers/apt/` | APT: Debian/Ubuntu package management. Implements `DetectHostCompatibility` (checks `which apt-get`) and `PostUpdateHook` (checks `/var/run/reboot-required`). |
| `uptrakit-plugin-package-manager-npm` | `crates/plugins/package-managers/npm/` | npm: global-package tracking via `registry.npmjs.org`. Implements `ControllerSideFetchReleases` and `DetectHostCompatibility` (checks `which npm`). |
| `uptrakit-plugin-generic-shell` | `crates/plugins/generic/shell/` | Generic shell plugin: custom `version_command` and `update_command`; agent-side only. |

## Adding a New Plugin

Checklist for adding a new first-party plugin:

1. **Create crate** — add a new crate under `crates/plugins/` (e.g. `crates/plugins/my-plugin/`).
2. **Implement `Plugin` trait** — implement `plugin_type()`, `capabilities()`, and all relevant
   optional methods.
3. **Declare capabilities** — include only the `PluginCapability` variants your plugin actually
   supports. Avoid declaring capabilities the plugin does not implement.
4. **Register in `PluginRegistry`** — add a single entry to the `register_plugins!` macro invocation
   in `crates/plugins/infrastructure/registry/src/registry.rs`. The macro generates all dispatch methods
   automatically.
5. **Add tests** — cover success and failure paths for all implemented methods.

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
    GenericShell =>          { config: ShellConfig,            plugin: ShellPlugin },
}
```

The macro generates seven methods:

- `PluginRegistry::create_plugin()` — deserializes config, validates it, and instantiates the plugin.
- `PluginRegistry::validate_config()` — deserializes and validates plugin configuration JSON.
- `PluginRegistry::mask_config_secrets()` / `restore_config_secrets()` — handles secret masking for
  API responses (delegates to the `SecretMasking` trait implemented on each config struct).
- `PluginRegistry::create_plugin_for_discovery()` — same as `create_plugin` but without calling
  `validate()`, so discovery works with empty or minimal configs.
- `PluginRegistry::discovery_plugins()` — returns the list of `PluginType` variants whose plugin
  reports `PluginCapability::DiscoverLocalSoftware` in `capabilities()`. Fully auto-derived from the
  macro — no manual list needed.
- `PluginRegistry::validate_package_identifier()` — dispatches to
  `<Config>::validate_identifier(value)` for each registered plugin type. Requires every config
  struct to implement the associated function (see [Package identifier validation](#package-identifier-validation)).

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
PluginRegistry::validate_package_identifier(PluginType::PackageManagerHomebrew, value)?;
```

**Trait object dispatch (via `PluginOps`):**

```rust
state.plugin_ops.validate_package_identifier_str(&config.plugin_type, value)?;
```

Returns `Ok(())` for unknown plugin types (no constraints apply). Returns `Err(String)` with a
human-readable message when the identifier is invalid.

**When adding a new plugin with identifier constraints:**

1. Add a crate-level `pub fn validate_identifier(value: &str) -> std::result::Result<(), String>`
   in your plugin crate (e.g. `crates/plugins/my-plugin/src/lib.rs`).
2. Add an associated function on your config struct that delegates to the crate-level function:

   ```rust
   impl MyPluginConfig {
       pub fn validate_identifier(value: &str) -> std::result::Result<(), String> {
           crate::validate_identifier(value)
       }
   }
   ```

   If your plugin imposes **no** constraints on `package_identifier`, add a no-op associated
   function that always returns `Ok(())`:

   ```rust
   impl MyPluginConfig {
       pub fn validate_identifier(_value: &str) -> std::result::Result<(), String> {
           Ok(())
       }
   }
   ```

3. Add your plugin to the `register_plugins!` macro invocation in
   `crates/plugins/infrastructure/registry/src/registry.rs`. The macro automatically generates
   `PluginRegistry::validate_package_identifier()` by calling
   `<YourConfig>::validate_identifier(value)` for each registered plugin type — no manual match
   arm is required.

4. Add unit tests in your plugin crate covering valid identifiers, empty identifiers, and all
   constraint violations.

Do **not** add plugin-specific identifier validation logic to the web API layer or query helpers. All
identifier validation must go through `PluginRegistry::validate_package_identifier`.

> **Implementation note:** `validate_package_identifier` is generated by the `register_plugins!`
> macro. Adding a plugin to the macro and implementing `Config::validate_identifier` is sufficient;
> the registry dispatch is updated automatically.

### Version string validation

Plugins that interpolate a `to_version` parameter into install commands must validate the version
string before command construction. This provides defense in depth even though `CommandSpec::exec()`
mode prevents shell injection — package managers have their own argument parsing that can be
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

See also: [Security — Input Validation](../security/secure-development.md#plugin-input-validation).

### Secret masking with the `SecretMasking` trait

The `SecretMasking` trait (`crates/plugins/infrastructure/core/src/secrets.rs`, re-exported from
`uptrakit-plugin-infrastructure-core`) provides a standard interface for masking and restoring secrets in plugin
configurations. It has two methods with default no-op implementations:

```rust
pub trait SecretMasking: Serialize + DeserializeOwned {
    /// Return a copy with secret fields replaced by `"***"`.
    fn with_secrets_masked(self) -> Self { self }

    /// Restore secret fields from an existing config where `self` contains `"***"` sentinels.
    fn restore_secrets_from(&mut self, _existing: &Self) {}
}
```

Plugins with no secrets (Homebrew, Proxmox Helper Scripts) use the default no-op implementations.
Plugins with secrets (GitHub, Docker) override both methods with field-level masking logic.

The registry uses generic helpers `mask_secrets_for::<T>()` and `restore_secrets_for::<T>()` that
deserialize the JSON config, apply the trait methods, and re-serialize. This eliminates duplicated
deserialize-method-serialize boilerplate per plugin.

When adding a new plugin with secrets, implement `SecretMasking` on your config struct. The
`register_plugins!` macro handles the dispatch automatically.

All plugin `new()` constructors must return `Result<Self, Report<PluginError>>` so the registry can
handle instantiation failures uniformly. The constructor should validate its configuration before
returning.

### Bidirectional error conversion

Every plugin crate defines its own error enum (e.g., `DockerError`, `GitHubError`) and implements
**bidirectional** `impl_report_conversion!` between the plugin-specific error and the shared
`PluginError`:

```rust
use uptrakit_shared_macros::impl_report_conversion;

// Plugin-specific → shared (for the registry to propagate errors)
impl_report_conversion!(DockerError => PluginError, |e| PluginError::PluginInternal(e.to_string()));

// Shared → plugin-specific (for plugins calling shared code that returns PluginError)
impl_report_conversion!(PluginError => DockerError, |e| DockerError::Configuration(e.to_string()));
```

This bidirectional pattern allows:

- The plugin registry to convert plugin errors into `PluginError` when dispatching.
- Plugin implementations to call shared code (e.g., from `uptrakit-plugin-infrastructure-core`) and convert
  `PluginError` back into their local error type.

When adding a new plugin, always implement both directions.

The agent crate imports `uptrakit-command` for shell execution and `uptrakit-plugin-infrastructure-registry` for
plugin dispatch — it does not depend on `uptrakit-plugin-infrastructure-core` directly. The web-api crate imports
`uptrakit-plugin-infrastructure-registry` (not `uptrakit-plugin-infrastructure-core`). The wire protocol crate
(`uptrakit-internal-wire`) imports `PluginType`, `ReleaseAsset`, and `ReleaseInfo` directly from
`uptrakit-shared-types`, keeping it free of plugin-implementation dependencies.

The update step can always be overridden by a custom shell script, regardless of plugin.

## Software Discovery

The `Plugin` trait includes an optional `discover_software()` method that allows plugins to enumerate
software they can manage on the local system. Plugins that support this capability declare
`PluginCapability::DiscoverLocalSoftware` in their `capabilities()` method. The method returns a
`Vec<DiscoveredSoftware>`, where each entry contains:

| Field | Type | Description |
| :--- | :--- | :--- |
| `package_identifier` | `String` | Plugin-specific identifier (maps to `SoftwareItem.package_identifier`). |
| `name` | `String` | Human-readable display name. |
| `installed_version` | `String` | Currently installed version (required; plugins omit items with unknown versions). |
| `targets` | `Vec<DiscoveryTarget>` | Structured targets for plugin config creation. Empty = use discovering plugin's config. |
| `extra` | `Option<serde_json::Value>` | Informational metadata only (e.g. Docker container names). Not used for config synthesis. |

The default implementation returns an empty list. Plugins that support discovery (e.g.,
Proxmox Helper-Scripts) override this method to scan the local system.

### Emitting `DiscoveryTarget` values

When your plugin discovers software that should be tracked by a **different** plugin type (cross-plugin
discovery), or when running without a pre-existing plugin config, emit `DiscoveryTarget` values in the
`targets` field of each `DiscoveredSoftware` item:

```rust
use uptrakit_plugin_infrastructure_core::{DiscoveredSoftware, DiscoveryTarget, PluginRole, PluginType};

DiscoveredSoftware {
    package_identifier: "booklore".to_string(),
    name: "BookLore".to_string(),
    installed_version: "1.18.5".to_string(),
    targets: vec![DiscoveryTarget {
        plugin_type: PluginType::ReleasesGithub,
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
        config_override: None,
        execution_site: None,
    }],
    extra: None,
}
```

**When to emit targets:**

- Your plugin discovers software that should be managed by a different plugin type (e.g. PHS
  discovers GitHub-managed apps).
- Your plugin is running in discover-all mode without a pre-existing config and needs the controller
  to auto-create named configs (e.g. Homebrew emitting `"Homebrew (Formulae)"` / `"Homebrew (Casks)"`).

**When to leave targets empty:**

- Your plugin is running with an existing `plugin_config_id` and all discovered items should use
  that config for all roles. The controller will use the config-ID-based path.

The controller processes targets generically: for each target, it finds or creates a plugin config
matching `(plugin_type, plugin_config)` and creates role assignments per `target.roles`. No
plugin-specific synthesis logic exists in the controller.

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

    #[tokio::test]
    async fn detect_host_compatibility_when_tool_present() {
        let plugin = AptPlugin::new(AptConfig::default(), FixedExitCodeExecutor::with_exit_code(0)).unwrap();
        let result = plugin.detect_host_compatibility().await.unwrap();
        assert_eq!(result, HostCompatibility::Compatible);
    }

    #[tokio::test]
    async fn detect_host_compatibility_when_tool_absent() {
        let plugin = AptPlugin::new(AptConfig::default(), FixedExitCodeExecutor::with_exit_code(1)).unwrap();
        let result = plugin.detect_host_compatibility().await.unwrap();
        assert!(matches!(result, HostCompatibility::Incompatible(_)));
    }
}
```

See also:

- [Plugin System Architecture](plugin-system.md) — how plugins relate to software items and host assignments.
- [Command Executor](command-executor.md) — full `CommandExecutor` trait reference.
- [Sudoers Management](../security/sudoers-management.md) — security model for privileged commands.

## GitHub Releases plugin (`uptrakit-plugin-releases-github`)

Fetches release metadata from the GitHub API and converts it into `UpstreamRelease` values.

**Config fields (`GitHubConfig`):**

| Field | Type | Required | Default | Description |
| :--- | :--- | :--- | :--- | :--- |
| `owner` | String | Yes | — | GitHub repository owner. |
| `repo` | String | Yes | — | GitHub repository name. |
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

**Config (`ProxmoxHelperScriptsConfig`):** No fields — the config is always `{}`.

**Capabilities:** `DiscoverLocalSoftware` only.

**Discovery targets emitted:**

- GitHub-managed apps: `DiscoveryTarget { plugin_type: ReleasesGithub, ... }` with owner, repo,
  `detect_installed_version_command`, and `install_command` pre-configured. Constants
  `PHS_DETECT_VERSION_CMD` and `PHS_INSTALL_CMD` are defined in
  `crates/plugins/discovery/proxmox-helper-scripts/src/discovery.rs`.
- APT-managed apps: `DiscoveryTarget { plugin_type: PackageManagerApt, config: {}, name: "APT (auto)" }`.

Cross-reference: [PHS end-user guide](../end-user/autodiscovery.md#proxmox-helper-scripts-discovery),
[PHS API notes](../api/autodiscovery.md#plugin-driven-discovery-targets).
