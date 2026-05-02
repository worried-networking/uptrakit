#![expect(
    clippy::string_slice,
    reason = "string slices use byte positions derived from ASCII-only content or fixed-length pattern matching; UTF-8 boundary safety is guaranteed by construction"
)]
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::command::{CommandExecutor, CommandSpec};
use uptrakit_plugin_infrastructure_core::helpers::validation_error_message;
use uptrakit_plugin_infrastructure_core::mpsc;
use uptrakit_plugin_infrastructure_core::{
    BatchDetectItem, BatchDetectResult, BatchFetchItem, BatchFetchResult, ConfigModel,
    ConfigTestKind, DiscoveredSoftware, DiscoveryTarget, ExecuteUpdateResult, HostCompatibility,
    HostRequirements, HostRuntime, PluginConfigValidationError, PluginError, PluginFamily,
    PluginRole, ReleaseInfo, Result, UpdateOutputLine, UpstreamRelease, Version, declare_plugin,
    execute_and_capture, plugin_ids,
};

use crate::config::MasConfig;

/// Validate a Mac App Store package identifier.
///
/// A valid identifier is:
/// - Non-empty
/// - All ASCII digits only
/// - At most 15 characters (App Store IDs are 9-10 digits as of 2025)
pub fn validate_identifier(value: &str) -> std::result::Result<(), PluginConfigValidationError> {
    if value.is_empty() {
        return Err(PluginConfigValidationError::InvalidIdentifier(
            "package_identifier must not be empty".to_string(),
        ));
    }
    if !value.chars().all(|c| c.is_ascii_digit()) {
        return Err(PluginConfigValidationError::InvalidIdentifier(
            "package_identifier must contain only digits (App Store numeric ID)".to_string(),
        ));
    }
    if value.len() > 15 {
        return Err(PluginConfigValidationError::InvalidIdentifier(
            "package_identifier is too long (App Store IDs are at most 15 digits)".to_string(),
        ));
    }
    Ok(())
}

/// Parses a single line from `mas list` output.
///
/// Format: `<id>  <name> (<installed_version>)`
///
/// Returns `(id, name, installed_version)` or `None` if the line is malformed.
fn parse_mas_list_line(line: &str) -> Option<(String, String, String)> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    // First whitespace-separated token is the numeric App Store ID.
    let (id_str, rest) = line.split_once(char::is_whitespace)?;
    if !id_str.chars().all(|c| c.is_ascii_digit()) || id_str.is_empty() {
        return None;
    }

    let rest = rest.trim();

    // The version is enclosed in the last set of parentheses on the line.
    let open_paren = rest.rfind('(')?;
    let name = rest[..open_paren].trim().to_string();
    if name.is_empty() {
        return None;
    }

    // Version string is everything between the last '(' and the closing ')'.
    let after_paren = &rest[open_paren + 1..];
    let close_paren = after_paren.find(')')?;
    let version_str = after_paren[..close_paren].trim();

    // If the version contains " -> " this is an outdated line -- skip in the list parser.
    if version_str.contains(" -> ") {
        return None;
    }

    if version_str.is_empty() {
        return None;
    }

    Some((id_str.to_string(), name, version_str.to_string()))
}

/// Parses a single line from `mas outdated` output.
///
/// Format: `<id>  <name> (<installed_version> -> <latest_version>)`
///
/// Returns `(id, latest_version)` or `None` if the line is malformed.
fn parse_mas_outdated_line(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    // First whitespace-separated token is the numeric App Store ID.
    let (id_str, rest) = line.split_once(char::is_whitespace)?;
    if !id_str.chars().all(|c| c.is_ascii_digit()) || id_str.is_empty() {
        return None;
    }

    let rest = rest.trim();

    // Find the last '(' which contains the version info.
    let open_paren = rest.rfind('(')?;
    let after_paren = &rest[open_paren + 1..];
    let close_paren = after_paren.find(')')?;
    let version_part = after_paren[..close_paren].trim();

    // Must contain " -> " to be an outdated entry.
    let arrow_pos = version_part.find(" -> ")?;
    let latest_version = version_part[arrow_pos + 4..].trim().to_string();

    if latest_version.is_empty() {
        return None;
    }

    Some((id_str.to_string(), latest_version))
}

/// Parses `mas list` output into a map of App Store ID -> installed version.
pub fn parse_mas_list(output: &str) -> HashMap<String, String> {
    output
        .lines()
        .filter_map(parse_mas_list_line)
        .map(|(id, _name, version)| (id, version))
        .collect()
}

