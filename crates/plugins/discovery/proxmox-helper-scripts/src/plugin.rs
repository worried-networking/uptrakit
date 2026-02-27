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

/// Absolute path where the PHS update helper script is installed on managed hosts.
///
/// This path is used for the sudoers entry and as the command in the Shell
/// plugin's `update_command` config. Installed during host bootstrap via
/// [`SudoHelperScript`].
const PHS_UPDATE_HELPER_PATH: &str = "/usr/local/bin/uptrakit-phs-update";

/// Content of the PHS update helper script, embedded at compile time.
///
/// The script runs `/usr/bin/update` with `PHS_SILENT=1` set, ensuring the
/// update proceeds without interactive whiptail prompts. No user arguments
/// are accepted — the script always performs the full PHS update pass for
/// all managed containers on the Proxmox node.
const PHS_UPDATE_HELPER_CONTENT: &str = include_str!("phs_update.sh");

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
/// Delegates to the `uptrakit-phs-update` helper script (installed by
/// bootstrap), which runs `/usr/bin/update` with `PHS_SILENT=1` so the update
/// proceeds without interactive prompts.
///
/// `sudo` is embedded in the command string because the Shell plugin executes
/// update commands through [`CommandSpec::shell`], which does not support the
/// `.privileged()` flag — shell commands must handle their own privilege
/// escalation. The corresponding sudoers entry is:
///
/// ```text
/// uptrakit ALL=(root) NOPASSWD: /usr/local/bin/uptrakit-phs-update
/// ```
const PHS_INSTALL_CMD: &str = "sudo /usr/local/bin/uptrakit-phs-update";

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
        if version.is_empty() { None } else { Some(version) }
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
            },
            SudoCommandEntry {
                command: "uptrakit-phs-update".into(),
                explanation: "Runs /usr/bin/update with PHS_SILENT=1 for unattended PHS \
                    container updates; the helper script takes no arguments so no \
                    argument validation is needed"
                    .into(),
                helper_script: Some(SudoHelperScript {
                    install_path: PHS_UPDATE_HELPER_PATH,
                    content: PHS_UPDATE_HELPER_CONTENT,
                }),
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
        // update_command must also use sudo (shell-mode, so .privileged() has no effect).
        let update_cmd = target.plugin_config["update_command"]
            .as_str()
            .expect("update_command is a string");
        assert!(
            update_cmd.starts_with("sudo "),
            "update_command must use sudo for shell-mode execution, got: {update_cmd}"
        );
        assert!(
            update_cmd.contains(PHS_UPDATE_HELPER_PATH),
            "update_command must invoke the update helper script, got: {update_cmd}"
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

    #[test]
    fn required_sudo_commands_uses_helper_scripts() {
        let plugin =
            ProxmoxHelperScriptsPlugin::new(ProxmoxHelperScriptsConfig::default(), test_executor())
                .expect("create");
        let entries = plugin.required_sudo_commands();
        assert_eq!(
            entries.len(),
            2,
            "expected two sudo entries: version + update"
        );

        // ── Version detection helper ─────────────────────────────────────────
        let version_entry = entries
            .iter()
            .find(|e| e.command == "uptrakit-phs-version")
            .expect("uptrakit-phs-version sudo entry must be present");
        assert!(!version_entry.explanation.is_empty());

        let version_helper = version_entry
            .helper_script
            .as_ref()
            .expect("version sudo entry must have a helper_script");
        assert_eq!(version_helper.install_path, PHS_VERSION_HELPER_PATH);
        assert!(!version_helper.content.is_empty());
        assert!(
            version_helper.content.contains("[!a-z0-9-]"),
            "version helper must validate slug characters"
        );
        assert!(
            version_helper.content.contains("/root/."),
            "version helper must read from /root/"
        );

        // ── Update helper ────────────────────────────────────────────────────
        let update_entry = entries
            .iter()
            .find(|e| e.command == "uptrakit-phs-update")
            .expect("uptrakit-phs-update sudo entry must be present");
        assert!(!update_entry.explanation.is_empty());

        let update_helper = update_entry
            .helper_script
            .as_ref()
            .expect("update sudo entry must have a helper_script");
        assert_eq!(update_helper.install_path, PHS_UPDATE_HELPER_PATH);
        assert!(!update_helper.content.is_empty());
        assert!(
            update_helper.content.contains("PHS_SILENT=1"),
            "update helper must set PHS_SILENT=1"
        );
        assert!(
            update_helper.content.contains("/usr/bin/update"),
            "update helper must call /usr/bin/update"
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
        assert_eq!(
            target.package_identifier.as_deref(),
            Some("@angular/cli")
        );
    }
}
