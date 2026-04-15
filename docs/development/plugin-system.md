# Plugin System Architecture

This document describes the architecture of the Uptrakit plugin system: how plugins are declared
and discovered, how they relate to software items and host assignments, the plugin discovery flow,
how new capabilities are added, and the relationship between first-party plugin crates.

## Overview

Plugins are first-party modules that define how Uptrakit detects, tracks, and updates software on
managed hosts. The plugin system uses a **descriptor/catalog** model built around three pillars:

1. **`PluginDescriptor`** -- a `'static` struct that every plugin exports via the `declare_plugin!`
   macro. It carries identity, config operations, role creation function pointers, and optional
   sections (extensions, surfaces, type settings, sudo commands, migrations).
2. **`PluginCatalog`** -- a runtime index built from `all_descriptors()` (a plain `Vec` of
   `&'static PluginDescriptor`). It manages singleton transports and lifecycle plugins, routes
   extension actions, aggregates plugin surface registrations, and implements the six focused
   `PluginOps` traits.
3. **Role traits** -- focused `async_trait` interfaces (`Discoverer`, `VersionDetector`,
   `ReleaseFetcher`, etc.) that plugins implement for the roles they support.

The system is composed of:

- **`uptrakit-plugin-infrastructure-core`** (`crates/plugins/infrastructure/core/`) -- the
  `PluginDescriptor` struct, `PluginMeta` trait, role traits, `HostRuntime` abstraction,
  `HostRequirements`, `PluginCapability` enum, `declare_plugin!` macro, `PluginCatalog`, and the
  six `PluginOps` traits.
- **First-party plugin crates** (`crates/plugins/*/`) -- one crate per plugin type, each exporting
  a `pub static DESCRIPTOR: PluginDescriptor` via `declare_plugin!`.
- **`uptrakit-plugin-infrastructure-registry`** (`crates/plugins/infrastructure/registry/`) -- the
  `all_descriptors()` function that assembles the authoritative list, `build_catalog()` entry
  point, and sudo command collection.

## Plugin Identity: `PluginTypeId`

Plugin types are identified by `PluginTypeId`, a newtype wrapping `Cow<'static, str>`. Well-known
constants live in the `plugin_ids` module (e.g., `plugin_ids::PACKAGE_MANAGER_APT`,
`plugin_ids::RELEASES_GITHUB`). The `PluginTypeId` replaces the old `PluginType` enum, making
plugin identity open-ended without requiring enum variant additions.

Every plugin struct implements the `PluginMeta` trait:

```rust
pub trait PluginMeta: Send + Sync + 'static {
    fn plugin_type_id(&self) -> PluginTypeId;
}
```

The `declare_plugin!` macro generates this implementation automatically using
`PluginTypeId::from_static(type_id)` (zero allocation for `&'static str` constants).

## How Plugins Relate to Software Items and Host Assignments

Each software item in Uptrakit has one or more **host assignments** (`host_software_items`). A host
assignment links a software item to a specific host and tracks per-host state such as
`installed_version` and `latest_version`.

### Role-Based Plugin Assignments

Each host assignment has **plugin assignments** (`host_software_item_plugins`), one per
**plugin role**:

| Role | String value | Responsibility |
| :--- | :--- | :--- |
| `DetectVersion` | `detect_version` | Detect the currently installed version on the agent host. |
| `FetchReleases` | `fetch_releases` | Fetch the latest available version from an upstream source. |
| `ExecuteUpdate` | `execute_update` | Execute the actual software update on the agent host. |
| `PreUpdateHook` | `pre_update_hook` | Run logic before an update; can abort. Multiple allowed, ordered by `ordinal`. |
| `PostUpdateHook` | `post_update_hook` | Run logic after an update; non-fatal. Multiple allowed, ordered by `ordinal`. |

Each plugin assignment row carries:

