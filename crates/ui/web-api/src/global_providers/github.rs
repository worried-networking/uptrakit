#![expect(
    clippy::map_err_ignore,
    reason = "original parse errors carry no useful context; replaced with contextual messages"
)]

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
use time::format_description::well_known::Rfc3339;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::{Instant, timeout};
use uptrakit_github_client::{
    AttemptOutcome, AuthKind, GitHubAuth, GitHubClient, GitHubClientConfig, GitHubClientError,
    RepositoryTreeEntryKind, ResponseMetadata, RetryDecision, classify_http_failure_with_auth,
};
use uptrakit_global_github_provider::{
    GitHubProviderClient, GitHubProviderError, GitHubProviderHandle, GitHubRepositoryTree,
    GitHubTreeEntry, GitHubTreeEntryKind, GlobalProviderConsumerId, TreeCommit,
};
use uptrakit_plugin_infrastructure_registry::{
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
    #[expect(
        dead_code,
        reason = "test-only helper; not always called from every test"
    )]
    pub(crate) fn cooldown_until_for_tests(&self) -> Option<Instant> {
        *self.cooldown_until.lock()
    }

    #[cfg(test)]
    pub(crate) fn metrics_snapshot_for_tests(&self) -> TestMetricsSnapshot {
        self.metrics.snapshot()
    }

    #[cfg(test)]
    #[expect(
        dead_code,
        reason = "test-only helper; not always called from every test"
    )]
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

    async fn list_recent_commit_dates_for_path(
        &self,
        consumer: GlobalProviderConsumerId,
        owner: &str,
        repo: &str,
        path: &str,
        limit: usize,
        expected_shas: &std::collections::HashSet<String>,
    ) -> Result<Vec<TreeCommit>, GitHubProviderError> {
        const PER_PAGE: usize = 30;
        const HARD_CAP: usize = 90;
        let effective_limit = limit.min(HARD_CAP);
        if effective_limit == 0 {
            return Ok(Vec::new());
        }
        let max_pages = effective_limit.div_ceil(PER_PAGE);

        // Paged commits walk (newest-first per call; reversed after collection).
        let mut commits: Vec<CommitItem> = Vec::new();
        'pages: for page in 1..=max_pages {
            let page_resp = self
                .run_with_retry_shell(consumer, |state| {
                    let owner = owner.to_string();
                    let repo = repo.to_string();
                    let path = path.to_string();
                    async move {
                        state
                            .client
                            .list_commits_for_path_page(
                                consumer, &owner, &repo, &path, PER_PAGE, page,
                            )
                            .await
                    }
                })
                .await?;
            let page_len = page_resp.len();
            if page_len == 0 {
                break;
            }
            for c in page_resp {
                commits.push(c);
                if commits.len() >= effective_limit {
                    break 'pages;
                }
            }
            // GitHub returns fewer than `per_page` rows on the last page; stop
            // paginating to avoid a redundant fetch (and the 404 it would yield
            // against a strict httpmock harness).
            if page_len < PER_PAGE {
                break;
            }
        }
        if commits.is_empty() {
            return Ok(Vec::new());
        }
        commits.reverse(); // oldest-first

        // Resolve subtree-at-path per commit. Cross-batch cache keyed by
        // (tree_sha, path_segment). Each cache miss is one non-recursive tree
        // call routed through the same retry shell.
        let path_segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let mut subtree_cache: std::collections::HashMap<(String, String), String> =
            std::collections::HashMap::new();

        let mut out: Vec<TreeCommit> = Vec::new();
        for commit in &commits {
            let mut current = commit.root_tree_sha.clone();
            let mut walk_ok = true;
            for seg in &path_segments {
                let cache_key = (current.clone(), (*seg).to_string());
                if let Some(child) = subtree_cache.get(&cache_key) {
                    current = child.clone();
                    continue;
                }
                let tree_entries = self
                    .run_with_retry_shell(consumer, |state| {
                        let owner = owner.to_string();
                        let repo = repo.to_string();
                        let tree_sha = current.clone();
                        async move {
                            state
                                .client
                                .fetch_tree_non_recursive(consumer, &owner, &repo, &tree_sha)
                                .await
                        }
                    })
                    .await?;
                let next = tree_entries.iter().find(|e| e.path == *seg);
                let Some(entry) = next else {
                    walk_ok = false;
                    break;
                };
                subtree_cache.insert(cache_key, entry.sha.clone());
                current = entry.sha.clone();
            }
            if !walk_ok {
                continue; // path renamed at this commit; skip
            }
            out.push(TreeCommit::new(current.clone(), commit.committed_at));
            // Short-circuit when every expected SHA is bound.
            if !expected_shas.is_empty()
                && expected_shas
                    .iter()
                    .all(|want| out.iter().any(|tc| &tc.tree_sha_at_path == want))
            {
                break;
            }
        }
        Ok(out)
    }
}