/// Parses `mas outdated` output into a map of App Store ID -> latest available version.
pub fn parse_mas_outdated(output: &str) -> HashMap<String, String> {
    output.lines().filter_map(parse_mas_outdated_line).collect()
}

/// Plugin for the Mac App Store via the `mas` CLI tool.
///
/// The `package_identifier` for a tracked App Store app is its numeric App Store ID
/// (e.g. `497799835` for Xcode). Install `mas` via Homebrew: `brew install mas`.
///
/// Version detection and updates run on the agent host; no controller-side
/// network access is required.
pub struct MasPlugin {
    /// Stored for registry dispatch and forward-compatibility with future config fields.
    /// `MasConfig` currently has no data fields, so this is never read after construction.
    _config: MasConfig,
    executor: Arc<dyn CommandExecutor>,
}

impl MasPlugin {
    /// Create a new `mas` plugin with the given configuration and host runtime.
    pub fn new(
        config: MasConfig,
        runtime: Arc<dyn HostRuntime>,
    ) -> std::result::Result<Self, String> {
        let executor = runtime.executor();
        Ok(Self {
            _config: config,
            executor,
        })
    }

    /// Run `mas list` and return the raw output.
    async fn run_mas_list(&self) -> Result<String> {
        execute_and_capture(
            self.executor.as_ref(),
            CommandSpec::exec("mas", ["list".to_string()]),
            "mas list",
        )
        .await
    }

    /// Run `mas outdated` and return the raw output.
    async fn run_mas_outdated(&self) -> Result<String> {
        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec("mas", ["outdated".to_string()]))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "mas outdated failed: {e}"
                )))
            })?;

        // `mas outdated` exits with code 0 even when there are no outdated apps.
        // A non-zero exit is unusual but not fatal -- treat as empty outdated list.
        if cmd_output.exit_code != 0 {
            tracing::warn!(
                exit_code = cmd_output.exit_code,
                "mas outdated exited with non-zero code; treating as empty outdated list"
            );
            return Ok(String::new());
        }

        Ok(cmd_output.output)
    }

    fn require_package_identifier(&self, package_identifier: &str) -> Result<()> {
        uptrakit_plugin_infrastructure_core::require_package_identifier(
            package_identifier,
            validate_identifier,
        )
    }
}

// ── Plugin descriptor ─────────────────────────────────────────────────────

declare_plugin!(MasPlugin, MasConfig, "package_manager_mas", {
    display_name: "Mac App Store",
    family: PluginFamily::Software,
    config_model: ConfigModel::PluginConfig,
    host_requirements: HostRequirements::POSIX,
    config_test: [ConfigTestKind::VersionDetection, ConfigTestKind::UpdateCommandValidation],
    roles: [Discoverer, VersionDetector, ReleaseFetcher, UpdateExecutor],
});

#[async_trait]
impl uptrakit_plugin_infrastructure_core::Discoverer for MasPlugin {
    #[tracing::instrument(skip_all)]
    async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> {
        tracing::debug!("discovering installed Mac App Store apps via mas list");

        let list_output = self.run_mas_list().await?;

        let items: Vec<DiscoveredSoftware> = list_output
            .lines()
            .filter_map(parse_mas_list_line)
            .map(|(id, name, installed_version)| {
                let target = DiscoveryTarget {
                    plugin_type: plugin_ids::PACKAGE_MANAGER_MAS.clone(),
                    plugin_config: serde_json::json!({}),
                    plugin_config_name: "Mac App Store".to_string(),
                    roles: vec![
                        PluginRole::DetectVersion,
                        PluginRole::FetchReleases,
                        PluginRole::ExecuteUpdate,
                    ],
                    package_identifier: None,
                    config_override: None,
                    execution_site: None,
                };
                DiscoveredSoftware {
                    package_identifier: id,
                    name,
                    installed_version,
                    targets: vec![target],
                    extra: None,
                    qualifier: None,
                    plugin_package_identifier: None,
                    featured: false,
                    installed_display_version: None,
                }
            })
            .collect();

        tracing::debug!(count = items.len(), "Mac App Store app discovery complete");
        Ok(items)
    }

