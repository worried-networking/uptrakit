use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::join_all;
use uptrakit_backoff::Backoff;
use uptrakit_command::CommandExecutor;
use uptrakit_internal_wire::{
    PluginAssignment, PluginTypeId, UpdateCategory, VersionCheckAssignment, VersionCheckResult,
};
use uptrakit_plugin_infrastructure_core::{
    BatchDetectItem, BatchFetchItem, HostCapabilities, HostRuntime, PluginError,
    construct_host_runtime,
};
use uptrakit_plugin_infrastructure_registry::{
    PluginCapability, PluginResult, ReleaseFetcher, get_descriptor,
};

use crate::connection_context::ConnectionContext;

/// Base delay between retry attempts for transient version check errors.
const RETRY_BASE_DELAY: Duration = Duration::from_secs(5);
/// Maximum delay between retry attempts.
const RETRY_MAX_DELAY: Duration = Duration::from_secs(20);
/// Maximum number of retries (3 total attempts).
const MAX_RETRIES: u32 = 2;

/// Result of a version check for a single software item.
pub struct VersionCheckOutcome {
    /// Detected installed version, if any.
    pub installed_version: Option<String>,
    /// Latest available version from the local package index, if the plugin
    /// supports agent-side release fetching.
    pub latest_version: Option<String>,
    /// Error message if detection failed.
    pub error: Option<String>,
    /// Classification of the available update (e.g. security, bugfix).
    pub update_category: UpdateCategory,
}

/// Check the installed version (and optionally the latest version) for a
/// software item using role-based plugin assignments.
///
/// - `detect`: plugin assignment for detecting the installed version. If `None`,
///   `installed_version` will be `None` in the outcome.
/// - `fetch`: plugin assignment for fetching the latest available version from
///   a local package index. If `None`, `latest_version` will be `None`.
///
/// The `ctx` parameter is used to inject connection-specific overrides (e.g.
/// a remote Docker host for the SSH agent) into the plugin config before
/// instantiation.
pub async fn check_version(
    detect: Option<&PluginAssignment>,
    fetch: Option<&PluginAssignment>,
    executor: Arc<dyn CommandExecutor>,
    ctx: &ConnectionContext,
) -> VersionCheckOutcome {
    let installed_version = if let Some(assignment) = detect {
        detect_installed(assignment, Arc::clone(&executor), ctx).await
    } else {
        Ok(None)
    };

    let (installed_version, detect_error) = match installed_version {
        Ok(v) => (v, None),
        Err(e) => (None, Some(e)),
    };

    let latest_result = if let Some(assignment) = fetch {
        fetch_latest(assignment, Arc::clone(&executor), ctx).await
    } else {
        Ok(None)
    };

    let (latest_version, update_category, fetch_error) = match latest_result {
        Ok(Some((version, category))) => (Some(version), category, None),
        Ok(None) => (None, UpdateCategory::Unknown, None),
        Err(e) => (None, UpdateCategory::Unknown, Some(e)),
    };

    // Combine errors if both roles failed.
    let error = match (detect_error, fetch_error) {
        (Some(d), Some(f)) => Some(format!("detect: {d}; fetch: {f}")),
        (Some(d), None) => Some(d),
        (None, Some(f)) => Some(f),
        (None, None) => None,
    };

    VersionCheckOutcome {
        installed_version,
        latest_version,
        error,
        update_category,
    }
}

/// Per-item detect result: `(installed_version, display_version, error)`.
type DetectItemResult = (Option<String>, Option<String>, Option<String>);

/// Per-item fetch result: `(latest_version, update_category, error)`.
type FetchItemResult = (Option<String>, UpdateCategory, Option<String>);

/// A group of assignments sharing the same plugin type and effective config,
/// ready for a single batch plugin invocation.
struct BatchGroup {
    plugin_type: PluginTypeId,
    effective_config: serde_json::Value,
    /// Each entry is `(assignment_index, package_identifier)`.
    items: Vec<(usize, String)>,
}

/// Group key: `(plugin_type, serialised effective config)`.
type GroupKey = (PluginTypeId, String);

type FetcherFactory = dyn Fn(
        &PluginTypeId,
        &serde_json::Value,
        Arc<dyn HostRuntime>,
    ) -> PluginResult<Box<dyn ReleaseFetcher>>
    + Send
    + Sync;

