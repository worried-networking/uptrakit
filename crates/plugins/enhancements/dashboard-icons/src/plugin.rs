use std::sync::Arc;

use async_trait::async_trait;
use uptrakit_plugin_infrastructure_core::{
    PluginBase, PluginCapability, SoftwareItemCreatedEvent, SoftwareItemLifecyclePlugin,
    SoftwareItemPatch, error::PluginError,
};

use crate::cache::DashboardIconCache;

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
impl PluginBase for DashboardIconsPlugin {
    fn plugin_type_id(&self) -> &str {
        "enhancement_dashboard_icons"
    }

    fn capabilities(&self) -> Vec<PluginCapability> {
        vec![PluginCapability::SoftwareItemLifecycle]
    }

    fn as_software_item_lifecycle(&self) -> Option<&dyn SoftwareItemLifecyclePlugin> {
        Some(self)
    }
}

#[async_trait]
impl SoftwareItemLifecyclePlugin for DashboardIconsPlugin {
    async fn on_software_item_created(
        &self,
        event: &SoftwareItemCreatedEvent,
    ) -> std::result::Result<Option<SoftwareItemPatch>, PluginError> {
        // Don't overwrite an existing icon.
        if event.icon_url.is_some() {
            return Ok(None);
        }

        if let Some(url) = self.cache.lookup(&event.name) {
            Ok(Some(SoftwareItemPatch::new().with_icon_url(Some(url))))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::DashboardIconCache;
    use std::collections::HashSet;

    fn make_plugin(slugs: &[&str]) -> DashboardIconsPlugin {
        let client = reqwest::Client::new();
        let set: HashSet<String> = slugs.iter().map(|s| s.to_string()).collect();
        let cache = DashboardIconCache::new_with_slugs(client, set);
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

    #[tokio::test]
    async fn sets_icon_when_match_found() {
        let plugin = make_plugin(&["nginx"]);
        let ev = event("Nginx", None);
        let patch = plugin.on_software_item_created(&ev).await.unwrap();
        assert!(patch.is_some());
        let patch = patch.unwrap();
        assert!(patch.icon_url.unwrap().unwrap().contains("nginx.svg"));
    }

    #[tokio::test]
    async fn no_patch_when_icon_already_set() {
        let plugin = make_plugin(&["nginx"]);
        let ev = event("Nginx", Some("https://example.com/icon.png"));
        let patch = plugin.on_software_item_created(&ev).await.unwrap();
        assert!(patch.is_none());
    }

    #[tokio::test]
    async fn no_patch_when_no_match() {
        let plugin = make_plugin(&["nginx"]);
        let ev = event("SomeUnknownApp", None);
        let patch = plugin.on_software_item_created(&ev).await.unwrap();
        assert!(patch.is_none());
    }
}