- `plugin_type` -- the plugin type string (e.g. `package_manager_apt`).
- `plugin_config_id` -- optional; which plugin config to use for this role (nullable since type
  settings may suffice).
- `package_identifier` -- the package name or image reference within that plugin.
- `config` -- optional per-host JSON override merged on top of the profile config and type settings.
- `execution_site` -- where the operation runs: `auto` (default), `agent`, or `controller`.
- `role` -- one of the role strings above.
- `ordinal` -- ordering within the same role. Used by hook roles (`pre_update_hook`,
  `post_update_hook`) to control execution order; always 0 for other roles.

This design allows **mix-and-match** plugin configurations per role. For example, a host could use
an APT plugin for `detect_version`, a GitHub plugin for `fetch_releases`, and a custom script
plugin for `execute_update` -- all for the same software item.

A **plugin config** (`plugin_configs` table) stores the serialized configuration for a specific
plugin type (e.g. a GitHub Releases config with `auth_token` and `tag_strip_prefix`, or a Homebrew
config with `package_type`). Multiple plugin assignments can share the same plugin config.
The `owner/repo` identifying a GitHub repository is **not** part of the plugin config -- it is the
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
    PreUpdateHook,
    PostUpdateHook,
    /// Unknown role from a newer peer -- deserialized via From<String>, never fails.
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
2. For each role, resolves the effective config by merging three layers: type settings (from
   `plugin_type_settings`), profile config (from `plugin_configs`), and per-assignment config
   (from `host_software_item_plugins.config`) via `resolve_effective_config()`.
3. Looks up the `PluginDescriptor` via `get_descriptor(plugin_type_str)` and accesses the
   appropriate `RoleSlot` (e.g., `desc.roles.version_detector`).
4. Calls the slot's `create` function pointer with the merged config JSON and an
   `Arc<dyn HostRuntime>`, which synchronously returns a `Box<dyn RoleTrait>`.
5. Runs the relevant method (`detect_installed_version`, `fetch_releases`,
   `execute_update`, etc.) on the returned role trait object.

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
    { "plugin_config_id": "...", "plugin_type": "package_manager_homebrew", "config": {...} },
    { "plugin_config_id": null, "plugin_type": "package_manager_apt", "config": {} }
  ]
}
```

`plugin_config_id` is `null` for auto-discovery runs where no pre-existing plugin config exists. The
agent uses a default/empty config and plugins emit `DiscoveryTarget` values inside each
`DiscoveredSoftware` item's `targets` array. The controller creates the appropriate `PluginConfig`
records from these structured targets.

The agent looks up the `PluginDescriptor` via `get_descriptor()`, accesses the `Discoverer` role
slot, and creates an instance with the config and `HostRuntime`. It then calls
`discover_software()` on the `Discoverer` trait object.

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

## The `declare_plugin!` Macro

Every plugin crate exports a `pub static DESCRIPTOR: PluginDescriptor` generated by
`declare_plugin!`. The macro replaces the old `register_plugins!` registry macro. Instead of
centralized dispatch code generation, each plugin declares itself independently and the registry
assembles them via `all_descriptors()`.

```rust
declare_plugin!(AptPlugin, AptConfig, "package_manager_apt", {
    display_name: "APT Package Manager",
    family: PluginFamily::Software,
    config_model: ConfigModel::PluginConfig,
    host_requirements: HostRequirements::POSIX,
    roles: [
        Discoverer,
        VersionDetector,
        ReleaseFetcher,
        PackageIndexer { host_requirements: HostRequirements::POSIX_PRIVILEGED },
        UpdateExecutor { host_requirements: HostRequirements::POSIX_PRIVILEGED },
    ],
    sudo: apt_sudo_commands,
});
```

The macro generates:

- **`impl PluginMeta`** -- returns `PluginTypeId::from_static(type_id)`.
- **Compile-time trait assertions** -- verifies the plugin struct implements all declared role
  traits (e.g., `Discoverer`, `VersionDetector`). A missing impl is a compile error.
- **Config delegation functions** -- `validate`, `mask_secrets`, `restore_secrets`, `sample`,
  `form_schema`, `validate_identifier` -- all delegating to the `PluginConfig` trait on the
  config type.
- **Per-role creation functions** -- `create_discoverer`, `create_version_detector`, etc. Each
  deserializes the config JSON and calls `Plugin::new(config, runtime)`.
- **`pub static DESCRIPTOR: PluginDescriptor`** -- the assembled descriptor with all fields
  populated from the macro arguments.

The `all_descriptors()` function in the registry crate returns the authoritative list:

```rust
pub fn all_descriptors() -> Vec<&'static PluginDescriptor> {
    let mut descs = vec![
        &uptrakit_plugin_releases_github::DESCRIPTOR,
        &uptrakit_plugin_package_manager_apt::DESCRIPTOR,
        // ... all compiled-in plugins
    ];
    #[cfg(feature = "notifications-webhook")]
    descs.push(&uptrakit_notification_plugin_webhook::DESCRIPTOR);
    descs
}
```

To add a new plugin: create a crate, use `declare_plugin!`, and add one line to
`all_descriptors()`.

### Test-Only Feature-Gated Descriptors

For deterministic controller-side failure-path integration tests, the registry includes a
`test-support` feature gate that adds two test-only descriptors to `all_descriptors()`:
`__test_fetch_fail` and `__test_per_item_fail`.

- Their corresponding `PluginTypeId` constants (`plugin_ids::TEST_FETCH_FAIL`,
  `plugin_ids::TEST_PER_ITEM_FAIL`) are defined behind `#[cfg(feature = "test-support")]`.
