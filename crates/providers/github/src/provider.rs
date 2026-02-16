use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;
use rootcause::prelude::*;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::sync::mpsc;

use uptrakit_provider_core::command::{CommandExecutor, CommandSpec, send_output, shell_escape};
use uptrakit_provider_core::{
    Provider, ProviderError, ProviderType, ReleaseAsset, ReleaseInfo, UpdateOutputLine,
    UpdateOutputStream, UpstreamRelease, Version,
};

use crate::api_types::{GitHubApiError, GitHubRelease};
use crate::config::GitHubConfig;
use crate::error::{GitHubError, Result};
use crate::tag::strip_tag_prefix;

/// GitHub Releases provider implementation.
///
/// Fetches release metadata from the GitHub API and converts it into
/// `UpstreamRelease` values for the controller.
pub struct GitHubProvider {
    client: reqwest::Client,
    config: GitHubConfig,
    asset_filters: Vec<Regex>,
    executor: Arc<dyn CommandExecutor>,
}

impl GitHubProvider {
    /// Create a new `GitHubProvider` from the given configuration.
    ///
    /// Validates the configuration and pre-compiles asset filter regexes.
    pub fn new(config: GitHubConfig, executor: Arc<dyn CommandExecutor>) -> Result<Self> {
        config
            .validate()
            .map_err(|e| report!(GitHubError::Configuration(e.to_string())))?;

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::ACCEPT,
            reqwest::header::HeaderValue::from_static("application/vnd.github+json"),
        );
        headers.insert(
            "X-GitHub-Api-Version",
            reqwest::header::HeaderValue::from_static("2022-11-28"),
        );

        if let Some(ref token) = config.auth_token {
            let value = format!("Bearer {}", token.expose_secret());
            let header_value = reqwest::header::HeaderValue::from_str(&value).map_err(|e| {
                report!(GitHubError::Configuration(format!(
                    "invalid auth token header value: {e}"
                )))
            })?;
            headers.insert(reqwest::header::AUTHORIZATION, header_value);
        }

        let client = reqwest::Client::builder()
            .user_agent(concat!(
                "uptrakit-provider-github/",
                env!("CARGO_PKG_VERSION")
            ))
            .default_headers(headers)
            .build()
            .map_err(|e| {
                report!(GitHubError::Request(format!(
                    "failed to build HTTP client: {e}"
                )))
            })?;

        let asset_filters: Vec<Regex> = config
            .asset_patterns
            .iter()
            .map(|p| {
                Regex::new(p).map_err(|e| {
                    report!(GitHubError::InvalidPattern(format!(
                        "invalid regex '{p}': {e}"
                    )))
                })
            })
            .collect::<Result<_>>()?;

        Ok(Self {
            client,
            config,
            asset_filters,
            executor,
        })
    }

    /// Build the releases API URL.
    fn releases_url(&self) -> String {
        format!(
            "{}/repos/{}/{}/releases?per_page=100",
            self.config.api_base_url(),
            self.config.owner,
            self.config.repo
        )
    }

    /// Convert a GitHub API release to an `UpstreamRelease`, applying filters.
    ///
    /// Returns `None` if the release should be skipped (draft, filtered prerelease).
    fn convert_release(&self, gh_release: &GitHubRelease) -> Option<UpstreamRelease> {
        // Skip drafts
        if gh_release.draft {
            return None;
        }

        // Skip prereleases unless configured to include them
        if gh_release.prerelease && !self.config.include_prereleases {
            return None;
        }

        let version_str = strip_tag_prefix(&gh_release.tag_name, &self.config.tag_strip_prefix);
        let version = Version::new(version_str);

        let published_at = gh_release.published_at.as_ref().and_then(|s| {
            OffsetDateTime::parse(s, &Rfc3339)
                .inspect_err(|e| {
                    tracing::warn!(
                        tag = %gh_release.tag_name,
                        error = %e,
                        "failed to parse published_at date"
                    );
                })
                .ok()
        });

        let assets: Vec<ReleaseAsset> = gh_release
            .assets
            .iter()
            .filter(|a| {
                if self.asset_filters.is_empty() {
                    return true;
                }
                self.asset_filters.iter().any(|re| re.is_match(&a.name))
            })
            .map(|a| ReleaseAsset {
                name: a.name.clone(),
                download_url: a.browser_download_url.clone(),
                size: Some(a.size),
                content_type: a.content_type.clone(),
            })
            .collect();

        Some(UpstreamRelease {
            version,
            tag: gh_release.tag_name.clone(),
            is_prerelease: gh_release.prerelease,
            release_url: gh_release.html_url.clone(),
            release_notes: gh_release.body.clone(),
            published_at,
            assets,
        })
    }

    /// Check rate limit headers and log warnings.
    fn check_rate_limit(&self, headers: &reqwest::header::HeaderMap) {
        let remaining = headers
            .get("x-ratelimit-remaining")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok());

        if let Some(remaining) = remaining
            && remaining < 10
        {
            let reset = headers
                .get("x-ratelimit-reset")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("unknown");
            tracing::warn!(
                remaining,
                reset,
                owner = %self.config.owner,
                repo = %self.config.repo,
                "GitHub API rate limit is low"
            );
        }
    }
}

