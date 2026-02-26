# Plugin Development Guidelines

Plugins are first-party extension modules that detect, report, and update software on managed hosts.
Each plugin crate implements the `Plugin` trait and is registered in `uptrakit-plugin-registry`. This
document describes the full lifecycle and conventions for building and extending plugins.

When adding or changing a plugin, document the full lifecycle:

- How the agent detects the installed version.
- How the controller resolves the latest upstream version.
- Version comparison rules (semver, tag prefixes, build metadata handling).
- Update execution steps, required privileges, and failure modes.
- Required configuration fields with examples.
- Any assumptions about the agent environment or custom scripts.

Plugins should keep parsing and comparison logic in pure functions so they are easy to test.

The plugin registry crate (`uptrakit-plugin-registry`) centralizes config validation, mask/restore
workflows, and creates plugin instances based on `PluginType`. Document plugin behavior so the registry
can continue to validate configs and mask secrets correctly.

`PluginType` implements `FromStr`, `Display`, and `as_str()` for string conversion. Use
`s.parse::<PluginType>()` to convert strings (returns `ParsePluginTypeError` on failure). The string
representations are: `github_releases`, `proxmox_helper_scripts`, `docker`, `homebrew`, `apt`.

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
    Incompatible { reason: String },
}
```

This allows the controller to skip discovery or version checks for plugins that are not applicable
to a given host, and to surface compatibility status in the UI.

### Pattern examples

**APT plugin** — checks whether `apt-get` is available:

```rust
fn detect_host_compatibility(&self, executor: &dyn CommandExecutor) -> HostCompatibility {
    match executor.run(CommandSpec::new("which").arg("apt-get")) {
        Ok(output) if output.exit_code == 0 => HostCompatibility::Compatible,
        _ => HostCompatibility::Incompatible {
            reason: "apt-get not found on this host".to_string(),
        },
    }
}
```

**Homebrew plugin** — checks whether `brew` is available:

```rust
fn detect_host_compatibility(&self, executor: &dyn CommandExecutor) -> HostCompatibility {
    match executor.run(CommandSpec::new("which").arg("brew")) {
        Ok(output) if output.exit_code == 0 => HostCompatibility::Compatible,
        _ => HostCompatibility::Incompatible {
            reason: "brew not found on this host".to_string(),
        },
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

**Current plugins with this capability:** `GitHubPlugin`, `DockerPlugin`.

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

Plugin crates should avoid unnecessary direct dependencies. The `uptrakit-plugin-core` crate
re-exports commonly needed types:

- **`uptrakit_plugin_core::mpsc`** — re-export of `tokio::sync::mpsc`. Use this instead of
  depending on tokio directly. Tokio should only be in `[dev-dependencies]` (for `#[tokio::test]`).
- **`uptrakit_plugin_core::CommandExecutor`**, **`CommandSpec`**, etc. — re-exports from
  `uptrakit-command`.
- **`uptrakit_plugin_core::SecretString`** — re-export from `uptrakit-shared-types`.

See [Dependency Policy](dependency-policy.md) for the full re-export strategy.

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
use uptrakit_plugin_core::SudoCommandEntry;

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

The `Plugin` trait (`crates/plugins/core/src/traits.rs`) defines the contract for all plugin
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
| `uptrakit-plugin-core` | `crates/plugins/core/` | Plugin trait/abstractions; re-exports shared types and executor types. |
| `uptrakit-plugin-registry` | `crates/plugins/registry/` | Centralized plugin dispatch and validation; re-exports `PluginType`. |
| `uptrakit-plugin-docker` | `crates/plugins/docker/` | Docker/OCI image tracking and container discovery. |
| `uptrakit-plugin-github` | `crates/plugins/github/` | GitHub Releases: fetches metadata; agent installs. |
| `uptrakit-plugin-homebrew` | `crates/plugins/homebrew/` | Homebrew: agent-side version tracking and updates. Implements `DetectHostCompatibility` (checks `which brew`). |
| `uptrakit-plugin-proxmox-helper-scripts` | `crates/plugins/proxmox-helper-scripts/` | Proxmox VE: auto-discovers and manages helper scripts. |
| `uptrakit-plugin-apt` | `crates/plugins/apt/` | APT: Debian/Ubuntu package management. Implements `DetectHostCompatibility` (checks `which apt-get`) and `PostUpdateHook` (checks `/var/run/reboot-required`). |

## Adding a New Plugin

Checklist for adding a new first-party plugin:

1. **Create crate** — add a new crate under `crates/plugins/` (e.g. `crates/plugins/my-plugin/`).
2. **Implement `Plugin` trait** — implement `plugin_type()`, `capabilities()`, and all relevant
   optional methods.
3. **Declare capabilities** — include only the `PluginCapability` variants your plugin actually
   supports. Avoid declaring capabilities the plugin does not implement.
4. **Register in `PluginRegistry`** — add a single entry to the `register_plugins!` macro invocation
   in `crates/plugins/registry/src/registry.rs`. The macro generates all dispatch methods
   automatically.
5. **Add tests** — cover success and failure paths for all implemented methods.

### The `register_plugins!` macro

The **Plugin Registry** crate centralizes all plugin operations using a `register_plugins!` macro
that generates all dispatch methods from a single declaration:

```rust
register_plugins! {
    GithubReleases => { config: GitHubConfig, plugin: GitHubPlugin },
    Docker => { config: DockerConfig, plugin: DockerPlugin },
    ProxmoxHelperScripts => { config: ProxmoxHelperScriptsConfig, plugin: ProxmoxHelperScriptsPlugin },
    Homebrew => { config: HomebrewConfig, plugin: HomebrewPlugin },
    Apt => { config: AptConfig, plugin: AptPlugin },
}
```

The macro generates six methods:

- `PluginRegistry::create_plugin()` — deserializes config, validates it, and instantiates the plugin.
- `PluginRegistry::validate_config()` — deserializes and validates plugin configuration JSON.
- `PluginRegistry::mask_config_secrets()` / `restore_config_secrets()` — handles secret masking for
  API responses (delegates to the `SecretMasking` trait implemented on each config struct).
- `PluginRegistry::create_plugin_for_discovery()` — same as `create_plugin` but without calling
  `validate()`, so discovery works with empty or minimal configs.
- `PluginRegistry::discovery_plugins()` — returns the list of `PluginType` variants whose plugin
  reports `PluginCapability::DiscoverLocalSoftware` in `capabilities()`. Fully auto-derived from the
  macro — no manual list needed.

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
PluginRegistry::validate_package_identifier(PluginType::Homebrew, value)?;
```

**Trait object dispatch (via `PluginOps`):**

```rust
state.plugin_ops.validate_package_identifier_str(&config.plugin_type, value)?;
```

Returns `Ok(())` for unknown plugin types (no constraints apply). Returns `Err(String)` with a
human-readable message when the identifier is invalid.

**When adding a new plugin with identifier constraints:**

1. Add a `pub fn validate_identifier(value: &str) -> Result<(), String>` to your plugin crate
   (e.g., `crates/plugins/my-plugin/src/plugin.rs`) and re-export it from `lib.rs`.
2. Add a match arm to `PluginRegistry::validate_package_identifier` in
   `crates/plugins/registry/src/registry.rs`:

   ```rust
   pub fn validate_package_identifier(
       plugin_type: PluginType,
       value: &str,
   ) -> std::result::Result<(), String> {
       match plugin_type {
           PluginType::Homebrew => uptrakit_plugin_homebrew::validate_identifier(value),
           PluginType::MyPlugin => uptrakit_plugin_my_plugin::validate_identifier(value),
           _ => Ok(()),
       }
   }
   ```

3. Add unit tests in your plugin crate covering valid identifiers, empty identifiers, and all
   constraint violations.

Do **not** add plugin-specific identifier validation logic to the web API layer or query helpers. All
identifier validation must go through `PluginRegistry::validate_package_identifier`.

### Secret masking with the `SecretMasking` trait

The `SecretMasking` trait (`crates/plugins/core/src/secrets.rs`, re-exported from
`uptrakit-plugin-core`) provides a standard interface for masking and restoring secrets in plugin
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
- Plugin implementations to call shared code (e.g., from `uptrakit-plugin-core`) and convert
  `PluginError` back into their local error type.

When adding a new plugin, always implement both directions.

The agent crate imports `uptrakit-command` for shell execution and `uptrakit-plugin-registry` for
plugin dispatch — it does not depend on `uptrakit-plugin-core` directly. The web-api crate imports
`uptrakit-plugin-registry` (not `uptrakit-plugin-core`). The wire protocol crate
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
use uptrakit_plugin_core::{DiscoveredSoftware, DiscoveryTarget, PluginRole, PluginType};

DiscoveredSoftware {
    package_identifier: "booklore".to_string(),
    name: "BookLore".to_string(),
    installed_version: "1.18.5".to_string(),
    targets: vec![DiscoveryTarget {
        plugin_type: PluginType::GithubReleases,
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
    use uptrakit_command::{CommandExecutor, CommandOutput, CommandSpec};

    struct FixedExitCodeExecutor {
        exit_code: i32,
        stdout: String,
    }

    impl CommandExecutor for FixedExitCodeExecutor {
        fn run(&self, _spec: &CommandSpec) -> Result<CommandOutput, CommandError> {
            Ok(CommandOutput {
                exit_code: self.exit_code,
                stdout: self.stdout.clone(),
                stderr: String::new(),
            })
        }
    }

    #[test]
    fn detect_host_compatibility_when_tool_present() {
        let executor = FixedExitCodeExecutor { exit_code: 0, stdout: "/usr/bin/apt-get".into() };
        let plugin = AptPlugin::new(AptConfig::default(), Arc::new(executor)).unwrap();
        assert!(matches!(plugin.detect_host_compatibility(), HostCompatibility::Compatible));
    }

    #[test]
    fn detect_host_compatibility_when_tool_absent() {
        let executor = FixedExitCodeExecutor { exit_code: 1, stdout: String::new() };
        let plugin = AptPlugin::new(AptConfig::default(), Arc::new(executor)).unwrap();
        assert!(matches!(
            plugin.detect_host_compatibility(),
            HostCompatibility::Incompatible { .. }
        ));
    }
}
```

See also:

- [Plugin System Architecture](plugin-system.md) — how plugins relate to software items and host assignments.
- [Command Executor](command-executor.md) — full `CommandExecutor` trait reference.
- [Sudoers Management](../security/sudoers-management.md) — security model for privileged commands.

## GitHub Releases plugin (`uptrakit-plugin-github`)

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

## Docker plugin (`uptrakit-plugin-docker`)

Tracks container image tags from OCI/Docker registries. Supports Docker Hub, GHCR, and any OCI
Distribution Spec-compliant registry. Supports both controller-side upstream version resolution and
agent-side container discovery.

See [Docker Plugin](../end-user/plugins/docker.md) for the full end-user reference.

## Proxmox Helper Scripts plugin (`uptrakit-plugin-proxmox-helper-scripts`)

Discovery-only plugin for software installed via [Proxmox VE community helper scripts](https://github.com/community-scripts/ProxmoxVE).
Does **not** perform version detection, upstream release fetching, or update execution. Its sole
responsibility is to discover which PHS-managed apps are present in a container and emit
`DiscoveryTarget` values that tell the controller which downstream plugin configs to create.

**Config (`ProxmoxHelperScriptsConfig`):** No fields — the config is always `{}`.

**Capabilities:** `DiscoverLocalSoftware` only.

**Discovery targets emitted:**

- GitHub-managed apps: `DiscoveryTarget { plugin_type: GithubReleases, ... }` with owner, repo,
  `detect_installed_version_command`, and `install_command` pre-configured. Constants
  `PHS_DETECT_VERSION_CMD` and `PHS_INSTALL_CMD` are defined in
  `crates/plugins/proxmox-helper-scripts/src/discovery.rs`.
- APT-managed apps: `DiscoveryTarget { plugin_type: Apt, config: {}, name: "APT (auto)" }`.

Cross-reference: [PHS end-user guide](../end-user/autodiscovery.md#proxmox-helper-scripts-discovery),
[PHS API notes](../api/autodiscovery.md#plugin-driven-discovery-targets).
