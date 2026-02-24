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
`ParseProviderTypeError` on failure). The string representations are: `github_releases`, `proxmox_helper_scripts`, `docker`, `homebrew`,
`apt`.

## Dependencies and re-exports

Provider crates should avoid unnecessary direct dependencies. The `uptrakit-provider-core` crate
re-exports commonly needed types:

- **`uptrakit_provider_core::mpsc`** — re-export of `tokio::sync::mpsc`. Use this instead of
  depending on tokio directly. Tokio should only be in `[dev-dependencies]` (for `#[tokio::test]`).
- **`uptrakit_provider_core::CommandExecutor`**, **`CommandSpec`**, etc. — re-exports from
  `uptrakit-command`.
- **`uptrakit_provider_core::SecretString`** — re-export from `uptrakit-shared-types`.

See [Dependency Policy](dependency-policy.md) for the full re-export strategy.

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
`crates/providers/registry/` | Centralized provider dispatch and validation; re-exports `ProviderType`. | | `uptrakit-provider-docker` |
`crates/providers/docker/` | Docker/OCI image tracking and container discovery. | | `uptrakit-provider-github` | `crates/providers/github/` | GitHub
Releases: fetches metadata; agent installs. | | `uptrakit-provider-homebrew` | `crates/providers/homebrew/` | Homebrew: agent-side version tracking
and updates. | | `uptrakit-provider-proxmox-helper-scripts` | `crates/providers/proxmox-helper-scripts/` | Proxmox VE: auto-discovers and manages
helper scripts. |

The **Provider Registry** crate centralizes all provider operations using a `register_providers!` macro that generates
all dispatch methods from a single declaration:

```rust
register_providers! {
    GithubReleases => { config: GitHubConfig, provider: GitHubProvider },
    Docker => { config: DockerConfig, provider: DockerProvider },
    ProxmoxHelperScripts => { config: ProxmoxHelperScriptsConfig, provider: ProxmoxHelperScriptsProvider },
    Homebrew => { config: HomebrewConfig, provider: HomebrewProvider },
}
```

The macro generates six methods:

- `ProviderRegistry::create_provider()` — deserializes config, validates it, and instantiates the provider.
- `ProviderRegistry::validate_config()` — deserializes and validates provider configuration JSON.
- `ProviderRegistry::mask_config_secrets()` / `restore_config_secrets()` — handles secret masking for API responses
  (delegates to the `SecretMasking` trait implemented on each config struct).
- `ProviderRegistry::create_provider_for_discovery()` — same as `create_provider` but without calling `validate()`,
  so discovery works with empty or minimal configs (e.g., `ProxmoxHelperScriptsConfig` with no `script_url`).
- `ProviderRegistry::discovery_provider_types()` — returns the list of `ProviderType` variants whose provider
  reports `ProviderCapability::DiscoverLocalSoftware` in `capabilities()`. Fully auto-derived from the macro —
  no manual list needed.

To add a new provider, add a single entry to the `register_providers!` invocation. The macro generates all match arms
for all six methods automatically. If the provider supports discovery, implement `capabilities()` to include
`ProviderCapability::DiscoverLocalSoftware` — `discovery_provider_types()` will automatically include it.

**Discovery capability is registry-derived.** Use `state.provider_ops.discovery_provider_types()` in route handlers
(or `ProviderRegistry::discovery_provider_types()` statically) to get the current list of discovery-capable provider
types. Do not maintain a separate static list or override method.

### Package identifier validation

Some providers impose format constraints on the `package_identifier` field of a `SoftwareItem`. For example, Homebrew
identifiers must not contain whitespace, path-traversal segments (`..`, `.`), or characters outside `[A-Za-z0-9\-_.@+/]`.

Provider-specific identifier validation is exposed through two APIs:

**Static dispatch (preferred for internal use):**

```rust
ProviderRegistry::validate_package_identifier(ProviderType::Homebrew, value)?;
```

**Trait object dispatch (via `ProviderOps`):**

```rust
state.provider_ops.validate_package_identifier_str(&config.provider_type, value)?;
```

Returns `Ok(())` for unknown provider types (no constraints apply). Returns `Err(String)` with a human-readable message
when the identifier is invalid.

**When adding a new provider with identifier constraints:**

1. Add a `pub fn validate_identifier(value: &str) -> Result<(), String>` to your provider crate
   (e.g., `crates/providers/my-provider/src/provider.rs`) and re-export it from `lib.rs`.
2. Add a match arm to `ProviderRegistry::validate_package_identifier` in
   `crates/providers/registry/src/registry.rs`:

   ```rust
   pub fn validate_package_identifier(
       provider_type: ProviderType,
       value: &str,
   ) -> std::result::Result<(), String> {
       match provider_type {
           ProviderType::Homebrew => uptrakit_provider_homebrew::validate_identifier(value),
           ProviderType::MyProvider => uptrakit_provider_my_provider::validate_identifier(value),
           _ => Ok(()),
       }
   }
   ```

3. Add unit tests in your provider crate covering valid identifiers, empty identifiers, and all
   constraint violations.