/// Build detect and fetch groups from assignments, keyed by
/// `(PluginTypeId, effective_config_json)`.
fn build_batch_groups(
    assignments: &[VersionCheckAssignment],
    ctx: &ConnectionContext,
) -> (HashMap<GroupKey, BatchGroup>, HashMap<GroupKey, BatchGroup>) {
    let mut detect_groups: HashMap<GroupKey, BatchGroup> = HashMap::new();
    let mut fetch_groups: HashMap<GroupKey, BatchGroup> = HashMap::new();

    for (idx, assignment) in assignments.iter().enumerate() {
        if let Some(pa) = &assignment.detect_version {
            insert_into_group(&mut detect_groups, pa, idx, ctx);
        }
        if let Some(pa) = &assignment.fetch_releases {
            insert_into_group(&mut fetch_groups, pa, idx, ctx);
        }
    }

    (detect_groups, fetch_groups)
}

/// Insert a single plugin assignment into the appropriate group map.
fn insert_into_group(
    groups: &mut HashMap<GroupKey, BatchGroup>,
    pa: &PluginAssignment,
    idx: usize,
    ctx: &ConnectionContext,
) {
    let mut effective_config = pa.config.clone();
    ctx.apply_to_config(&pa.plugin_type, &mut effective_config);
    let key = (pa.plugin_type.clone(), effective_config.to_string());
    let group = groups.entry(key).or_insert_with(|| BatchGroup {
        plugin_type: pa.plugin_type.clone(),
        effective_config,
        items: vec![],
    });
    group.items.push((idx, pa.package_identifier.clone()));
}

/// Produce an error result for every item in a group.
fn error_for_all_detect_items(
    items: &[(usize, String)],
    err: String,
) -> Vec<(usize, DetectItemResult)> {
    items
        .iter()
        .map(|(idx, _)| (*idx, (None, None, Some(err.clone()))))
        .collect()
}

/// Produce an error result for every item in a fetch group.
fn error_for_all_fetch_items(
    items: &[(usize, String)],
    err: String,
) -> Vec<(usize, FetchItemResult)> {
    items
        .iter()
        .map(|(idx, _)| (*idx, (None, UpdateCategory::Unknown, Some(err.clone()))))
        .collect()
}

/// Run a single detect group: create the plugin and call
/// `batch_detect_installed_version`.
///
/// Returns `(assignment_index, (installed_version, error))` for each item.
async fn run_detect_group(
    group: BatchGroup,
    executor: Arc<dyn CommandExecutor>,
) -> Vec<(usize, DetectItemResult)> {
    let batch_items: Vec<BatchDetectItem> = group
        .items
        .iter()
        .map(|(_, pkg)| BatchDetectItem::new(pkg.clone()))
        .collect();

    let runtime = construct_host_runtime(executor, HostCapabilities::default());

    let desc = match get_descriptor(group.plugin_type.as_str()) {
        Some(d) => d,
        None => {
            return error_for_all_detect_items(
                &group.items,
                format!("unknown plugin type: {}", group.plugin_type),
            );
        }
    };

    let slot = match desc.roles.version_detector.as_ref() {
        Some(s) => s,
        None => {
            return error_for_all_detect_items(
                &group.items,
                format!(
                    "plugin {} does not implement VersionDetectorPlugin",
                    group.plugin_type
                ),
            );
        }
    };

    let detector = match (slot.create)(&group.effective_config, runtime) {
        Ok(d) => d,
        Err(e) => {
            return error_for_all_detect_items(
                &group.items,
                format!("failed to create plugin: {e}"),
            );
        }
    };

    let results = match detector.batch_detect(&batch_items).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                plugin_type = %group.plugin_type,
                item_count = group.items.len(),
                error = %e,
                "batch detect failed"
            );
            return error_for_all_detect_items(&group.items, format!("detection failed: {e}"));
        }
    };

    let result_map: HashMap<String, DetectItemResult> = results
        .into_iter()
        .map(|r| {
            let version = r.installed_version.map(|v| v.to_string());
            let display_version = r.display_version;
            let error = r.error.map(|e| format!("detection failed: {e}"));
            (r.package_identifier, (version, display_version, error))
        })
        .collect();

    group
        .items
        .iter()
        .map(|(idx, pkg)| {
            let outcome = result_map
                .get(pkg.as_str())
                .cloned()
                .unwrap_or((None, None, None));
            (*idx, outcome)
        })
        .collect()
}

