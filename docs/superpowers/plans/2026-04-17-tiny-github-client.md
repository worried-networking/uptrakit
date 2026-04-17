# Tiny GitHub Client Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `octocrab` in the global GitHub provider runtime with a small
shared `reqwest`-based GitHub client crate while preserving current
global-provider behavior for `dashboard-icons`.

**Architecture:** Add a new shared crate at
`crates/shared/github-client/` that owns endpoint building, neutral GitHub REST
models, one-attempt execution, response classification, and retry
recommendations. Keep `uptrakit-web-api` responsible for validated transport
construction, shared cooldown state, retry execution, and mapping neutral
client outcomes into the stable `uptrakit-global-github-provider` contract.

**Tech Stack:** Rust, Tokio, reqwest, serde, http, SeaORM, httpmock,
markdownlint

---

## File Structure

### New files

- Create: `crates/shared/github-client/Cargo.toml`
- Create: `crates/shared/github-client/src/lib.rs`

### Modified files

- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `crates/ui/web-api/Cargo.toml`
- Modify: `crates/ui/web-api/src/global_providers/github.rs`
- Modify: `crates/ui/web-api/src/global_providers/tests.rs`
- Modify: `docs/superpowers/specs/2026-04-17-tiny-github-client-design.md`

### Test files

- Test: `crates/shared/github-client/src/lib.rs`
- Test: `crates/ui/web-api/src/global_providers/tests.rs`

---

### Task 1: Add The Shared GitHub Client Crate Skeleton

**Files:**

- Create: `crates/shared/github-client/Cargo.toml`
- Create: `crates/shared/github-client/src/lib.rs`
- Modify: `Cargo.toml`
- Test: `crates/shared/github-client/src/lib.rs`

- [ ] **Step 1: Write the failing crate-level tests**

Add tests in `crates/shared/github-client/src/lib.rs` covering the initial public contract:

```rust
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
}

#[test]
fn bearer_auth_emits_required_headers() {
    let config = GitHubClientConfig::new(
        reqwest::Client::new(),
        url::Url::parse("https://api.github.com").unwrap(),
        GitHubAuth::BearerToken(uptrakit_internal_wire::SecretString::new("ghp_test")),
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
}
```

- [ ] **Step 2: Run the targeted test to verify it fails**

Run:

```bash
cargo test -p uptrakit-github-client anonymous_auth_does_not_emit_authorization_header -- --nocapture
```

Expected: FAIL because the new crate and symbols do not exist yet.

- [ ] **Step 3: Create the crate manifest and workspace entry**

Add the new crate to the workspace root and create the initial crate manifest:

```toml
# Cargo.toml
[workspace.dependencies]
uptrakit-github-client = { path = "crates/shared/github-client" }
```

```toml
# crates/shared/github-client/Cargo.toml
[package]
name = "uptrakit-github-client"
version = "0.0.1"
edition = "2024"
publish = false

[dependencies]
http = { workspace = true }
reqwest = { workspace = true }
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
time = { workspace = true }
thiserror = { workspace = true }
url = { workspace = true }
uptrakit-internal-wire = { workspace = true }

[dev-dependencies]
httpmock = { workspace = true }
tokio = { workspace = true, features = ["macros", "rt-multi-thread"] }
```

- [ ] **Step 4: Add the minimal contract and request builder**

Create `crates/shared/github-client/src/lib.rs` with the initial config/auth/endpoint types:

```rust
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
    BearerToken(uptrakit_internal_wire::SecretString),
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
                url.set_path(&format!("/repos/{owner}/{repo}/git/trees/{git_ref}"));
                url.query_pairs_mut()
                    .append_pair("recursive", if *recursive { "1" } else { "0" });
            }
        }

        let mut request = config.http_client.get(url)
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
#[non_exhaustive]
pub enum GitHubClientError {
    Misconfigured(String),
}
```

- [ ] **Step 5: Run the new crate tests to verify they pass**

Run:

