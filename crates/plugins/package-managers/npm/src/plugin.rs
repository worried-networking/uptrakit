use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uptrakit_plugin_infrastructure_core::command::{CommandExecutor, CommandSpec, send_output};
use uptrakit_plugin_infrastructure_core::mpsc;
use uptrakit_plugin_infrastructure_core::{
    DiscoveredSoftware, HostCompatibility, OutputStreamType, Plugin, PluginCapability, PluginError,
    PluginType, ReleaseInfo, Result, SudoCommandEntry, UpdateOutputLine, UpstreamRelease, Version,
};

use crate::config::NpmConfig;

/// npm packages that are package-manager infrastructure, not tracked applications.
///
/// These are filtered out during autodiscovery so that tooling does not appear
/// as managed software items alongside real applications.
pub const SYSTEM_NPM_PACKAGES: &[&str] = &["npm", "n", "nvm", "yarn", "pnpm", "corepack"];

/// Pre-release dist-tags that may be emitted when `include_prereleases` is true.
const PRERELEASE_DIST_TAGS: &[&str] = &["next", "beta", "alpha", "rc", "canary"];

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
pub fn validate_identifier(value: &str) -> std::result::Result<(), String> {
    if value.is_empty() {
        return Err("package_identifier must not be empty".to_string());
    }

    if let Some(without_at) = value.strip_prefix('@') {
        // Scoped package: @scope/name
        let slash = without_at.find('/').ok_or_else(|| {
            "scoped package_identifier must contain a '/' after the scope".to_string()
        })?;
        let scope = &without_at[..slash];
        let name = &without_at[slash + 1..];
        validate_npm_name_part(scope, "scope")?;
        validate_npm_name_part(name, "name")?;
        // Total length including `@` and `/`.
        if value.len() > 214 {
            return Err("package_identifier must not exceed 214 characters".to_string());
        }
    } else {
        validate_npm_name_part(value, "package")?;
    }

    Ok(())
}

/// Validate a single npm name component (scope or package name).
fn validate_npm_name_part(part: &str, role: &str) -> std::result::Result<(), String> {
    if part.is_empty() {
        return Err(format!("package_identifier {role} must not be empty"));
    }
    if part.len() > 214 {
        return Err(format!(
            "package_identifier {role} must not exceed 214 characters"
        ));
    }

    let first = part.chars().next().unwrap();
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(format!(
            "package_identifier {role} must start with a lowercase letter or digit, found '{first}'"
        ));
    }

    for ch in part.chars() {
        if !ch.is_ascii_lowercase()
            && !ch.is_ascii_digit()
            && !matches!(ch, '-' | '.' | '_')
        {
            return Err(format!(
                "package_identifier {role} contains invalid character: '{ch}'"
            ));
        }
    }

    if part.contains("..") {
        return Err(format!(
            "package_identifier {role} must not contain '..'"
        ));
    }

    Ok(())
}

