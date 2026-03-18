# Host Runtime Abstraction

The host runtime abstraction makes the execution environment a first-class concept in the plugin framework.
Plugins receive `Arc<dyn HostRuntime>` at creation time and downcast to the concrete runtime they need.
This design allows future non-POSIX host types (RouterOS, Windows) to be added without modifying the core
framework or existing plugins.

See also:

- [Plugin Guidelines](plugin-guidelines.md) -- plugin construction patterns
- [Plugin System](plugin-system.md) -- overall architecture
- [Command Executor](command-executor.md) -- the `CommandExecutor` trait used by POSIX plugins

## Design Principles

1. **Typed at boundaries, open in the middle.** OS family and features are parsed from raw strings at the
   DB/wire boundary. Inside the framework, everything is typed.
2. **Explicit probing, no inference.** Host features are reported by the agent after probing. The framework
   never infers features from the OS family.
3. **Single dispatch point.** `construct_host_runtime()` is the only place that decides which `HostRuntime`
   implementation to create. Adding a new host type means adding one match arm.
4. **Downcast at plugin construction.** The `HostRuntime` trait has no typed accessors for specific runtimes.
   Plugins downcast via `as_any()`. A mismatch produces a clear error at construction time, not at call time.

## OsFamily

**Crate:** `uptrakit-shared-types` | **File:** `crates/shared/types/src/os_family.rs`

Typed operating system family, derived from the `host.os_type` database string. This is **not** a wire type --
the wire carries `os_type: String` as before. The enum is parsed at the DB/wire boundary.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum OsFamily {
    Linux,
    MacOs,
    FreeBsd,
    RouterOs,  // groundwork -- no runtime implementation yet
    Windows,   // groundwork -- no runtime implementation yet
}
```

Parsing uses `OsFamily::from_os_type(s: &str) -> Option<Self>`. Unknown strings yield `None` at the call site.
The accepted values are lowercase: `"linux"`, `"macos"`, `"freebsd"`, `"routeros"`, `"windows"`.

All variants are `Copy`, which enables `&'static [OsFamily]` slices in `HostRequirements` role slots.

`Display` roundtrips through `from_os_type`: `OsFamily::Linux.to_string()` produces `"linux"`, and
`OsFamily::from_os_type("linux")` returns `Some(OsFamily::Linux)`.

## HostFeature

**Crate:** `uptrakit-shared-types` | **File:** `crates/shared/types/src/host_feature.rs`

Fine-grained host capability flags, reported by the agent after explicit probing. Features are **not** derived
from `OsFamily` -- the agent detects each one independently. This prevents misclassification of containers,
minimal images, and non-standard configurations.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum HostFeature {
    PosixShell,          // agent checks: `which sh`
    PrivilegeEscalation, // agent checks: `sudo -n true`
    Systemd,             // agent checks: `systemctl --version`
    RouterOsCli,         // agent checks: SSH banner or `/system/identity/print` (groundwork only)
}
```

All variants are `Copy` and implement `Ord`, which enables use in `BTreeSet` for deterministic serialization.

Serialization uses `snake_case` names: `"posix_shell"`, `"privilege_escalation"`, `"systemd"`, `"router_os_cli"`.
Unknown feature strings fail deserialization (unlike `OsFamily`, which returns `None`).

## HostCapabilities

**Crate:** `uptrakit-shared-types` | **File:** `crates/shared/types/src/host_capabilities.rs`

Runtime description of a host's execution environment. Combines the OS family, version, architecture, and
agent-reported feature flags into a single struct.

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HostCapabilities {
    pub os_family: Option<OsFamily>,
    pub os_version: Option<String>,
    pub architecture: Option<String>,
    pub features: BTreeSet<HostFeature>,
}
```

### Construction

The canonical constructor accepts raw strings from the DB or agent:

