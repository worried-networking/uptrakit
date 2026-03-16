use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::command::CommandSpec;
use uptrakit_plugin_infrastructure_core::{
    BatchDetectItem, BatchDetectResult, PluginError, Result, Version, execute_and_capture,
};

use crate::plugin::{HomebrewPlugin, validate_identifier};

impl HomebrewPlugin {
    /// Parse the installed version from `brew info --json=v2` output for a
    /// specific package.
    pub(crate) fn parse_installed_version(
        json: &serde_json::Value,
        pkg: &str,
        is_cask: bool,
    ) -> Option<String> {
        if is_cask {
            let casks = json.get("casks")?.as_array()?;
            let cask = casks
                .iter()
                .find(|c| c.get("token").and_then(|t| t.as_str()) == Some(pkg))?;
            cask.get("installed")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        } else {
            let formulae = json.get("formulae")?.as_array()?;
            let formula = formulae
                .iter()
                .find(|f| f.get("name").and_then(|n| n.as_str()) == Some(pkg))?;
            let installed = formula.get("installed")?.as_array()?;
            installed
                .first()?
                .get("version")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        }
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::VersionDetectorPlugin for HomebrewPlugin {
    #[tracing::instrument(skip_all)]
    async fn detect_installed_version(&self, package_identifier: &str) -> Result<Option<Version>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "detecting installed Homebrew version");
        let stdout = execute_and_capture(
            self.executor.as_ref(),
            CommandSpec::exec(
                "brew",
                [
                    "info".to_string(),
                    "--json=v2".to_string(),
                    package_identifier.to_string(),
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

        let version = Self::parse_installed_version(&json, package_identifier, self.is_cask())
            .map(|v| Version::new(&v));
        tracing::debug!(version = ?version, "Homebrew version detection result");
        Ok(version)
    }

    /// Detect installed versions for multiple packages using a single `brew info` call.
    ///
    /// Runs:
    /// ```text
    /// brew info --json=v2 pkg1 pkg2 pkg3
    /// ```
    ///
    /// Parses the returned JSON once and looks up each package individually using the
    /// existing [`parse_installed_version`](Self::parse_installed_version) helper. If
    /// the command fails, all items receive the same error.
    #[tracing::instrument(skip_all)]
    async fn batch_detect_installed_version(
        &self,
        items: &[BatchDetectItem],
    ) -> Result<Vec<BatchDetectResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        // Validate all identifiers up front.
        for item in items {
            validate_identifier(&item.package_identifier)
                .map_err(|e| report!(PluginError::Configuration(e)))?;
        }

        let mut args = vec!["info".to_string(), "--json=v2".to_string()];
        for item in items {
            args.push(item.package_identifier.clone());
        }

        tracing::debug!(
            count = items.len(),
            "batch detecting Homebrew installed versions"
        );

        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec("brew", args))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "brew info failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            // Fail all items with the same error.
            let error_str = format!("brew info exited with code {}", cmd_output.exit_code);
            return Ok(items
                .iter()
                .map(|item| {
                    BatchDetectResult::error(item.package_identifier.clone(), error_str.clone())
                })
                .collect());
        }

        let json: serde_json::Value = serde_json::from_str(&cmd_output.output).map_err(|e| {
            report!(PluginError::PluginInternal(format!(
                "failed to parse brew info JSON: {e}"
            )))
        })?;

        let is_cask = self.is_cask();
        let results = items
            .iter()
            .map(|item| {
                let installed_version =
                    Self::parse_installed_version(&json, &item.package_identifier, is_cask)
                        .map(|v| Version::new(&v));
                BatchDetectResult::new(item.package_identifier.clone(), installed_version, None)
            })
            .collect();

        tracing::debug!(
            count = items.len(),
            "Homebrew batch version detection complete"
        );
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use uptrakit_plugin_infrastructure_core::command::CommandExecutor;
    use uptrakit_plugin_infrastructure_core::testing::FixedOutputExecutor;
    use uptrakit_plugin_infrastructure_core::{
        BatchDetectItem, LocalCommandExecutor, Version, VersionDetectorPlugin,
    };

    use crate::config::{HomebrewConfig, HomebrewPackageType};

    fn test_executor() -> Arc<dyn CommandExecutor> {
        Arc::new(LocalCommandExecutor)
    }

    fn sample_formula_json() -> serde_json::Value {
        serde_json::json!({
            "formulae": [{
                "name": "wget",
                "full_name": "wget",
                "versions": {
                    "stable": "1.24.5",
                    "head": null
                },
                "installed": [{
                    "version": "1.24.4",
                    "installed_as_dependency": false
                }],
                "homepage": "https://www.gnu.org/software/wget/"
            }],
            "casks": []
        })
    }

    fn sample_formula_json_not_installed() -> serde_json::Value {
        serde_json::json!({
            "formulae": [{
                "name": "wget",
                "full_name": "wget",
                "versions": {
                    "stable": "1.24.5",
                    "head": null
                },
                "installed": [],
                "homepage": "https://www.gnu.org/software/wget/"
            }],
            "casks": []
        })
    }

    fn sample_cask_json() -> serde_json::Value {
        serde_json::json!({
            "formulae": [],
            "casks": [{
                "token": "firefox",
                "name": ["Mozilla Firefox"],
                "version": "133.0",
                "installed": "132.0",
                "homepage": "https://www.mozilla.org/firefox/"
            }]
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

    fn multi_formula_json() -> serde_json::Value {
        serde_json::json!({
            "formulae": [
                {
                    "name": "wget",
                    "full_name": "wget",
                    "versions": { "stable": "1.24.5" },
                    "installed": [{ "version": "1.24.4" }],
                    "homepage": "https://www.gnu.org/software/wget/"
                },
                {
                    "name": "jq",
                    "full_name": "jq",
                    "versions": { "stable": "1.7.1" },
                    "installed": [{ "version": "1.7.1" }],
                    "homepage": "https://jqlang.github.io/jq/"
                },
                {
                    "name": "curl",
                    "full_name": "curl",
                    "versions": { "stable": "8.5.0" },
                    "installed": [],
                    "homepage": "https://curl.se/"
                }
            ],
            "casks": []
        })
    }

    fn multi_cask_json() -> serde_json::Value {
        serde_json::json!({
            "formulae": [],
            "casks": [
                {
                    "token": "firefox",
                    "name": ["Mozilla Firefox"],
                    "version": "133.0",
                    "installed": "132.0",
                    "homepage": "https://www.mozilla.org/firefox/"
                },
                {
                    "token": "google-chrome",
                    "name": ["Google Chrome"],
                    "version": "120.0",
                    "installed": null,
                    "homepage": "https://www.google.com/chrome/"
                }
            ]
        })
    }

    // ── parse_installed_version ─────────────────────────────────────────

    #[test]
    fn parse_installed_version_formula() {
        let json = sample_formula_json();
        let version = HomebrewPlugin::parse_installed_version(&json, "wget", false);
        assert_eq!(version, Some("1.24.4".to_string()));
    }

    #[test]
    fn parse_installed_version_formula_not_installed() {
        let json = sample_formula_json_not_installed();
        let version = HomebrewPlugin::parse_installed_version(&json, "wget", false);
        assert_eq!(version, None);
    }

    #[test]
    fn parse_installed_version_cask() {
        let json = sample_cask_json();
        let version = HomebrewPlugin::parse_installed_version(&json, "firefox", true);
        assert_eq!(version, Some("132.0".to_string()));
    }

    #[test]
    fn parse_installed_version_cask_not_installed() {
        let json = sample_cask_json_not_installed();
        let version = HomebrewPlugin::parse_installed_version(&json, "firefox", true);
        assert_eq!(version, None);
    }

    #[test]
    fn parse_installed_version_unknown_package() {
        let json = sample_formula_json();
        let version = HomebrewPlugin::parse_installed_version(&json, "nonexistent", false);
        assert_eq!(version, None);
    }

    // ── batch_detect_installed_version ───────────────────────────────────

    #[tokio::test]
    async fn batch_detect_installed_version_formulae() {
        let plugin = HomebrewPlugin::new(
            HomebrewConfig {
                package_type: HomebrewPackageType::Formula,
            },
            FixedOutputExecutor::success(multi_formula_json().to_string()),
        )
        .await
        .expect("create");

        let items = vec![
            BatchDetectItem::new("wget".to_string()),
            BatchDetectItem::new("jq".to_string()),
            BatchDetectItem::new("curl".to_string()),
        ];
        let results = plugin
            .batch_detect_installed_version(&items)
            .await
            .expect("ok");

        assert_eq!(results.len(), 3);

        let wget = results
            .iter()
            .find(|r| r.package_identifier == "wget")
            .unwrap();
        assert_eq!(wget.installed_version, Some(Version::new("1.24.4")));
        assert!(wget.error.is_none());

        let jq = results
            .iter()
            .find(|r| r.package_identifier == "jq")
            .unwrap();
        assert_eq!(jq.installed_version, Some(Version::new("1.7.1")));

        let curl = results
            .iter()
            .find(|r| r.package_identifier == "curl")
            .unwrap();
        assert!(
            curl.installed_version.is_none(),
            "curl has empty installed array"
        );
        assert!(curl.error.is_none());
    }

    #[tokio::test]
    async fn batch_detect_installed_version_casks() {
        let plugin = HomebrewPlugin::new(
            HomebrewConfig {
                package_type: HomebrewPackageType::Cask,
            },
            FixedOutputExecutor::success(multi_cask_json().to_string()),
        )
        .await
        .expect("create");

        let items = vec![
            BatchDetectItem::new("firefox".to_string()),
            BatchDetectItem::new("google-chrome".to_string()),
        ];
        let results = plugin
            .batch_detect_installed_version(&items)
            .await
            .expect("ok");

        assert_eq!(results.len(), 2);

        let firefox = results
            .iter()
            .find(|r| r.package_identifier == "firefox")
            .unwrap();
        assert_eq!(firefox.installed_version, Some(Version::new("132.0")));

        let chrome = results
            .iter()
            .find(|r| r.package_identifier == "google-chrome")
            .unwrap();
        assert!(
            chrome.installed_version.is_none(),
            "chrome is not installed (installed: null)"
        );
        assert!(chrome.error.is_none());
    }

    #[tokio::test]
    async fn batch_detect_installed_version_empty_returns_empty() {
        let plugin = HomebrewPlugin::new(HomebrewConfig::default(), test_executor())
            .await
            .expect("create");
        let results = plugin
            .batch_detect_installed_version(&[])
            .await
            .expect("ok");
        assert!(results.is_empty());
    }
}
