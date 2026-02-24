# CODEREVIEW — Providers (all 6 crates)

Crates reviewed:
- `uptrakit-provider-core` (`crates/providers/core/`)
- `uptrakit-provider-registry` (`crates/providers/registry/`)
- `uptrakit-provider-github` (`crates/providers/github/`)
- `uptrakit-provider-docker` (`crates/providers/docker/`) — renamed from `uptrakit-provider-docker-registry` (`crates/providers/docker-registry/`)
- `uptrakit-provider-homebrew` (`crates/providers/homebrew/`)
- `uptrakit-provider-proxmox-helper-scripts` (`crates/providers/proxmox-helper-scripts/`)

Related shared crate reviewed for cross-cutting issues:
- `uptrakit-shared-types` (`crates/shared/types/src/provider_types.rs`)

---

## Summary

The provider subsystem is the strongest-structured domain in the codebase. The `Provider` trait is clean and object-safe, the `register_providers!` macro eliminates dispatch duplication for the normal code path, secret masking is correctly designed, and dependency injection via `CommandExecutor` makes all providers unit-testable without spawning real processes. Individual provider crates are focused, well-tested, and follow consistent patterns.

The discovery-codepath split has been resolved: `ProviderType::supports_discovery()` has been removed, the hardcoded slice in `agent_ws.rs` is gone, and `create_provider_for_discovery` is now macro-generated alongside the other five dispatch methods. Adding a new discovery-capable provider requires only implementing `capabilities()` to include `ProviderCapability::DiscoverLocalSoftware` — the registry derives everything else automatically.

A secondary concern is that platform-specific providers (Homebrew for macOS, ProxmoxHelperScripts for Proxmox VE) are compiled unconditionally into all agent binaries, causing Linux agents to accept configuration for `HomebrewProvider` and fail only at runtime when the `brew` binary is absent.

The remaining issues — a hardcoded `per_page=100` in GitHub pagination and a wrong Docker Hub user-image URL in both implementation and test — are lower severity but should be corrected before the next release.

---

## Architecture

### Strengths

**Object-safe `Provider` trait with opt-in methods.**
`crates/providers/core/src/traits.rs:22-98`. The trait has exactly one required method (`provider_type`). Every other method has a default implementation that returns a typed error. New providers only override what they support. `capabilities()` returns `&'static [ProviderCapability]` — no heap allocation on the hot version-check path. `has_capability` is a trivial slice contains check.

**`register_providers!` macro eliminates all dispatch duplication.**
`crates/providers/registry/src/registry.rs:43-156`. A single declaration block generates all six dispatch methods (`create_provider`, `validate_config`, `mask_config_secrets`, `restore_config_secrets`, `create_provider_for_discovery`, `discovery_provider_types`) with consistent error handling (`context_to`, `RegistryError`). Adding a new provider requires exactly one line in this macro invocation plus a `Cargo.toml` dependency. Discovery capability is automatically derived from each provider's `capabilities()` method — no manual list needed. The doc-comment on `ProviderRegistry` explicitly states this invariant.

**`ProviderOps` trait decouples the web API from the concrete registry.**
`crates/providers/registry/src/lib.rs:57-86`. `AppState` holds `Arc<dyn ProviderOps>` rather than a direct reference to `ProviderRegistry`. Route handlers and query helpers are testable in isolation by substituting a mock implementation. `ProviderRegistry` implements `ProviderOps` through delegation, keeping the blanket impl trivial.

**`SecretMasking` trait with no-op defaults.**
`crates/providers/core/src/secrets.rs:9-17`. Providers with no secrets implement `SecretMasking` with a single empty `impl` (see `HomebrewConfig`). Providers with secrets override the two methods. The JSON round-trip pattern in `mask_secrets_for` and `restore_secrets_for` (registry.rs:17-39) means masking logic never diverges from the serialized representation.

