use std::sync::Arc;

use async_trait::async_trait;
use uptrakit_plugin_infrastructure_core::{
    CatalogConfig, ConfigModel, PluginFamily, SoftwareItemCreatedEvent, SoftwareItemLifecycle,
    SoftwareItemPatch, declare_plugin, error::PluginError,
};

use crate::cache::DashboardIconCache;
use crate::config::DashboardIconsConfig;

/// Dashboard Icons enhancement plugin.
///
/// Looks up software item names in the Dashboard Icons repository and returns
/// an icon URL when a match is found.
pub struct DashboardIconsPlugin {
    cache: Arc<DashboardIconCache>,
}

impl DashboardIconsPlugin {
    /// Create a new plugin backed by the given cache.
    pub fn new(cache: Arc<DashboardIconCache>) -> Self {
        Self { cache }
    }
}

#[async_trait]
impl SoftwareItemLifecycle for DashboardIconsPlugin {
    async fn on_software_item_created(
        &self,
        event: &SoftwareItemCreatedEvent,
    ) -> std::result::Result<Option<SoftwareItemPatch>, PluginError> {
        // Don't overwrite an existing icon.
        if event.icon_url.is_some() {
            tracing::debug!(item_id = %event.id, name = %event.name, "dashboard icons skipped: icon already set");
            return Ok(None);
        }

        if let Some(url) = self.cache.lookup(&event.name) {
            tracing::debug!(item_id = %event.id, name = %event.name, icon_url = %url, "dashboard icons match found");
            Ok(Some(SoftwareItemPatch::new().with_icon_url(Some(url))))
        } else {
            tracing::debug!(item_id = %event.id, name = %event.name, "dashboard icons no match");
            Ok(None)
        }
    }
}

// ── Creation function for the catalog ────────────────────────────────────

/// Create the singleton `SoftwareItemLifecycle` instance from catalog config.
///
/// Constructs a `DashboardIconCache` using the shared HTTP client and
/// cancellation token from `CatalogConfig`, then spawns the background
/// refresh loop.
fn create_dashboard_icons_lifecycle(
    config: &CatalogConfig,
) -> uptrakit_plugin_infrastructure_core::Result<Arc<dyn SoftwareItemLifecycle>> {
    let client = config.http_client.clone().unwrap_or_default();

    let cache = Arc::new(DashboardIconCache::new(client));

    if let Some(cancel) = config.cancellation_token.clone() {
        DashboardIconCache::spawn_refresh_loop(Arc::clone(&cache), cancel);
    }

    Ok(Arc::new(DashboardIconsPlugin::new(cache)))
}

// ── declare_plugin! ──────────────────────────────────────────────────────

declare_plugin!(DashboardIconsPlugin, DashboardIconsConfig, "enhancement_dashboard_icons", {
    display_name: "Dashboard Icons",
    family: PluginFamily::Enhancement,
    config_model: ConfigModel::None,
    roles: [SoftwareItemLifecycle],
    software_item_lifecycle: create_dashboard_icons_lifecycle,
});

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::DashboardIconCache;
    use uptrakit_plugin_infrastructure_core::PluginMeta;

    fn make_plugin(paths: &[&str]) -> DashboardIconsPlugin {
        let cache = DashboardIconCache::new_with_paths(reqwest::Client::new(), paths);
        DashboardIconsPlugin::new(Arc::new(cache))
    }

    fn event(name: &str, icon_url: Option<&str>) -> SoftwareItemCreatedEvent {
        SoftwareItemCreatedEvent::new(
            uuid::Uuid::nil(),
            uuid::Uuid::nil(),
            name.to_string(),
            true,
            icon_url.map(String::from),
        )
    }

    #[test]
    fn plugin_meta_returns_correct_type_id() {
        let plugin = make_plugin(&[]);
        assert_eq!(
            plugin.plugin_type_id().as_str(),
            "enhancement_dashboard_icons"
        );
    }

    #[test]
    fn descriptor_has_correct_metadata() {
        assert_eq!(DESCRIPTOR.type_id, "enhancement_dashboard_icons");
        assert_eq!(DESCRIPTOR.display_name, "Dashboard Icons");
        assert_eq!(DESCRIPTOR.family, PluginFamily::Enhancement);
        assert_eq!(DESCRIPTOR.config_model, ConfigModel::None);
    }

    #[tokio::test]
    async fn sets_icon_when_match_found() {
        let plugin = make_plugin(&["svg/nginx.svg"]);
        let ev = event("Nginx", None);
        let patch = plugin.on_software_item_created(&ev).await.unwrap();
        assert!(patch.is_some());
        let patch = patch.unwrap();
        assert!(patch.icon_url.unwrap().unwrap().contains("nginx.svg"));
    }

    #[tokio::test]
    async fn no_patch_when_icon_already_set() {
        let plugin = make_plugin(&["svg/nginx.svg"]);
        let ev = event("Nginx", Some("https://example.com/icon.png"));
        let patch = plugin.on_software_item_created(&ev).await.unwrap();
        assert!(patch.is_none());
    }

    #[tokio::test]
    async fn no_patch_when_no_match() {
        let plugin = make_plugin(&["svg/nginx.svg"]);
        let ev = event("SomeUnknownApp", None);
        let patch = plugin.on_software_item_created(&ev).await.unwrap();
        assert!(patch.is_none());
    }

    #[tokio::test]
    async fn actual_budget_maps_to_actual_budget_slug() {
        let plugin = make_plugin(&["svg/actual-budget-light.svg"]);
        let ev = event("Actual Budget", None);
        let patch = plugin.on_software_item_created(&ev).await.unwrap();
        assert!(patch.is_some());
        let patch = patch.unwrap();
        assert!(
            patch
                .icon_url
                .unwrap()
                .unwrap()
                .contains("actual-budget-light.svg")
        );
    }
}