- They are intentionally excluded from `plugin_ids::ALL` so production/always-on invariants remain
  unchanged.
- The `descriptors_subset_of_known_ids` registry test conditionally extends its known-ID set with
  these two IDs only when `test-support` is enabled.
- These descriptors are manually constructed in `registry/src/test_support.rs` (not generated via
  `declare_plugin!`) because they are test infrastructure, not first-party plugin crates.

Decision rationale: [ADR-0022](../internal/changes/TASK-0012/ADR-0022.md).

## Plugin Families

The `PluginFamily` enum categorizes plugins for UI grouping and catalog queries:

| Family | Description | Examples |
| :--- | :--- | :--- |
| `Software` | Version detection, release fetching, updates | APT, Homebrew, GitHub, Docker |
| `Hook` | Pre/post-update lifecycle hooks | Shell Hook, Systemd Hook |
| `Notification` | Notification delivery channels | Webhook, Telegram, Email |
| `Infrastructure` | Host lifecycle, guest execution | Proxmox |
| `Enhancement` | Controller-side item enrichment | Dashboard Icons |

## Plugin Capabilities

The `PluginCapability` enum defines the optional behaviors a plugin may support. Capabilities are
automatically derived from declared roles by the `declare_plugin!` macro, with optional
`extra_capabilities` for capabilities not tied to a role.

| Capability | Derived from role | Description |
| :--- | :--- | :--- |
| `DiscoverLocalSoftware` | `Discoverer` | Enumerate locally installed software. |
| `DetectHostCompatibility` | `Discoverer` | Determine if plugin is applicable to the current host. |
| `VersionDetection` | `VersionDetector` | Detect installed versions. |
| `ReleaseFetching` | `ReleaseFetcher` | Fetch upstream releases. |
| `RefreshPackageIndex` | `PackageIndexer` | Refresh local package index (e.g. `apt update`). |
| `UpdateExecution` | `UpdateExecutor` | Execute software updates. |
| `UpdateLifecycle` | `LifecycleHook` | Pre/post-update hooks. |
| `NotificationDelivery` | `NotificationTransport` | Deliver notification messages. |
| `SoftwareItemLifecycle` | `SoftwareItemLifecycle` | React to software item lifecycle events. |
| `ControllerSideFetchReleases` | (extra) | `fetch_releases()` can run on the controller. |
| `ConfigTest` | (config_test) | Plugin supports config testing. |

