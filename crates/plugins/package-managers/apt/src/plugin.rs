use std::borrow::Cow;
use std::sync::Arc;

use uptrakit_plugin_infrastructure_core::command::CommandExecutor;
use uptrakit_plugin_infrastructure_core::{
    ConfigModel, ConfigTestKind, HostRequirements, HostRuntime, PluginFamily, Result,
    SudoCommandEntry, declare_plugin,
};
use uptrakit_shared_types::PackageIdentifierRules;

use crate::config::AptConfig;

/// Fixed path for the temporary APT preferences file used during batch updates.
///
/// This path is hardcoded on both the write side (`execute_batch_update`) and
/// the sudoers declaration side (`required_sudo_commands`) so that the sudoers
/// rule can be maximally restrictive: the rule locks in exactly this path and
/// no other. Changing this value requires updating both uses simultaneously.
pub(crate) const APT_BATCH_PREF_FILE: &str = "/tmp/uptrakit-apt-batch.pref";

const IDENTIFIER_RULES: PackageIdentifierRules = PackageIdentifierRules {
    min_len: 2,
    max_len: 64,
    first_char_valid: |c| c.is_ascii_lowercase() || c.is_ascii_digit(),
    char_valid: |c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '+' | '-' | '.'),
    reject_double_dot: true,
};

/// Validate a Debian APT package identifier.
///
/// Enforces Debian package naming rules from the Debian Policy Manual:
/// - Between 2 and 64 characters long.
/// - Must start with a lowercase letter or digit (`[a-z0-9]`).
/// - May only contain lowercase letters, digits, `+`, `-`, and `.`.
/// - Must not contain `..` (path traversal protection).
pub fn validate_identifier(value: &str) -> std::result::Result<(), String> {
    IDENTIFIER_RULES.validate(value)
}

/// Validate a Debian APT version string before it is interpolated into install commands.
///
/// Allows Debian version characters (`[a-zA-Z0-9.+~:-]`). Rejects:
/// - Empty strings
/// - Strings starting with `-` (could be interpreted as a command-line flag by apt-get)
/// - Strings exceeding 256 characters
pub fn validate_version(version: &str) -> std::result::Result<(), String> {
    if version.is_empty() {
        return Err("version must not be empty".to_string());
    }
    if version.len() > 256 {
        return Err("version must not exceed 256 characters".to_string());
    }
    if version.starts_with('-') {
        return Err("version must not start with '-' (would be interpreted as a flag)".to_string());
    }
    for ch in version.chars() {
        if !ch.is_ascii_alphanumeric() && !matches!(ch, '.' | '+' | '~' | ':' | '-') {
            return Err(format!("version contains invalid character: '{ch}'"));
        }
    }
    Ok(())
}

/// Plugin for APT (Debian/Ubuntu package manager).
///
/// Supports installed version detection, package index refresh, autodiscovery,
/// and updates for Debian packages managed by `apt-get`.
///
/// The `package_identifier` in `SoftwareItem` is the Debian package name
/// (e.g., `nginx`, `python3`, `apt-utils`).
pub struct AptPlugin {
    pub(crate) config: AptConfig,
    pub(crate) executor: Arc<dyn CommandExecutor>,
}

/// Parsed result from a single `apt-cache madison` line.
pub(crate) struct MadisonEntry {
    pub(crate) version: String,
    pub(crate) source: String,
}

impl AptPlugin {
    /// Create a new APT plugin with the given configuration.
    pub fn new(
        config: AptConfig,
        runtime: Arc<dyn HostRuntime>,
    ) -> std::result::Result<Self, String> {
        let executor = runtime.executor();
        Ok(Self { config, executor })
    }

    pub(crate) fn require_package_identifier(&self, package_identifier: &str) -> Result<()> {
        uptrakit_plugin_infrastructure_core::require_package_identifier(
            package_identifier,
            validate_identifier,
        )
    }

    /// Return the sudo commands required by the APT plugin.
    ///
    /// This is a static function taking the serialized config (not `&self`)
    /// because the descriptor stores it as a function pointer.
    pub fn required_sudo_commands(_config: &serde_json::Value) -> Vec<SudoCommandEntry> {
        vec![
            SudoCommandEntry::new("apt-get", "Refresh the APT package index")
                // Restrict to `apt-get update` only (with optional flags).
                .with_args_suffix(Cow::Borrowed("update *"))
                .with_setenv(),
            SudoCommandEntry::new("apt-get", "Install or upgrade an APT package")
                // Restrict to `apt-get install` only; covers single and batch installs.
                .with_args_suffix(Cow::Borrowed("install *"))
                .with_setenv(),
            SudoCommandEntry::new(
                "apt-get",
                "Upgrade packages using a pinned preferences file (batch update)",
            )
            // Lock in the exact -o Dir::Etc::Preferences= invocation that
            // execute_batch_update uses. The path is intentionally hardcoded on
            // both sides; see APT_BATCH_PREF_FILE. Using `apt-get upgrade` (not
            // `install`) preserves the apt manual/auto install mark — packages
            // auto-installed as dependencies keep their `auto` mark, allowing
            // `apt autoremove` to clean them up correctly.
            .with_args_suffix(Cow::Owned(format!(
                "-o Dir::Etc::Preferences={APT_BATCH_PREF_FILE} upgrade *"
            )))
            .with_setenv(),
        ]
    }
}

