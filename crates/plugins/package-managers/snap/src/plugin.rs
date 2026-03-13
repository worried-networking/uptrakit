use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use time::OffsetDateTime;
use uptrakit_plugin_infrastructure_core::command::{CommandExecutor, CommandSpec, send_output};
use uptrakit_plugin_infrastructure_core::mpsc;
use uptrakit_plugin_infrastructure_core::{
    BatchDetectItem, BatchDetectResult, BatchUpdateItem, BatchUpdateResult, DiscoveredSoftware,
    DiscoveryPlugin, DiscoveryTarget, HostCompatibility, OutputStreamType, PluginCapability,
    PluginError, PluginRole, PluginType, ReleaseFetcherPlugin, ReleaseInfo, Result,
    SudoCommandEntry, UpdateExecutorPlugin, UpdateOutputLine, UpstreamRelease, Version,
    VersionDetectorPlugin,
};

use crate::config::SnapConfig;

/// System snaps that are excluded from discovery output.
///
/// These are infrastructure/base snaps managed by `snapd` itself and are not
/// user-installed software packages. Including them would create spurious
/// software items in Uptrakit.
const SYSTEM_SNAPS: &[&str] = &[
    "core", "core18", "core20", "core22", "core24", "core26", "snapd", "bare",
];

/// Internal: channel info parsed from `snap info` output.
struct SnapChannelInfo {
    version: String,
    published_at: Option<OffsetDateTime>,
}

/// Validate a Snap package identifier.
///
/// Enforces Snap package naming rules:
/// - Between 2 and 40 characters long.
/// - Charset: `[a-z0-9-]` only (lowercase letters, digits, hyphens; no dots).
/// - Must start and end with `[a-z0-9]` (not a hyphen).
/// - No consecutive hyphens (`--`).
pub fn validate_identifier(value: &str) -> std::result::Result<(), String> {
    if value.is_empty() {
        return Err("package_identifier must not be empty".to_string());
    }
    if value.len() < 2 {
        return Err("package_identifier must be at least 2 characters long".to_string());
    }
    if value.len() > 40 {
        return Err("package_identifier must not exceed 40 characters".to_string());
    }

    // Must start with [a-z0-9].
    let first = value.chars().next().unwrap_or('\0');
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(format!(
            "package_identifier must start with a lowercase letter or digit, found '{first}'"
        ));
    }

    // Must end with [a-z0-9].
    let last = value.chars().next_back().unwrap_or('\0');
    if !last.is_ascii_lowercase() && !last.is_ascii_digit() {
        return Err("package_identifier must end with a lowercase letter or digit".to_string());
    }

    // All characters must be in [a-z0-9-].
    for ch in value.chars() {
        if !ch.is_ascii_lowercase() && !ch.is_ascii_digit() && ch != '-' {
            return Err(format!(
                "package_identifier contains invalid character: '{ch}' (only lowercase letters, digits, and hyphens are allowed)"
            ));
        }
    }

    // No consecutive hyphens.
    if value.contains("--") {
        return Err("package_identifier must not contain consecutive hyphens ('--')".to_string());
    }

    Ok(())
}

/// Returns `true` if the risk segment of a channel string indicates a pre-release.
///
/// A channel is pre-release if its risk component is `"beta"` or `"edge"`.
/// Both bare risks (`"edge"`) and track-qualified risks (`"latest/edge"`) are handled.
fn is_prerelease_channel(channel: &str) -> bool {
    let risk = if let Some(slash) = channel.rfind('/') {
        &channel[slash + 1..]
    } else {
        channel
    };
    matches!(risk, "beta" | "edge")
}

/// Parse a single line from `snap list` output.
///
/// Format: `<Name>  <Version>  <Rev>  <Tracking>  <Publisher>  <Notes>`
///
/// Returns `(name, version)` or `None` if the line is a header or malformed.
/// The header line starts with `"Name"` and is skipped.
fn parse_snap_list_line(line: &str) -> Option<(String, String)> {
    let mut parts = line.split_whitespace();
    let name = parts.next()?;
    if name == "Name" {
        return None; // Skip header
    }
    let version = parts.next()?;
    Some((name.to_string(), version.to_string()))
}

