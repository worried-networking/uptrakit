use std::any::Any;
use std::sync::Arc;
use std::time::Duration;

#[cfg(test)]
use std::collections::VecDeque;

use arc_swap::ArcSwapOption;
use async_trait::async_trait;
use parking_lot::Mutex;
use sea_orm::DatabaseConnection;
#[cfg(test)]
use sha2::Digest;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::{Instant, timeout};
use uptrakit_github_client::{
    AttemptOutcome, GitHubAuth, GitHubClient, GitHubClientConfig, GitHubClientError,
    RepositoryTreeEntryKind, ResponseMetadata, RetryDecision,
};
use uptrakit_global_github_provider::{
    GitHubProviderClient, GitHubProviderError, GitHubProviderHandle, GitHubRepositoryTree,
    GitHubTreeEntry, GitHubTreeEntryKind, GlobalProviderConsumerId,
};
use uptrakit_plugin_infrastructure_core::{
    GlobalProviderLookup, PluginHttpClientConfig, build_plugin_http_client,
};
use uptrakit_shared_db::provider_settings::{
    DEFAULT_GITHUB_API_BASE_URL, GitHubProviderDefaults, github_provider_generation,
    load_github_provider_defaults, normalize_github_provider_defaults,
};
use uptrakit_web_api_types::events::AdminEvent;

const GITHUB_PROVIDER_ID: &str = "github";
#[cfg(test)]
const ANONYMOUS_FINGERPRINT: &str = "anonymous";
const DEFAULT_GENERATION_RECHECK_INTERVAL: Duration = Duration::from_secs(30);
const DEFAULT_QUEUE_WAIT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_CONCURRENCY_LIMIT: usize = 8;
const DEFAULT_BASE_BACKOFF: Duration = Duration::from_millis(500);
const DEFAULT_MAX_BACKOFF: Duration = Duration::from_secs(30);
const DEFAULT_MAX_RETRIES: usize = 2;
const GITHUB_PROVIDER_USER_AGENT: &str = "uptrakit-controller";

#[cfg(test)]
pub const TEST_GLOBAL_CONSUMER: GlobalProviderConsumerId =
    GlobalProviderConsumerId::new("test-global");

struct GitHubProviderRuntimeState {
    generation: [u8; 32],
    #[cfg(test)]
    credential_fingerprint: String,
    key_kind: &'static str,
    client: Arc<dyn GitHubRequestExecutor>,
}

pub struct GlobalProviders {
    github: Arc<GitHubProviderRuntime>,
    github_handle: Arc<dyn Any + Send + Sync>,
}

impl GlobalProviders {
    pub fn new(db: DatabaseConnection) -> Self {
        let github = GitHubProviderRuntime::new(db);
        let github_handle: Arc<dyn Any + Send + Sync> =
            Arc::new(GitHubProviderHandle::new(github.clone()));
        Self {
            github,
            github_handle,
        }
    }

    pub fn github(&self) -> Arc<GitHubProviderRuntime> {
        Arc::clone(&self.github)
    }
}

impl GlobalProviderLookup for GlobalProviders {
    fn lookup(&self, provider_id: &str) -> Option<Arc<dyn Any + Send + Sync>> {
        (provider_id == GITHUB_PROVIDER_ID).then(|| Arc::clone(&self.github_handle))
    }
}

struct RuntimeMetrics {
    requests: std::sync::atomic::AtomicU64,
    retries: std::sync::atomic::AtomicU64,
    auth_failures: std::sync::atomic::AtomicU64,
    cooldown_seconds: std::sync::atomic::AtomicU64,
}

