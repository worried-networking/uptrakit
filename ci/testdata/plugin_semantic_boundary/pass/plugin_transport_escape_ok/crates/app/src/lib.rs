pub fn good_code() {
    // Uses the non-transport-specific variant — should not be flagged
    let _: Option<uptrakit_plugin_infrastructure_registry::NotificationPluginError> = None;
}