/// Parse the `channels:` section from `snap info <name>` output.
///
/// Finds the `channels:` header line, then parses subsequent indented lines of the form:
/// `  <channel>: <version> <date> (<rev>) <size> <notes>`
///
/// Lines containing `↑` (same-as-above indicator) are skipped rather than resolved upward.
///
/// Returns a map of channel name → [`SnapChannelInfo`].
fn parse_snap_info_channels(output: &str) -> HashMap<String, SnapChannelInfo> {
    let mut result = HashMap::new();
    let mut in_channels = false;

    for line in output.lines() {
        // Detect the "channels:" header (may be indented or unindented).
        if line.trim() == "channels:" {
            in_channels = true;
            continue;
        }

        if !in_channels {
            continue;
        }

        // Channel lines start with at least two spaces.
        if !line.starts_with(' ') && !line.starts_with('\t') {
            break;
        }

        let line = line.trim_start();
        // Format: `<channel>: <rest>`
        let Some(colon_pos) = line.find(':') else {
            continue;
        };

        let channel_name = line[..colon_pos].trim();
        if channel_name.is_empty() {
            continue;
        }

        let rest = line[colon_pos + 1..].trim();

        // Skip ↑ entries (same version as the channel above).
        if rest.starts_with('↑') || rest == "↑" {
            continue;
        }

        // Parse: `<version> <date> (<rev>) <size> <notes>`
        let mut parts = rest.split_whitespace();
        let Some(version) = parts.next() else {
            continue;
        };
        if version.is_empty() {
            continue;
        }

        // Try to parse the date (second token, format "YYYY-MM-DD").
        let published_at = parts.next().and_then(parse_snap_date);

        result.insert(
            channel_name.to_string(),
            SnapChannelInfo {
                version: version.to_string(),
                published_at,
            },
        );
    }

    result
}

/// Parse a snap date string (`YYYY-MM-DD`) into an [`OffsetDateTime`].
///
/// Snap info output uses date-only strings. The time component is set to midnight UTC.
/// Returns `None` if the string cannot be parsed.
fn parse_snap_date(s: &str) -> Option<OffsetDateTime> {
    let format = time::macros::format_description!("[year]-[month]-[day]");
    time::Date::parse(s, &format)
        .ok()
        .map(|d| d.with_time(time::Time::MIDNIGHT).assume_utc())
}

/// Plugin for Snap (universal Linux package manager by Canonical).
///
/// Supports installed version detection, autodiscovery, release fetching via
/// `snap info`, and updates via `snap refresh` for Snap packages managed by
/// `snapd`.
///
/// The `package_identifier` in `SoftwareItem` is the Snap package name
/// (e.g., `vlc`, `code`, `firefox`). Updates use channel-based tracking;
/// Snap does not support pinning to a specific version string.
pub struct SnapPlugin {
    config: SnapConfig,
    executor: Arc<dyn CommandExecutor>,
}

impl SnapPlugin {
    /// Compile-time capabilities for the Snap plugin.
    pub const CAPABILITIES: &'static [PluginCapability] = &[
        PluginCapability::DiscoverLocalSoftware,
        PluginCapability::DetectHostCompatibility,
    ];

    /// Create a new Snap plugin with the given configuration and command executor.
    pub async fn new(config: SnapConfig, executor: Arc<dyn CommandExecutor>) -> Result<Self> {
        config
            .validate()
            .map_err(|e| report!(PluginError::Configuration(e.to_string())))?;
        Ok(Self { config, executor })
    }

    fn require_package_identifier(&self, package_identifier: &str) -> Result<()> {
        validate_identifier(package_identifier).map_err(|e| report!(PluginError::Configuration(e)))
    }
}

// ── PluginBase + subtrait implementations ────────────────────────────────

uptrakit_plugin_infrastructure_core::impl_plugin_base_config!(
    SnapPlugin,
    SnapConfig,
    "package_manager_snap",
    {
        fn capabilities(&self) -> Vec<PluginCapability> {
            Self::CAPABILITIES.to_vec()
        }
        fn required_sudo_commands(
            &self,
        ) -> Vec<uptrakit_plugin_infrastructure_core::SudoCommandEntry> {
            vec![
                // `refresh *` covers `snap refresh PKG`, `snap refresh PKG --channel=stable`,
                // and batch `snap refresh PKG1 PKG2 ...`.
                SudoCommandEntry::new("snap", "Snap package refresh requires root privileges")
                    .with_args_suffix(Cow::Borrowed("refresh *")),
            ]
        }

        fn as_discovery(
            &self,
        ) -> Option<&dyn uptrakit_plugin_infrastructure_core::DiscoveryPlugin> {
            Some(self)
        }
        fn as_version_detector(
            &self,
        ) -> Option<&dyn uptrakit_plugin_infrastructure_core::VersionDetectorPlugin> {
            Some(self)
        }
        fn as_release_fetcher(
            &self,
        ) -> Option<&dyn uptrakit_plugin_infrastructure_core::ReleaseFetcherPlugin> {
            Some(self)
        }
        fn as_update_executor(
            &self,
        ) -> Option<&dyn uptrakit_plugin_infrastructure_core::UpdateExecutorPlugin> {
            Some(self)
        }
    }
);

