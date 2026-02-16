# Provider Development Guidelines

When adding or changing a provider, document the full lifecycle:

- How the agent detects the installed version.
- How the controller resolves the latest upstream version.
- Version comparison rules (semver, tag prefixes, build metadata handling).
- Update execution steps, required privileges, and failure modes.
- Required configuration fields with examples.
- Any assumptions about the agent environment or custom scripts.

Providers should keep parsing and comparison logic in pure functions so they are easy to test.

The provider registry crate (`uptrakit-provider-registry`) centralizes config validation, mask/restore workflows, and creates provider instances based
on `ProviderType`. Document provider behavior so the registry can continue to validate configs and mask secrets correctly.

`ProviderType` implements `FromStr`, `Display`, and `as_str()` for string conversion. Use `s.parse::<ProviderType>()` to convert strings (returns
`ParseProviderTypeError` on failure). The string representations are: `github_releases`, `proxmox_helper_scripts`, `docker_registry`, `homebrew`.

## Command Executor Injection

Providers do not spawn processes directly. Instead, each provider receives an `Arc<dyn CommandExecutor>` at construction
time and delegates all command execution through that trait. This decouples provider logic from the execution transport,
enabling the same provider code to run commands locally (via `LocalCommandExecutor`) or remotely (e.g., over SSH in the
future).

See [Command Executor](command-executor.md) for the full trait reference, `CommandSpec` constructors, and guidance on
implementing custom executors.

## Provider Trait: Required Methods

The `Provider` trait (`crates/providers/core/src/traits.rs`) defines the contract for all provider implementations. Two methods are required (no
default implementation):

| Method | Signature | Description | | :--- | :--- | :--- | | `provider_type` | `fn provider_type(&self) -> ProviderType` | Returns the provider's
type for introspection, logging, and telemetry. | | `capabilities` | `fn capabilities(&self) -> Vec<ProviderCapability>` | Declares which optional
features the provider supports. |

All other methods (`detect_installed_version`, `fetch_releases`, `execute_update`, `discover_software`, `refresh_package_index`) have default
implementations that return errors or empty results, so providers override only what they support.

When implementing a new provider, always return the correct `ProviderType` variant from `provider_type()`. This ensures that boxed `dyn Provider`
objects can be introspected after creation by `ProviderRegistry::create_provider()`.

## Provider Architecture - Detailed

Each software item is associated with a provider. A provider defines:

| Concern | Runs on | Responsibility | | :---------------------- | :------------------ |
:-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
| | Remote/upstream version | Controller or Agent | Fetch latest version metadata. Most providers (GitHub, Docker) resolve on the controller.
Providers with a local package index (Homebrew) resolve on the agent via `RefreshPackageIndex` + `fetch_releases()` and report `latest_version` in
`VersionCheckResult`. | | Local/installed version | Agent | Detect currently installed version | | Update execution | Agent | Run the update (via
sudo-allowlisted commands or custom script) |

Provider crates:

| Crate | Path | Purpose | | :----------------------------------------- | :----------------------------------------- |
:-------------------------------------------------------------------------------------------------------------------- | | `uptrakit-shared-types` |
`crates/shared/types/` | Canonical home for `ProviderType`, `ReleaseAsset`, and `ReleaseInfo` (plus `SecretString`, hex helpers). | |
`uptrakit-command` | `crates/shared/command/` | Shell execution, `CommandExecutor` trait, `CommandSpec`, `LocalCommandExecutor`. | |
`uptrakit-provider-core` |
`crates/providers/core/` | Provider trait/abstractions; re-exports shared types and executor types. | | `uptrakit-provider-registry` |
`crates/providers/registry/` | Centralized provider dispatch and validation; re-exports `ProviderType`. | | `uptrakit-provider-docker-registry` |
`crates/providers/docker-registry/` | Docker/OCI Registry: tracks image tags. | | `uptrakit-provider-github` | `crates/providers/github/` | GitHub
Releases: fetches metadata; agent installs. | | `uptrakit-provider-homebrew` | `crates/providers/homebrew/` | Homebrew: agent-side version tracking
and updates. | | `uptrakit-provider-proxmox-helper-scripts` | `crates/providers/proxmox-helper-scripts/` | Proxmox VE: auto-discovers and manages
helper scripts. |

