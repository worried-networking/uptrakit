use std::sync::Arc;

use async_trait::async_trait;
use uptrakit_plugin_infrastructure_core::command::{CommandExecutor, CommandSpec};
use uptrakit_plugin_infrastructure_core::{
    DiscoveredSoftware, DiscoveryTarget, HostCompatibility, Plugin, PluginCapability, PluginRole,
    PluginType, SudoCommandEntry, SudoHelperScript, TrackingSystem,
};

use crate::config::ProxmoxHelperScriptsConfig;
use crate::discovery::{
    UPDATE_SCRIPT_PATH, analyze_phs_script, extract_apt_package_candidates, extract_npm_package,
    parse_phs_scripts, parse_version_file, slug_to_display_name,
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
/// Runs `/usr/bin/update` with `PHS_SILENT=1` to suppress interactive whiptail
/// dialogs and `TERM=xterm` so that terminal commands (e.g. `clear`) succeed
/// over a non-interactive SSH channel.
///
/// `sudo` is embedded in the command string because the Shell plugin executes
/// update commands through [`CommandSpec::shell`], which does not support the
/// `.privileged()` flag — shell commands must handle their own privilege
/// escalation. The inline `NAME=VALUE` assignments are accepted by sudo because
/// the corresponding sudoers entry carries `SETENV:`:
///
/// ```text
/// uptrakit ALL=(root) NOPASSWD: SETENV: /usr/bin/update
/// ```
const PHS_INSTALL_CMD: &str = "sudo PHS_SILENT=1 TERM=xterm /usr/bin/update";

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
    /// Compile-time capabilities for the Proxmox Helper Scripts plugin.
    pub const CAPABILITIES: &'static [PluginCapability] = &[
        PluginCapability::DiscoverLocalSoftware,
        PluginCapability::DetectHostCompatibility,
    ];

    /// Create a new Proxmox Helper Scripts plugin.
    pub async fn new(
        config: ProxmoxHelperScriptsConfig,
        executor: Arc<dyn CommandExecutor>,
    ) -> uptrakit_plugin_infrastructure_core::Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!(
                "uptrakit-plugin-discovery-proxmox-helper-scripts/",
                env!("CARGO_PKG_VERSION")
            ))
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60))
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
                &CommandSpec::exec(PHS_VERSION_HELPER_PATH, [version_file_basename.to_string()])
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

    /// Build a `DiscoveryTarget` for the Forgejo releases role.
    ///
    /// The plugin config carries only Forgejo-level settings (no `owner`/`repo`).
    /// The `owner/repo` pair is expressed as the `package_identifier` override
    /// so the controller routes release queries to the right repo while sharing
    /// a single plugin config instance across all tracked Forgejo repositories.
    fn forgejo_fetch_target(owner: &str, repo: &str) -> DiscoveryTarget {
        DiscoveryTarget {
            plugin_type: PluginType::ReleasesForgejo,
            plugin_config: serde_json::json!({
                "api_base_url": "https://codeberg.org",
                "tag_strip_prefix": "v",
                "include_prereleases": false,
                "asset_patterns": [],
            }),
            plugin_config_name: "Forgejo Releases".to_string(),
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

    /// Build a `DiscoveryTarget` for an npm-managed PHS app.
    fn npm_target(package: &str) -> DiscoveryTarget {
        DiscoveryTarget {
            plugin_type: PluginType::PackageManagerNpm,
            plugin_config: serde_json::json!({}),
            plugin_config_name: "NPM (auto)".to_string(),
            roles: vec![
                PluginRole::DetectVersion,
                PluginRole::FetchReleases,
                PluginRole::ExecuteUpdate,
            ],
            package_identifier: Some(package.to_string()),
            config_override: None,
            execution_site: None,
        }
    }

    /// Run `npm list -g <package> --depth=0 --json` to detect the installed
    /// version of a globally-installed npm package.
    ///
    /// Returns the version string, or `None` if the package is not installed
    /// or the command fails.
    async fn npm_global_version(&self, package: &str) -> Option<String> {
        let output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "npm",
                [
                    "list".to_string(),
                    "-g".to_string(),
                    package.to_string(),
                    "--depth=0".to_string(),
                    "--json".to_string(),
                ],
            ))
            .await
            .ok()?;

        if output.exit_code != 0 {
            return None;
        }

        // Parse {"dependencies":{"<package>":{"version":"X.Y.Z"}}}
        let json: serde_json::Value = serde_json::from_str(&output.output).ok()?;
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
        Self::CAPABILITIES
    }

    async fn detect_host_compatibility(
        &self,
    ) -> uptrakit_plugin_infrastructure_core::Result<HostCompatibility> {
        // A Proxmox Helper Scripts host is identified by the presence of
        // `/usr/bin/update` — the PHS update script installed on all Proxmox VE
        // nodes.  Any other system (Flatcar Linux, Ubuntu servers, macOS, …)
        // will not have this file, so the plugin is incompatible and its helper
        // scripts must not be installed.
        match self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "test",
                ["-f".to_string(), UPDATE_SCRIPT_PATH.to_string()],
            ))
            .await
        {
            Ok(_) => Ok(HostCompatibility::Compatible),
            Err(_) => Ok(HostCompatibility::Incompatible(format!(
                "PHS update script not found at {UPDATE_SCRIPT_PATH} — not a Proxmox Helper Scripts host"
            ))),
        }
    }

    fn required_sudo_commands(&self) -> Vec<SudoCommandEntry> {
        vec![
            SudoCommandEntry {
                command: "uptrakit-phs-version".into(),
                explanation: "Reads /root/.<slug> for PHS version detection; the helper script \
                    validates the slug argument to prevent path traversal"
                    .into(),
                helper_script: Some(SudoHelperScript {
                    install_path: PHS_VERSION_HELPER_PATH,
                    content: PHS_VERSION_HELPER_CONTENT,
                }),
                needs_setenv: false,
            },
            SudoCommandEntry {
                command: "update".into(),
                explanation: "Runs /usr/bin/update with PHS_SILENT=1 and TERM=xterm for \
                    unattended PHS container updates; SETENV: is required so the agent \
                    can pass the env vars inline in the sudo call"
                    .into(),
                helper_script: None,
                needs_setenv: true,
            },
        ]
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
                    tracking_system: TrackingSystem::Targeted,
                });
            } else if let (Some(owner), Some(repo)) =
                (&analysis.forgejo_owner, &analysis.forgejo_repo)
            {
                // Forgejo-managed: read version via the same PHS helper script.
                let vfb = analysis
                    .version_file_basename
                    .as_deref()
                    .unwrap_or(&script.slug);
                let Some(installed_version) = self.phs_version(vfb).await else {
                    tracing::debug!(slug = %script.slug,
                        "PHS version helper absent or version file absent; skipping Forgejo item");
                    continue;
                };

                tracing::debug!(
                    slug = %script.slug,
                    version_file_basename = %vfb,
                    version = %installed_version,
                    owner = %owner,
                    repo = %repo,
                    "discovered Forgejo-managed PHS software"
                );

                discovered.push(DiscoveredSoftware {
                    package_identifier: script.slug.clone(),
                    name: display_name,
                    installed_version,
                    targets: vec![
                        Self::forgejo_fetch_target(owner, repo),
                        Self::phs_shell_target(analysis.version_file_basename.as_deref()),
                    ],
                    extra: None,
                    tracking_system: TrackingSystem::Targeted,
                });
            } else if let Some(ref npm_pkg) = analysis.npm_package {
                // npm-managed: verify installed via `npm list -g`.
                let Some(installed_version) = self.npm_global_version(npm_pkg).await else {
                    tracing::debug!(slug = %script.slug, package = %npm_pkg,
                        "npm package not installed globally; skipping");
                    continue;
                };

                tracing::debug!(
                    slug = %script.slug,
                    package = %npm_pkg,
                    version = %installed_version,
                    "discovered npm-managed PHS software"
                );

                discovered.push(DiscoveredSoftware {
                    package_identifier: npm_pkg.clone(),
                    name: display_name,
                    installed_version,
                    targets: vec![Self::npm_target(npm_pkg)],
                    extra: None,
                    tracking_system: TrackingSystem::Targeted,
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
                    tracking_system: TrackingSystem::Targeted,
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

                // Some apps (e.g. n8n) do not reference npm in their CT script
                // but do install via `npm install -g <pkg>` in the install script.
                // Try npm detection first so these are discovered as npm-managed
                // rather than being incorrectly skipped or classified as APT.
                if let Some(npm_pkg) = extract_npm_package(&install_body) {
                    let Some(installed_version) = self.npm_global_version(&npm_pkg).await else {
                        tracing::debug!(slug = %script.slug, package = %npm_pkg,
                            "npm package not installed globally; skipping");
                        continue;
                    };

                    tracing::debug!(
                        slug = %script.slug,
                        package = %npm_pkg,
                        version = %installed_version,
                        "discovered install-script npm-managed PHS software"
                    );

                    discovered.push(DiscoveredSoftware {
                        package_identifier: npm_pkg.clone(),
                        name: display_name,
                        installed_version,
                        targets: vec![Self::npm_target(&npm_pkg)],
                        extra: None,
                        tracking_system: TrackingSystem::Targeted,
                    });
                    continue;
                }

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
                        tracking_system: TrackingSystem::Targeted,
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

    #[tokio::test]
    async fn capabilities_includes_discovery_and_compat_check() {
        let plugin =
            ProxmoxHelperScriptsPlugin::new(ProxmoxHelperScriptsConfig::default(), test_executor())
                .await
                .expect("create");
        assert!(plugin.has_capability(PluginCapability::DiscoverLocalSoftware));
        assert!(plugin.has_capability(PluginCapability::DetectHostCompatibility));
        assert!(!plugin.has_capability(PluginCapability::RefreshPackageIndex));
        assert_eq!(plugin.capabilities().len(), 2);
    }

    #[tokio::test]
    async fn detect_host_compatibility_returns_ok_on_non_phs_host() {
        // On a non-PHS system (dev machine, CI) the result must be Ok — never Err.
        // The exact variant (Compatible / Incompatible) depends on whether
        // /usr/bin/update is present, so we only assert that no error is returned.
        let plugin =
            ProxmoxHelperScriptsPlugin::new(ProxmoxHelperScriptsConfig::default(), test_executor())
                .await
                .expect("create");
        let result = plugin.detect_host_compatibility().await;
        assert!(result.is_ok(), "detect_host_compatibility must not error");
    }

    #[tokio::test]
    async fn detect_host_compatibility_incompatible_carries_update_script_path() {
        // When incompatible the reason string must mention the expected path so
        // operators know what to look for.
        let plugin =
            ProxmoxHelperScriptsPlugin::new(ProxmoxHelperScriptsConfig::default(), test_executor())
                .await
                .expect("create");
        if let Ok(HostCompatibility::Incompatible(reason)) =
            plugin.detect_host_compatibility().await
        {
            assert!(
                reason.contains(UPDATE_SCRIPT_PATH),
                "incompatible reason should mention {UPDATE_SCRIPT_PATH}: {reason}"
            );
        }
        // If Compatible, the test is vacuously true (running on a Proxmox node).
    }

    #[tokio::test]
    async fn plugin_type_is_proxmox_helper_scripts() {
        let plugin =
            ProxmoxHelperScriptsPlugin::new(ProxmoxHelperScriptsConfig::default(), test_executor())
                .await
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
                .await
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
    fn forgejo_fetch_target_structure() {
        let target = ProxmoxHelperScriptsPlugin::forgejo_fetch_target("readeck", "readeck");
        assert_eq!(target.plugin_type, PluginType::ReleasesForgejo);
        assert_eq!(target.plugin_config_name, "Forgejo Releases");
        // FetchReleases only — no agent-side roles.
        assert_eq!(target.roles.len(), 1);
        assert_eq!(target.roles[0], PluginRole::FetchReleases);
        // api_base_url points to Codeberg (PHS scripts use check_for_codeberg_release).
        assert_eq!(
            target
                .plugin_config
                .get("api_base_url")
                .and_then(|v| v.as_str()),
            Some("https://codeberg.org")
        );
        // No owner/repo in config.
        assert!(target.plugin_config.get("owner").is_none());
        assert!(target.plugin_config.get("repo").is_none());
        // package_identifier carries the "owner/repo" override.
        assert_eq!(
            target.package_identifier.as_deref(),
            Some("readeck/readeck")
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
        // update_command must use sudo (shell-mode, so .privileged() has no effect).
        let update_cmd = target.plugin_config["update_command"]
            .as_str()
            .expect("update_command is a string");
        assert!(
            update_cmd.starts_with("sudo "),
            "update_command must use sudo for shell-mode execution, got: {update_cmd}"
        );
        assert!(
            update_cmd.contains("PHS_SILENT=1"),
            "update_command must set PHS_SILENT=1, got: {update_cmd}"
        );
        assert!(
            update_cmd.contains("TERM=xterm"),
            "update_command must set TERM=xterm, got: {update_cmd}"
        );
        assert!(
            update_cmd.contains("/usr/bin/update"),
            "update_command must call /usr/bin/update, got: {update_cmd}"
        );
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

    #[tokio::test]
    async fn required_sudo_commands_structure() {
        let plugin =
            ProxmoxHelperScriptsPlugin::new(ProxmoxHelperScriptsConfig::default(), test_executor())
                .await
                .expect("create");
        let entries = plugin.required_sudo_commands();
        assert_eq!(
            entries.len(),
            2,
            "expected two sudo entries: version helper + update"
        );

        // ── Version detection helper (still uses a helper script) ─────────────
        let version_entry = entries
            .iter()
            .find(|e| e.command == "uptrakit-phs-version")
            .expect("uptrakit-phs-version sudo entry must be present");
        assert!(!version_entry.explanation.is_empty());
        assert!(
            !version_entry.needs_setenv,
            "version helper does not use env var forwarding"
        );

        let version_helper = version_entry
            .helper_script
            .as_ref()
            .expect("version sudo entry must have a helper_script");
        assert_eq!(version_helper.install_path, PHS_VERSION_HELPER_PATH);
        assert!(
            version_helper.content.contains("[!a-z0-9-]"),
            "version helper must validate slug"
        );
        assert!(
            version_helper.content.contains("/root/."),
            "version helper must read /root/.<slug>"
        );

        // ── Update — direct /usr/bin/update, no helper script ─────────────────
        let update_entry = entries
            .iter()
            .find(|e| e.command == "update")
            .expect("'update' sudo entry must be present");
        assert!(!update_entry.explanation.is_empty());
        assert!(
            update_entry.needs_setenv,
            "update entry must set needs_setenv=true (PHS_SILENT=1 passed inline)"
        );
        assert!(
            update_entry.helper_script.is_none(),
            "update entry must not use a helper script"
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

    #[test]
    fn npm_target_structure() {
        let target = ProxmoxHelperScriptsPlugin::npm_target("n8n");
        assert_eq!(target.plugin_type, PluginType::PackageManagerNpm);
        assert_eq!(target.plugin_config_name, "NPM (auto)");
        assert_eq!(target.roles.len(), 3);
        assert!(target.roles.contains(&PluginRole::DetectVersion));
        assert!(target.roles.contains(&PluginRole::FetchReleases));
        assert!(target.roles.contains(&PluginRole::ExecuteUpdate));
        assert_eq!(target.plugin_config, serde_json::json!({}));
        // The npm package name is carried as the package_identifier override.
        assert_eq!(target.package_identifier.as_deref(), Some("n8n"));
    }

    #[test]
    fn npm_target_scoped_package() {
        let target = ProxmoxHelperScriptsPlugin::npm_target("@angular/cli");
        assert_eq!(target.plugin_type, PluginType::PackageManagerNpm);
        assert_eq!(target.package_identifier.as_deref(), Some("@angular/cli"));
    }
}