**`CommandExecutor` dependency injection.**
All four local providers (`HomebrewProvider`, `ProxmoxHelperScriptsProvider`, `GitHubProvider`, `DockerProvider`) receive `Arc<dyn CommandExecutor>` at construction time. Integration and unit tests pass a `LocalCommandExecutor`; production code does the same. This makes all provider logic testable without real subprocesses.

**`ProviderCapability::Other(String)` forward compatibility.**
Unknown capabilities from a newer peer are preserved and excluded from capability intersection checks. `#[non_exhaustive]` on `ProviderCapability` reinforces this. Capability gating at the event-loop level is correctly restricted to `is_known()` variants only.

**`ProxmoxHelperScriptsConfig` two-context design.**
`crates/providers/proxmox-helper-scripts/src/config.rs:61-118`. The `script_url` field defaults to an empty string at deserialization time (`#[serde(default)]`). `validate()` rejects an empty URL for version-check or update contexts. The comment at lines 63-66 clearly documents that `validate()` must not be called during discovery. This is an explicit and documented design choice, not a hidden special case.

---

## Security & Safety

### Strengths

**`SecretString` at all provider credential boundaries.**
`GitHubConfig.auth_token`, `DockerAuth::Basic.password`, `DockerAuth::Bearer.token`, and `GitHubReleaseSource.auth_token` are all `Option<SecretString>`. Secrets are never stored as `String`. The `expose_secret()` call is confined to the single point where the header value is constructed (`github/src/provider.rs:52`).

**No `unsafe` in any provider crate.**
Zero `unsafe` blocks across all six provider crates. The async executor is driven through safe trait objects only.

**Auth token header construction validates the header value.**
`github/src/provider.rs:53-57`. `HeaderValue::from_str(&value).map_err(...)` correctly propagates an error if the token contains non-ASCII characters, preventing a panic from `from_static`.

**Path traversal validation in `ProxmoxHelperScriptsConfig`.**
`crates/providers/proxmox-helper-scripts/src/config.rs:47-55`. The `GitHubReleaseSource` validation explicitly rejects `owner` and `repo` values containing `/` or `..`, defending against URL path traversal when constructing API URLs.

**`DockerAuth` uses `#[serde(tag = "type")]` tagged union.**
The discriminant is a fixed set of known variants. No free-form string dispatch. Unknown auth types fail at deserialization, not silently fall through to wrong behaviour.

**`SecretMasking::with_secrets_masked` is infallible.**
Providers that have no secrets implement an empty `impl SecretMasking for HomebrewConfig {}` and take the default no-op. The masking path can never panic by design.

### Issues

**[SEVERITY: Low]** `crates/providers/github/src/provider.rs:52` — Bearer token is materialized as a plain `String` in memory

The format string `format!("Bearer {}", token.expose_secret())` creates a heap-allocated `String` containing the full credential. This string lives until the `reqwest::Client` is dropped. For long-lived provider instances this extends the lifetime of the plaintext credential in memory beyond what `SecretString` is designed to allow. A `SecretString` wrapper around the formatted header value, or constructing the header directly from the token bytes without an intermediate allocation, would reduce the exposure window.

---

## Code Quality

### Strengths

**Consistent `validate()` → `new()` construction pattern.**
Every provider follows the same three-step construction: deserialize config, call `validate()`, construct provider. This is enforced by the `register_providers!` macro for the standard path. `GitHubProvider::new` re-validates inside its own constructor as a defense-in-depth measure.

**Pre-compiled regex in `GitHubProvider`.**
Asset filter patterns are compiled once at construction time (`github/src/provider.rs:74-84`) and stored as `Vec<Regex>`. Pattern errors are surfaced at provider creation, not at runtime during each `fetch_releases` call.

**`HomebrewPackageType` default is `None` at the config level, not `Formula`.**
`HomebrewConfig.package_type: Option<HomebrewPackageType>` correctly distinguishes between "discover all" (None) and "track a specific type" (Some(...)). The `is_cask()` helper explicitly documents this at line 182-188 of `provider.rs`. Without this distinction, the autodiscovery "discover all" mode would have required a separate config type.