/// Build the npm registry URL for a package identifier.
///
/// Scoped packages (`@scope/name`) are URL-encoded: the `/` is encoded as `%2F`.
pub fn npm_registry_url(package_identifier: &str) -> String {
    if let Some(without_at) = package_identifier.strip_prefix('@') {
        // Encode `@scope/name` as `@scope%2Fname`.
        let encoded = without_at.replacen('/', "%2F", 1);
        format!("https://registry.npmjs.org/@{encoded}")
    } else {
        format!("https://registry.npmjs.org/{package_identifier}")
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
    config: NpmConfig,
    executor: Arc<dyn CommandExecutor>,
    client: reqwest::Client,
}

impl NpmPlugin {
    /// Create a new npm plugin with the given configuration.
    pub fn new(config: NpmConfig, executor: Arc<dyn CommandExecutor>) -> Result<Self> {
        config
            .validate()
            .map_err(|e| report!(PluginError::Configuration(e.to_string())))?;

        let client = reqwest::Client::builder()
            .user_agent(concat!(
                "uptrakit-plugin-package-manager-npm/",
                env!("CARGO_PKG_VERSION")
            ))
            .build()
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "failed to build HTTP client: {e}"
                )))
            })?;

        Ok(Self {
            config,
            executor,
            client,
        })
    }

    fn require_package_identifier(&self, package_identifier: &str) -> Result<()> {
        validate_identifier(package_identifier)
            .map_err(|e| report!(PluginError::Configuration(e)))
    }

    /// Parse installed version from `npm list -g <package> --depth=0 --json` output.
    ///
    /// Expected JSON shape:
    /// ```json
    /// { "dependencies": { "<package>": { "version": "X.Y.Z" } } }
    /// ```
    fn parse_npm_list_version(output: &str, package: &str) -> Option<String> {
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
    fn parse_npm_list_all(output: &str) -> Vec<(String, String)> {
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
    fn parse_registry_response(
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
        let mut seen_versions = std::collections::HashSet::new();

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

            releases.push(UpstreamRelease {
                version: Version::new(latest_version),
                tag: latest_version.to_string(),
                is_prerelease: false,
                release_url: npm_release_url(package_identifier, latest_version),
                release_notes: None,
                published_at,
                assets: vec![],
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

                releases.push(UpstreamRelease {
                    version: Version::new(version),
                    tag: version.to_string(),
                    is_prerelease: true,
                    release_url: npm_release_url(package_identifier, version),
                    release_notes: None,
                    published_at,
                    assets: vec![],
                });
            }
        }

        releases
    }
}

#[async_trait]
impl Plugin for NpmPlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::PackageManagerNpm
    }

    fn capabilities(&self) -> &'static [PluginCapability] {
        &[
            PluginCapability::DiscoverLocalSoftware,
            PluginCapability::DetectHostCompatibility,
            PluginCapability::ControllerSideFetchReleases,
        ]
    }

    fn required_sudo_commands(&self) -> Vec<SudoCommandEntry> {
        vec![SudoCommandEntry {
            command: "npm".into(),
            explanation: "Global npm package installation requires root".into(),
            helper_script: None,
        }]
    }

    async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
        let result = self
            .executor
            .execute_quiet(&CommandSpec::exec("which", ["npm".to_string()]))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "which npm failed: {e}"
                )))
            })?;

        if result.exit_code == 0 {
            Ok(HostCompatibility::Compatible)
        } else {
            Ok(HostCompatibility::Incompatible("npm not found".to_string()))
        }
    }

    async fn detect_installed_version(&self, package_identifier: &str) -> Result<Option<Version>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "detecting npm installed version");

        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "npm",
                [
                    "list".to_string(),
                    "-g".to_string(),
                    package_identifier.to_string(),
                    "--depth=0".to_string(),
                    "--json".to_string(),
                ],
            ))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "npm list failed: {e}"
                )))
            })?;

        // npm exits non-zero when a package is not found; treat that as not installed.
        if cmd_output.exit_code != 0 {
            tracing::debug!(
                package = %package_identifier,
                exit_code = cmd_output.exit_code,
                "npm list returned non-zero; package not installed"
            );
            return Ok(None);
        }

        let version =
            Self::parse_npm_list_version(&cmd_output.output, package_identifier);
        tracing::debug!(package = %package_identifier, version = ?version, "npm installed version");
        Ok(version.map(|v| Version::new(&v)))
    }

    async fn fetch_releases(&self, package_identifier: &str) -> Result<Vec<UpstreamRelease>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "fetching npm releases from registry");

        let url = npm_registry_url(package_identifier);
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "npm registry request failed: {e}"
                )))
            })?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            tracing::debug!(package = %package_identifier, "package not found in npm registry");
            return Ok(vec![]);
        }

        if !response.status().is_success() {
            bail!(PluginError::PluginInternal(format!(
                "npm registry returned HTTP {}",
                response.status()
            )));
        }

        let json: serde_json::Value = response.json().await.map_err(|e| {
            report!(PluginError::PluginInternal(format!(
                "failed to parse npm registry response: {e}"
            )))
        })?;

        let releases = self.parse_registry_response(&json, package_identifier);
        tracing::debug!(
            package = %package_identifier,
            count = releases.len(),
            "npm releases fetched"
        );
        Ok(releases)
    }

    async fn execute_update(
        &self,
        package_identifier: &str,
        to_version: &str,
        _release_info: Option<&ReleaseInfo>,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<String> {
        self.require_package_identifier(package_identifier)?;

        let pkg_version = format!("{package_identifier}@{to_version}");
        let args = vec!["install".to_string(), "-g".to_string(), pkg_version];

        tracing::debug!(
            package = %package_identifier,
            version = %to_version,
            "running npm install -g"
        );

        let display_args = std::iter::once("npm")
            .chain(args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        send_output(
            output_tx,
            &format!("Running: {display_args}"),
            OutputStreamType::Stdout,
        )
        .await;
        let mut output = format!("Running: {display_args}\n");

        let cmd_output = self
            .executor
            .execute(&CommandSpec::exec("npm", args).privileged(), output_tx)
            .await
            .map_err(|e| report!(PluginError::InstallFailed(e.to_string())))?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::InstallFailed(format!(
                "npm install -g failed with exit code {}",
                cmd_output.exit_code
            )));
        }

        output.push_str(&cmd_output.output);
        Ok(output)
    }

    async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> {
        tracing::info!("discovering globally installed npm packages");

        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "npm",
                [
                    "list".to_string(),
                    "-g".to_string(),
                    "--depth=0".to_string(),
                    "--json".to_string(),
                ],
            ))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "npm list -g failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::CommandFailed(cmd_output.exit_code));
        }

        let all_packages = Self::parse_npm_list_all(&cmd_output.output);

        let packages: Vec<DiscoveredSoftware> = all_packages
            .into_iter()
            .filter(|(name, _)| !SYSTEM_NPM_PACKAGES.contains(&name.as_str()))
            .map(|(name, version)| DiscoveredSoftware {
                package_identifier: name.clone(),
                name,
                installed_version: version,
                targets: vec![],
                extra: None,
            })
            .collect();

        tracing::debug!(count = packages.len(), "npm software discovery complete");
        Ok(packages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_core::{CommandOutput, LocalCommandExecutor};

    fn test_executor() -> Arc<dyn CommandExecutor> {
        Arc::new(LocalCommandExecutor)
    }

    struct FixedOutputExecutor {
        output: String,
        exit_code: i32,
    }

    impl FixedOutputExecutor {
        fn with_output(output: impl Into<String>, exit_code: i32) -> Arc<dyn CommandExecutor> {
            Arc::new(Self {
                output: output.into(),
                exit_code,
            })
        }
    }

    #[async_trait]
    impl CommandExecutor for FixedOutputExecutor {
        async fn execute(
            &self,
            _spec: &CommandSpec,
            _output_tx: &tokio::sync::mpsc::Sender<UpdateOutputLine>,
        ) -> uptrakit_command::Result<CommandOutput> {
            Ok(CommandOutput {
                output: self.output.clone(),
                exit_code: self.exit_code,
            })
        }

        async fn execute_quiet(
            &self,
            _spec: &CommandSpec,
        ) -> uptrakit_command::Result<CommandOutput> {
            Ok(CommandOutput {
                output: self.output.clone(),
                exit_code: self.exit_code,
            })
        }
    }

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
        assert!(err.contains("empty"));
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
            npm_registry_url("n8n"),
            "https://registry.npmjs.org/n8n"
        );
    }

    #[test]
    fn registry_url_scoped_package() {
        assert_eq!(
            npm_registry_url("@angular/cli"),
            "https://registry.npmjs.org/@angular%2Fcli"
        );
    }

    #[test]
    fn registry_url_scoped_nested_slash_only_encodes_first() {
        // Only the first `/` after the scope is encoded.
        assert_eq!(
            npm_registry_url("@scope/name"),
            "https://registry.npmjs.org/@scope%2Fname"
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
        let plugin = NpmPlugin::new(config, test_executor()).expect("create");
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
        let config = NpmConfig { include_prereleases: true };
        let plugin = NpmPlugin::new(config, test_executor()).expect("create");
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
        assert!(releases.iter().any(|r| r.tag == "1.18.0" && !r.is_prerelease));
        assert!(releases.iter().any(|r| r.tag == "1.19.0-beta.1" && r.is_prerelease));
    }

    #[test]
    fn parse_registry_response_prerelease_same_as_latest_deduped() {
        let config = NpmConfig { include_prereleases: true };
        let plugin = NpmPlugin::new(config, test_executor()).expect("create");
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
        let plugin = NpmPlugin::new(config, test_executor()).expect("create");
        let json = serde_json::json!({});
        let releases = plugin.parse_registry_response(&json, "n8n");
        assert!(releases.is_empty());
    }

    // ── capabilities ──────────────────────────────────────────────────────────

    #[test]
    fn npm_plugin_capabilities() {
        let plugin = NpmPlugin::new(NpmConfig::default(), test_executor()).expect("create");
        assert!(plugin.has_capability(PluginCapability::DiscoverLocalSoftware));
        assert!(plugin.has_capability(PluginCapability::DetectHostCompatibility));
        assert!(plugin.has_capability(PluginCapability::ControllerSideFetchReleases));
        assert!(!plugin.has_capability(PluginCapability::RefreshPackageIndex));
        assert_eq!(plugin.capabilities().len(), 3);
    }

    // ── required_sudo_commands ────────────────────────────────────────────────

    #[test]
    fn npm_plugin_required_sudo_commands() {
        let plugin = NpmPlugin::new(NpmConfig::default(), test_executor()).expect("create");
        let entries = plugin.required_sudo_commands();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command, "npm");
        assert!(!entries[0].explanation.is_empty());
        assert!(entries[0].helper_script.is_none());
    }

    // ── detect_host_compatibility ─────────────────────────────────────────────

    #[tokio::test]
    async fn detect_host_compatibility_compatible_when_which_exits_zero() {
        let plugin = NpmPlugin::new(
            NpmConfig::default(),
            FixedOutputExecutor::with_output("", 0),
        )
        .expect("create");
        let result = plugin.detect_host_compatibility().await.expect("ok");
        assert_eq!(result, HostCompatibility::Compatible);
    }

    #[tokio::test]
    async fn detect_host_compatibility_incompatible_when_which_exits_nonzero() {
        let plugin = NpmPlugin::new(
            NpmConfig::default(),
            FixedOutputExecutor::with_output("", 1),
        )
        .expect("create");
        let result = plugin.detect_host_compatibility().await.expect("ok");
        match result {
            HostCompatibility::Incompatible(msg) => {
                assert_eq!(msg, "npm not found");
            }
            HostCompatibility::Compatible => panic!("expected Incompatible"),
        }
    }

    // ── detect_installed_version ──────────────────────────────────────────────

    #[tokio::test]
    async fn detect_installed_version_found() {
        let json = r#"{"dependencies":{"n8n":{"version":"1.18.0"}}}"#;
        let plugin =
            NpmPlugin::new(NpmConfig::default(), FixedOutputExecutor::with_output(json, 0)).expect("create");
        let result = plugin.detect_installed_version("n8n").await.expect("ok");
        assert_eq!(result, Some(Version::new("1.18.0")));
    }

    #[tokio::test]
    async fn detect_installed_version_not_installed() {
        let plugin = NpmPlugin::new(
            NpmConfig::default(),
            FixedOutputExecutor::with_output("", 1),
        )
        .expect("create");
        let result = plugin.detect_installed_version("n8n").await.expect("ok");
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn detect_installed_version_empty_identifier_fails() {
        let plugin = NpmPlugin::new(NpmConfig::default(), test_executor()).expect("create");
        let result = plugin.detect_installed_version("").await;
        assert!(result.is_err());
    }

    // ── plugin_type ───────────────────────────────────────────────────────────

    #[test]
    fn npm_plugin_type() {
        let plugin = NpmPlugin::new(NpmConfig::default(), test_executor()).expect("create");
        assert_eq!(plugin.plugin_type(), PluginType::PackageManagerNpm);
    }
}
