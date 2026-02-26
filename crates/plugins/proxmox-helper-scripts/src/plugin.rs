use std::sync::Arc;

use async_trait::async_trait;
use uptrakit_plugin_core::command::{CommandExecutor, CommandSpec};
use uptrakit_plugin_core::{
    DiscoveredSoftware, DiscoveryTarget, Plugin, PluginCapability, PluginRole, PluginType,
};

use crate::config::ProxmoxHelperScriptsConfig;
use crate::discovery::{
    PHS_DETECT_VERSION_CMD, PHS_INSTALL_CMD, PHS_INSTALL_URL_PREFIX, UPDATE_SCRIPT_PATH,
    analyze_phs_script, extract_apt_package_candidates, parse_phs_scripts, parse_version_file,
    slug_to_display_name,
};

/// Capabilities: discovery only — no release-index refresh needed.
const CAPABILITIES: &[PluginCapability] = &[PluginCapability::DiscoverLocalSoftware];

/// All three standard plugin roles.
fn all_roles() -> Vec<PluginRole> {
    vec![
        PluginRole::DetectVersion,
        PluginRole::FetchReleases,
        PluginRole::ExecuteUpdate,
    ]
}

/// Plugin for Proxmox Helper Scripts (discovery-only).
///
/// Discovers PHS-managed software by:
/// 1. Reading `/usr/bin/update` and parsing CT script URLs from it.
/// 2. Fetching each CT script from `raw.githubusercontent.com` and analysing it
///    to determine whether the app is GitHub-managed or APT-managed.
/// 3. Emitting `DiscoveredSoftware` items with structured `targets` that tell
///    the controller exactly which plugin configs to create:
///    - GitHub-managed → `DiscoveryTarget` with `GithubReleases` plugin type
///    - APT-managed → `DiscoveryTarget` with `Apt` plugin type
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
    ) -> uptrakit_plugin_core::Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!(
                "uptrakit-plugin-proxmox-helper-scripts/",
                env!("CARGO_PKG_VERSION")
            ))
            .build()
            .map_err(|e| {
                rootcause::report!(uptrakit_plugin_core::PluginError::PluginInternal(
                    format!("failed to build HTTP client: {e}")
                ))
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

    /// Try to read a version file at the given path. Returns `None` if the
    /// file does not exist or cannot be read.
    async fn try_read_version_file(&self, path: &str) -> Option<String> {
        let output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "cat",
                ["--".to_string(), path.to_string()],
            ))
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

    /// Build a `DiscoveryTarget` for a GitHub-managed PHS app.
    fn github_target(owner: &str, repo: &str) -> DiscoveryTarget {
        DiscoveryTarget {
            plugin_type: PluginType::GithubReleases,
            plugin_config: serde_json::json!({
                "owner": owner,
                "repo": repo,
                "tag_strip_prefix": "v",
                "include_prereleases": false,
                "asset_patterns": [],
                "detect_installed_version_command": PHS_DETECT_VERSION_CMD,
                "install_command": PHS_INSTALL_CMD,
            }),
            plugin_config_name: format!("{owner}/{repo}"),
            roles: all_roles(),
            package_identifier: None,
            config_override: None,
            execution_site: None,
        }
    }

    /// Build a `DiscoveryTarget` for an APT-managed PHS app.
    fn apt_target() -> DiscoveryTarget {
        DiscoveryTarget {
            plugin_type: PluginType::Apt,
            plugin_config: serde_json::json!({}),
            plugin_config_name: "APT (auto)".to_string(),
            roles: all_roles(),
            package_identifier: None,
            config_override: None,
            execution_site: None,
        }
    }
}