#[async_trait]
impl DiscoveryPlugin for SnapPlugin {
    /// Discover Snap packages installed on the local system.
    ///
    /// Runs `snap list` and returns all user-installed snaps, excluding known
    /// system/infrastructure snaps (`core*`, `snapd`, `bare`).
    ///
    /// In discover-all mode (default config `{}`), emits one [`DiscoveryTarget`]
    /// per snap so the controller can auto-create a default Snap plugin config and
    /// role assignments. When a real config is present (channel explicitly set),
    /// no targets are emitted — the config-ID path is used instead.
    #[tracing::instrument(skip_all)]
    async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> {
        tracing::info!("discovering Snap-managed software");

        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec("snap", ["list".to_string()]))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "snap list failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::CommandFailed(cmd_output.exit_code));
        }

        let emit_targets = self.config.is_discover_all_mode();

        let packages: Vec<DiscoveredSoftware> = cmd_output
            .output
            .lines()
            .filter_map(parse_snap_list_line)
            .filter(|(name, _)| !SYSTEM_SNAPS.contains(&name.as_str()))
            .map(|(name, version)| {
                let targets = if emit_targets {
                    vec![DiscoveryTarget {
                        plugin_type: PluginType::PackageManagerSnap,
                        plugin_config: serde_json::json!({}),
                        plugin_config_name: "Snap".to_string(),
                        roles: vec![
                            PluginRole::DetectVersion,
                            PluginRole::FetchReleases,
                            PluginRole::ExecuteUpdate,
                        ],
                        package_identifier: None,
                        config_override: None,
                        execution_site: None,
                    }]
                } else {
                    vec![]
                };
                DiscoveredSoftware {
                    package_identifier: name.clone(),
                    name,
                    installed_version: version,
                    targets,
                    extra: None,
                    qualifier: None,
                    plugin_package_identifier: None,
                    featured: false,
                }
            })
            .collect();

        tracing::debug!(count = packages.len(), "Snap software discovery complete");
        Ok(packages)
    }

    #[tracing::instrument(skip_all)]
    async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
        match self
            .executor
            .execute_quiet(&CommandSpec::exec("which", ["snap".to_string()]))
            .await
        {
            Ok(_) => Ok(HostCompatibility::Compatible),
            Err(_) => Ok(HostCompatibility::Incompatible(
                "snap not found".to_string(),
            )),
        }
    }
}

#[async_trait]
impl VersionDetectorPlugin for SnapPlugin {
    #[tracing::instrument(skip_all)]
    async fn detect_installed_version(&self, package_identifier: &str) -> Result<Option<Version>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "detecting Snap installed version");

        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "snap",
                ["list".to_string(), package_identifier.to_string()],
            ))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "snap list failed: {e}"
                )))
            })?;

        match cmd_output.exit_code {
            0 => {
                // Output: header line + one data line.
                // Parse the data line (second column is version).
                let version = cmd_output
                    .output
                    .lines()
                    .filter_map(parse_snap_list_line)
                    .next()
                    .map(|(_, v)| v);

                match version {
                    Some(v) if !v.is_empty() => {
                        tracing::debug!(version = %v, "Snap installed version detected");
                        Ok(Some(Version::new(&v)))
                    }
                    _ => Ok(None),
                }
            }
            // Exit code 1 means the snap was not found.
            1 => {
                tracing::debug!(package = %package_identifier, "snap not found in installed list");
                Ok(None)
            }
            code => bail!(PluginError::CommandFailed(code)),
        }
    }

    /// Detect installed versions for multiple packages using a single `snap list` call.
    ///
    /// Runs `snap list` (no arguments) to get all installed snaps, then looks up
    /// each requested package in the resulting map. The exit code is treated
    /// non-fatally — partial output is still useful even if `snapd` reports a warning.
    #[tracing::instrument(skip_all)]
    async fn batch_detect_installed_version(
        &self,
        items: &[BatchDetectItem],
    ) -> Result<Vec<BatchDetectResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        // Validate all identifiers up front.
        for item in items {
            validate_identifier(&item.package_identifier)
                .map_err(|e| report!(PluginError::Configuration(e)))?;
        }

        tracing::debug!(
            count = items.len(),
            "batch detecting Snap installed versions"
        );

        // A single `snap list` (no args) returns all installed snaps.
        let stdout = match self
            .executor
            .execute_quiet(&CommandSpec::exec("snap", ["list".to_string()]))
            .await
        {
            Ok(o) => o.output,
            Err(e) => {
                let error_str = format!("snap list failed: {e}");
                return Ok(items
                    .iter()
                    .map(|item| {
                        BatchDetectResult::error(item.package_identifier.clone(), error_str.clone())
                    })
                    .collect());
            }
        };

        // Build a name -> version map from the output.
        let installed: HashMap<String, String> =
            stdout.lines().filter_map(parse_snap_list_line).collect();

        Ok(items
            .iter()
            .map(|item| {
                let installed_version = installed.get(&item.package_identifier).map(Version::new);
                BatchDetectResult::new(item.package_identifier.clone(), installed_version, None)
            })
            .collect())
    }
}

