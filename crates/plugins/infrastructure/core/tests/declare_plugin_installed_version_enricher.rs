//! Smoke test: `declare_plugin!` accepts the optional
//! `installed_version_enricher_create:` arm and populates the
//! `InstalledVersionEnricher` slot on `DESCRIPTOR.roles`.

use std::sync::Arc;

use uptrakit_plugin_infrastructure_core::{
    ConfigModel, HostRequirements, HostRuntime, InstalledVersionDisplay, InstalledVersionEnricher,
    InstalledVersionEnrichmentContext, InstalledVersionItem, PluginFamily, Result, declare_plugin,
};

struct DummyPlugin;

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
struct DummyConfig {}

impl uptrakit_plugin_infrastructure_core::PluginConfig for DummyConfig {}

#[async_trait::async_trait]
impl InstalledVersionEnricher for DummyPlugin {
    async fn enrich_installed_versions(
        &self,
        _items: &[InstalledVersionItem],
    ) -> Result<Vec<InstalledVersionDisplay>> {
        Ok(Vec::new())
    }
}

fn factory(
    _cfg: &serde_json::Value,
    _runtime: Arc<dyn HostRuntime>,
    _ctx: &InstalledVersionEnrichmentContext,
) -> Result<Box<dyn InstalledVersionEnricher>> {
    Ok(Box::new(DummyPlugin))
}

declare_plugin!(DummyPlugin, DummyConfig, "test_dummy_ive", {
    display_name: "Dummy",
    family: PluginFamily::Software,
    config_model: ConfigModel::PluginConfig,
    host_requirements: HostRequirements::POSIX,
    roles: [],
    installed_version_enricher_create: {
        create: factory,
        host_requirements: HostRequirements::CONTROLLER_ONLY,
    },
});

#[test]
fn declare_plugin_accepts_installed_version_enricher_create() {
    assert!(
        DESCRIPTOR.roles.installed_version_enricher.is_some(),
        "macro must populate the slot"
    );
}
