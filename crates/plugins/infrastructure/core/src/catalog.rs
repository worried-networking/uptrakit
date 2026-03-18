//! Plugin catalog — unified descriptor index with singleton management.
//!
//! [`PluginCatalog`] replaces `PluginRegistry`. It indexes `PluginDescriptor`s
//! by type ID, constructs singleton transports and lifecycle plugins at startup,
//! and provides extension action routing.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use uptrakit_shared_types::PluginTypeId;

use crate::descriptor::{
    CatalogConfig, ExtensionActionContext, ExtensionActionHandler, PluginDescriptor,
};
use crate::error::PluginError;
use crate::plugin_base::{SoftwareItemCreatedEvent, SoftwareItemPatch};
use crate::plugin_ops::{
    NotificationOps, PluginConfigOps, PluginExtensionOps, PluginMetadataOps,
    SoftwareItemLifecycleOps,
};
use crate::roles::{NotificationTransport, SoftwareItemLifecycle};

/// Errors during catalog construction.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CatalogError {
    #[error("duplicate plugin type_id: {0}")]
    DuplicateTypeId(&'static str),

    #[error("duplicate notification transport: {0}")]
    DuplicateTransport(&'static str),

    #[error("duplicate extension prefix: {0}")]
    DuplicateExtensionPrefix(&'static str),

    #[error(
        "overlapping extension prefix: '{new_prefix}' (from {new_owner}) \
         overlaps with '{existing_prefix}' (from {existing_owner})"
    )]
    OverlappingExtensionPrefix {
        new_prefix: &'static str,
        existing_prefix: &'static str,
        new_owner: &'static str,
        existing_owner: &'static str,
    },

    #[error("failed to create singleton: {0}")]
    SingletonCreation(String),
}

/// Unified plugin catalog — indexes descriptors, manages singletons.
///
/// BTreeMap for deterministic iteration order (alphabetical by type_id).
pub struct PluginCatalog {
    descriptors: BTreeMap<&'static str, &'static PluginDescriptor>,
    transports: BTreeMap<&'static str, Arc<dyn NotificationTransport>>,
    lifecycle_plugins: Vec<Arc<dyn SoftwareItemLifecycle>>,
    extension_routes: Vec<(&'static str, ExtensionActionHandler)>,
}

impl PluginCatalog {
    /// Construct a new catalog from descriptors and shared config.
    ///
    /// Validates uniqueness of type IDs and extension prefixes.
    /// Creates singleton transports and lifecycle plugins.
    pub fn new(
        descriptors: Vec<&'static PluginDescriptor>,
        config: &CatalogConfig,
    ) -> crate::Result<Self> {
        let mut map = BTreeMap::new();
        let mut transports = BTreeMap::new();
        let mut lifecycle_plugins = Vec::new();
        let mut extension_routes = Vec::new();
        // (prefix, owner_type_id) pairs for overlap detection
        let mut seen_ext_prefixes: Vec<(&'static str, &'static str)> = Vec::new();

        for desc in descriptors {
            // ── Uniqueness: type_id ──
            if map.insert(desc.type_id, desc).is_some() {
                return Err(rootcause::report!(PluginError::UnsupportedOperation(
                    format!("duplicate plugin type_id: {}", desc.type_id)
                )));
            }

            // ── Singleton: notification transport ──
            if let Some(create) = desc.roles.notification_transport {
                if transports.contains_key(desc.type_id) {
                    return Err(rootcause::report!(PluginError::UnsupportedOperation(
                        format!("duplicate notification transport: {}", desc.type_id)
                    )));
                }
                let transport = create(config).map_err(|e| {
                    rootcause::report!(PluginError::UnsupportedOperation(format!(
                        "failed to create transport '{}': {e}",
                        desc.type_id
                    )))
                })?;
                transports.insert(desc.type_id, transport);
            }

            // ── Singleton: software item lifecycle enhancement ──
            if let Some(create) = desc.roles.software_item_lifecycle {
                let plugin = create(config).map_err(|e| {
                    rootcause::report!(PluginError::UnsupportedOperation(format!(
                        "failed to create lifecycle plugin '{}': {e}",
                        desc.type_id
                    )))
                })?;
                lifecycle_plugins.push(plugin);
            }

            // ── Uniqueness + overlap: extension prefixes ──
            if let Some(ext) = desc.extensions {
                for prefix in ext.owned_ids {
                    // Reject overlapping prefixes from DIFFERENT descriptors
                    for &(existing_prefix, owner) in &seen_ext_prefixes {
                        if owner == desc.type_id {
                            continue;
                        }
                        if prefix.starts_with(existing_prefix)
                            || existing_prefix.starts_with(prefix)
                        {
                            return Err(rootcause::report!(PluginError::UnsupportedOperation(
                                format!(
                                    "overlapping extension prefix: '{prefix}' (from {}) \
                                     overlaps with '{existing_prefix}' (from {owner})",
                                    desc.type_id
                                )
                            )));
                        }
                    }
                    seen_ext_prefixes.push((prefix, desc.type_id));
                    extension_routes.push((*prefix, ext.handle_action));
                }
            }
        }

        // Longest prefix first for greedy matching
        extension_routes.sort_by(|a, b| b.0.len().cmp(&a.0.len()));

        Ok(Self {
            descriptors: map,
            transports,
            lifecycle_plugins,
            extension_routes,
        })
    }