```bash
cargo test -p uptrakit-github-client -- --nocapture
```

Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/shared/github-client/Cargo.toml crates/shared/github-client/src/lib.rs
git commit -m "feat(github-client): add shared crate skeleton"
```

---

### Task 2: Implement Neutral Models, Error Classification, And Attempt Outcomes

**Files:**

- Modify: `crates/shared/github-client/src/lib.rs`
- Test: `crates/shared/github-client/src/lib.rs`

- [ ] **Step 1: Write the failing classification tests**

Add unit tests covering the spec’s one-attempt outcome rules:

```rust
#[test]
fn classify_503_with_retry_after_as_upstream_unavailable_retry_after() {
    let headers = http::HeaderMap::from_iter([(
        http::HeaderName::from_static("retry-after"),
        http::HeaderValue::from_static("60"),
    )]);

    let (error, decision, _meta) =
        classify_http_failure(http::StatusCode::SERVICE_UNAVAILABLE, &headers, "{}").unwrap();

    assert!(matches!(error, GitHubClientError::UpstreamUnavailable(_)));
    assert_eq!(decision, RetryDecision::RetryAfter(std::time::Duration::from_secs(60)));
}

#[test]
fn classify_403_without_rate_limit_evidence_as_forbidden() {
    let headers = http::HeaderMap::new();
    let (error, decision, _meta) =
        classify_http_failure(http::StatusCode::FORBIDDEN, &headers, "{\"message\":\"forbidden\"}")
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
```

- [ ] **Step 2: Run the targeted tests to verify they fail**

Run:

```bash
cargo test -p uptrakit-github-client classify_503_with_retry_after_as_upstream_unavailable_retry_after -- --nocapture
```

Expected: FAIL because the classification helpers and types do not exist yet.

- [ ] **Step 3: Add the neutral models and outcome types**

Add the client-owned data and error types:

```rust
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

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum GitHubClientError {
    AuthFailed(String),
    Forbidden(String),
    NotFound(String),
    RateLimited(String),
    UpstreamUnavailable(String),
    InvalidResponse(String),
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
```

- [ ] **Step 4: Implement deterministic classification helpers**

Add helpers that preserve the reviewed semantics:

```rust
fn classify_http_failure_with_auth(
    status: http::StatusCode,
    headers: &http::HeaderMap,
    body: &str,
    auth_kind: AuthKind,
) -> Result<(GitHubClientError, RetryDecision, ResponseMetadata), GitHubClientError> {
    let message = extract_message(status, body);
    let meta = ResponseMetadata {
        status: Some(status),
        auth_kind,
        authenticated_not_found: status == http::StatusCode::NOT_FOUND && auth_kind == AuthKind::Bearer,
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

fn classify_http_failure(
    status: http::StatusCode,
    headers: &http::HeaderMap,
    body: &str,
) -> Result<(GitHubClientError, RetryDecision, ResponseMetadata), GitHubClientError> {
    classify_http_failure_with_auth(status, headers, body, AuthKind::Anonymous)
}

fn looks_rate_limited(headers: &http::HeaderMap, body: &str) -> bool {
    headers.get("retry-after").is_some()
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

#[derive(Debug, Deserialize)]
struct RepositoryTreeDto {
    truncated: bool,
    tree: Vec<RepositoryTreeEntryDto>,
}

#[derive(Debug, Deserialize)]
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
```

- [ ] **Step 5: Run the new crate tests again**

Run:

```bash
cargo test -p uptrakit-github-client -- --nocapture
```

Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/shared/github-client/src/lib.rs
git commit -m "feat(github-client): add neutral models and classification"
```

---

### Task 3: Add One-Attempt HTTP Execution For Repository Trees

**Files:**

- Modify: `crates/shared/github-client/src/lib.rs`
- Test: `crates/shared/github-client/src/lib.rs`

- [ ] **Step 1: Write the failing execution test**

Add a unit test for tree-response decoding and outcome wrapping:

```rust
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

    let outcome = client.fetch_repository_tree("homarr-labs", "dashboard-icons", "main", true).await.unwrap();
    let AttemptOutcome::Success(tree, _) = outcome else { panic!("expected success"); };
    assert_eq!(tree.entries.len(), 2);
}
```

- [ ] **Step 2: Run the targeted test to verify it fails**

Run:

```bash
cargo test -p uptrakit-github-client fetch_repository_tree_decodes_blob_and_tree_entries -- --nocapture
```

Expected: FAIL because `GitHubClient::fetch_repository_tree` does not exist yet.

- [ ] **Step 3: Add the concrete client and DTO mapping**

Implement the concrete client and endpoint-local DTOs:

```rust
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
            owner: owner.to_string(),
            repo: repo.to_string(),
            git_ref: git_ref.to_string(),
            recursive,
        }
        .build_request(&self.config)?;

        let response = self.config.http_client.execute(request)
            .await
            .map_err(|error| GitHubClientError::UpstreamUnavailable(error.to_string()))?;
        let status = response.status();
        let headers = response.headers().clone();
        let body = response.text()
            .await
            .map_err(|error| GitHubClientError::UpstreamUnavailable(error.to_string()))?;

        let auth_kind = match &self.config.auth {
            GitHubAuth::Anonymous => AuthKind::Anonymous,
            GitHubAuth::BearerToken(_) => AuthKind::Bearer,
        };

        if status.is_success() {
            let parsed: RepositoryTreeDto = serde_json::from_str(&body)
                .map_err(|error| GitHubClientError::InvalidResponse(error.to_string()))?;
            let tree = parsed.into_model()?;
            return Ok(AttemptOutcome::Success(tree, ResponseMetadata {
                status: Some(status),
                auth_kind,
                authenticated_not_found: false,
                rate_limit_remaining: headers.get("x-ratelimit-remaining")
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse().ok()),
            }));
        }

        let (error, decision, meta) =
            classify_http_failure_with_auth(status, &headers, &body, auth_kind)?;
        Ok(AttemptOutcome::Failure(error, decision, meta))
    }
}
```

- [ ] **Step 4: Run the new crate test suite**

Run:

```bash
cargo test -p uptrakit-github-client -- --nocapture
```

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/shared/github-client/src/lib.rs
git commit -m "feat(github-client): implement repository tree execution"
```

---

### Task 4: Replace The `octocrab` Runtime Adapter In `uptrakit-web-api`

**Files:**

- Modify: `crates/ui/web-api/Cargo.toml`
- Modify: `crates/ui/web-api/src/global_providers/github.rs`
- Modify: `crates/ui/web-api/src/global_providers/tests.rs`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Test: `crates/ui/web-api/src/global_providers/tests.rs`

- [ ] **Step 1: Write the failing runtime adapter tests**

Extend `crates/ui/web-api/src/global_providers/tests.rs` with an adapter-mapping test:

```rust
// Add this test-only constructor beside the existing helpers in
// crates/ui/web-api/src/global_providers/github.rs:
impl TestClientBehavior {
    pub fn not_found() -> Self {
        Self {
            outcomes: std::collections::VecDeque::from([Err(
                RuntimeRequestError::NotFound("404: Not Found".into()),
            )]),
        }
    }
}

#[tokio::test]
async fn runtime_maps_client_not_found_to_request_failed() {
    let runtime = TestRuntimeBuilder::new()
        .with_client_behavior(TestClientBehavior::not_found())
        .build()
        .await;

    let err = runtime
        .github_client()
        .await
        .unwrap()
        .fetch_repository_tree(DASHBOARD_ICONS, "o", "r", "main", true)
        .await
        .unwrap_err();

    assert!(matches!(err, GitHubProviderError::RequestFailed(message) if message.contains("404")));
}
```

- [ ] **Step 2: Run the targeted runtime test to verify it fails**

Run:

```bash
cargo test -p uptrakit-web-api --no-default-features --features db-sqlite,dashboard-icons runtime_maps_client_not_found_to_request_failed -- --nocapture
```

Expected: FAIL because the runtime still uses the `octocrab` implementation path.

- [ ] **Step 3: Replace the factory with a tiny-client-backed implementation**

Update dependencies:

```toml
# Cargo.toml
[workspace.dependencies]
uptrakit-github-client = { path = "crates/shared/github-client" }
```

```toml
# crates/ui/web-api/Cargo.toml
uptrakit-github-client = { workspace = true }
# remove octocrab
```

Then replace the `OctocrabGitHubClientFactory` in
`crates/ui/web-api/src/global_providers/github.rs` with a runtime adapter:

```rust
struct ReqwestGitHubClientFactory;

impl GitHubClientFactory for ReqwestGitHubClientFactory {
    fn build(
        &self,
        defaults: Option<&uptrakit_shared_db::provider_settings::GitHubProviderDefaults>,
    ) -> Result<Arc<dyn GitHubRequestExecutor>, GitHubProviderError> {
        let http_client = uptrakit_plugin_infrastructure_core::build_plugin_http_client(
            uptrakit_plugin_infrastructure_core::PluginHttpClientConfig {
                user_agent: "uptrakit-controller",
                ..Default::default()
            },
        )
        .map_err(|error| GitHubProviderError::Misconfigured(error.to_string()))?;

        let base_url = defaults
            .and_then(|value| value.api_base_url.as_deref())
            .unwrap_or(uptrakit_shared_db::provider_settings::DEFAULT_GITHUB_API_BASE_URL);
        let base_url = url::Url::parse(base_url)
            .map_err(|error| GitHubProviderError::Misconfigured(error.to_string()))?;

        let auth = match defaults.and_then(|value| value.auth_token.clone()) {
            Some(token) => uptrakit_github_client::GitHubAuth::BearerToken(
                uptrakit_internal_wire::SecretString::new(token),
            ),
            None => uptrakit_github_client::GitHubAuth::Anonymous,
        };

        let client = uptrakit_github_client::GitHubClient::new(
            uptrakit_github_client::GitHubClientConfig::new(
                http_client,
                base_url,
                auth,
                "uptrakit-controller",
            ),
        );

        Ok(Arc::new(ReqwestGitHubRequestExecutor { client }))
    }
}
```

- [ ] **Step 4: Keep the runtime seam and map neutral outcomes back to provider errors**

Retain `GitHubRequestExecutor`, but change it to return neutral runtime errors:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RuntimeRequestError {
    Throttled { retry_after: std::time::Duration },
    AuthFailed(String),
    NotFound(String),
    UpstreamUnavailable(String),
    RequestFailed(String),
    Misconfigured(String),
}

#[async_trait]
impl GitHubRequestExecutor for ReqwestGitHubRequestExecutor {
    async fn fetch_repository_tree(
        &self,
        _consumer: GlobalProviderConsumerId,
        owner: &str,
        repo: &str,
        git_ref: &str,
        recursive: bool,
    ) -> Result<GitHubRepositoryTree, RuntimeRequestError> {
        let outcome = self
            .client
            .fetch_repository_tree(owner, repo, git_ref, recursive)
            .await
            .map_err(|error| match error {
                uptrakit_github_client::GitHubClientError::AuthFailed(message) => {
                    RuntimeRequestError::AuthFailed(message)
                }
                uptrakit_github_client::GitHubClientError::Forbidden(message) => {
                    RuntimeRequestError::AuthFailed(message)
                }
                uptrakit_github_client::GitHubClientError::NotFound(message) => {
                    RuntimeRequestError::NotFound(message)
                }
                uptrakit_github_client::GitHubClientError::InvalidResponse(message) => {
                    RuntimeRequestError::RequestFailed(message)
                }
                uptrakit_github_client::GitHubClientError::Misconfigured(message) => {
                    RuntimeRequestError::Misconfigured(message)
                }
                uptrakit_github_client::GitHubClientError::RateLimited(_) => {
                    RuntimeRequestError::Throttled {
                        retry_after: std::time::Duration::from_secs(30),
                    }
                }
                uptrakit_github_client::GitHubClientError::UpstreamUnavailable(message) => {
                    RuntimeRequestError::UpstreamUnavailable(message)
                }
                _ => RuntimeRequestError::RequestFailed(
                    "unsupported GitHub client error".to_string(),
                ),
            })?;

        match outcome {
            uptrakit_github_client::AttemptOutcome::Success(tree, _) => {
                let entries = tree
                    .entries
                    .into_iter()
                    .map(|entry| {
                        let kind = match entry.kind {
                            uptrakit_github_client::RepositoryTreeEntryKind::Blob => GitHubTreeEntryKind::Blob,
                            uptrakit_github_client::RepositoryTreeEntryKind::Tree => GitHubTreeEntryKind::Tree,
                            _ => {
                                return Err(RuntimeRequestError::RequestFailed(
                                    "unsupported GitHub tree entry kind".to_string(),
                                ));
                            }
                        };

                        Ok(GitHubTreeEntry {
                            path: entry.path,
                            kind,
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;

                Ok(GitHubRepositoryTree {
                    truncated: tree.truncated,
                    entries,
                })
            }
            uptrakit_github_client::AttemptOutcome::Failure(error, decision, _) => {
                match (error, decision) {
                    (uptrakit_github_client::GitHubClientError::RateLimited(message), uptrakit_github_client::RetryDecision::RetryAfter(retry_after)) => {
                        let _ = message;
                        Err(RuntimeRequestError::Throttled { retry_after })
                    }
                    (uptrakit_github_client::GitHubClientError::RateLimited(_), _) => {
                        Err(RuntimeRequestError::Throttled { retry_after: std::time::Duration::from_secs(30) })
                    }
                    (uptrakit_github_client::GitHubClientError::UpstreamUnavailable(_), uptrakit_github_client::RetryDecision::RetryAfter(retry_after)) => {
                        Err(RuntimeRequestError::Throttled { retry_after })
                    }
                    (uptrakit_github_client::GitHubClientError::AuthFailed(message), _) => Err(RuntimeRequestError::AuthFailed(message)),
                    (uptrakit_github_client::GitHubClientError::Forbidden(message), _) => Err(RuntimeRequestError::AuthFailed(message)),
                    (uptrakit_github_client::GitHubClientError::NotFound(message), _) => Err(RuntimeRequestError::NotFound(message)),
                    (uptrakit_github_client::GitHubClientError::UpstreamUnavailable(message), _) => Err(RuntimeRequestError::UpstreamUnavailable(message)),
                    (uptrakit_github_client::GitHubClientError::InvalidResponse(message), _) => Err(RuntimeRequestError::RequestFailed(message)),
                    (uptrakit_github_client::GitHubClientError::Misconfigured(message), _) => Err(RuntimeRequestError::Misconfigured(message)),
                    _ => Err(RuntimeRequestError::RequestFailed(
                        "unsupported GitHub client outcome".to_string(),
                    )),
                }
            }
            _ => Err(RuntimeRequestError::RequestFailed(
                "unsupported GitHub client outcome".to_string(),
            )),
        }
    }
}
```

- [ ] **Step 4A: Update the runtime retry loop for the new `NotFound` variant**

Add the missing `GitHubProviderRuntime` match arm so the runtime contract stays
consistent:

```rust
Err(RuntimeRequestError::NotFound(message)) => {
    self.metrics.record_request(consumer, "request_failed");
    return Err(GitHubProviderError::RequestFailed(message));
}
```

- [ ] **Step 5: Remove `octocrab` and run the targeted verification**

Run:

```bash
cargo test -p uptrakit-web-api --no-default-features --features db-sqlite,dashboard-icons global_providers -- --nocapture
cargo test -p uptrakit-web-api --no-default-features --features db-sqlite,dashboard-icons github_provider_settings -- --nocapture
cargo check -p uptrakit-controller --no-default-features --features db-sqlite,dashboard-icons
```

Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock crates/ui/web-api/Cargo.toml crates/ui/web-api/src/global_providers/github.rs crates/ui/web-api/src/global_providers/tests.rs
git commit -m "refactor(web-api): replace octocrab with tiny GitHub client"
```

---

### Task 5: Final Verification And Documentation Cleanup

**Files:**

- Modify: `docs/superpowers/specs/2026-04-17-tiny-github-client-design.md`

- [ ] **Step 1: Update the spec if implementation reality forced any naming changes**

If the implemented crate uses slightly different concrete names while preserving
the reviewed architecture, reconcile the spec immediately. Also update any
other docs in this worktree that still describe an `octocrab`-based global
provider runtime.

For example:

```markdown
- replace `SecretString-like wrapper` with the actual chosen type name
- replace any placeholder helper names with the exact final names
```

- [ ] **Step 2: Run the full targeted verification suite**

Run:

```bash
cargo test -p uptrakit-github-client -- --nocapture
cargo test -p uptrakit-web-api --no-default-features --features db-sqlite,dashboard-icons global_providers -- --nocapture
cargo test -p uptrakit-web-api --no-default-features --features db-sqlite,dashboard-icons github_provider_settings -- --nocapture
cargo test -p uptrakit-plugin-enhancement-dashboard-icons -- --nocapture
cargo check -p uptrakit-controller --no-default-features --features db-sqlite,dashboard-icons
# If `frontend/build/` is missing, run `cd frontend && npm ci && npm run build` before this next command.
cargo clippy --all-targets --all-features
cargo deny check
cargo fmt --all --check
markdownlint --config .markdownlint.json docs/superpowers/specs/2026-04-17-tiny-github-client-design.md docs/superpowers/plans/2026-04-17-tiny-github-client.md
```

Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/specs/2026-04-17-tiny-github-client-design.md
git commit -m "docs: align tiny GitHub client spec with implementation"
```
