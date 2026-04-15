use crate::PluginTypeId;

pub fn legacy_semantic_boundary(id: PluginTypeId) {
    let _settings_key = "settings_dashboard_icons";
    let _enabled_key = "dashboard_icons.enabled";
    let _display = PluginTypeId::display_name(id);
    let _instance_display = id.display_name();
    let _package_manager = id.is_package_manager();
    let _plugin_id = crate::plugin_ids::GENERIC_SHELL;
}

fn is_dashboard_icons() -> bool {
    true
}