/// Run a single fetch group: create the plugin and call
/// `batch_fetch_releases`.
///
/// Returns `(assignment_index, (latest_version, update_category, error))` for
/// each item.
async fn run_fetch_group(
    group: BatchGroup,
    executor: Arc<dyn CommandExecutor>,
    fetcher_factory: &FetcherFactory,
) -> Vec<(usize, FetchItemResult)> {
    let batch_items: Vec<BatchFetchItem> = group
        .items
        .iter()
        .map(|(_, pkg)| BatchFetchItem::new(pkg.clone()))
        .collect();

    let runtime = construct_host_runtime(executor, HostCapabilities::default());

    let fetcher = match fetcher_factory(&group.plugin_type, &group.effective_config, runtime) {
        Ok(f) => f,
        Err(e) => {
            return error_for_all_fetch_items(
                &group.items,
                format!("failed to create plugin: {e}"),
            );
        }
    };

    let results = match fetcher.batch_fetch(&batch_items).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                plugin_type = %group.plugin_type,
                item_count = group.items.len(),
                error = %e,
                "batch fetch failed"
            );
            return error_for_all_fetch_items(&group.items, format!("fetch_releases failed: {e}"));
        }
    };

    let result_map: HashMap<String, FetchItemResult> = results
        .into_iter()
        .map(|r| {
            let error = r.error.map(|e| format!("fetch_releases failed: {e}"));
            let (version, category) = r
                .releases
                .first()
                .map(|release| {
                    let category = release.category.clone().unwrap_or_default();
                    (Some(release.version.to_string()), category)
                })
                .unwrap_or((None, UpdateCategory::Unknown));
            (r.package_identifier, (version, category, error))
        })
        .collect();

    group
        .items
        .iter()
        .map(|(idx, pkg)| {
            let outcome = result_map.get(pkg.as_str()).cloned().unwrap_or((
                None,
                UpdateCategory::Unknown,
                None,
            ));
            (*idx, outcome)
        })
        .collect()
}

fn default_fetcher_factory(
    plugin_type: &PluginTypeId,
    config: &serde_json::Value,
    runtime: Arc<dyn HostRuntime>,
) -> PluginResult<Box<dyn ReleaseFetcher>> {
    let desc = get_descriptor(plugin_type.as_str()).ok_or_else(|| {
        rootcause::report!(PluginError::Configuration(format!(
            "unknown plugin type: {plugin_type}"
        )))
    })?;

    let slot = desc.roles.release_fetcher.as_ref().ok_or_else(|| {
        rootcause::report!(PluginError::Configuration(format!(
            "plugin {plugin_type} does not implement ReleaseFetcherPlugin"
        )))
    })?;

    (slot.create)(config, runtime)
}