impl Default for RuntimeMetrics {
    fn default() -> Self {
        Self {
            requests: std::sync::atomic::AtomicU64::new(0),
            retries: std::sync::atomic::AtomicU64::new(0),
            auth_failures: std::sync::atomic::AtomicU64::new(0),
            cooldown_seconds: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

impl RuntimeMetrics {
    fn record_request(&self, consumer: GlobalProviderConsumerId, status: &'static str) {
        self.requests
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        metrics::counter!(
            "uptrakit_global_provider_requests_total",
            "provider" => "github",
            "consumer" => consumer.as_str(),
            "status" => status
        )
        .increment(1);
    }

    fn record_retry(&self, consumer: GlobalProviderConsumerId, reason: &'static str) {
        self.retries
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        metrics::counter!(
            "uptrakit_global_provider_retries_total",
            "provider" => "github",
            "consumer" => consumer.as_str(),
            "reason" => reason
        )
        .increment(1);
    }

    fn record_auth_failure(&self, consumer: GlobalProviderConsumerId) {
        self.auth_failures
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        metrics::counter!(
            "uptrakit_global_provider_auth_failures_total",
            "provider" => "github",
            "consumer" => consumer.as_str()
        )
        .increment(1);
    }

    fn record_cooldown(&self, key_kind: &'static str, duration: Duration) {
        self.cooldown_seconds
            .store(duration.as_secs(), std::sync::atomic::Ordering::Relaxed);
        metrics::gauge!(
            "uptrakit_global_provider_cooldown_seconds",
            "provider" => "github",
            "key_kind" => key_kind
        )
        .set(duration.as_secs_f64());
    }

    #[cfg(test)]
    #[allow(dead_code)]
    fn snapshot(&self) -> TestMetricsSnapshot {
        TestMetricsSnapshot {
            requests: self.requests.load(std::sync::atomic::Ordering::Relaxed),
            retries: self.retries.load(std::sync::atomic::Ordering::Relaxed),
            auth_failures: self
                .auth_failures
                .load(std::sync::atomic::Ordering::Relaxed),
            cooldown_seconds: self
                .cooldown_seconds
                .load(std::sync::atomic::Ordering::Relaxed),
        }
    }
}

#[derive(Clone, Copy)]
struct RetryPolicy {
    max_retries: usize,
    base_backoff: Duration,
    max_backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: DEFAULT_MAX_RETRIES,
            base_backoff: DEFAULT_BASE_BACKOFF,
            max_backoff: DEFAULT_MAX_BACKOFF,
        }
    }
}

pub struct GitHubProviderRuntime {
    db: DatabaseConnection,
    state: ArcSwapOption<GitHubProviderRuntimeState>,
    factory: Arc<dyn GitHubClientFactory>,
    generation_recheck_interval: Duration,
    queue_wait_timeout: Duration,
    request_permits: Arc<Semaphore>,
    retry_policy: RetryPolicy,
    last_generation_check: Mutex<Option<Instant>>,
    cooldown_until: Mutex<Option<Instant>>,
    metrics: RuntimeMetrics,
    sleeper: Arc<dyn RuntimeSleeper>,
}

impl GitHubProviderRuntime {
    pub fn new(db: DatabaseConnection) -> Arc<Self> {
        Arc::new(Self {
            db,
            state: ArcSwapOption::default(),
            factory: Arc::new(ReqwestGitHubClientFactory),
            generation_recheck_interval: DEFAULT_GENERATION_RECHECK_INTERVAL,
            queue_wait_timeout: DEFAULT_QUEUE_WAIT_TIMEOUT,
            request_permits: Arc::new(Semaphore::new(DEFAULT_CONCURRENCY_LIMIT)),
            retry_policy: RetryPolicy::default(),
            last_generation_check: Mutex::new(None),
            cooldown_until: Mutex::new(None),
            metrics: RuntimeMetrics::default(),
            sleeper: Arc::new(TokioRuntimeSleeper),
        })
    }

    #[cfg(test)]
    pub(crate) fn new_for_tests(
        db: DatabaseConnection,
        factory: Arc<dyn GitHubClientFactory>,
        options: GitHubProviderRuntimeOptions,
    ) -> Arc<Self> {
        Arc::new(Self {
            db,
            state: ArcSwapOption::default(),
            factory,
            generation_recheck_interval: options.generation_recheck_interval,
            queue_wait_timeout: options.queue_wait_timeout,
            request_permits: Arc::new(Semaphore::new(options.concurrency_limit)),
            retry_policy: RetryPolicy::default(),
            last_generation_check: Mutex::new(None),
            cooldown_until: Mutex::new(None),
            metrics: RuntimeMetrics::default(),
            sleeper: Arc::new(RecordingRuntimeSleeper::default()),
        })
    }