### `ControllerSideFetchReleases` Capability

Plugins that declare `ControllerSideFetchReleases` (via `extra_capabilities`) signal that their
`fetch_releases()` implementation does not require any local system state -- no package index, no
filesystem access, no local commands. This means the controller can call `fetch_releases()` directly
rather than delegating to an agent.

Current plugins with this capability:

| Plugin | Reason |
| :--- | :--- |
| `GitHubPlugin` | Fetches releases via the GitHub REST API -- pure HTTP calls. |
| `GitLabPlugin` | Fetches releases via the GitLab REST API -- pure HTTP calls. |
| `ForgejoPlugin` | Fetches releases via the Forgejo REST API -- pure HTTP calls. |
| `DockerPlugin` | Queries OCI registry tag lists via HTTP -- no local Docker daemon needed. |

Plugins **without** this capability (e.g. `HomebrewPlugin`, `AptPlugin`) require a local package
index and must always run `fetch_releases()` on the agent.

#### Docker `daemon` Feature Gate

The `uptrakit-plugin-releases-docker` crate has a `daemon` feature (enabled by default) that
gates the bollard Docker client and local Docker operations (`DiscoverLocalSoftware`). Controller
builds disable this feature (`default-features = false`) since the controller only needs
`fetch_releases()` (HTTP-based registry queries), not local Docker daemon access. Agent builds
enable it by default. This avoids pulling the heavy bollard + TLS dependency chain into the
controller binary.

### Execution Site Decision Logic

The `execution_site` field on each plugin assignment controls where the operation runs. The three
values are:

| Value | Behaviour |
| :--- | :--- |
| `auto` | **Default.** The system decides based on plugin capabilities. For the `fetch_releases` role: if the plugin declares `ControllerSideFetchReleases`, the controller runs `fetch_releases()` once per unique `(plugin_config_id, package_identifier)` and propagates the result to all hosts sharing that combination. Otherwise, the agent runs it. For `detect_version` and `execute_update` roles, the agent always runs them. |
| `agent` | Force agent-side execution regardless of plugin capabilities. Useful when the controller cannot reach the upstream source (e.g. registry behind a firewall accessible only from the agent host). |
| `controller` | Force controller-side execution. Only valid for the `fetch_releases` role. The controller creates a plugin instance with a `ControllerRuntime` and calls `fetch_releases()` directly. |

The version check executor runs in two phases:

1. **Phase A -- Controller-side `fetch_releases`:** Queries `host_software_item_plugins` rows with
   `role = 'fetch_releases'` that resolve to controller-side execution (`execution_site =
   'controller'`, or `execution_site = 'auto'` with `ControllerSideFetchReleases`). Groups by
   `(plugin_config_id, package_identifier)` to deduplicate API calls, then stores the result in
   `host_software_items.latest_version`.
2. **Phase B -- Agent-side assignments:** Builds `VersionCheckAssignment` per
   `(service_id, host_machine_id)` group using `detect_version` role plugins and `fetch_releases`
   role plugins that resolve to agent-side execution. Sends `CheckVersions` wire messages as before.

### Adding a New Capability

To add a new capability to the plugin system:

1. Add a new variant to the `PluginCapability` enum in
   `crates/plugins/infrastructure/core/src/types.rs`. The enum is `#[non_exhaustive]`.
2. If the capability maps to a new role trait, add the trait to
   `crates/plugins/infrastructure/core/src/roles.rs` and add the corresponding `RoleSlot` field
   to `RoleCreators`, a macro arm to `__set_role_field!` and `__define_role_creator!`, and a
   `__accumulate_role_caps!` arm.
3. If the capability is independent of a role, use `extra_capabilities` in `declare_plugin!`.
4. Implement the new trait or capability in the plugin crates that should support it.
5. Update this document with the new capability description.

