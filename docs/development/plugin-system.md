# Plugin System Architecture

This document describes the architecture of the Uptrakit plugin system: how plugins relate to
software items and host assignments, the plugin discovery flow, how new capabilities are added, and
the relationship between first-party plugin crates.

## Overview

Plugins are first-party extension modules that define how Uptrakit detects, tracks, and updates
software on managed hosts. The plugin system replaced the earlier "provider" system and introduces
a richer capability model including host compatibility detection and per-plugin lifecycle hooks.

The plugin system is composed of:

- **`uptrakit-plugin-core`** (`crates/plugins/core/`) — the `Plugin` trait, `PluginCapability` enum,
  and supporting types (`HostCompatibility`, `UpdateHookContext`, `PreUpdateHookResult`,
  `SecretMasking`).
- **First-party plugin crates** (`crates/plugins/*/`) — one crate per plugin type, each implementing
  the `Plugin` trait.
- **`uptrakit-plugin-registry`** (`crates/plugins/registry/`) — centralized dispatch and validation
  using the `register_plugins!` macro.

## How Plugins Relate to Software Items and Host Assignments

Each software item in Uptrakit has one or more **host assignments** (`host_software_items`). A host
assignment links a software item to a specific host and tracks per-host state such as
`installed_version` and `latest_version`.

### Role-Based Plugin Assignments

Each host assignment has up to three **plugin assignments** (`host_software_item_plugins`), one per
**plugin role**:

| Role | String value | Responsibility |
| :--- | :--- | :--- |
| `DetectVersion` | `detect_version` | Detect the currently installed version on the agent host. |
| `FetchReleases` | `fetch_releases` | Fetch the latest available version from an upstream source. |
| `ExecuteUpdate` | `execute_update` | Execute the actual software update on the agent host. |

Each plugin assignment row carries:

- `plugin_config_id` — which plugin config to use for this role.
- `package_identifier` — the package name or image reference within that plugin.
- `config_override` — optional per-host JSON override merged on top of the base plugin config.
- `execution_site` — where the operation runs: `auto` (default), `agent`, or `controller`.
- `role` — one of the three role strings above.
- `ordinal` — ordering within the same role (currently always 0; reserved for future multi-instance
  roles such as hook chains).

This design allows **mix-and-match** plugin configurations per role. For example, a host could use
an APT plugin for `detect_version`, a GitHub plugin for `fetch_releases`, and a custom script
plugin for `execute_update` — all for the same software item.

A **plugin config** (`plugin_configs` table) stores the serialized configuration for a specific
plugin type (e.g. a GitHub Releases config with `auth_token` and `tag_strip_prefix`, or a Homebrew
config with `package_type`). Multiple plugin assignments can share the same plugin config.
The `owner/repo` identifying a GitHub repository is **not** part of the plugin config — it is the
`package_identifier` on the software item host assignment, allowing one GitHub config to serve
all tracked repositories.

### Plugin Role Enum

The `PluginRole` enum (`crates/shared/types/src/plugin_role.rs`) is `#[non_exhaustive]` and
forward-compatible over the wire:

```rust
#[non_exhaustive]
pub enum PluginRole {
    DetectVersion,
    FetchReleases,
    ExecuteUpdate,
    /// Unknown role from a newer peer — deserialized via From<String>, never fails.
    Other(String),
}
```

- **Serde deserialization is infallible**: unknown role strings (e.g. from a newer server) become
  `Other(String)` rather than a parse error, allowing older agents to survive rolling upgrades.
- **`FromStr` is fallible**: used in API validation and URL parameter parsing where unknown roles
  should be rejected.

### Plugin Instance Creation Flow

When Uptrakit needs to check or update a software item on a host, it:

1. Loads the host assignment and its plugin assignments for the relevant role(s).
2. For each role, loads the plugin config and merges any `config_override`.
3. Creates a plugin instance via `PluginRegistry::create_plugin()` with the merged config.
4. Runs the relevant plugin method (`detect_installed_version`, `fetch_releases`,
   `execute_update`, etc.) via the injected `CommandExecutor` (local for the regular agent,
   SSH for `agent-ssh`).

### Per-Host Latest Version Tracking

The `available_versions` table has been removed. Latest version information is now tracked
per-host on `host_software_items.latest_version` (with `latest_version_fetched_at` and
`latest_release_metadata`). This reflects the reality that different hosts may see different
latest versions depending on their `fetch_releases` plugin and execution site.

## Plugin Discovery Flow

When a host registers (or discovery is manually triggered), the controller sends a
`discover_software` wire message to the agent:

```json
{
  "type": "discover_software",
  "host_machine_id": "...",
  "plugins": [
    { "plugin_config_id": "...", "plugin_type": "homebrew", "config": {...} },
    { "plugin_config_id": null, "plugin_type": "apt", "config": {} }
  ]
}
```

`plugin_config_id` is `null` for auto-discovery runs where no pre-existing plugin config exists. The
agent uses a default/empty config and plugins emit `DiscoveryTarget` values inside each
`DiscoveredSoftware` item's `targets` array. The controller creates the appropriate `PluginConfig`
records from these structured targets.

The agent calls `PluginRegistry::create_plugin_for_discovery()` for each entry (which skips
`validate()` to allow empty configs), then calls `discover_software()` on each plugin instance.

Discovery results are sent back in a `discovery_results` message. The controller processes these in
`process_discovery_results()` using a generic, target-based flow:

1. **Target-based path** (non-empty `targets`): For each `DiscoveryTarget`, find-or-create the
   plugin config matching `(plugin_type, plugin_config)`, then create role assignments per
   `target.roles`.
2. **Config-ID path** (empty `targets`, `plugin_config_id` set): Use the discovering plugin's own
   config for all three roles.

This eliminates all plugin-specific synthesis logic from the web-API layer. Plugins are responsible
for emitting the correct `DiscoveryTarget` values that describe how their discovered items should be
tracked.

## The `register_plugins!` Macro

The plugin registry uses a macro to generate all dispatch methods from a single declaration:

```rust
register_plugins! {
    GithubReleases       => { config: GitHubConfig,                   plugin: GitHubPlugin },
    Docker               => { config: DockerConfig,                   plugin: DockerPlugin },
    ProxmoxHelperScripts => { config: ProxmoxHelperScriptsConfig,     plugin: ProxmoxHelperScriptsPlugin },
    Homebrew             => { config: HomebrewConfig,                 plugin: HomebrewPlugin },
    Apt                  => { config: AptConfig,                      plugin: AptPlugin },
    Shell                => { config: ShellConfig,                    plugin: ShellPlugin },
}
```

The macro generates:

- `create_plugin()` — deserializes config, validates, instantiates.
- `validate_config()` — deserializes and validates only.
- `mask_config_secrets()` / `restore_config_secrets()` — secret masking for API responses.
- `create_plugin_for_discovery()` — instantiates without validation (allows empty configs).
- `discovery_plugin_types()` — automatically derives which plugin types support discovery by
  checking `PluginCapability::DiscoverLocalSoftware` in each plugin's `capabilities()` method.

To add a new plugin, add one line to this macro. All dispatch is generated automatically.

## Plugin Capabilities

The `PluginCapability` enum defines the optional behaviors a plugin may support:

| Capability | Description |
| :--- | :--- |
| `DiscoverLocalSoftware` | Plugin can enumerate locally installed software via `discover_software()`. |
| `RefreshPackageIndex` | Plugin can refresh its local package index before version checks (e.g. `apt update`). |
| `DetectHostCompatibility` | Plugin can determine if it is applicable to the current host via `detect_host_compatibility()`. |
| `PreUpdateHook` | Plugin can run logic before an update via `pre_update_hook()`; can abort the update. |
| `PostUpdateHook` | Plugin can run logic after an update via `post_update_hook()`; non-fatal. |
| `ControllerSideFetchReleases` | Plugin's `fetch_releases()` does not require local system state and can run on the controller instead of the agent. See [Execution Site Decision Logic](#execution-site-decision-logic). |