```rust
HostCapabilities::new(
    os_type: Option<&str>,         // parsed via OsFamily::from_os_type
    os_version: Option<&str>,
    architecture: Option<&str>,
    feature_strings: &[String],    // parsed via serde; unknown strings silently dropped
)
```

A convenience method handles the DB read path where features are JSON-encoded:

```rust
HostCapabilities::from_json_features(
    os_type: Option<&str>,
    os_version: Option<&str>,
    architecture: Option<&str>,
    features_json: Option<&str>,   // e.g. r#"["posix_shell","systemd"]"#
)
```

### Forward Compatibility

Unknown feature strings are silently dropped during construction. They cannot match any
`HostRequirements::required_features` entry, so they do not affect validation. This is intentional: a newer
agent can report features the controller does not know about yet without causing errors.

### Legacy Agent Handling

Legacy agents report an empty `features` set. The validation logic in `HostRequirements::is_compatible_with()`
skips the feature check entirely when `features` is empty, so existing host assignments are not rejected.

## HostRequirements

**Crate:** `uptrakit-plugin-infrastructure-core` | **File:** `crates/plugins/infrastructure/core/src/host_requirements.rs`

Per-role requirements that describe what a role needs from its target host. Lives on `RoleSlot`, not on
`PluginDescriptor` -- a single plugin can have roles with different execution requirements (for example,
a Proxmox plugin with a controller-only `ReleaseFetcher` and an agent-side `InfraBundle` requiring Linux).

```rust
pub struct HostRequirements {
    pub os_families: &'static [OsFamily],        // empty = any OS family
    pub required_features: &'static [HostFeature], // all must be present
    pub controller_only: bool,                   // when true, os/feature checks are skipped
}
```

### Named Constants

| Constant | OS Families | Required Features | Controller Only |
| --- | --- | --- | --- |
| `CONTROLLER_ONLY` | (any) | (none) | `true` |
| `NONE` | alias for `CONTROLLER_ONLY` | | |
| `POSIX` | Linux, MacOs, FreeBsd | `PosixShell` | `false` |
| `POSIX_PRIVILEGED` | Linux, MacOs, FreeBsd | `PosixShell`, `PrivilegeEscalation` | `false` |

Custom requirements use `HostRequirements::new()`, which is `const fn` and usable in `static` contexts:

```rust
const LINUX_SYSTEMD: HostRequirements = HostRequirements::new(
    &[OsFamily::Linux],
    &[HostFeature::PosixShell, HostFeature::Systemd],
    false,
);
```

### Validation

`is_compatible_with(&self, caps: &HostCapabilities) -> Result<(), Report<HostCompatibilityError>>` runs two
checks in order:

1. **OS family check** -- always applied (derived from `host.os_type`, always available). If `os_families` is
   non-empty and the host's OS family is not in the list, validation fails. Unknown OS family (`None`) also
   fails.
2. **Feature check** -- only applied when the host has reported features (non-empty `features` set). All
   entries in `required_features` must be present. Legacy agents with empty features skip this check.

Controller-only requirements (`controller_only: true`) skip both checks.

## HostRuntime Trait

**Crate:** `uptrakit-plugin-infrastructure-core` | **File:** `crates/plugins/infrastructure/core/src/host_runtime.rs`

The core trait that all runtime implementations satisfy:

```rust
pub trait HostRuntime: Send + Sync + 'static {
    fn capabilities(&self) -> &HostCapabilities;
    fn as_any(&self) -> &dyn std::any::Any;
}
```

The trait is intentionally minimal. There are no typed accessors for specific runtimes. Plugins obtain their
runtime-specific interface by downcasting via `as_any()`. This keeps the trait stable as new runtime types
are added.

## PosixHostRuntime

**Crate:** `uptrakit-plugin-infrastructure-core` | **File:** `crates/plugins/infrastructure/core/src/host_runtime.rs`

Runtime for POSIX hosts. Wraps `Arc<dyn CommandExecutor>` and `HostCapabilities`.

