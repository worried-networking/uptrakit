use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::command::CommandSpec;
use uptrakit_plugin_infrastructure_core::{
    BatchFetchItem, BatchFetchResult, PluginError, Result, UpstreamRelease, Version,
    execute_and_capture,
};

use crate::plugin::{HomebrewPlugin, validate_identifier};

impl HomebrewPlugin {
    /// Parse the latest available version from `brew info --json=v2` output for
    /// a specific package.
    pub(crate) fn parse_latest_version(
        json: &serde_json::Value,
        pkg: &str,
        is_cask: bool,
    ) -> Option<String> {
        if is_cask {
            let casks = json.get("casks")?.as_array()?;
            let cask = casks
                .iter()
                .find(|c| c.get("token").and_then(|t| t.as_str()) == Some(pkg))?;
            cask.get("version")
                .and_then(|v| v.as_str())
                .filter(|v| *v != "latest")
                .map(|s| s.to_string())
        } else {
            let formulae = json.get("formulae")?.as_array()?;
            let formula = formulae
                .iter()
                .find(|f| f.get("name").and_then(|n| n.as_str()) == Some(pkg))?;
            let versions = formula.get("versions")?;
            versions
                .get("stable")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        }
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::ReleaseFetcher for HomebrewPlugin {
    #[tracing::instrument(skip_all)]
    async fn fetch_releases(&self, package_identifier: &str) -> Result<Vec<UpstreamRelease>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "fetching Homebrew releases");
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

        let Some(version_str) =
            Self::parse_latest_version(&json, package_identifier, self.is_cask())
        else {
            return Ok(vec![]);
        };

        let homepage = if self.is_cask() {
            json.get("casks")
                .and_then(|c| c.as_array())
                .and_then(|arr| arr.first())
                .and_then(|c| c.get("homepage"))
                .and_then(|h| h.as_str())
                .unwrap_or("")
        } else {
            json.get("formulae")
                .and_then(|f| f.as_array())
                .and_then(|arr| arr.first())
                .and_then(|f| f.get("homepage"))
                .and_then(|h| h.as_str())
                .unwrap_or("")
        };

        let releases = vec![{
            let mut r = UpstreamRelease::new(Version::new(&version_str), version_str, false, "");
            r.release_url = homepage.to_string();
            r
        }];
        tracing::debug!(count = releases.len(), "Homebrew releases fetched");
        Ok(releases)
    }