**`parse_installed_formulae`/`parse_installed_casks` skip rather than error on missing fields.**
`homebrew/src/provider.rs:98-114`. Items missing a `name` or `version` field are silently skipped with `continue`. This is the correct behaviour for discovery output from a real system — a partially corrupt `brew info` output should not abort the entire scan.

**`DockerConfig` exposes `page_size` as a user-configurable field.**
`docker/src/config.rs`. Unlike GitHub's hardcoded `per_page=100`, the Docker provider allows the operator to tune pagination. Default is 1000 which is appropriate for most registries.

**Rate limit awareness in `GitHubProvider`.**
`github/src/provider.rs:163-185`. The provider reads `x-ratelimit-remaining` and `x-ratelimit-reset` headers, logs a warning when remaining < 10, and surfaces a clear error message with reset timestamp when the limit is fully exhausted.

### Issues

**[SEVERITY: Low]** `crates/providers/github/src/provider.rs:97` — GitHub releases API hardcoded at `per_page=100`, no pagination

```rust
format!("{}/repos/{}/{}/releases?per_page=100", ...)
```

The GitHub API returns at most 100 releases per page. For repositories with more than 100 releases the returned list is silently truncated. There is no `Link` header follow-through. Affected users will see stale or missing versions without any error. The `DockerConfig` pattern of a configurable `page_size` field shows the established solution in this codebase.

#### 2026-02-24 Review

##### Issues

**[SEVERITY: Low]** `docker/src/auth.rs` — `Mutex::lock().unwrap()` on `cached_token` uses `std::sync::Mutex` in an async context

Risks blocking the Tokio runtime thread if contended. `tokio::sync::Mutex` would be idiomatic.

---

## Tests

### Strengths

**`CommandExecutor` DI makes providers unit-testable.**
All six provider crates contain in-process unit tests that exercise the full provider trait methods by passing a `LocalCommandExecutor` (or mock variants). No Docker daemon or `brew` binary is required for the unit test suite to pass.

**`Provider` trait default-method tests in `core/src/traits.rs`.**
Tests at lines 100-279 cover: all five default method returns, `has_capability` for empty and non-empty capability slices, multi-capability providers, error message content (operation name present in error), and capability composition. This gives a clear specification of the default contract.

**Registry tests cover the full dispatch table.**
`registry/src/registry.rs:244-597`. Tests cover: config parsing for all four providers, valid/invalid configs for each, `create_provider` round-trip for all four, `mask_config_secrets` and `restore_config_secrets` for GitHub and ProxmoxHelperScripts (the two providers with secrets), capability verification on constructed providers, and string-type variants of all three `ProviderOps` methods.

**`DockerConfig` and `ImageRef` test coverage.**
`docker/src/config.rs` and `docker/src/image_ref.rs`. Covers validation, image reference parsing for all six reference formats (official, user, GHCR, private, localhost, port), `ImageRef::web_url` and `server_address`, serialization round-trips, `DockerAuth` both variants including masking and secret restore, and the `TrackingMode` permutations. Image reference parsing logic is centralized in `ImageRef::from_str` (previously duplicated across `infer_registry` and `resolve_repository`). 30+ test cases across the two modules.

**ProxmoxHelperScripts path-traversal tests.**
`proxmox-helper-scripts/src/config.rs:249-276`. Explicit tests for `owner` containing `/`, `repo` containing `..`, covering both path-traversal vectors in `GitHubReleaseSource.validate()`.

**HomebrewProvider parse-logic tests use JSON fixtures, not live `brew`.**
`homebrew/src/provider.rs:428-724`. All JSON parsing helpers (`parse_installed_version`, `parse_latest_version`, `parse_installed_formulae`, `parse_installed_casks`) are tested with in-process fixtures. The `LocalCommandExecutor` is used for empty-identifier guard tests without actually invoking `brew`.

### Issues

**[SEVERITY: Medium]** `crates/providers/github/src/provider.rs` — No test for `fetch_releases` HTTP interaction