The **Provider Registry** crate centralizes all provider operations:

- `ProviderRegistry::create_provider()` — creates `Provider` instances from `ProviderType`, config, and an `Arc<dyn CommandExecutor>`
- `ProviderRegistry::validate_config()` — validates provider configuration JSON
- `ProviderRegistry::mask_config_secrets()` / `restore_config_secrets()` — handles secret masking for API responses (delegates to the `SecretMasking`
  trait implemented on each config struct)

### Secret masking with the `SecretMasking` trait

The `SecretMasking` trait (`crates/providers/core/src/secrets.rs`, re-exported from `uptrakit-provider-core`) provides a standard interface for
masking and restoring secrets in provider configurations. It has two methods with default no-op implementations:

```rust
pub trait SecretMasking: Serialize + DeserializeOwned {
    /// Return a copy with secret fields replaced by `"***"`.
    fn with_secrets_masked(self) -> Self { self }

    /// Restore secret fields from an existing config where `self` contains `"***"` sentinels.
    fn restore_secrets_from(&mut self, _existing: &Self) {}
}
```

Providers with no secrets (Homebrew, Proxmox Helper Scripts) use the default no-op implementations. Providers with secrets (GitHub, Docker Registry)
override both methods with field-level masking logic.

The registry uses generic helpers `mask_secrets_for::<T>()` and `restore_secrets_for::<T>()` that deserialize the JSON config, apply the trait
methods, and re-serialize. This eliminates duplicated deserialize-method-serialize boilerplate per provider.

When adding a new provider with secrets, implement `SecretMasking` on your config struct. The registry match arms become one-liners:

```rust
ProviderType::MyProvider => mask_secrets_for::<MyProviderConfig>(config),
```

The agent crate imports `uptrakit-command` for shell execution and `uptrakit-provider-registry` for provider dispatch — it does not depend on
`uptrakit-provider-core` directly. The web-api crate imports `uptrakit-provider-registry` (not `uptrakit-provider-core`). The wire protocol crate
(`uptrakit-internal-wire`) imports `ProviderType`, `ReleaseAsset`, and `ReleaseInfo` directly from `uptrakit-shared-types`, keeping it free of
provider-implementation dependencies. This eliminates scattered string-based provider matching and keeps all dispatch logic in one place.

The update step can always be overridden by a custom shell script, regardless of provider.

### Provider capabilities

The `ProviderCapability` enum defines optional features a provider may support:

| Capability | Trait method | Description | | :---------------------- | :------------------------ |
:-------------------------------------------------------------- | | `DiscoverLocalSoftware` | `discover_software()` | Enumerate software the provider
can manage on the local system. | | `RefreshPackageIndex` | `refresh_package_index()` | Refresh local package index (for example, `apt update`). |

### Software discovery

The `Provider` trait includes an optional `discover_software()` method that allows providers to enumerate software they can manage on the local
system. Providers that support this capability declare `ProviderCapability::DiscoverLocalSoftware` in their `capabilities()` method. The method
returns a `Vec<DiscoveredSoftware>`, where each entry contains:

| Field | Type | Description | | :------------------- | :-------------------------- |
:------------------------------------------------------------------------ | | `package_identifier` | `String` | Provider-specific identifier (maps to
`SoftwareItem.package_identifier`). | | `name` | `String` | Human-readable display name. | | `installed_version` | `Option<Version>` | Currently
installed version, if detected. | | `extra` | `Option<serde_json::Value>` | Arbitrary provider-specific metadata (for example, install path). |

The default implementation returns an empty list. Providers that support discovery (e.g., Proxmox Helper-Scripts) override this method to scan the
local system.

### GitHub Releases provider (`uptrakit-provider-github`)

Fetches release metadata from the GitHub API and converts it into `UpstreamRelease` values.

**Config fields (`GitHubConfig`):**

