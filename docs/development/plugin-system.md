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
assignment links a software item to a specific host and carries:

- `plugin_config_id` — which plugin config to use for this host.
- `package_identifier` — the package name or image reference within that plugin.
- `config_override` — optional per-host JSON override merged on top of the base plugin config.

A **plugin config** (`plugin_configs` table) stores the serialized configuration for a specific
plugin type (e.g. a GitHub Releases config with `owner` and `repo`, or a Homebrew config with
`package_type`). Multiple host assignments can share the same plugin config.

When Uptrakit needs to check or update a software item on a host, it:

1. Loads the host assignment to find the `plugin_config_id` and `package_identifier`.
2. Loads the plugin config and merges any `config_override`.
3. Creates a plugin instance via `PluginRegistry::create_plugin()` with the merged config.
4. Runs the relevant plugin method (`detect_installed_version`, `execute_update`, etc.) via the
   injected `CommandExecutor` (local for the regular agent, SSH for `agent-ssh`).

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
agent uses a default/empty config and annotates results with `extra` metadata. The controller then
auto-creates named plugin configs (`"Docker"`, `"Homebrew (Formulae)"`, `"APT"`, etc.).

The agent calls `PluginRegistry::create_plugin_for_discovery()` for each entry (which skips
`validate()` to allow empty configs), then calls `discover_software()` on each plugin instance.

Discovery results are sent back in a `discovery_results` message. The controller processes these in
`process_discovery_results()` and creates pending software items for any newly discovered packages.

## The `register_plugins!` Macro

The plugin registry uses a macro to generate all dispatch methods from a single declaration:

```rust
register_plugins! {
    GithubReleases => { config: GitHubConfig, plugin: GitHubPlugin },
    Docker => { config: DockerConfig, plugin: DockerPlugin },
    ProxmoxHelperScripts => { config: ProxmoxHelperScriptsConfig, plugin: ProxmoxHelperScriptsPlugin },
    Homebrew => { config: HomebrewConfig, plugin: HomebrewPlugin },
    Apt => { config: AptConfig, plugin: AptPlugin },
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

Each capability maps to an optional method on the `Plugin` trait. Plugins that do not implement a
method should not declare the corresponding capability, and vice versa.

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

| Plugin type | Crate | Host compat | Pre-hook | Post-hook | Discovery |
| :--- | :--- | :---: | :---: | :---: | :---: |
| `github_releases` | `uptrakit-plugin-github` | No | No | No | No |
| `docker` | `uptrakit-plugin-docker` | No | No | No | Yes |
| `homebrew` | `uptrakit-plugin-homebrew` | Yes | No | No | Yes |
| `proxmox_helper_scripts` | `uptrakit-plugin-proxmox-helper-scripts` | No | No | No | Yes |
| `apt` | `uptrakit-plugin-apt` | Yes | No | Yes | Yes |

## Future Roadmap

- **Compatibility detection results surfaced in UI** — display per-host plugin compatibility in the
  Hosts and Software dashboards.
- **RebootRequired event system** — post-update events (e.g. APT `PostUpdateHook` detecting
  `/var/run/reboot-required`) surfaced as controller-side notifications or Home Assistant entities.
- **"Run arbitrary commands" plugin type** — a plugin that executes user-provided shell scripts for
  both version detection and updates, enabling one-off integrations without writing a Rust crate.
- **Formal multi-plugin-config-synthesis protocol** — a documented, stable contract for how
  discovery-only plugins (like PHS) emit `extra` metadata that the controller uses to synthesize
  downstream plugin configs.
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
