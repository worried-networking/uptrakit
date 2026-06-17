use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use uptrakit_plugin_infrastructure_core::{
    BatchFetchItem, BatchFetchResult, ConfigModel, ConfigOps, HostRequirements, HostRuntime,
    InstalledVersionDisplay, InstalledVersionEnricher, InstalledVersionEnricherSlot,
    InstalledVersionEnrichmentContext, InstalledVersionItem, PluginCapability,
    PluginConfigValidationError, PluginDescriptor, PluginError, PluginFamily, PluginMeta,
    PluginScope, ReleaseFetcher, Result, RoleCreators, UpstreamRelease,
    descriptor::ReleaseFetcherSlot, form_schema::FormFieldDescriptor, roles::ReleaseFetchContext,
};
use uptrakit_shared_types::PluginTypeId;

/// Whether the last `create_ctx_capture_fetcher` call received a non-`None` lookup.
static CTX_CAPTURE_HAD_LOOKUP: AtomicBool = AtomicBool::new(false);

/// Reset the context-capture flag. Call before executing the test.
pub fn reset_ctx_capture_had_lookup() {
    CTX_CAPTURE_HAD_LOOKUP.store(false, Ordering::SeqCst);
}

/// Returns whether the last `create_ctx_capture_fetcher` call saw a non-`None` lookup.
pub fn ctx_capture_had_lookup() -> bool {
    CTX_CAPTURE_HAD_LOOKUP.load(Ordering::SeqCst)
}

const BATCH_LEVEL_FETCH_FAILURE: &str = "test: batch-level fetch failure";
const PER_ITEM_FETCH_FAILURE: &str = "test: per-item fetch failure";
const TEST_RELEASE_FETCH_CAPABILITIES: &[PluginCapability] = &[
    PluginCapability::ReleaseFetching,
    PluginCapability::ControllerSideFetchReleases,
];

pub struct TestFetchFailPlugin;

pub struct TestPerItemFailPlugin;

#[async_trait]
impl ReleaseFetcher for TestFetchFailPlugin {
    async fn fetch_releases(&self, _package_identifier: &str) -> Result<Vec<UpstreamRelease>> {
        Err(rootcause::report!(PluginError::PluginInternal(
            BATCH_LEVEL_FETCH_FAILURE.into()
        )))
    }

    async fn batch_fetch(&self, _items: &[BatchFetchItem]) -> Result<Vec<BatchFetchResult>> {
        Err(rootcause::report!(PluginError::PluginInternal(
            BATCH_LEVEL_FETCH_FAILURE.into()
        )))
    }
}

impl PluginMeta for TestFetchFailPlugin {
    fn plugin_type_id(&self) -> PluginTypeId {
        PluginTypeId::from_static("__test_fetch_fail")
    }
}

#[async_trait]
impl ReleaseFetcher for TestPerItemFailPlugin {
    async fn fetch_releases(&self, _package_identifier: &str) -> Result<Vec<UpstreamRelease>> {
        Err(rootcause::report!(PluginError::PluginInternal(
            PER_ITEM_FETCH_FAILURE.into()
        )))
    }
}

impl PluginMeta for TestPerItemFailPlugin {
    fn plugin_type_id(&self) -> PluginTypeId {
        PluginTypeId::from_static("__test_per_item_fail")
    }
}

fn create_release_fetcher(
    _cfg: &serde_json::Value,
    _runtime: Arc<dyn HostRuntime>,
    _ctx: &ReleaseFetchContext,
) -> Result<Box<dyn ReleaseFetcher>> {
    Ok(Box::new(TestFetchFailPlugin))
}

fn create_per_item_fail_release_fetcher(
    _cfg: &serde_json::Value,
    _runtime: Arc<dyn HostRuntime>,
    _ctx: &ReleaseFetchContext,
) -> Result<Box<dyn ReleaseFetcher>> {
    Ok(Box::new(TestPerItemFailPlugin))
}

fn validate(_cfg: &serde_json::Value) -> std::result::Result<(), PluginConfigValidationError> {
    Ok(())
}

fn mask_secrets(cfg: &serde_json::Value) -> serde_json::Value {
    cfg.clone()
}

fn restore_secrets(_cfg: &mut serde_json::Value, _masked: &serde_json::Value) {}

fn sample() -> serde_json::Value {
    serde_json::json!({})
}

fn form_schema() -> Vec<FormFieldDescriptor> {
    vec![]
}

fn validate_identifier(_value: &str) -> std::result::Result<(), PluginConfigValidationError> {
    Ok(())
}