## Host Compatibility Detection

Plugins implementing the `Discoverer` trait include a `detect_host_compatibility()` method
(default: `Compatible`) that allows the agent to skip discovery and version checks for plugins
that are not applicable to the current host. The method returns `HostCompatibility`:

- `Compatible` -- the plugin can run on this host.
- `Incompatible(reason)` -- the plugin is not applicable; include a human-readable reason.

Current implementations:

| Plugin | Check performed |
| :--- | :--- |
| `AptPlugin` | `which apt-get` -- compatible if exit code 0 |
| `HomebrewPlugin` | `which brew` -- compatible if exit code 0 |

The controller can use compatibility results to surface per-host plugin status in the UI (planned).

## Host Runtime Abstraction

Plugins receive `Arc<dyn HostRuntime>` at construction time instead of a direct
`Arc<dyn CommandExecutor>`. The `HostRuntime` trait provides an extensible abstraction over the
execution environment:

```rust
pub trait HostRuntime: Send + Sync + 'static {
    fn capabilities(&self) -> &HostCapabilities;
    fn as_any(&self) -> &dyn std::any::Any;
}
```

Plugins downcast to the concrete runtime they need via `as_any()`. Two implementations exist:

| Runtime | Used by | Provides |
| :--- | :--- | :--- |
| `PosixHostRuntime` | Agent-side plugins on POSIX hosts | `Arc<dyn CommandExecutor>` via `executor()` |
| `ControllerRuntime` | Controller-side `fetch_releases` | `CatalogConfig`, shared HTTP client, cancellation token |

The convenience helper `require_posix_executor(runtime)` performs the downcast and returns the
executor, or a clear error if the runtime is not POSIX. POSIX plugins use this in their `new()`
constructor:

```rust
impl AptPlugin {
    pub fn new(config: AptConfig, runtime: Arc<dyn HostRuntime>) -> Result<Self> {
        let executor = require_posix_executor(runtime.as_ref())?;
        Ok(Self { config, executor })
    }
}
```

Plugin construction is **synchronous** (`fn new(config, runtime) -> Result<Self>`). The
`construct_host_runtime()` function is the single dispatch point for runtime type selection,
currently always returning `PosixHostRuntime`. When non-POSIX host types are added (e.g.,
RouterOS), this function dispatches based on the host's `OsFamily`.

See [Host Runtime](host-runtime.md) for the full design rationale.

## Host Requirements

`HostRequirements` specifies what a role needs from its target host: compatible OS families,
required host features, and whether the role is controller-only. Requirements live **per-role**
on `RoleSlot`, not on the descriptor as a whole. This allows a single plugin to have roles with
different requirements.

Named constants cover common cases:

| Constant | OS families | Features | Controller-only |
| :--- | :--- | :--- | :---: |
| `HostRequirements::POSIX` | Linux, macOS, FreeBSD | `PosixShell` | No |
| `HostRequirements::POSIX_PRIVILEGED` | Linux, macOS, FreeBSD | `PosixShell`, `PrivilegeEscalation` | No |
| `HostRequirements::CONTROLLER_ONLY` | any | none | Yes |

Per-role overrides in `declare_plugin!`:

```rust
roles: [
    Discoverer,                                                  // uses descriptor default
    PackageIndexer { host_requirements: HostRequirements::POSIX_PRIVILEGED },  // override
    UpdateExecutor { host_requirements: HostRequirements::POSIX_PRIVILEGED },  // override
]
```

Validation is performed at assignment time by `validate_role_compatibility()` on
`PluginMetadataOps`, using the host's `HostCapabilities`.

## The `PluginOps` Trait Family

The old monolithic `PluginOps` god trait (21 methods) is split into six focused traits.
`PluginCatalog` implements all six. Most consuming code should depend on the narrowest trait
it actually needs.

