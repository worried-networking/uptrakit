use std::sync::Arc;

use async_trait::async_trait;
use uptrakit_plugin_infrastructure_core::command::{CommandExecutor, CommandSpec};
use uptrakit_plugin_infrastructure_core::{
    DiscoveredSoftware, DiscoveryTarget, Plugin, PluginCapability, PluginRole, PluginType,
    SudoCommandEntry, SudoHelperScript,
};

use crate::config::ProxmoxHelperScriptsConfig;
use crate::discovery::{
    UPDATE_SCRIPT_PATH, analyze_phs_script, extract_apt_package_candidates, parse_phs_scripts,
    parse_version_file, slug_to_display_name,
};

/// Absolute path where the PHS version helper script is installed on managed hosts.
///
/// This path is used both for the sudoers entry and as the command in the Shell
/// plugin's `version_command` config. It is installed during host bootstrap by the
/// sudoers generation machinery via [`SudoHelperScript`].
const PHS_VERSION_HELPER_PATH: &str = "/usr/local/bin/uptrakit-phs-version";

/// Content of the PHS version helper script, embedded at compile time.
///
/// The script validates its slug argument (must be `[a-z0-9][a-z0-9-]*`) before
/// reading `/root/.<slug>`, providing argument-level restriction that sudoers
/// wildcards cannot express safely (sudoers `*` matches `/`, making path-based
/// wildcard restrictions ineffective).
const PHS_VERSION_HELPER_CONTENT: &str = include_str!("phs_version.sh");

/// Shell command to detect the installed version of a GitHub-managed PHS app.
///
/// PHS scripts execute via `pct exec` as root and write their version files
/// under `/root/.<slug>`. This command calls the dedicated helper script
/// (`uptrakit-phs-version`) which validates the slug before accessing the file,
/// preventing any path traversal.
///
/// `sudo` is embedded in the command string because the Shell plugin executes
/// version commands through [`CommandSpec::shell`], which does not support the
/// `.privileged()` flag — shell commands must handle their own privilege
/// escalation. The corresponding sudoers entry is:
///
/// ```text
/// uptrakit ALL=(root) NOPASSWD: /usr/local/bin/uptrakit-phs-version
/// ```
///
/// `{package_identifier}` is the PHS slug (shell-escaped at runtime by the
/// Shell plugin's `detect_installed_version()` implementation).
const PHS_DETECT_VERSION_CMD: &str =
    "sudo /usr/local/bin/uptrakit-phs-version {package_identifier}";

/// Install command for PHS-managed apps.
///
/// Uses the unattended mode (`PHS_SILENT=1`) exactly as the official
/// `update-apps.sh` PVE tool does via `pct exec`, so the update runs without
/// interactive prompts and without requiring a network fetch of the script.
const PHS_INSTALL_CMD: &str = "env PHS_SILENT=1 /usr/bin/update";

/// Capabilities: discovery only — no release-index refresh needed.
const CAPABILITIES: &[PluginCapability] = &[PluginCapability::DiscoverLocalSoftware];

/// Plugin for Proxmox Helper Scripts (discovery-only).
///
/// Discovers PHS-managed software by:
/// 1. Reading `/usr/bin/update` and parsing CT script URLs from it.
/// 2. Fetching each CT script from `raw.githubusercontent.com` and analysing it
///    to determine whether the app is GitHub-managed or APT-managed.
/// 3. Emitting `DiscoveredSoftware` items with structured `targets` that tell
///    the controller exactly which plugin configs to create:
///    - GitHub-managed → two `DiscoveryTarget`s: one `GithubReleases`
///      (FetchReleases only, with `owner/repo` as `package_identifier`) and
///      one `Shell` (DetectVersion + ExecuteUpdate using PHS conventions).
///    - APT-managed → one `DiscoveryTarget` with `Apt` plugin type.
///
/// The controller processes targets generically without any PHS-specific logic.
pub struct ProxmoxHelperScriptsPlugin {
    _config: ProxmoxHelperScriptsConfig,
    executor: Arc<dyn CommandExecutor>,
    client: reqwest::Client,
}

