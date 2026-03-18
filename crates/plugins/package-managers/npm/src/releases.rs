use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_backoff::Backoff;
use uptrakit_plugin_infrastructure_core::{PluginError, Result, UpstreamRelease};

use crate::plugin::{
    FETCH_BACKOFF_BASE, FETCH_BACKOFF_MAX, FETCH_MAX_RETRIES, NpmPlugin, npm_registry_url,
};

#[async_trait]
impl uptrakit_plugin_infrastructure_core::ReleaseFetcher for NpmPlugin {
    #[tracing::instrument(skip_all)]
    async fn fetch_releases(&self, package_identifier: &str) -> Result<Vec<UpstreamRelease>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "fetching npm releases from registry");

        let url = npm_registry_url(package_identifier, self.config.registry_url.as_deref());
        let mut backoff = Backoff::new(FETCH_BACKOFF_BASE, FETCH_BACKOFF_MAX);
        let mut last_err: Option<String> = None;

        for attempt in 1..=FETCH_MAX_RETRIES {
            let response = match self.client.get(&url).send().await {
                Ok(r) => r,
                Err(e) => {
                    let msg = format!("npm registry request failed: {e}");
                    tracing::warn!(
                        package = %package_identifier,
                        attempt,
                        error = %e,
                        "transient npm registry request error; will retry"
                    );
                    last_err = Some(msg);
                    if attempt < FETCH_MAX_RETRIES {
                        tokio::time::sleep(backoff.next_delay()).await;
                    }
                    continue;
                }
            };

            let status = response.status();

            // 404 is a permanent, non-retryable condition.
            if status == reqwest::StatusCode::NOT_FOUND {
                tracing::debug!(package = %package_identifier, "package not found in npm registry");
                return Ok(vec![]);
            }

            // 5xx responses are transient; retry after backoff.
            if status.is_server_error() {
                let msg = format!("npm registry returned HTTP {status}");
                tracing::warn!(
                    package = %package_identifier,
                    attempt,
                    %status,
                    "transient npm registry server error; will retry"
                );
                last_err = Some(msg);
                if attempt < FETCH_MAX_RETRIES {
                    tokio::time::sleep(backoff.next_delay()).await;
                }
                continue;
            }

            if !status.is_success() {
                bail!(PluginError::PluginInternal(format!(
                    "npm registry returned HTTP {status}"
                )));
            }

            let json: serde_json::Value = response.json().await.map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "failed to parse npm registry response: {e}"
                )))
            })?;

            let releases = self.parse_registry_response(&json, package_identifier);
            tracing::debug!(
                package = %package_identifier,
                count = releases.len(),
                "npm releases fetched"
            );
            return Ok(releases);
        }

        // All retries exhausted.
        bail!(PluginError::PluginInternal(last_err.unwrap_or_else(|| {
            "npm registry request failed after retries".to_string()
        })));
    }
}
