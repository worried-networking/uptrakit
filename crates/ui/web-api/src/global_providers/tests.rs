use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use uptrakit_github_client::{AuthKind, GitHubClientError, ResponseMetadata};
use uptrakit_global_github_provider::{
    DASHBOARD_ICONS, GitHubProviderError, GitHubRepositoryTree, GitHubTreeEntry,
    GitHubTreeEntryKind,
};
use uptrakit_web_api_types::events::AdminEvent;

use super::github::{
    GitHubClientFactory, GitHubProviderRuntime, GitHubProviderRuntimeOptions,
    GitHubRequestExecutor, RuntimeRequestError, emit_global_github_provider_diagnostic_if_needed,
    map_client_failure_for_tests,
};

struct RecordingFactory;

impl GitHubClientFactory for RecordingFactory {
    fn build(
        &self,
        _defaults: Option<&uptrakit_shared_db::provider_settings::GitHubProviderDefaults>,
    ) -> Result<Arc<dyn GitHubRequestExecutor>, GitHubProviderError> {
        Ok(Arc::new(FakeExecutor::success()))
    }
}

struct FakeExecutor {
    result: Mutex<Result<GitHubRepositoryTree, RuntimeRequestError>>,
}

impl FakeExecutor {
    fn success() -> Self {
        Self {
            result: Mutex::new(Ok(GitHubRepositoryTree {
                truncated: false,
                entries: vec![GitHubTreeEntry {
                    path: "svg/nginx.svg".to_string(),
                    kind: GitHubTreeEntryKind::Blob,
                }],
            })),
        }
    }
}

#[async_trait]
impl GitHubRequestExecutor for FakeExecutor {
    async fn fetch_repository_tree(
        &self,
        _consumer: uptrakit_global_github_provider::GlobalProviderConsumerId,
        _owner: &str,
        _repo: &str,
        _git_ref: &str,
        _recursive: bool,
    ) -> Result<GitHubRepositoryTree, RuntimeRequestError> {
        self.result.lock().clone()
    }
}

async fn test_db() -> DatabaseConnection {
    let _ = uptrakit_crypto::init_master_key(zeroize::Zeroizing::new([0x11u8; 32]));
    let mut options = ConnectOptions::new("sqlite::memory:".to_owned());
    options.max_connections(1).min_connections(1);
    let db = Database::connect(options).await.expect("db");
    uptrakit_shared_db::migration::run_migrations(&db)
        .await
        .expect("migrations");
    db
}

#[tokio::test]
async fn invalidation_rebuilds_client_generation() {
    let db = test_db().await;
    uptrakit_shared_db::provider_settings::upsert_github_provider_defaults(
        &db,
        Some("ghp_one"),
        None,
    )
    .await
    .expect("seed");
    let runtime = GitHubProviderRuntime::new_for_tests(
        db.clone(),
        Arc::new(RecordingFactory),
        GitHubProviderRuntimeOptions::default(),
    );

    let _first = runtime.github_client().await.expect("first client");
    let first_generation = runtime
        .cached_generation_for_tests()
        .expect("first generation");

    uptrakit_shared_db::provider_settings::upsert_github_provider_defaults(
        &db,
        Some("ghp_two"),
        None,
    )
    .await
    .expect("rotate");
    runtime.invalidate();

    let _second = runtime.github_client().await.expect("second client");
    let second_generation = runtime
        .cached_generation_for_tests()
        .expect("second generation");
    assert_ne!(first_generation, second_generation);
}

#[tokio::test]
async fn other_instances_recheck_generation_within_thirty_seconds() {
    let db = test_db().await;
    uptrakit_shared_db::provider_settings::upsert_github_provider_defaults(
        &db,
        Some("ghp_one"),
        None,
    )
    .await
    .expect("seed");
    let options = GitHubProviderRuntimeOptions {
        generation_recheck_interval: Duration::from_millis(10),
        queue_wait_timeout: Duration::from_secs(30),
        concurrency_limit: 8,
    };
    let writer =
        GitHubProviderRuntime::new_for_tests(db.clone(), Arc::new(RecordingFactory), options);
    let reader =
        GitHubProviderRuntime::new_for_tests(db.clone(), Arc::new(RecordingFactory), options);

    let _ = reader.github_client().await.expect("reader client");
    let first_generation = reader
        .cached_generation_for_tests()
        .expect("reader generation");

    uptrakit_shared_db::provider_settings::upsert_github_provider_defaults(
        &db,
        Some("ghp_two"),
        None,
    )
    .await
    .expect("rotate");
    writer.invalidate();

    tokio::time::sleep(Duration::from_millis(20)).await;
    let _ = reader
        .github_client()
        .await
        .expect("reader refreshed client");

    assert_ne!(
        first_generation,
        reader
            .cached_generation_for_tests()
            .expect("reader generation")
    );
}

