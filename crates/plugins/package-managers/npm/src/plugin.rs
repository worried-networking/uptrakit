#![expect(
    clippy::string_slice,
    reason = "string slices use byte positions derived from ASCII-only content or fixed-length pattern matching; UTF-8 boundary safety is guaranteed by construction"
)]
use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uptrakit_plugin_infrastructure_core::command::CommandExecutor;
use uptrakit_plugin_infrastructure_core::{
    ConfigModel, ConfigTestKind, HostRequirements, HostRuntime, PluginConfigValidationError,
    PluginFamily, Result, SudoCommandEntry, UpstreamRelease, Version, declare_plugin,
};
use uptrakit_plugin_infrastructure_core::{PluginHttpClientConfig, build_plugin_http_client};

use crate::config::NpmConfig;

/// Maximum number of retry attempts for transient npm registry request failures.
pub(crate) const FETCH_MAX_RETRIES: usize = 3;

/// Initial backoff delay for npm registry retries.
pub(crate) const FETCH_BACKOFF_BASE: Duration = Duration::from_secs(1);

/// Maximum backoff delay between retry attempts.
pub(crate) const FETCH_BACKOFF_MAX: Duration = Duration::from_secs(10);

/// npm packages that are package-manager infrastructure, not tracked applications.
///
/// These are filtered out during autodiscovery so that tooling does not appear
/// as managed software items alongside real applications.
pub const SYSTEM_NPM_PACKAGES: &[&str] = &["npm", "n", "nvm", "yarn", "pnpm", "corepack"];

/// Pre-release dist-tags that may be emitted when `include_prereleases` is true.
pub(crate) const PRERELEASE_DIST_TAGS: &[&str] = &["next", "beta", "alpha", "rc", "canary"];

/// Validate an npm package identifier.
///
/// Accepts both plain packages and scoped packages (`@scope/name`):
///
/// **Plain packages:**
/// - Between 1 and 214 characters.
/// - Must start with `[a-z0-9]` (no leading `.` or `_`).
/// - May only contain `[a-z0-9\-._]` — no uppercase, no spaces.
/// - Must not contain `..` (path-traversal protection).
///
/// **Scoped packages** (`@scope/name`):
/// - Must start with `@`.
/// - The scope part (before `/`) and the name part (after `/`) each follow
///   the plain package rules above.
pub fn validate_identifier(value: &str) -> std::result::Result<(), PluginConfigValidationError> {
    if value.is_empty() {
        return Err(PluginConfigValidationError::InvalidIdentifier(
            "package_identifier must not be empty".to_string(),
        ));
    }

    if let Some(without_at) = value.strip_prefix('@') {
        // Scoped package: @scope/name
        let slash = without_at.find('/').ok_or_else(|| {
            PluginConfigValidationError::InvalidIdentifier(
                "scoped package_identifier must contain a '/' after the scope".to_string(),
            )
        })?;
        let scope = &without_at[..slash];
        let name = &without_at[slash + 1..];
        validate_npm_name_part(scope, "scope")?;
        validate_npm_name_part(name, "name")?;
        // Total length including `@` and `/`.
        if value.len() > 214 {
            return Err(PluginConfigValidationError::InvalidIdentifier(
                "package_identifier must not exceed 214 characters".to_string(),
            ));
        }
    } else {
        validate_npm_name_part(value, "package")?;
    }

    Ok(())
}

/// Validate a single npm name component (scope or package name).
fn validate_npm_name_part(
    part: &str,
    role: &str,
) -> std::result::Result<(), PluginConfigValidationError> {
    if part.len() > 214 {
        return Err(PluginConfigValidationError::InvalidIdentifier(format!(
            "package_identifier {role} must not exceed 214 characters"
        )));
    }

    let Some(first) = part.chars().next() else {
        return Err(PluginConfigValidationError::InvalidIdentifier(format!(
            "package_identifier {role} must not be empty"
        )));
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(PluginConfigValidationError::InvalidIdentifier(format!(
            "package_identifier {role} must start with a lowercase letter or digit, found '{first}'"
        )));
    }

    for ch in part.chars() {
        if !ch.is_ascii_lowercase() && !ch.is_ascii_digit() && !matches!(ch, '-' | '.' | '_') {
            return Err(PluginConfigValidationError::InvalidIdentifier(format!(
                "package_identifier {role} contains invalid character: '{ch}'"
            )));
        }
    }

    if part.contains("..") {
        return Err(PluginConfigValidationError::InvalidIdentifier(format!(
            "package_identifier {role} must not contain '..'"
        )));
    }

    Ok(())
}

