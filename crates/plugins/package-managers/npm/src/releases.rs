use async_trait::async_trait;
use backon::{ExponentialBuilder, Retryable};
use rootcause::prelude::*;
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

        let builder = ExponentialBuilder::default()
            .with_min_delay(FETCH_BACKOFF_BASE)
            .with_max_delay(FETCH_BACKOFF_MAX)
            .with_jitter()
            // 2 retries after the first attempt = 3 total (preserves FETCH_MAX_RETRIES).
            .with_max_times(FETCH_MAX_RETRIES - 1);

        let fetch = || async {
            let response = self
                .client
                .get(&url)
                .send()
                .await
                // Pre-HTTP transport error (TCP/TLS/DNS): retryable → PluginInternal.
                .map_err(|e| {
                    report!(PluginError::PluginInternal(format!(
                        "npm registry request failed: {e}"
                    )))
                })?;

            let status = response.status();

            // 404 is a permanent "no releases" condition — terminal success, never an error.
            if status == reqwest::StatusCode::NOT_FOUND {
                tracing::debug!(package = %package_identifier, "package not found in npm registry");
                return Ok(Vec::new());
            }

            // 5xx: registry overload / rate-limit — retryable → PluginInternal.
            if status.is_server_error() {
                bail!(PluginError::PluginInternal(format!(
                    "npm registry returned HTTP {status}"
                )));
            }

            // Other non-success (4xx) — terminal, do NOT retry → Configuration.
            if !status.is_success() {
                bail!(PluginError::Configuration(format!(
                    "npm registry returned HTTP {status}"
                )));
            }

            // Malformed body on a 2xx — terminal, retrying won't help → Serialization.
            let json: serde_json::Value = response.json().await.map_err(|e| {
                report!(PluginError::Serialization(format!(
                    "failed to parse npm registry response: {e}"
                )))
            })?;

            let releases = self.parse_registry_response(&json, package_identifier);
            tracing::debug!(
                package = %package_identifier,
                count = releases.len(),
                "npm releases fetched"
            );
            Ok(releases)
        };

        fetch
            .retry(builder)
            .when(|e: &rootcause::Report<PluginError>| e.current_context().is_retryable())
            .notify(|e, delay| {
                tracing::warn!(
                    package = %package_identifier,
                    delay_ms = delay.as_millis(),
                    error = %e,
                    "transient npm registry error; retrying"
                );
            })
            .await
    }
}

#[cfg(test)]
mod fetch_branch_tests {
    use std::sync::Arc;

    use httpmock::prelude::*;
    use uptrakit_command::NoopCommandExecutor;
    use uptrakit_plugin_infrastructure_core::ReleaseFetcher;
    use uptrakit_plugin_infrastructure_core::{
        PluginHttpClientConfig, SsrfMode, build_plugin_http_client,
    };

    use crate::config::NpmConfig;
    use crate::plugin::NpmPlugin;

    /// Build an NpmPlugin whose HTTP client uses SsrfSafeResolver::permissive() so it
    /// can reach the httpmock server on 127.0.0.1.
    fn test_plugin_for_mock(server: &MockServer) -> NpmPlugin {
        let client = build_plugin_http_client(PluginHttpClientConfig {
            user_agent: "uptrakit-plugin-package-manager-npm-test",
            ssrf_mode: SsrfMode::Permissive,
            ..PluginHttpClientConfig::default()
        })
        .expect("build test HTTP client");

        NpmPlugin {
            config: NpmConfig {
                include_prereleases: false,
                registry_url: Some(server.base_url()),
            },
            executor: Arc::new(NoopCommandExecutor),
            client,
        }
    }

    /// 404 → Ok(empty vec), server must receive exactly 1 request (no retry).
    #[tokio::test]
    async fn not_found_returns_empty_without_retry() {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(GET).path("/lodash");
                then.status(404);
            })
            .await;

        let plugin = test_plugin_for_mock(&server);
        let result = plugin.fetch_releases("lodash").await;

        assert!(result.is_ok(), "expected Ok, got: {result:?}");
        assert!(result.unwrap().is_empty());
        mock.assert_calls_async(1).await;
    }

    /// 403 → Err (non-retryable Configuration error), server must receive exactly 1 request.
    #[tokio::test]
    async fn client_error_is_terminal_no_retry() {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(GET).path("/lodash");
                then.status(403);
            })
            .await;

        let plugin = test_plugin_for_mock(&server);
        let result = plugin.fetch_releases("lodash").await;

        assert!(result.is_err(), "expected Err for 403");
        mock.assert_calls_async(1).await;
    }

    /// Always-500 → Err after 3 total attempts (FETCH_MAX_RETRIES = 3).
    /// Does NOT use start_paused — real httpmock server, real backoff (~3s).
    #[tokio::test]
    async fn server_error_retries_three_times() {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(GET).path("/lodash");
                then.status(500);
            })
            .await;

        let plugin = test_plugin_for_mock(&server);
        let result = plugin.fetch_releases("lodash").await;

        assert!(result.is_err(), "expected Err after all retries exhausted");
        mock.assert_calls_async(3).await;
    }
}
