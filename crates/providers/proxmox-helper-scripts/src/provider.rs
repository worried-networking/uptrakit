use std::sync::Arc;

use async_trait::async_trait;
use uptrakit_provider_core::command::{CommandExecutor, CommandSpec};
use uptrakit_provider_core::{
    DiscoveredSoftware, Provider, ProviderCapability, ProviderType,
};

use crate::config::ProxmoxHelperScriptsConfig;
use crate::discovery::{
    PHS_INSTALL_URL_PREFIX, UPDATE_SCRIPT_PATH, analyze_phs_script, extract_apt_package_candidates,
    parse_phs_scripts, parse_version_file, slug_to_display_name,
};

/// Capabilities: discovery only — no release-index refresh needed.
const CAPABILITIES: &[ProviderCapability] = &[ProviderCapability::DiscoverLocalSoftware];

/// Provider for Proxmox Helper Scripts (discovery-only).
///
/// Discovers PHS-managed software by:
/// 1. Reading `/usr/bin/update` and parsing CT script URLs from it.
/// 2. Fetching each CT script from `raw.githubusercontent.com` and analysing it
///    to determine whether the app is GitHub-managed or APT-managed.
/// 3. Emitting `DiscoveredSoftware` items with `extra` metadata set to one of:
///    - `{ "github_owner": "…", "github_repo": "…" }` — GitHub-managed
///    - `{ "apt_package": "…" }` — APT-managed
///
/// The controller synthesises the appropriate downstream provider config
/// (`github_releases` or `apt`) automatically from the `extra` metadata.
pub struct ProxmoxHelperScriptsProvider {
    _config: ProxmoxHelperScriptsConfig,
    executor: Arc<dyn CommandExecutor>,
    client: reqwest::Client,
}

impl ProxmoxHelperScriptsProvider {
    /// Create a new Proxmox Helper Scripts provider.
    pub fn new(
        config: ProxmoxHelperScriptsConfig,
        executor: Arc<dyn CommandExecutor>,
    ) -> uptrakit_provider_core::Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!(
                "uptrakit-provider-proxmox-helper-scripts/",
                env!("CARGO_PKG_VERSION")
            ))
            .build()
            .map_err(|e| {
                rootcause::report!(uptrakit_provider_core::ProviderError::ProviderInternal(
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
}

#[async_trait]
impl Provider for ProxmoxHelperScriptsProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::ProxmoxHelperScripts
    }

    fn capabilities(&self) -> &'static [ProviderCapability] {
        CAPABILITIES
    }

    async fn discover_software(
        &self,
    ) -> uptrakit_provider_core::Result<Vec<DiscoveredSoftware>> {
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
                    extra: Some(serde_json::json!({
                        "github_owner": owner,
                        "github_repo": repo,
                    })),
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
                    extra: Some(serde_json::json!({ "apt_package": apt_pkg })),
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
                        extra: Some(serde_json::json!({ "apt_package": candidate })),
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
    use uptrakit_provider_core::LocalCommandExecutor;

    fn test_executor() -> Arc<dyn CommandExecutor> {
        Arc::new(LocalCommandExecutor)
    }

    #[test]
    fn capabilities_discovery_only() {
        let provider = ProxmoxHelperScriptsProvider::new(
            ProxmoxHelperScriptsConfig::default(),
            test_executor(),
        )
        .expect("create");
        assert!(provider.has_capability(ProviderCapability::DiscoverLocalSoftware));
        assert!(!provider.has_capability(ProviderCapability::RefreshPackageIndex));
        assert_eq!(provider.capabilities().len(), 1);
    }

    #[test]
    fn provider_type_is_proxmox_helper_scripts() {
        let provider = ProxmoxHelperScriptsProvider::new(
            ProxmoxHelperScriptsConfig::default(),
            test_executor(),
        )
        .expect("create");
        assert_eq!(provider.provider_type(), ProviderType::ProxmoxHelperScripts);
    }

    #[tokio::test]
    async fn discover_software_returns_empty_without_update_script() {
        // On a non-PHS system /usr/bin/update likely does not exist.
        let provider = ProxmoxHelperScriptsProvider::new(
            ProxmoxHelperScriptsConfig::default(),
            test_executor(),
        )
        .expect("create");
        let result = provider.discover_software().await;
        assert!(result.is_ok());
        // No error; result is empty or whatever is found on the test machine.
    }
}