/// Validate an npm version string before it is interpolated into install commands.
///
/// Allows only semver-compatible characters (`[a-zA-Z0-9._+-]`). Rejects:
/// - Empty strings
/// - Protocol prefixes (`file:`, `git+`, `http:`, `https:`) that could redirect npm
///   to attacker-controlled sources
/// - Strings exceeding 256 characters
pub fn validate_version(version: &str) -> std::result::Result<(), PluginConfigValidationError> {
    if version.is_empty() {
        return Err(PluginConfigValidationError::Contract(
            "version must not be empty".to_string(),
        ));
    }
    if version.len() > 256 {
        return Err(PluginConfigValidationError::Contract(
            "version must not exceed 256 characters".to_string(),
        ));
    }
    // Reject protocol prefixes that npm would interpret as non-registry sources.
    for prefix in &["file:", "git+", "http:", "https:"] {
        if version.starts_with(prefix) {
            return Err(PluginConfigValidationError::Contract(format!(
                "version must not start with protocol prefix '{prefix}'"
            )));
        }
    }
    for ch in version.chars() {
        if !ch.is_ascii_alphanumeric() && !matches!(ch, '.' | '_' | '+' | '-') {
            return Err(PluginConfigValidationError::Contract(format!(
                "version contains invalid character: '{ch}'"
            )));
        }
    }
    Ok(())
}

/// The default npm registry base URL.
const DEFAULT_REGISTRY_BASE: &str = "https://registry.npmjs.org";

/// Build the npm registry URL for a package identifier.
///
/// Scoped packages (`@scope/name`) are URL-encoded: the `/` is encoded as `%2F`.
///
/// The `registry_base` parameter overrides the default (`https://registry.npmjs.org`).
/// Pass `None` to use the default.
pub fn npm_registry_url(package_identifier: &str, registry_base: Option<&str>) -> String {
    let base = registry_base
        .unwrap_or(DEFAULT_REGISTRY_BASE)
        .trim_end_matches('/');
    if let Some(without_at) = package_identifier.strip_prefix('@') {
        // Encode `@scope/name` as `@scope%2Fname`.
        let encoded = without_at.replacen('/', "%2F", 1);
        format!("{base}/@{encoded}")
    } else {
        format!("{base}/{package_identifier}")
    }
}

/// Build the npm website URL for a specific package version.
pub fn npm_release_url(package_identifier: &str, version: &str) -> String {
    format!("https://www.npmjs.com/package/{package_identifier}/v/{version}")
}

/// Plugin for the npm package manager.
///
/// Supports:
/// - Installed version detection via `npm list -g <package> --depth=0 --json`.
/// - Controller-side release fetching from the npm registry.
/// - Autodiscovery of globally-installed npm packages.
/// - Privileged updates via `npm install -g <package>@<version>`.
pub struct NpmPlugin {
    pub(crate) config: NpmConfig,
    pub(crate) executor: Arc<dyn CommandExecutor>,
    pub(crate) client: reqwest::Client,
}

impl NpmPlugin {
    /// Create a new npm plugin with the given configuration and host runtime.
    pub fn new(
        config: NpmConfig,
        runtime: Arc<dyn HostRuntime>,
    ) -> std::result::Result<Self, String> {
        let executor = runtime.executor();

        let client = build_plugin_http_client(PluginHttpClientConfig {
            user_agent: concat!(
                "uptrakit-plugin-package-manager-npm/",
                env!("CARGO_PKG_VERSION")
            ),
            ..Default::default()
        })
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;

        Ok(Self {
            config,
            executor,
            client,
        })
    }

    /// Sudo commands required by this plugin.
    fn required_sudo_commands(_config: &serde_json::Value) -> Vec<SudoCommandEntry> {
        // `install -g *` covers both `npm install -g PKG@VER` and
        // batch `npm install -g PKG1@VER1 PKG2@VER2 ...`.
        vec![
            SudoCommandEntry::new("npm", "Install or upgrade a global npm package")
                .with_args_suffix(Cow::Borrowed("install -g *")),
        ]
    }

    pub(crate) fn require_package_identifier(&self, package_identifier: &str) -> Result<()> {
        uptrakit_plugin_infrastructure_core::require_package_identifier(
            package_identifier,
            validate_identifier,
        )
    }

