use std::sync::Arc;

use async_trait::async_trait;
use uptrakit_plugin_infrastructure_core::{
    BatchFetchItem, BatchFetchResult, ConfigModel, ConfigOps, HostRequirements, HostRuntime,
    PluginCapability, PluginConfigValidationError, PluginDescriptor, PluginError, PluginFamily,
    PluginMeta, ReleaseFetcher, Result, RoleCreators, RoleSlot, UpstreamRelease,
    form_schema::FormFieldDescriptor,
};
use uptrakit_shared_types::PluginTypeId;

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
) -> Result<Box<dyn ReleaseFetcher>> {
    Ok(Box::new(TestFetchFailPlugin))
}

fn create_per_item_fail_release_fetcher(
    _cfg: &serde_json::Value,
    _runtime: Arc<dyn HostRuntime>,
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
        release_fetcher: Some(RoleSlot {
            create: create_release_fetcher,
            host_requirements: HostRequirements::CONTROLLER_ONLY,
        }),
        package_indexer: None,
        update_executor: None,
        lifecycle_hook: None,
        notification_transport: None,
        software_item_lifecycle: None,
        controller_update_protection: None,
        infra: None,
    },
    surface_actions: None,
    surfaces: None,
    type_settings: None,
    config_test: None,
    sudo: None,
    raw_settings_keys: &[],
    migrations: None,
    global_provider_consumers: &[],
};

pub static PER_ITEM_FAIL_DESCRIPTOR: PluginDescriptor = PluginDescriptor {
    type_id: "__test_per_item_fail",
    display_name: "Test Per-Item Fail (test-only)",
    family: PluginFamily::Software,
    config_model: ConfigModel::None,
    capabilities: TEST_RELEASE_FETCH_CAPABILITIES,
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
        release_fetcher: Some(RoleSlot {
            create: create_per_item_fail_release_fetcher,
            host_requirements: HostRequirements::CONTROLLER_ONLY,
        }),
        package_indexer: None,
        update_executor: None,
        lifecycle_hook: None,
        notification_transport: None,
        software_item_lifecycle: None,
        controller_update_protection: None,
        infra: None,
    },
    surface_actions: None,
    surfaces: None,
    type_settings: None,
    config_test: None,
    sudo: None,
    raw_settings_keys: &[],
    migrations: None,
    global_provider_consumers: &[],
};
