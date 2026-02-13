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

## Provider Architecture - Detailed

Each software item is associated with a provider. A provider defines:

| Concern | Runs on | Responsibility |
| :---------------------- | :------------------ | :----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Remote/upstream version | Controller or Agent | Fetch latest version metadata. Most providers (GitHub, Docker) resolve on the controller. Providers with a local package index (Homebrew) resolve on the agent via `RefreshPackageIndex` + `fetch_releases()` and report `latest_version` in `VersionCheckResult`. |
| Local/installed version | Agent | Detect currently installed version |
| Update execution | Agent | Run the update (via sudo-allowlisted commands or custom script) |

Provider crates:

| Crate | Path | Purpose |
| :----------------------------------------- | :----------------------------------------- | :----------------------------------------------------- |
| `uptrakit-command` | `crates/shared/command/` | Shell command execution and streaming utilities. |
| `uptrakit-provider-core` | `crates/providers/core/` | Provider trait/abstractions; delegates commands. |
| `uptrakit-provider-registry` | `crates/providers/registry/` | Centralized provider dispatch and validation. |
| `uptrakit-provider-docker-registry` | `crates/providers/docker-registry/` | Docker/OCI Registry: tracks image tags. |
| `uptrakit-provider-github` | `crates/providers/github/` | GitHub Releases: fetches metadata; agent installs. |
| `uptrakit-provider-homebrew` | `crates/providers/homebrew/` | Homebrew: agent-side version tracking and updates. |
| `uptrakit-provider-proxmox-helper-scripts` | `crates/providers/proxmox-helper-scripts/` | Proxmox VE: auto-discovers and manages helper scripts. |

The **Provider Registry** crate centralizes all provider operations:

- `ProviderRegistry::create_provider()` — creates `Provider` instances from `ProviderType` and config
- `ProviderRegistry::validate_config()` — validates provider configuration JSON
- `ProviderRegistry::mask_config_secrets()` / `restore_config_secrets()` — handles secret masking for API responses
  (delegates to typed `with_secrets_masked()` / `restore_secrets_from()` methods on each config struct)

The agent crate imports `uptrakit-command` for shell execution and `uptrakit-provider-registry` for provider dispatch —
it does not depend on `uptrakit-provider-core` directly. The web-api crate imports only `uptrakit-provider-registry`.
This eliminates scattered string-based provider matching and keeps all dispatch logic in one place.

The update step can always be overridden by a custom shell script, regardless of provider.

### Provider capabilities

The `ProviderCapability` enum defines optional features a provider may support:

| Capability | Trait method | Description |
| :---------------------- | :------------------------ | :-------------------------------------------------------------- |
| `DiscoverLocalSoftware` | `discover_software()` | Enumerate software the provider can manage on the local system. |
| `RefreshPackageIndex` | `refresh_package_index()` | Refresh local package index (for example, `apt update`). |

### Software discovery

The `Provider` trait includes an optional `discover_software()` method that allows providers to enumerate software they
can manage on the local system. Providers that support this capability declare
`ProviderCapability::DiscoverLocalSoftware` in their `capabilities()` method. The method returns a
`Vec<DiscoveredSoftware>`, where each entry contains:

| Field | Type | Description |
| :------------------- | :-------------------------- | :------------------------------------------------------------------------ |
| `package_identifier` | `String` | Provider-specific identifier (maps to `SoftwareItem.package_identifier`). |
| `name` | `String` | Human-readable display name. |
| `installed_version` | `Option<Version>` | Currently installed version, if detected. |
| `extra` | `Option<serde_json::Value>` | Arbitrary provider-specific metadata (for example, install path). |

The default implementation returns an empty list. Providers that support discovery (e.g., Proxmox Helper-Scripts)
override this method to scan the local system.

### GitHub Releases provider (`uptrakit-provider-github`)

Fetches release metadata from the GitHub API and converts it into `UpstreamRelease` values.

**Config fields (`GitHubConfig`):**

| Field | Type | Required | Default | Description |
| :-------------------- | :------------ | :------- | :----------------------- | :----------------------------------------------------------- |
| `owner` | String | Yes | — | GitHub repository owner. |
| `repo` | String | Yes | — | GitHub repository name. |
| `auth_token` | String | No | `null` | Personal access token (private repos or higher rate limits). |
| `api_base_url` | String | No | `https://api.github.com` | API base URL (for GitHub Enterprise). |
| `include_prereleases` | bool | No | `false` | Whether to include pre-release versions. |
| `tag_strip_prefix` | String | No | `"v"` | Prefix to strip from tag names to extract version strings. |
| `asset_patterns` | `Vec<String>` | No | `[]` | Regex patterns to filter release assets (empty means all). |

**Behaviour:**

- Drafts are always skipped
- Rate limit headers are checked; warnings logged when remaining < 10
- 403/429 responses with `x-ratelimit-remaining: 0` return a rate-limit error
- Asset filtering uses regex matching against asset names

### Docker Registry provider (`uptrakit-provider-docker-registry`)

Tracks container image tags from OCI/Docker registries. Supports Docker Hub, GHCR, and any OCI Distribution
Spec-compliant registry. Currently controller-side only; agent-side container discovery is not implemented.

**Config fields (`DockerRegistryConfig`):**

| Field | Type | Required | Default | Description |
| :-------------------- | :------------------- | :------- | :-------------------- | :--------------------------------------------------------- |
| `image` | String | Yes | -- | Full image reference (e.g. `nginx`, `ghcr.io/owner/repo`). |
| `registry` | `Option<String>` | No | inferred from `image` | Override registry hostname. |
| `auth` | `Option<DockerAuth>` | No | `null` | Authentication credentials. |
| `tracking_mode` | TrackingMode | No | `semver_tags` | `semver_tags` or `digest_tracking`. |
| `tag_patterns` | `Vec<String>` | No | `[]` | Regex patterns to filter tags (semver mode, OR logic). |
| `tag_strip_prefix` | String | No | `"v"` | Prefix to strip before semver parsing. |
| `include_prereleases` | bool | No | `false` | Include pre-release versions. |
| `tracked_tag` | `Option<String>` | No | `"latest"` | Tag to track (digest mode). |
| `page_size` | u32 | No | `1000` | Max tags per API request. |

**DockerAuth** (tagged enum with `#[serde(tag = "type")]`):

- `basic`: `username` + `password`
- `bearer`: `token`

**Tracking modes:**

- **SemverTags** (default): Lists tags from the registry, filters by `tag_patterns` (OR logic, empty = all), strips
  `tag_strip_prefix`, parses as semver (non-semver tags excluded), filters pre-releases unless `include_prereleases`,
  sorts descending by version. Each tag becomes an `UpstreamRelease` (no `release_notes`, no `published_at`, no
  `assets`).
- **DigestTracking**: Gets the manifest digest for `tracked_tag` (default `"latest"`). Returns a single
  `UpstreamRelease` with the digest as the version string. Useful for detecting when a mutable tag has been updated.