Each capability maps to an optional method on the `Plugin` trait. Plugins that do not implement a
method should not declare the corresponding capability, and vice versa.

### `ControllerSideFetchReleases` Capability

Plugins that declare `ControllerSideFetchReleases` signal that their `fetch_releases()`
implementation does not require any local system state — no package index, no filesystem access,
no local commands. This means the controller can call `fetch_releases()` directly rather than
delegating to an agent.

Current plugins with this capability:

| Plugin | Reason |
| :--- | :--- |
| `GitHubPlugin` | Fetches releases via the GitHub REST API — pure HTTP calls. |
| `DockerPlugin` | Queries OCI registry tag lists via HTTP — no local Docker daemon needed. |

Plugins **without** this capability (e.g. `HomebrewPlugin`, `AptPlugin`) require a local package
index and must always run `fetch_releases()` on the agent.

### Execution Site Decision Logic

The `execution_site` field on each plugin assignment controls where the operation runs. The three
values are:

| Value | Behaviour |
| :--- | :--- |
| `auto` | **Default.** The system decides based on plugin capabilities. For the `fetch_releases` role: if the plugin declares `ControllerSideFetchReleases`, the controller runs `fetch_releases()` once per unique `(plugin_config_id, package_identifier)` and propagates the result to all hosts sharing that combination. Otherwise, the agent runs it. For `detect_version` and `execute_update` roles, the agent always runs them. |
| `agent` | Force agent-side execution regardless of plugin capabilities. Useful when the controller cannot reach the upstream source (e.g. registry behind a firewall accessible only from the agent host). |
| `controller` | Force controller-side execution. Only valid for the `fetch_releases` role. The controller creates a plugin instance with a `NoopCommandExecutor` and calls `fetch_releases()` directly. |

The version check executor runs in two phases:

1. **Phase A — Controller-side `fetch_releases`:** Queries `host_software_item_plugins` rows with
   `role = 'fetch_releases'` that resolve to controller-side execution (`execution_site =
   'controller'`, or `execution_site = 'auto'` with `ControllerSideFetchReleases`). Groups by
   `(plugin_config_id, package_identifier)` to deduplicate API calls, then stores the result in
   `host_software_items.latest_version`.
2. **Phase B — Agent-side assignments:** Builds `VersionCheckAssignment` per
   `(service_id, host_machine_id)` group using `detect_version` role plugins and `fetch_releases`
   role plugins that resolve to agent-side execution. Sends `CheckVersions` wire messages as before.

### Adding a New Capability

To add a new capability to the plugin system:

1. Add a new variant to the `PluginCapability` enum in `crates/plugins/core/src/traits.rs`.
   Mark the enum `#[non_exhaustive]` (already the case).
2. Add the corresponding method to the `Plugin` trait with a default no-op implementation.
3. Define any new input/output types (e.g. `MyHookContext`, `MyHookResult`) in
   `crates/plugins/core/src/`.
4. Implement the new method in the plugin crates that should support it.
5. Update `AGENTS.md` and this document with the new capability description.

## Host Compatibility Detection

Plugins implementing `DetectHostCompatibility` allow the agent to skip discovery and version checks
for plugins that are not applicable to the current host. The method returns `HostCompatibility`:

- `Compatible` — the plugin can run on this host.
- `Incompatible { reason }` — the plugin is not applicable; include a human-readable reason.

Current implementations:

| Plugin | Check performed |
| :--- | :--- |
| `AptPlugin` | `which apt-get` — compatible if exit code 0 |
| `HomebrewPlugin` | `which brew` — compatible if exit code 0 |