/// Refresh the package index for each unique fetch group.
///
/// Runs sequentially to avoid concurrent `apt-get update` / `brew update`.
async fn refresh_package_indexes(
    fetch_groups: &HashMap<GroupKey, BatchGroup>,
    executor: &Arc<dyn CommandExecutor>,
) {
    for group in fetch_groups.values() {
        let Some(desc) = get_descriptor(group.plugin_type.as_str()) else {
            continue;
        };

        if !desc
            .capabilities
            .contains(&PluginCapability::RefreshPackageIndex)
        {
            continue;
        }

        let Some(slot) = desc.roles.package_indexer.as_ref() else {
            continue;
        };

        let runtime = construct_host_runtime(Arc::clone(executor), HostCapabilities::default());

        match (slot.create)(&group.effective_config, runtime) {
            Ok(pkg_index) => {
                tracing::info!(
                    plugin_type = %group.plugin_type,
                    "refreshing package index"
                );
                if let Err(e) = pkg_index.refresh_package_index().await {
                    tracing::warn!(
                        plugin_type = %group.plugin_type,
                        error = %e,
                        "failed to refresh package index"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    plugin_type = %group.plugin_type,
                    error = %e,
                    "failed to create plugin for index refresh"
                );
            }
        }
    }
}

/// Merge detect and fetch results into a combined error string.
fn merge_errors(detect_error: Option<String>, fetch_error: Option<String>) -> Option<String> {
    match (detect_error, fetch_error) {
        (Some(d), Some(f)) => Some(format!("detect: {d}; fetch: {f}")),
        (Some(d), None) => Some(d),
        (None, Some(f)) => Some(f),
        (None, None) => None,
    }
}

/// Check installed versions and latest versions for a batch of software items,
/// using native batch operations when the plugin supports them.
///
/// Groups assignments by `(PluginTypeId, effective_config_json)` so that all
/// items sharing the same plugin configuration are checked in a single plugin
/// invocation. Plugins that override `batch_detect_installed_version` or
/// `batch_fetch_releases` (e.g. APT, Homebrew, npm) benefit from a single
/// subprocess call per group instead of N per-item calls.
///
/// `RefreshPackageIndex` is called at most once per unique fetch-group
/// (plugin_type, config) before `batch_fetch_releases` runs. It is not called
/// for detect-only groups.
async fn batch_check_versions_inner(
    assignments: Vec<VersionCheckAssignment>,
    executor: Arc<dyn CommandExecutor>,
    ctx: &ConnectionContext,
    fetcher_factory: &FetcherFactory,
) -> Vec<VersionCheckResult> {
    if assignments.is_empty() {
        return vec![];
    }

    // ── Step 1: Build detect and fetch groups ────────────────────────────────
    let (detect_groups, fetch_groups) = build_batch_groups(&assignments, ctx);

    // ── Step 2: Run detect groups in parallel ────────────────────────────────
    let detect_futs: Vec<_> = detect_groups
        .into_values()
        .map(|group| {
            tracing::trace!(
                plugin_type = %group.plugin_type,
                item_count = group.items.len(),
                "queuing detect_installed_version batch group"
            );
            run_detect_group(group, Arc::clone(&executor))
        })
        .collect();

    let mut detect_map: HashMap<usize, DetectItemResult> = HashMap::new();
    for group_results in join_all(detect_futs).await {
        for (idx, result) in group_results {
            detect_map.insert(idx, result);
        }
    }

    // ── Step 3: RefreshPackageIndex – at most once per unique fetch group ───
    refresh_package_indexes(&fetch_groups, &executor).await;

    // ── Step 4: Run fetch groups in parallel ─────────────────────────────────
    let fetch_futs: Vec<_> = fetch_groups
        .into_values()
        .map(|group| {
            tracing::trace!(
                plugin_type = %group.plugin_type,
                item_count = group.items.len(),
                "queuing fetch_releases batch group"
            );
            run_fetch_group(group, Arc::clone(&executor), fetcher_factory)
        })
        .collect();

    let mut fetch_map: HashMap<usize, FetchItemResult> = HashMap::new();
    for group_results in join_all(fetch_futs).await {
        for (idx, result) in group_results {
            fetch_map.insert(idx, result);
        }
    }

    // ── Step 5: Merge per-item into VersionCheckResult (preserving order) ────
    assignments
        .iter()
        .enumerate()
        .map(|(idx, assignment)| {
            let (installed_version, installed_display_version, detect_error) =
                detect_map.get(&idx).cloned().unwrap_or((None, None, None));
            let (latest_version, update_category, fetch_error) = fetch_map
                .get(&idx)
                .cloned()
                .unwrap_or((None, UpdateCategory::Unknown, None));

            VersionCheckResult {
                software_item_id: assignment.software_item_id,
                host_software_item_id: assignment.host_software_item_id,
                installed_version,
                installed_display_version,
                latest_version,
                error: merge_errors(detect_error, fetch_error),
                update_category,
            }
        })
        .collect()
}

/// Check installed versions and latest versions for a batch of software items,
/// using native batch operations when the plugin supports them.
///
/// Results are returned in the same order as `assignments`.
#[tracing::instrument(skip_all, fields(assignment_count = assignments.len()))]
pub async fn batch_check_versions(
    assignments: Vec<VersionCheckAssignment>,
    executor: Arc<dyn CommandExecutor>,
    ctx: &ConnectionContext,
) -> Vec<VersionCheckResult> {
    batch_check_versions_inner(assignments, executor, ctx, &default_fetcher_factory).await
}

/// Run `op` with exponential backoff retry on transient plugin errors.
///
/// Retries up to `max_retries` times when `PluginError::is_retryable()` returns
/// `true`. On each transient failure, sleeps an exponentially increasing delay
/// and logs a debug message. Returns the first successful result or an error
/// string if all attempts fail.
async fn run_with_retry<'a, T>(
    label: &'static str,
    max_retries: u32,
    mut op: impl FnMut() -> std::pin::Pin<
        Box<dyn std::future::Future<Output = PluginResult<T>> + Send + 'a>,
    >,
) -> Result<T, String> {
    let mut backoff = Backoff::new(RETRY_BASE_DELAY, RETRY_MAX_DELAY);
    let mut last_error = None;

    for attempt in 0..=max_retries {
        match op().await {
            Ok(v) => return Ok(v),
            Err(e) => {
                let retryable = e.current_context().is_retryable() && attempt < max_retries;
                if retryable {
                    let delay = backoff.next_delay();
                    tracing::debug!(
                        attempt = attempt + 1,
                        max_retries,
                        delay_ms = delay.as_millis() as u64,
                        error = %e,
                        label,
                        "transient error, retrying",
                    );
                    tokio::time::sleep(delay).await;
                    last_error = Some(e);
                    continue;
                }
                return Err(format!("{label} failed: {e}"));
            }
        }
    }

    Err(format!(
        "{label} failed: {}",
        last_error.expect("last_error is set when loop exhausts retries")
    ))
}