| Field | Type | Required | Default | Description | | :-------------------- | :------------ | :------- | :----------------------- |
:----------------------------------------------------------- | | `owner` | String | Yes | — | GitHub repository owner. | | `repo` | String | Yes | — |
GitHub repository name. | | `auth_token` | String | No | `null` | Personal access token (private repos or higher rate limits). | | `api_base_url` |
String | No | `https://api.github.com` | API base URL (for GitHub Enterprise). | | `include_prereleases` | bool | No | `false` | Whether to include
pre-release versions. | | `tag_strip_prefix` | String | No | `"v"` | Prefix to strip from tag names to extract version strings. | | `asset_patterns` |
`Vec<String>` | No | `[]` | Regex patterns to filter release assets (empty means all). | | `install_command` | `Option<String>` | No | `null` | Custom
shell command to execute after downloading the release asset. Supports `{version}`, `{tag}`, `{asset_url}`, `{asset_name}` placeholders
(shell-escaped). |

**Behaviour:**

- Drafts are always skipped
- Rate limit headers are checked; warnings logged when remaining < 10
- 403/429 responses with `x-ratelimit-remaining: 0` return a rate-limit error
- Asset filtering uses regex matching against asset names

### Docker Registry provider (`uptrakit-provider-docker-registry`)

Tracks container image tags from OCI/Docker registries. Supports Docker Hub, GHCR, and any OCI Distribution Spec-compliant registry. Currently
controller-side only; agent-side container discovery is not implemented.

**Config fields (`DockerRegistryConfig`):**

| Field | Type | Required | Default | Description | | :-------------------- | :------------------- | :------- | :-------------------- |
:--------------------------------------------------------- | | `image` | String | Yes | -- | Full image reference (e.g. `nginx`,
`ghcr.io/owner/repo`). | | `registry` | `Option<String>` | No | inferred from `image` | Override registry hostname. | | `auth` | `Option<DockerAuth>`
| No | `null` | Authentication credentials. | | `tracking_mode` | TrackingMode | No | `semver_tags` | `semver_tags` or `digest_tracking`. | |
`tag_patterns` | `Vec<String>` | No | `[]` | Regex patterns to filter tags (semver mode, OR logic). | | `tag_strip_prefix` | String | No | `"v"` |
Prefix to strip before semver parsing. | | `include_prereleases` | bool | No | `false` | Include pre-release versions. | | `tracked_tag` |
`Option<String>` | No | `"latest"` | Tag to track (digest mode). | | `page_size` | u32 | No | `1000` | Max tags per API request. | | `restart_command`
| `Option<String>` | No | `null` | Custom shell command to run after `docker pull` (e.g. `docker compose up -d`). Supports `{image}`, `{tag}`,
`{version}` placeholders (shell-escaped). |

**DockerAuth** (tagged enum with `#[serde(tag = "type")]`):

- `basic`: `username` + `password`
- `bearer`: `token`

**Tracking modes:**

- **SemverTags** (default): Lists tags from the registry, filters by `tag_patterns` (OR logic, empty = all), strips `tag_strip_prefix`, parses as
  semver (non-semver tags excluded), filters pre-releases unless `include_prereleases`, sorts descending by version. Each tag becomes an
  `UpstreamRelease` (no `release_notes`, no `published_at`, no `assets`).
- **DigestTracking**: Gets the manifest digest for `tracked_tag` (default `"latest"`). Returns a single `UpstreamRelease` with the digest as the
  version string. Useful for detecting when a mutable tag has been updated.

### Proxmox Helper Scripts provider (`uptrakit-provider-proxmox-helper-scripts`)

Executes Proxmox VE community helper script updates via `curl | bash`.

**Config fields (`ProxmoxHelperScriptsConfig`):**

| Field | Type | Required | Default | Description | | :------------ | :----- | :------- | :------ |
:-------------------------------------------------------- | | `script_url` | String | Yes | -- | URL of the helper script to execute for updates. |

**Behaviour:**

- Update execution runs `curl -fsSL -- "$script_url" | bash -s -- --update` with `set -euo pipefail`.
- The `script_url` is passed as a positional argument to bash (not interpolated into the command string), preventing injection.
- `detect_installed_version` returns `None` (version detection is not supported).
- `fetch_releases` returns an empty list (upstream version checking is not supported).

**Security note:** The `curl | bash` pattern runs arbitrary remote code. The user must trust the script URL source.