The controller can use compatibility results to surface per-host plugin status in the UI (planned).

## Plugin Lifecycle Hooks

Plugin-level hooks (`PreUpdateHook`, `PostUpdateHook`) run as part of the update execution flow
managed by `agent-core`. They are distinct from user-configured JSON hooks (configured in plugin
config or `config_override` under a `hooks` key — see [Update Hooks](update-hooks.md)).

Order of operations during an update:

1. User-configured pre-update hook (JSON `hooks.pre_update`) — runs shell commands.
2. Plugin-level `pre_update_hook()` — can abort; failure aborts the update.
3. Main update execution (`execute_update()`).
4. Plugin-level `post_update_hook()` — non-fatal; errors are logged as warnings.
5. User-configured post-update hook (JSON `hooks.post_update`) — runs shell commands.

The `UpdateHookContext` passed to plugin hooks contains `package_identifier`, `to_version`, and
`from_version` (the installed version before the update, if known).

## First-Party Plugin Crates

| Plugin type | Crate | Host compat | Pre-hook | Post-hook | Discovery | Controller-side fetch |
| :--- | :--- | :---: | :---: | :---: | :---: | :---: |
| `github_releases` | `uptrakit-plugin-github` | No | No | No | No | Yes |
| `shell` | `uptrakit-plugin-shell` | No | No | No | No | No |
| `docker` | `uptrakit-plugin-docker` | No | No | No | Yes | Yes |
| `homebrew` | `uptrakit-plugin-homebrew` | Yes | No | No | Yes | No |
| `proxmox_helper_scripts` | `uptrakit-plugin-proxmox-helper-scripts` | No | No | No | Yes | No |
| `apt` | `uptrakit-plugin-apt` | Yes | No | Yes | Yes | No |

**Shell plugin** (`uptrakit-plugin-shell`): agent-side plugin with two independently-optional
shell commands. `version_command` detects the installed version (first non-empty trimmed stdout
line). `update_command` executes an update. Both commands support `{package_identifier}`,
`{version}`, and `{tag}` placeholders (shell-escaped). At least one field must be set.
The Shell plugin has **no** `ControllerSideFetchReleases` capability — all operations run
agent-side.

## Future Roadmap

- **Compatibility detection results surfaced in UI** — display per-host plugin compatibility in the
  Hosts and Software dashboards.
- **RebootRequired event system** — post-update events (e.g. APT `PostUpdateHook` detecting
  `/var/run/reboot-required`) surfaced as controller-side notifications or Home Assistant entities.
- **~~"Run arbitrary commands" plugin type~~** — completed. The `shell` plugin (`uptrakit-plugin-shell`)
  provides agent-side version detection and update execution via user-supplied shell commands,
  enabling one-off integrations without writing a Rust crate.
- **~~Formal multi-plugin-config-synthesis protocol~~** — completed. Plugins now emit structured
  `DiscoveryTarget` values via `DiscoveredSoftware.targets`. The controller processes them
  generically without plugin-specific synthesis logic.
- **Pre-update hook abort propagation** — surface abort reasons in the update history UI.

## Related Documentation

- [Plugin Guidelines](plugin-guidelines.md) — detailed plugin development conventions, patterns, and
  testing guidance.
- [Command Executor](command-executor.md) — `CommandExecutor` trait, `CommandSpec`, and
  `LocalCommandExecutor` / `SshCommandExecutor`.
- [Update Hooks](update-hooks.md) — user-configured JSON hook format (separate from plugin-level hooks).
- [Autodiscovery](../end-user/autodiscovery.md) — end-user discovery workflow and ignore rules.
- [API: Autodiscovery](../api/autodiscovery.md) — REST endpoints and PHS config synthesis.
- [Software Item Entity](../architecture/software-item-entity.md) — data model for software items,
  host assignments, and plugin configs.