#[tokio::test]
async fn queue_wait_times_out_as_throttled() {
    let runtime = GitHubProviderRuntime::new_for_tests(
        test_db().await,
        Arc::new(RecordingFactory),
        GitHubProviderRuntimeOptions {
            generation_recheck_interval: Duration::from_secs(30),
            queue_wait_timeout: Duration::from_millis(10),
            concurrency_limit: 1,
        },
    );
    let _held = runtime.acquire_request_permit_for_tests().await;

    let err = runtime
        .github_client()
        .await
        .expect("client")
        .fetch_repository_tree(DASHBOARD_ICONS, "o", "r", "main", true)
        .await
        .expect_err("queue wait should time out");
    assert_eq!(err, GitHubProviderError::Throttled);
}

#[tokio::test]
async fn startup_diagnostic_broadcasts_invalid_global_provider_record() {
    let db = test_db().await;
    uptrakit_shared_db::provider_settings::upsert_github_provider_defaults(
        &db,
        None,
        Some("https://ghe.example.com/api/v3"),
    )
    .await
    .expect("seed invalid record");

    let broadcaster = crate::event_broadcaster::EventBroadcaster::new();
    let mut rx = broadcaster.subscribe(uuid::Uuid::now_v7()).await;

    emit_global_github_provider_diagnostic_if_needed(&db, &broadcaster).await;

    let event = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("event should arrive")
        .expect("event should be present");
    assert!(matches!(
        event,
        AdminEvent::GlobalGitHubProviderMisconfigured { ref problem }
            if problem.contains("api_base_url requires auth_token")
    ));
}

struct SequenceFactory {
    results: Arc<Mutex<VecDeque<Result<GitHubRepositoryTree, RuntimeRequestError>>>>,
}

impl SequenceFactory {
    fn new(
        results: impl IntoIterator<Item = Result<GitHubRepositoryTree, RuntimeRequestError>>,
    ) -> Self {
        Self {
            results: Arc::new(Mutex::new(results.into_iter().collect())),
        }
    }
}

impl GitHubClientFactory for SequenceFactory {
    fn build(
        &self,
        _defaults: Option<&uptrakit_shared_db::provider_settings::GitHubProviderDefaults>,
    ) -> Result<Arc<dyn GitHubRequestExecutor>, GitHubProviderError> {
        Ok(Arc::new(SequenceExecutor {
            results: Arc::clone(&self.results),
        }))
    }
}

struct SequenceExecutor {
    results: Arc<Mutex<VecDeque<Result<GitHubRepositoryTree, RuntimeRequestError>>>>,
}

#[async_trait]
impl GitHubRequestExecutor for SequenceExecutor {
    async fn fetch_repository_tree(
        &self,
        _consumer: uptrakit_global_github_provider::GlobalProviderConsumerId,
        _owner: &str,
        _repo: &str,
        _git_ref: &str,
        _recursive: bool,
    ) -> Result<GitHubRepositoryTree, RuntimeRequestError> {
        self.results
            .lock()
            .pop_front()
            .unwrap_or_else(|| FakeExecutor::success().result.lock().clone())
    }
}

#[tokio::test]
async fn missing_global_credentials_builds_anonymous_public_client() {
    let runtime = GitHubProviderRuntime::new_for_tests(
        test_db().await,
        Arc::new(RecordingFactory),
        GitHubProviderRuntimeOptions::default(),
    );
    let client = runtime.github_client().await.expect("client");
    client
        .fetch_repository_tree(
            DASHBOARD_ICONS,
            "homarr-labs",
            "dashboard-icons",
            "main",
            true,
        )
        .await
        .expect("anonymous public request succeeds");
    assert_eq!(
        runtime.cached_credential_fingerprint_for_tests(),
        Some("anonymous".to_string())
    );
}

