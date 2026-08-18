use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use uptrakit_plugin_infrastructure_core::command::{CommandExecutor, CommandSpec};
use uptrakit_plugin_infrastructure_core::{
    ConfigModel, ConfigTestKind, DiscoveredSoftware, DiscoveryTarget, HostCompatibility,
    HostRequirements, HostRuntime, PluginConfig, PluginConfigValidationError, PluginFamily,
    PluginRole, Result, declare_plugin, execute_and_capture, plugin_ids,
};
use uptrakit_shared_types::PackageIdentifierRules;

use crate::config::UvConfig;

/// PEP 503/508 project-name charset: ASCII alphanumeric plus `.`, `_`, `-`;
/// must start (and per PEP 508 end, enforced loosely here) alphanumeric.
const IDENTIFIER_RULES: PackageIdentifierRules = PackageIdentifierRules {
    min_len: 1,
    max_len: 128,
    first_char_valid: |c| c.is_ascii_alphanumeric(),
    char_valid: |c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-',
    reject_double_dot: true,
};

/// Validate a uv tool package identifier (PEP 503/508 project-name charset).
///
/// uv prints PEP 503-normalized names (`ruamel.yaml.cmd` → `ruamel-yaml-cmd`),
/// but the raw charset still admits `.` and `_` so operator-entered
/// identifiers in either form validate. `..` and path separators are rejected
/// (the identifier is later used as a path segment under the uv tools dir).
pub fn validate_identifier(value: &str) -> std::result::Result<(), PluginConfigValidationError> {
    IDENTIFIER_RULES
        .validate(value)
        .map_err(PluginConfigValidationError::InvalidIdentifier)
}

/// Parse `uv tool list` output into a tool name → version map.
///
/// Stdout format (verified on uv 0.11.29):
///
/// ```text
/// ruff v0.6.8
/// - ruff
/// ```
///
/// The input is the **merged stdout+stderr stream** (`CommandOutput.output`
/// concatenates both), so acceptance is hard-anchored. A line is accepted only
/// when, split at the first `" v"`:
/// - the name part starts at column 0, is non-empty, and passes the
///   PEP 503/508 identifier charset (leading whitespace, `- ` bullets,
///   `warning:`/`note:` prefixes all fail this);
/// - the version part starts with an ASCII digit (PEP 440 normalized forms
///   cannot start otherwise) and contains no whitespace — any trailing
///   content rejects the line.
///
/// Non-matching lines (entrypoint bullets, `No tools installed` on stderr,
/// uv warnings/notices) are skipped without error; there is no "malformed
/// payload" bail for this free-text format. The degenerate all-noise case
/// yields an empty map, indistinguishable from "no tools" by design.
pub fn parse_uv_tool_list(output: &str) -> HashMap<String, String> {
    let mut result = HashMap::new();
    for line in output.lines() {
        let Some((name, version)) = line.split_once(" v") else {
            continue;
        };
        if name.is_empty() || validate_identifier(name).is_err() {
            continue;
        }
        if !version.starts_with(|c: char| c.is_ascii_digit())
            || version.contains(char::is_whitespace)
        {
            continue;
        }
        result.insert(name.to_string(), version.to_string());
    }
    result
}

/// Plugin for tracking Python CLI tools installed via `uv tool install`.
///
/// - **Discovery**: `uv tool list` — one featured software item per tool.
/// - **Version detection**: `uv tool list` — single call, looked up in the map.
/// - Release fetching (Plan 2) and update execution (Plan 3) follow.
///
/// **Scope:** the agent user's tools only (`uv tool list` sees one user's
/// tools dir). Tools installed by other users are invisible — consistent with
/// the unprivileged-agent invariant. The agent user's `PATH` must include
/// uv's bin dir (typically `~/.local/bin`).
pub struct UvPlugin {
    pub(crate) executor: Arc<dyn CommandExecutor>,
}

impl UvPlugin {
    /// Create a new uv plugin with the given configuration and host runtime.
    pub fn new(
        config: UvConfig,
        runtime: Arc<dyn HostRuntime>,
    ) -> std::result::Result<Self, String> {
        config.validate().map_err(|e| e.to_string())?;
        Ok(Self {
            executor: runtime.executor(),
        })
    }

    pub(crate) fn require_package_identifier(&self, package_identifier: &str) -> Result<()> {
        uptrakit_plugin_infrastructure_core::require_package_identifier(
            package_identifier,
            validate_identifier,
        )
    }
}

