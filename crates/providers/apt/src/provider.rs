use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_provider_core::command::{CommandExecutor, CommandSpec, send_output};
use uptrakit_provider_core::mpsc;
use uptrakit_provider_core::{
    DiscoveredSoftware, OutputStreamType, Provider, ProviderCapability, ProviderError,
    ProviderType, ReleaseInfo, Result, UpdateOutputLine, UpstreamRelease, Version,
};

use crate::config::{AptConfig, AptDiscoveryFilter};

/// Validate a Debian APT package identifier.
///
/// Enforces Debian package naming rules from the Debian Policy Manual:
/// - Between 2 and 64 characters long.
/// - Must start with a lowercase letter or digit (`[a-z0-9]`).
/// - May only contain lowercase letters, digits, `+`, `-`, and `.`.
/// - Must not contain `..` (path traversal protection).
pub fn validate_identifier(value: &str) -> std::result::Result<(), String> {
    if value.is_empty() {
        return Err("package_identifier must not be empty".to_string());
    }
    if value.len() < 2 {
        return Err("package_identifier must be at least 2 characters long".to_string());
    }
    if value.len() > 64 {
        return Err("package_identifier must not exceed 64 characters".to_string());
    }

    // Must start with [a-z0-9].
    let Some(first) = value.chars().next() else {
        return Err("package_identifier must not be empty".to_string());
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(format!(
            "package_identifier must start with a lowercase letter or digit, found '{first}'"
        ));
    }

    // All characters must be in [a-z0-9+\-.].
    for ch in value.chars() {
        if !ch.is_ascii_lowercase()
            && !ch.is_ascii_digit()
            && !matches!(ch, '+' | '-' | '.')
        {
            return Err(format!(
                "package_identifier contains invalid character: '{ch}'"
            ));
        }
    }

    // No path traversal via '..'.
    if value.contains("..") {
        return Err("package_identifier must not contain '..'".to_string());
    }

    Ok(())
}

/// Provider for APT (Debian/Ubuntu package manager).
///
/// Supports installed version detection, package index refresh, autodiscovery,
/// and updates for Debian packages managed by `apt-get`.
///
/// The `package_identifier` in `SoftwareItem` is the Debian package name
/// (e.g., `nginx`, `python3`, `apt-utils`).
pub struct AptProvider {
    config: AptConfig,
    executor: Arc<dyn CommandExecutor>,
}

impl AptProvider {
    /// Create a new APT provider with the given configuration.
    pub fn new(config: AptConfig, executor: Arc<dyn CommandExecutor>) -> Result<Self> {
        config
            .validate()
            .map_err(|e| report!(ProviderError::Configuration(e.to_string())))?;
        Ok(Self { config, executor })
    }

    /// Parse `dpkg-query --show --showformat=${Package}\t${Version}\n` output.
    ///
    /// Each line is a tab-separated `package\tversion` pair. Lines with an
    /// empty version are skipped.
    fn parse_dpkg_output(output: &str) -> Vec<(String, String)> {
        output
            .lines()
            .filter_map(|line| {
                let mut parts = line.splitn(2, '\t');
                let name = parts.next()?.trim();
                let version = parts.next()?.trim();
                if name.is_empty() || version.is_empty() {
                    None
                } else {
                    Some((name.to_string(), version.to_string()))
                }
            })
            .collect()
    }

    /// Parse `apt-cache madison <package>` output.
    ///
    /// Each line has the format:
    /// `   <package> | <version> | <source>`
    ///
    /// Returns the version string from the first valid line (highest-priority
    /// candidate), or `None` if the output is empty or contains no parseable
    /// lines.
    fn parse_madison_output(output: &str) -> Option<String> {
        output.lines().find_map(|line| {
            let mut parts = line.splitn(3, '|');
            let _package = parts.next()?;
            let version = parts.next()?.trim();
            if version.is_empty() {
                None
            } else {
                Some(version.to_string())
            }
        })
    }