Do **not** add provider-specific identifier validation logic to the web API layer or query helpers. All identifier
validation must go through `ProviderRegistry::validate_package_identifier`.

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

Providers with no secrets (Homebrew) use the default no-op implementations. Providers with secrets (GitHub, Docker, Proxmox Helper Scripts
when GitHub config is present) override both methods with field-level masking logic.

The registry uses generic helpers `mask_secrets_for::<T>()` and `restore_secrets_for::<T>()` that deserialize the JSON config, apply the trait
methods, and re-serialize. This eliminates duplicated deserialize-method-serialize boilerplate per provider.

When adding a new provider with secrets, implement `SecretMasking` on your config struct. The `register_providers!`
macro handles the dispatch automatically.

All provider `new()` constructors must return `Result<Self, Report<ProviderError>>` so the registry can handle
instantiation failures uniformly. The constructor should validate its configuration before returning.

### Bidirectional error conversion

Every provider crate defines its own error enum (e.g., `DockerError`, `GitHubError`) and implements **bidirectional** `impl_report_conversion!`
between the provider-specific error and the shared `ProviderError`:

```rust
use uptrakit_shared_macros::impl_report_conversion;

// Provider-specific → shared (for the registry to propagate errors)
impl_report_conversion!(DockerError => ProviderError, |e| ProviderError::ProviderInternal(e.to_string()));

// Shared → provider-specific (for providers calling shared code that returns ProviderError)
impl_report_conversion!(ProviderError => DockerError, |e| DockerError::Configuration(e.to_string()));
```

This bidirectional pattern allows:

- The provider registry to convert provider errors into `ProviderError` when dispatching.
- Provider implementations to call shared code (e.g., from `uptrakit-provider-core`) and convert `ProviderError` back into their local error type.

When adding a new provider, always implement both directions.

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

### Docker provider (`uptrakit-provider-docker`)

Tracks container image tags from OCI/Docker registries. Supports Docker Hub, GHCR, and any OCI
Distribution Spec-compliant registry. Supports both controller-side upstream version resolution
and agent-side container discovery.

**No fields are required.** An empty config `{}` is valid (suitable for discovery and digest
tracking of images whose reference is stored on the software item).

The `package_identifier` on the software item is the image reference (e.g. `nginx`,
`ghcr.io/owner/app`). Image reference parsing follows the standard Docker format: missing registry
defaults to Docker Hub, missing tag defaults to `latest`, single-segment names have `library/`
prepended for Docker Hub.

**Config struct: `DockerConfig`**

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `docker_host` | `Option<String>` | `null` | Docker daemon endpoint override |
| `auth` | `Option<DockerAuth>` | `null` | Registry authentication credentials |
| `tracking_mode` | `TrackingMode` | `semver_tags` | `semver_tags` or `digest_tracking` |
| `tag_patterns` | `Vec<String>` | `[]` | Regex filters for tags (OR logic; empty = all) |
| `tag_strip_prefix` | String | `"v"` | Prefix to strip before semver parsing |
| `include_prereleases` | bool | `false` | Include pre-release versions |
| `tracked_tag` | `Option<String>` | `"latest"` | Tag to track in digest mode |
| `page_size` | u32 | `1000` | Max tags per registry API request |
| `compose_restart` | `Option<ComposeRestartConfig>` | `null` | Run `docker compose up -d` after pull |
| `post_pull_command` | `Option<String>` | `null` | Custom shell command after pull |
| `ssh_key_path` | `Option<String>` | `null` | SSH key path for SSH docker connections (injected by agent-ssh) |

**DockerAuth** (tagged enum with `#[serde(tag = "type")]`):

- `basic`: `username` + `password`
- `bearer`: `token`

**Tracking modes:**

- **SemverTags** (default): Lists tags from the registry, filters by `tag_patterns` (OR logic, empty = all), strips `tag_strip_prefix`, parses as
  semver (non-semver tags excluded), filters pre-releases unless `include_prereleases`, sorts descending by version. `detect_installed_version`
  always returns `None` in this mode.
- **DigestTracking**: Gets the manifest digest for `tracked_tag` (default `"latest"`). Returns a single `UpstreamRelease` with the digest as the
  version string. `detect_installed_version` calls bollard `inspect_image` to read the local image digest from `RepoDigests`.

**Capabilities: `[DiscoverLocalSoftware]`**

Discovery calls bollard `list_containers(all=true)` then `inspect_image` per image. Bare SHA image
refs and locally built images (no `RepoDigests`) are skipped.

**Image pulling via bollard (`execute_update`):**

