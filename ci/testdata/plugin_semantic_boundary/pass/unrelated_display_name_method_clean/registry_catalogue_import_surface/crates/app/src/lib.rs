use uptrakit_plugin_infrastructure_catalog::all_descriptors as catalog_descriptors;
use uptrakit_plugin_infrastructure_catalogue::all_descriptors as catalogue_descriptors;
use uptrakit_plugin_infrastructure_registry::PluginCatalog;
use uptrakit_shared_types::PluginTypeId;

pub fn allowed_registry_catalogue_surface(catalog: &dyn PluginCatalog, id: &PluginTypeId) {
    let _ = (catalog_descriptors, catalogue_descriptors);
    let _name = catalog.display_name(id);
}
