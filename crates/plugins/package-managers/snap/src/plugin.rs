use std::collections::HashMap;
use std::sync::Arc;

use time::OffsetDateTime;
use uptrakit_plugin_infrastructure_core::command::CommandExecutor;
use uptrakit_plugin_infrastructure_core::{
    ConfigModel, ConfigTestKind, HostRequirements, HostRuntime, PluginConfig, PluginError,
    PluginFamily, Result, SudoCommandEntry, declare_plugin, require_posix_executor,
};
use uptrakit_shared_types::PackageIdentifierRules;

use crate::config::SnapConfig;

/// System snaps that are excluded from discovery output.
///
/// These are infrastructure/base snaps managed by `snapd` itself and are not
/// user-installed software packages. Including them would create spurious
/// software items in Uptrakit.
pub(crate) const SYSTEM_SNAPS: &[&str] = &[
    "core", "core18", "core20", "core22", "core24", "core26", "snapd", "bare",
];

/// Internal: channel info parsed from `snap info` output.
pub(crate) struct SnapChannelInfo {
    pub(crate) version: String,
    pub(crate) published_at: Option<OffsetDateTime>,
}

const IDENTIFIER_RULES: PackageIdentifierRules = PackageIdentifierRules {
    min_len: 2,
    max_len: 40,
    first_char_valid: |c| c.is_ascii_lowercase() || c.is_ascii_digit(),
    char_valid: |c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-',
    reject_double_dot: false,
};

/// Validate a Snap package identifier.
///
/// Enforces Snap package naming rules:
/// - Between 2 and 40 characters long.
/// - Charset: `[a-z0-9-]` only (lowercase letters, digits, hyphens; no dots).
/// - Must start and end with `[a-z0-9]` (not a hyphen).
/// - No consecutive hyphens (`--`).
pub fn validate_identifier(value: &str) -> std::result::Result<(), String> {
    IDENTIFIER_RULES.validate(value)?;

    let last = value.chars().next_back().unwrap_or('\0');
    if !last.is_ascii_lowercase() && !last.is_ascii_digit() {
        return Err("package_identifier must end with a lowercase letter or digit".to_string());
    }

    if value.contains("--") {
        return Err("package_identifier must not contain consecutive hyphens".to_string());
    }

    Ok(())
}

/// Returns `true` if the risk segment of a channel string indicates a pre-release.
///
/// A channel is pre-release if its risk component is `"beta"` or `"edge"`.
/// Both bare risks (`"edge"`) and track-qualified risks (`"latest/edge"`) are handled.
pub(crate) fn is_prerelease_channel(channel: &str) -> bool {
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
pub(crate) fn parse_snap_list_line(line: &str) -> Option<(String, String)> {
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
/// Lines containing `\u{2191}` (same-as-above indicator) are skipped rather than resolved upward.
///
/// Returns a map of channel name -> [`SnapChannelInfo`].
pub(crate) fn parse_snap_info_channels(output: &str) -> HashMap<String, SnapChannelInfo> {
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

        // Skip \u{2191} entries (same version as the channel above).
        if rest.starts_with('\u{2191}') || rest == "\u{2191}" {
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
pub(crate) fn parse_snap_date(s: &str) -> Option<OffsetDateTime> {
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
    pub(crate) config: SnapConfig,
    pub(crate) executor: Arc<dyn CommandExecutor>,
}

impl SnapPlugin {
    /// Create a new Snap plugin with the given configuration and host runtime.
    pub fn new(
        config: SnapConfig,
        runtime: Arc<dyn HostRuntime>,
    ) -> std::result::Result<Self, String> {
        let executor = require_posix_executor(runtime.as_ref()).map_err(|e| format!("{e}"))?;
        config.validate()?;
        Ok(Self { config, executor })
    }

    /// Sudo commands required by this plugin.
    fn required_sudo_commands(_config: &serde_json::Value) -> Vec<SudoCommandEntry> {
        vec![
            // `refresh *` covers `snap refresh PKG`, `snap refresh PKG --channel=stable`,
            // and batch `snap refresh PKG1 PKG2 ...`.
            SudoCommandEntry::new("snap", "Refresh one or more Snap packages")
                .with_args_suffix("refresh *"),
        ]
    }

    pub(crate) fn require_package_identifier(&self, package_identifier: &str) -> Result<()> {
        validate_identifier(package_identifier)
            .map_err(|e| rootcause::report!(PluginError::Configuration(e)))
    }
}

// ── Plugin descriptor ─────────────────────────────────────────────────────

declare_plugin!(SnapPlugin, SnapConfig, "package_manager_snap", {
    display_name: "Snap",
    family: PluginFamily::Software,
    config_model: ConfigModel::PluginConfig,
    host_requirements: HostRequirements::POSIX,
    config_test: [ConfigTestKind::VersionDetection, ConfigTestKind::UpdateCommandValidation],
    type_settings: true,
    roles: [Discoverer, VersionDetector, ReleaseFetcher,
            UpdateExecutor { host_requirements: HostRequirements::POSIX_PRIVILEGED }],
    sudo: SnapPlugin::required_sudo_commands,
});

#[cfg(test)]
mod tests {
    use super::*;

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
        // Only a name, no version -- malformed line.
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
        // \u{2191} entries should be skipped
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

    // ── system_snaps constant ─────────────────────────────────────────────────

    #[test]
    fn system_snaps_contains_expected_entries() {
        assert!(SYSTEM_SNAPS.contains(&"core"));
        assert!(SYSTEM_SNAPS.contains(&"snapd"));
        assert!(SYSTEM_SNAPS.contains(&"bare"));
        assert!(SYSTEM_SNAPS.contains(&"core22"));
    }

    // ── required_sudo_commands ───────────────────────────────────────────────

    #[test]
    fn snap_plugin_required_sudo_commands() {
        assert!(DESCRIPTOR.sudo.is_some());
        let entries = (DESCRIPTOR.sudo.unwrap())(&serde_json::json!({}));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command, "snap");
        assert!(!entries[0].needs_setenv);
        assert_eq!(entries[0].args_suffix.as_deref(), Some("refresh *"));
    }

    // ── capabilities ─────────────────────────────────────────────────────────

    #[test]
    fn snap_plugin_capabilities() {
        use uptrakit_plugin_infrastructure_core::PluginCapability;
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
        // Snap does not need RefreshPackageIndex -- snapd manages its own cache.
        assert!(
            !DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::RefreshPackageIndex)
        );
        assert_eq!(DESCRIPTOR.capabilities.len(), 6);
    }
}