pub static DESCRIPTOR: PluginDescriptor = PluginDescriptor {
    type_id: "__test_fetch_fail",
    display_name: "Test Fetch Fail (test-only)",
    family: PluginFamily::Software,
    config_model: ConfigModel::None,
    capabilities: TEST_RELEASE_FETCH_CAPABILITIES,
    scope: PluginScope::Tenant,
    instance_config: None,
    config: ConfigOps {
        validate,
        mask_secrets,
        restore_secrets,
        sample,
        form_schema,
        validate_identifier,
    },
    roles: RoleCreators {
        discoverer: None,
        version_detector: None,
        release_fetcher: Some(ReleaseFetcherSlot::new(
            create_release_fetcher,
            HostRequirements::CONTROLLER_ONLY,
        )),
        package_indexer: None,
        update_executor: None,
        lifecycle_hook: None,
        notification_transport: None,
        software_item_lifecycle: None,
        controller_update_protection: None,
        controller_update_hook: None,
        infra: None,
        installed_version_enricher: None,
    },
    surface_actions: None,
    surfaces: None,
    type_settings: None,
    config_test: None,
    sudo: None,
    raw_settings_keys: &[],
    migrations: None,
    reset_tenant_data: None,
    db_migrate_tables: None,
    global_provider_consumers: &[],
};

pub static PER_ITEM_FAIL_DESCRIPTOR: PluginDescriptor = PluginDescriptor {
    type_id: "__test_per_item_fail",
    display_name: "Test Per-Item Fail (test-only)",
    family: PluginFamily::Software,
    config_model: ConfigModel::None,
    capabilities: TEST_RELEASE_FETCH_CAPABILITIES,
    scope: PluginScope::Tenant,
    instance_config: None,
    config: ConfigOps {
        validate,
        mask_secrets,
        restore_secrets,
        sample,
        form_schema,
        validate_identifier,
    },
    roles: RoleCreators {
        discoverer: None,
        version_detector: None,
        release_fetcher: Some(ReleaseFetcherSlot::new(
            create_per_item_fail_release_fetcher,
            HostRequirements::CONTROLLER_ONLY,
        )),
        package_indexer: None,
        update_executor: None,
        lifecycle_hook: None,
        notification_transport: None,
        software_item_lifecycle: None,
        controller_update_protection: None,
        controller_update_hook: None,
        infra: None,
        installed_version_enricher: None,
    },
    surface_actions: None,
    surfaces: None,
    type_settings: None,
    config_test: None,
    sudo: None,
    raw_settings_keys: &[],
    migrations: None,
    reset_tenant_data: None,
    db_migrate_tables: None,
    global_provider_consumers: &[],
};

// ── Context-capture plugin ────────────────────────────────────────────────────

pub struct TestCtxCapturePlugin;

impl PluginMeta for TestCtxCapturePlugin {
    fn plugin_type_id(&self) -> PluginTypeId {
        PluginTypeId::from_static("__test_ctx_capture")
    }
}

#[async_trait]
impl ReleaseFetcher for TestCtxCapturePlugin {
    async fn fetch_releases(&self, _package_identifier: &str) -> Result<Vec<UpstreamRelease>> {
        Ok(vec![])
    }

    async fn batch_fetch(&self, _items: &[BatchFetchItem]) -> Result<Vec<BatchFetchResult>> {
        Ok(vec![])
    }
}

fn create_ctx_capture_fetcher(
    _cfg: &serde_json::Value,
    _runtime: Arc<dyn HostRuntime>,
    ctx: &ReleaseFetchContext,
) -> Result<Box<dyn ReleaseFetcher>> {
    CTX_CAPTURE_HAD_LOOKUP.store(ctx.global_provider_lookup.is_some(), Ordering::SeqCst);
    Ok(Box::new(TestCtxCapturePlugin))
}

const TEST_ENRICHER_CAPABILITIES: &[PluginCapability] = &[PluginCapability::EnrichInstalledVersion];

/// Echoes the input `installed_version` as `display_version = Some("date_for_{sha}")`.
/// `None` input → `None` output (per the `None`-input contract).
pub struct TestEnricherEchoPlugin;

impl PluginMeta for TestEnricherEchoPlugin {
    fn plugin_type_id(&self) -> PluginTypeId {
        PluginTypeId::from_static("__test_enricher_echo")
    }
}

#[async_trait]
impl InstalledVersionEnricher for TestEnricherEchoPlugin {
    async fn enrich_installed_versions(
        &self,
        items: &[InstalledVersionItem],
    ) -> Result<Vec<InstalledVersionDisplay>> {
        Ok(items
            .iter()
            .map(|item| {
                InstalledVersionDisplay::new(
                    item.package_identifier.clone(),
                    item.installed_version.clone(),
                    item.installed_version
                        .as_ref()
                        .map(|sha| format!("date_for_{sha}")),
                )
            })
            .collect())
    }
}

fn create_test_enricher_echo(
    _cfg: &serde_json::Value,
    _runtime: Arc<dyn HostRuntime>,
    _ctx: &InstalledVersionEnrichmentContext,
) -> Result<Box<dyn InstalledVersionEnricher>> {
    Ok(Box::new(TestEnricherEchoPlugin))
}