#[tokio::test]
async fn metrics_record_requests_retries_auth_failures_and_cooldowns() {
    let runtime = GitHubProviderRuntime::new_for_tests(
        test_db().await,
        Arc::new(SequenceFactory::new([
            Err(RuntimeRequestError::UpstreamUnavailable {
                message: "transient".to_string(),
                retry_after: None,
            }),
            FakeExecutor::success().result.lock().clone(),
            Err(RuntimeRequestError::AuthFailed(
                "bad credentials".to_string(),
            )),
            Err(RuntimeRequestError::Throttled {
                retry_after: Some(Duration::from_secs(5)),
            }),
            FakeExecutor::success().result.lock().clone(),
        ])),
        GitHubProviderRuntimeOptions::default(),
    );
    let client = runtime.github_client().await.expect("client");
    client
        .fetch_repository_tree(DASHBOARD_ICONS, "o", "r", "main", true)
        .await
        .expect("retry should recover");
    let _ = client
        .fetch_repository_tree(DASHBOARD_ICONS, "o", "r", "main", true)
        .await;
    let _ = client
        .fetch_repository_tree(DASHBOARD_ICONS, "o", "r", "main", true)
        .await;

    let snapshot = runtime.metrics_snapshot_for_tests();
    assert_eq!(snapshot.requests, 10);
    assert_eq!(snapshot.retries, 2);
    assert_eq!(snapshot.auth_failures, 1);
    assert!((4..=5).contains(&snapshot.cooldown_seconds));
}

#[tokio::test]
async fn cooldown_is_shared_across_two_global_consumers() {
    let runtime = GitHubProviderRuntime::new_for_tests(
        test_db().await,
        Arc::new(SequenceFactory::new([
            Err(RuntimeRequestError::Throttled {
                retry_after: Some(Duration::from_secs(5)),
            }),
            FakeExecutor::success().result.lock().clone(),
            FakeExecutor::success().result.lock().clone(),
        ])),
        GitHubProviderRuntimeOptions::default(),
    );
    let client = runtime.github_client().await.expect("client");
    client
        .fetch_repository_tree(DASHBOARD_ICONS, "o", "r", "main", true)
        .await
        .expect("throttled request should recover");
    let last_sleep = runtime
        .last_sleep_for_tests()
        .expect("sleep should be recorded");
    assert!(last_sleep >= Duration::from_millis(4900));
    assert!(last_sleep <= Duration::from_secs(5));
    client
        .fetch_repository_tree(
            uptrakit_global_github_provider::GlobalProviderConsumerId::new("test-global"),
            "o",
            "r",
            "main",
            true,
        )
        .await
        .expect("second consumer should reuse the same shared runtime");
}

#[tokio::test]
async fn runtime_maps_not_found_to_request_failed() {
    let runtime = GitHubProviderRuntime::new_for_tests(
        test_db().await,
        Arc::new(SequenceFactory::new([Err(RuntimeRequestError::NotFound(
            "404: Not Found".to_string(),
        ))])),
        GitHubProviderRuntimeOptions::default(),
    );
    let client = runtime.github_client().await.expect("client");
    let err = client
        .fetch_repository_tree(DASHBOARD_ICONS, "o", "r", "main", true)
        .await
        .expect_err("not found should map to request failed");
    assert!(matches!(
        err,
        GitHubProviderError::RequestFailed(message) if message.contains("404")
    ));
}

#[test]
fn authenticated_not_found_is_promoted_to_auth_failed() {
    let err = map_client_failure_for_tests(
        GitHubClientError::NotFound("404: Not Found".to_string()),
        None,
        Some(ResponseMetadata {
            status: Some(http::StatusCode::NOT_FOUND),
            auth_kind: AuthKind::Bearer,
            authenticated_not_found: true,
            rate_limit_remaining: None,
        }),
    );

    assert!(matches!(err, RuntimeRequestError::AuthFailed(message) if message.contains("404")));
}

#[tokio::test]
async fn runtime_maps_non_rate_limit_403_to_auth_failed() {
    let runtime = GitHubProviderRuntime::new_for_tests(
        test_db().await,
        Arc::new(SequenceFactory::new([Err(
            RuntimeRequestError::AuthFailed(
                "403: Resource not accessible by integration".to_string(),
            ),
        )])),
        GitHubProviderRuntimeOptions::default(),
    );
    let client = runtime.github_client().await.expect("client");
    let err = client
        .fetch_repository_tree(DASHBOARD_ICONS, "o", "r", "main", true)
        .await
        .expect_err("non-rate-limit forbidden should map to auth failed");
    assert!(matches!(
        err,
        GitHubProviderError::AuthFailed(message) if message.contains("403")
    ));
}