#[async_trait]
impl Provider for GitHubProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::GithubReleases
    }

    async fn fetch_releases(
        &self,
        _package_identifier: &str,
    ) -> uptrakit_provider_core::Result<Vec<UpstreamRelease>> {
        let url = self.releases_url();
        tracing::debug!(url = %url, "fetching GitHub releases");

        let response = self.client.get(&url).send().await.map_err(|e| {
            report!(ProviderError::Configuration(format!(
                "HTTP request failed: {e}"
            )))
        })?;

        let status = response.status();
        self.check_rate_limit(response.headers());

        if !status.is_success() {
            let status_code = status.as_u16();

            // Check for rate limiting
            if status_code == 403 || status_code == 429 {
                let remaining = response
                    .headers()
                    .get("x-ratelimit-remaining")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok());

                if remaining == Some(0) {
                    let reset_at = response
                        .headers()
                        .get("x-ratelimit-reset")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("unknown")
                        .to_string();
                    bail!(ProviderError::Configuration(format!(
                        "GitHub API rate limit exceeded (resets at {reset_at})"
                    )));
                }
            }

            let body = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<GitHubApiError>(&body)
                .map(|e| e.message)
                .unwrap_or(body);

            bail!(ProviderError::Configuration(format!(
                "GitHub API error: {status_code} {message}"
            )));
        }

        let releases: Vec<GitHubRelease> = response.json().await.map_err(|e| {
            report!(ProviderError::Serialization(format!(
                "failed to parse GitHub API response: {e}"
            )))
        })?;

        let upstream_releases: Vec<UpstreamRelease> = releases
            .iter()
            .filter_map(|r| self.convert_release(r))
            .collect();

        tracing::debug!(
            count = upstream_releases.len(),
            total = releases.len(),
            "fetched GitHub releases"
        );

        Ok(upstream_releases)
    }

    async fn detect_installed_version(
        &self,
        _package_identifier: &str,
    ) -> uptrakit_provider_core::Result<Option<Version>> {
        Ok(None)
    }

    async fn execute_update(
        &self,
        package_identifier: &str,
        to_version: &str,
        release_info: Option<&ReleaseInfo>,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> uptrakit_provider_core::Result<String> {
        let mut output = String::new();

        let release_info =
            release_info.ok_or_else(|| report!(ProviderError::MissingReleaseInfo))?;

        send_output(
            output_tx,
            &format!(
                "Downloading release {} from {}",
                release_info.tag, release_info.release_url
            ),
            UpdateOutputStream::Stdout,
        )
        .await;
        output.push_str(&format!(
            "Downloading release {} from {}\n",
            release_info.tag, release_info.release_url
        ));

        if let Some(ref cmd_str) = self.config.install_command {
            let cmd = cmd_str
                .replace("{version}", &shell_escape(to_version))
                .replace("{tag}", &shell_escape(&release_info.tag))
                .replace("{package_identifier}", &shell_escape(package_identifier));

            send_output(
                output_tx,
                &format!("Running install command: {cmd}"),
                UpdateOutputStream::Stdout,
            )
            .await;

            match self
                .executor
                .execute(&CommandSpec::shell(&cmd), output_tx)
                .await
            {
                Ok(cmd_output) => {
                    output.push_str(&cmd_output.output);
                }
                Err(e) => {
                    bail!(ProviderError::InstallFailed(e.to_string()));
                }
            }
        } else {
            send_output(
                output_tx,
                "No install_command configured, skipping automated installation",
                UpdateOutputStream::Stdout,
            )
            .await;
            output.push_str("No install_command configured, skipping automated installation\n");
        }

        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_types::{GitHubAsset, GitHubRelease};
    use tokio::sync::mpsc;
    use uptrakit_provider_core::LocalCommandExecutor;

    fn test_config() -> GitHubConfig {
        GitHubConfig {
            owner: "octocat".to_string(),
            repo: "hello-world".to_string(),
            auth_token: None,
            api_base_url: None,
            include_prereleases: false,
            tag_strip_prefix: "v".to_string(),
            asset_patterns: vec![],
            install_command: None,
        }
    }

    fn test_executor() -> Arc<dyn CommandExecutor> {
        Arc::new(LocalCommandExecutor)
    }

    fn test_provider() -> GitHubProvider {
        GitHubProvider::new(test_config(), test_executor()).expect("valid config")
    }

    fn make_release(tag: &str, draft: bool, prerelease: bool) -> GitHubRelease {
        GitHubRelease {
            tag_name: tag.to_string(),
            name: Some(format!("Release {tag}")),
            draft,
            prerelease,
            html_url: format!("https://github.com/octocat/hello-world/releases/tag/{tag}"),
            body: Some("Release notes".to_string()),
            published_at: Some("2024-01-28T12:00:00Z".to_string()),
            assets: vec![],
        }
    }

    #[test]
    fn convert_normal_release() {
        let provider = test_provider();
        let gh = make_release("v1.0.0", false, false);
        let release = provider.convert_release(&gh).expect("should convert");
        assert_eq!(release.version.as_str(), "1.0.0");
        assert_eq!(release.tag, "v1.0.0");
        assert!(!release.is_prerelease);
        assert!(release.published_at.is_some());
    }

    #[test]
    fn skip_draft_release() {
        let provider = test_provider();
        let gh = make_release("v1.0.0", true, false);
        assert!(provider.convert_release(&gh).is_none());
    }

    #[test]
    fn skip_prerelease_by_default() {
        let provider = test_provider();
        let gh = make_release("v1.0.0-beta.1", false, true);
        assert!(provider.convert_release(&gh).is_none());
    }

    #[test]
    fn include_prerelease_when_configured() {
        let mut config = test_config();
        config.include_prereleases = true;
        let provider = GitHubProvider::new(config, test_executor()).expect("valid config");
        let gh = make_release("v1.0.0-beta.1", false, true);
        let release = provider.convert_release(&gh).expect("should convert");
        assert!(release.is_prerelease);
        assert_eq!(release.version.as_str(), "1.0.0-beta.1");
    }

    #[test]
    fn tag_stripping() {
        let provider = test_provider();
        let gh = make_release("v2.3.4", false, false);
        let release = provider.convert_release(&gh).expect("should convert");
        assert_eq!(release.version.as_str(), "2.3.4");
    }

    #[test]
    fn tag_without_prefix() {
        let provider = test_provider();
        let gh = make_release("1.0.0", false, false);
        let release = provider.convert_release(&gh).expect("should convert");
        assert_eq!(release.version.as_str(), "1.0.0");
    }

    #[test]
    fn custom_tag_prefix() {
        let mut config = test_config();
        config.tag_strip_prefix = "release-".to_string();
        let provider = GitHubProvider::new(config, test_executor()).expect("valid config");
        let gh = make_release("release-3.0.0", false, false);
        let release = provider.convert_release(&gh).expect("should convert");
        assert_eq!(release.version.as_str(), "3.0.0");
    }

    #[test]
    fn asset_filtering() {
        let mut config = test_config();
        config.asset_patterns = vec![r".*\.tar\.gz$".to_string()];
        let provider = GitHubProvider::new(config, test_executor()).expect("valid config");

        let gh = GitHubRelease {
            tag_name: "v1.0.0".to_string(),
            name: None,
            draft: false,
            prerelease: false,
            html_url: "https://example.com".to_string(),
            body: None,
            published_at: None,
            assets: vec![
                GitHubAsset {
                    name: "app-linux-amd64.tar.gz".to_string(),
                    browser_download_url: "https://example.com/app.tar.gz".to_string(),
                    size: 1000,
                    content_type: Some("application/gzip".to_string()),
                },
                GitHubAsset {
                    name: "app-linux-amd64.deb".to_string(),
                    browser_download_url: "https://example.com/app.deb".to_string(),
                    size: 2000,
                    content_type: Some("application/vnd.debian.binary-package".to_string()),
                },
                GitHubAsset {
                    name: "checksums.txt".to_string(),
                    browser_download_url: "https://example.com/checksums.txt".to_string(),
                    size: 256,
                    content_type: Some("text/plain".to_string()),
                },
            ],
        };

        let release = provider.convert_release(&gh).expect("should convert");
        assert_eq!(release.assets.len(), 1);
        assert_eq!(release.assets[0].name, "app-linux-amd64.tar.gz");
    }

    #[test]
    fn no_asset_filter_includes_all() {
        let provider = test_provider();
        let gh = GitHubRelease {
            tag_name: "v1.0.0".to_string(),
            name: None,
            draft: false,
            prerelease: false,
            html_url: "https://example.com".to_string(),
            body: None,
            published_at: None,
            assets: vec![
                GitHubAsset {
                    name: "a.tar.gz".to_string(),
                    browser_download_url: "https://example.com/a".to_string(),
                    size: 100,
                    content_type: None,
                },
                GitHubAsset {
                    name: "b.deb".to_string(),
                    browser_download_url: "https://example.com/b".to_string(),
                    size: 200,
                    content_type: None,
                },
            ],
        };

        let release = provider.convert_release(&gh).expect("should convert");
        assert_eq!(release.assets.len(), 2);
    }

    #[test]
    fn url_construction() {
        let provider = test_provider();
        let url = provider.releases_url();
        assert_eq!(
            url,
            "https://api.github.com/repos/octocat/hello-world/releases?per_page=100"
        );
    }

    #[test]
    fn url_construction_custom_base() {
        let mut config = test_config();
        config.api_base_url = Some("https://ghe.corp.com/api/v3".to_string());
        let provider = GitHubProvider::new(config, test_executor()).expect("valid config");
        let url = provider.releases_url();
        assert_eq!(
            url,
            "https://ghe.corp.com/api/v3/repos/octocat/hello-world/releases?per_page=100"
        );
    }

    #[test]
    fn date_parsing() {
        let provider = test_provider();
        let gh = make_release("v1.0.0", false, false);
        let release = provider.convert_release(&gh).expect("should convert");
        let published = release.published_at.expect("should have published_at");
        assert_eq!(published.year(), 2024);
        assert_eq!(published.month() as u8, 1);
        assert_eq!(published.day(), 28);
    }

    #[test]
    fn invalid_date_does_not_fail() {
        let provider = test_provider();
        let gh = GitHubRelease {
            tag_name: "v1.0.0".to_string(),
            name: None,
            draft: false,
            prerelease: false,
            html_url: "https://example.com".to_string(),
            body: None,
            published_at: Some("not-a-date".to_string()),
            assets: vec![],
        };
        let release = provider.convert_release(&gh).expect("should convert");
        assert!(release.published_at.is_none());
    }

    #[test]
    fn provider_creation_fails_with_invalid_config() {
        let config = test_config();
        let config = GitHubConfig {
            owner: String::new(),
            repo: "test".to_string(),
            auth_token: config.auth_token,
            api_base_url: config.api_base_url,
            include_prereleases: config.include_prereleases,
            tag_strip_prefix: config.tag_strip_prefix,
            asset_patterns: config.asset_patterns,
            install_command: config.install_command,
        };
        assert!(GitHubProvider::new(config, test_executor()).is_err());
    }

    #[tokio::test]
    async fn detect_installed_version_returns_none() {
        let provider = test_provider();
        let result = provider.detect_installed_version("example").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn execute_update_missing_release_info_returns_error() {
        let provider = test_provider();
        let (tx, _rx) = mpsc::channel(100);
        let result = provider
            .execute_update("octocat/hello-world", "1.0.0", None, &tx)
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err.current_context(), ProviderError::MissingReleaseInfo),
            "Expected MissingReleaseInfo, got: {err}"
        );
    }

    #[tokio::test]
    async fn execute_update_no_install_command_succeeds() {
        let provider = test_provider();
        let (tx, mut rx) = mpsc::channel(100);
        let release_info = ReleaseInfo {
            tag: "v1.0.0".to_string(),
            release_url: "https://example.com".to_string(),
            assets: vec![],
        };
        let result = provider
            .execute_update("octocat/hello-world", "1.0.0", Some(&release_info), &tx)
            .await;
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.contains("No install_command configured"));
        rx.close();
        while rx.recv().await.is_some() {}
    }
}