#[async_trait]
impl ReleaseFetcherPlugin for SnapPlugin {
    /// Fetch the latest release for a Snap package from a specific channel.
    ///
    /// Runs `snap info <name>` and parses the `channels:` section. Returns a
    /// single [`UpstreamRelease`] for the configured channel
    /// (default: `"latest/stable"`), or an empty vec if the snap is not
    /// available on that channel.
    #[tracing::instrument(skip_all)]
    async fn fetch_releases(&self, package_identifier: &str) -> Result<Vec<UpstreamRelease>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "fetching Snap releases via snap info");

        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "snap",
                ["info".to_string(), package_identifier.to_string()],
            ))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "snap info failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::CommandFailed(cmd_output.exit_code));
        }

        let channels = parse_snap_info_channels(&cmd_output.output);
        let target_channel = self.config.effective_channel();

        let Some(info) = channels.get(target_channel) else {
            tracing::debug!(
                package = %package_identifier,
                channel = %target_channel,
                "snap not available on channel"
            );
            return Ok(vec![]);
        };

        let is_prerelease = is_prerelease_channel(target_channel);
        tracing::debug!(
            version = %info.version,
            channel = %target_channel,
            is_prerelease,
            "Snap upstream version resolved"
        );

        Ok(vec![{
            let mut release = UpstreamRelease::new(
                Version::new(&info.version),
                info.version.clone(),
                is_prerelease,
                "",
            );
            release.published_at = info.published_at;
            release
        }])
    }
}

#[async_trait]
impl UpdateExecutorPlugin for SnapPlugin {
    /// Execute a single Snap package update via `snap refresh`.
    ///
    /// Runs `snap refresh <name>` with an optional `--channel=<channel>` argument
    /// when a channel is explicitly configured. Snap tracks channels rather than
    /// pinned version strings; the `to_version` parameter is used only for the
    /// display message prefix.
    #[tracing::instrument(skip_all)]
    async fn execute_update(
        &self,
        package_identifier: &str,
        to_version: &str,
        _release_info: Option<&ReleaseInfo>,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<String> {
        self.require_package_identifier(package_identifier)?;

        let mut args = vec!["refresh".to_string(), package_identifier.to_string()];
        if let Some(channel) = &self.config.channel {
            args.push(format!("--channel={channel}"));
        }

        tracing::debug!(
            package = %package_identifier,
            to_version = %to_version,
            channel = ?self.config.channel,
            "running snap refresh"
        );

        let display_args = std::iter::once("snap")
            .chain(args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");

        send_output(
            output_tx,
            &format!("Updating {package_identifier} to {to_version}\nRunning: {display_args}"),
            OutputStreamType::Stdout,
        )
        .await;
        let mut output = format!("Running: {display_args}\n");

        let cmd_output = self
            .executor
            .execute(&CommandSpec::exec("snap", args).privileged(), output_tx)
            .await
            .map_err(|e| report!(PluginError::InstallFailed(e.to_string())))?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::InstallFailed(format!(
                "snap refresh failed with exit code {}",
                cmd_output.exit_code
            )));
        }

        output.push_str(&cmd_output.output);
        Ok(output)
    }