impl GitHubProviderRuntime {
    /// Retry shell mirroring `fetch_repository_tree`: per-call cooldown wait,
    /// permit acquisition, metrics, and per-variant `RuntimeRequestError` ladder.
    /// The per-variant arms must remain identical to `fetch_repository_tree`'s
    /// arms to preserve observability parity.
    async fn run_with_retry_shell<T, F, Fut>(
        &self,
        consumer: GlobalProviderConsumerId,
        mut call: F,
    ) -> Result<T, GitHubProviderError>
    where
        F: FnMut(Arc<GitHubProviderRuntimeState>) -> Fut,
        Fut: std::future::Future<Output = Result<T, RuntimeRequestError>>,
    {
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

            match call(Arc::clone(&state)).await {
                Ok(value) => {
                    self.metrics.record_request(consumer, "success");
                    return Ok(value);
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

    async fn list_commits_for_path_page(
        &self,
        consumer: GlobalProviderConsumerId,
        owner: &str,
        repo: &str,
        path: &str,
        per_page: usize,
        page: usize,
    ) -> Result<Vec<CommitItem>, RuntimeRequestError>;

    async fn fetch_tree_non_recursive(
        &self,
        consumer: GlobalProviderConsumerId,
        owner: &str,
        repo: &str,
        tree_sha: &str,
    ) -> Result<Vec<TreeEntry>, RuntimeRequestError>;
}

/// Newest-first commit-by-path projection. Fields parsed from the GitHub
/// `/repos/{owner}/{repo}/commits` response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommitItem {
    pub(crate) root_tree_sha: String,
    pub(crate) committed_at: time::OffsetDateTime,
}

/// Non-recursive tree-entry projection. Filtered to `type = "tree"` rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TreeEntry {
    pub(crate) path: String,
    pub(crate) sha: String,
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
        .map_err(|error| GitHubProviderError::Misconfigured(error.to_string()))?;

        let base_url = defaults
            .and_then(|value| value.api_base_url.as_deref())
            .unwrap_or(DEFAULT_GITHUB_API_BASE_URL);
        let base_url = url::Url::parse(base_url)
            .map_err(|error| GitHubProviderError::Misconfigured(error.to_string()))?;

        let auth = match defaults.and_then(|value| value.auth_token.clone()) {
            Some(token) => GitHubAuth::BearerToken(uptrakit_wire::SecretString::new(token)),
            None => GitHubAuth::Anonymous,
        };

        let config =
            GitHubClientConfig::new(http_client, base_url, auth, GITHUB_PROVIDER_USER_AGENT);
        let client = GitHubClient::new(config.clone());

        Ok(Arc::new(ReqwestGitHubRequestExecutor { client, config }))
    }
}

struct ReqwestGitHubRequestExecutor {
    client: GitHubClient,
    config: GitHubClientConfig,
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

    async fn list_commits_for_path_page(
        &self,
        _consumer: GlobalProviderConsumerId,
        owner: &str,
        repo: &str,
        path: &str,
        per_page: usize,
        page: usize,
    ) -> Result<Vec<CommitItem>, RuntimeRequestError> {
        let mut url = self.config.base_url.clone();
        url.path_segments_mut()
            .map_err(|()| {
                RuntimeRequestError::Misconfigured(
                    "base_url cannot be used as a path base".to_string(),
                )
            })?
            .push("repos")
            .push(owner)
            .push(repo)
            .push("commits");
        url.query_pairs_mut()
            .append_pair("path", path)
            .append_pair("per_page", &per_page.to_string())
            .append_pair("page", &page.to_string());

        let body = self.run_get_json(url).await?;
        let commits: Vec<CommitDto> = serde_json::from_str(&body)
            .map_err(|error| RuntimeRequestError::RequestFailed(error.to_string()))?;
        commits
            .into_iter()
            .map(CommitDto::into_model)
            .collect::<Result<Vec<_>, _>>()
    }