#[tokio::test]
async fn upstream_unavailable_retry_after_updates_cooldown_without_throttled_mapping() {
    let runtime = GitHubProviderRuntime::new_for_tests(
        test_db().await,
        Arc::new(SequenceFactory::new([
            Err(RuntimeRequestError::UpstreamUnavailable {
                message: "503: Service Unavailable".to_string(),
                retry_after: Some(Duration::from_secs(5)),
            }),
            Err(RuntimeRequestError::UpstreamUnavailable {
                message: "503: Service Unavailable".to_string(),
                retry_after: Some(Duration::from_secs(5)),
            }),
            Err(RuntimeRequestError::UpstreamUnavailable {
                message: "503: Service Unavailable".to_string(),
                retry_after: Some(Duration::from_secs(5)),
            }),
        ])),
        GitHubProviderRuntimeOptions::default(),
    );
    let client = runtime.github_client().await.expect("client");
    let err = client
        .fetch_repository_tree(DASHBOARD_ICONS, "o", "r", "main", true)
        .await
        .expect_err("retry budget should return upstream unavailable");
    assert!(matches!(
        err,
        GitHubProviderError::UpstreamUnavailable(message) if message.contains("503")
    ));
    let last_sleep = runtime
        .last_sleep_for_tests()
        .expect("sleep should be recorded");
    assert!(last_sleep >= Duration::from_millis(4900));
    assert!(last_sleep <= Duration::from_secs(5));
    assert!((4..=5).contains(&runtime.metrics_snapshot_for_tests().cooldown_seconds));
}

#[tokio::test]
async fn rate_limited_backoff_uses_runtime_backoff_without_shared_cooldown() {
    let runtime = GitHubProviderRuntime::new_for_tests(
        test_db().await,
        Arc::new(SequenceFactory::new([
            Err(RuntimeRequestError::Throttled { retry_after: None }),
            FakeExecutor::success().result.lock().clone(),
        ])),
        GitHubProviderRuntimeOptions::default(),
    );
    let client = runtime.github_client().await.expect("client");
    client
        .fetch_repository_tree(DASHBOARD_ICONS, "o", "r", "main", true)
        .await
        .expect("backoff retry should recover");

    assert_eq!(
        runtime.last_sleep_for_tests(),
        Some(Duration::from_millis(500))
    );
    assert_eq!(runtime.metrics_snapshot_for_tests().cooldown_seconds, 0);
}

#[tokio::test]
async fn shorter_retry_after_does_not_shrink_existing_shared_cooldown() {
    let runtime = GitHubProviderRuntime::new_for_tests(
        test_db().await,
        Arc::new(RecordingFactory),
        GitHubProviderRuntimeOptions::default(),
    );

    runtime.set_cooldown_for_tests(Duration::from_secs(30));
    let initial = runtime
        .remaining_cooldown_for_tests()
        .expect("initial cooldown should exist");

    runtime.set_cooldown_for_tests(Duration::from_secs(5));
    let after_shorter_update = runtime
        .remaining_cooldown_for_tests()
        .expect("cooldown should still exist");

    assert!(after_shorter_update >= initial.saturating_sub(Duration::from_millis(100)));
}

#[tokio::test]
async fn waiter_does_not_clear_cooldown_extended_by_later_response() {
    let runtime = GitHubProviderRuntime::new_for_tests(
        test_db().await,
        Arc::new(RecordingFactory),
        GitHubProviderRuntimeOptions::default(),
    );

    runtime.set_cooldown_for_tests(Duration::from_millis(20));
    let waiter_runtime = Arc::clone(&runtime);
    let waiter = tokio::spawn(async move {
        waiter_runtime
            .wait_for_cooldown_for_tests("github")
            .await
            .expect("wait should succeed");
    });

    tokio::time::sleep(Duration::from_millis(5)).await;
    runtime.set_cooldown_for_tests(Duration::from_millis(40));
    waiter.await.expect("waiter should join");

    let remaining = runtime
        .remaining_cooldown_for_tests()
        .expect("extended cooldown should remain");
    assert!(remaining >= Duration::from_millis(15));
}