| Trait | Responsibility |
| :--- | :--- |
| `PluginMetadataOps` | Descriptor lookup, known type IDs, capability queries, host requirements |
| `PluginConfigOps` (extends `PluginMetadataOps`) | Config validation, secret masking, form schemas, type settings |
| `PluginExtensionOps` | Extension manifest collection, extension action routing |
| `PluginSurfaceOps` | Plugin-owned shared-surface provider registrations |
| `NotificationOps` | Notification transport lookup, supported types |
| `SoftwareItemLifecycleOps` | Fire `on_software_item_created` across all lifecycle plugins |

A blanket `PluginOps` alias combines all six for the few places that need the full surface
(e.g., `AppState`):

```rust
pub trait PluginOps:
    PluginMetadataOps + PluginConfigOps + PluginExtensionOps + PluginSurfaceOps
    + NotificationOps + SoftwareItemLifecycleOps {}
```

`PluginSurfaceOps::surface_registrations()` is the plugin-side bridge into the shared surface
runtime. The controller bootstraps these registrations into `SurfaceRegistry` at startup, then the
normal `/api/v1/surfaces/*` and frontend shared renderer path handle UI delivery.

## Update Lifecycle Hooks

Update lifecycle hooks are standalone plugin assignments with roles `PreUpdateHook` and
`PostUpdateHook`. They implement the `LifecycleHook` trait and are executed by
`agent-core` as part of the update pipeline.

Order of operations during an update:

1. **Pre-update hook plugins** (ordered by `ordinal` ASC) -- each calls `execute_pre_hook()`.
   First failure aborts the update.
2. Attestation gate (if applicable).
3. Main update execution (`execute_update()`).
4. **Post-update hook plugins** (ordered by `ordinal` ASC) -- each calls `execute_post_hook()`
   with `update_succeeded` set. Errors are logged as warnings, non-fatal.
5. Version detection (`detect_installed_version()`).

The `UpdateLifecycleContext` passed to hook plugins contains `package_identifier`, `to_version`,
`from_version`, `release_info`, and `update_succeeded` (`None` during pre-hooks, `Some(bool)`
during post-hooks).

See [Update Lifecycle Plugins](update-hooks.md) for full details on the systemd and shell
hook plugins.

## Infrastructure Plugins via `InfraBundle`

Infrastructure plugins (e.g., Proxmox) manage host lifecycle, state reporting, and guest
execution. They use the same `declare_plugin!` macro as all other plugins, with the `infra`
section specifying an `InfraSlot`:

```rust
declare_plugin!(ProxmoxPlugin, ProxmoxConfig, "infrastructure_proxmox", {
    display_name: "Proxmox VE",
    family: PluginFamily::Infrastructure,
    config_model: ConfigModel::PluginConfig,
    roles: [],
    infra: {
        create: create_proxmox_bundle,
        host_requirements: HostRequirements::new(&[OsFamily::Linux], &[], false),
        capabilities: &[PluginCapability::InfraLifecycle],
    },
});
```

The `InfraSlot.create` function returns an `InfraBundle` -- a struct of optional narrow trait
objects:

| Trait | Responsibility |
| :--- | :--- |
| `HostLifecycle` | Bootstrap detection, sync, post-report hooks |
| `HostReport` | Check whether infra state exists for a host |
| `GuestExec` | Execute commands inside guest VMs, handle service-side extension actions |

All three traits are `#[cfg(feature = "agent-infra")]`-gated. The `InfraBundle` carries `Option`
for each, so an infrastructure plugin can implement any subset. The catalog creates all infra
bundles at startup via `create_infra_bundles()`.

## Notification Plugins

Notification plugins use the same `declare_plugin!` macro and `PluginDescriptor` struct as
software plugins. They declare a `notification_transport` creation function and the
`NotificationTransport` role:

```rust
declare_plugin!(WebhookPlugin, WebhookConfig, "webhook", {
    display_name: "Webhook",
    family: PluginFamily::Notification,
    config_model: ConfigModel::NotificationChannel,
    roles: [NotificationTransport],
    notification_transport: create_webhook_transport,
    extensions: { manifests: ..., actions: ..., handle_action: ... },
    surfaces: { registrations: ... },
});
```