    async fn fetch_tree_non_recursive(
        &self,
        _consumer: GlobalProviderConsumerId,
        owner: &str,
        repo: &str,
        tree_sha: &str,
    ) -> Result<Vec<TreeEntry>, RuntimeRequestError> {
        let mut url = self.config.base_url.clone();
        url.path_segments_mut()
            .map_err(|()| {
                RuntimeRequestError::Misconfigured(
                    "base_url cannot be used as a path base".to_string(),
                )
            })?
            .push("repos")
            .push(owner)
            .push(repo)
            .push("git")
            .push("trees")
            .push(tree_sha);

        let body = self.run_get_json(url).await?;
        let parsed: TreeListDto = serde_json::from_str(&body)
            .map_err(|error| RuntimeRequestError::RequestFailed(error.to_string()))?;
        Ok(parsed
            .tree
            .into_iter()
            .filter(|entry| entry.kind == "tree")
            .map(|entry| TreeEntry {
                path: entry.path,
                sha: entry.sha,
            })
            .collect())
    }
}

impl ReqwestGitHubRequestExecutor {
    /// Issue a GET against `url` using the configured auth/user-agent headers, then
    /// classify HTTP/transport failures via the shared `classify_http_failure_with_auth`
    /// ladder so 404/401/403/429/5xx mapping stays identical to `GitHubClient`.
    async fn run_get_json(&self, url: url::Url) -> Result<String, RuntimeRequestError> {
        let auth_kind = match &self.config.auth {
            GitHubAuth::Anonymous => AuthKind::Anonymous,
            GitHubAuth::BearerToken(_) => AuthKind::Bearer,
            other => {
                tracing::warn!(
                    auth = ?std::mem::discriminant(other),
                    "unknown GitHubAuth variant; defaulting to Anonymous"
                );
                AuthKind::Anonymous
            }
        };

        let mut builder = self
            .config
            .http_client
            .get(url)
            .header(http::header::USER_AGENT, &self.config.user_agent)
            .header(http::header::ACCEPT, "application/vnd.github+json")
            .header("x-github-api-version", "2022-11-28");
        if let GitHubAuth::BearerToken(token) = &self.config.auth {
            builder = builder.bearer_auth(token.expose_secret());
        }

        let response =
            builder
                .send()
                .await
                .map_err(|error| RuntimeRequestError::UpstreamUnavailable {
                    message: error.to_string(),
                    retry_after: None,
                })?;
        let status = response.status();
        let headers = response.headers().clone();
        let body = response
            .text()
            .await
            .map_err(|error| RuntimeRequestError::RequestFailed(error.to_string()))?;

        if status.is_success() {
            return Ok(body);
        }

        let (error, decision, metadata) =
            classify_http_failure_with_auth(status, &headers, &body, auth_kind)
                .map_err(|error| map_client_failure(error, None, None))?;
        Err(map_client_failure(error, Some(decision), Some(metadata)))
    }
}

#[derive(Debug, serde::Deserialize)]
struct CommitDto {
    commit: CommitMetaDto,
}

#[derive(Debug, serde::Deserialize)]
struct CommitMetaDto {
    committer: CommitterDto,
    tree: CommitTreeDto,
}

#[derive(Debug, serde::Deserialize)]
struct CommitterDto {
    date: String,
}

#[derive(Debug, serde::Deserialize)]
struct CommitTreeDto {
    sha: String,
}

impl CommitDto {
    fn into_model(self) -> Result<CommitItem, RuntimeRequestError> {
        let committed_at = time::OffsetDateTime::parse(&self.commit.committer.date, &Rfc3339)
            .map_err(|error| {
                RuntimeRequestError::RequestFailed(format!(
                    "invalid committer date {:?}: {error}",
                    self.commit.committer.date
                ))
            })?;
        Ok(CommitItem {
            root_tree_sha: self.commit.tree.sha,
            committed_at,
        })
    }
}

#[derive(Debug, serde::Deserialize)]
struct TreeListDto {
    tree: Vec<TreeListEntryDto>,
}

#[derive(Debug, serde::Deserialize)]
struct TreeListEntryDto {
    path: String,
    #[serde(rename = "type")]
    kind: String,
    sha: String,
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
                sha: entry.sha,
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
            sha: "aabbcc1122334455667788aabbcc1122334455aa".to_string(),
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

    async fn list_commits_for_path_page(
        &self,
        _consumer: GlobalProviderConsumerId,
        _owner: &str,
        _repo: &str,
        _path: &str,
        _per_page: usize,
        _page: usize,
    ) -> Result<Vec<CommitItem>, RuntimeRequestError> {
        Err(RuntimeRequestError::RequestFailed(
            "test fake: list_commits_for_path_page not configured".to_string(),
        ))
    }