    pub fn invalidate(&self) {
        self.state.store(None);
        *self.last_generation_check.lock() = None;
        *self.cooldown_until.lock() = None;
    }

    pub async fn github_client(
        self: &Arc<Self>,
    ) -> Result<Arc<dyn GitHubProviderClient>, GitHubProviderError> {
        let _ = self.current_state().await?;
        Ok(self.clone())
    }

    async fn current_state(&self) -> Result<Arc<GitHubProviderRuntimeState>, GitHubProviderError> {
        if let Some(state) = self.state.load_full() {
            if !self.should_recheck_generation() {
                return Ok(state);
            }

            let refreshed = self.load_runtime_state().await?;
            if refreshed.generation == state.generation {
                *self.last_generation_check.lock() = Some(Instant::now());
                return Ok(state);
            }

            self.store_runtime_state(refreshed.clone());
            return Ok(refreshed);
        }

        let state = self.load_runtime_state().await?;
        self.store_runtime_state(state.clone());
        Ok(state)
    }

    fn should_recheck_generation(&self) -> bool {
        let guard = self.last_generation_check.lock();
        match *guard {
            None => true,
            Some(last) => last.elapsed() >= self.generation_recheck_interval,
        }
    }

    async fn load_runtime_state(
        &self,
    ) -> Result<Arc<GitHubProviderRuntimeState>, GitHubProviderError> {
        let defaults = load_github_provider_defaults(&self.db)
            .await
            .map_err(|error| GitHubProviderError::Misconfigured(error.to_string()))?;
        let defaults = normalize_github_provider_defaults(defaults)
            .map_err(|error| GitHubProviderError::Misconfigured(error.to_string()))?;
        let generation = defaults
            .as_ref()
            .map(github_provider_generation)
            .unwrap_or([0; 32]);
        #[cfg(test)]
        let credential_fingerprint = credential_fingerprint(defaults.as_ref());
        let key_kind = if defaults.is_some() {
            "authenticated"
        } else {
            "anonymous"
        };
        let client = self.factory.build(defaults.as_ref())?;

        Ok(Arc::new(GitHubProviderRuntimeState {
            generation,
            #[cfg(test)]
            credential_fingerprint,
            key_kind,
            client,
        }))
    }

    fn store_runtime_state(&self, state: Arc<GitHubProviderRuntimeState>) {
        self.state.store(Some(state));
        *self.last_generation_check.lock() = Some(Instant::now());
        *self.cooldown_until.lock() = None;
    }

    async fn wait_for_cooldown(&self, key_kind: &'static str) -> Result<(), GitHubProviderError> {
        let maybe_wait = {
            let guard = self.cooldown_until.lock();
            let now = Instant::now();
            guard.and_then(|until| {
                until
                    .checked_duration_since(now)
                    .map(|wait_for| (until, wait_for))
            })
        };

        let Some((waited_until, wait_for)) = maybe_wait else {
            return Ok(());
        };

        self.metrics.record_cooldown(key_kind, wait_for);
        timeout(self.queue_wait_timeout, self.sleeper.sleep(wait_for))
            .await
            .map_err(|_| GitHubProviderError::Throttled)?;
        let mut guard = self.cooldown_until.lock();
        if matches!(*guard, Some(current_until) if current_until <= waited_until) {
            *guard = None;
        }
        Ok(())
    }

    async fn acquire_request_permit(&self) -> Result<OwnedSemaphorePermit, GitHubProviderError> {
        timeout(
            self.queue_wait_timeout,
            self.request_permits.clone().acquire_owned(),
        )
        .await
        .map_err(|_| GitHubProviderError::Throttled)?
        .map_err(|_| GitHubProviderError::Throttled)
    }

    fn set_cooldown(&self, retry_after: Duration) {
        let new_until = Instant::now() + retry_after;
        let mut guard = self.cooldown_until.lock();
        *guard = Some(match *guard {
            Some(existing_until) if existing_until > new_until => existing_until,
            _ => new_until,
        });
    }

