pub fn keep_non_canonical_dashboard_tokens_clean() {
    let plugin_type = "settings_dashboard_icons";
    let channel_type = "dashboard_icons.enabled";
    let route = "/api/plugin-types/settings_dashboard_icons/config";
    let _ = (plugin_type, channel_type, route);
}