`GitHubProvider::fetch_releases` is the primary controller-side operation for the GitHub provider. It is the only significant async operation in the crates with no test coverage. There is no mock HTTP server (e.g., `httpmock`) test exercising: a normal 200 response with releases, a 403 rate-limited response, a 404 not-found response, or an invalid JSON body. The existing `url_construction` test covers URL generation only. The rest of the codebase uses `MockApiServer` via `httpmock` for exactly this pattern (see CLI integration tests); it should be used here.

**[SEVERITY: Low]** `crates/providers/homebrew/src/provider.rs:708-722` — `detect_installed_version` and `fetch_releases` tested only for the empty-identifier guard

The tests `homebrew_provider_detect_installed_version_empty_identifier_fails` and `homebrew_provider_fetch_releases_empty_identifier_fails` only verify the early-return guard. There are no tests for the JSON parsing code path inside `detect_installed_version` or `fetch_releases` using the sample JSON fixtures that are already defined in the test module. The existing `parse_installed_version` and `parse_latest_version` unit tests verify the parsing helpers directly, but the full async method path (including the `is_cask()` branch and the `Version::new` wrapping) is not exercised through mocked executor output.

**[SEVERITY: Low]** `crates/providers/registry/src/registry.rs:243` — Tests use `LocalCommandExecutor` directly, not a mock executor

All registry tests construct providers with `LocalCommandExecutor`. This is acceptable for construction and config tests since providers are not invoked. However, any future test of `create_provider_for_discovery` discovery behaviour would need a mock executor. Introducing a `MockCommandExecutor` (already present elsewhere in the codebase for scheduler tests) would enable more thorough registry-level tests.

#### 2026-02-24 Review

##### Issues

**[SEVERITY: Medium]** `core/src/traits.rs:142-252`, `docker/src/provider.rs`, `github/src/provider.rs`, `homebrew/src/provider.rs`, `proxmox-helper-scripts/src/provider.rs` — All 21 provider crate async tests use bare `#[tokio::test]`

None use `start_paused = true`. Per `testing.md`, required for all async tests.

---

## High Availability

### Strengths

**`GitHubProvider` HTTP client built once at construction time.**
The `reqwest::Client` with pre-configured headers (Accept, X-GitHub-Api-Version, Authorization) is created in `new()` and held for the lifetime of the provider. Connection pool reuse is automatic. There are no per-request client allocations.

**`HomebrewProvider` command execution is stateless.**
Each call to `detect_installed_version`, `fetch_releases`, or `discover_software` invokes `brew` as a fresh subprocess. No shared mutable state means no races between concurrent calls. The `CommandExecutor` abstraction does not preclude parallelism at the caller level.

**Provider construction is infallible at the type level after `validate()`.**
If `create_provider` succeeds, the returned `Box<dyn Provider>` is guaranteed to be in a valid state. Validation errors are surfaced synchronously at configuration time, not lazily at first use.

### Issues

**[SEVERITY: Low]** `crates/providers/github/src/provider.rs` — No retry on transient HTTP failure

`fetch_releases` wraps `self.client.get(&url).send().await` with a direct `.map_err` that immediately propagates any network error. A DNS resolution failure, a TCP RST, or a 5xx response during a transient GitHub outage will abort the version check entirely. The calling agent will mark the check as failed and wait for the next scheduled interval. The `uptrakit-service-sdk` backoff utilities exist in the workspace; a simple exponential retry (max 3 attempts) on `reqwest::Error::is_connect()` or `is_timeout()` would reduce noise from transient failures.

---

## Database

### Strengths

The provider crates themselves contain no database access. All persistence is handled by `uptrakit-web-api` query helpers. The provider configuration is stored as `serde_json::Value` in the `provider_configs` table, deserialized on demand by `ProviderRegistry`. This design means database schema changes for provider configs require only migration changes, not provider crate changes.

---

## Coding Standards

### Strengths