    #[tracing::instrument(skip_all)]
    async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
        match self
            .executor
            .execute_quiet(&CommandSpec::exec("which", ["mas".to_string()]))
            .await
        {
            Ok(_) => Ok(HostCompatibility::Compatible),
            Err(_) => Ok(HostCompatibility::Incompatible("mas not found".to_string())),
        }
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::VersionDetector for MasPlugin {
    #[tracing::instrument(skip_all)]
    async fn detect_installed_version(&self, package_identifier: &str) -> Result<Option<Version>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "detecting installed mas version");

        let list_output = self.run_mas_list().await?;
        let installed_map = parse_mas_list(&list_output);

        let version = installed_map.get(package_identifier).map(Version::new);

        tracing::debug!(
            package = %package_identifier,
            version = ?version,
            "mas version detection result"
        );
        Ok(version)
    }

    #[tracing::instrument(skip_all)]
    async fn batch_detect(&self, items: &[BatchDetectItem]) -> Result<Vec<BatchDetectResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        // Validate all identifiers up front.
        for item in items {
            validate_identifier(&item.package_identifier)
                .map_err(|e| report!(PluginError::Configuration(validation_error_message(e))))?;
        }

        tracing::debug!(
            count = items.len(),
            "batch detecting mas installed versions"
        );

        let list_output = self.run_mas_list().await?;
        let installed_map = parse_mas_list(&list_output);

        let results = items
            .iter()
            .map(|item| {
                let installed_version = installed_map
                    .get(&item.package_identifier)
                    .map(Version::new);
                BatchDetectResult::new(item.package_identifier.clone(), installed_version, None)
            })
            .collect();

        tracing::debug!(count = items.len(), "mas batch version detection complete");
        Ok(results)
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::ReleaseFetcher for MasPlugin {
    #[tracing::instrument(skip_all)]
    async fn fetch_releases(&self, package_identifier: &str) -> Result<Vec<UpstreamRelease>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "fetching mas releases");

        let list_output = self.run_mas_list().await?;
        let outdated_output = self.run_mas_outdated().await?;

        let installed_map = parse_mas_list(&list_output);
        let outdated_map = parse_mas_outdated(&outdated_output);

        let latest_version = if let Some(v) = outdated_map.get(package_identifier) {
            v.clone()
        } else if let Some(v) = installed_map.get(package_identifier) {
            v.clone()
        } else {
            bail!(PluginError::PluginInternal(format!(
                "package not found: {package_identifier}"
            )));
        };

        let release_url = format!("https://apps.apple.com/app/id{package_identifier}");

        let releases = vec![{
            let mut r =
                UpstreamRelease::new(Version::new(&latest_version), latest_version, false, "");
            r.release_url = release_url;
            r
        }];

        tracing::debug!(
            package = %package_identifier,
            count = releases.len(),
            "mas releases fetched"
        );
        Ok(releases)
    }

    #[tracing::instrument(skip_all)]
    async fn batch_fetch(&self, items: &[BatchFetchItem]) -> Result<Vec<BatchFetchResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        // Validate all identifiers up front.
        for item in items {
            validate_identifier(&item.package_identifier)
                .map_err(|e| report!(PluginError::Configuration(validation_error_message(e))))?;
        }

        tracing::debug!(count = items.len(), "batch fetching mas releases");

        let list_output = self.run_mas_list().await?;
        let outdated_output = self.run_mas_outdated().await?;

        let installed_map = parse_mas_list(&list_output);
        let outdated_map = parse_mas_outdated(&outdated_output);

        let results = items
            .iter()
            .map(|item| {
                let id = &item.package_identifier;
                let latest_version = outdated_map
                    .get(id)
                    .or_else(|| installed_map.get(id))
                    .cloned();

                match latest_version {
                    Some(v) => {
                        let release_url = format!("https://apps.apple.com/app/id{id}");
                        BatchFetchResult::found(
                            id.clone(),
                            vec![{
                                let mut r = UpstreamRelease::new(Version::new(&v), v, false, "");
                                r.release_url = release_url;
                                r
                            }],
                        )
                    }
                    None => BatchFetchResult::error(id.clone(), format!("package not found: {id}")),
                }
            })
            .collect();

        tracing::debug!(count = items.len(), "mas batch fetch complete");
        Ok(results)
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::UpdateExecutor for MasPlugin {
    #[tracing::instrument(skip_all)]
    async fn execute_update(
        &self,
        package_identifier: &str,
        _to_version: &str,
        _release_info: Option<&ReleaseInfo>,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<ExecuteUpdateResult> {
        self.require_package_identifier(package_identifier)?;

        tracing::debug!(package = %package_identifier, "running mas upgrade");

        let output = uptrakit_plugin_infrastructure_core::execute_command_update(
            uptrakit_plugin_infrastructure_core::CommandUpdateParams {
                executor: self.executor.as_ref(),
                binary: "mas",
                args: vec!["upgrade".to_string(), package_identifier.to_string()],
                privileged: false,
                spec_modifier: None,
                exit_code_success: Some(|_| true),
                exit_code_error: None,
            },
            output_tx,
        )
        .await?;
        Ok(ExecuteUpdateResult::new(output, false))
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions use assert!(result.is_ok()) pattern"
    )]
    use super::*;
    use uptrakit_plugin_infrastructure_core::{
        CommandOutput, HostCapabilities, StandardHostRuntime, UpdateOutputLine, mpsc::Sender,
    };

    // ─── Mock executor ───────────────────────────────────────────────────────

    /// A configurable mock executor that returns preset output for `mas` sub-commands.
    struct MockMasExecutor {
        /// Output returned for `mas list`.
        list_output: String,
        /// Output returned for `mas outdated`.
        outdated_output: String,
        /// Exit code for `which mas` (0 = found, non-zero = not found).
        which_exit_code: i32,
    }

    impl MockMasExecutor {
        fn new(list_output: &str, outdated_output: &str) -> Self {
            Self {
                list_output: list_output.to_string(),
                outdated_output: outdated_output.to_string(),
                which_exit_code: 0,
            }
        }

        fn incompatible(list_output: &str) -> Self {
            Self {
                list_output: list_output.to_string(),
                outdated_output: String::new(),
                which_exit_code: 1,
            }
        }
    }

    #[async_trait::async_trait]
    impl CommandExecutor for MockMasExecutor {
        async fn execute(
            &self,
            spec: &CommandSpec,
            _output_tx: &Sender<UpdateOutputLine>,
        ) -> uptrakit_command::Result<CommandOutput> {
            self.execute_quiet(spec).await
        }

        async fn execute_quiet(
            &self,
            spec: &CommandSpec,
        ) -> uptrakit_command::Result<CommandOutput> {
            use uptrakit_plugin_infrastructure_core::command::CommandMode;
            let (program, args) = match &spec.mode {
                CommandMode::Exec { program, args } => (
                    program.as_str(),
                    args.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                ),
                _ => {
                    return Ok(CommandOutput {
                        output: String::new(),
                        exit_code: 127,
                    });
                }
            };
            let output = match (program, args.as_slice()) {
                ("which", ["mas"]) => {
                    if self.which_exit_code != 0 {
                        return Err(report!(
                            uptrakit_command::error::CommandError::CommandFailed(
                                self.which_exit_code
                            )
                        ));
                    }
                    CommandOutput {
                        output: "/opt/homebrew/bin/mas\n".to_string(),
                        exit_code: 0,
                    }
                }
                ("mas", ["list"]) => CommandOutput {
                    output: self.list_output.clone(),
                    exit_code: 0,
                },
                ("mas", ["outdated"]) => CommandOutput {
                    output: self.outdated_output.clone(),
                    exit_code: 0,
                },
                ("mas", _) => CommandOutput {
                    output: String::new(),
                    exit_code: 0,
                },
                _ => CommandOutput {
                    output: String::new(),
                    exit_code: 127,
                },
            };
            Ok(output)
        }
    }

    fn make_plugin_from_executor(executor: Arc<dyn CommandExecutor>) -> MasPlugin {
        let caps = HostCapabilities::default();
        let runtime = Arc::new(StandardHostRuntime::new(executor, caps)) as Arc<dyn HostRuntime>;
        MasPlugin::new(MasConfig::default(), runtime).unwrap()
    }

    fn make_plugin(list_output: &str, outdated_output: &str) -> MasPlugin {
        let executor = Arc::new(MockMasExecutor::new(list_output, outdated_output));
        make_plugin_from_executor(executor)
    }

    const SAMPLE_LIST: &str = "\
497799835  Xcode (15.4)
1147396723  WhatsApp (24.23.82)
408981434   iMovie (10.3.9)
";

    const SAMPLE_OUTDATED: &str = "\
497799835  Xcode (15.4 -> 16.0)
";

    // ─── validate_identifier ────────────────────────────────────────────────

    #[test]
    fn validate_identifier_accepts_valid_ids() {
        assert!(validate_identifier("497799835").is_ok());
        assert!(validate_identifier("1147396723").is_ok());
        assert!(validate_identifier("1").is_ok());
        assert!(validate_identifier("123456789012345").is_ok()); // 15 digits
    }

    #[test]
    fn validate_identifier_rejects_empty() {
        assert!(validate_identifier("").is_err());
    }

    #[test]
    fn validate_identifier_rejects_non_digits() {
        assert!(validate_identifier("xcode").is_err());
        assert!(validate_identifier("497799835a").is_err());
        assert!(validate_identifier("497-799835").is_err());
        assert!(validate_identifier(" 497799835").is_err());
    }

    #[test]
    fn validate_identifier_rejects_too_long() {
        // 16 digits -- over the 15-char limit
        assert!(validate_identifier("1234567890123456").is_err());
    }

    // ─── parse_mas_list ─────────────────────────────────────────────────────

    #[test]
    fn parse_mas_list_standard_output() {
        let map = parse_mas_list(SAMPLE_LIST);
        assert_eq!(map.get("497799835").map(String::as_str), Some("15.4"));
        assert_eq!(map.get("1147396723").map(String::as_str), Some("24.23.82"));
        assert_eq!(map.get("408981434").map(String::as_str), Some("10.3.9"));
    }

    #[test]
    fn parse_mas_list_multi_word_names() {
        let output = "1234567890  My Cool App (2.0.1)\n";
        let map = parse_mas_list(output);
        assert_eq!(map.get("1234567890").map(String::as_str), Some("2.0.1"));
    }

    #[test]
    fn parse_mas_list_ignores_empty_lines() {
        let output = "\n497799835  Xcode (15.4)\n\n";
        let map = parse_mas_list(output);
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn parse_mas_list_skips_outdated_format() {
        let output = "497799835  Xcode (15.4 -> 16.0)\n";
        let map = parse_mas_list(output);
        assert!(map.is_empty());
    }

    // ─── parse_mas_outdated ─────────────────────────────────────────────────

    #[test]
    fn parse_mas_outdated_standard_output() {
        let map = parse_mas_outdated(SAMPLE_OUTDATED);
        assert_eq!(map.get("497799835").map(String::as_str), Some("16.0"));
    }

    #[test]
    fn parse_mas_outdated_empty_output() {
        let map = parse_mas_outdated("");
        assert!(map.is_empty());
    }

    #[test]
    fn parse_mas_outdated_multi_entry() {
        let output = "\
497799835  Xcode (15.4 -> 16.0)
1147396723  WhatsApp (24.23.82 -> 24.25.0)
";
        let map = parse_mas_outdated(output);
        assert_eq!(map.get("497799835").map(String::as_str), Some("16.0"));
        assert_eq!(map.get("1147396723").map(String::as_str), Some("24.25.0"));
    }

    // ─── discover_software ──────────────────────────────────────────────────

    #[tokio::test]
    async fn discover_software_returns_all_apps() {
        use uptrakit_plugin_infrastructure_core::Discoverer;
        let plugin = make_plugin(SAMPLE_LIST, "");
        let discovered = plugin.discover_software().await.expect("discover");

        assert_eq!(discovered.len(), 3);

        let xcode = discovered
            .iter()
            .find(|d| d.package_identifier == "497799835")
            .expect("Xcode");
        assert_eq!(xcode.name, "Xcode");
        assert_eq!(xcode.installed_version, "15.4");
        assert_eq!(xcode.targets.len(), 1);
        assert_eq!(
            xcode.targets[0].plugin_type,
            plugin_ids::PACKAGE_MANAGER_MAS.clone()
        );
        assert_eq!(xcode.targets[0].plugin_config_name, "Mac App Store");
        assert_eq!(xcode.targets[0].plugin_config, serde_json::json!({}));
        assert!(xcode.targets[0].roles.contains(&PluginRole::DetectVersion));
        assert!(xcode.targets[0].roles.contains(&PluginRole::FetchReleases));
        assert!(xcode.targets[0].roles.contains(&PluginRole::ExecuteUpdate));
    }

    #[tokio::test]
    async fn discover_software_empty_list() {
        use uptrakit_plugin_infrastructure_core::Discoverer;
        let plugin = make_plugin("", "");
        let discovered = plugin.discover_software().await.expect("discover");
        assert!(discovered.is_empty());
    }

    // ─── batch_detect_installed_version ────────────────────────────────────

    #[tokio::test]
    async fn batch_detect_returns_correct_versions() {
        use uptrakit_plugin_infrastructure_core::VersionDetector;
        let plugin = make_plugin(SAMPLE_LIST, "");
        let items = vec![
            BatchDetectItem::new("497799835".to_string()),
            BatchDetectItem::new("1147396723".to_string()),
        ];
        let results = plugin.batch_detect(&items).await.expect("batch detect");
        assert_eq!(results.len(), 2);

        let xcode_result = results
            .iter()
            .find(|r| r.package_identifier == "497799835")
            .expect("Xcode");
        assert!(xcode_result.error.is_none());
        assert_eq!(
            xcode_result.installed_version.as_ref().map(|v| v.as_str()),
            Some("15.4")
        );
    }

    #[tokio::test]
    async fn batch_detect_returns_none_for_unknown_id() {
        use uptrakit_plugin_infrastructure_core::VersionDetector;
        let plugin = make_plugin(SAMPLE_LIST, "");
        let items = vec![BatchDetectItem::new("999999999".to_string())];
        let results = plugin.batch_detect(&items).await.expect("batch detect");
        assert_eq!(results.len(), 1);
        assert!(results[0].installed_version.is_none());
        assert!(results[0].error.is_none());
    }

    #[tokio::test]
    async fn batch_detect_empty_items() {
        use uptrakit_plugin_infrastructure_core::VersionDetector;
        let plugin = make_plugin(SAMPLE_LIST, "");
        let results = plugin.batch_detect(&[]).await.expect("batch detect");
        assert!(results.is_empty());
    }

    // ─── batch_fetch_releases ───────────────────────────────────────────────

    #[tokio::test]
    async fn batch_fetch_outdated_path() {
        use uptrakit_plugin_infrastructure_core::ReleaseFetcher;
        let plugin = make_plugin(SAMPLE_LIST, SAMPLE_OUTDATED);
        let items = vec![BatchFetchItem::new("497799835".to_string())];
        let results = plugin.batch_fetch(&items).await.expect("batch fetch");
        assert_eq!(results.len(), 1);
        assert!(results[0].error.is_none());
        assert_eq!(results[0].releases.len(), 1);
        assert_eq!(results[0].releases[0].tag, "16.0");
        assert_eq!(
            results[0].releases[0].release_url,
            "https://apps.apple.com/app/id497799835"
        );
    }

    #[tokio::test]
    async fn batch_fetch_up_to_date_path() {
        use uptrakit_plugin_infrastructure_core::ReleaseFetcher;
        let plugin = make_plugin(SAMPLE_LIST, SAMPLE_OUTDATED);
        let items = vec![BatchFetchItem::new("1147396723".to_string())];
        let results = plugin.batch_fetch(&items).await.expect("batch fetch");
        assert_eq!(results.len(), 1);
        assert!(results[0].error.is_none());
        assert_eq!(results[0].releases.len(), 1);
        assert_eq!(results[0].releases[0].tag, "24.23.82");
    }

    #[tokio::test]
    async fn batch_fetch_unknown_id_returns_error() {
        use uptrakit_plugin_infrastructure_core::ReleaseFetcher;
        let plugin = make_plugin(SAMPLE_LIST, "");
        let items = vec![BatchFetchItem::new("999999999".to_string())];
        let results = plugin.batch_fetch(&items).await.expect("batch fetch");
        assert_eq!(results.len(), 1);
        assert!(results[0].error.is_some());
        assert!(results[0].releases.is_empty());
    }

    // ─── detect_host_compatibility ──────────────────────────────────────────

    #[tokio::test]
    async fn detect_host_compatibility_compatible() {
        use uptrakit_plugin_infrastructure_core::Discoverer;
        let plugin = make_plugin(SAMPLE_LIST, "");
        let compat = plugin
            .detect_host_compatibility()
            .await
            .expect("compatibility");
        assert!(matches!(compat, HostCompatibility::Compatible));
    }

    #[tokio::test]
    async fn detect_host_compatibility_incompatible() {
        use uptrakit_plugin_infrastructure_core::Discoverer;
        let plugin = make_plugin_from_executor(Arc::new(MockMasExecutor::incompatible("")));
        let compat = plugin
            .detect_host_compatibility()
            .await
            .expect("compatibility");
        assert!(matches!(compat, HostCompatibility::Incompatible(_)));
    }
}
