#[derive(Clone)]
pub struct GitHubClientConfig {
    pub http_client: reqwest::Client,
    pub base_url: url::Url,
    pub auth: GitHubAuth,
    pub user_agent: String,
}

impl GitHubClientConfig {
    pub fn new(
        http_client: reqwest::Client,
        base_url: url::Url,
        auth: GitHubAuth,
        user_agent: impl Into<String>,
    ) -> Self {
        Self {
            http_client,
            base_url,
            auth,
            user_agent: user_agent.into(),
        }
    }
}

#[derive(Clone)]
#[non_exhaustive]
pub enum GitHubAuth {
    Anonymous,
    BearerToken(uptrakit_wire::SecretString),
}

#[non_exhaustive]
pub enum GitHubEndpoint {
    RepositoryTree {
        owner: String,
        repo: String,
        git_ref: String,
        recursive: bool,
    },
}

impl GitHubEndpoint {
    #[expect(
        clippy::map_err_ignore,
        reason = "url::PathSegmentsMutError is a unit struct that carries no information"
    )]
    pub fn build_request(
        &self,
        config: &GitHubClientConfig,
    ) -> Result<reqwest::Request, GitHubClientError> {
        let mut url = config.base_url.clone();
        match self {
            Self::RepositoryTree {
                owner,
                repo,
                git_ref,
                recursive,
            } => {
                url.path_segments_mut()
                    .map_err(|_| {
                        GitHubClientError::Misconfigured(
                            "base_url cannot be used as a path base".to_string(),
                        )
                    })?
                    .push("repos")
                    .push(owner)
                    .push(repo)
                    .push("git")
                    .push("trees")
                    .push(git_ref);
                url.query_pairs_mut()
                    .append_pair("recursive", if *recursive { "1" } else { "0" });
            }
        }

        let mut request = config
            .http_client
            .get(url)
            .header(http::header::USER_AGENT, &config.user_agent)
            .header(http::header::ACCEPT, "application/vnd.github+json")
            .header("x-github-api-version", "2022-11-28");

        if let GitHubAuth::BearerToken(token) = &config.auth {
            request = request.bearer_auth(token.expose_secret());
        }

        request
            .build()
            .map_err(|error| GitHubClientError::Misconfigured(error.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryTreeResponse {
    pub truncated: bool,
    pub entries: Vec<RepositoryTreeEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryTreeEntry {
    pub path: String,
    pub kind: RepositoryTreeEntryKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RepositoryTreeEntryKind {
    Blob,
    Tree,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum GitHubClientError {
    #[error("GitHub authentication failed: {0}")]
    AuthFailed(String),
    #[error("GitHub request forbidden: {0}")]
    Forbidden(String),
    #[error("GitHub resource not found: {0}")]
    NotFound(String),
    #[error("GitHub rate limit reached: {0}")]
    RateLimited(String),
    #[error("GitHub upstream unavailable: {0}")]
    UpstreamUnavailable(String),
    #[error("invalid GitHub response: {0}")]
    InvalidResponse(String),
    #[error("misconfigured GitHub client: {0}")]
    Misconfigured(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RetryDecision {
    DoNotRetry,
    RetryAfter(std::time::Duration),
    Backoff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthKind {
    Anonymous,
    Bearer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseMetadata {
    pub status: Option<http::StatusCode>,
    pub auth_kind: AuthKind,
    pub authenticated_not_found: bool,
    pub rate_limit_remaining: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AttemptOutcome<T> {
    Success(T, ResponseMetadata),
    Failure(GitHubClientError, RetryDecision, ResponseMetadata),
}

pub fn classify_http_failure_with_auth(
    status: http::StatusCode,
    headers: &http::HeaderMap,
    body: &str,
    auth_kind: AuthKind,
) -> Result<(GitHubClientError, RetryDecision, ResponseMetadata), GitHubClientError> {
    let message = extract_message(status, body);
    let meta = ResponseMetadata {
        status: Some(status),
        auth_kind,
        authenticated_not_found: status == http::StatusCode::NOT_FOUND
            && auth_kind == AuthKind::Bearer,
        rate_limit_remaining: headers
            .get("x-ratelimit-remaining")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse().ok()),
    };

    let result = match status {
        http::StatusCode::UNAUTHORIZED => (
            GitHubClientError::AuthFailed(message),
            RetryDecision::DoNotRetry,
            meta,
        ),
        http::StatusCode::FORBIDDEN if looks_rate_limited(headers, body) => (
            GitHubClientError::RateLimited(message),
            retry_after_or_backoff(headers),
            meta,
        ),
        http::StatusCode::FORBIDDEN => (
            GitHubClientError::Forbidden(message),
            RetryDecision::DoNotRetry,
            meta,
        ),
        http::StatusCode::NOT_FOUND => (
            GitHubClientError::NotFound(message),
            RetryDecision::DoNotRetry,
            meta,
        ),
        http::StatusCode::TOO_MANY_REQUESTS => (
            GitHubClientError::RateLimited(message),
            retry_after_or_backoff(headers),
            meta,
        ),
        status if status.is_server_error() && headers.contains_key("retry-after") => (
            GitHubClientError::UpstreamUnavailable(message),
            retry_after_or_backoff(headers),
            meta,
        ),
        status if status.is_server_error() => (
            GitHubClientError::UpstreamUnavailable(message),
            RetryDecision::Backoff,
            meta,
        ),
        _ => (
            GitHubClientError::InvalidResponse(message),
            RetryDecision::DoNotRetry,
            meta,
        ),
    };

    Ok(result)
}

pub fn classify_http_failure(
    status: http::StatusCode,
    headers: &http::HeaderMap,
    body: &str,
) -> Result<(GitHubClientError, RetryDecision, ResponseMetadata), GitHubClientError> {
    classify_http_failure_with_auth(status, headers, body, AuthKind::Anonymous)
}

fn looks_rate_limited(headers: &http::HeaderMap, body: &str) -> bool {
    headers.contains_key("retry-after")
        || headers
            .get("x-ratelimit-remaining")
            .and_then(|value| value.to_str().ok())
            == Some("0")
        || body.to_ascii_lowercase().contains("rate limit")
}

fn extract_message(status: http::StatusCode, body: &str) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body)
        && let Some(message) = value.get("message").and_then(|value| value.as_str())
    {
        return format!("{status}: {message}");
    }

    format!("{status}: {body}")
}

fn retry_after_or_backoff(headers: &http::HeaderMap) -> RetryDecision {
    if let Some(retry_after) = headers
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
    {
        return RetryDecision::RetryAfter(std::time::Duration::from_secs(retry_after));
    }

    if let Some(reset_at) = headers
        .get("x-ratelimit-reset")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<i64>().ok())
    {
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        if reset_at > now {
            return RetryDecision::RetryAfter(std::time::Duration::from_secs(
                (reset_at - now) as u64,
            ));
        }
    }

    RetryDecision::Backoff
}

pub struct GitHubClient {
    config: GitHubClientConfig,
}

impl GitHubClient {
    pub fn new(config: GitHubClientConfig) -> Self {
        Self { config }
    }

    pub async fn fetch_repository_tree(
        &self,
        owner: &str,
        repo: &str,
        git_ref: &str,
        recursive: bool,
    ) -> Result<AttemptOutcome<RepositoryTreeResponse>, GitHubClientError> {
        let request = GitHubEndpoint::RepositoryTree {
            owner: owner.to_owned(),
            repo: repo.to_owned(),
            git_ref: git_ref.to_owned(),
            recursive,
        }
        .build_request(&self.config)?;

        let auth_kind = match &self.config.auth {
            GitHubAuth::Anonymous => AuthKind::Anonymous,
            GitHubAuth::BearerToken(_) => AuthKind::Bearer,
        };

        let response = match self.config.http_client.execute(request).await {
            Ok(response) => response,
            Err(error) => {
                return Ok(AttemptOutcome::Failure(
                    GitHubClientError::UpstreamUnavailable(error.to_string()),
                    RetryDecision::Backoff,
                    ResponseMetadata {
                        status: None,
                        auth_kind,
                        authenticated_not_found: false,
                        rate_limit_remaining: None,
                    },
                ));
            }
        };
        let status = response.status();
        let headers = response.headers().clone();
        let body = match response.text().await {
            Ok(body) => body,
            Err(error) => {
                let (classified_error, decision, metadata) =
                    classify_body_read_failure(status, &headers, &error.to_string(), auth_kind)?;
                return Ok(AttemptOutcome::Failure(
                    classified_error,
                    decision,
                    metadata,
                ));
            }
        };

        if status.is_success() {
            let metadata = ResponseMetadata {
                status: Some(status),
                auth_kind,
                authenticated_not_found: false,
                rate_limit_remaining: headers
                    .get("x-ratelimit-remaining")
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse().ok()),
            };
            let parsed: RepositoryTreeDto = match serde_json::from_str(&body) {
                Ok(parsed) => parsed,
                Err(error) => {
                    return Ok(AttemptOutcome::Failure(
                        GitHubClientError::InvalidResponse(error.to_string()),
                        RetryDecision::DoNotRetry,
                        metadata,
                    ));
                }
            };
            let tree = match parsed.into_model() {
                Ok(tree) => tree,
                Err(error) => {
                    return Ok(AttemptOutcome::Failure(
                        error,
                        RetryDecision::DoNotRetry,
                        metadata,
                    ));
                }
            };
            return Ok(AttemptOutcome::Success(tree, metadata));
        }

        let (error, decision, metadata) =
            classify_http_failure_with_auth(status, &headers, &body, auth_kind)?;
        Ok(AttemptOutcome::Failure(error, decision, metadata))
    }
}

fn classify_body_read_failure(
    status: http::StatusCode,
    headers: &http::HeaderMap,
    error_message: &str,
    auth_kind: AuthKind,
) -> Result<(GitHubClientError, RetryDecision, ResponseMetadata), GitHubClientError> {
    if status.is_success() {
        return Ok((
            GitHubClientError::UpstreamUnavailable(error_message.to_string()),
            RetryDecision::Backoff,
            ResponseMetadata {
                status: Some(status),
                auth_kind,
                authenticated_not_found: false,
                rate_limit_remaining: headers
                    .get("x-ratelimit-remaining")
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse().ok()),
            },
        ));
    }

    classify_http_failure_with_auth(status, headers, error_message, auth_kind)
}

#[derive(Debug, serde::Deserialize)]
struct RepositoryTreeDto {
    truncated: bool,
    tree: Vec<RepositoryTreeEntryDto>,
}

#[derive(Debug, serde::Deserialize)]
struct RepositoryTreeEntryDto {
    path: String,
    #[serde(rename = "type")]
    kind: String,
}

impl RepositoryTreeDto {
    fn into_model(self) -> Result<RepositoryTreeResponse, GitHubClientError> {
        let entries = self
            .tree
            .into_iter()
            .map(|entry| {
                let kind = match entry.kind.as_str() {
                    "blob" => RepositoryTreeEntryKind::Blob,
                    "tree" => RepositoryTreeEntryKind::Tree,
                    other => {
                        return Err(GitHubClientError::InvalidResponse(format!(
                            "unsupported tree entry kind: {other}"
                        )));
                    }
                };

                Ok(RepositoryTreeEntry {
                    path: entry.path,
                    kind,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(RepositoryTreeResponse {
            truncated: self.truncated,
            entries,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_tree_endpoint_builds_expected_url() {
        let config = GitHubClientConfig::new(
            reqwest::Client::new(),
            url::Url::parse("https://api.github.com").unwrap(),
            GitHubAuth::Anonymous,
            "uptrakit-test",
        );
        let request = GitHubEndpoint::RepositoryTree {
            owner: "homarr-labs".into(),
            repo: "dashboard-icons".into(),
            git_ref: "main".into(),
            recursive: true,
        }
        .build_request(&config)
        .unwrap();

        assert_eq!(
            request.url().as_str(),
            "https://api.github.com/repos/homarr-labs/dashboard-icons/git/trees/main?recursive=1"
        );
    }

    #[test]
    fn repository_tree_endpoint_preserves_api_base_path() {
        let config = GitHubClientConfig::new(
            reqwest::Client::new(),
            url::Url::parse("https://ghe.example.com/api/v3").unwrap(),
            GitHubAuth::Anonymous,
            "uptrakit-test",
        );
        let request = GitHubEndpoint::RepositoryTree {
            owner: "homarr-labs".into(),
            repo: "dashboard-icons".into(),
            git_ref: "main".into(),
            recursive: true,
        }
        .build_request(&config)
        .unwrap();

        assert_eq!(
            request.url().as_str(),
            "https://ghe.example.com/api/v3/repos/homarr-labs/dashboard-icons/git/trees/main?recursive=1"
        );
    }

    #[test]
    fn repository_tree_endpoint_percent_encodes_git_ref_segment() {
        let config = GitHubClientConfig::new(
            reqwest::Client::new(),
            url::Url::parse("https://api.github.com").unwrap(),
            GitHubAuth::Anonymous,
            "uptrakit-test",
        );
        let request = GitHubEndpoint::RepositoryTree {
            owner: "homarr-labs".into(),
            repo: "dashboard-icons".into(),
            git_ref: "feature/foo".into(),
            recursive: true,
        }
        .build_request(&config)
        .unwrap();

        assert_eq!(
            request.url().as_str(),
            "https://api.github.com/repos/homarr-labs/dashboard-icons/git/trees/feature%2Ffoo?recursive=1"
        );
    }

    #[test]
    fn anonymous_auth_does_not_emit_authorization_header() {
        let config = GitHubClientConfig::new(
            reqwest::Client::new(),
            url::Url::parse("https://api.github.com").unwrap(),
            GitHubAuth::Anonymous,
            "uptrakit-test",
        );
        let request = GitHubEndpoint::RepositoryTree {
            owner: "homarr-labs".into(),
            repo: "dashboard-icons".into(),
            git_ref: "main".into(),
            recursive: true,
        }
        .build_request(&config)
        .unwrap();

        assert!(request.headers().get(http::header::AUTHORIZATION).is_none());
        assert_eq!(
            request.headers().get(http::header::ACCEPT).unwrap(),
            "application/vnd.github+json"
        );
        assert_eq!(
            request.headers().get("x-github-api-version").unwrap(),
            "2022-11-28"
        );
        assert_eq!(
            request.headers().get(http::header::USER_AGENT).unwrap(),
            "uptrakit-test"
        );
    }

    #[test]
    fn bearer_auth_emits_required_headers() {
        let config = GitHubClientConfig::new(
            reqwest::Client::new(),
            url::Url::parse("https://api.github.com").unwrap(),
            GitHubAuth::BearerToken(uptrakit_wire::SecretString::new("ghp_test")),
            "uptrakit-test",
        );
        let request = GitHubEndpoint::RepositoryTree {
            owner: "homarr-labs".into(),
            repo: "dashboard-icons".into(),
            git_ref: "main".into(),
            recursive: false,
        }
        .build_request(&config)
        .unwrap();

        assert_eq!(
            request.headers().get(http::header::AUTHORIZATION).unwrap(),
            "Bearer ghp_test"
        );
        assert_eq!(
            request.headers().get(http::header::USER_AGENT).unwrap(),
            "uptrakit-test"
        );
        assert_eq!(
            request.headers().get(http::header::ACCEPT).unwrap(),
            "application/vnd.github+json"
        );
        assert_eq!(
            request.headers().get("x-github-api-version").unwrap(),
            "2022-11-28"
        );
    }

    #[test]
    fn classify_503_with_retry_after_as_upstream_unavailable_retry_after() {
        let headers = http::HeaderMap::from_iter([(
            http::HeaderName::from_static("retry-after"),
            http::HeaderValue::from_static("60"),
        )]);

        let (error, decision, _meta) =
            classify_http_failure(http::StatusCode::SERVICE_UNAVAILABLE, &headers, "{}").unwrap();

        assert!(matches!(error, GitHubClientError::UpstreamUnavailable(_)));
        assert_eq!(
            decision,
            RetryDecision::RetryAfter(std::time::Duration::from_secs(60))
        );
    }

    #[test]
    fn classify_403_without_rate_limit_evidence_as_forbidden() {
        let headers = http::HeaderMap::new();
        let (error, decision, _meta) = classify_http_failure(
            http::StatusCode::FORBIDDEN,
            &headers,
            "{\"message\":\"forbidden\"}",
        )
        .unwrap();

        assert!(matches!(error, GitHubClientError::Forbidden(_)));
        assert_eq!(decision, RetryDecision::DoNotRetry);
    }

    #[test]
    fn classify_403_with_rate_limit_evidence_as_rate_limited() {
        let headers = http::HeaderMap::from_iter([(
            http::HeaderName::from_static("x-ratelimit-remaining"),
            http::HeaderValue::from_static("0"),
        )]);
        let (error, decision, _meta) = classify_http_failure(
            http::StatusCode::FORBIDDEN,
            &headers,
            "{\"message\":\"secondary rate limit\"}",
        )
        .unwrap();

        assert!(matches!(error, GitHubClientError::RateLimited(_)));
        assert!(matches!(
            decision,
            RetryDecision::RetryAfter(_) | RetryDecision::Backoff
        ));
    }

    #[test]
    fn classify_404_records_authenticated_context() {
        let headers = http::HeaderMap::new();
        let (_, _, meta) = classify_http_failure_with_auth(
            http::StatusCode::NOT_FOUND,
            &headers,
            "{\"message\":\"Not Found\"}",
            AuthKind::Bearer,
        )
        .unwrap();

        assert!(meta.authenticated_not_found);
    }

    #[test]
    fn classify_body_read_failure_on_success_is_retryable() {
        let headers = http::HeaderMap::new();
        let (error, decision, meta) = classify_body_read_failure(
            http::StatusCode::OK,
            &headers,
            "stream closed",
            AuthKind::Anonymous,
        )
        .unwrap();

        assert!(matches!(error, GitHubClientError::UpstreamUnavailable(_)));
        assert_eq!(decision, RetryDecision::Backoff);
        assert_eq!(meta.status, Some(http::StatusCode::OK));
    }

    #[tokio::test]
    async fn fetch_repository_tree_decodes_blob_and_tree_entries() {
        use httpmock::Method::GET;
        use httpmock::MockServer;

        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET)
                .path("/repos/homarr-labs/dashboard-icons/git/trees/main")
                .query_param("recursive", "1");
            then.status(200).json_body(serde_json::json!({
                "truncated": false,
                "tree": [
                    { "path": "svg/nginx.svg", "type": "blob" },
                    { "path": "svg", "type": "tree" }
                ]
            }));
        });

        let client = GitHubClient::new(GitHubClientConfig::new(
            reqwest::Client::new(),
            url::Url::parse(&server.base_url()).unwrap(),
            GitHubAuth::Anonymous,
            "uptrakit-test",
        ));

        let outcome = client
            .fetch_repository_tree("homarr-labs", "dashboard-icons", "main", true)
            .await
            .unwrap();
        let AttemptOutcome::Success(tree, _) = outcome else {
            panic!("expected success");
        };
        assert_eq!(tree.entries.len(), 2);
    }

    #[tokio::test]
    async fn fetch_repository_tree_wraps_transport_failures_as_retryable_outcomes() {
        let client = GitHubClient::new(GitHubClientConfig::new(
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_millis(25))
                .build()
                .unwrap(),
            url::Url::parse("http://127.0.0.1:9").unwrap(),
            GitHubAuth::Anonymous,
            "uptrakit-test",
        ));

        let outcome = client
            .fetch_repository_tree("homarr-labs", "dashboard-icons", "main", true)
            .await
            .unwrap();

        let AttemptOutcome::Failure(error, decision, meta) = outcome else {
            panic!("expected failure outcome");
        };
        assert!(matches!(error, GitHubClientError::UpstreamUnavailable(_)));
        assert_eq!(decision, RetryDecision::Backoff);
        assert_eq!(meta.status, None);
        assert_eq!(meta.auth_kind, AuthKind::Anonymous);
    }

    #[tokio::test]
    async fn fetch_repository_tree_wraps_invalid_success_payloads_as_do_not_retry_failures() {
        use httpmock::Method::GET;
        use httpmock::MockServer;

        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET)
                .path("/repos/homarr-labs/dashboard-icons/git/trees/main")
                .query_param("recursive", "1");
            then.status(200).json_body(serde_json::json!({
                "truncated": false,
                "tree": [
                    { "path": "svg/nginx.svg", "type": "symlink" }
                ]
            }));
        });

        let client = GitHubClient::new(GitHubClientConfig::new(
            reqwest::Client::new(),
            url::Url::parse(&server.base_url()).unwrap(),
            GitHubAuth::Anonymous,
            "uptrakit-test",
        ));

        let outcome = client
            .fetch_repository_tree("homarr-labs", "dashboard-icons", "main", true)
            .await
            .unwrap();

        let AttemptOutcome::Failure(error, decision, meta) = outcome else {
            panic!("expected failure outcome");
        };
        assert!(matches!(error, GitHubClientError::InvalidResponse(_)));
        assert_eq!(decision, RetryDecision::DoNotRetry);
        assert_eq!(meta.status, Some(http::StatusCode::OK));
    }
}