    async fn fetch_tree_non_recursive(
        &self,
        _consumer: GlobalProviderConsumerId,
        _owner: &str,
        _repo: &str,
        _tree_sha: &str,
    ) -> Result<Vec<TreeEntry>, RuntimeRequestError> {
        Err(RuntimeRequestError::RequestFailed(
            "test fake: fetch_tree_non_recursive not configured".to_string(),
        ))
    }
}

#[cfg(test)]
mod list_recent_commit_dates_for_path_tests {
    use std::collections::HashSet;
    use std::sync::Arc;

    use httpmock::prelude::*;
    use uptrakit_github_client::{GitHubAuth, GitHubClient, GitHubClientConfig};
    use uptrakit_global_github_provider::{GitHubProviderClient, GitHubProviderError};
    use uptrakit_plugin_infrastructure_registry::{
        PluginHttpClientConfig, build_plugin_http_client,
    };
    use uptrakit_shared_db::provider_settings::GitHubProviderDefaults;

    use super::{
        GITHUB_PROVIDER_USER_AGENT, GitHubClientFactory, GitHubProviderRuntime,
        GitHubProviderRuntimeOptions, GitHubRequestExecutor, ReqwestGitHubRequestExecutor,
        runtime_test_db,
    };

    /// Test factory that builds a real `ReqwestGitHubRequestExecutor` pointed at the
    /// mock server's `base_url`, bypassing the HTTPS-only validation enforced by
    /// `normalize_github_provider_defaults` on the upsert path.
    struct MockServerFactory {
        base_url: url::Url,
    }

    impl GitHubClientFactory for MockServerFactory {
        fn build(
            &self,
            _defaults: Option<&GitHubProviderDefaults>,
        ) -> Result<Arc<dyn GitHubRequestExecutor>, GitHubProviderError> {
            let http_client = build_plugin_http_client(PluginHttpClientConfig {
                user_agent: GITHUB_PROVIDER_USER_AGENT,
                ..PluginHttpClientConfig::default()
            })
            .map_err(|error| GitHubProviderError::Misconfigured(error.to_string()))?;
            // Use anonymous auth so 404 responses map to `NotFound` → `RequestFailed`
            // through `classify_http_failure_with_auth`. With bearer auth the same
            // ladder remaps to `AuthFailed`, which is correct production behaviour
            // but obscures the 404 marker the test exercises.
            let config = GitHubClientConfig::new(
                http_client,
                self.base_url.clone(),
                GitHubAuth::Anonymous,
                GITHUB_PROVIDER_USER_AGENT,
            );
            let client = GitHubClient::new(config.clone());
            Ok(Arc::new(ReqwestGitHubRequestExecutor { client, config }))
        }
    }

    /// Build a runtime wired to a real `ReqwestGitHubRequestExecutor` whose
    /// `base_url` points at the supplied httpmock server.
    async fn make_runtime_for_test(server: &MockServer) -> Arc<GitHubProviderRuntime> {
        let db = runtime_test_db().await;
        let base_url = url::Url::parse(&server.base_url()).expect("valid mock server URL");
        GitHubProviderRuntime::new_for_tests(
            db,
            Arc::new(MockServerFactory { base_url }),
            GitHubProviderRuntimeOptions::default(),
        )
    }

