use std::sync::Arc;

use async_trait::async_trait;
use rootcause::report;
use uptrakit_global_github_provider::lookup_github_provider;
use uptrakit_plugin_infrastructure_core::{
    CatalogConfig, ConfigModel, PluginFamily, SoftwareItemCreatedEvent, SoftwareItemLifecycle,
    SoftwareItemLifecycleContext, SoftwareItemPatch, declare_plugin, error::PluginError,
    plugin_ids,
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
        ctx: &SoftwareItemLifecycleContext,
    ) -> std::result::Result<Option<SoftwareItemPatch>, PluginError> {
        let enabled = ctx
            .typed_type_setting::<DashboardIconsConfig>(&plugin_ids::ENHANCEMENT_DASHBOARD_ICONS)
            .map(|cfg| cfg.enabled)
            .unwrap_or(true);
        if !enabled {
            tracing::debug!(item_id = %event.id, name = %event.name, "dashboard icons disabled via type settings");
            return Ok(None);
        }

        // Don't overwrite an existing icon.
        if event.icon_url.is_some() {
            tracing::debug!(item_id = %event.id, name = %event.name, "dashboard icons skipped: icon already set");
            return Ok(None);
        }

        if let Some(url) = self.cache.lookup_or_try_refresh(&event.name).await {
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
    let github_provider = lookup_github_provider(config).ok_or_else(|| {
        report!(PluginError::PluginInternal(
            "global GitHub provider lookup missing for dashboard-icons".to_string()
        ))
    })?;

    let cache = Arc::new(DashboardIconCache::new(github_provider));

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
    type_settings: true,
    roles: [SoftwareItemLifecycle],
    software_item_lifecycle: create_dashboard_icons_lifecycle,
    global_provider_consumers: ["github"],
});

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::DashboardIconCache;
    use uptrakit_global_github_provider::{
        GitHubProviderClient, GitHubProviderError, GitHubRepositoryTree, GitHubTreeEntry,
        GitHubTreeEntryKind, GlobalProviderConsumerId,
    };
    use uptrakit_plugin_infrastructure_core::{PluginMeta, SoftwareItemLifecycleContext};

    fn make_plugin(paths: &[&str]) -> DashboardIconsPlugin {
        let cache = DashboardIconCache::new_with_paths(paths);
        DashboardIconsPlugin::new(Arc::new(cache))
    }

    fn make_cold_plugin() -> DashboardIconsPlugin {
        struct Provider;

        #[async_trait::async_trait]
        impl GitHubProviderClient for Provider {
            async fn fetch_repository_tree(
                &self,
                _consumer: GlobalProviderConsumerId,
                _owner: &str,
                _repo: &str,
                _git_ref: &str,
                _recursive: bool,
            ) -> std::result::Result<GitHubRepositoryTree, GitHubProviderError> {
                Ok(GitHubRepositoryTree {
                    truncated: false,
                    entries: vec![GitHubTreeEntry {
                        path: "svg/nginx.svg".to_string(),
                        kind: GitHubTreeEntryKind::Blob,
                    }],
                })
            }
        }

        DashboardIconsPlugin::new(Arc::new(DashboardIconCache::new(Arc::new(Provider))))
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

    fn context() -> SoftwareItemLifecycleContext {
        SoftwareItemLifecycleContext::default()
    }

    fn context_with_explicit_disabled_setting() -> SoftwareItemLifecycleContext {
        let mut ctx = SoftwareItemLifecycleContext::default();
        ctx.insert_type_setting(
            plugin_ids::ENHANCEMENT_DASHBOARD_ICONS,
            serde_json::json!({ "enabled": false }),
        );
        ctx
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
        assert_eq!(DESCRIPTOR.global_provider_consumers.len(), 1);
        assert_eq!(DESCRIPTOR.global_provider_consumers[0].as_str(), "github");
    }

    #[tokio::test]
    async fn sets_icon_when_match_found() {
        let plugin = make_plugin(&["svg/nginx.svg"]);
        let ev = event("Nginx", None);
        let patch = plugin
            .on_software_item_created(&ev, &context())
            .await
            .unwrap();
        assert!(patch.is_some());
        let patch = patch.unwrap();
        assert!(patch.icon_url.unwrap().unwrap().contains("nginx.svg"));
    }

    #[tokio::test]
    async fn no_patch_when_icon_already_set() {
        let plugin = make_plugin(&["svg/nginx.svg"]);
        let ev = event("Nginx", Some("https://example.com/icon.png"));
        let patch = plugin
            .on_software_item_created(&ev, &context())
            .await
            .unwrap();
        assert!(patch.is_none());
    }

    #[tokio::test]
    async fn no_patch_when_no_match() {
        let plugin = make_plugin(&["svg/nginx.svg"]);
        let ev = event("SomeUnknownApp", None);
        let patch = plugin
            .on_software_item_created(&ev, &context())
            .await
            .unwrap();
        assert!(patch.is_none());
    }

    #[tokio::test]
    async fn cold_cache_refreshes_on_first_creation_lookup() {
        let plugin = make_cold_plugin();
        let ev = event("Nginx", None);
        let patch = plugin
            .on_software_item_created(&ev, &context())
            .await
            .unwrap();
        assert!(patch.is_some());
        let patch = patch.unwrap();
        assert!(patch.icon_url.unwrap().unwrap().contains("nginx.svg"));
    }

    #[tokio::test]
    async fn actual_budget_maps_to_actual_budget_slug() {
        let plugin = make_plugin(&["svg/actual-budget-light.svg"]);
        let ev = event("Actual Budget", None);
        let patch = plugin
            .on_software_item_created(&ev, &context())
            .await
            .unwrap();
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

    #[tokio::test]
    async fn explicit_disabled_setting_disables_enrichment() {
        let plugin = make_plugin(&["svg/nginx.svg"]);
        let ev = event("Nginx", None);
        let patch = plugin
            .on_software_item_created(&ev, &context_with_explicit_disabled_setting())
            .await
            .unwrap();
        assert!(patch.is_none());
    }
}