**Consistent use of `bail!` and `report!` throughout all provider crates.**
No `Result<T, String>`, no `Report::new()`, no raw `Err(...)` construction. All errors use the `rootcause` prelude. Error types are crate-specific enums (`GitHubError`, `DockerError`, `RegistryError`) with `thiserror`-derived `Display`.

**`#[serde(rename_all = "snake_case")]` consistently applied.**
All public enums and structs in provider configs use `snake_case` serialization, matching the `ProviderType::as_str()` convention. `DockerAuth` uses `#[serde(tag = "type")]` correctly. `TrackingMode` and `HomebrewPackageType` both use `rename_all = "snake_case"`.

**`skip_serializing_if = "Option::is_none"` on all optional config fields.**
Optional fields such as `DockerConfig.auth`, `DockerConfig.tracked_tag`, `ProxmoxHelperScriptsConfig.github`, and `GitHubReleaseSource.auth_token` all use `skip_serializing_if`. API responses do not include null fields for unset options.

**Provider crate `lib.rs` files use selective re-exports.**
Each provider crate re-exports only its public surface (`Config`, `Provider`, `Error`). Internal helpers are not re-exported. `uptrakit-provider-registry/src/lib.rs` explicitly documents its re-export strategy and the purpose of each public type.

**Zero `#[allow(clippy::...)]` in any provider crate.**
No suppressed Clippy lints across all six crates.

### Issues

**[SEVERITY: Medium]** `crates/providers/registry/src/registry.rs` — No `#[must_use]` on `mask_config_secrets` and `mask_config_secrets_str`

Both methods return a `serde_json::Value` representing the masked configuration. If a caller forgets to use the return value (for example, calling `mask_config_secrets(...)` without assigning the result), the masking has no effect and the original config is used. Adding `#[must_use]` to both methods would produce a compiler warning for callers that discard the result. `restore_config_secrets` correctly mutates in place and does not need `#[must_use]`.

**[SEVERITY: Low]** `crates/providers/proxmox-helper-scripts/src/config.rs:67` — `script_url` empty-string default is a semantic workaround, not a type-safe design

The `#[serde(default)]` on `script_url` exists to allow `{}` to deserialize successfully for the discovery context. The doc-comment explains this is intentional. However, the `String` type with an empty-string sentinel conflates "not provided" with "explicitly set to empty". Using `Option<String>` for `script_url` with `#[serde(default)]` would make the distinction type-safe: `None` means "not provided" (valid for discovery), `Some("")` would remain an error from `validate()`. This would also eliminate the need for the special-case comment warning developers not to call `validate()` during discovery, since the caller could pass `script_url: None` explicitly.

---

## Extensibility

### Strengths

**Adding any provider requires exactly one line in the macro.**
The `register_providers!` invocation at `registry.rs` is the single authoritative source for all six dispatch methods. For a provider without discovery, the full extension path is: new crate, new `ProviderType` variant, one macro line, one `Cargo.toml` dependency. For a discovery-capable provider, additionally include `ProviderCapability::DiscoverLocalSoftware` in the provider's `capabilities()` method — `discovery_provider_types()` is fully auto-derived. No other files require changes.

**`ProviderCapability` is `#[non_exhaustive]`.**
`crates/shared/types/src/provider_types.rs:10`. Unknown capability variants from a newer binary are preserved through the `Other(String)` case in the wire protocol and excluded from capability intersection, rather than causing deserialization errors. New capabilities can be added in a future release without breaking older agents.

**`ServiceHandler` trait externalizes the entire service-specific surface.**
The `ServiceHandler` trait in `service-sdk` means adding a new service type (agent, SSH agent, MQTT broker) requires only a new `ServiceHandler` implementor. Provider crates are decoupled from service lifecycle concerns.

**`CloseReason::Unknown(String)` and `Capability::Other(String)` establish a consistent forward-compatibility pattern.**
Both wire enums use the same `Unknown`/`Other` string-preserving variant. New provider capability or close reason additions are forward-compatible across versions.