impl ProxmoxHelperScriptsPlugin {
    /// Create a new Proxmox Helper Scripts plugin.
    pub fn new(
        config: ProxmoxHelperScriptsConfig,
        executor: Arc<dyn CommandExecutor>,
    ) -> uptrakit_plugin_infrastructure_core::Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!(
                "uptrakit-plugin-discovery-proxmox-helper-scripts/",
                env!("CARGO_PKG_VERSION")
            ))
            .build()
            .map_err(|e| {
                rootcause::report!(
                    uptrakit_plugin_infrastructure_core::PluginError::PluginInternal(format!(
                        "failed to build HTTP client: {e}"
                    ))
                )
            })?;

        Ok(Self {
            _config: config,
            executor,
            client,
        })
    }

    /// Fetch the body of a URL as text, returning `None` on any error.
    async fn fetch_text(&self, url: &str) -> Option<String> {
        let response = self.client.get(url).send().await.ok()?;
        if !response.status().is_success() {
            tracing::warn!(url, status = %response.status(), "HTTP fetch failed");
            return None;
        }
        response.text().await.ok()
    }

    /// Run the PHS version helper script with `version_file_basename`.
    ///
    /// The helper script (`uptrakit-phs-version`) validates the argument before
    /// reading `/root/.<version_file_basename>`, so this call is both correct
    /// and safe — no path traversal is possible.
    ///
    /// The `version_file_basename` is normally the container slug, but for apps
    /// where the `check_for_gh_release` key differs from the slug (e.g.
    /// Paperless-ngx uses key `"paperless"` for slug `"paperless-ngx"`), it
    /// must be the key instead.
    ///
    /// Returns the installed version string, or `None` if the helper is not
    /// installed, the version file does not exist, or the output is unparseable.
    async fn phs_version(&self, version_file_basename: &str) -> Option<String> {
        let output = self
            .executor
            .execute_quiet(
                &CommandSpec::exec(
                    PHS_VERSION_HELPER_PATH,
                    [version_file_basename.to_string()],
                )
                .privileged(),
            )
            .await
            .ok()?;

        parse_version_file(&output.output).map(String::from)
    }

    /// Run `dpkg-query` to detect the installed version of a Debian package.
    /// Returns `None` if the package is not installed or the command fails.
    async fn dpkg_version(&self, apt_package: &str) -> Option<String> {
        let output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "dpkg-query",
                [
                    "-W".to_string(),
                    "-f=${Version}".to_string(),
                    apt_package.to_string(),
                ],
            ))
            .await
            .ok()?;
        let v = output.output.trim().to_string();
        if v.is_empty() { None } else { Some(v) }
    }

    /// Build a `DiscoveryTarget` for the GitHub releases role.
    ///
    /// The plugin config carries only GitHub-level settings (no `owner`/`repo`).
    /// The `owner/repo` pair is expressed as the `package_identifier` override
    /// so the controller routes release queries to the right repo while sharing
    /// a single plugin config instance across all tracked GitHub repos.
    fn github_fetch_target(owner: &str, repo: &str) -> DiscoveryTarget {
        DiscoveryTarget {
            plugin_type: PluginType::ReleasesGithub,
            plugin_config: serde_json::json!({
                "tag_strip_prefix": "v",
                "include_prereleases": false,
                "asset_patterns": [],
            }),
            plugin_config_name: "GitHub Releases".to_string(),
            roles: vec![PluginRole::FetchReleases],
            package_identifier: Some(format!("{owner}/{repo}")),
            config_override: None,
            execution_site: None,
        }
    }

    /// Build a `DiscoveryTarget` for the Shell plugin covering both
    /// `DetectVersion` and `ExecuteUpdate` using PHS conventions.
    ///
    /// When `version_file_basename` is `Some`, it is set as the
    /// `package_identifier` override on the target. The Shell plugin expands
    /// `{package_identifier}` in the `version_command` at runtime, so the
    /// helper script will be invoked as:
    ///
    /// ```text
    /// sudo /usr/local/bin/uptrakit-phs-version <version_file_basename>
    /// ```
    ///
    /// which reads `/root/.<version_file_basename>` — the correct version file
    /// for apps like Paperless-ngx where the `check_for_gh_release` key
    /// (`"paperless"`) differs from the container slug (`"paperless-ngx"`).
    ///
    /// When `None`, `{package_identifier}` resolves to the software item's own
    /// `package_identifier` (the container slug), which is correct for the
    /// common case where key == slug.
    fn phs_shell_target(version_file_basename: Option<&str>) -> DiscoveryTarget {
        DiscoveryTarget {
            plugin_type: PluginType::GenericShell,
            plugin_config: serde_json::json!({
                "version_command": PHS_DETECT_VERSION_CMD,
                "update_command": PHS_INSTALL_CMD,
            }),
            plugin_config_name: "PHS Shell".to_string(),
            roles: vec![PluginRole::DetectVersion, PluginRole::ExecuteUpdate],
            package_identifier: version_file_basename.map(str::to_string),
            config_override: None,
            execution_site: None,
        }
    }

    /// Build a `DiscoveryTarget` for an APT-managed PHS app.
    fn apt_target() -> DiscoveryTarget {
        DiscoveryTarget {
            plugin_type: PluginType::PackageManagerApt,
            plugin_config: serde_json::json!({}),
            plugin_config_name: "APT (auto)".to_string(),
            roles: vec![
                PluginRole::DetectVersion,
                PluginRole::FetchReleases,
                PluginRole::ExecuteUpdate,
            ],
            package_identifier: None,
            config_override: None,
            execution_site: None,
        }
    }
}