The `execute_update` step pulls the image directly through the Docker daemon API using the
[bollard](https://github.com/fussybeaver/bollard) crate — no `docker` CLI binary is required. The daemon
is reached via the configured `docker_host` (or defaults).

Progress events from the daemon are streamed to the caller via `output_tx`. Errors surfaced in the daemon
response are converted to `DockerError::PullFailed` and fail the update immediately.

The internal `DockerClient` trait abstracts pull, inspect, and list\_containers so that tests can inject a
`MockDockerClient` without a live Docker daemon:

```rust
// Production: BollardDockerClient (created by DockerProvider::new)
// Tests:      MockDockerClient (injected via DockerProvider::new_for_test)
```

Auth credentials from `DockerAuth` are forwarded to bollard's `DockerCredentials`:

- `basic` auth → `DockerCredentials { username, password, serveraddress }` (serveraddress inferred from
  the image reference via `ImageRef::server_address()`).
- `bearer` auth → `DockerCredentials { registrytoken }`.

After a successful pull, `compose_restart` (if set) runs `docker compose up -d`, then
`post_pull_command` (if set) is executed via the injected `CommandExecutor`.

**ConnectionContext injection (SSH agent):**

`agent-ssh` constructs a `ConnectionContext` with `docker_host_override = "ssh://user@host:port"`
and passes it to every `handle_*` call in `agent-core`. The context is applied to the provider
config before deserialization via `ConnectionContext::apply_to_config()`. This means the Docker
provider automatically reaches the remote daemon over SSH when invoked by `agent-ssh`, with no
manual `docker_host` configuration required.

See [Autodiscovery](../end-user/providers/docker.md#remote-docker-via-ssh) for end-user
documentation on remote Docker.

### Proxmox Helper Scripts provider (`uptrakit-provider-proxmox-helper-scripts`)

Manages software installed via [Proxmox VE community helper scripts](https://github.com/community-scripts/ProxmoxVE).
Supports automatic discovery of PHS-managed software, installed version detection, and update execution via `curl | bash`.

**Config fields (`ProxmoxHelperScriptsConfig`):**

| Field | Type | Required | Default | Description | | :------------ | :-------------------------- | :------- | :------ |
:-------------------------------------------------------- | | `script_url` | String | Yes | -- | URL of the helper script to execute for updates. | |
`github` | `Option<GitHubReleaseSource>` | No | `null` | GitHub release source for upstream version detection (see below). |

**Capabilities:** `DiscoverLocalSoftware`, conditionally `RefreshPackageIndex` (when `github` config is present)

**Discovery (`discover_software()`):**

PHS containers created by community-scripts have a well-known update script at `/usr/bin/update` containing
`curl | bash` invocations pointing at the community-scripts GitHub repository. The provider:

1. Reads `/usr/bin/update` via `cat`.
2. Parses the file for URLs matching `https://raw.githubusercontent.com/community-scripts/ProxmoxVE/main/ct/{slug}.sh`.
3. Validates each slug (`[a-z0-9][a-z0-9-]*`), deduplicates by slug.
4. For each slug, attempts to read the version file at `$HOME/.{slug}`.
5. Returns `DiscoveredSoftware` entries with `package_identifier` set to the slug, a display name derived from the slug,
   the installed version (if found), and the script URL as `extra` metadata.

If `/usr/bin/update` does not exist (not a PHS container), discovery returns an empty list without error.

**Version detection (`detect_installed_version()`):**

PHS scripts store the installed version in `$HOME/.{app_lc}` (e.g. `~/.booklore` contains `1.18.5`).
The provider reads this file for the given `package_identifier` (which must be a valid slug). If the file
does not exist or is empty, returns `None`.

The `package_identifier` is validated against `[a-z0-9][a-z0-9-]*` to prevent path traversal.

**Update execution:**

- Runs `curl -fsSL -- "$script_url" | bash -s -- --update` with `set -euo pipefail`.
- The `script_url` is passed as a positional argument to bash (not interpolated into the command string), preventing injection.

**Upstream version detection via GitHub (`github` config):**

Many PHS-installed applications are distributed via GitHub Releases (e.g., BookLore, Crafty Controller).
When the optional `github` field is present in the config, the provider gains the `RefreshPackageIndex` capability
and delegates `fetch_releases()` to an internal `GitHubProvider` instance. The `refresh_package_index()` method
is a no-op since the GitHub API doesn't require a local index refresh.

Since different PHS apps have different upstream GitHub repos, the `github` field is typically provided via
per-item `config_override` (merged by the agent at runtime). Example `config_override`:

```json
{
  "github": {
    "owner": "BookLore",
    "repo": "BookLore"
  }
}
```

| Field | Type | Required | Default | Description |
| :-------------------- | :------------ | :------- | :------ | :-------------------------------------------------------- |
| `owner` | String | Yes | -- | GitHub repository owner (user or organization). |
| `repo` | String | Yes | -- | GitHub repository name. |
| `auth_token` | String | No | `null` | Personal access token (private repos or higher rate limits). |
| `tag_strip_prefix` | String | No | `"v"` | Prefix to strip from tag names to extract version strings. |
| `include_prereleases` | bool | No | `false` | Whether to include pre-release versions. |

Without the `github` field, `fetch_releases()` returns an error (upstream version checking is unavailable).

**Security note:** The `curl | bash` pattern runs arbitrary remote code. The user must trust the script URL source.

**Parsing is pure and testable:** All parsing logic (URL extraction, slug validation, version file parsing,
display name generation) lives in the `discovery` module as pure functions with comprehensive unit tests.
