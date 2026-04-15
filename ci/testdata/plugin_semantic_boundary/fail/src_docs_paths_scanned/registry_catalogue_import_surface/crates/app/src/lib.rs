use uptrakit_plugin_infrastructure_catalog::all_descriptors as catalog_descriptors;
use uptrakit_plugin_infrastructure_catalogue::all_descriptors as catalogue_descriptors;
use uptrakit_plugin_infrastructure_registry::PluginCatalog;

pub fn allowed_registry_catalogue_surface(_catalog: &dyn PluginCatalog) {
    let _ = (catalog_descriptors, catalogue_descriptors);
}