    fn backoff_for_attempt(&self, attempt: usize) -> Duration {
        let delay = self
            .retry_policy
            .base_backoff
            .saturating_mul(2u32.saturating_pow(attempt as u32));
        delay.min(self.retry_policy.max_backoff)
    }

    #[cfg(test)]
    pub(crate) fn cached_generation_for_tests(&self) -> Option<[u8; 32]> {
        self.state.load_full().map(|state| state.generation)
    }

    #[cfg(test)]
    pub(crate) fn cached_credential_fingerprint_for_tests(&self) -> Option<String> {
        self.state
            .load_full()
            .map(|state| state.credential_fingerprint.clone())
    }

    #[cfg(test)]
    pub(crate) async fn acquire_request_permit_for_tests(&self) -> OwnedSemaphorePermit {
        self.request_permits
            .clone()
            .acquire_owned()
            .await
            .expect("permit")
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn cooldown_until_for_tests(&self) -> Option<Instant> {
        *self.cooldown_until.lock()
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn metrics_snapshot_for_tests(&self) -> TestMetricsSnapshot {
        self.metrics.snapshot()
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) async fn replace_defaults_for_tests(&self, defaults: GitHubProviderDefaults) {
        uptrakit_shared_db::provider_settings::upsert_github_provider_defaults(
            &self.db,
            defaults.auth_token.as_deref(),
            defaults.api_base_url.as_deref(),
        )
        .await
        .expect("replace defaults");
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn last_sleep_for_tests(&self) -> Option<Duration> {
        self.sleeper.last_sleep()
    }

    #[cfg(test)]
    pub(crate) fn set_cooldown_for_tests(&self, retry_after: Duration) {
        self.set_cooldown(retry_after);
    }

    #[cfg(test)]
    pub(crate) fn remaining_cooldown_for_tests(&self) -> Option<Duration> {
        self.cooldown_until
            .lock()
            .and_then(|until| until.checked_duration_since(Instant::now()))
    }

    #[cfg(test)]
    pub(crate) async fn wait_for_cooldown_for_tests(
        &self,
        key_kind: &'static str,
    ) -> Result<(), GitHubProviderError> {
        self.wait_for_cooldown(key_kind).await
    }
}

#[async_trait]
impl GitHubProviderClient for GitHubProviderRuntime {
    async fn fetch_repository_tree(
        &self,
        consumer: GlobalProviderConsumerId,
        owner: &str,
        repo: &str,
        git_ref: &str,
        recursive: bool,
    ) -> Result<GitHubRepositoryTree, GitHubProviderError> {
        for attempt in 0..=self.retry_policy.max_retries {
            let state = self.current_state().await?;
            self.wait_for_cooldown(state.key_kind).await?;
            let _permit = self.acquire_request_permit().await?;
            self.metrics.record_request(consumer, "attempt");
            metrics::counter!(
                "uptrakit_global_provider_requests_total",
                "provider" => "github",
                "consumer" => consumer.as_str(),
                "status" => "scheduled"
            )
            .increment(1);

            match state
                .client
                .fetch_repository_tree(consumer, owner, repo, git_ref, recursive)
                .await
            {
                Ok(tree) => {
                    self.metrics.record_request(consumer, "success");
                    return Ok(tree);
                }
                Err(RuntimeRequestError::Throttled { retry_after }) => {
                    self.metrics.record_request(consumer, "throttled");
                    if let Some(retry_after) = retry_after {
                        self.set_cooldown(retry_after);
                        self.metrics.record_cooldown(state.key_kind, retry_after);
                    }
                    if attempt < self.retry_policy.max_retries {
                        self.metrics.record_retry(consumer, "throttled");
                        if retry_after.is_none() {
                            let backoff = self.backoff_for_attempt(attempt);
                            self.sleeper.sleep(backoff).await;
                        }
                        continue;
                    }
                    return Err(GitHubProviderError::Throttled);
                }
                Err(RuntimeRequestError::AuthFailed(message)) => {
                    self.metrics.record_request(consumer, "auth_failed");
                    self.metrics.record_auth_failure(consumer);
                    return Err(GitHubProviderError::AuthFailed(message));
                }
                Err(RuntimeRequestError::NotFound(message)) => {
                    self.metrics.record_request(consumer, "request_failed");
                    return Err(GitHubProviderError::RequestFailed(message));
                }
                Err(RuntimeRequestError::UpstreamUnavailable {
                    message,
                    retry_after,
                }) => {
                    self.metrics
                        .record_request(consumer, "upstream_unavailable");
                    if let Some(retry_after) = retry_after {
                        self.set_cooldown(retry_after);
                        self.metrics.record_cooldown(state.key_kind, retry_after);
                    }
                    if attempt < self.retry_policy.max_retries {
                        self.metrics.record_retry(consumer, "upstream_unavailable");
                        if retry_after.is_none() {
                            let backoff = self.backoff_for_attempt(attempt);
                            self.sleeper.sleep(backoff).await;
                        }
                        continue;
                    }
                    return Err(GitHubProviderError::UpstreamUnavailable(message));
                }
                Err(RuntimeRequestError::RequestFailed(message)) => {
                    self.metrics.record_request(consumer, "request_failed");
                    return Err(GitHubProviderError::RequestFailed(message));
                }
                Err(RuntimeRequestError::Misconfigured(message)) => {
                    self.metrics.record_request(consumer, "misconfigured");
                    return Err(GitHubProviderError::Misconfigured(message));
                }
            }
        }

        Err(GitHubProviderError::UpstreamUnavailable(
            "retry budget exhausted".to_string(),
        ))
    }
}

pub(crate) async fn detect_global_github_provider_problem(
    db: &DatabaseConnection,
) -> Option<String> {
    match load_github_provider_defaults(db).await {
        Ok(defaults) => normalize_github_provider_defaults(defaults)
            .err()
            .map(|error| error.to_string()),
        Err(error) => Some(error.to_string()),
    }
}

pub async fn emit_global_github_provider_diagnostic_if_needed(
    db: &DatabaseConnection,
    broadcaster: &crate::event_broadcaster::EventBroadcaster,
) -> Option<String> {
    let problem = detect_global_github_provider_problem(db).await?;
    tracing::warn!(problem = %problem, "global GitHub provider misconfiguration");
    broadcaster
        .send_global(AdminEvent::GlobalGitHubProviderMisconfigured {
            problem: problem.clone(),
        })
        .await;
    Some(problem)
}

#[cfg(test)]
fn credential_fingerprint(defaults: Option<&GitHubProviderDefaults>) -> String {
    let Some(defaults) = defaults else {
        return ANONYMOUS_FINGERPRINT.to_string();
    };
    let Some(token) = defaults.auth_token.as_deref() else {
        return ANONYMOUS_FINGERPRINT.to_string();
    };
    let mut hasher = sha2::Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) struct GitHubProviderRuntimeOptions {
    pub(crate) generation_recheck_interval: Duration,
    pub(crate) queue_wait_timeout: Duration,
    pub(crate) concurrency_limit: usize,
}

#[cfg(test)]
impl Default for GitHubProviderRuntimeOptions {
    fn default() -> Self {
        Self {
            generation_recheck_interval: DEFAULT_GENERATION_RECHECK_INTERVAL,
            queue_wait_timeout: DEFAULT_QUEUE_WAIT_TIMEOUT,
            concurrency_limit: DEFAULT_CONCURRENCY_LIMIT,
        }
    }
}

pub(crate) trait GitHubClientFactory: Send + Sync {
    fn build(
        &self,
        defaults: Option<&GitHubProviderDefaults>,
    ) -> Result<Arc<dyn GitHubRequestExecutor>, GitHubProviderError>;
}

#[async_trait]
pub(crate) trait GitHubRequestExecutor: Send + Sync {
    async fn fetch_repository_tree(
        &self,
        consumer: GlobalProviderConsumerId,
        owner: &str,
        repo: &str,
        git_ref: &str,
        recursive: bool,
    ) -> Result<GitHubRepositoryTree, RuntimeRequestError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RuntimeRequestError {
    Throttled {
        retry_after: Option<Duration>,
    },
    AuthFailed(String),
    NotFound(String),
    UpstreamUnavailable {
        message: String,
        retry_after: Option<Duration>,
    },
    RequestFailed(String),
    Misconfigured(String),
}

#[async_trait]
trait RuntimeSleeper: Send + Sync {
    async fn sleep(&self, duration: Duration);

    #[cfg(test)]
    #[allow(dead_code)]
    fn last_sleep(&self) -> Option<Duration> {
        None
    }
}

struct TokioRuntimeSleeper;

#[async_trait]
impl RuntimeSleeper for TokioRuntimeSleeper {
    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

#[cfg(test)]
#[derive(Default)]
struct RecordingRuntimeSleeper {
    last_sleep: Mutex<Option<Duration>>,
}

#[cfg(test)]
#[async_trait]
impl RuntimeSleeper for RecordingRuntimeSleeper {
    async fn sleep(&self, duration: Duration) {
        *self.last_sleep.lock() = Some(duration);
        tokio::time::sleep(duration).await;
    }

    fn last_sleep(&self) -> Option<Duration> {
        *self.last_sleep.lock()
    }
}

struct ReqwestGitHubClientFactory;

impl GitHubClientFactory for ReqwestGitHubClientFactory {
    fn build(
        &self,
        defaults: Option<&GitHubProviderDefaults>,
    ) -> Result<Arc<dyn GitHubRequestExecutor>, GitHubProviderError> {
        let http_client = build_plugin_http_client(PluginHttpClientConfig {
            user_agent: GITHUB_PROVIDER_USER_AGENT,
            ..PluginHttpClientConfig::default()
        })
        .map_err(GitHubProviderError::Misconfigured)?;

        let base_url = defaults
            .and_then(|value| value.api_base_url.as_deref())
            .unwrap_or(DEFAULT_GITHUB_API_BASE_URL);
        let base_url = url::Url::parse(base_url)
            .map_err(|error| GitHubProviderError::Misconfigured(error.to_string()))?;

        let auth = match defaults.and_then(|value| value.auth_token.clone()) {
            Some(token) => {
                GitHubAuth::BearerToken(uptrakit_internal_wire::SecretString::new(token))
            }
            None => GitHubAuth::Anonymous,
        };

        let client = GitHubClient::new(GitHubClientConfig::new(
            http_client,
            base_url,
            auth,
            GITHUB_PROVIDER_USER_AGENT,
        ));

        Ok(Arc::new(ReqwestGitHubRequestExecutor { client }))
    }
}

struct ReqwestGitHubRequestExecutor {
    client: GitHubClient,
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
            .map_err(|error| map_client_failure(error, None, None))?;

        match outcome {
            AttemptOutcome::Success(tree, _metadata) => map_repository_tree_response(tree),
            AttemptOutcome::Failure(error, decision, metadata) => {
                Err(map_client_failure(error, Some(decision), Some(metadata)))
            }
            _ => Err(RuntimeRequestError::RequestFailed(
                "unsupported GitHub client outcome".to_string(),
            )),
        }
    }
}

fn map_repository_tree_response(
    tree: uptrakit_github_client::RepositoryTreeResponse,
) -> Result<GitHubRepositoryTree, RuntimeRequestError> {
    let entries = tree
        .entries
        .into_iter()
        .map(|entry| {
            let kind = match entry.kind {
                RepositoryTreeEntryKind::Blob => GitHubTreeEntryKind::Blob,
                RepositoryTreeEntryKind::Tree => GitHubTreeEntryKind::Tree,
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

fn map_client_failure(
    error: GitHubClientError,
    decision: Option<RetryDecision>,
    metadata: Option<ResponseMetadata>,
) -> RuntimeRequestError {
    match error {
        GitHubClientError::AuthFailed(message) | GitHubClientError::Forbidden(message) => {
            RuntimeRequestError::AuthFailed(message)
        }
        GitHubClientError::NotFound(message) => {
            if metadata.is_some_and(|metadata| metadata.authenticated_not_found) {
                RuntimeRequestError::AuthFailed(message)
            } else {
                RuntimeRequestError::NotFound(message)
            }
        }
        GitHubClientError::RateLimited(_message) => RuntimeRequestError::Throttled {
            retry_after: retry_after_from_decision(decision),
        },
        GitHubClientError::UpstreamUnavailable(message) => {
            RuntimeRequestError::UpstreamUnavailable {
                message,
                retry_after: retry_after_from_decision(decision),
            }
        }
        GitHubClientError::InvalidResponse(message) => RuntimeRequestError::RequestFailed(message),
        GitHubClientError::Misconfigured(message) => RuntimeRequestError::Misconfigured(message),
        _ => RuntimeRequestError::RequestFailed("unsupported GitHub client outcome".to_string()),
    }
}

#[cfg(test)]
pub(crate) fn map_client_failure_for_tests(
    error: GitHubClientError,
    decision: Option<RetryDecision>,
    metadata: Option<ResponseMetadata>,
) -> RuntimeRequestError {
    map_client_failure(error, decision, metadata)
}

fn retry_after_from_decision(decision: Option<RetryDecision>) -> Option<Duration> {
    match decision {
        Some(RetryDecision::RetryAfter(retry_after)) => Some(retry_after),
        Some(RetryDecision::Backoff) | Some(RetryDecision::DoNotRetry) | None => None,
        Some(_) => None,
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TestMetricsSnapshot {
    pub requests: u64,
    pub retries: u64,
    pub auth_failures: u64,
    pub cooldown_seconds: u64,
}

#[cfg(test)]
#[derive(Clone)]
pub struct TestClientBehavior {
    outcomes: VecDeque<Result<GitHubRepositoryTree, RuntimeRequestError>>,
}

#[cfg(test)]
impl TestClientBehavior {
    pub fn success() -> Self {
        Self {
            outcomes: VecDeque::from([Ok(default_test_tree())]),
        }
    }

    pub fn retry_then_success() -> Self {
        Self {
            outcomes: VecDeque::from([
                Err(RuntimeRequestError::UpstreamUnavailable {
                    message: "transient".to_string(),
                    retry_after: None,
                }),
                Ok(default_test_tree()),
            ]),
        }
    }

    pub fn auth_failed() -> Self {
        Self {
            outcomes: VecDeque::from([Err(RuntimeRequestError::AuthFailed(
                "bad credentials".to_string(),
            ))]),
        }
    }

    pub fn throttled_then_success(retry_after: Duration) -> Self {
        Self {
            outcomes: VecDeque::from([
                Err(RuntimeRequestError::Throttled {
                    retry_after: Some(retry_after),
                }),
                Ok(default_test_tree()),
            ]),
        }
    }
}

#[cfg(test)]
#[derive(Clone)]
pub struct TestRuntimeBuilder {
    defaults: Option<GitHubProviderDefaults>,
    behaviors: Vec<TestClientBehavior>,
    options: GitHubProviderRuntimeOptions,
}

#[cfg(test)]
impl Default for TestRuntimeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl TestRuntimeBuilder {
    pub fn new() -> Self {
        Self {
            defaults: Some(valid_defaults()),
            behaviors: Vec::new(),
            options: GitHubProviderRuntimeOptions::default(),
        }
    }

    pub fn paired_runtimes() -> Self {
        Self::new()
    }

    pub fn with_valid_defaults(mut self) -> Self {
        self.defaults = Some(valid_defaults());
        self
    }

    pub fn without_global_defaults(mut self) -> Self {
        self.defaults = None;
        self
    }

    pub fn with_client_behavior(mut self, behavior: TestClientBehavior) -> Self {
        self.behaviors.push(behavior);
        self
    }

    pub fn with_concurrency_limit(mut self, concurrency_limit: usize) -> Self {
        self.options.concurrency_limit = concurrency_limit;
        self
    }

    pub fn with_queue_wait_timeout(mut self, queue_wait_timeout: Duration) -> Self {
        self.options.queue_wait_timeout = queue_wait_timeout;
        self
    }

    pub fn rotated_defaults() -> GitHubProviderDefaults {
        GitHubProviderDefaults {
            auth_token: Some("ghp_rotated".to_string()),
            api_base_url: Some("https://ghe.example.com/api/v3".to_string()),
        }
    }

    pub async fn build(self) -> Arc<GitHubProviderRuntime> {
        let db = runtime_test_db().await;
        seed_runtime_defaults(&db, self.defaults.clone()).await;
        GitHubProviderRuntime::new_for_tests(
            db,
            Arc::new(FakeGitHubClientFactory::new(self.behaviors)),
            self.options,
        )
    }

    pub async fn build_pair(self) -> (Arc<GitHubProviderRuntime>, Arc<GitHubProviderRuntime>) {
        let db = runtime_test_db().await;
        seed_runtime_defaults(&db, self.defaults.clone()).await;
        let factory = Arc::new(FakeGitHubClientFactory::new(self.behaviors));
        (
            GitHubProviderRuntime::new_for_tests(db.clone(), factory.clone(), self.options),
            GitHubProviderRuntime::new_for_tests(db, factory, self.options),
        )
    }
}

#[cfg(test)]
async fn seed_runtime_defaults(db: &DatabaseConnection, defaults: Option<GitHubProviderDefaults>) {
    if let Some(defaults) = defaults {
        uptrakit_shared_db::provider_settings::upsert_github_provider_defaults(
            db,
            defaults.auth_token.as_deref(),
            defaults.api_base_url.as_deref(),
        )
        .await
        .expect("seed defaults");
    }
}

#[cfg(test)]
async fn runtime_test_db() -> DatabaseConnection {
    uptrakit_crypto::enable_plaintext_mode();
    let mut options = sea_orm::ConnectOptions::new("sqlite::memory:".to_owned());
    options.max_connections(1).min_connections(1);
    let db = sea_orm::Database::connect(options).await.expect("db");
    uptrakit_shared_db::migration::run_migrations(&db)
        .await
        .expect("migrations");
    db
}

#[cfg(test)]
fn valid_defaults() -> GitHubProviderDefaults {
    GitHubProviderDefaults {
        auth_token: Some("ghp_valid".to_string()),
        api_base_url: Some(DEFAULT_GITHUB_API_BASE_URL.to_string()),
    }
}

#[cfg(test)]
fn default_test_tree() -> GitHubRepositoryTree {
    GitHubRepositoryTree {
        truncated: false,
        entries: vec![GitHubTreeEntry {
            path: "svg/nginx.svg".to_string(),
            kind: GitHubTreeEntryKind::Blob,
        }],
    }
}

#[cfg(test)]
struct FakeGitHubClientFactory {
    outcomes: Arc<Mutex<VecDeque<Result<GitHubRepositoryTree, RuntimeRequestError>>>>,
}

#[cfg(test)]
impl FakeGitHubClientFactory {
    fn new(behaviors: Vec<TestClientBehavior>) -> Self {
        let outcomes = behaviors
            .into_iter()
            .flat_map(|behavior| behavior.outcomes.into_iter())
            .collect();
        Self {
            outcomes: Arc::new(Mutex::new(outcomes)),
        }
    }
}

#[cfg(test)]
impl GitHubClientFactory for FakeGitHubClientFactory {
    fn build(
        &self,
        _defaults: Option<&GitHubProviderDefaults>,
    ) -> Result<Arc<dyn GitHubRequestExecutor>, GitHubProviderError> {
        Ok(Arc::new(FakeGitHubRequestExecutor {
            outcomes: Arc::clone(&self.outcomes),
        }))
    }
}

#[cfg(test)]
struct FakeGitHubRequestExecutor {
    outcomes: Arc<Mutex<VecDeque<Result<GitHubRepositoryTree, RuntimeRequestError>>>>,
}

#[cfg(test)]
#[async_trait]
impl GitHubRequestExecutor for FakeGitHubRequestExecutor {
    async fn fetch_repository_tree(
        &self,
        _consumer: GlobalProviderConsumerId,
        _owner: &str,
        _repo: &str,
        _git_ref: &str,
        _recursive: bool,
    ) -> Result<GitHubRepositoryTree, RuntimeRequestError> {
        self.outcomes
            .lock()
            .pop_front()
            .unwrap_or_else(|| Ok(default_test_tree()))
    }
}