// ── declare_plugin! ──────────────────────────────────────────────────────

declare_plugin!(AptPlugin, AptConfig, "package_manager_apt", {
    display_name: "APT Package Manager",
    family: PluginFamily::Software,
    config_model: ConfigModel::PluginConfig,
    host_requirements: HostRequirements::POSIX,
    config_test: [ConfigTestKind::VersionDetection, ConfigTestKind::UpdateCommandValidation],
    type_settings: true,
    roles: [
        Discoverer,
        VersionDetector,
        ReleaseFetcher,
        PackageIndexer { host_requirements: HostRequirements::POSIX_PRIVILEGED },
        UpdateExecutor { host_requirements: HostRequirements::POSIX_PRIVILEGED },
    ],
    sudo: AptPlugin::required_sudo_commands,
});

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use uptrakit_plugin_infrastructure_core::command::CommandExecutor;
    use uptrakit_plugin_infrastructure_core::{
        HostCapabilities, LocalCommandExecutor, PluginCapability, PluginMeta, StandardHostRuntime,
    };

    /// Helper to create an `AptPlugin` for testing.
    fn test_plugin(config: AptConfig) -> AptPlugin {
        let executor = Arc::new(LocalCommandExecutor) as Arc<dyn CommandExecutor>;
        let caps = HostCapabilities::default();
        let runtime = Arc::new(StandardHostRuntime::new(executor, caps)) as Arc<dyn HostRuntime>;
        AptPlugin::new(config, runtime).unwrap()
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

    // ── validate_version ────────────────────────────────────────────────

    #[test]
    fn validate_version_debian_standard() {
        assert!(validate_version("1.24.0-2ubuntu7.3").is_ok());
        assert!(validate_version("3.11.0-5ubuntu2").is_ok());
        assert!(validate_version("1:2.3.4-5").is_ok()); // epoch format
    }

    #[test]
    fn validate_version_with_tilde() {
        assert!(validate_version("1.0~beta1").is_ok());
    }

    #[test]
    fn validate_version_empty_fails() {
        let err = validate_version("").expect_err("should fail");
        assert!(err.contains("empty"));
    }

    #[test]
    fn validate_version_too_long_fails() {
        let long = "1".repeat(257);
        assert!(validate_version(&long).is_err());
    }

    #[test]
    fn validate_version_leading_dash_fails() {
        let err = validate_version("--allow-unauthenticated").expect_err("should fail");
        assert!(err.contains("flag"));
    }

    #[test]
    fn validate_version_space_fails() {
        assert!(validate_version("1.0 --allow-unauthenticated").is_err());
    }

    #[test]
    fn validate_version_equals_fails() {
        assert!(validate_version("1.0=extra").is_err());
    }

    #[test]
    fn validate_version_max_length_ok() {
        let v = "1".repeat(256);
        assert!(validate_version(&v).is_ok());
    }

    // ── plugin_type_id ──────────────────────────────────────────────────

    #[test]
    fn plugin_type_id() {
        let plugin = test_plugin(AptConfig::default());
        assert_eq!(plugin.plugin_type_id().as_str(), "package_manager_apt");
    }

    // ── descriptor capabilities ─────────────────────────────────────────

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
                .contains(&PluginCapability::RefreshPackageIndex)
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
    }

    // ── descriptor roles ────────────────────────────────────────────────

    #[test]
    fn descriptor_has_all_roles() {
        assert!(DESCRIPTOR.roles.discoverer.is_some());
        assert!(DESCRIPTOR.roles.version_detector.is_some());
        assert!(DESCRIPTOR.roles.release_fetcher.is_some());
        assert!(DESCRIPTOR.roles.package_indexer.is_some());
        assert!(DESCRIPTOR.roles.update_executor.is_some());
        assert!(DESCRIPTOR.roles.lifecycle_hook.is_none());
    }

    // ── descriptor type settings ────────────────────────────────────────

    #[test]
    fn descriptor_has_type_settings() {
        assert!(DESCRIPTOR.type_settings.is_some());
    }

    // ── descriptor sudo ─────────────────────────────────────────────────

    #[test]
    fn descriptor_has_sudo() {
        assert!(DESCRIPTOR.sudo.is_some());
    }

    // ── required_sudo_commands ───────────────────────────────────────────

    #[test]
    fn apt_plugin_required_sudo_commands() {
        let entries = AptPlugin::required_sudo_commands(&serde_json::json!({}));
        assert_eq!(entries.len(), 3);
        // All three entries are for apt-get.
        assert!(entries.iter().all(|e| e.command == "apt-get"));
        // All three require SETENV: (DEBIAN_FRONTEND=noninteractive).
        assert!(entries.iter().all(|e| e.needs_setenv));
        // Index refresh entry.
        assert_eq!(entries[0].args_suffix.as_deref(), Some("update *"));
        // Single-package install entry.
        assert_eq!(entries[1].args_suffix.as_deref(), Some("install *"));
        // Batch upgrade entry locks in the pref-file path.
        let batch_suffix = entries[2].args_suffix.as_deref().unwrap();
        assert!(
            batch_suffix.contains(APT_BATCH_PREF_FILE),
            "batch args_suffix must reference APT_BATCH_PREF_FILE"
        );
        assert!(batch_suffix.starts_with("-o Dir::Etc::Preferences="));
        assert!(batch_suffix.ends_with("upgrade *"));
    }
}
