use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::command::CommandSpec;
use uptrakit_plugin_infrastructure_core::{
    BatchFetchItem, BatchFetchResult, PluginError, Result, UpstreamRelease, Version,
};

use crate::plugin::{PacmanPlugin, validate_identifier};

#[async_trait]
impl uptrakit_plugin_infrastructure_core::ReleaseFetcherPlugin for PacmanPlugin {
    #[tracing::instrument(skip_all)]
    async fn fetch_releases(&self, package_identifier: &str) -> Result<Vec<UpstreamRelease>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "fetching Pacman releases via pacman -Si");

        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "pacman",
                ["-Si".to_string(), package_identifier.to_string()],
            ))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "pacman -Si failed: {e}"
                )))
            })?;

        match cmd_output.exit_code {
            0 => {}
            // Exit code 1 means the package was not found in any repository.
            1 => {
                tracing::debug!(
                    package = %package_identifier,
                    "package not found in any configured repository"
                );
                return Ok(vec![]);
            }
            code => bail!(PluginError::CommandFailed(code)),
        }

        let Some(version) = PacmanPlugin::parse_si_output(&cmd_output.output) else {
            return Ok(vec![]);
        };

        tracing::debug!(
            version = %version,
            "Pacman upstream version resolved"
        );
        Ok(vec![UpstreamRelease::new(
            Version::new(&version),
            version,
            false,
            "",
        )])
    }

    /// Fetch available releases for multiple packages using a single
    /// `pacman -Si` call.
    ///
    /// Runs:
    /// ```text
    /// pacman -Si pkg1 pkg2 pkg3
    /// ```
    ///
    /// Output blocks (one per package) are separated by blank lines. Only
    /// packages present in the output have releases; absent packages have
    /// empty releases. The exit code is intentionally ignored because
    /// `pacman -Si` exits non-zero when any package is not in the repos.
    #[tracing::instrument(skip_all)]
    async fn batch_fetch_releases(
        &self,
        items: &[BatchFetchItem],
    ) -> Result<Vec<BatchFetchResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        // Validate all identifiers up front.
        for item in items {
            validate_identifier(&item.package_identifier)
                .map_err(|e| report!(PluginError::Configuration(e)))?;
        }

        let mut args = vec!["-Si".to_string()];
        for item in items {
            args.push(item.package_identifier.clone());
        }

        tracing::debug!(
            count = items.len(),
            "batch fetching Pacman releases via pacman -Si"
        );

        // Non-zero exit is expected when any package is not in repos; ignore it.
        let stdout = match self
            .executor
            .execute_quiet(&CommandSpec::exec("pacman", args))
            .await
        {
            Ok(o) => o.output,
            Err(e) => {
                let error_str = format!("pacman -Si failed: {e}");
                return Ok(items
                    .iter()
                    .map(|item| {
                        BatchFetchResult::error(item.package_identifier.clone(), error_str.clone())
                    })
                    .collect());
            }
        };

        let parsed = PacmanPlugin::parse_si_output_batch(&stdout);

        let results = items
            .iter()
            .map(|item| {
                let Some(version) = parsed.get(&item.package_identifier) else {
                    // Package not found in any configured repository.
                    return BatchFetchResult::empty(item.package_identifier.clone());
                };

                BatchFetchResult::found(
                    item.package_identifier.clone(),
                    vec![UpstreamRelease::new(
                        Version::new(version),
                        version.clone(),
                        false,
                        "",
                    )],
                )
            })
            .collect();

        tracing::debug!(count = items.len(), "Pacman batch fetch complete");
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use uptrakit_plugin_infrastructure_core::testing::RoutedOutputExecutor;
    use uptrakit_plugin_infrastructure_core::{BatchFetchItem, ReleaseFetcherPlugin};

    use crate::config::PacmanConfig;
    use crate::plugin::PacmanPlugin;

    // ── batch_fetch_releases ─────────────────────────────────────────────────

    #[tokio::test]
    async fn batch_fetch_releases_mixed_packages() {
        let executor = RoutedOutputExecutor::success([(
            "pacman",
            concat!(
                "Repository      : extra\n",
                "Name            : nginx\n",
                "Version         : 1.26.3-1\n",
                "\n",
                "Repository      : core\n",
                "Name            : git\n",
                "Version         : 2.47.2-1\n",
            ),
        )]);
        let plugin = PacmanPlugin::new(PacmanConfig::default(), executor)
            .await
            .expect("create");

        let items = vec![
            BatchFetchItem::new("nginx".to_string()),
            BatchFetchItem::new("git".to_string()),
            BatchFetchItem::new("curl".to_string()),
        ];
        let results = plugin.batch_fetch_releases(&items).await.expect("ok");

        assert_eq!(results.len(), 3);

        let nginx = results
            .iter()
            .find(|r| r.package_identifier == "nginx")
            .unwrap();
        assert_eq!(nginx.releases.len(), 1);
        assert_eq!(nginx.releases[0].tag, "1.26.3-1");
        assert!(nginx.error.is_none());

        let git = results
            .iter()
            .find(|r| r.package_identifier == "git")
            .unwrap();
        assert_eq!(git.releases.len(), 1);
        assert_eq!(git.releases[0].tag, "2.47.2-1");
        assert!(git.error.is_none());

        let curl = results
            .iter()
            .find(|r| r.package_identifier == "curl")
            .unwrap();
        assert!(curl.releases.is_empty(), "absent package has no releases");
        assert!(curl.error.is_none(), "absent package is not an error");
    }

    #[tokio::test]
    async fn batch_fetch_releases_empty_items_returns_empty() {
        use std::sync::Arc;
        use uptrakit_plugin_infrastructure_core::LocalCommandExecutor;

        let plugin = PacmanPlugin::new(PacmanConfig::default(), Arc::new(LocalCommandExecutor))
            .await
            .expect("create");
        let results = plugin.batch_fetch_releases(&[]).await.expect("ok");
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn batch_fetch_releases_invalid_identifier_fails() {
        use std::sync::Arc;
        use uptrakit_plugin_infrastructure_core::LocalCommandExecutor;

        let plugin = PacmanPlugin::new(PacmanConfig::default(), Arc::new(LocalCommandExecutor))
            .await
            .expect("create");
        let items = vec![BatchFetchItem::new("INVALID".to_string())];
        let result = plugin.batch_fetch_releases(&items).await;
        assert!(result.is_err());
    }
}