    /// Execute batch Snap package updates using a single `snap refresh` invocation.
    ///
    /// Snap natively supports refreshing multiple packages in a single call:
    /// `snap refresh name1 name2 ...`. All items share the same success/failure
    /// status and output, since `snap refresh` handles them atomically.
    #[tracing::instrument(skip_all)]
    async fn execute_batch_update(
        &self,
        items: &[BatchUpdateItem],
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<Vec<BatchUpdateResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        // Validate all package identifiers up front.
        for item in items {
            validate_identifier(&item.package_identifier)
                .map_err(|e| report!(PluginError::Configuration(e)))?;
        }

        let mut args = vec!["refresh".to_string()];
        for item in items {
            args.push(item.package_identifier.clone());
        }
        if let Some(channel) = &self.config.channel {
            args.push(format!("--channel={channel}"));
        }

        let display_args = std::iter::once("snap")
            .chain(args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");

        let pkg_list: Vec<&str> = items
            .iter()
            .map(|i| i.package_identifier.as_str())
            .collect();

        send_output(
            output_tx,
            &format!(
                "Batch updating {} Snap packages: {}\nRunning: {display_args}",
                items.len(),
                pkg_list.join(", ")
            ),
            OutputStreamType::Stdout,
        )
        .await;
        let mut output = format!("Running: {display_args}\n");

        tracing::debug!(
            count = items.len(),
            packages = ?pkg_list,
            "running snap refresh batch"
        );

        let cmd_output = self
            .executor
            .execute(&CommandSpec::exec("snap", args).privileged(), output_tx)
            .await
            .map_err(|e| report!(PluginError::InstallFailed(e.to_string())))?;

        output.push_str(&cmd_output.output);
        let success = cmd_output.exit_code == 0;

        Ok(items
            .iter()
            .map(|item| {
                BatchUpdateResult::new(item.package_identifier.clone(), success, output.clone())
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_core::{CommandOutput, PluginBase};

    /// Mock executor that returns a fixed output and exit code for all commands.
    struct FixedOutputExecutor {
        output: String,
        exit_code: i32,
    }

    #[async_trait]
    impl CommandExecutor for FixedOutputExecutor {
        async fn execute(
            &self,
            _spec: &CommandSpec,
            _output_tx: &mpsc::Sender<UpdateOutputLine>,
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
            // Always return Ok with the exit_code field — non-zero exit is not an Err.
            // Err is reserved for cases where the command could not be started at all.
            Ok(CommandOutput {
                output: self.output.clone(),
                exit_code: self.exit_code,
            })
        }
    }

    fn make_executor(stdout: &str, exit_code: i32) -> Arc<dyn CommandExecutor> {
        Arc::new(FixedOutputExecutor {
            output: stdout.to_string(),
            exit_code,
        })
    }

    // ── validate_identifier ───────────────────────────────────────────────────

    #[test]
    fn validate_identifier_simple_valid() {
        assert!(validate_identifier("vlc").is_ok());
        assert!(validate_identifier("firefox").is_ok());
        assert!(validate_identifier("code").is_ok());
        assert!(validate_identifier("hello-world").is_ok());
        assert!(validate_identifier("ab").is_ok());
    }

    #[test]
    fn validate_identifier_with_digits() {
        assert!(validate_identifier("package1").is_ok());
        assert!(validate_identifier("1password").is_ok());
    }

    #[test]
    fn validate_identifier_empty_fails() {
        assert!(validate_identifier("").is_err());
    }

    #[test]
    fn validate_identifier_too_short_fails() {
        assert!(validate_identifier("a").is_err());
    }

    #[test]
    fn validate_identifier_too_long_fails() {
        assert!(validate_identifier(&"a".repeat(41)).is_err());
    }

    #[test]
    fn validate_identifier_exactly_40_valid() {
        // 38 'a' chars + "ab" start = 40 total
        let name = format!("a{}", "b".repeat(39));
        assert!(validate_identifier(&name).is_ok());
    }

    #[test]
    fn validate_identifier_hyphen_start_fails() {
        assert!(validate_identifier("-vlc").is_err());
    }

    #[test]
    fn validate_identifier_hyphen_end_fails() {
        assert!(validate_identifier("vlc-").is_err());
    }

    #[test]
    fn validate_identifier_consecutive_hyphens_fails() {
        assert!(validate_identifier("hello--world").is_err());
    }

    #[test]
    fn validate_identifier_uppercase_fails() {
        assert!(validate_identifier("VLC").is_err());
        assert!(validate_identifier("Firefox").is_err());
    }

    #[test]
    fn validate_identifier_dot_fails() {
        // Snap names must not contain dots (unlike Debian package names).
        assert!(validate_identifier("my.package").is_err());
    }

    #[test]
    fn validate_identifier_underscore_fails() {
        assert!(validate_identifier("my_package").is_err());
    }

    // ── is_prerelease_channel ─────────────────────────────────────────────────

    #[test]
    fn is_prerelease_channel_stable() {
        assert!(!is_prerelease_channel("stable"));
        assert!(!is_prerelease_channel("latest/stable"));
        assert!(!is_prerelease_channel("1.0/stable"));
    }

    #[test]
    fn is_prerelease_channel_candidate() {
        assert!(!is_prerelease_channel("candidate"));
        assert!(!is_prerelease_channel("latest/candidate"));
    }

    #[test]
    fn is_prerelease_channel_beta() {
        assert!(is_prerelease_channel("beta"));
        assert!(is_prerelease_channel("latest/beta"));
    }

    #[test]
    fn is_prerelease_channel_edge() {
        assert!(is_prerelease_channel("edge"));
        assert!(is_prerelease_channel("latest/edge"));
        assert!(is_prerelease_channel("1.0/edge"));
    }

    // ── parse_snap_list_line ──────────────────────────────────────────────────

    #[test]
    fn parse_snap_list_line_header_skipped() {
        assert!(parse_snap_list_line("Name    Version   Rev    Tracking").is_none());
    }

    #[test]
    fn parse_snap_list_line_data_line() {
        let result = parse_snap_list_line("vlc     3.0.20    2359   latest/stable");
        assert_eq!(result, Some(("vlc".to_string(), "3.0.20".to_string())));
    }

    #[test]
    fn parse_snap_list_line_empty() {
        assert!(parse_snap_list_line("").is_none());
    }

    #[test]
    fn parse_snap_list_line_single_token() {
        // Only a name, no version — malformed line.
        assert!(parse_snap_list_line("vlc").is_none());
    }

    // ── parse_snap_info_channels ──────────────────────────────────────────────

    #[test]
    fn parse_snap_info_channels_basic() {
        // Channel lines must have leading spaces to be detected as part of the section.
        let output = concat!(
            "name:      vlc\n",
            "channels:\n",
            "  latest/stable:    3.0.20     2024-01-12 (2359) 215MB -\n",
            "  latest/candidate: \u{2191}\n",
            "  latest/beta:      \u{2191}\n",
            "  latest/edge:      3.0.21     2024-01-15 (2400) 216MB -\n",
        );
        let channels = parse_snap_info_channels(output);
        assert_eq!(channels.len(), 2);
        assert_eq!(channels["latest/stable"].version, "3.0.20");
        assert_eq!(channels["latest/edge"].version, "3.0.21");
        // ↑ entries should be skipped
        assert!(!channels.contains_key("latest/candidate"));
        assert!(!channels.contains_key("latest/beta"));
    }

    #[test]
    fn parse_snap_info_channels_no_channels_section() {
        let output = "name:      vlc\nsummary:   VLC media player\n";
        let channels = parse_snap_info_channels(output);
        assert!(channels.is_empty());
    }

    #[test]
    fn parse_snap_info_channels_with_track() {
        let output = concat!(
            "channels:\n",
            "  latest/stable:  2.0.1  2024-01-01 (100) 50MB -\n",
            "  1.0/stable:     1.0.5  2023-06-20 (90)  45MB -\n",
        );
        let channels = parse_snap_info_channels(output);
        assert_eq!(channels["latest/stable"].version, "2.0.1");
        assert_eq!(channels["1.0/stable"].version, "1.0.5");
    }

    #[test]
    fn parse_snap_info_channels_date_parsed() {
        let output = "channels:\n  latest/stable: 1.0.0 2024-03-15 (100) 50MB -\n";
        let channels = parse_snap_info_channels(output);
        assert!(channels["latest/stable"].published_at.is_some());
    }

    #[test]
    fn parse_snap_info_channels_no_date_is_none() {
        // If the date field is missing or malformed, published_at should be None.
        let output = "channels:\n  latest/stable: 1.0.0\n";
        let channels = parse_snap_info_channels(output);
        assert_eq!(channels["latest/stable"].version, "1.0.0");
        assert!(channels["latest/stable"].published_at.is_none());
    }

    // ── discover_software: system snap exclusion ──────────────────────────────

    #[tokio::test]
    async fn discover_software_excludes_system_snaps() {
        let snap_list_output = "Name    Version   Rev    Tracking         Publisher  Notes\n\
                                snapd   2.61.3    21759  latest/stable    canonical  snapd\n\
                                core20  20231212  2105   latest/stable    canonical  base\n\
                                core22  20231201  1234   latest/stable    canonical  base\n\
                                bare    1.0       5      latest/stable    canonical  base\n\
                                vlc     3.0.20    2359   latest/stable    videolan   -\n\
                                code    1.85.2    163351 latest/stable    vscode     -\n";

        let executor = make_executor(snap_list_output, 0);
        let plugin = SnapPlugin::new(SnapConfig::default(), executor)
            .await
            .unwrap();

        let discovered = plugin.discover_software().await.unwrap();

        // Only vlc and code should be discovered; system snaps excluded.
        assert_eq!(discovered.len(), 2);
        let names: Vec<&str> = discovered.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"vlc"));
        assert!(names.contains(&"code"));
        assert!(!names.contains(&"snapd"));
        assert!(!names.contains(&"core20"));
        assert!(!names.contains(&"core22"));
        assert!(!names.contains(&"bare"));
    }

    #[tokio::test]
    async fn discover_software_emits_targets_in_discover_all_mode() {
        let snap_list_output = "Name    Version   Rev    Tracking         Publisher  Notes\n\
                                vlc     3.0.20    2359   latest/stable    videolan   -\n";

        let executor = make_executor(snap_list_output, 0);
        // Default config → discover-all mode.
        let plugin = SnapPlugin::new(SnapConfig::default(), executor)
            .await
            .unwrap();

        let discovered = plugin.discover_software().await.unwrap();
        assert_eq!(discovered.len(), 1);
        assert!(!discovered[0].targets.is_empty());
        assert_eq!(
            discovered[0].targets[0].plugin_type,
            PluginType::PackageManagerSnap
        );
    }

    #[tokio::test]
    async fn discover_software_no_targets_with_explicit_config() {
        let snap_list_output = "Name    Version   Rev    Tracking         Publisher  Notes\n\
                                vlc     3.0.20    2359   latest/stable    videolan   -\n";

        let executor = make_executor(snap_list_output, 0);
        // Explicit channel → config-ID path, no targets.
        let plugin = SnapPlugin::new(
            SnapConfig {
                channel: Some("latest/stable".to_string()),
            },
            executor,
        )
        .await
        .unwrap();

        let discovered = plugin.discover_software().await.unwrap();
        assert_eq!(discovered.len(), 1);
        assert!(discovered[0].targets.is_empty());
    }

    // ── detect_installed_version ──────────────────────────────────────────────

    #[tokio::test]
    async fn detect_installed_version_found() {
        let output = "Name  Version  Rev   Tracking        Publisher  Notes\n\
                      vlc   3.0.20   2359  latest/stable   videolan   -\n";
        let executor = make_executor(output, 0);
        let plugin = SnapPlugin::new(SnapConfig::default(), executor)
            .await
            .unwrap();

        let result = plugin.detect_installed_version("vlc").await.unwrap();
        assert_eq!(result, Some(Version::new("3.0.20")));
    }

    #[tokio::test]
    async fn detect_installed_version_not_found() {
        let executor = make_executor("error: snap \"vlc\" is not installed\n", 1);
        let plugin = SnapPlugin::new(SnapConfig::default(), executor)
            .await
            .unwrap();

        let result = plugin.detect_installed_version("vlc").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn detect_installed_version_invalid_identifier_fails() {
        let executor = make_executor("", 0);
        let plugin = SnapPlugin::new(SnapConfig::default(), executor)
            .await
            .unwrap();

        assert!(plugin.detect_installed_version("VLC").await.is_err());
        assert!(plugin.detect_installed_version("-invalid").await.is_err());
    }

    // ── batch_detect_installed_version ────────────────────────────────────────

    #[tokio::test]
    async fn batch_detect_installed_version_basic() {
        let output = "Name    Version   Rev    Tracking         Publisher  Notes\n\
                      vlc     3.0.20    2359   latest/stable    videolan   -\n\
                      code    1.85.2    163351 latest/stable    vscode     -\n";

        let executor = make_executor(output, 0);
        let plugin = SnapPlugin::new(SnapConfig::default(), executor)
            .await
            .unwrap();

        let items = vec![
            BatchDetectItem::new("vlc".to_string()),
            BatchDetectItem::new("code".to_string()),
            BatchDetectItem::new("notinstalled".to_string()),
        ];

        let results = plugin.batch_detect_installed_version(&items).await.unwrap();
        assert_eq!(results.len(), 3);

        let vlc = results
            .iter()
            .find(|r| r.package_identifier == "vlc")
            .unwrap();
        assert_eq!(vlc.installed_version, Some(Version::new("3.0.20")));

        let code = results
            .iter()
            .find(|r| r.package_identifier == "code")
            .unwrap();
        assert_eq!(code.installed_version, Some(Version::new("1.85.2")));

        let missing = results
            .iter()
            .find(|r| r.package_identifier == "notinstalled")
            .unwrap();
        assert!(missing.installed_version.is_none());
    }

    #[tokio::test]
    async fn batch_detect_installed_version_empty_returns_empty() {
        let executor = make_executor("", 0);
        let plugin = SnapPlugin::new(SnapConfig::default(), executor)
            .await
            .unwrap();

        let results = plugin.batch_detect_installed_version(&[]).await.unwrap();
        assert!(results.is_empty());
    }

    // ── fetch_releases ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn fetch_releases_latest_stable() {
        let output = concat!(
            "name:    vlc\n",
            "channels:\n",
            "  latest/stable: 3.0.20 2024-01-12 (2359) 215MB -\n",
            "  latest/edge:   3.0.21 2024-01-15 (2400) 216MB -\n",
        );
        let executor = make_executor(output, 0);
        let plugin = SnapPlugin::new(SnapConfig::default(), executor)
            .await
            .unwrap();

        let releases = plugin.fetch_releases("vlc").await.unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].tag, "3.0.20");
        assert!(!releases[0].is_prerelease);
    }

    #[tokio::test]
    async fn fetch_releases_edge_channel_is_prerelease() {
        let output = "channels:\n  latest/edge: 3.0.21 2024-01-15 (2400) 216MB -\n";
        let executor = make_executor(output, 0);
        let plugin = SnapPlugin::new(
            SnapConfig {
                channel: Some("latest/edge".to_string()),
            },
            executor,
        )
        .await
        .unwrap();

        let releases = plugin.fetch_releases("vlc").await.unwrap();
        assert_eq!(releases.len(), 1);
        assert!(releases[0].is_prerelease);
    }

    #[tokio::test]
    async fn fetch_releases_channel_not_in_output_returns_empty() {
        let output = "channels:\n  latest/stable: 3.0.20 2024-01-12 (2359) 215MB -\n";
        let executor = make_executor(output, 0);
        let plugin = SnapPlugin::new(
            SnapConfig {
                channel: Some("1.0/stable".to_string()),
            },
            executor,
        )
        .await
        .unwrap();

        let releases = plugin.fetch_releases("vlc").await.unwrap();
        assert!(releases.is_empty());
    }

    // ── capabilities ─────────────────────────────────────────────────────────

    #[test]
    fn snap_capabilities_declared() {
        assert!(SnapPlugin::CAPABILITIES.contains(&PluginCapability::DiscoverLocalSoftware));
        assert!(SnapPlugin::CAPABILITIES.contains(&PluginCapability::DetectHostCompatibility));
        // Snap does not need RefreshPackageIndex — snapd manages its own cache.
        assert!(!SnapPlugin::CAPABILITIES.contains(&PluginCapability::RefreshPackageIndex));
    }

    // ── system_snaps constant ─────────────────────────────────────────────────

    #[test]
    fn system_snaps_contains_expected_entries() {
        assert!(SYSTEM_SNAPS.contains(&"core"));
        assert!(SYSTEM_SNAPS.contains(&"snapd"));
        assert!(SYSTEM_SNAPS.contains(&"bare"));
        assert!(SYSTEM_SNAPS.contains(&"core22"));
    }

    // ── required_sudo_commands ───────────────────────────────────────────────

    #[tokio::test]
    async fn snap_plugin_required_sudo_commands() {
        let plugin = SnapPlugin::new(SnapConfig::default(), make_executor("", 0))
            .await
            .expect("create plugin");
        let entries = plugin.required_sudo_commands();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command, "snap");
        assert!(!entries[0].needs_setenv);
        assert_eq!(entries[0].args_suffix.as_deref(), Some("refresh *"));
    }
}