Notification transports are **singletons** created at catalog construction time. The `PluginCatalog`
stores them as `Arc<dyn NotificationTransport>` and exposes them via the `NotificationOps` trait.
Each transport's `deliver()` method receives the channel-specific config JSON and settings JSON at
call time.

Feature flags gate notification plugins in the registry: `notifications-webhook` (default),
`notifications-telegram`, `notifications-email`. These propagate as feature flags through web-api
and controller crates.

## Enhancement Plugins

Enhancement plugins react to software item lifecycle events on the controller side. They use the
`SoftwareItemLifecycle` role to enrich items after creation (e.g., fetching dashboard icons).

```rust
declare_plugin!(DashboardIconsPlugin, DashboardIconsConfig, "enhancement_dashboard_icons", {
    display_name: "Dashboard Icons",
    family: PluginFamily::Enhancement,
    config_model: ConfigModel::None,
    roles: [SoftwareItemLifecycle],
    software_item_lifecycle: create_dashboard_icons,
});
```

Enhancement plugins are **singletons** created at catalog construction time. The catalog fires
`on_software_item_created` across all registered lifecycle plugins when a new software item is
created, merging any `SoftwareItemPatch` results (e.g., setting `icon_url`).

## `CatalogConfig`

`CatalogConfig` provides shared deployment-level resources to singleton plugin constructors:

| Field | Feature gate | Description |
| :--- | :--- | :--- |
| `allow_private_urls` | always | Whether HTTP clients allow private/loopback addresses |
| `http_client` | `catalog` | Pre-configured `reqwest::Client` with SSRF protection and timeouts |
| `cancellation_token` | `catalog` | `CancellationToken` for graceful shutdown of background tasks |

The struct is always compiled (not feature-gated) so that `CreateTransportFn` and
`CreateEnhancementFn` type signatures are visible in all plugin crates. Feature-gated fields
only exist when the `catalog` feature is active (controller builds).

## First-Party Plugin Crates

| Plugin type | Crate | Family | Discovery | Controller-side fetch | Host compat | Lifecycle hooks |
| :--- | :--- | :--- | :---: | :---: | :---: | :---: |
| `releases_github` | `uptrakit-plugin-releases-github` | Software | No | Yes | No | No |
| `releases_gitlab` | `uptrakit-plugin-releases-gitlab` | Software | No | Yes | No | No |
| `releases_forgejo` | `uptrakit-plugin-releases-forgejo` | Software | No | Yes | No | No |
| `releases_docker` | `uptrakit-plugin-releases-docker` | Software | Yes | Yes | No | No |
| `discovery_proxmox_helper_scripts` | `uptrakit-plugin-discovery-proxmox-helper-scripts` | Software | Yes | No | No | No |
| `package_manager_apt` | `uptrakit-plugin-package-manager-apt` | Software | Yes | No | Yes | No |
| `package_manager_homebrew` | `uptrakit-plugin-package-manager-homebrew` | Software | Yes | No | Yes | No |
| `package_manager_dnf` | `uptrakit-plugin-package-manager-dnf` | Software | Yes | No | Yes | No |
| `package_manager_npm` | `uptrakit-plugin-package-manager-npm` | Software | Yes | No | Yes | No |
| `package_manager_mas` | `uptrakit-plugin-package-manager-mas` | Software | Yes | No | Yes | No |
| `package_manager_pacman` | `uptrakit-plugin-package-manager-pacman` | Software | Yes | No | Yes | No |
| `package_manager_pkg` | `uptrakit-plugin-package-manager-pkg` | Software | Yes | No | Yes | No |
| `package_manager_apk` | `uptrakit-plugin-package-manager-apk` | Software | Yes | No | Yes | No |
| `package_manager_snap` | `uptrakit-plugin-package-manager-snap` | Software | Yes | No | Yes | No |
| `package_manager_cargo` | `uptrakit-plugin-package-manager-cargo` | Software | Yes | No | Yes | No |
| `generic_shell` | `uptrakit-plugin-generic-shell` | Software | No | No | No | No |
| `hook_systemd` | `uptrakit-plugin-hook-systemd` | Hook | No | No | No | Yes |
| `hook_shell` | `uptrakit-plugin-hook-shell` | Hook | No | No | No | Yes |
| `infrastructure_proxmox` | `uptrakit-plugin-infrastructure-proxmox` | Infrastructure | No | No | No | No |
| `webhook` | `uptrakit-notification-plugin-webhook` | Notification | No | No | No | No |
| `telegram` | `uptrakit-notification-plugin-telegram` | Notification | No | No | No | No |
| `email` | `uptrakit-notification-plugin-email` | Notification | No | No | No | No |
| `enhancement_dashboard_icons` | `uptrakit-plugin-enhancement-dashboard-icons` | Enhancement | No | No | No | No |