// ── Plugin descriptor ─────────────────────────────────────────────────────
// Plan 2 adds: ReleaseFetcher role + extra_capabilities
// [PluginCapability::ControllerSideFetchReleases].
// Plan 3 adds: UpdateExecutor role + ConfigTestKind::UpdateCommandValidation.

declare_plugin!(UvPlugin, UvConfig, "package_manager_uv", {
    display_name: "uv Tools",
    family: PluginFamily::Software,
    config_model: ConfigModel::PluginConfig,
    host_requirements: HostRequirements::POSIX,
    config_test: [ConfigTestKind::VersionDetection],
    type_settings: true,
    roles: [Discoverer, VersionDetector],
});

#[async_trait]
impl uptrakit_plugin_infrastructure_core::Discoverer for UvPlugin {
    /// Discover tools installed via `uv tool install` for the agent user.
    #[tracing::instrument(skip_all)]
    async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> {
        tracing::info!("discovering uv-installed tools");

        let output = execute_and_capture(
            self.executor.as_ref(),
            CommandSpec::exec("uv", ["tool".to_string(), "list".to_string()]),
            "uv tool list",
        )
        .await?;

        let installed = parse_uv_tool_list(&output);

        if installed.is_empty()
            && !output.trim().is_empty()
            && !output.contains("No tools installed")
        {
            // Format-drift guard: non-empty output that parses to zero tools
            // means the hard-anchored parser no longer matches the
            // `uv tool list` format (e.g. a future uv release). Without this
            // signal every uv item would go `missing_since` → deactivated
            // fleet-wide with nothing distinguishing it from real removal.
            tracing::warn!(
                output_len = output.len(),
                "uv tool list output parsed to zero tools; possible uv output format change"
            );
        }

        let tools: Vec<DiscoveredSoftware> = installed
            .into_iter()
            .map(|(name, version)| {
                let targets = vec![DiscoveryTarget {
                    plugin_type: plugin_ids::PACKAGE_MANAGER_UV.clone(),
                    plugin_config: serde_json::json!({}),
                    plugin_config_name: "uv".to_string(),
                    // Plan 2 adds FetchReleases; Plan 3 adds ExecuteUpdate —
                    // emit only roles the descriptor currently declares.
                    roles: vec![PluginRole::DetectVersion],
                    package_identifier: None,
                    config_override: None,
                    execution_site: None,
                }];
                DiscoveredSoftware {
                    package_identifier: name.clone(),
                    name,
                    installed_version: version,
                    targets,
                    extra: None,
                    qualifier: None,
                    plugin_package_identifier: None,
                    featured: true,
                    installed_display_version: None,
                }
            })
            .collect();

        tracing::debug!(count = tools.len(), "uv tool discovery complete");
        Ok(tools)
    }