    /// Fetch available releases for multiple packages using a single `brew info` call.
    ///
    /// Runs:
    /// ```text
    /// brew info --json=v2 pkg1 pkg2 pkg3
    /// ```
    ///
    /// Parses the returned JSON once and resolves the latest version and homepage for
    /// each package individually. If the command fails, all items receive the same error.
    #[tracing::instrument(skip_all)]
    async fn batch_fetch(&self, items: &[BatchFetchItem]) -> Result<Vec<BatchFetchResult>> {
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

        tracing::debug!(count = items.len(), "batch fetching Homebrew releases");

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
            let error_str = format!("brew info exited with code {}", cmd_output.exit_code);
            return Ok(items
                .iter()
                .map(|item| {
                    BatchFetchResult::error(item.package_identifier.clone(), error_str.clone())
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
                let Some(version_str) =
                    Self::parse_latest_version(&json, &item.package_identifier, is_cask)
                else {
                    return BatchFetchResult::empty(item.package_identifier.clone());
                };

                let homepage = if is_cask {
                    Self::find_cask_homepage(&json, &item.package_identifier)
                } else {
                    Self::find_formula_homepage(&json, &item.package_identifier)
                };

                BatchFetchResult::found(
                    item.package_identifier.clone(),
                    vec![{
                        let mut r = UpstreamRelease::new(
                            Version::new(&version_str),
                            version_str,
                            false,
                            "",
                        );
                        r.release_url = homepage;
                        r
                    }],
                )
            })
            .collect();

        tracing::debug!(count = items.len(), "Homebrew batch fetch complete");
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
        BatchFetchItem, HostCapabilities, HostRuntime, PosixHostRuntime, ReleaseFetcher,
    };

    use crate::config::{HomebrewConfig, HomebrewPackageType};

    fn test_runtime() -> Arc<dyn HostRuntime> {
        let executor = Arc::new(uptrakit_plugin_infrastructure_core::LocalCommandExecutor)
            as Arc<dyn CommandExecutor>;
        Arc::new(PosixHostRuntime::new(executor, HostCapabilities::default()))
    }

    fn test_runtime_with_executor(executor: Arc<dyn CommandExecutor>) -> Arc<dyn HostRuntime> {
        Arc::new(PosixHostRuntime::new(executor, HostCapabilities::default()))
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

    fn sample_cask_latest_version_json() -> serde_json::Value {
        serde_json::json!({
            "formulae": [],
            "casks": [{
                "token": "google-chrome",
                "name": ["Google Chrome"],
                "version": "latest",
                "installed": "latest",
                "homepage": "https://www.google.com/chrome/"
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

    // ── parse_latest_version ────────────────────────────────────────────

    #[test]
    fn parse_latest_version_formula() {
        let json = sample_formula_json();
        let version = HomebrewPlugin::parse_latest_version(&json, "wget", false);
        assert_eq!(version, Some("1.24.5".to_string()));
    }

    #[test]
    fn parse_latest_version_cask() {
        let json = sample_cask_json();
        let version = HomebrewPlugin::parse_latest_version(&json, "firefox", true);
        assert_eq!(version, Some("133.0".to_string()));
    }

    #[test]
    fn parse_latest_version_cask_with_latest_marker() {
        let json = sample_cask_latest_version_json();
        let version = HomebrewPlugin::parse_latest_version(&json, "google-chrome", true);
        // "latest" is filtered out — not a useful version string
        assert_eq!(version, None);
    }

    #[test]
    fn parse_latest_version_unknown_package() {
        let json = sample_formula_json();
        let version = HomebrewPlugin::parse_latest_version(&json, "nonexistent", false);
        assert_eq!(version, None);
    }

    // ── batch_fetch_releases ─────────────────────────────────────────────

    #[tokio::test]
    async fn batch_fetch_releases_formulae() {
        let plugin = HomebrewPlugin::new(
            HomebrewConfig {
                package_type: HomebrewPackageType::Formula,
            },
            test_runtime_with_executor(FixedOutputExecutor::success(
                multi_formula_json().to_string(),
            )),
        )
        .expect("create");

        let items = vec![
            BatchFetchItem::new("wget".to_string()),
            BatchFetchItem::new("jq".to_string()),
            BatchFetchItem::new("curl".to_string()),
        ];
        let results = plugin.batch_fetch(&items).await.expect("ok");

        assert_eq!(results.len(), 3);

        let wget = results
            .iter()
            .find(|r| r.package_identifier == "wget")
            .unwrap();
        assert_eq!(wget.releases.len(), 1);
        assert_eq!(wget.releases[0].tag, "1.24.5");
        assert_eq!(
            wget.releases[0].release_url,
            "https://www.gnu.org/software/wget/"
        );
        assert!(wget.error.is_none());

        let jq = results
            .iter()
            .find(|r| r.package_identifier == "jq")
            .unwrap();
        assert_eq!(jq.releases.len(), 1);
        assert_eq!(jq.releases[0].release_url, "https://jqlang.github.io/jq/");

        let curl = results
            .iter()
            .find(|r| r.package_identifier == "curl")
            .unwrap();
        assert_eq!(curl.releases.len(), 1, "curl has a latest stable version");
        assert_eq!(curl.releases[0].tag, "8.5.0");
    }

    #[tokio::test]
    async fn batch_fetch_releases_casks() {
        let plugin = HomebrewPlugin::new(
            HomebrewConfig {
                package_type: HomebrewPackageType::Cask,
            },
            test_runtime_with_executor(FixedOutputExecutor::success(multi_cask_json().to_string())),
        )
        .expect("create");

        let items = vec![
            BatchFetchItem::new("firefox".to_string()),
            BatchFetchItem::new("google-chrome".to_string()),
        ];
        let results = plugin.batch_fetch(&items).await.expect("ok");

        assert_eq!(results.len(), 2);

        let firefox = results
            .iter()
            .find(|r| r.package_identifier == "firefox")
            .unwrap();
        assert_eq!(firefox.releases.len(), 1);
        assert_eq!(firefox.releases[0].tag, "133.0");
        assert_eq!(
            firefox.releases[0].release_url,
            "https://www.mozilla.org/firefox/"
        );

        let chrome = results
            .iter()
            .find(|r| r.package_identifier == "google-chrome")
            .unwrap();
        assert_eq!(chrome.releases.len(), 1);
        assert_eq!(
            chrome.releases[0].release_url,
            "https://www.google.com/chrome/"
        );
    }

    #[tokio::test]
    async fn batch_fetch_releases_empty_returns_empty() {
        let plugin =
            HomebrewPlugin::new(HomebrewConfig::default(), test_runtime()).expect("create");
        let results = plugin.batch_fetch(&[]).await.expect("ok");
        assert!(results.is_empty());
    }
}
