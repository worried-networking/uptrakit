use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::command::CommandSpec;
use uptrakit_plugin_infrastructure_core::{
    DiscoveredSoftware, DiscoveryTarget, HostCompatibility, PluginError, PluginRole, Result,
    execute_and_capture, plugin_ids,
};

use crate::config::HomebrewPackageType;
use crate::plugin::HomebrewPlugin;

impl HomebrewPlugin {
    /// Parse installed formulae from `brew info --installed --json=v2` output.
    ///
    /// Emits items only for packages with a known installed version.
    /// Each item carries a `DiscoveryTarget` with `{"package_type": "formula"}`
    /// config so the controller can find-or-create the correct Homebrew plugin config.
    pub(crate) fn parse_installed_formulae(json: &serde_json::Value) -> Vec<DiscoveredSoftware> {
        let mut result = Vec::new();
        if let Some(formulae) = json.get("formulae").and_then(|f| f.as_array()) {
            for formula in formulae {
                let Some(name) = formula.get("name").and_then(|n| n.as_str()) else {
                    continue;
                };
                let full_name = formula
                    .get("full_name")
                    .and_then(|n| n.as_str())
                    .unwrap_or(name);
                let Some(installed_version) = formula
                    .get("installed")
                    .and_then(|arr| arr.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|obj| obj.get("version"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                else {
                    // Skip packages without a known installed version.
                    continue;
                };

                // Formulae pinned to "latest" have no deterministic version
                // and cannot be meaningfully tracked or upgraded.
                if installed_version == "latest" {
                    tracing::debug!(name, "skipping formula with version=latest from discovery");
                    continue;
                }

                let targets = vec![DiscoveryTarget {
                    plugin_type: plugin_ids::PACKAGE_MANAGER_HOMEBREW.clone(),
                    plugin_config: serde_json::json!({"package_type": "formula"}),
                    plugin_config_name: "Homebrew (Formulae)".to_string(),
                    roles: vec![
                        PluginRole::DetectVersion,
                        PluginRole::FetchReleases,
                        PluginRole::ExecuteUpdate,
                    ],
                    package_identifier: None,
                    config_override: Some(serde_json::json!({"package_type": "formula"})),
                    execution_site: None,
                }];

                result.push(DiscoveredSoftware {
                    package_identifier: name.to_string(),
                    name: full_name.to_string(),
                    installed_version,
                    targets,
                    extra: None,
                    qualifier: None,
                    plugin_package_identifier: None,
                    featured: false,
                    installed_display_version: None,
                });
            }
        }
        result
    }

    /// Parse installed casks from `brew info --installed --json=v2` output.
    ///
    /// Emits items only for packages with a known installed version.
    /// Each item carries a `DiscoveryTarget` with `{"package_type": "cask"}`
    /// config so the controller can find-or-create the correct Homebrew plugin config.
    pub(crate) fn parse_installed_casks(json: &serde_json::Value) -> Vec<DiscoveredSoftware> {
        let mut result = Vec::new();
        if let Some(casks) = json.get("casks").and_then(|c| c.as_array()) {
            for cask in casks {
                let Some(token) = cask.get("token").and_then(|t| t.as_str()) else {
                    continue;
                };
                let name = cask
                    .get("name")
                    .and_then(|n| n.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|n| n.as_str())
                    .unwrap_or(token);
                let Some(installed_version) = cask
                    .get("installed")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                else {
                    // Skip casks without a known installed version.
                    continue;
                };

                // Casks with version "latest" have no deterministic version
                // and cannot be meaningfully tracked or upgraded.
                if installed_version == "latest" {
                    tracing::debug!(token, "skipping cask with version=latest from discovery");
                    continue;
                }

                // Casks with `auto_updates: true` manage their own update
                // mechanism (e.g. Google Chrome) and cannot be upgraded via
                // `brew upgrade`. Exclude them from discovery so they don't
                // appear in the UI.
                if cask
                    .get("auto_updates")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    tracing::debug!(
                        token,
                        "skipping auto-updating cask from discovery (auto_updates=true)"
                    );
                    continue;
                }

                let targets = vec![DiscoveryTarget {
                    plugin_type: plugin_ids::PACKAGE_MANAGER_HOMEBREW.clone(),
                    plugin_config: serde_json::json!({"package_type": "cask"}),
                    plugin_config_name: "Homebrew (Casks)".to_string(),
                    roles: vec![
                        PluginRole::DetectVersion,
                        PluginRole::FetchReleases,
                        PluginRole::ExecuteUpdate,
                    ],
                    package_identifier: None,
                    config_override: Some(serde_json::json!({"package_type": "cask"})),
                    execution_site: None,
                }];

                result.push(DiscoveredSoftware {
                    package_identifier: token.to_string(),
                    name: name.to_string(),
                    installed_version,
                    targets,
                    extra: None,
                    qualifier: None,
                    plugin_package_identifier: None,
                    featured: false,
                    installed_display_version: None,
                });
            }
        }
        result
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::Discoverer for HomebrewPlugin {
    #[tracing::instrument(skip_all)]
    async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> {
        let stdout = execute_and_capture(
            self.executor.as_ref(),
            CommandSpec::exec(
                "brew",
                [
                    "info".to_string(),
                    "--installed".to_string(),
                    "--json=v2".to_string(),
                ],
            ),
            "brew info",
        )
        .await?;

        let json: serde_json::Value = serde_json::from_str(&stdout).map_err(|e| {
            report!(PluginError::PluginInternal(format!(
                "failed to parse brew info JSON: {e}"
            )))
        })?;

        let packages = match self.config.package_type {
            HomebrewPackageType::Both => {
                // Discover both formulae and casks, each tagged with
                // extra metadata so the controller can route them to the
                // correct plugin configs.
                tracing::debug!("discovering all installed Homebrew packages (formulae + casks)");
                let mut all = Self::parse_installed_formulae(&json);
                all.extend(Self::parse_installed_casks(&json));
                all
            }
            HomebrewPackageType::Formula => {
                tracing::debug!("discovering installed Homebrew formulae");
                Self::parse_installed_formulae(&json)
            }
            HomebrewPackageType::Cask => {
                tracing::debug!("discovering installed Homebrew casks");
                Self::parse_installed_casks(&json)
            }
        };

        Ok(packages)
    }

    #[tracing::instrument(skip_all)]
    async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
        match self
            .executor
            .execute_quiet(&CommandSpec::exec("which", ["brew".to_string()]))
            .await
        {
            Ok(_) => Ok(HostCompatibility::Compatible),
            Err(_) => Ok(HostCompatibility::Incompatible(
                "brew not found".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions use assert!(result.is_ok()) pattern"
    )]
    use super::*;
    use uptrakit_plugin_infrastructure_core::testing::{
        FixedOutputExecutor, test_runtime, test_runtime_with_executor,
    };
    use uptrakit_plugin_infrastructure_core::{Discoverer, plugin_ids};

    use crate::config::HomebrewConfig;

    fn sample_installed_json() -> serde_json::Value {
        serde_json::json!({
            "formulae": [
                {
                    "name": "wget",
                    "full_name": "wget",
                    "versions": { "stable": "1.24.5" },
                    "installed": [{ "version": "1.24.4" }]
                },
                {
                    "name": "jq",
                    "full_name": "jq",
                    "versions": { "stable": "1.7.1" },
                    "installed": [{ "version": "1.7.1" }]
                }
            ],
            "casks": [
                {
                    "token": "firefox",
                    "name": ["Mozilla Firefox"],
                    "version": "133.0",
                    "installed": "132.0"
                }
            ]
        })
    }

    fn sample_cask_json_not_installed() -> serde_json::Value {
        serde_json::json!({
            "formulae": [],
            "casks": [{
                "token": "firefox",
                "name": ["Mozilla Firefox"],
                "version": "133.0",
                "installed": null,
                "homepage": "https://www.mozilla.org/firefox/"
            }]
        })
    }

    // ── parse_installed_formulae / parse_installed_casks ────────────────

    #[test]
    fn parse_installed_formulae_emits_targets() {
        let json = sample_installed_json();
        let packages = HomebrewPlugin::parse_installed_formulae(&json);
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].package_identifier, "wget");
        assert_eq!(packages[0].name, "wget");
        assert_eq!(packages[0].installed_version, "1.24.4");
        assert_eq!(packages[0].targets.len(), 1);
        assert_eq!(
            packages[0].targets[0].plugin_type,
            plugin_ids::PACKAGE_MANAGER_HOMEBREW.clone()
        );
        assert_eq!(
            packages[0].targets[0].plugin_config,
            serde_json::json!({"package_type": "formula"})
        );
        assert_eq!(
            packages[0].targets[0].plugin_config_name,
            "Homebrew (Formulae)"
        );
        assert_eq!(packages[0].targets[0].roles.len(), 3);
        assert_eq!(packages[1].package_identifier, "jq");
        assert_eq!(packages[1].installed_version, "1.7.1");
    }

    #[test]
    fn parse_installed_casks_emits_targets() {
        let json = sample_installed_json();
        let packages = HomebrewPlugin::parse_installed_casks(&json);
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].package_identifier, "firefox");
        assert_eq!(packages[0].name, "Mozilla Firefox");
        assert_eq!(packages[0].installed_version, "132.0");
        assert_eq!(packages[0].targets.len(), 1);
        assert_eq!(
            packages[0].targets[0].plugin_type,
            plugin_ids::PACKAGE_MANAGER_HOMEBREW.clone()
        );
        assert_eq!(
            packages[0].targets[0].plugin_config,
            serde_json::json!({"package_type": "cask"})
        );
        assert_eq!(
            packages[0].targets[0].plugin_config_name,
            "Homebrew (Casks)"
        );
        assert_eq!(packages[0].targets[0].roles.len(), 3);
    }