    #[tokio::test]
    async fn list_recent_commit_dates_for_path_walks_commits_then_trees() {
        let server = httpmock::MockServer::start_async().await;

        // 1 page of 2 commits touching skills/foo.
        server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET)
                    .path("/repos/o/r/commits")
                    .query_param("path", "skills/foo")
                    .query_param("per_page", "30")
                    .query_param("page", "1");
                then.status(200).json_body(serde_json::json!([
                    {
                        "sha": "c2",
                        "commit": {
                            "committer": {"date": "2026-06-11T01:15:00Z"},
                            "tree": {"sha": "root_c2"}
                        }
                    },
                    {
                        "sha": "c1",
                        "commit": {
                            "committer": {"date": "2026-05-01T08:00:00Z"},
                            "tree": {"sha": "root_c1"}
                        }
                    }
                ]));
            })
            .await;
        server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET)
                    .path("/repos/o/r/commits")
                    .query_param("page", "2");
                then.status(200).json_body(serde_json::json!([]));
            })
            .await;

        // Non-recursive tree calls (per commit) — each returns one level.
        server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET)
                    .path("/repos/o/r/git/trees/root_c1");
                then.status(200).json_body(serde_json::json!({
                    "tree": [{"path": "skills", "type": "tree", "sha": "skills_c1"}]
                }));
            })
            .await;
        server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET)
                    .path("/repos/o/r/git/trees/skills_c1");
                then.status(200).json_body(serde_json::json!({
                    "tree": [{"path": "foo", "type": "tree", "sha": "foo_c1"}]
                }));
            })
            .await;
        server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET)
                    .path("/repos/o/r/git/trees/root_c2");
                then.status(200).json_body(serde_json::json!({
                    "tree": [{"path": "skills", "type": "tree", "sha": "skills_c2"}]
                }));
            })
            .await;
        server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET)
                    .path("/repos/o/r/git/trees/skills_c2");
                then.status(200).json_body(serde_json::json!({
                    "tree": [{"path": "foo", "type": "tree", "sha": "foo_c2"}]
                }));
            })
            .await;

        let runtime = make_runtime_for_test(&server).await;
        let out = runtime
            .list_recent_commit_dates_for_path(
                uptrakit_global_github_provider::PACKAGE_MANAGER_SKILLS,
                "o",
                "r",
                "skills/foo",
                90,
                &HashSet::new(),
            )
            .await
            .expect("ok");

        // Oldest-first ordering.
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].tree_sha_at_path, "foo_c1");
        assert_eq!(out[1].tree_sha_at_path, "foo_c2");
    }

    #[tokio::test]
    async fn list_recent_commit_dates_for_path_short_circuits_on_expected_shas() {
        let server = httpmock::MockServer::start_async().await;

        server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET)
                    .path("/repos/o/r/commits")
                    .query_param("page", "1");
                then.status(200).json_body(serde_json::json!([
                    {"sha":"c2","commit":{"committer":{"date":"2026-06-11T01:15:00Z"},"tree":{"sha":"root_c2"}}},
                    {"sha":"c1","commit":{"committer":{"date":"2026-05-01T08:00:00Z"},"tree":{"sha":"root_c1"}}}
                ]));
            })
            .await;
        // ONLY define tree mocks for c1 (oldest). If the walker hits c2's trees the
        // test fails.
        server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET)
                    .path("/repos/o/r/git/trees/root_c1");
                then.status(200).json_body(serde_json::json!({
                    "tree": [{"path":"skills","type":"tree","sha":"skills_c1"}]
                }));
            })
            .await;
        server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET)
                    .path("/repos/o/r/git/trees/skills_c1");
                then.status(200).json_body(serde_json::json!({
                    "tree": [{"path":"foo","type":"tree","sha":"foo_c1"}]
                }));
            })
            .await;

        let runtime = make_runtime_for_test(&server).await;
        let mut expected: HashSet<String> = HashSet::new();
        expected.insert("foo_c1".to_string());

        let out = runtime
            .list_recent_commit_dates_for_path(
                uptrakit_global_github_provider::PACKAGE_MANAGER_SKILLS,
                "o",
                "r",
                "skills/foo",
                90,
                &expected,
            )
            .await
            .expect("ok");

        assert_eq!(
            out.len(),
            1,
            "must short-circuit after binding all expected"
        );
        assert_eq!(out[0].tree_sha_at_path, "foo_c1");
    }

    #[tokio::test]
    async fn list_recent_commit_dates_for_path_404_surfaces_with_marker() {
        let server = httpmock::MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET)
                    .path("/repos/o/r/commits");
                then.status(404);
            })
            .await;
        let runtime = make_runtime_for_test(&server).await;
        let err = runtime
            .list_recent_commit_dates_for_path(
                uptrakit_global_github_provider::PACKAGE_MANAGER_SKILLS,
                "o",
                "r",
                "skills/foo",
                90,
                &HashSet::new(),
            )
            .await
            .unwrap_err();
        // The existing `fetch_repository_tree` ladder maps
        // `RuntimeRequestError::NotFound` to `GitHubProviderError::RequestFailed`.
        // Assert on the 404 marker substring so the downstream dispatcher can pick
        // up the `upstream_gone` tag.
        let msg = err.to_string();
        assert!(msg.contains("404"), "404 marker missing from error: {msg}");
        assert!(
            matches!(err, GitHubProviderError::RequestFailed(_)),
            "expected RequestFailed, got: {err:?}"
        );
    }
}