    /// Parse installed version from `npm list -g <package> --depth=0 --json` output.
    ///
    /// Expected JSON shape:
    /// ```json
    /// { "dependencies": { "<package>": { "version": "X.Y.Z" } } }
    /// ```
    pub(crate) fn parse_npm_list_version(output: &str, package: &str) -> Option<String> {
        let json: serde_json::Value = serde_json::from_str(output).ok()?;
        let version = json
            .get("dependencies")?
            .get(package)?
            .get("version")?
            .as_str()?
            .to_string();
        if version.is_empty() {
            None
        } else {
            Some(version)
        }
    }

    /// Parse all globally installed packages from `npm list -g --depth=0 --json`.
    ///
    /// Returns `(name, version)` pairs for all entries in `dependencies`.
    pub(crate) fn parse_npm_list_all(output: &str) -> Vec<(String, String)> {
        let Ok(json) = serde_json::from_str::<serde_json::Value>(output) else {
            return vec![];
        };
        let Some(deps) = json.get("dependencies").and_then(|d| d.as_object()) else {
            return vec![];
        };
        deps.iter()
            .filter_map(|(name, info)| {
                let version = info.get("version")?.as_str()?.to_string();
                if version.is_empty() {
                    None
                } else {
                    Some((name.clone(), version))
                }
            })
            .collect()
    }

    /// Parse upstream releases from the npm registry API response.
    ///
    /// Reads `dist-tags.latest` as the primary release. If `include_prereleases`
    /// is enabled, also emits entries for dist-tags `next`, `beta`, `alpha`,
    /// `rc`, `canary` — deduplicated against the `latest` version.
    pub(crate) fn parse_registry_response(
        &self,
        json: &serde_json::Value,
        package_identifier: &str,
    ) -> Vec<UpstreamRelease> {
        let dist_tags = match json.get("dist-tags").and_then(|d| d.as_object()) {
            Some(t) => t,
            None => return vec![],
        };

        let time_map = json.get("time").and_then(|t| t.as_object());

        let mut releases = Vec::new();
        let mut seen_versions = HashSet::new();

        // Always emit `latest`.
        if let Some(latest_version) = dist_tags.get("latest").and_then(|v| v.as_str()) {
            seen_versions.insert(latest_version.to_string());
            let published_at = time_map
                .and_then(|t| t.get(latest_version))
                .and_then(|v| v.as_str())
                .and_then(|s| {
                    OffsetDateTime::parse(s, &Rfc3339)
                        .inspect_err(|e| {
                            tracing::warn!(
                                package = %package_identifier,
                                version = %latest_version,
                                error = %e,
                                "failed to parse published_at for latest"
                            );
                        })
                        .ok()
                });

            releases.push({
                let mut r = UpstreamRelease::new(
                    Version::new(latest_version),
                    latest_version.to_string(),
                    false,
                    npm_release_url(package_identifier, latest_version),
                );
                r.published_at = published_at;
                r
            });
        }

        // Emit pre-release dist-tags if configured.
        if self.config.include_prereleases {
            for tag in PRERELEASE_DIST_TAGS {
                let Some(version) = dist_tags.get(*tag).and_then(|v| v.as_str()) else {
                    continue;
                };
                if !seen_versions.insert(version.to_string()) {
                    // Already emitted (matches `latest`).
                    continue;
                }

                let published_at = time_map
                    .and_then(|t| t.get(version))
                    .and_then(|v| v.as_str())
                    .and_then(|s| {
                        OffsetDateTime::parse(s, &Rfc3339)
                            .inspect_err(|e| {
                                tracing::warn!(
                                    package = %package_identifier,
                                    version = %version,
                                    tag = %tag,
                                    error = %e,
                                    "failed to parse published_at for pre-release"
                                );
                            })
                            .ok()
                    });

                releases.push({
                    let mut r = UpstreamRelease::new(
                        Version::new(version),
                        version.to_string(),
                        true,
                        npm_release_url(package_identifier, version),
                    );
                    r.published_at = published_at;
                    r
                });
            }
        }

        releases
    }
}