    #[test]
    fn parse_installed_casks_skips_not_installed() {
        let json = sample_cask_json_not_installed();
        let packages = HomebrewPlugin::parse_installed_casks(&json);
        assert!(packages.is_empty());
    }

    #[test]
    fn parse_installed_casks_skips_auto_updates_true() {
        let json = serde_json::json!({
            "formulae": [],
            "casks": [
                {
                    "token": "google-chrome",
                    "name": ["Google Chrome"],
                    "version": "130.0",
                    "installed": "129.0",
                    "auto_updates": true
                },
                {
                    "token": "firefox",
                    "name": ["Mozilla Firefox"],
                    "version": "133.0",
                    "installed": "132.0",
                    "auto_updates": false
                }
            ]
        });
        let packages = HomebrewPlugin::parse_installed_casks(&json);
        // google-chrome (auto_updates=true) is excluded; firefox (auto_updates=false) is included.
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].package_identifier, "firefox");
    }

    #[test]
    fn parse_installed_casks_includes_cask_without_auto_updates_field() {
        let json = serde_json::json!({
            "formulae": [],
            "casks": [{
                "token": "iterm2",
                "name": ["iTerm2"],
                "version": "3.5.0",
                "installed": "3.4.23"
            }]
        });
        let packages = HomebrewPlugin::parse_installed_casks(&json);
        // No auto_updates field → defaults to false → included.
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].package_identifier, "iterm2");
    }

    #[test]
    fn parse_installed_casks_skips_latest_version() {
        let json = serde_json::json!({
            "formulae": [],
            "casks": [
                {
                    "token": "some-cask",
                    "name": ["Some Cask"],
                    "version": "latest",
                    "installed": "latest"
                },
                {
                    "token": "iterm2",
                    "name": ["iTerm2"],
                    "version": "3.5.0",
                    "installed": "3.4.23"
                }
            ]
        });
        let packages = HomebrewPlugin::parse_installed_casks(&json);
        // "latest" cask excluded; iterm2 included.
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].package_identifier, "iterm2");
    }

    #[test]
    fn parse_installed_formulae_skips_latest_version() {
        let json = serde_json::json!({
            "formulae": [
                {
                    "name": "some-formula",
                    "full_name": "some-formula",
                    "versions": { "stable": "latest" },
                    "installed": [{ "version": "latest" }]
                },
                {
                    "name": "wget",
                    "full_name": "wget",
                    "versions": { "stable": "1.24.5" },
                    "installed": [{ "version": "1.24.4" }]
                }
            ],
            "casks": []
        });
        let packages = HomebrewPlugin::parse_installed_formulae(&json);
        // "latest" formula excluded; wget included.
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].package_identifier, "wget");
    }

    #[test]
    fn parse_installed_packages_empty() {
        let json = serde_json::json!({"formulae": [], "casks": []});
        let packages = HomebrewPlugin::parse_installed_formulae(&json);
        assert!(packages.is_empty());
    }

    // ── detect_host_compatibility ────────────────────────────────────────

    #[tokio::test]
    async fn detect_host_compatibility_compatible_when_which_exits_zero() {
        let plugin = HomebrewPlugin::new(
            HomebrewConfig::default(),
            test_runtime_with_executor(FixedOutputExecutor::failure(0)),
        )
        .expect("create");
        let result = plugin.detect_host_compatibility().await.expect("ok");
        assert_eq!(result, HostCompatibility::Compatible);
    }

    #[tokio::test]
    async fn detect_host_compatibility_incompatible_when_which_exits_nonzero() {
        let plugin = HomebrewPlugin::new(
            HomebrewConfig::default(),
            test_runtime_with_executor(FixedOutputExecutor::failure(1)),
        )
        .expect("create");
        let result = plugin.detect_host_compatibility().await.expect("ok");
        match result {
            HostCompatibility::Incompatible(msg) => {
                assert_eq!(msg, "brew not found");
            }
            HostCompatibility::Compatible => panic!("expected Incompatible"),
            _ => panic!("unexpected HostCompatibility variant"),
        }
    }

    // ── empty identifier guards ──────────────────────────────────────────

    #[tokio::test]
    async fn homebrew_plugin_detect_installed_empty_identifier_fails() {
        use uptrakit_plugin_infrastructure_core::VersionDetector;
        let plugin =
            HomebrewPlugin::new(HomebrewConfig::default(), test_runtime()).expect("create");
        let result = plugin.detect_installed_version("").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn homebrew_plugin_fetch_releases_empty_identifier_fails() {
        use uptrakit_plugin_infrastructure_core::ReleaseFetcher;
        let plugin =
            HomebrewPlugin::new(HomebrewConfig::default(), test_runtime()).expect("create");
        let result = plugin.fetch_releases("").await;
        assert!(result.is_err());
    }
}