### Issues

**[SEVERITY: Medium]** `crates/providers/registry/src/registry.rs:151-156` — No feature-flag gating for platform-specific providers

`HomebrewProvider` is macOS-specific. `ProxmoxHelperScriptsProvider` is Proxmox VE-specific. Both are compiled unconditionally into all agent binaries, including Linux agents where `brew` is absent. A Linux agent will accept a valid `HomebrewProvider` configuration (since `validate()` does not check for `brew` presence), construct the provider successfully, and fail only when `detect_installed_version` or `discover_software` is called.

The correct fix for `HomebrewProvider` is conditional compilation with `#[cfg(target_os = "macos")]` or a `homebrew` Cargo feature. For `ProxmoxHelperScriptsProvider`, a `proxmox` Cargo feature would allow operators to build agents for Proxmox environments specifically.

**[SEVERITY: Medium]** `crates/shared/wire/src/lib.rs:214-234` — `ServiceMessage` and `ControllerMessage` mix agent and MQTT concerns

MQTT-specific message variants are deserializable on agent WebSocket connections. `ServiceHandler` implementors must mentally classify each variant to know which ones apply to their connection type. This makes the trait surface larger than necessary for new implementors, and creates a risk of an agent responding to an MQTT-targeted message. Splitting into `AgentMessage`/`MqttMessage` union types, or introducing a `MessageTarget` discriminant, would clarify the intended receiver.

**[SEVERITY: Low]** `crates/shared/wire/src/lib.rs:316-318` — `EnrollPayload.service_type` deprecation is documented but unenforced

The comment states that `service_type` will eventually be inferred from capabilities. There is no `#[deprecated]` attribute, no compiler warning, and no tracking issue. New consumers of `EnrollPayload` will use the field without knowing it is intended for removal.

**[SEVERITY: Low]** `crates/providers/proxmox-helper-scripts/src/config.rs:67-74` — No shared abstraction for "config valid for discovery but not update"

`ProxmoxHelperScriptsConfig` uses an empty-string default for `script_url` to enable discovery with a minimal config. There is no shared trait or type that expresses "this config is valid for discovery but not for update execution". A future provider with a similar split between discovery-config and full-config validity will face the same design challenge and may solve it differently, leading to an inconsistent pattern across providers.

#### 2026-02-24 Review

##### Strengths

- **`refresh_package_index` method with default error enables incremental capability adoption.** `core/src/traits.rs:92-97` — Existing providers compile without changes; only supporting providers override.

##### Issues

**[SEVERITY: Low]** `core/src/types.rs:18-25` — `ProviderCapability` has `#[non_exhaustive]` but no `Other(String)` variant unlike its wire counterpart

New capabilities require synchronized recompilation of all agents.

---

## Provider Extension Checklist

When adding a new provider, the following steps are required. Steps marked (clean) are covered by the macro or trait and require no extra work. Steps marked (manual) require coordinated changes in multiple locations and are tracked as issues above.

| Step | Location | Status |
|------|----------|--------|
| New crate with `Provider` + config struct | `crates/providers/<name>/` | Clean |
| Implement `SecretMasking` | provider config struct | Clean |
| One line in `register_providers!` | `registry.rs` | Clean |
| Dependency in registry `Cargo.toml` | `registry/Cargo.toml` | Clean |
| New variant in `ProviderType` | `shared/types/src/provider_types.rs` | Clean |
| `as_str()`, `FromStr`, `Display` for new variant | `shared/types/src/provider_types.rs` | Clean |
| **If discovery-capable:** include `ProviderCapability::DiscoverLocalSoftware` in `capabilities()` | provider crate | Clean |
| **If special identifier rules:** implement `validate_package_identifier` | provider crate | Clean |

All previously "Manual" steps have been eliminated — discovery support is now fully auto-derived from the `register_providers!` macro and the provider's `capabilities()` method. Package-identifier validation is now handled through `ProviderRegistry::validate_package_identifier`.