/// Detect the installed version using a specific plugin assignment.
///
/// Retries up to [`MAX_RETRIES`] times when the error is transient
/// (see [`PluginError::is_retryable`]).
async fn detect_installed(
    assignment: &PluginAssignment,
    executor: Arc<dyn CommandExecutor>,
    ctx: &ConnectionContext,
) -> Result<Option<String>, String> {
    tracing::debug!(
        plugin_type = ?assignment.plugin_type,
        package = %assignment.package_identifier,
        "detecting installed version"
    );

    let mut effective_config = assignment.config.clone();
    ctx.apply_to_config(&assignment.plugin_type, &mut effective_config);

    // Plugin creation is not retried — config/instantiation errors aren't transient.
    let runtime = construct_host_runtime(executor, HostCapabilities::default());

    let desc = get_descriptor(assignment.plugin_type.as_str())
        .ok_or_else(|| format!("unknown plugin type: {}", assignment.plugin_type))?;

    let slot = desc.roles.version_detector.as_ref().ok_or_else(|| {
        format!(
            "plugin {} does not implement VersionDetectorPlugin",
            assignment.plugin_type
        )
    })?;

    let detector = (slot.create)(&effective_config, runtime).map_err(|e| e.to_string())?;

    let pkg = &assignment.package_identifier;
    match run_with_retry("detect_installed_version", MAX_RETRIES, || {
        Box::pin(detector.detect_installed_version(pkg))
    })
    .await
    {
        Ok(Some(version)) => {
            tracing::debug!(version = %version, "installed version detected");
            Ok(Some(version.to_string()))
        }
        Ok(None) => {
            tracing::debug!("no installed version detected");
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// Fetch the latest available version using a specific plugin assignment.
///
/// Returns `Ok(Some((version_string, category)))` when a release is found.
/// Retries up to [`MAX_RETRIES`] times when the error is transient
/// (see [`PluginError::is_retryable`]).
async fn fetch_latest(
    assignment: &PluginAssignment,
    executor: Arc<dyn CommandExecutor>,
    ctx: &ConnectionContext,
) -> Result<Option<(String, UpdateCategory)>, String> {
    tracing::debug!(
        plugin_type = ?assignment.plugin_type,
        package = %assignment.package_identifier,
        "fetching releases"
    );

    let mut effective_config = assignment.config.clone();
    ctx.apply_to_config(&assignment.plugin_type, &mut effective_config);

    // Plugin creation is not retried — config/instantiation errors aren't transient.
    let runtime = construct_host_runtime(executor, HostCapabilities::default());

    let desc = get_descriptor(assignment.plugin_type.as_str())
        .ok_or_else(|| format!("unknown plugin type: {}", assignment.plugin_type))?;

    let slot = desc.roles.release_fetcher.as_ref().ok_or_else(|| {
        format!(
            "plugin {} does not implement ReleaseFetcherPlugin",
            assignment.plugin_type
        )
    })?;

    let fetcher = (slot.create)(&effective_config, runtime).map_err(|e| e.to_string())?;

    let pkg = &assignment.package_identifier;
    let releases = match run_with_retry("fetch_releases", MAX_RETRIES, || {
        Box::pin(fetcher.fetch_releases(pkg))
    })
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(error = %e, "failed to fetch latest version from plugin");
            return Err(e);
        }
    };

    tracing::debug!(count = releases.len(), "releases fetched");
    Ok(releases.first().map(|r| {
        let category = r.category.clone().unwrap_or_default();
        (r.version.to_string(), category)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use uptrakit_command::LocalCommandExecutor;
    use uptrakit_internal_wire::plugin_ids;
    use uptrakit_plugin_infrastructure_core::{
        BatchFetchItem, BatchFetchResult, PluginMeta, UpstreamRelease,
    };

    fn test_executor() -> Arc<dyn CommandExecutor> {
        Arc::new(LocalCommandExecutor)
    }

    fn no_ctx() -> ConnectionContext {
        ConnectionContext::default()
    }

    fn fetch_assignment(name: &str, package_identifier: &str) -> VersionCheckAssignment {
        fetch_assignment_with_plugin(
            name,
            package_identifier,
            plugin_ids::RELEASES_DOCKER.clone(),
        )
    }

    fn fetch_assignment_with_plugin(
        name: &str,
        package_identifier: &str,
        plugin_type: PluginTypeId,
    ) -> VersionCheckAssignment {
        VersionCheckAssignment {
            software_item_id: uuid::Uuid::now_v7(),
            name: name.to_string(),
            detect_version: None,
            fetch_releases: Some(PluginAssignment {
                plugin_type,
                package_identifier: package_identifier.to_string(),
                config: serde_json::json!({}),
            }),
            host_software_item_id: Some(uuid::Uuid::now_v7()),
        }
    }

    struct AlwaysFailFetcher;

    impl PluginMeta for AlwaysFailFetcher {
        fn plugin_type_id(&self) -> PluginTypeId {
            PluginTypeId::from_static("test_always_fail")
        }
    }

    #[async_trait]
    impl ReleaseFetcher for AlwaysFailFetcher {
        async fn fetch_releases(
            &self,
            _package_identifier: &str,
        ) -> PluginResult<Vec<UpstreamRelease>> {
            Err(rootcause::report!(PluginError::PluginInternal(
                "simulated registry unavailable".to_string()
            )))
        }
    }

    struct BatchLevelFailFetcher;

    impl PluginMeta for BatchLevelFailFetcher {
        fn plugin_type_id(&self) -> PluginTypeId {
            PluginTypeId::from_static("test_batch_level_fail")
        }
    }

    #[async_trait]
    impl ReleaseFetcher for BatchLevelFailFetcher {
        async fn fetch_releases(
            &self,
            _package_identifier: &str,
        ) -> PluginResult<Vec<UpstreamRelease>> {
            unreachable!("batch_fetch is overridden; fetch_releases should not be called")
        }

        async fn batch_fetch(
            &self,
            _items: &[BatchFetchItem],
        ) -> PluginResult<Vec<BatchFetchResult>> {
            Err(rootcause::report!(PluginError::PluginInternal(
                "simulated batch-level network failure".to_string()
            )))
        }
    }

    fn failing_fetcher_factory(
        _plugin_type: &PluginTypeId,
        _config: &serde_json::Value,
        _runtime: Arc<dyn HostRuntime>,
    ) -> PluginResult<Box<dyn ReleaseFetcher>> {
        Ok(Box::new(AlwaysFailFetcher))
    }

    fn batch_failing_fetcher_factory(
        _plugin_type: &PluginTypeId,
        _config: &serde_json::Value,
        _runtime: Arc<dyn HostRuntime>,
    ) -> PluginResult<Box<dyn ReleaseFetcher>> {
        Ok(Box::new(BatchLevelFailFetcher))
    }

    fn broken_factory(
        _plugin_type: &PluginTypeId,
        _config: &serde_json::Value,
        _runtime: Arc<dyn HostRuntime>,
    ) -> PluginResult<Box<dyn ReleaseFetcher>> {
        Err(rootcause::report!(PluginError::Configuration(
            "simulated plugin creation failure".to_string()
        )))
    }

    fn gh_assignment() -> PluginAssignment {
        PluginAssignment {
            plugin_type: plugin_ids::RELEASES_GITHUB.clone(),
            package_identifier: "octocat/hello-world".to_string(),
            // GitHub plugin config no longer contains owner/repo — those are
            // expressed via package_identifier at the software item level.
            config: serde_json::json!({}),
        }
    }

    #[tokio::test]
    async fn check_version_github_detect_not_supported() {
        // The GitHub plugin is fetch-only; using it for detect returns an error.
        let assignment = gh_assignment();
        let outcome = check_version(Some(&assignment), None, test_executor(), &no_ctx()).await;
        assert!(outcome.installed_version.is_none());
        assert!(outcome.latest_version.is_none());
        // The GitHub plugin's default detect_installed_version returns an error.
        assert!(outcome.error.is_some());
    }

    #[tokio::test]
    async fn check_version_docker_installed_reflects_local_daemon() {
        // With digest-based tracking, detect_installed_version always queries the
        // local Docker daemon for the image digest. The exact outcome depends on
        // whether the daemon is running and whether the image is installed locally:
        //
        // - daemon up + image present  → installed_version = Some(digest), error = None
        // - daemon up + image absent   → installed_version = None,         error = None
        // - daemon down                → installed_version = None,         error = Some(...)
        //
        // Only the invariants that hold across all cases are asserted here.
        let assignment = PluginAssignment {
            plugin_type: plugin_ids::RELEASES_DOCKER.clone(),
            package_identifier: "nginx".to_string(),
            config: serde_json::json!({}),
        };
        let outcome = check_version(Some(&assignment), None, test_executor(), &no_ctx()).await;
        // No fetch assignment → latest_version must always be None.
        assert!(outcome.latest_version.is_none());
        // installed_version and error are mutually exclusive: a successful inspect
        // returns a version (or None), while a daemon connection error sets error.
        assert!(
            !(outcome.installed_version.is_some() && outcome.error.is_some()),
            "installed_version and error cannot both be set: {:?} / {:?}",
            outcome.installed_version,
            outcome.error,
        );
    }

    #[tokio::test]
    async fn check_version_proxmox_is_discovery_only() {
        // PHS is discovery-only; `detect_installed_version` is not supported.
        let assignment = PluginAssignment {
            plugin_type: plugin_ids::DISCOVERY_PROXMOX_HELPER_SCRIPTS.clone(),
            package_identifier: "booklore".to_string(),
            config: serde_json::json!({}),
        };
        let outcome = check_version(Some(&assignment), None, test_executor(), &no_ctx()).await;
        assert!(outcome.installed_version.is_none());
        assert!(outcome.latest_version.is_none());
        // The trait default returns an error for unsupported operations.
        assert!(outcome.error.is_some());
    }

    #[tokio::test]
    async fn check_version_github_invalid_config() {
        // GitHub is a release-only plugin — it does not implement VersionDetector.
        // Assigning it to the detect role should produce an error.
        let assignment = PluginAssignment {
            plugin_type: plugin_ids::RELEASES_GITHUB.clone(),
            package_identifier: "octocat/hello-world".to_string(),
            config: serde_json::json!({"api_base_url": "http://api.github.com"}),
        };
        let outcome = check_version(Some(&assignment), None, test_executor(), &no_ctx()).await;
        assert!(outcome.installed_version.is_none());
        assert!(outcome.error.is_some());
        assert!(
            outcome.error.as_ref().unwrap().contains("VersionDetector"),
            "expected error about missing VersionDetector role, got: {}",
            outcome.error.unwrap()
        );
    }

    #[tokio::test]
    async fn check_version_homebrew_default_returns_none() {
        let assignment = PluginAssignment {
            plugin_type: plugin_ids::PACKAGE_MANAGER_HOMEBREW.clone(),
            package_identifier: String::new(),
            config: serde_json::json!({}),
        };
        let outcome = check_version(Some(&assignment), None, test_executor(), &no_ctx()).await;
        assert!(outcome.installed_version.is_none());
        assert!(outcome.latest_version.is_none());
        assert!(outcome.error.is_some());
    }

    #[tokio::test]
    async fn check_version_docker_default_context_does_not_panic() {
        let assignment = PluginAssignment {
            plugin_type: plugin_ids::RELEASES_DOCKER.clone(),
            package_identifier: "nginx".to_string(),
            config: serde_json::json!({}),
        };
        let ctx = ConnectionContext::default();
        // Default context (no keep-alive handles) should not panic during
        // plugin creation. The check itself will fail (no daemon) but that
        // proves the creation path runs without panicking.
        let outcome = check_version(Some(&assignment), None, test_executor(), &ctx).await;
        let _ = outcome;
    }

    // -- Fetch-failure error propagation tests --------------------------------
    // Execution model:
    // - Workflow: .github/workflows/agent-core-batch-ci.yml
    // - Job: agent-core-batch-failure-path
    // - Command: cargo test -p uptrakit-agent-core --all-features batch_check_versions_
    //
    // Precondition for this group:
    // - fetch_assignment() uses plugin_ids::RELEASES_DOCKER.
    // - The Docker descriptor does not implement RefreshPackageIndex.
    // - refresh_package_indexes() therefore skips these assignments, keeping the
    //   tests deterministic with no Docker daemon, external network, or DB usage.

    #[tokio::test]
    async fn batch_check_versions_fetch_error_sets_error_field() {
        let results = batch_check_versions_inner(
            vec![fetch_assignment("pkg-1", "nginx:latest")],
            test_executor(),
            &no_ctx(),
            &failing_fetcher_factory,
        )
        .await;

        assert_eq!(results.len(), 1);
        let result = &results[0];
        assert!(result.error.is_some(), "expected error when fetch fails");
        assert!(
            result.latest_version.is_none(),
            "expected latest_version to be None, got {:?}",
            result.latest_version
        );
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|error| error.contains("simulated registry unavailable")),
            "unexpected error: {:?}",
            result.error
        );
    }

    #[tokio::test]
    async fn batch_check_versions_all_items_get_error_on_total_fetch_failure() {
        let assignments = vec![
            fetch_assignment("pkg-1", "img-1:latest"),
            fetch_assignment("pkg-2", "img-2:latest"),
            fetch_assignment("pkg-3", "img-3:latest"),
        ];

        let results = batch_check_versions_inner(
            assignments,
            test_executor(),
            &no_ctx(),
            &failing_fetcher_factory,
        )
        .await;

        assert_eq!(results.len(), 3);
        for (idx, result) in results.iter().enumerate() {
            assert!(result.error.is_some(), "result[{idx}] should have error");
            assert!(
                result.latest_version.is_none(),
                "result[{idx}] latest_version should be None, got {:?}",
                result.latest_version
            );
        }
    }

    #[tokio::test]
    async fn batch_check_versions_factory_error_sets_error_field() {
        let assignments = vec![
            fetch_assignment("pkg-1", "img-1:latest"),
            fetch_assignment("pkg-2", "img-2:latest"),
        ];

        let results =
            batch_check_versions_inner(assignments, test_executor(), &no_ctx(), &broken_factory)
                .await;

        assert_eq!(results.len(), 2);
        for (idx, result) in results.iter().enumerate() {
            assert!(result.error.is_some(), "result[{idx}] should have error");
            assert!(
                result.latest_version.is_none(),
                "result[{idx}] latest_version should be None, got {:?}",
                result.latest_version
            );
            assert!(
                result
                    .error
                    .as_deref()
                    .is_some_and(|error| error.contains("failed to create plugin")),
                "result[{idx}] unexpected error: {:?}",
                result.error
            );
        }
    }

    #[tokio::test]
    async fn batch_check_versions_batch_level_fetch_error_sets_error_field() {
        let assignments = vec![
            fetch_assignment("pkg-1", "img-1:latest"),
            fetch_assignment("pkg-2", "img-2:latest"),
        ];

        let results = batch_check_versions_inner(
            assignments,
            test_executor(),
            &no_ctx(),
            &batch_failing_fetcher_factory,
        )
        .await;

        assert_eq!(results.len(), 2);
        for (idx, result) in results.iter().enumerate() {
            assert!(result.error.is_some(), "result[{idx}] should have error");
            assert!(
                result.latest_version.is_none(),
                "result[{idx}] latest_version should be None, got {:?}",
                result.latest_version
            );
            assert!(
                result
                    .error
                    .as_deref()
                    .is_some_and(|error| error.contains("fetch_releases failed")),
                "result[{idx}] unexpected error: {:?}",
                result.error
            );
        }
    }

    #[tokio::test]
    async fn batch_check_versions_public_api_uses_default_factory_role_validation() {
        let assignments = vec![fetch_assignment_with_plugin(
            "pkg-1",
            "booklore",
            plugin_ids::DISCOVERY_PROXMOX_HELPER_SCRIPTS.clone(),
        )];

        let results = batch_check_versions(assignments, test_executor(), &no_ctx()).await;

        assert_eq!(results.len(), 1);
        let result = &results[0];
        assert!(
            result.error.is_some(),
            "expected error when fetch plugin role is missing"
        );
        assert!(
            result.latest_version.is_none(),
            "expected latest_version to be None, got {:?}",
            result.latest_version
        );
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|error| error.contains("does not implement ReleaseFetcherPlugin")),
            "unexpected error: {:?}",
            result.error
        );
    }

    #[tokio::test]
    async fn check_version_no_assignments_returns_empty() {
        let outcome = check_version(None, None, test_executor(), &no_ctx()).await;
        assert!(outcome.installed_version.is_none());
        assert!(outcome.latest_version.is_none());
        assert!(outcome.error.is_none());
    }
}