**Shell plugin** (`uptrakit-plugin-generic-shell`): agent-side plugin with two independently-optional
shell commands. `version_command` detects the installed version (first non-empty trimmed stdout
line). `update_command` executes an update. Both commands support `{package_identifier}`,
`{version}`, and `{tag}` placeholders (shell-escaped). At least one field must be set.
The Shell plugin has **no** `ControllerSideFetchReleases` capability -- all operations run
agent-side.

## Future Roadmap

- **Compatibility detection results surfaced in UI** -- display per-host plugin compatibility in the
  Hosts and Software dashboards.
- **RebootRequired event system** -- post-update events (e.g. detecting
  `/var/run/reboot-required` via a shell hook plugin) surfaced as controller-side notifications or
  Home Assistant entities.
- **Non-POSIX host runtimes** -- `construct_host_runtime()` dispatches on `OsFamily` to create
  runtime implementations for non-POSIX platforms (e.g., RouterOS).
- **Pre-update hook abort propagation** -- surface abort reasons in the update history UI.
- ~~**"Run arbitrary commands" plugin type**~~ -- completed. The `generic_shell` plugin provides
  agent-side version detection and update execution via user-supplied shell commands.
- ~~**Formal multi-plugin-config-synthesis protocol**~~ -- completed. Plugins emit structured
  `DiscoveryTarget` values via `DiscoveredSoftware.targets`.
- ~~**Update lifecycle hooks as standalone plugins**~~ -- completed. The `hook_systemd` and
  `hook_shell` plugins replace the old embedded hook system.
- ~~**Unified descriptor model**~~ -- completed. All plugin families (software, hook, notification,
  infrastructure, enhancement) use the same `declare_plugin!` macro and `PluginDescriptor` struct.
- ~~**Focused PluginOps traits**~~ -- completed. The monolithic `PluginOps` god trait is split into
  five focused traits.

## Related Documentation

- [Plugin Guidelines](plugin-guidelines.md) -- detailed plugin development conventions, patterns,
  and testing guidance.
- [Host Runtime](host-runtime.md) -- `HostRuntime` trait, `PosixHostRuntime`,
  `ControllerRuntime`, and runtime selection.
- [Command Executor](command-executor.md) -- `CommandExecutor` trait, `CommandSpec`, and
  `LocalCommandExecutor` / `SshCommandExecutor`.
- [Update Lifecycle Plugins](update-hooks.md) -- systemd and shell hook plugins for pre/post-update
  hooks.
- [Autodiscovery](../end-user/autodiscovery.md) -- end-user discovery workflow and ignore rules.
- [API: Autodiscovery](../api/autodiscovery.md) -- REST endpoints and PHS config synthesis.
- [Software Item Entity](../architecture/software-item-entity.md) -- data model for software items,
  host assignments, and plugin configs.