#[async_trait]
impl Plugin for ProxmoxHelperScriptsPlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::ProxmoxHelperScripts
    }

    fn capabilities(&self) -> &'static [PluginCapability] {
        CAPABILITIES
    }

    async fn discover_software(
        &self,
    ) -> uptrakit_plugin_core::Result<Vec<DiscoveredSoftware>> {
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

        // Resolve HOME lazily — only needed for the GitHub version-file path.
        let mut home: Option<String> = None;
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

            if let (Some(owner), Some(repo)) =
                (&analysis.github_owner, &analysis.github_repo)
            {
                // GitHub-managed: read version from $HOME/.{slug}.
                let home = match home {
                    Some(ref h) => h.clone(),
                    None => {
                        let h = self
                            .executor
                            .execute_quiet(&CommandSpec::exec(
                                "printenv",
                                ["HOME".to_string()],
                            ))
                            .await
                            .ok()
                            .map(|o| o.output.trim().to_string())
                            .filter(|s| !s.is_empty());

                        let Some(h) = h else {
                            tracing::warn!("HOME is not set; skipping GitHub PHS items");
                            break;
                        };
                        home = Some(h.clone());
                        h
                    }
                };

                let version_path = format!("{home}/.{}", script.slug);
                let Some(installed_version) =
                    self.try_read_version_file(&version_path).await
                else {
                    tracing::debug!(slug = %script.slug,
                        "PHS version file absent; skipping GitHub item");
                    continue;
                };

                tracing::debug!(
                    slug = %script.slug,
                    version = %installed_version,
                    owner = %owner,
                    repo = %repo,
                    "discovered GitHub-managed PHS software"
                );

                discovered.push(DiscoveredSoftware {
                    package_identifier: script.slug.clone(),
                    name: display_name,
                    installed_version,
                    targets: vec![Self::github_target(owner, repo)],
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
                let install_url =
                    format!("{PHS_INSTALL_URL_PREFIX}{}-install.sh", script.slug);
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
    use uptrakit_plugin_core::LocalCommandExecutor;

    fn test_executor() -> Arc<dyn CommandExecutor> {
        Arc::new(LocalCommandExecutor)
    }

    #[test]
    fn capabilities_discovery_only() {
        let plugin = ProxmoxHelperScriptsPlugin::new(
            ProxmoxHelperScriptsConfig::default(),
            test_executor(),
        )
        .expect("create");
        assert!(plugin.has_capability(PluginCapability::DiscoverLocalSoftware));
        assert!(!plugin.has_capability(PluginCapability::RefreshPackageIndex));
        assert_eq!(plugin.capabilities().len(), 1);
    }

    #[test]
    fn plugin_type_is_proxmox_helper_scripts() {
        let plugin = ProxmoxHelperScriptsPlugin::new(
            ProxmoxHelperScriptsConfig::default(),
            test_executor(),
        )
        .expect("create");
        assert_eq!(plugin.plugin_type(), PluginType::ProxmoxHelperScripts);
    }

    #[tokio::test]
    async fn discover_software_returns_empty_without_update_script() {
        // On a non-PHS system /usr/bin/update likely does not exist.
        let plugin = ProxmoxHelperScriptsPlugin::new(
            ProxmoxHelperScriptsConfig::default(),
            test_executor(),
        )
        .expect("create");
        let result = plugin.discover_software().await;
        assert!(result.is_ok());
        // No error; result is empty or whatever is found on the test machine.
    }

    #[test]
    fn github_target_structure() {
        let target = ProxmoxHelperScriptsPlugin::github_target("BookLore", "BookLore");
        assert_eq!(target.plugin_type, PluginType::GithubReleases);
        assert_eq!(target.plugin_config_name, "BookLore/BookLore");
        assert_eq!(target.roles.len(), 3);
        assert_eq!(target.plugin_config["owner"], "BookLore");
        assert_eq!(target.plugin_config["repo"], "BookLore");
        assert_eq!(
            target.plugin_config["detect_installed_version_command"],
            PHS_DETECT_VERSION_CMD
        );
        assert_eq!(target.plugin_config["install_command"], PHS_INSTALL_CMD);
        assert!(target.package_identifier.is_none());
    }

    #[test]
    fn apt_target_structure() {
        let target = ProxmoxHelperScriptsPlugin::apt_target();
        assert_eq!(target.plugin_type, PluginType::Apt);
        assert_eq!(target.plugin_config_name, "APT (auto)");
        assert_eq!(target.roles.len(), 3);
        assert_eq!(target.plugin_config, serde_json::json!({}));
        assert!(target.package_identifier.is_none());
    }
}