```rust
pub struct PosixHostRuntime {
    executor: Arc<dyn CommandExecutor>,
    capabilities: HostCapabilities,
}
```

POSIX plugins downcast to this at construction time:

```rust
let posix = runtime.as_any().downcast_ref::<PosixHostRuntime>()
    .ok_or_else(|| report!(PluginError::Configuration("requires POSIX host".into())))?;
let executor = posix.executor().clone();
```

## ControllerRuntime

**Crate:** `uptrakit-plugin-infrastructure-core` | **File:** `crates/plugins/infrastructure/core/src/descriptor.rs`

Runtime for controller-side plugins that have no physical host. Wraps `CatalogConfig`, which provides
shared resources: a pre-configured HTTP client (with SSRF protection and timeouts) and a cancellation token
for graceful shutdown. Gated on the `catalog` feature.

```rust
#[cfg(feature = "catalog")]
pub struct ControllerRuntime {
    config: CatalogConfig,
}
```

Accessor methods:

- `catalog_config() -> &CatalogConfig` -- full config access
- `http_client() -> Option<&reqwest::Client>` -- shared HTTP client
- `cancellation_token() -> Option<&CancellationToken>` -- shutdown signal

`ControllerRuntime` implements `HostRuntime` with default (empty) capabilities -- no OS family, no features.
Controller-side per-instance roles (for example, a GitHub `ReleaseFetcher`) downcast to this to access the
shared HTTP client.

## CatalogConfig

**Crate:** `uptrakit-plugin-infrastructure-core` | **File:** `crates/plugins/infrastructure/core/src/descriptor.rs`

Shared resources provided to singleton plugins during catalog construction. Always compiled (not
feature-gated), but gains additional fields when the `catalog` feature is active:

| Field | Feature Gate | Description |
| --- | --- | --- |
| `allow_private_urls` | (none) | When `true`, HTTP clients allow private/loopback URLs |
| `http_client` | `catalog` | Pre-configured `reqwest::Client` with SSRF protection |
| `cancellation_token` | `catalog` | `CancellationToken` for graceful shutdown |

## construct_host_runtime

**Crate:** `uptrakit-plugin-infrastructure-core` | **File:** `crates/plugins/infrastructure/core/src/host_runtime.rs`

Centralized dispatch function that selects the appropriate `HostRuntime` implementation based on host
capabilities:

```rust
pub fn construct_host_runtime(
    executor: Arc<dyn CommandExecutor>,
    caps: HostCapabilities,
) -> Arc<dyn HostRuntime> {
    // Currently always returns PosixHostRuntime.
    // Future: match on caps.os_family for non-POSIX hosts.
    Arc::new(PosixHostRuntime::new(executor, caps))
}
```

This is the **single point of change** when adding new host types. The function currently always returns
`PosixHostRuntime`. When RouterOS or Windows support is implemented, a match on `caps.os_family` will
dispatch to the appropriate runtime.

## require_posix_executor

**Crate:** `uptrakit-plugin-infrastructure-core` | **File:** `crates/plugins/infrastructure/core/src/host_runtime.rs`

Convenience helper for POSIX plugin constructors. Downcasts the runtime to `PosixHostRuntime` and returns the
executor, or produces a `PluginError::Configuration` error:

```rust
pub fn require_posix_executor(
    runtime: &dyn HostRuntime,
) -> Result<Arc<dyn CommandExecutor>> {
    runtime.as_any()
        .downcast_ref::<PosixHostRuntime>()
        .map(|r| Arc::clone(r.executor()))
        .ok_or_else(|| report!(PluginError::Configuration(
            "this plugin requires a POSIX host runtime".to_string()
        )))
}
```

Typical usage in a plugin's `CreateRoleFn`:

```rust
fn create_discoverer(
    config: &serde_json::Value,
    runtime: Arc<dyn HostRuntime>,
) -> Result<Box<dyn Discoverer>> {
    let executor = require_posix_executor(runtime.as_ref())?;
    let parsed: MyConfig = serde_json::from_value(config.clone())?;
    Ok(Box::new(MyDiscoverer::new(executor, parsed)))
}
```

## RoleKey

**Crate:** `uptrakit-plugin-infrastructure-core` | **File:** `crates/plugins/infrastructure/core/src/host_requirements.rs`

Typed discriminant for per-instance plugin roles. Used by `validate_role_compatibility()` and
`host_requirements_for_role()` to select the correct `RoleSlot` without string matching. Excludes singleton
roles (transport, lifecycle, infra) which are not assigned to hosts.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RoleKey {
    Discoverer,
    VersionDetector,
    ReleaseFetcher,
    PackageIndexer,
    UpdateExecutor,
    LifecycleHook,
}
```

`Display` produces lowercase snake_case names: `"discoverer"`, `"version_detector"`, `"release_fetcher"`,
`"package_indexer"`, `"update_executor"`, `"lifecycle_hook"`.

## HostCompatibilityError

**Crate:** `uptrakit-plugin-infrastructure-core` | **File:** `crates/plugins/infrastructure/core/src/host_requirements.rs`

Typed error enum for host compatibility validation failures. Propagated as
`rootcause::Report<HostCompatibilityError>` and converted to `PluginError::UnsupportedOperation` via
`impl_report_conversion!`.

| Variant | Meaning |
| --- | --- |
| `IncompatibleOsFamily { actual, expected }` | Host OS is not in the role's allowed list |
| `UnknownOsFamily` | Host has no `os_type` set (cannot validate) |
| `MissingFeature(HostFeature)` | Host lacks a required feature |
| `UnsupportedRole { plugin_type, role }` | Plugin does not implement the requested role |

## How to Add a New Host Type

Adding support for a new host type (for example, RouterOS) requires changes in three layers:

### 1. Shared Types

If the OS family or features do not already exist, add variants:

- **`OsFamily`** -- add the variant (e.g., `RouterOs` already exists as groundwork) and update
  `from_os_type()` and `Display`.
- **`HostFeature`** -- add agent-probed capabilities (e.g., `RouterOsCli` already exists as groundwork).

Both enums are `#[non_exhaustive]`, so adding variants is non-breaking.

### 2. Runtime Implementation

Create a new struct implementing `HostRuntime`:

```rust
pub struct RouterOsHostRuntime {
    session: Arc<dyn RouterOsSession>,  // your transport abstraction
    capabilities: HostCapabilities,
}

impl HostRuntime for RouterOsHostRuntime {
    fn capabilities(&self) -> &HostCapabilities { &self.capabilities }
    fn as_any(&self) -> &dyn std::any::Any { self }
}
```

### 3. Dispatch

Add a match arm to `construct_host_runtime()`:

```rust
pub fn construct_host_runtime(
    executor: Arc<dyn CommandExecutor>,
    caps: HostCapabilities,
) -> Arc<dyn HostRuntime> {
    match caps.os_family {
        Some(OsFamily::RouterOs) => {
            // Build RouterOS-specific runtime
            Arc::new(RouterOsHostRuntime::new(session, caps))
        }
        _ => Arc::new(PosixHostRuntime::new(executor, caps)),
    }
}
```

The function signature may need to evolve to accept transport-specific parameters. The key constraint is
that this remains the **single dispatch point** -- callers do not choose the runtime type.

### 4. Plugin Roles

Plugins targeting the new host type:

- Set `HostRequirements` on their `RoleSlot` to require the new OS family and features.
- Downcast to the new runtime in their `CreateRoleFn`.
- Use `require_posix_executor()` style helpers if a convenience function is warranted.

Existing POSIX plugins are unaffected -- their `HostRequirements` exclude the new OS family, so they are
never constructed with the new runtime.