declare_plugin!(NpmPlugin, NpmConfig, "package_manager_npm", {
    display_name: "npm",
    family: PluginFamily::Software,
    config_model: ConfigModel::PluginConfig,
    host_requirements: HostRequirements::POSIX,
    config_test: [ConfigTestKind::Connectivity, ConfigTestKind::VersionDetection],
    roles: [
        Discoverer,
        VersionDetector,
        ReleaseFetcher { host_requirements: HostRequirements::CONTROLLER_ONLY },
        UpdateExecutor,
    ],
    sudo: NpmPlugin::required_sudo_commands,
});

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions use assert!(result.is_ok()) pattern"
    )]
    use super::*;
    use uptrakit_plugin_infrastructure_core::testing::{
        FixedOutputExecutor, test_runtime, test_runtime_with_executor,
    };
    use uptrakit_plugin_infrastructure_core::{PluginCapability, PluginMeta};

    // ── validate_identifier ───────────────────────────────────────────────────

    #[test]
    fn validate_identifier_plain_simple() {
        assert!(validate_identifier("n8n").is_ok());
        assert!(validate_identifier("typescript").is_ok());
        assert!(validate_identifier("pm2").is_ok());
    }

    #[test]
    fn validate_identifier_plain_with_allowed_chars() {
        assert!(validate_identifier("my-package").is_ok());
        assert!(validate_identifier("my.package").is_ok());
        assert!(validate_identifier("my_package").is_ok());
    }

    #[test]
    fn validate_identifier_plain_starts_with_digit() {
        assert!(validate_identifier("2check").is_ok());
    }

    #[test]
    fn validate_identifier_scoped_valid() {
        assert!(validate_identifier("@angular/cli").is_ok());
        assert!(validate_identifier("@nestjs/cli").is_ok());
        assert!(validate_identifier("@scope/name").is_ok());
    }

    #[test]
    fn validate_identifier_empty_fails() {
        let err = validate_identifier("").expect_err("should fail");
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn validate_identifier_uppercase_fails() {
        assert!(validate_identifier("MyPackage").is_err());
    }

    #[test]
    fn validate_identifier_space_fails() {
        assert!(validate_identifier("my package").is_err());
    }

    #[test]
    fn validate_identifier_path_traversal_fails() {
        assert!(validate_identifier("foo..bar").is_err());
    }

    #[test]
    fn validate_identifier_leading_dot_fails() {
        assert!(validate_identifier(".foo").is_err());
    }

    #[test]
    fn validate_identifier_leading_underscore_fails() {
        // npm doesn't allow leading `_` in package names
        assert!(validate_identifier("_foo").is_err());
    }

    #[test]
    fn validate_identifier_scoped_missing_slash_fails() {
        assert!(validate_identifier("@scope-only").is_err());
    }

    #[test]
    fn validate_identifier_scoped_empty_name_fails() {
        assert!(validate_identifier("@scope/").is_err());
    }

    #[test]
    fn validate_identifier_scoped_empty_scope_fails() {
        assert!(validate_identifier("@/name").is_err());
    }

    #[test]
    fn validate_identifier_scoped_path_traversal_fails() {
        assert!(validate_identifier("@scope/foo..bar").is_err());
    }

    #[test]
    fn validate_identifier_too_long_fails() {
        let long = "a".repeat(215);
        assert!(validate_identifier(&long).is_err());
    }

    // ── npm_registry_url ──────────────────────────────────────────────────────

    #[test]
    fn registry_url_plain_package() {
        assert_eq!(
            npm_registry_url("n8n", None),
            "https://registry.npmjs.org/n8n"
        );
    }

    #[test]
    fn registry_url_scoped_package() {
        assert_eq!(
            npm_registry_url("@angular/cli", None),
            "https://registry.npmjs.org/@angular%2Fcli"
        );
    }

    #[test]
    fn registry_url_scoped_nested_slash_only_encodes_first() {
        // Only the first `/` after the scope is encoded.
        assert_eq!(
            npm_registry_url("@scope/name", None),
            "https://registry.npmjs.org/@scope%2Fname"
        );
    }

    #[test]
    fn registry_url_custom_base() {
        assert_eq!(
            npm_registry_url("lodash", Some("https://my.registry.example.com")),
            "https://my.registry.example.com/lodash"
        );
        assert_eq!(
            npm_registry_url("@scope/pkg", Some("https://my.registry.example.com/")),
            "https://my.registry.example.com/@scope%2Fpkg"
        );
    }

    // ── npm_release_url ───────────────────────────────────────────────────────

    #[test]
    fn release_url_plain() {
        assert_eq!(
            npm_release_url("n8n", "1.2.3"),
            "https://www.npmjs.com/package/n8n/v/1.2.3"
        );
    }

    #[test]
    fn release_url_scoped() {
        assert_eq!(
            npm_release_url("@angular/cli", "17.0.0"),
            "https://www.npmjs.com/package/@angular/cli/v/17.0.0"
        );
    }

    // ── parse_npm_list_version ────────────────────────────────────────────────

    #[test]
    fn parse_npm_list_version_found() {
        let json = r#"{"dependencies":{"n8n":{"version":"1.18.0"}}}"#;
        assert_eq!(
            NpmPlugin::parse_npm_list_version(json, "n8n"),
            Some("1.18.0".to_string())
        );
    }

    #[test]
    fn parse_npm_list_version_not_found() {
        let json = r#"{"dependencies":{}}"#;
        assert_eq!(NpmPlugin::parse_npm_list_version(json, "n8n"), None);
    }

    #[test]
    fn parse_npm_list_version_no_dependencies_key() {
        let json = r#"{}"#;
        assert_eq!(NpmPlugin::parse_npm_list_version(json, "n8n"), None);
    }

    #[test]
    fn parse_npm_list_version_malformed_json() {
        assert_eq!(NpmPlugin::parse_npm_list_version("not json", "n8n"), None);
    }

    // ── parse_npm_list_all ────────────────────────────────────────────────────

    #[test]
    fn parse_npm_list_all_multiple() {
        let json = r#"{"dependencies":{"n8n":{"version":"1.18.0"},"pm2":{"version":"5.3.0"}}}"#;
        let result = NpmPlugin::parse_npm_list_all(json);
        assert_eq!(result.len(), 2);
        let n8n = result.iter().find(|(n, _)| n == "n8n").expect("n8n");
        assert_eq!(n8n.1, "1.18.0");
        let pm2 = result.iter().find(|(n, _)| n == "pm2").expect("pm2");
        assert_eq!(pm2.1, "5.3.0");
    }

    #[test]
    fn parse_npm_list_all_empty_deps() {
        let json = r#"{"dependencies":{}}"#;
        assert!(NpmPlugin::parse_npm_list_all(json).is_empty());
    }

    #[test]
    fn parse_npm_list_all_no_deps_key() {
        let json = r#"{}"#;
        assert!(NpmPlugin::parse_npm_list_all(json).is_empty());
    }

    // ── parse_registry_response ───────────────────────────────────────────────

    #[test]
    fn parse_registry_response_latest_only() {
        let config = NpmConfig::default();
        let plugin = NpmPlugin::new(config, test_runtime()).expect("create");
        let json = serde_json::json!({
            "dist-tags": { "latest": "1.18.0" },
            "time": { "1.18.0": "2024-01-15T10:00:00.000Z" }
        });
        let releases = plugin.parse_registry_response(&json, "n8n");
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].tag, "1.18.0");
        assert!(!releases[0].is_prerelease);
        assert!(releases[0].published_at.is_some());
    }

    #[test]
    fn parse_registry_response_with_prereleases() {
        let config = NpmConfig {
            include_prereleases: true,
            registry_url: None,
        };
        let plugin = NpmPlugin::new(config, test_runtime()).expect("create");
        let json = serde_json::json!({
            "dist-tags": {
                "latest": "1.18.0",
                "next": "1.19.0-beta.1",
                "beta": "1.19.0-beta.1"
            },
            "time": {}
        });
        let releases = plugin.parse_registry_response(&json, "n8n");
        // latest + next (beta deduped against next's version)
        assert_eq!(releases.len(), 2);
        assert!(
            releases
                .iter()
                .any(|r| r.tag == "1.18.0" && !r.is_prerelease)
        );
        assert!(
            releases
                .iter()
                .any(|r| r.tag == "1.19.0-beta.1" && r.is_prerelease)
        );
    }

    #[test]
    fn parse_registry_response_prerelease_same_as_latest_deduped() {
        let config = NpmConfig {
            include_prereleases: true,
            registry_url: None,
        };
        let plugin = NpmPlugin::new(config, test_runtime()).expect("create");
        let json = serde_json::json!({
            "dist-tags": {
                "latest": "1.18.0",
                "next": "1.18.0"
            },
            "time": {}
        });
        let releases = plugin.parse_registry_response(&json, "n8n");
        // next version equals latest → deduplicated
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].tag, "1.18.0");
    }

    #[test]
    fn parse_registry_response_no_dist_tags() {
        let config = NpmConfig::default();
        let plugin = NpmPlugin::new(config, test_runtime()).expect("create");
        let json = serde_json::json!({});
        let releases = plugin.parse_registry_response(&json, "n8n");
        assert!(releases.is_empty());
    }

    // ── plugin_type_id ──────────────────────────────────────────────────────

    #[test]
    fn npm_plugin_type_id() {
        let plugin = NpmPlugin::new(NpmConfig::default(), test_runtime()).expect("create");
        assert_eq!(plugin.plugin_type_id().as_str(), "package_manager_npm");
    }

    // ── validate_version ────────────────────────────────────────────────────

    #[test]
    fn validate_version_semver() {
        assert!(validate_version("1.18.0").is_ok());
        assert!(validate_version("0.0.1-beta.1").is_ok());
        assert!(validate_version("2.0.0-rc.1+build.123").is_ok());
    }

    #[test]
    fn validate_version_empty_fails() {
        let err = validate_version("").expect_err("should fail");
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn validate_version_too_long_fails() {
        let long = "1".repeat(257);
        assert!(validate_version(&long).is_err());
    }

    #[test]
    fn validate_version_file_protocol_fails() {
        assert!(validate_version("file:../malicious").is_err());
    }

    #[test]
    fn validate_version_git_protocol_fails() {
        assert!(validate_version("git+https://attacker.com").is_err());
    }

    #[test]
    fn validate_version_http_protocol_fails() {
        assert!(validate_version("http://evil.com").is_err());
        assert!(validate_version("https://evil.com").is_err());
    }

    #[test]
    fn validate_version_space_fails() {
        assert!(validate_version("1.0 --flag").is_err());
    }

    #[test]
    fn validate_version_at_sign_fails() {
        assert!(validate_version("1.0@latest").is_err());
    }

    #[test]
    fn validate_version_max_length_ok() {
        let v = "1".repeat(256);
        assert!(validate_version(&v).is_ok());
    }

    // ── descriptor capabilities ────────────────────────────────────────────────

    #[test]
    fn descriptor_capabilities() {
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::DiscoverLocalSoftware)
        );
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::DetectHostCompatibility)
        );
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::VersionDetection)
        );
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::ReleaseFetching)
        );
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::UpdateExecution)
        );
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::ConfigTest)
        );
        assert!(
            !DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::RefreshPackageIndex)
        );
    }

    #[test]
    fn descriptor_has_expected_roles() {
        assert!(DESCRIPTOR.roles.discoverer.is_some());
        assert!(DESCRIPTOR.roles.version_detector.is_some());
        assert!(DESCRIPTOR.roles.release_fetcher.is_some());
        assert!(DESCRIPTOR.roles.update_executor.is_some());
        assert!(DESCRIPTOR.roles.package_indexer.is_none());
        assert!(DESCRIPTOR.roles.lifecycle_hook.is_none());
    }

    #[test]
    fn descriptor_release_fetcher_is_controller_only() {
        let slot = DESCRIPTOR.roles.release_fetcher.as_ref().unwrap();
        assert!(slot.host_requirements.controller_only);
    }

    // ── required_sudo_commands ────────────────────────────────────────────────

    #[test]
    fn npm_plugin_required_sudo_commands() {
        let entries = NpmPlugin::required_sudo_commands(&serde_json::json!({}));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command, "npm");
        assert!(!entries[0].needs_setenv);
        assert!(entries[0].helper_script.is_none());
        assert_eq!(entries[0].args_suffix.as_deref(), Some("install -g *"));
    }

    // ── detect_host_compatibility (via discovery module, tested here for convenience)

    #[tokio::test]
    async fn detect_host_compatibility_compatible_when_which_exits_zero() {
        use uptrakit_plugin_infrastructure_core::Discoverer;
        let plugin = NpmPlugin::new(
            NpmConfig::default(),
            test_runtime_with_executor(FixedOutputExecutor::new("", 0)),
        )
        .expect("create");
        let result = plugin.detect_host_compatibility().await.expect("ok");
        assert_eq!(
            result,
            uptrakit_plugin_infrastructure_core::HostCompatibility::Compatible
        );
    }

    #[tokio::test]
    async fn detect_host_compatibility_incompatible_when_which_exits_nonzero() {
        use uptrakit_plugin_infrastructure_core::Discoverer;
        let plugin = NpmPlugin::new(
            NpmConfig::default(),
            test_runtime_with_executor(FixedOutputExecutor::new("", 1)),
        )
        .expect("create");
        let result = plugin.detect_host_compatibility().await.expect("ok");
        match result {
            uptrakit_plugin_infrastructure_core::HostCompatibility::Incompatible(msg) => {
                assert_eq!(msg, "npm not found");
            }
            uptrakit_plugin_infrastructure_core::HostCompatibility::Compatible => {
                panic!("expected Incompatible")
            }
            _ => panic!("unexpected HostCompatibility variant"),
        }
    }
}