    /// Route an extension action to the correct handler by prefix match.
    pub fn route_extension_action(&self, ext_id: &str) -> Option<ExtensionActionHandler> {
        self.extension_routes
            .iter()
            .find(|(prefix, _)| ext_id.starts_with(prefix))
            .map(|(_, handler)| *handler)
    }

    /// Collect all controller-side database migrations contributed by plugins.
    #[cfg(feature = "migrations")]
    pub fn all_controller_migrations(&self) -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        self.descriptors
            .values()
            .filter_map(|d| d.migrations)
            .flat_map(|f| f())
            .collect()
    }

    /// Create all compiled-in infrastructure plugin bundles.
    #[cfg(feature = "agent-infra")]
    pub fn create_infra_bundles(
        &self,
        config: &CatalogConfig,
    ) -> Vec<crate::descriptor::InfraBundle> {
        self.descriptors
            .values()
            .filter_map(|d| d.roles.infra.as_ref())
            .filter_map(|slot| (slot.create)(config).ok())
            .collect()
    }
}

// ── Trait implementations ───────────────────────────────────────────────────

impl PluginMetadataOps for PluginCatalog {
    fn get(&self, id: &PluginTypeId) -> Option<&PluginDescriptor> {
        self.descriptors.get(id.as_str()).copied()
    }

    fn all(&self) -> Vec<&PluginDescriptor> {
        self.descriptors.values().copied().collect()
    }
}

impl PluginConfigOps for PluginCatalog {} // all defaults via PluginMetadataOps

impl PluginExtensionOps for PluginCatalog {
    fn extension_manifests_and_actions(
        &self,
    ) -> Vec<(
        uptrakit_extension_framework::ExtensionManifest,
        Vec<uptrakit_extension_framework::ActionDef>,
    )> {
        let mut result = Vec::new();
        for desc in self.descriptors.values() {
            if let Some(ext) = desc.extensions {
                let manifests = (ext.manifests)();
                let actions = (ext.actions)();
                for manifest in manifests {
                    result.push((manifest, actions.clone()));
                }
            }
        }
        result
    }

    fn handle_extension_action<'a>(
        &'a self,
        ctx: &'a ExtensionActionContext<'a>,
        ext_id: &'a str,
        action_id: &'a str,
        params: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<serde_json::Value, String>> + Send + 'a>>
    {
        Box::pin(async move {
            let handler = self
                .route_extension_action(ext_id)
                .ok_or_else(|| format!("no plugin handles extension '{ext_id}'"))?;
            handler(ctx, ext_id, action_id, params).await
        })
    }
}

impl NotificationOps for PluginCatalog {
    fn transport(&self, id: &PluginTypeId) -> Option<Arc<dyn NotificationTransport>> {
        self.transports.get(id.as_str()).cloned()
    }

    fn notification_supported_types(&self) -> Vec<PluginTypeId> {
        self.transports
            .keys()
            .map(|k| PluginTypeId::from_static(k))
            .collect()
    }
}

impl SoftwareItemLifecycleOps for PluginCatalog {
    fn on_software_item_created<'a>(
        &'a self,
        event: &'a SoftwareItemCreatedEvent,
    ) -> Pin<Box<dyn Future<Output = Option<SoftwareItemPatch>> + Send + 'a>> {
        Box::pin(async move {
            let mut merged: Option<SoftwareItemPatch> = None;

            for plugin in &self.lifecycle_plugins {
                match plugin.on_software_item_created(event).await {
                    Ok(Some(patch)) => {
                        let m = merged.get_or_insert_with(SoftwareItemPatch::new);
                        if patch.icon_url.is_some() {
                            m.icon_url = patch.icon_url;
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::warn!(
                            plugin = %plugin.plugin_type_id(),
                            error = %e,
                            "software item lifecycle plugin error"
                        );
                    }
                }
            }

            merged
        })
    }

    fn software_item_lifecycle_plugins(&self) -> &[Arc<dyn SoftwareItemLifecycle>] {
        &self.lifecycle_plugins
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::*;

    /// Empty catalog builds successfully.
    #[test]
    fn empty_catalog() {
        let catalog = PluginCatalog::new(vec![], &CatalogConfig::default()).unwrap();
        assert!(catalog.all().is_empty());
        assert!(catalog.known_type_ids().is_empty());
    }
}