pub static ENRICHER_ECHO_DESCRIPTOR: PluginDescriptor = PluginDescriptor {
    type_id: "__test_enricher_echo",
    display_name: "Test Enricher Echo (test-only)",
    family: PluginFamily::Software,
    config_model: ConfigModel::None,
    capabilities: TEST_ENRICHER_CAPABILITIES,
    scope: PluginScope::Tenant,
    instance_config: None,
    config: ConfigOps {
        validate,
        mask_secrets,
        restore_secrets,
        sample,
        form_schema,
        validate_identifier,
    },
    roles: RoleCreators {
        discoverer: None,
        version_detector: None,
        release_fetcher: None,
        package_indexer: None,
        update_executor: None,
        lifecycle_hook: None,
        notification_transport: None,
        software_item_lifecycle: None,
        controller_update_protection: None,
        controller_update_hook: None,
        infra: None,
        installed_version_enricher: Some(InstalledVersionEnricherSlot::new(
            create_test_enricher_echo,
            HostRequirements::CONTROLLER_ONLY,
        )),
    },
    surface_actions: None,
    surfaces: None,
    type_settings: None,
    config_test: None,
    sudo: None,
    raw_settings_keys: &[],
    migrations: None,
    reset_tenant_data: None,
    db_migrate_tables: None,
    global_provider_consumers: &[],
};

/// Always returns `display_version = None`, regardless of input.
pub struct TestEnricherMissPlugin;

impl PluginMeta for TestEnricherMissPlugin {
    fn plugin_type_id(&self) -> PluginTypeId {
        PluginTypeId::from_static("__test_enricher_miss")
    }
}

#[async_trait]
impl InstalledVersionEnricher for TestEnricherMissPlugin {
    async fn enrich_installed_versions(
        &self,
        items: &[InstalledVersionItem],
    ) -> Result<Vec<InstalledVersionDisplay>> {
        Ok(items
            .iter()
            .map(|item| {
                InstalledVersionDisplay::new(
                    item.package_identifier.clone(),
                    item.installed_version.clone(),
                    None,
                )
            })
            .collect())
    }
}

fn create_test_enricher_miss(
    _cfg: &serde_json::Value,
    _runtime: Arc<dyn HostRuntime>,
    _ctx: &InstalledVersionEnrichmentContext,
) -> Result<Box<dyn InstalledVersionEnricher>> {
    Ok(Box::new(TestEnricherMissPlugin))
}

pub static ENRICHER_MISS_DESCRIPTOR: PluginDescriptor = PluginDescriptor {
    type_id: "__test_enricher_miss",
    display_name: "Test Enricher Miss (test-only)",
    family: PluginFamily::Software,
    config_model: ConfigModel::None,
    capabilities: TEST_ENRICHER_CAPABILITIES,
    scope: PluginScope::Tenant,
    instance_config: None,
    config: ConfigOps {
        validate,
        mask_secrets,
        restore_secrets,
        sample,
        form_schema,
        validate_identifier,
    },
    roles: RoleCreators {
        discoverer: None,
        version_detector: None,
        release_fetcher: None,
        package_indexer: None,
        update_executor: None,
        lifecycle_hook: None,
        notification_transport: None,
        software_item_lifecycle: None,
        controller_update_protection: None,
        controller_update_hook: None,
        infra: None,
        installed_version_enricher: Some(InstalledVersionEnricherSlot::new(
            create_test_enricher_miss,
            HostRequirements::CONTROLLER_ONLY,
        )),
    },
    surface_actions: None,
    surfaces: None,
    type_settings: None,
    config_test: None,
    sudo: None,
    raw_settings_keys: &[],
    migrations: None,
    reset_tenant_data: None,
    db_migrate_tables: None,
    global_provider_consumers: &[],
};

pub static CTX_CAPTURE_DESCRIPTOR: PluginDescriptor = PluginDescriptor {
    type_id: "__test_ctx_capture",
    display_name: "Test Context Capture (test-only)",
    family: PluginFamily::Software,
    config_model: ConfigModel::None,
    capabilities: TEST_RELEASE_FETCH_CAPABILITIES,
    scope: PluginScope::Tenant,
    instance_config: None,
    config: ConfigOps {
        validate,
        mask_secrets,
        restore_secrets,
        sample,
        form_schema,
        validate_identifier,
    },
    roles: RoleCreators {
        discoverer: None,
        version_detector: None,
        release_fetcher: Some(ReleaseFetcherSlot::new(
            create_ctx_capture_fetcher,
            HostRequirements::CONTROLLER_ONLY,
        )),
        package_indexer: None,
        update_executor: None,
        lifecycle_hook: None,
        notification_transport: None,
        software_item_lifecycle: None,
        controller_update_protection: None,
        controller_update_hook: None,
        infra: None,
        installed_version_enricher: None,
    },
    surface_actions: None,
    surfaces: None,
    type_settings: None,
    config_test: None,
    sudo: None,
    raw_settings_keys: &[],
    migrations: None,
    reset_tenant_data: None,
    db_migrate_tables: None,
    global_provider_consumers: &[],
};