    #[tracing::instrument(skip_all)]
    async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
        // Non-zero `which` exit has a meaningful non-error interpretation
        // (uv absent), so call execute_quiet directly — the documented
        // carve-out from the execute_and_capture mandate.
        match self
            .executor
            .execute_quiet(&CommandSpec::exec("which", ["uv".to_string()]))
            .await
        {
            Ok(out) if out.exit_code == 0 => Ok(HostCompatibility::Compatible),
            _ => Ok(HostCompatibility::Incompatible(
                "uv not found in PATH".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_core::testing::{
        FixedOutputExecutor, test_runtime_with_executor,
    };
    use uptrakit_plugin_infrastructure_core::{
        Discoverer, HostCompatibility, PluginCapability, PluginError, PluginRole, plugin_ids,
    };

    use crate::config::UvConfig;

    fn make_plugin(stdout: &str) -> UvPlugin {
        UvPlugin::new(
            UvConfig::default(),
            test_runtime_with_executor(FixedOutputExecutor::success(stdout)),
        )
        .expect("construct plugin")
    }

    fn make_failing_plugin(exit_code: i32) -> UvPlugin {
        UvPlugin::new(
            UvConfig::default(),
            test_runtime_with_executor(FixedOutputExecutor::failure(exit_code)),
        )
        .expect("construct plugin")
    }

    // ── descriptor ────────────────────────────────────────────────────────

    #[test]
    fn uv_plugin_no_sudo() {
        // All `uv tool` operations are per-user; the descriptor declares no
        // sudo commands (mirrors cargo's descriptor test).
        assert!(DESCRIPTOR.sudo.is_none());
    }

    #[test]
    fn uv_plugin_capabilities() {
        for cap in [
            PluginCapability::DiscoverLocalSoftware,
            PluginCapability::DetectHostCompatibility,
            PluginCapability::VersionDetection,
            PluginCapability::ConfigTest,
        ] {
            assert!(DESCRIPTOR.capabilities.contains(&cap), "missing {cap:?}");
        }
        // Plan 2 adds ReleaseFetching + ControllerSideFetchReleases (=> 6);
        // Plan 3 adds UpdateExecution (=> 7).
        assert_eq!(DESCRIPTOR.capabilities.len(), 4);
    }

    // ── discover_software ─────────────────────────────────────────────────

    #[tokio::test]
    async fn discover_software_emits_featured_targets() {
        let output = "ruff v0.6.8\n- ruff\nblack v24.4.2\n- black\n";
        let discovered = make_plugin(output).discover_software().await.unwrap();
        assert_eq!(discovered.len(), 2);
        for item in &discovered {
            assert!(item.featured, "uv tools are individual featured items");
            assert_eq!(item.targets.len(), 1);
            let target = item.targets.first().expect("one target");
            assert_eq!(target.plugin_type, plugin_ids::PACKAGE_MANAGER_UV.clone());
            assert_eq!(target.plugin_config, serde_json::json!({}));
            assert_eq!(target.plugin_config_name, "uv");
            // Plan 2 adds FetchReleases; Plan 3 adds ExecuteUpdate.
            assert_eq!(target.roles, vec![PluginRole::DetectVersion]);
        }
        let ruff = discovered
            .iter()
            .find(|i| i.package_identifier == "ruff")
            .unwrap();
        assert_eq!(ruff.installed_version, "0.6.8");
    }

    #[tokio::test]
    async fn discover_software_empty_list_is_ok() {
        // Exit 0 + `No tools installed` on stderr (merged stream).
        let discovered = make_plugin("No tools installed\n")
            .discover_software()
            .await
            .unwrap();
        assert!(discovered.is_empty());
    }

    /// Non-empty output that is neither `No tools installed` nor parseable
    /// (future uv format drift) still returns `Ok(empty)` — the format-drift
    /// `warn!` in `discover_software` is the only signal.
    #[tokio::test]
    async fn discover_software_unparseable_output_is_ok_empty() {
        let discovered = make_plugin("something entirely different\n")
            .discover_software()
            .await
            .unwrap();
        assert!(discovered.is_empty());
    }

    /// `uv tool list` failure surfaces `PluginError::PluginInternal`:
    /// real executors return `Err(CommandError::CommandFailed)` on non-zero
    /// exit and `execute_and_capture` folds any executor `Err` into
    /// `PluginInternal`. `FixedOutputExecutor::failure` matches that contract
    /// on `execute_quiet`. Never assert `PluginError::CommandFailed` here —
    /// that arm is production-unreachable.
    #[tokio::test]
    async fn discover_software_command_failure_is_plugin_internal() {
        let Err(err) = make_failing_plugin(1).discover_software().await else {
            panic!("expected discovery to fail");
        };
        assert!(matches!(
            err.current_context(),
            PluginError::PluginInternal(_)
        ));
    }

    // ── detect_host_compatibility ─────────────────────────────────────────

    #[tokio::test]
    async fn detect_host_compatibility_compatible_when_uv_found() {
        let result = make_plugin("").detect_host_compatibility().await.unwrap();
        assert_eq!(result, HostCompatibility::Compatible);
    }

    #[tokio::test]
    async fn detect_host_compatibility_incompatible_when_uv_missing() {
        let result = make_failing_plugin(1)
            .detect_host_compatibility()
            .await
            .unwrap();
        assert!(matches!(result, HostCompatibility::Incompatible(_)));
    }

    #[test]
    fn validate_identifier_valid_names() {
        validate_identifier("ruff").unwrap();
        validate_identifier("ruamel-yaml-cmd").unwrap();
        validate_identifier("ruamel.yaml.cmd").unwrap();
        validate_identifier("typing_extensions").unwrap();
        validate_identifier("2to3").unwrap(); // digits are valid PEP 508 first chars
        validate_identifier("a").unwrap();
        validate_identifier(&"a".repeat(128)).unwrap();
    }

    #[test]
    fn validate_identifier_invalid_names() {
        validate_identifier("").unwrap_err();
        validate_identifier(&"a".repeat(129)).unwrap_err();
        validate_identifier("-ruff").unwrap_err();
        validate_identifier(".ruff").unwrap_err();
        validate_identifier("a..b").unwrap_err();
        validate_identifier("owner/pkg").unwrap_err();
        validate_identifier("pkg name").unwrap_err();
        validate_identifier("pkg==1.0").unwrap_err();
    }

    // ── parse_uv_tool_list ────────────────────────────────────────────────

    #[test]
    fn parse_uv_tool_list_basic() {
        let output = "ruff v0.6.8\n- ruff\nblack v24.4.2\n- black\n- blackd\n";
        let map = parse_uv_tool_list(output);
        assert_eq!(map.get("ruff"), Some(&"0.6.8".to_string()));
        assert_eq!(map.get("black"), Some(&"24.4.2".to_string()));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn parse_uv_tool_list_skips_entrypoint_bullets_and_indented_lines() {
        let map = parse_uv_tool_list("- ruff\n    ruff v0.6.8\n");
        assert!(map.is_empty());
    }

    /// `CommandOutput.output` merges stdout and stderr; the empty state prints
    /// `No tools installed` on stderr with exit 0. The sentinel line must not
    /// become a phantom entry alongside real output.
    #[test]
    fn parse_uv_tool_list_no_tools_installed_stderr_merge() {
        let map = parse_uv_tool_list("No tools installed\nruff v0.6.8\n");
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("ruff"), Some(&"0.6.8".to_string()));
    }

    /// Merged-stream fixture: a stderr warning line is rejected by the
    /// identifier guard alone (its trailing `foo` fragment fails the PEP
    /// 503/508 charset even though the version part `2.0` is otherwise
    /// clean), isolating that guard from the version-whitespace guard.
    #[test]
    fn parse_uv_tool_list_interleaved_warning_line() {
        let output = "warning: foo v2.0\nruff v0.6.8\n- ruff\n";
        let map = parse_uv_tool_list(output);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("ruff"), Some(&"0.6.8".to_string()));
    }

    /// PEP 440 normalized forms are not semver; the parser must accept them
    /// verbatim as the map value (no semver-shaped validation is performed).
    #[test]
    fn parse_uv_tool_list_accepts_pep440_version_forms() {
        let output = "ruff v1.0.0rc1\nblack v2.0.0b3\nmypy v1.0.post1\npoetry v1.0.dev0\nhttpie v1.0+local\n";
        let map = parse_uv_tool_list(output);
        assert_eq!(map.get("ruff"), Some(&"1.0.0rc1".to_string()));
        assert_eq!(map.get("black"), Some(&"2.0.0b3".to_string()));
        assert_eq!(map.get("mypy"), Some(&"1.0.post1".to_string()));
        assert_eq!(map.get("poetry"), Some(&"1.0.dev0".to_string()));
        assert_eq!(map.get("httpie"), Some(&"1.0+local".to_string()));
        assert_eq!(map.len(), 5);
    }

    /// `str::lines` strips a trailing `\r` from CRLF-terminated input, so
    /// Windows-style line endings must yield clean version values.
    #[test]
    fn parse_uv_tool_list_handles_crlf_line_endings() {
        let output = "ruff v0.6.8\r\nblack v24.8.0\r\n";
        let map = parse_uv_tool_list(output);
        assert_eq!(map.get("ruff"), Some(&"0.6.8".to_string()));
        assert_eq!(map.get("black"), Some(&"24.8.0".to_string()));
        assert_eq!(map.len(), 2);
    }

    /// Two lines for the same tool name: `HashMap::insert` overwrites the
    /// prior value, so the later line's version wins deliberately.
    #[test]
    fn parse_uv_tool_list_duplicate_name_last_wins() {
        let output = "ruff v0.6.8\nruff v0.6.9\n";
        let map = parse_uv_tool_list(output);
        assert_eq!(map.get("ruff"), Some(&"0.6.9".to_string()));
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn parse_uv_tool_list_rejects_trailing_content_and_non_digit_versions() {
        assert!(parse_uv_tool_list("ruff v0.6.8 (extra)\n").is_empty());
        assert!(parse_uv_tool_list("ruff vlatest\n").is_empty());
        assert!(parse_uv_tool_list("ruff v\n").is_empty());
    }

    #[test]
    fn parse_uv_tool_list_empty_input() {
        assert!(parse_uv_tool_list("").is_empty());
        assert!(parse_uv_tool_list("   \n\t\n").is_empty());
    }
}