#[async_trait]
impl Plugin for ProxmoxHelperScriptsPlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::DiscoveryProxmoxHelperScripts
    }

    fn capabilities(&self) -> &'static [PluginCapability] {
        CAPABILITIES
    }

    fn required_sudo_commands(&self) -> Vec<SudoCommandEntry> {
        vec![SudoCommandEntry {
            command: "uptrakit-phs-version".into(),
            explanation: "Reads /root/.<slug> for PHS version detection; the helper script \
                validates the slug argument to prevent path traversal"
                .into(),
            helper_script: Some(SudoHelperScript {
                install_path: PHS_VERSION_HELPER_PATH,
                content: PHS_VERSION_HELPER_CONTENT,
            }),
        }]
    }

    async fn discover_software(
        &self,
    ) -> uptrakit_plugin_infrastructure_core::Result<Vec<DiscoveredSoftware>> {
        tracing::debug!("reading PHS update script from {UPDATE_SCRIPT_PATH}");

        let update_content = match self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "cat",
                ["--".to_string(), UPDATE_SCRIPT_PATH.to_string()],
            ))
            .await
        {
            Ok(output) => output.output,
            Err(_) => {
                tracing::debug!("no PHS update script found at {UPDATE_SCRIPT_PATH}");
                return Ok(vec![]);
            }
        };

        let scripts = parse_phs_scripts(&update_content);
        if scripts.is_empty() {
            tracing::debug!("no PHS script references found in {UPDATE_SCRIPT_PATH}");
            return Ok(vec![]);
        }

        let mut discovered = Vec::new();

        for script in &scripts {
            // Fetch CT script body.
            let Some(body) = self.fetch_text(&script.script_url).await else {
                tracing::warn!(slug = %script.slug, url = %script.script_url,
                    "failed to fetch CT script; skipping");
                continue;
            };

            let analysis = analyze_phs_script(&script.slug, &body);
            let display_name = analysis
                .app_name
                .clone()
                .unwrap_or_else(|| slug_to_display_name(&script.slug));

            if let (Some(owner), Some(repo)) = (&analysis.github_owner, &analysis.github_repo) {
                // GitHub-managed: read version via the helper script.
                // Use the version file basename from the analysis (which may differ
                // from the slug when the check_for_gh_release key differs, e.g.
                // Paperless-ngx uses key "paperless" → /root/.paperless).
                let vfb = analysis
                    .version_file_basename
                    .as_deref()
                    .unwrap_or(&script.slug);
                let Some(installed_version) = self.phs_version(vfb).await else {
                    tracing::debug!(slug = %script.slug,
                        "PHS version helper absent or version file absent; skipping GitHub item");
                    continue;
                };

                tracing::debug!(
                    slug = %script.slug,
                    version_file_basename = %vfb,
                    version = %installed_version,
                    owner = %owner,
                    repo = %repo,
                    "discovered GitHub-managed PHS software"
                );

                discovered.push(DiscoveredSoftware {
                    package_identifier: script.slug.clone(),
                    name: display_name,
                    installed_version,
                    targets: vec![
                        Self::github_fetch_target(owner, repo),
                        Self::phs_shell_target(analysis.version_file_basename.as_deref()),
                    ],
                    extra: None,
                });
            } else if let Some(ref apt_pkg) = analysis.apt_package {
                // APT direct: verify installed via dpkg-query.
                let Some(installed_version) = self.dpkg_version(apt_pkg).await else {
                    tracing::debug!(slug = %script.slug, package = %apt_pkg,
                        "APT package not installed; skipping");
                    continue;
                };

                tracing::debug!(
                    slug = %script.slug,
                    package = %apt_pkg,
                    version = %installed_version,
                    "discovered APT-managed PHS software"
                );

                discovered.push(DiscoveredSoftware {
                    package_identifier: apt_pkg.clone(),
                    name: display_name,
                    installed_version,
                    targets: vec![Self::apt_target()],
                    extra: None,
                });
            } else {
                // Neither — try install-script fallback.
                use crate::discovery::PHS_INSTALL_URL_PREFIX;
                let install_url = format!("{PHS_INSTALL_URL_PREFIX}{}-install.sh", script.slug);
                let Some(install_body) = self.fetch_text(&install_url).await else {
                    tracing::warn!(slug = %script.slug,
                        "install-script fetch failed; skipping");
                    continue;
                };

                let candidates = extract_apt_package_candidates(&install_body);
                if candidates.is_empty() {
                    tracing::warn!(
                        slug = %script.slug,
                        "no APT candidates from install script; skipping"
                    );
                    continue;
                }

                let mut found_any = false;
                for candidate in &candidates {
                    let Some(installed_version) = self.dpkg_version(candidate).await else {
                        continue;
                    };

                    tracing::debug!(
                        slug = %script.slug,
                        package = %candidate,
                        version = %installed_version,
                        "discovered install-script fallback PHS software"
                    );

                    discovered.push(DiscoveredSoftware {
                        package_identifier: candidate.clone(),
                        name: display_name.clone(),
                        installed_version,
                        targets: vec![Self::apt_target()],
                        extra: None,
                    });
                    found_any = true;
                }

                if !found_any {
                    tracing::warn!(
                        slug = %script.slug,
                        candidates = ?candidates,
                        "no install-script APT candidates are installed; skipping"
                    );
                }
            }
        }

        Ok(discovered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_core::LocalCommandExecutor;

    fn test_executor() -> Arc<dyn CommandExecutor> {
        Arc::new(LocalCommandExecutor)
    }

    #[test]
    fn capabilities_discovery_only() {
        let plugin =
            ProxmoxHelperScriptsPlugin::new(ProxmoxHelperScriptsConfig::default(), test_executor())
                .expect("create");
        assert!(plugin.has_capability(PluginCapability::DiscoverLocalSoftware));
        assert!(!plugin.has_capability(PluginCapability::RefreshPackageIndex));
        assert_eq!(plugin.capabilities().len(), 1);
    }

    #[test]
    fn plugin_type_is_proxmox_helper_scripts() {
        let plugin =
            ProxmoxHelperScriptsPlugin::new(ProxmoxHelperScriptsConfig::default(), test_executor())
                .expect("create");
        assert_eq!(
            plugin.plugin_type(),
            PluginType::DiscoveryProxmoxHelperScripts
        );
    }

    #[tokio::test]
    async fn discover_software_returns_empty_without_update_script() {
        // On a non-PHS system /usr/bin/update likely does not exist.
        let plugin =
            ProxmoxHelperScriptsPlugin::new(ProxmoxHelperScriptsConfig::default(), test_executor())
                .expect("create");
        let result = plugin.discover_software().await;
        assert!(result.is_ok());
        // No error; result is empty or whatever is found on the test machine.
    }

    #[test]
    fn github_fetch_target_structure() {
        let target = ProxmoxHelperScriptsPlugin::github_fetch_target("BookLore", "BookLore");
        assert_eq!(target.plugin_type, PluginType::ReleasesGithub);
        assert_eq!(target.plugin_config_name, "GitHub Releases");
        // FetchReleases only — no agent-side roles.
        assert_eq!(target.roles.len(), 1);
        assert_eq!(target.roles[0], PluginRole::FetchReleases);
        // No owner/repo in config.
        assert!(target.plugin_config.get("owner").is_none());
        assert!(target.plugin_config.get("repo").is_none());
        // package_identifier carries the "owner/repo" override.
        assert_eq!(
            target.package_identifier.as_deref(),
            Some("BookLore/BookLore")
        );
    }

    #[test]
    fn phs_shell_target_structure() {
        let target = ProxmoxHelperScriptsPlugin::phs_shell_target(None);
        assert_eq!(target.plugin_type, PluginType::GenericShell);
        assert_eq!(target.plugin_config_name, "PHS Shell");
        assert_eq!(target.roles.len(), 2);
        assert!(target.roles.contains(&PluginRole::DetectVersion));
        assert!(target.roles.contains(&PluginRole::ExecuteUpdate));
        assert_eq!(
            target.plugin_config["version_command"],
            PHS_DETECT_VERSION_CMD
        );
        // Verify the version command uses the helper script (not raw cat).
        let version_cmd = target.plugin_config["version_command"]
            .as_str()
            .expect("version_command is a string");
        assert!(
            version_cmd.contains(PHS_VERSION_HELPER_PATH),
            "version_command must invoke the helper script, got: {version_cmd}"
        );
        assert!(
            version_cmd.starts_with("sudo "),
            "version_command must use sudo for shell-mode execution, got: {version_cmd}"
        );
        assert_eq!(target.plugin_config["update_command"], PHS_INSTALL_CMD);
        // Without an override, the software item's own package_identifier
        // (the container slug) is used at runtime.
        assert!(target.package_identifier.is_none());
    }

    #[test]
    fn phs_shell_target_with_version_file_override() {
        // Paperless-ngx: version file basename "paperless" differs from slug.
        // The target must carry a package_identifier override so the Shell
        // plugin calls `uptrakit-phs-version paperless` instead of
        // `uptrakit-phs-version paperless-ngx`.
        let target = ProxmoxHelperScriptsPlugin::phs_shell_target(Some("paperless"));
        assert_eq!(target.plugin_type, PluginType::GenericShell);
        assert_eq!(
            target.plugin_config["version_command"],
            PHS_DETECT_VERSION_CMD
        );
        assert_eq!(target.plugin_config["update_command"], PHS_INSTALL_CMD);
        assert_eq!(
            target.package_identifier.as_deref(),
            Some("paperless"),
            "package_identifier must be the version file basename override"
        );
    }

    #[test]
    fn required_sudo_commands_uses_helper_script() {
        let plugin =
            ProxmoxHelperScriptsPlugin::new(ProxmoxHelperScriptsConfig::default(), test_executor())
                .expect("create");
        let entries = plugin.required_sudo_commands();
        assert_eq!(entries.len(), 1);

        let entry = &entries[0];
        assert_eq!(entry.command, "uptrakit-phs-version");
        assert!(!entry.explanation.is_empty());

        // Must use a helper script, not a bare `cat` command.
        let helper = entry
            .helper_script
            .as_ref()
            .expect("PHS sudo entry must have a helper_script");
        assert_eq!(helper.install_path, PHS_VERSION_HELPER_PATH);

        // Helper script content must be non-empty and validate the slug.
        assert!(!helper.content.is_empty());
        assert!(
            helper.content.contains("[!a-z0-9-]"),
            "helper script must validate slug characters"
        );
        assert!(
            helper.content.contains("/root/."),
            "helper script must read from /root/"
        );
    }

    #[test]
    fn apt_target_structure() {
        let target = ProxmoxHelperScriptsPlugin::apt_target();
        assert_eq!(target.plugin_type, PluginType::PackageManagerApt);
        assert_eq!(target.plugin_config_name, "APT (auto)");
        assert_eq!(target.roles.len(), 3);
        assert_eq!(target.plugin_config, serde_json::json!({}));
        assert!(target.package_identifier.is_none());
    }
}