    fn require_package_identifier(&self, package_identifier: &str) -> Result<()> {
        validate_identifier(package_identifier)
            .map_err(|e| report!(ProviderError::Configuration(e)))
    }
}

#[async_trait]
impl Provider for AptProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::Apt
    }

    fn capabilities(&self) -> &'static [ProviderCapability] {
        &[
            ProviderCapability::DiscoverLocalSoftware,
            ProviderCapability::RefreshPackageIndex,
        ]
    }

    async fn refresh_package_index(&self) -> Result<()> {
        tracing::info!("refreshing APT package index");
        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "sudo",
                ["apt-get".to_string(), "update".to_string(), "-q".to_string()],
            ))
            .await
            .map_err(|e| {
                report!(ProviderError::ProviderInternal(format!(
                    "apt-get update failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            bail!(ProviderError::CommandFailed(cmd_output.exit_code));
        }

        tracing::info!("APT package index refreshed");
        Ok(())
    }

    async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> {
        tracing::info!("discovering APT-managed software");

        // Step 1: Query all installed packages from dpkg.
        let dpkg_output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "dpkg-query",
                [
                    "--show".to_string(),
                    "--showformat=${Package}\\t${Version}\\n".to_string(),
                ],
            ))
            .await
            .map_err(|e| {
                report!(ProviderError::ProviderInternal(format!(
                    "dpkg-query failed: {e}"
                )))
            })?;

        if dpkg_output.exit_code != 0 {
            bail!(ProviderError::CommandFailed(dpkg_output.exit_code));
        }

        let all_packages = Self::parse_dpkg_output(&dpkg_output.output);

        // Step 2: For the Manual filter, build a set of manually-installed packages.
        let manual_set: Option<HashSet<String>> = match self.config.discovery_filter {
            AptDiscoveryFilter::Manual => {
                let mark_output = self
                    .executor
                    .execute_quiet(&CommandSpec::exec(
                        "apt-mark",
                        ["showmanual".to_string()],
                    ))
                    .await
                    .map_err(|e| {
                        report!(ProviderError::ProviderInternal(format!(
                            "apt-mark showmanual failed: {e}"
                        )))
                    })?;

                if mark_output.exit_code != 0 {
                    bail!(ProviderError::CommandFailed(mark_output.exit_code));
                }

                let set: HashSet<String> = mark_output
                    .output
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect();
                Some(set)
            }
            AptDiscoveryFilter::All => None,
        };

        // Step 3: Filter by the manual set (if applicable) and build results.
        let packages: Vec<DiscoveredSoftware> = all_packages
            .into_iter()
            .filter(|(name, _)| {
                manual_set
                    .as_ref()
                    .is_none_or(|set| set.contains(name.as_str()))
            })
            .map(|(name, version)| DiscoveredSoftware {
                package_identifier: name.clone(),
                name,
                installed_version: version,
                extra: None,
            })
            .collect();

        tracing::debug!(count = packages.len(), "APT software discovery complete");
        Ok(packages)
    }

    async fn detect_installed_version(&self, package_identifier: &str) -> Result<Option<Version>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "detecting APT installed version");

        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "dpkg-query",
                [
                    "--show".to_string(),
                    "--showformat=${Version}\\n".to_string(),
                    package_identifier.to_string(),
                ],
            ))
            .await
            .map_err(|e| {
                report!(ProviderError::ProviderInternal(format!(
                    "dpkg-query failed: {e}"
                )))
            })?;

        match cmd_output.exit_code {
            0 => {
                let version = cmd_output.output.trim().to_string();
                if version.is_empty() {
                    return Ok(None);
                }
                tracing::debug!(version = %version, "APT installed version detected");
                Ok(Some(Version::new(&version)))
            }
            // Exit code 1 means the package was not found.
            1 => {
                tracing::debug!(
                    package = %package_identifier,
                    "package not found in dpkg database"
                );
                Ok(None)
            }
            code => bail!(ProviderError::CommandFailed(code)),
        }
    }

    async fn fetch_releases(&self, package_identifier: &str) -> Result<Vec<UpstreamRelease>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "fetching APT releases via apt-cache madison");

        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "apt-cache",
                ["madison".to_string(), package_identifier.to_string()],
            ))
            .await
            .map_err(|e| {
                report!(ProviderError::ProviderInternal(format!(
                    "apt-cache madison failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            bail!(ProviderError::CommandFailed(cmd_output.exit_code));
        }

        let Some(version_str) = Self::parse_madison_output(&cmd_output.output) else {
            // Package not found in any configured repository.
            return Ok(vec![]);
        };

        tracing::debug!(version = %version_str, "APT upstream version resolved");
        Ok(vec![UpstreamRelease {
            version: Version::new(&version_str),
            tag: version_str,
            is_prerelease: false,
            release_url: String::new(),
            release_notes: None,
            published_at: None,
            assets: vec![],
        }])
    }

    async fn execute_update(
        &self,
        package_identifier: &str,
        to_version: &str,
        _release_info: Option<&ReleaseInfo>,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<String> {
        self.require_package_identifier(package_identifier)?;

        let pkg_version = format!("{package_identifier}={to_version}");
        let args = vec![
            "apt-get".to_string(),
            "install".to_string(),
            "--yes".to_string(),
            "--no-install-recommends".to_string(),
            pkg_version,
        ];

        tracing::debug!(
            package = %package_identifier,
            version = %to_version,
            "running sudo apt-get install"
        );

        send_output(
            output_tx,
            &format!("Running: sudo {}", args.join(" ")),
            OutputStreamType::Stdout,
        )
        .await;
        let mut output = format!("Running: sudo {}\n", args.join(" "));

        let cmd_output = self
            .executor
            .execute(&CommandSpec::exec("sudo", args), output_tx)
            .await
            .map_err(|e| report!(ProviderError::InstallFailed(e.to_string())))?;

        if cmd_output.exit_code != 0 {
            bail!(ProviderError::InstallFailed(format!(
                "apt-get install failed with exit code {}",
                cmd_output.exit_code
            )));
        }

        output.push_str(&cmd_output.output);
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_provider_core::LocalCommandExecutor;

    fn test_executor() -> Arc<dyn CommandExecutor> {
        Arc::new(LocalCommandExecutor)
    }

    // ── validate_identifier ──────────────────────────────────────────────

    #[test]
    fn validate_identifier_valid_simple() {
        assert!(validate_identifier("nginx").is_ok());
    }

    #[test]
    fn validate_identifier_valid_with_dash() {
        assert!(validate_identifier("apt-utils").is_ok());
    }

    #[test]
    fn validate_identifier_valid_with_plus() {
        assert!(validate_identifier("g++").is_ok());
    }

    #[test]
    fn validate_identifier_valid_with_dot() {
        assert!(validate_identifier("python3.11").is_ok());
    }

    #[test]
    fn validate_identifier_valid_starts_with_digit() {
        assert!(validate_identifier("2ping").is_ok());
    }

    #[test]
    fn validate_identifier_valid_min_length() {
        assert!(validate_identifier("bc").is_ok());
    }

    #[test]
    fn validate_identifier_valid_max_length() {
        let name = "a".repeat(64);
        assert!(validate_identifier(&name).is_ok());
    }

    #[test]
    fn validate_identifier_empty_fails() {
        let err = validate_identifier("").expect_err("should fail");
        assert!(err.contains("empty"));
    }

    #[test]
    fn validate_identifier_too_short_fails() {
        let err = validate_identifier("a").expect_err("should fail");
        assert!(err.contains("2 characters"));
    }

    #[test]
    fn validate_identifier_too_long_fails() {
        let name = "a".repeat(65);
        let err = validate_identifier(&name).expect_err("should fail");
        assert!(err.contains("64"));
    }

    #[test]
    fn validate_identifier_uppercase_fails() {
        assert!(validate_identifier("Nginx").is_err());
    }

    #[test]
    fn validate_identifier_starts_with_dash_fails() {
        assert!(validate_identifier("-foo").is_err());
    }

    #[test]
    fn validate_identifier_starts_with_dot_fails() {
        assert!(validate_identifier(".foo").is_err());
    }

    #[test]
    fn validate_identifier_path_traversal_fails() {
        assert!(validate_identifier("foo..bar").is_err());
    }

    #[test]
    fn validate_identifier_slash_fails() {
        assert!(validate_identifier("foo/bar").is_err());
    }

    #[test]
    fn validate_identifier_whitespace_fails() {
        assert!(validate_identifier("foo bar").is_err());
    }

    // ── parse_madison_output ────────────────────────────────────────────

    #[test]
    fn parse_madison_output_single_entry() {
        let output = "   nginx | 1.24.0-2ubuntu7.3 | http://archive.ubuntu.com/ubuntu noble-updates/main amd64 Packages\n";
        assert_eq!(
            AptProvider::parse_madison_output(output),
            Some("1.24.0-2ubuntu7.3".to_string())
        );
    }

    #[test]
    fn parse_madison_output_multiple_entries_returns_first() {
        let output = concat!(
            "   nginx | 1.24.0-2ubuntu7.3 | http://archive.ubuntu.com/ubuntu noble-updates/main amd64 Packages\n",
            "   nginx | 1.18.0-6ubuntu14 | http://archive.ubuntu.com/ubuntu focal/main amd64 Packages\n",
        );
        assert_eq!(
            AptProvider::parse_madison_output(output),
            Some("1.24.0-2ubuntu7.3".to_string())
        );
    }

    #[test]
    fn parse_madison_output_malformed_line_skipped_gracefully() {
        let output = concat!(
            "no pipe here\n",
            "   nginx | 1.24.0 | source\n",
        );
        assert_eq!(
            AptProvider::parse_madison_output(output),
            Some("1.24.0".to_string())
        );
    }

    #[test]
    fn parse_madison_output_empty() {
        assert_eq!(AptProvider::parse_madison_output(""), None);
    }

    // ── parse_dpkg_output ───────────────────────────────────────────────

    #[test]
    fn parse_dpkg_output_normal() {
        let output = "nginx\t1.24.0-2ubuntu7.3\npython3\t3.11.0-5ubuntu2\n";
        let result = AptProvider::parse_dpkg_output(output);
        assert_eq!(result.len(), 2);
        assert_eq!(
            result[0],
            ("nginx".to_string(), "1.24.0-2ubuntu7.3".to_string())
        );
        assert_eq!(
            result[1],
            ("python3".to_string(), "3.11.0-5ubuntu2".to_string())
        );
    }

    #[test]
    fn parse_dpkg_output_empty_version_skipped() {
        let output = "nginx\t\npython3\t3.11.0\n";
        let result = AptProvider::parse_dpkg_output(output);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "python3");
        assert_eq!(result[0].1, "3.11.0");
    }

    #[test]
    fn parse_dpkg_output_empty_input() {
        let result = AptProvider::parse_dpkg_output("");
        assert!(result.is_empty());
    }

    // ── capabilities ────────────────────────────────────────────────────

    #[test]
    fn apt_provider_capabilities() {
        let provider =
            AptProvider::new(AptConfig::default(), test_executor()).expect("create provider");
        assert!(provider.has_capability(ProviderCapability::DiscoverLocalSoftware));
        assert!(provider.has_capability(ProviderCapability::RefreshPackageIndex));
        assert_eq!(provider.capabilities().len(), 2);
    }

    // ── empty identifier guards ──────────────────────────────────────────

    #[tokio::test]
    async fn detect_installed_version_empty_identifier_fails() {
        let provider =
            AptProvider::new(AptConfig::default(), test_executor()).expect("create provider");
        let result = provider.detect_installed_version("").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn fetch_releases_empty_identifier_fails() {
        let provider =
            AptProvider::new(AptConfig::default(), test_executor()).expect("create provider");
        let result = provider.fetch_releases("").await;
        assert!(result.is_err());
    }
}
