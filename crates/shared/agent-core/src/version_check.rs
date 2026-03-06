use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::join_all;
use uptrakit_backoff::Backoff;
use uptrakit_command::CommandExecutor;
use uptrakit_internal_wire::{
    PluginAssignment, PluginType, UpdateCategory, VersionCheckAssignment, VersionCheckResult,
};
use uptrakit_plugin_infrastructure_core::{BatchDetectItem, BatchFetchItem};
use uptrakit_plugin_infrastructure_registry::{PluginCapability, PluginRegistry};

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

/// Check installed versions and latest versions for a batch of software items,
/// using native batch operations when the plugin supports them.
///
/// Groups assignments by `(PluginType, effective_config_json)` so that all
/// items sharing the same plugin configuration are checked in a single plugin
/// invocation. Plugins that override `batch_detect_installed_version` or
/// `batch_fetch_releases` (e.g. APT, Homebrew, npm) benefit from a single
/// subprocess call per group instead of N per-item calls.
///
/// `RefreshPackageIndex` is called at most once per unique fetch-group
/// (plugin_type, config) before `batch_fetch_releases` runs. It is not called
/// for detect-only groups.
///
/// Results are returned in the same order as `assignments`.
#[tracing::instrument(skip_all, fields(assignment_count = assignments.len()))]
pub async fn batch_check_versions(
    assignments: Vec<VersionCheckAssignment>,
    executor: Arc<dyn CommandExecutor>,
    ctx: &ConnectionContext,
) -> Vec<VersionCheckResult> {
    if assignments.is_empty() {
        return vec![];
    }

    // Group key: (plugin_type, serialised effective config) — unique per config.
    type GroupKey = (PluginType, String);

    struct Group {
        plugin_type: PluginType,
        effective_config: serde_json::Value,
        /// (assignment index, package_identifier)
        items: Vec<(usize, String)>,
    }

    // ── Step 1: Build detect and fetch groups ────────────────────────────────
    let mut detect_groups: HashMap<GroupKey, Group> = HashMap::new();
    let mut fetch_groups: HashMap<GroupKey, Group> = HashMap::new();

    for (idx, assignment) in assignments.iter().enumerate() {
        if let Some(pa) = &assignment.detect_version {
            let mut effective_config = pa.config.clone();
            ctx.apply_to_config(&pa.plugin_type, &mut effective_config);
            let key = (pa.plugin_type.clone(), effective_config.to_string());
            let group = detect_groups.entry(key).or_insert_with(|| Group {
                plugin_type: pa.plugin_type.clone(),
                effective_config,
                items: vec![],
            });
            group.items.push((idx, pa.package_identifier.clone()));
        }
        if let Some(pa) = &assignment.fetch_releases {
            let mut effective_config = pa.config.clone();
            ctx.apply_to_config(&pa.plugin_type, &mut effective_config);
            let key = (pa.plugin_type.clone(), effective_config.to_string());
            let group = fetch_groups.entry(key).or_insert_with(|| Group {
                plugin_type: pa.plugin_type.clone(),
                effective_config,
                items: vec![],
            });
            group.items.push((idx, pa.package_identifier.clone()));
        }
    }

    // ── Step 2: Run detect groups in parallel ────────────────────────────────
    // Each future resolves to Vec<(item_idx, (Option<version_str>, Option<error_str>))>.
    let detect_futs: Vec<_> = detect_groups
        .into_values()
        .map(|group| {
            let executor = Arc::clone(&executor);
            async move {
                let batch_items: Vec<BatchDetectItem> = group
                    .items
                    .iter()
                    .map(|(_, pkg)| BatchDetectItem {
                        package_identifier: pkg.clone(),
                    })
                    .collect();

                let plugin = match PluginRegistry::create_plugin(
                    group.plugin_type.clone(),
                    &group.effective_config,
                    executor,
                )
                .await
                {
                    Ok(p) => p,
                    Err(e) => {
                        let err = format!("failed to create plugin: {e}");
                        return group
                            .items
                            .iter()
                            .map(|(idx, _)| (*idx, (None::<String>, Some(err.clone()))))
                            .collect::<Vec<_>>();
                    }
                };

                let results = match plugin.batch_detect_installed_version(&batch_items).await {
                    Ok(r) => r,
                    Err(e) => {
                        let err = format!("detection failed: {e}");
                        return group
                            .items
                            .iter()
                            .map(|(idx, _)| (*idx, (None::<String>, Some(err.clone()))))
                            .collect::<Vec<_>>();
                    }
                };

                let result_map: HashMap<String, (Option<String>, Option<String>)> = results
                    .into_iter()
                    .map(|r| {
                        let version = r.installed_version.map(|v| v.to_string());
                        let error = r.error.map(|e| format!("detection failed: {e}"));
                        (r.package_identifier, (version, error))
                    })
                    .collect();

                group
                    .items
                    .iter()
                    .map(|(idx, pkg)| {
                        let outcome = result_map
                            .get(pkg.as_str())
                            .cloned()
                            .unwrap_or((None, None));
                        (*idx, outcome)
                    })
                    .collect::<Vec<_>>()
            }
        })
        .collect();

    let mut detect_map: HashMap<usize, (Option<String>, Option<String>)> = HashMap::new();
    for group_results in join_all(detect_futs).await {
        for (idx, result) in group_results {
            detect_map.insert(idx, result);
        }
    }

    // ── Step 3: RefreshPackageIndex – at most once per unique fetch group ───
    // Runs sequentially to avoid concurrent `apt-get update` / `brew update`.
    for (plugin_type, effective_config) in fetch_groups
        .values()
        .map(|g| (g.plugin_type.clone(), g.effective_config.clone()))
    {
        match PluginRegistry::create_plugin(
            plugin_type.clone(),
            &effective_config,
            Arc::clone(&executor),
        )
        .await
        {
            Ok(plugin) if plugin.has_capability(PluginCapability::RefreshPackageIndex) => {
                tracing::info!(
                    plugin_type = %plugin_type,
                    "refreshing package index"
                );
                if let Err(e) = plugin.refresh_package_index().await {
                    tracing::warn!(
                        plugin_type = %plugin_type,
                        error = %e,
                        "failed to refresh package index"
                    );
                }
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(
                    plugin_type = %plugin_type,
                    error = %e,
                    "failed to create plugin for index refresh"
                );
            }
        }
    }

    // ── Step 4: Run fetch groups in parallel ─────────────────────────────────
    // Each future resolves to Vec<(item_idx, (Option<version_str>, UpdateCategory, Option<error_str>))>.
    let fetch_futs: Vec<_> = fetch_groups
        .into_values()
        .map(|group| {
            let executor = Arc::clone(&executor);
            async move {
                let batch_items: Vec<BatchFetchItem> = group
                    .items
                    .iter()
                    .map(|(_, pkg)| BatchFetchItem {
                        package_identifier: pkg.clone(),
                    })
                    .collect();

                let plugin = match PluginRegistry::create_plugin(
                    group.plugin_type.clone(),
                    &group.effective_config,
                    executor,
                )
                .await
                {
                    Ok(p) => p,
                    Err(e) => {
                        let err = format!("failed to create plugin: {e}");
                        return group
                            .items
                            .iter()
                            .map(|(idx, _)| {
                                (
                                    *idx,
                                    (None::<String>, UpdateCategory::Unknown, Some(err.clone())),
                                )
                            })
                            .collect::<Vec<_>>();
                    }
                };

                let results = match plugin.batch_fetch_releases(&batch_items).await {
                    Ok(r) => r,
                    Err(e) => {
                        let err = format!("fetch_releases failed: {e}");
                        return group
                            .items
                            .iter()
                            .map(|(idx, _)| {
                                (
                                    *idx,
                                    (None::<String>, UpdateCategory::Unknown, Some(err.clone())),
                                )
                            })
                            .collect::<Vec<_>>();
                    }
                };

                let result_map: HashMap<String, (Option<String>, UpdateCategory, Option<String>)> =
                    results
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
                    .collect::<Vec<_>>()
            }
        })
        .collect();

    let mut fetch_map: HashMap<usize, (Option<String>, UpdateCategory, Option<String>)> =
        HashMap::new();
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
            let (installed_version, detect_error) =
                detect_map.get(&idx).cloned().unwrap_or((None, None));
            let (latest_version, update_category, fetch_error) = fetch_map
                .get(&idx)
                .cloned()
                .unwrap_or((None, UpdateCategory::Unknown, None));

            let error = match (detect_error, fetch_error) {
                (Some(d), Some(f)) => Some(format!("detect: {d}; fetch: {f}")),
                (Some(d), None) => Some(d),
                (None, Some(f)) => Some(f),
                (None, None) => None,
            };

            VersionCheckResult {
                software_item_id: assignment.software_item_id,
                host_package_id: assignment.host_package_id,
                installed_version,
                latest_version,
                error,
                update_category,
            }
        })
        .collect()
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
    let plugin =
        PluginRegistry::create_plugin(assignment.plugin_type.clone(), &effective_config, executor)
            .await
            .map_err(|e| e.to_string())?;

    let mut backoff = Backoff::new(RETRY_BASE_DELAY, RETRY_MAX_DELAY);
    let mut last_error = None;

    for attempt in 0..=MAX_RETRIES {
        match plugin
            .detect_installed_version(&assignment.package_identifier)
            .await
        {
            Ok(Some(version)) => {
                tracing::debug!(version = %version, "installed version detected");
                return Ok(Some(version.to_string()));
            }
            Ok(None) => {
                tracing::debug!("no installed version detected");
                return Ok(None);
            }
            Err(e) => {
                let retryable = e.current_context().is_retryable() && attempt < MAX_RETRIES;
                if retryable {
                    let delay = backoff.next_delay();
                    tracing::debug!(
                        attempt = attempt + 1,
                        max_retries = MAX_RETRIES,
                        delay_ms = delay.as_millis() as u64,
                        error = %e,
                        "transient detect error, retrying"
                    );
                    tokio::time::sleep(delay).await;
                    last_error = Some(e);
                    continue;
                }
                return Err(format!("detection failed: {e}"));
            }
        }
    }

    match last_error {
        Some(e) => Err(format!("detection failed: {e}")),
        None => Err("detection failed: unknown error".to_string()),
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
    let plugin =
        PluginRegistry::create_plugin(assignment.plugin_type.clone(), &effective_config, executor)
            .await
            .map_err(|e| e.to_string())?;

    let mut backoff = Backoff::new(RETRY_BASE_DELAY, RETRY_MAX_DELAY);
    let mut last_error = None;

    for attempt in 0..=MAX_RETRIES {
        match plugin.fetch_releases(&assignment.package_identifier).await {
            Ok(releases) => {
                tracing::debug!(count = releases.len(), "releases fetched");
                return Ok(releases.first().map(|r| {
                    let category = r.category.clone().unwrap_or_default();
                    (r.version.to_string(), category)
                }));
            }
            Err(e) => {
                let retryable = e.current_context().is_retryable() && attempt < MAX_RETRIES;
                if retryable {
                    let delay = backoff.next_delay();
                    tracing::debug!(
                        attempt = attempt + 1,
                        max_retries = MAX_RETRIES,
                        delay_ms = delay.as_millis() as u64,
                        error = %e,
                        "transient fetch error, retrying"
                    );
                    tokio::time::sleep(delay).await;
                    last_error = Some(e);
                    continue;
                }
                tracing::debug!(error = %e, "failed to fetch latest version from plugin");
                return Err(format!("fetch_releases failed: {e}"));
            }
        }
    }

    match last_error {
        Some(e) => Err(format!("fetch_releases failed: {e}")),
        None => Err("fetch_releases failed: unknown error".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_command::LocalCommandExecutor;
    use uptrakit_internal_wire::PluginType;

    fn test_executor() -> Arc<dyn CommandExecutor> {
        Arc::new(LocalCommandExecutor)
    }

    fn no_ctx() -> ConnectionContext {
        ConnectionContext::default()
    }

    fn gh_assignment() -> PluginAssignment {
        PluginAssignment {
            plugin_type: PluginType::ReleasesGithub,
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
            plugin_type: PluginType::ReleasesDocker,
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
            plugin_type: PluginType::DiscoveryProxmoxHelperScripts,
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
        // A non-https api_base_url fails GitHub config validation.
        let assignment = PluginAssignment {
            plugin_type: PluginType::ReleasesGithub,
            package_identifier: "octocat/hello-world".to_string(),
            config: serde_json::json!({"api_base_url": "http://api.github.com"}),
        };
        let outcome = check_version(Some(&assignment), None, test_executor(), &no_ctx()).await;
        assert!(outcome.installed_version.is_none());
        assert!(outcome.error.is_some());
        assert!(outcome.error.unwrap().contains("https"));
    }

    #[tokio::test]
    async fn check_version_homebrew_default_returns_none() {
        let assignment = PluginAssignment {
            plugin_type: PluginType::PackageManagerHomebrew,
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
            plugin_type: PluginType::ReleasesDocker,
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

    #[tokio::test]
    async fn check_version_no_assignments_returns_empty() {
        let outcome = check_version(None, None, test_executor(), &no_ctx()).await;
        assert!(outcome.installed_version.is_none());
        assert!(outcome.latest_version.is_none());
        assert!(outcome.error.is_none());
    }
}
